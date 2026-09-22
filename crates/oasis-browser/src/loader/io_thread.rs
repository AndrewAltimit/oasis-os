//! Background I/O thread for non-blocking network resource loading.
//!
//! The [`IoThread`] processes HTTP/HTTPS requests off the main thread,
//! preventing network latency from blocking the browser's rendering loop.
//! VFS (local) requests remain synchronous since they are fast in-memory
//! reads.
//!
//! The thread owns its own `CookieJar` clone and receives TLS provider
//! access via `Arc`. Cookie updates and cache validators are sent back
//! with each response so the main thread can apply them.

#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
mod inner {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    use oasis_net::tls::TlsProvider;
    use oasis_types::error::OasisError;

    use crate::loader::cookies::CookieJar;
    use crate::loader::csp;
    use crate::loader::{self, HttpMethod, LoadedResource, ResourceRequest, Url};

    /// Return type for `execute_request`: the loaded resource and any
    /// cookie updates that need replaying on the main-thread jar.
    type RequestResult = (
        oasis_types::error::Result<LoadedResource>,
        Vec<(String, Vec<(String, String)>)>,
    );

    /// Unique identifier for an in-flight I/O request.
    pub type IoRequestId = u64;

    /// Categorises what kind of request this is, so the main thread
    /// knows how to handle the response.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IoRequestKind {
        /// A top-level page navigation (GET or POST).
        PageLoad,
        /// A sub-resource image fetch.
        Image,
        /// An external `<link rel="stylesheet">` CSS fetch.
        Stylesheet,
    }

    /// A request submitted to the I/O thread.
    pub struct IoWork {
        pub id: IoRequestId,
        pub kind: IoRequestKind,
        pub request: ResourceRequest,
        /// Pre-extracted conditional-request validators from the cache
        /// (ETag, If-Modified-Since). Avoids sending the non-Send cache.
        pub cache_validators: Option<(Option<String>, Option<String>)>,
        /// The resolved image URL key (only for `Image` kind).
        pub image_key: Option<String>,
    }

    /// A completed response from the I/O thread.
    pub struct IoResult {
        pub id: IoRequestId,
        pub kind: IoRequestKind,
        pub result: oasis_types::error::Result<LoadedResource>,
        /// Cookie `Set-Cookie` updates to apply to the main-thread jar.
        pub cookie_updates: Vec<(String, Vec<(String, String)>)>,
        /// The resolved image URL key (only for `Image` kind).
        pub image_key: Option<String>,
    }

    /// Background I/O thread for non-blocking HTTP requests.
    ///
    /// Lazily spawned on first use. Communicates via `mpsc` channels.
    /// The thread processes requests sequentially (not in parallel) to
    /// keep resource usage predictable.
    ///
    /// On drop, queued requests are abandoned and the sender channel is
    /// closed. An idle worker is joined; a worker blocked in a request
    /// (a slow or stalled server) is detached instead of stalling the
    /// caller's thread — it exits as soon as that request returns. The
    /// worker owns an `Arc` clone of the `TlsProvider`, so the provider
    /// can never be freed while the thread may still use it.
    pub struct IoThread {
        tx: mpsc::Sender<IoWork>,
        rx: mpsc::Receiver<IoResult>,
        handle: Option<std::thread::JoinHandle<()>>,
        next_id: IoRequestId,
        in_flight: usize,
        /// Set on drop: the worker stops before starting another request.
        cancelled: Arc<AtomicBool>,
    }

    impl Drop for IoThread {
        fn drop(&mut self) {
            self.cancelled.store(true, Ordering::SeqCst);
            // Close the sender so the worker's `recv()` returns `Err`.
            // (Happens automatically when `tx` is dropped, but we drop
            // it explicitly here before joining for clarity.)
            drop(std::mem::replace(&mut self.tx, mpsc::channel().0));
            if let Some(handle) = self.handle.take()
                && (self.in_flight == 0 || handle.is_finished())
            {
                let _ = handle.join();
            }
        }
    }

    impl IoThread {
        /// Create and start the background I/O thread.
        ///
        /// `tls` is cloned into the thread for HTTPS support.
        /// `cookie_jar` is cloned so the thread can send/receive cookies.
        ///
        /// Returns the OS error if the thread cannot be spawned (e.g. thread
        /// or memory limits), so callers can degrade instead of panicking.
        pub fn spawn(
            tls: Option<Arc<dyn TlsProvider>>,
            cookie_jar: CookieJar,
        ) -> std::io::Result<Self> {
            let (work_tx, work_rx) = mpsc::channel::<IoWork>();
            let (result_tx, result_rx) = mpsc::channel::<IoResult>();
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = Arc::clone(&cancelled);

            let handle = std::thread::Builder::new()
                .name("browser-io".into())
                .spawn(move || {
                    Self::worker_loop(work_rx, result_tx, tls, cookie_jar, &worker_cancelled);
                })?;

            Ok(IoThread {
                tx: work_tx,
                rx: result_rx,
                handle: Some(handle),
                next_id: 1,
                in_flight: 0,
                cancelled,
            })
        }

        /// Submit a request to the I/O thread. Returns the request ID.
        pub fn send(
            &mut self,
            kind: IoRequestKind,
            request: ResourceRequest,
            cache_validators: Option<(Option<String>, Option<String>)>,
            image_key: Option<String>,
        ) -> IoRequestId {
            let id = self.next_id;
            self.next_id += 1;
            let work = IoWork {
                id,
                kind,
                request,
                cache_validators,
                image_key,
            };
            // If the channel is disconnected the thread has panicked.
            // In that case we just drop the request (the caller will
            // time out or notice the missing response).
            if self.tx.send(work).is_ok() {
                self.in_flight += 1;
            }
            id
        }

        /// Poll for completed responses (non-blocking).
        ///
        /// Returns `None` when no responses are ready.
        pub fn poll(&mut self) -> Option<IoResult> {
            match self.rx.try_recv() {
                Ok(result) => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    Some(result)
                },
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Worker thread died. Reset in-flight counter.
                    self.in_flight = 0;
                    None
                },
            }
        }

        /// Number of requests currently in-flight.
        pub fn in_flight(&self) -> usize {
            self.in_flight
        }

        /// Worker loop running on the background thread.
        fn worker_loop(
            work_rx: mpsc::Receiver<IoWork>,
            result_tx: mpsc::Sender<IoResult>,
            tls: Option<Arc<dyn TlsProvider>>,
            mut cookie_jar: CookieJar,
            cancelled: &AtomicBool,
        ) {
            while let Ok(work) = work_rx.recv() {
                // The owner is gone: drop the backlog (e.g. a closed
                // browser's queued image fetches) instead of fetching it.
                if cancelled.load(Ordering::SeqCst) {
                    break;
                }
                let (loaded, cookie_updates) =
                    Self::execute_request(&work, tls.as_deref(), &mut cookie_jar);

                let io_result = IoResult {
                    id: work.id,
                    kind: work.kind,
                    result: loaded,
                    cookie_updates,
                    image_key: work.image_key,
                };

                if result_tx.send(io_result).is_err() {
                    break; // Main thread dropped its receiver.
                }
            }
        }

        /// Execute a single network request.
        ///
        /// Returns the loaded resource and any cookie updates that need
        /// to be applied to the main-thread cookie jar.
        fn execute_request(
            work: &IoWork,
            tls: Option<&dyn TlsProvider>,
            cookie_jar: &mut CookieJar,
        ) -> RequestResult {
            let request = &work.request;
            let url = match Url::parse(&request.url) {
                Some(u) => u,
                None => {
                    return (
                        Err(OasisError::Backend(
                            format!("invalid URL: {}", request.url).into(),
                        )),
                        Vec::new(),
                    );
                },
            };

            let method = match request.method {
                HttpMethod::Get => "GET",
                HttpMethod::Post => "POST",
            };

            // Build extra headers.
            let mut extra: Vec<(String, String)> = Vec::new();

            // Referrer header.
            if let Some(ref referrer) = request.referrer {
                extra.push(("Referer".to_string(), referrer.clone()));
            }

            // Cookie header.
            if let Some(cookie_val) = cookie_jar.cookie_header(&url) {
                extra.push(("Cookie".to_string(), cookie_val));
            }

            // Conditional request headers from pre-extracted cache validators.
            if let Some((ref etag, ref last_mod)) = work.cache_validators {
                if let Some(e) = etag {
                    extra.push(("If-None-Match".to_string(), e.clone()));
                }
                if let Some(lm) = last_mod {
                    extra.push(("If-Modified-Since".to_string(), lm.clone()));
                }
            }

            let extra_refs: Vec<(&str, &str)> = extra
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            match url.scheme.as_str() {
                "http" | "https" => {
                    match loader::http::http_request_full(
                        method,
                        &url,
                        request.body.as_deref(),
                        &extra_refs,
                        tls,
                    ) {
                        Ok((resp, headers)) => {
                            // Collect cookie updates to replay on main thread.
                            let mut cookie_updates = Vec::new();
                            let set_cookies: Vec<(String, String)> = headers
                                .iter()
                                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                                .cloned()
                                .collect();
                            if !set_cookies.is_empty() {
                                // Apply to our local jar too.
                                cookie_jar.set_cookies(&url, &headers);
                                cookie_updates.push((url.to_string(), set_cookies));
                            }

                            // Extract cache validators.
                            let etag = loader::http::response_find_header(&headers, "etag")
                                .map(String::from);
                            let last_modified =
                                loader::http::response_find_header(&headers, "last-modified")
                                    .map(String::from);

                            // Parse CSP header.
                            let csp = loader::http::response_find_header(
                                &headers,
                                "content-security-policy",
                            )
                            .map(csp::parse_csp);

                            let loaded = LoadedResource {
                                response: resp,
                                etag,
                                last_modified,
                                csp,
                            };

                            (Ok(loaded), cookie_updates)
                        },
                        Err(e) => (Err(e), Vec::new()),
                    }
                },
                "gemini" => {
                    let gemini_result = loader::gemini_fetch::gemini_get(&url, tls)
                        .map(LoadedResource::from_response);
                    (gemini_result, Vec::new())
                },
                scheme => (
                    Err(OasisError::Backend(
                        format!("unsupported network scheme: {scheme}").into(),
                    )),
                    Vec::new(),
                ),
            }
        }
    }
}

#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
pub use inner::*;

#[cfg(all(test, not(any(target_arch = "wasm32", feature = "psp"))))]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use oasis_net::tls::TlsProvider;
    use oasis_types::backend::NetworkStream;

    use super::IoThread;
    use crate::loader::cookies::CookieJar;

    /// TLS provider that records when it is dropped.
    struct DropFlagProvider(Arc<AtomicBool>);

    impl Drop for DropFlagProvider {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl TlsProvider for DropFlagProvider {
        fn connect_tls(
            &self,
            stream: Box<dyn NetworkStream>,
            _server_name: &str,
        ) -> oasis_types::error::Result<Box<dyn NetworkStream>> {
            Ok(stream)
        }
    }

    #[test]
    fn worker_keeps_tls_provider_alive_after_caller_drops_it() {
        let dropped = Arc::new(AtomicBool::new(false));
        let provider: Arc<dyn TlsProvider> = Arc::new(DropFlagProvider(Arc::clone(&dropped)));

        let io = IoThread::spawn(Some(Arc::clone(&provider)), CookieJar::new())
            .expect("spawn I/O thread");
        // The caller's handle goes away (e.g. `set_tls_provider` replaced it).
        drop(provider);
        assert!(
            !dropped.load(Ordering::SeqCst),
            "provider must outlive the I/O thread that references it"
        );

        // Joining the worker releases the last strong reference.
        drop(io);
        assert!(dropped.load(Ordering::SeqCst));
    }

    /// Dropping the thread while a request is blocked on a stalled server
    /// must not block the dropping (UI) thread until the request returns,
    /// and the worker must exit once it does, skipping queued work.
    #[test]
    fn drop_with_request_in_flight_does_not_block() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        use super::IoRequestKind;
        use crate::loader::{HttpMethod, ResourceRequest, ResourceSource};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut conns = Vec::new();
            // Accept the first request, stall it, then answer it.
            if let Ok((mut c, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = c.read(&mut buf);
                let _ = accepted_tx.send(());
                std::thread::sleep(Duration::from_millis(600));
                use std::io::Write;
                let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                conns.push(c);
            }
            // Any further request would be the abandoned backlog.
            listener.set_nonblocking(true).expect("nonblocking");
            let deadline = Instant::now() + Duration::from_millis(800);
            let mut extra = 0;
            while Instant::now() < deadline {
                if listener.accept().is_ok() {
                    extra += 1;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            extra
        });

        let mut io = IoThread::spawn(None, CookieJar::new()).expect("spawn");
        let req = |path: &str| ResourceRequest {
            url: format!("http://127.0.0.1:{port}{path}"),
            base_url: None,
            source: ResourceSource::Network,
            method: HttpMethod::Get,
            body: None,
            referrer: None,
        };
        io.send(IoRequestKind::PageLoad, req("/slow"), None, None);
        for i in 0..5 {
            io.send(IoRequestKind::Image, req(&format!("/img{i}")), None, None);
        }
        accepted_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server saw the first request");
        let start = Instant::now();
        drop(io);
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "drop blocked for {:?}",
            start.elapsed()
        );
        let extra = server.join().expect("server thread");
        assert_eq!(extra, 0, "queued requests must be abandoned after drop");
    }
}
