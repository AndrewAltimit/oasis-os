//! Navigation, resource loading, and content processing for [`BrowserWidget`].

use std::collections::HashMap;

use oasis_vfs::Vfs;

use crate::css;
use crate::gemini;
use crate::html;
use crate::html::dom::NodeId;
#[cfg(feature = "javascript")]
use crate::js_dom;
use crate::layout;
use crate::loader::cache::CacheEntry;
use crate::loader::{
    self, ContentType, ResourceRequest, ResourceResponse, ResourceSource, load_resource,
};
use crate::reader;
use crate::{BrowserWidget, LoadingState, SimpleTextMeasurer};

#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
use crate::loader::io_thread::{IoRequestKind, IoThread};

/// Wrapper to share a `TlsProvider` reference with the I/O thread.
///
/// # Safety
///
/// The raw pointer is valid for the lifetime of the `BrowserWidget` that
/// owns the original `Box<dyn TlsProvider>`. `IoThread::drop()` closes
/// the sender channel and joins the worker thread, ensuring it has fully
/// exited before `tls` (and thus the pointee) is freed.
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
struct SharedTlsProvider(*const dyn oasis_net::tls::TlsProvider);

// SAFETY: TlsProvider is Send + Sync, and the pointer is valid for the
// lifetime of the BrowserWidget. The I/O thread never outlives the widget.
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
unsafe impl Send for SharedTlsProvider {}
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
unsafe impl Sync for SharedTlsProvider {}

#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
impl oasis_net::tls::TlsProvider for SharedTlsProvider {
    fn connect_tls(
        &self,
        stream: Box<dyn oasis_types::backend::NetworkStream>,
        server_name: &str,
    ) -> oasis_types::error::Result<Box<dyn oasis_types::backend::NetworkStream>> {
        // SAFETY: pointer is valid for our lifetime (see above).
        unsafe { &*self.0 }.connect_tls(stream, server_name)
    }

    fn connect_tls_with_alpn(
        &self,
        stream: Box<dyn oasis_types::backend::NetworkStream>,
        server_name: &str,
        alpn_protocols: &[&[u8]],
    ) -> oasis_types::error::Result<oasis_types::tls::TlsConnection> {
        // SAFETY: pointer is valid for our lifetime (see above).
        // Forwarding this is load-bearing: without it the default
        // trait impl silently drops the ALPN offer and the HTTP/2
        // path in the loader never gets taken, which makes sites
        // like wikipedia.org (h2-only) fail with "malformed HTTP
        // response" when the HTTP/1.1 parser sees an h2 frame.
        unsafe { &*self.0 }.connect_tls_with_alpn(stream, server_name, alpn_protocols)
    }
}

impl BrowserWidget {
    /// Navigate via HTTP POST to a URL with the given body.
    ///
    /// Used for `<form method="post">` submissions. The encoded form
    /// data is sent as the request body with
    /// `Content-Type: application/x-www-form-urlencoded`.
    pub fn navigate_post(&mut self, url: &str, body: Vec<u8>, vfs: &dyn Vfs) {
        self.reset_for_navigation();

        let source = if self.config.features.sandbox_only {
            ResourceSource::Vfs
        } else {
            ResourceSource::VfsThenNetwork
        };

        let referrer = self.nav.current_url().and_then(loader::strip_referrer);
        let request = ResourceRequest {
            url: url.to_string(),
            base_url: self.nav.current_url().map(String::from),
            source,
            method: loader::HttpMethod::Post,
            body: Some(body),
            referrer,
        };

        // POST requests always go to the network, so offload to IO thread.
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        {
            if self.should_use_io_thread(&request, vfs) {
                self.submit_page_load_to_io_thread(request);
                return;
            }
        }

        // Synchronous fallback (VFS-only or WASM/PSP).
        self.execute_sync_load(url, vfs, request);
    }

    /// Navigate using the in-memory cache if available, otherwise fetch.
    ///
    /// Used by back/forward navigation to avoid re-fetching pages that
    /// were already loaded. Falls back to `navigate_vfs()` on cache miss.
    pub fn navigate_cached_or_fetch(&mut self, url: &str, vfs: &dyn Vfs) {
        use crate::loader::ContentType;

        // Check if we have a cached response for this URL.
        if let Some(entry) = self.cache.get(url) {
            let body = entry.response.body.clone();
            let ct = entry.response.content_type;
            if ct == ContentType::Html || ct == ContentType::PlainText || ct == ContentType::Unknown
            {
                // Re-render from cached HTML without a network round-trip.
                // Skip nav.navigate() to preserve forward stack.
                self.skip_nav_push = true;
                self.state = LoadingState::Loading;
                self.selected_link = -1;
                self.reader_mode = false;
                self.reader_html = None;
                self.error_message = None;
                self.page_csp = None;
                self.page_errors.clear();
                self.decoded_images.clear();
                self.mask_image_arcs.clear();
                self.image_textures.clear();
                self.image_atlas.clear_without_destroy();
                self.pending_images.clear();
                self.decoded_image_bytes = 0;
                self.decoded_image_lru.clear();
                self.cached_image_info.clear();
                self.image_info_dirty = false;

                let text = String::from_utf8_lossy(&body);
                self.load_html(&text, url);

                self.collect_page_image_requests();
                if !self.pending_images.is_empty() {
                    self.state = LoadingState::Loading;
                }
                return;
            }
        }

        // Cache miss or non-HTML content — fetch from network.
        self.navigate_vfs(url, vfs);
    }

    /// Reset browser state in preparation for a new navigation.
    ///
    /// Clears the parsed document and layout tree along with all
    /// per-page image/atlas caches so callers can rely on a blank slate
    /// whether or not a fresh document ends up being loaded (error
    /// pages, iframe-overlay mode, …).
    fn reset_for_navigation(&mut self) {
        self.state = LoadingState::Loading;
        self.selected_link = -1;
        self.reader_mode = false;
        self.reader_html = None;
        self.error_message = None;
        self.page_csp = None;
        self.page_errors.clear();
        self.document = None;
        self.layout_root = None;
        self.decoded_images.clear();
        self.broken_image_urls.clear();
        self.mask_image_arcs.clear();
        self.image_textures.clear();
        self.image_atlas.clear_without_destroy();
        self.pending_images.clear();
        self.decoded_image_bytes = 0;
        self.decoded_image_lru.clear();
        self.cached_image_info.clear();
        self.image_info_dirty = false;
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        {
            self.pending_page_load = None;
            self.pending_io_images.clear();
        }
    }

    /// Navigate to a URL using the VFS as the resource source.
    pub fn navigate_vfs(&mut self, url: &str, vfs: &dyn Vfs) {
        // iframe-overlay mode (WASM): an external browser iframe paints
        // http(s) pages. The OASIS engine only needs to track the URL
        // for the chrome bar and history, so skip the sync fetch, DOM
        // parse, and JS engine init — every one of those would fail or
        // panic without a network stack.
        if self.config.features.iframe_http_mode
            && (url.starts_with("http://") || url.starts_with("https://"))
        {
            self.reset_for_navigation();
            // We don't parse the page so no <title> is ever extracted;
            // use the hostname (falling back to the full URL) as a
            // human-readable placeholder for the chrome bar and the
            // back/forward history entries.
            let parsed = loader::Url::parse(url);
            let title = parsed
                .as_ref()
                .map(|u| u.host.as_str())
                .filter(|h| !h.is_empty())
                .unwrap_or(url);
            self.nav.navigate(url, title);
            self.state = LoadingState::Idle;
            return;
        }

        self.reset_for_navigation();

        // Internal pages: serve directly without hitting the VFS or
        // network. Only `vfs://bookmarks` is wired up for now — the
        // bookmarks button in the chrome navigates here. History is
        // available through `nav.history_page_html()` if we ever wire
        // a second button for it.
        if url == "vfs://bookmarks" || url == "oasis://bookmarks" {
            let body = self.nav.bookmarks_page_html().into_bytes();
            let response = crate::loader::ResourceResponse {
                url: url.to_string(),
                content_type: crate::loader::ContentType::Html,
                body,
                status: 200,
            };
            self.process_response(response);
            return;
        }

        let source = if self.config.features.sandbox_only {
            ResourceSource::Vfs
        } else {
            ResourceSource::VfsThenNetwork
        };

        let referrer = self.nav.current_url().and_then(loader::strip_referrer);
        let request = ResourceRequest {
            url: url.to_string(),
            base_url: self.nav.current_url().map(String::from),
            source,
            method: loader::HttpMethod::Get,
            body: None,
            referrer,
        };

        // Determine if this is a network request that can be offloaded.
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        {
            if self.should_use_io_thread(&request, vfs) {
                self.submit_page_load_to_io_thread(request);
                return;
            }
        }

        // Synchronous path: VFS-only or fallback.
        self.execute_sync_load(url, vfs, request);
    }

    /// Synchronous resource load and processing (VFS or fallback).
    fn execute_sync_load(&mut self, url: &str, vfs: &dyn Vfs, request: ResourceRequest) {
        self.diag(&format!("[BR] sync_load fetch start: {url}"));
        match load_resource(
            vfs,
            &request,
            self.tls.as_deref(),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            Some(&mut self.cookie_jar),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            Some(&self.cache),
        ) {
            Ok(loaded) => {
                self.diag(&format!(
                    "[BR] sync_load fetch ok: {} bytes, status={}",
                    loaded.response.body.len(),
                    loaded.response.status,
                ));
                let url_str = loaded.response.url.clone();
                let etag = loaded.etag.clone();
                let last_modified = loaded.last_modified.clone();
                self.page_csp = loaded.csp;
                self.diag("[BR] process_response start");
                self.process_response(loaded.response);
                self.diag("[BR] process_response done");
                self.cache.set_validators(&url_str, etag, last_modified);
            },
            Err(e) => {
                let err_msg = e.to_string();
                self.diag(&format!("[BR] sync_load fetch err: {err_msg}"));
                let err_resp = loader::vfs::error_page(url, &err_msg);
                self.process_response(err_resp);
                self.state = LoadingState::Error;
                self.error_message = Some(err_msg.clone());
                self.record_error(crate::BrowserErrorKind::Network, err_msg);
            },
        }

        // Collect image requests for time-sliced loading across frames.
        self.collect_page_image_requests();
        self.diag(&format!(
            "[BR] image queue: {} pending",
            self.pending_images.len()
        ));

        // On PSP, tick() isn't currently called from the main loop
        // (originally because of a misdiagnosed "std::time::Instant
        // crashes on Allegrex" claim — the real cause was the
        // orphaned rust-psp std time overlay, fixed in branch
        // fix/psp-hardware-std-overlay-alignment-and-time). Until the
        // PSP main loop is wired up to call tick() each frame, load
        // all images synchronously here before returning.
        #[cfg(feature = "psp")]
        if !self.pending_images.is_empty() {
            self.diag("[BR] image batch start (PSP synchronous)");
            self.load_next_image_batch(vfs, 5000);
            self.diag("[BR] image batch done");
        }

        // On desktop/WASM, images stream in via `load_next_image_batch()`
        // called from `tick()` each frame.
        #[cfg(not(feature = "psp"))]
        if !self.pending_images.is_empty() {
            self.state = LoadingState::Loading;
        }
    }

    /// Check if a request should be offloaded to the I/O thread.
    ///
    /// Returns `true` for network-only requests (no VFS fallback needed
    /// for the initial attempt). For `VfsThenNetwork`, we try VFS first
    /// synchronously and only offload to the IO thread on VFS miss.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    fn should_use_io_thread(&self, request: &ResourceRequest, vfs: &dyn Vfs) -> bool {
        // VFS-only URLs (vfs://) are always local reads.
        if request.url.starts_with("vfs://") {
            return false;
        }

        // If no TLS provider is configured, handle HTTPS/Gemini URLs
        // synchronously so error pages render immediately.
        if self.tls.is_none() {
            let url_lower = request.url.to_ascii_lowercase();
            if url_lower.starts_with("https://") || url_lower.starts_with("gemini://") {
                return false;
            }
        }

        match request.source {
            ResourceSource::Network => true,
            ResourceSource::VfsThenNetwork => {
                // If VFS has the resource, no need for the IO thread.
                loader::vfs::load_from_vfs(vfs, request).is_err()
            },
            ResourceSource::Vfs => false,
        }
    }

    /// Ensure the I/O thread is running and return a mutable reference.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    fn ensure_io_thread(&mut self) {
        if self.io_thread.is_some() {
            return;
        }
        let tls: Option<std::sync::Arc<dyn oasis_net::tls::TlsProvider>> =
            self.tls.as_ref().map(|t| {
                // Share the TLS provider with the IO thread via a raw
                // pointer wrapper (SharedTlsProvider). See its SAFETY
                // documentation above.
                std::sync::Arc::from(Self::clone_tls_provider_to_arc(t.as_ref()))
            });

        let cookie_jar = self.cookie_jar.clone();
        self.io_thread = Some(IoThread::spawn(tls, cookie_jar));
    }

    /// Clone a `Box<dyn TlsProvider>` reference into a boxed trait object
    /// suitable for wrapping in `Arc`.
    ///
    /// Since `TlsProvider` doesn't require `Clone`, we use a wrapper
    /// that shares the original provider via a raw pointer. This is safe
    /// because the IO thread lifetime is bounded by `BrowserWidget`'s
    /// lifetime (the thread is joined/dropped when BrowserWidget drops).
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    fn clone_tls_provider_to_arc(
        provider: &dyn oasis_net::tls::TlsProvider,
    ) -> Box<dyn oasis_net::tls::TlsProvider + 'static> {
        // We use a SharedTlsProvider that holds a raw pointer.
        // SAFETY: The IoThread is dropped before BrowserWidget (which
        // owns the TLS provider), so the pointer remains valid.
        let ptr = provider as *const dyn oasis_net::tls::TlsProvider;
        // SAFETY: We are erasing the lifetime. The IoThread is destroyed
        // before the BrowserWidget (and thus before the TLS provider).
        let ptr: *const dyn oasis_net::tls::TlsProvider = unsafe { std::mem::transmute(ptr) };
        Box::new(SharedTlsProvider(ptr))
    }

    /// Submit a page load request to the I/O thread.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    fn submit_page_load_to_io_thread(&mut self, request: ResourceRequest) {
        self.ensure_io_thread();

        // Extract cache validators before sending.
        let validators = self.cache.peek_validators(&request.url);

        if let Some(ref mut io) = self.io_thread {
            let id = io.send(IoRequestKind::PageLoad, request, validators, None);
            self.pending_page_load = Some(id);
        }
    }

    /// Submit an image load request to the I/O thread.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    pub(crate) fn submit_image_to_io_thread(
        &mut self,
        resolved_url: String,
        request: ResourceRequest,
    ) {
        self.ensure_io_thread();

        let validators = self.cache.peek_validators(&request.url);

        if let Some(ref mut io) = self.io_thread {
            let id = io.send(
                IoRequestKind::Image,
                request,
                validators,
                Some(resolved_url.clone()),
            );
            self.pending_io_images.insert(id, resolved_url);
        }
    }

    /// Poll the I/O thread for completed requests and process results.
    ///
    /// Called from `tick()` each frame. Handles both page load and image
    /// load completions.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    pub(crate) fn poll_io_thread(&mut self) {
        let io = match &mut self.io_thread {
            Some(io) => io,
            None => return,
        };

        // Drain all available results.
        let mut page_results = Vec::new();
        let mut image_results = Vec::new();
        let mut stylesheet_results = Vec::new();

        while let Some(result) = io.poll() {
            // Apply cookie updates to the main-thread jar.
            for (url_str, headers) in &result.cookie_updates {
                if let Some(url) = loader::Url::parse(url_str) {
                    self.cookie_jar.set_cookies(&url, headers);
                }
            }

            match result.kind {
                IoRequestKind::PageLoad => page_results.push(result),
                IoRequestKind::Image => image_results.push(result),
                IoRequestKind::Stylesheet => stylesheet_results.push(result),
            }
        }

        // Process page load results.
        for result in page_results {
            if self.pending_page_load == Some(result.id) {
                self.pending_page_load = None;
                match result.result {
                    Ok(loaded) => {
                        let url_str = loaded.response.url.clone();
                        let etag = loaded.etag.clone();
                        let last_modified = loaded.last_modified.clone();
                        self.page_csp = loaded.csp;
                        self.process_response(loaded.response);
                        self.cache.set_validators(&url_str, etag, last_modified);
                    },
                    Err(e) => {
                        let err_msg = e.to_string();
                        let url = "about:error";
                        let err_resp = loader::vfs::error_page(url, &err_msg);
                        self.process_response(err_resp);
                        self.state = LoadingState::Error;
                        self.error_message = Some(err_msg.clone());
                        self.record_error(crate::BrowserErrorKind::Network, err_msg);
                    },
                }

                self.collect_page_image_requests();
                if !self.pending_images.is_empty() {
                    self.state = LoadingState::Loading;
                }
            }
        }

        // Process image load results.
        for result in image_results {
            let resolved_url = result
                .image_key
                .or_else(|| self.pending_io_images.remove(&result.id));
            let Some(resolved) = resolved_url else {
                continue;
            };
            self.pending_io_images.remove(&result.id);

            match result.result {
                Ok(loaded) => {
                    let body = loaded.response.body;
                    // Dispatch to the background decode thread.
                    self.ensure_decode_thread();
                    let sent = if let Some(ref tx) = self.image_decode_tx {
                        tx.send((resolved.clone(), body.clone())).is_ok()
                    } else {
                        false
                    };
                    if sent {
                        self.image_decode_in_flight += 1;
                    } else {
                        // Sync fallback — use placeholder on decode failure.
                        let decoded = match crate::image::decode_image(&body) {
                            Some(img) => img,
                            None => {
                                self.broken_image_urls.insert(resolved.clone());
                                crate::image::broken_image_placeholder(24, 24)
                            },
                        };
                        let img_bytes = decoded.width as usize * decoded.height as usize * 4;
                        self.decoded_image_bytes += img_bytes;
                        self.decoded_image_lru.push_front(resolved.clone());
                        self.decoded_images.insert(resolved, decoded);
                        self.image_info_dirty = true;
                        self.layout_dirty = true;
                    }
                },
                Err(_) => {
                    // Network fetch failed — show broken-image placeholder.
                    let placeholder = crate::image::broken_image_placeholder(24, 24);
                    let img_bytes = placeholder.width as usize * placeholder.height as usize * 4;
                    self.decoded_image_bytes += img_bytes;
                    self.broken_image_urls.insert(resolved.clone());
                    self.decoded_image_lru.push_front(resolved.clone());
                    self.decoded_images.insert(resolved, placeholder);
                    self.image_info_dirty = true;
                    self.layout_dirty = true;
                },
            }
        }

        // Process external stylesheet results. Each fills the pre-allocated
        // slot at the DOM-order index that was recorded when the fetch was
        // submitted, then flags `pending_external_css_apply` so the next
        // tick re-cascades and relays out with the now-fuller style set.
        const MAX_STYLESHEET_BYTES: usize = 512 * 1024;
        for result in stylesheet_results {
            let Some(idx) = self.pending_io_stylesheets.remove(&result.id) else {
                continue;
            };
            let sheet = match result.result {
                Ok(loaded) => {
                    let body = loaded.response.body;
                    if body.len() > MAX_STYLESHEET_BYTES {
                        log::warn!(
                            "external stylesheet {} too large ({} bytes), skipping",
                            loaded.response.url,
                            body.len()
                        );
                        // Slot stays `None` but still flag a cascade pass so
                        // any peer sheets that *did* arrive this tick get
                        // applied; otherwise a single oversized sheet can
                        // strand the page unstyled with no retry signal.
                        self.pending_external_css_apply = true;
                        continue;
                    }
                    let css_text = String::from_utf8_lossy(&body).into_owned();
                    let viewport = css::parser::MediaViewport {
                        width: self.window_w as f32,
                        height: self.window_h as f32,
                        dark_mode: false,
                        prefers_reduced_motion: false,
                        hover: true,
                        pointer: "fine",
                    };
                    css::parser::Stylesheet::parse_with_viewport(&css_text, viewport)
                },
                Err(e) => {
                    log::debug!("external stylesheet fetch failed: {e}");
                    self.pending_external_css_apply = true;
                    continue;
                },
            };
            if idx < self.external_stylesheets.len() {
                self.external_stylesheets[idx] = Some(sheet);
                self.pending_external_css_apply = true;
            }
        }
    }

    /// Process a loaded resource response.
    pub fn process_response(&mut self, response: ResourceResponse) {
        let url = response.url.clone();
        let content_type = response.content_type;

        // Cache the response first (clone on PSP, move-after-borrow on desktop).
        self.cache.insert(
            url.clone(),
            CacheEntry {
                response: response.clone(),
                texture: None,
                etag: None,
                last_modified: None,
            },
        );

        match content_type {
            ContentType::Html | ContentType::PlainText | ContentType::Unknown => {
                let body = String::from_utf8_lossy(&response.body);
                self.load_html(&body, &url);
            },
            ContentType::GeminiText => {
                let body = String::from_utf8_lossy(&response.body);
                self.load_gemini(&body, &url);
            },
            ContentType::Css => {
                let wrapped = format!(
                    "<html><body><pre>{}</pre></body></html>",
                    String::from_utf8_lossy(&response.body)
                );
                self.load_html(&wrapped, &url);
            },
            _ if content_type.is_image() => {
                // Decode the bytes we already have so the wrapping
                // `<img>` doesn't trigger a second network round-trip
                // (and inherit any cookie / redirect / cache failure
                // path that the page-load fetch already cleared).
                if let Some(decoded) = crate::image::decode_image(&response.body) {
                    let img_bytes = decoded.width as usize * decoded.height as usize * 4;
                    // Evict oldest decoded images if over budget so the
                    // byte counter tracks actual resident memory.
                    while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                        if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                            if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                                let evicted_bytes =
                                    evicted.width as usize * evicted.height as usize * 4;
                                self.decoded_image_bytes -= evicted_bytes;
                                self.mask_image_arcs.remove(&evict_url);
                                self.image_info_dirty = true;
                            }
                        } else {
                            break;
                        }
                    }
                    self.decoded_image_bytes += img_bytes;
                    self.decoded_image_lru.push_front(url.clone());
                    self.decoded_images.insert(url.clone(), decoded);
                    self.image_info_dirty = true;
                } else {
                    self.broken_image_urls.insert(url.clone());
                }
                let mut escaped = String::with_capacity(url.len());
                push_escaped(&mut escaped, &url);
                let wrapped = format!(
                    "<!DOCTYPE html><html><head><title>{escaped}</title><style>\
                     html,body{{margin:0;padding:0;background:#1f1f1f;}}\
                     .image-frame{{text-align:center;padding:8px 0;}}\
                     img{{max-width:100%;display:inline-block;}}\
                     </style></head><body>\
                     <div class=\"image-frame\">\
                     <img src=\"{escaped}\" alt=\"{escaped}\">\
                     </div></body></html>",
                );
                self.load_html(&wrapped, &url);
            },
            _ => {
                let wrapped = format!(
                    "<html><body><p>Cannot display \
                     content type: {:?}</p></body></html>",
                    content_type
                );
                self.load_html(&wrapped, &url);
            },
        }
    }

    /// Parse HTML, run the CSS cascade, build layout, and prepare
    /// for painting.
    /// Maximum HTML source size accepted for parsing (10 MB).
    const MAX_HTML_SOURCE_BYTES: usize = 10 * 1024 * 1024;

    pub fn load_html(&mut self, html_source: &str, url: &str) {
        self.diag(&format!(
            "[BR] load_html start: {} bytes",
            html_source.len()
        ));
        // Guard against oversized HTML input.
        let source = if html_source.len() > Self::MAX_HTML_SOURCE_BYTES {
            log::warn!(
                "HTML source too large ({} bytes), truncating to {} bytes",
                html_source.len(),
                Self::MAX_HTML_SOURCE_BYTES,
            );
            &html_source[..html_source.floor_char_boundary(Self::MAX_HTML_SOURCE_BYTES)]
        } else {
            html_source
        };

        // 1. Tokenize and build DOM, reusing the previous document's
        //    arena allocation when available to avoid reallocations.
        self.diag("[BR] html tokenize start");
        let tokens = html::tokenizer::Tokenizer::new(source).tokenize();
        self.diag(&format!("[BR] html tokenize done: {} tokens", tokens.len()));
        self.diag("[BR] tree build start");
        let doc = if let Some(old_doc) = self.document.take() {
            html::tree_builder::TreeBuilder::build_reuse(tokens, old_doc)
        } else {
            html::tree_builder::TreeBuilder::build(tokens)
        };
        self.diag(&format!("[BR] tree build done: {} nodes", doc.nodes.len()));

        // 1b. Execute inline <script> blocks (if JS enabled).
        //
        // The document is wrapped in Rc<RefCell<>> so JS closures can
        // mutate it. After the engine is dropped (freeing all JS-side
        // Rc clones), we unwrap the Rc to recover an owned Document.
        #[cfg(feature = "javascript")]
        {
            self.console_output.clear();
            // Drop any previously retained engine before loading a new page.
            self.js_engine = None;
            self.js_doc = None;
        }
        #[cfg(feature = "javascript")]
        let doc = {
            let (scripts, deferred) = Self::collect_scripts(&doc);
            self.deferred_scripts = deferred;
            let shared: js_dom::SharedDoc = std::rc::Rc::new(std::cell::RefCell::new(doc));
            match oasis_js::JsEngine::new(8 * 1024 * 1024) {
                Ok(engine) => {
                    let s = std::rc::Rc::clone(&shared);
                    let nav = std::rc::Rc::clone(&self.js_nav_actions);
                    let js_sty = std::rc::Rc::clone(&self.js_styles);
                    let ls = std::rc::Rc::clone(&self.js_local_storage);
                    let dirty = std::rc::Rc::clone(&self.js_dom_dirty);
                    // Reset between page loads so a previous page's
                    // mutation doesn't force an extra relayout here.
                    dirty.set(false);
                    if let Err(e) = engine.with_context(|ctx| {
                        js_dom::install_document_global_with_csp(
                            &ctx,
                            &s,
                            url,
                            &nav,
                            &js_sty,
                            self.page_csp.as_ref(),
                            Some(&ls),
                            Some(&dirty),
                        )
                    }) {
                        log::warn!("JS DOM install failed: {}", e.message);
                        self.record_error(
                            crate::BrowserErrorKind::Script,
                            format!("JS DOM install: {}", e.message),
                        );
                    }
                    // Install canvas 2D context bindings.
                    #[cfg(feature = "canvas")]
                    {
                        let cm = std::rc::Rc::clone(&self.canvas_states);
                        if let Err(e) =
                            engine.with_context(|ctx| js_dom::install_canvas_bindings(&ctx, &cm))
                        {
                            log::warn!("Canvas bindings install failed: {}", e.message);
                        }
                    }
                    if !scripts.is_empty() {
                        let script_refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
                        engine.eval_all(&script_refs);
                    }
                    // Install site-compat shims (togglecomment, etc.)
                    // before wiring inline handlers, so onclick bodies
                    // that reference them find defined globals. User
                    // scripts already ran and can override if they
                    // defined their own versions.
                    js_dom::install_site_compat_shims(&engine);
                    // Register inline event handlers (onclick, etc.)
                    // after scripts so Element class is available.
                    {
                        let doc_borrow = shared.borrow();
                        js_dom::register_inline_handlers(&engine, &doc_borrow);
                    }
                    // Fire DOMContentLoaded event.
                    let _ = engine.eval(
                        "if (typeof document !== 'undefined' && document.dispatchEvent) { \
                         document.dispatchEvent(new Event('DOMContentLoaded')); \
                         }",
                    );
                    self.console_output = engine.console_output();
                    // Retain engine + shared doc for event dispatch.
                    self.js_engine = Some(engine);
                    self.js_doc = Some(std::rc::Rc::clone(&shared));
                },
                Err(e) => {
                    log::warn!("JS engine init failed: {}", e.message);
                    self.record_error(
                        crate::BrowserErrorKind::Script,
                        format!("JS engine init: {}", e.message),
                    );
                },
            }
            // Try to take ownership without cloning. This succeeds when
            // no JS engine was retained (init failure or no scripts).
            // When the engine holds a clone via js_doc, fall back to clone.
            match std::rc::Rc::try_unwrap(shared) {
                Ok(cell) => cell.into_inner(),
                Err(shared) => shared.borrow().clone(),
            }
        };

        // 2. Extract page title.
        let title = doc.title().unwrap_or_else(|| url.to_string());

        // 3. Collect <style> blocks and inline style="" attributes from DOM.
        //    Cache them so hover restyles don't re-parse.
        self.diag("[BR] collect stylesheets start");
        let media_viewport = css::parser::MediaViewport {
            width: self.window_w as f32,
            height: self.window_h as f32,
            dark_mode: false,
            prefers_reduced_motion: false,
            hover: true,
            pointer: "fine",
        };
        let (author_sheets, author_sheet_positions) =
            Self::collect_style_sheets(&doc, media_viewport);
        let inline_styles = Self::collect_inline_styles(&doc);
        self.diag(&format!(
            "[BR] collect stylesheets done: {} sheets, {} inline",
            author_sheets.len(),
            inline_styles.len()
        ));

        // 3b. Collect `<link rel="stylesheet" href>` URLs and dispatch
        //     async fetches. The initial render applies only inline
        //     `<style>` sheets so the page appears immediately; external
        //     sheets are re-cascaded and relaid out as they arrive. This
        //     is the primary reason old.reddit.com renders as unstyled
        //     bullets without this pass — the site ships essentially no
        //     inline CSS.
        let (linked_urls, linked_positions) = Self::collect_linked_stylesheet_urls(&doc, url);
        self.external_stylesheets = vec![None; linked_urls.len()];
        self.external_stylesheet_positions = linked_positions;
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        {
            self.pending_io_stylesheets.clear();
            self.pending_vfs_stylesheets.clear();
        }
        self.pending_external_css_apply = false;
        if !linked_urls.is_empty() {
            self.submit_external_stylesheets(linked_urls, url);
        }

        // 4. CSS cascade: user-agent + author stylesheets + inline styles.
        self.diag("[BR] cascade start");
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in &author_sheets {
            all_sheets.push(sheet);
        }
        let mut styles = {
            let ctx = css::cascade::CascadeContext {
                hover_node: self.hover_node,
                visited_urls: Some(&self.visited_urls),
                focused_node: None,
                containers: None,
                global_layers: None,
            };
            css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx)
        };
        self.diag("[BR] selector index start");
        let selector_index = css::cascade::SelectorIndex::build(&all_sheets);
        self.diag(&format!("[BR] cascade done: {} styles", styles.len()));

        // 4b. Web fonts: collect @font-face rules from stylesheets.
        //     Actual font data loading happens in `load_web_fonts()`
        //     which is called with VFS access before the first paint.
        #[cfg(feature = "web-fonts")]
        {
            let mut font_reg = self.font_registry.borrow_mut();
            // Clear previous page's fonts.
            *font_reg = crate::font::FontRegistry::new();
            self.fonts_load_attempted = false;

            // Collect @font-face rules from all stylesheets.
            for sheet in &all_sheets {
                font_reg.collect_font_faces(&sheet.font_faces);
            }
        }

        // 5. Build link href map from DOM.
        let href_map = Self::build_link_map(&doc);

        // 6. Build layout tree.
        self.diag("[BR] layout start");
        let content_h = self.config.content_height(self.window_h);
        self.refresh_image_info();
        let shared = std::rc::Rc::clone(&self.text_cache);
        let measurer =
            layout::text_cache::CachingMeasurer::with_shared(&SimpleTextMeasurer, shared);
        let viewport_w = self.window_w as f32 / self.config.zoom_level;
        let viewport_h = content_h as f32 / self.config.zoom_level;
        let mut layout_root = layout::block::build_layout_tree(
            &doc,
            &styles,
            &measurer,
            viewport_w,
            viewport_h,
            Some(url),
            &self.cached_image_info,
        );
        self.diag("[BR] layout done");

        // 6b. `@container` queries: if any author/UA rule is gated behind
        //     a container condition, do a second cascade+layout pass with
        //     the post-layout container sizes plumbed into cascade. Pages
        //     without container queries skip the work entirely.
        let container_lookup = if css::cascade::stylesheets_use_container_queries(&all_sheets) {
            let first_lookup = css::cascade::build_container_lookup(&layout_root);
            if !first_lookup.is_empty() {
                self.diag("[BR] container-query restyle");
                let ctx = css::cascade::CascadeContext {
                    hover_node: self.hover_node,
                    visited_urls: Some(&self.visited_urls),
                    focused_node: None,
                    containers: Some(&first_lookup),
                    global_layers: None,
                };
                styles = css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx);
                layout_root = layout::block::build_layout_tree(
                    &doc,
                    &styles,
                    &measurer,
                    viewport_w,
                    viewport_h,
                    Some(url),
                    &self.cached_image_info,
                );
                // Rebuild from the post-second-layout tree so the
                // cached lookup reflects the *final* container sizes.
                // Hover/focus restyles reuse this and would otherwise
                // see stale pre-restyle dimensions for nested
                // container-query cases.
                Some(css::cascade::build_container_lookup(&layout_root))
            } else {
                Some(first_lookup)
            }
        } else {
            None
        };

        // Update shared computed styles for JS getComputedStyle().
        #[cfg(feature = "javascript")]
        {
            *self.js_styles.borrow_mut() = styles.clone();
        }

        // 4b. Register CSS animations with the animation engine using
        //     the final styles (after any container-query restyle).
        {
            let mut all_keyframes: Vec<&css::parser::KeyframesRule> = Vec::new();
            all_keyframes.extend(ua_sheet.keyframes.iter());
            for sheet in &author_sheets {
                all_keyframes.extend(sheet.keyframes.iter());
            }
            self.animation_engine = css::animation::AnimationEngine::new();
            for (node_id, maybe_style) in styles.iter().enumerate() {
                if let Some(computed) = maybe_style
                    && !computed.animations.is_empty()
                {
                    let kf_owned: Vec<css::parser::KeyframesRule> =
                        all_keyframes.iter().copied().cloned().collect();
                    self.animation_engine.start_animations(
                        node_id,
                        &computed.animations,
                        &kf_owned,
                    );
                }
            }
        }
        drop(all_sheets);

        // Cache parsed sheets and selector index for hover restyles.
        self.cached_author_sheets = author_sheets;
        self.cached_author_sheet_positions = author_sheet_positions;
        self.cached_inline_styles = inline_styles;
        self.cached_selector_index = Some(selector_index);
        self.container_lookup = container_lookup;

        // 7a. Collect canvas states from layout tree.
        #[cfg(feature = "javascript")]
        {
            self.canvas_states.borrow_mut().clear();
            crate::canvas::collect_canvas_states(&layout_root, &self.canvas_states);
        }

        // 7. Store results.
        self.body_node_id = doc.body();
        self.document = Some(doc);
        self.styles = styles;
        self.href_map = href_map;
        self.layout_root = Some(layout_root);
        self.link_map.clear();

        // 7b. Rebuild form state from the fresh DOM. Without this
        // `form_manager.forms` stays empty and every click on a
        // `<form>`'s inputs silently fails to find an owning form,
        // which means keystrokes never land in the search box.
        self.form_manager.clear();
        if let Some(doc) = &self.document {
            Self::populate_forms_from_dom(doc, &mut self.form_manager);
        }
        self.scroll.reset();
        self.nested_scroll_offsets.clear();
        self.state = LoadingState::Idle;
        self.layout_dirty = false;
        self.last_layout_w = self.window_w;
        // Invalidate the cached display list so it gets rebuilt on next paint.
        self.display_list.clear();

        // 8. Update navigation (skip if restoring from history).
        if !self.skip_nav_push {
            self.nav.navigate(url, &title);
        }
        self.skip_nav_push = false;
        self.diag("[BR] load_html done");
    }

    /// Walk the DOM and register every `<form>` (plus its descendant
    /// inputs / selects / textareas / buttons) with `form_manager`.
    ///
    /// This is what makes a newly-loaded page actually interactive:
    /// the click handler and keyboard router both look up whether an
    /// element belongs to a registered form, and without this pass
    /// they always miss.
    fn populate_forms_from_dom(
        doc: &html::dom::Document,
        form_manager: &mut crate::forms::FormManager,
    ) {
        use crate::forms::{FormElement, FormMethod, InputType, SelectOption};
        use html::dom::{NodeKind, TagName};

        // Find every <form>. Nested <form> elements are disallowed in
        // HTML5, so a flat walk is sufficient.
        for (form_nid, form_node) in doc.nodes.iter().enumerate() {
            let NodeKind::Element(form_elem) = &form_node.kind else {
                continue;
            };
            if form_elem.tag != TagName::Form {
                continue;
            }

            let action = form_elem.get_attribute("action").unwrap_or("").to_string();
            let method = match form_elem
                .get_attribute("method")
                .unwrap_or("get")
                .to_ascii_lowercase()
                .as_str()
            {
                "post" => FormMethod::Post,
                _ => FormMethod::Get,
            };
            let form_id = form_manager.add_form(&action, method);

            // DFS the form's subtree collecting form-element descendants.
            // Reverse the initial children so `pop()` yields them in
            // document order (matches the inner `iter().rev()` push).
            // Without this, direct <form> children are visited in reverse,
            // making submitted form data appear in reverse document order.
            let mut stack: Vec<usize> = form_node.children.iter().rev().copied().collect();
            while let Some(nid) = stack.pop() {
                let node = &doc.nodes[nid];
                for &c in node.children.iter().rev() {
                    stack.push(c);
                }
                let NodeKind::Element(elem) = &node.kind else {
                    continue;
                };
                match elem.tag {
                    TagName::Input => {
                        let input_type = elem
                            .get_attribute("type")
                            .unwrap_or("text")
                            .to_ascii_lowercase();
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        let value = elem.get_attribute("value").unwrap_or("").to_string();
                        let placeholder =
                            elem.get_attribute("placeholder").unwrap_or("").to_string();
                        let maxlength = elem
                            .get_attribute("maxlength")
                            .and_then(|v| v.parse::<usize>().ok());
                        let required = elem.get_attribute("required").is_some();

                        match input_type.as_str() {
                            "hidden" => {
                                form_manager
                                    .add_element(form_id, FormElement::HiddenInput { name, value });
                            },
                            "submit" => {
                                let label = if value.is_empty() {
                                    "Submit".to_string()
                                } else {
                                    value.clone()
                                };
                                form_manager.add_element(
                                    form_id,
                                    FormElement::SubmitButton { name, value, label },
                                );
                            },
                            "reset" => {
                                let label = if value.is_empty() {
                                    "Reset".to_string()
                                } else {
                                    value.clone()
                                };
                                form_manager
                                    .add_element(form_id, FormElement::ResetButton { label });
                            },
                            "button" => {
                                // Plain push button — no submission. Treat
                                // like SubmitButton for focus purposes but
                                // don't submit on click.
                            },
                            "checkbox" => {
                                let checked = elem.get_attribute("checked").is_some();
                                let label = String::new();
                                form_manager.add_element(
                                    form_id,
                                    FormElement::Checkbox {
                                        name,
                                        value,
                                        checked,
                                        label,
                                    },
                                );
                            },
                            "radio" => {
                                let checked = elem.get_attribute("checked").is_some();
                                form_manager.add_element(
                                    form_id,
                                    FormElement::RadioButton {
                                        name: name.clone(),
                                        value,
                                        checked,
                                        group: name,
                                    },
                                );
                            },
                            _ => {
                                // text / search / email / password / number
                                // and unknown types collapse to TextInput so
                                // the user can at least type into them.
                                let it = match input_type.as_str() {
                                    "password" => InputType::Password,
                                    "email" => InputType::Email,
                                    "number" => InputType::Number,
                                    _ => InputType::Text,
                                };
                                form_manager.add_element(
                                    form_id,
                                    FormElement::TextInput {
                                        name,
                                        value,
                                        placeholder,
                                        maxlength,
                                        input_type: it,
                                        required,
                                        minlength: elem
                                            .get_attribute("minlength")
                                            .and_then(|v| v.parse::<usize>().ok()),
                                        pattern: elem
                                            .get_attribute("pattern")
                                            .map(|s| s.to_string()),
                                        min: elem
                                            .get_attribute("min")
                                            .and_then(|v| v.parse::<f64>().ok()),
                                        max: elem
                                            .get_attribute("max")
                                            .and_then(|v| v.parse::<f64>().ok()),
                                    },
                                );
                            },
                        }
                    },
                    TagName::Textarea => {
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        // `<textarea>`'s initial value is its text children.
                        let value = doc.text_content(nid);
                        let placeholder =
                            elem.get_attribute("placeholder").unwrap_or("").to_string();
                        let rows = elem
                            .get_attribute("rows")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(3);
                        let cols = elem
                            .get_attribute("cols")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(40);
                        let required = elem.get_attribute("required").is_some();
                        form_manager.add_element(
                            form_id,
                            FormElement::TextArea {
                                name,
                                value,
                                rows,
                                cols,
                                placeholder,
                                required,
                                minlength: elem
                                    .get_attribute("minlength")
                                    .and_then(|v| v.parse::<usize>().ok()),
                                maxlength: elem
                                    .get_attribute("maxlength")
                                    .and_then(|v| v.parse::<usize>().ok()),
                            },
                        );
                    },
                    TagName::Select => {
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        let mut options = Vec::new();
                        let mut selected_index = None;
                        // Walk every descendant of the <select> so
                        // <option> elements wrapped in <optgroup> are
                        // collected too. `<optgroup>` is parsed as
                        // `TagName::Unknown("optgroup")` since the
                        // enum has no dedicated variant. We push
                        // children in reverse onto the stack so the
                        // DFS yields them back in document order,
                        // which keeps `selected_index` stable.
                        let mut opt_stack: Vec<usize> =
                            node.children.iter().rev().copied().collect();
                        let mut ordered: Vec<usize> = Vec::new();
                        while let Some(nid) = opt_stack.pop() {
                            ordered.push(nid);
                            for &c in doc.nodes[nid].children.iter().rev() {
                                opt_stack.push(c);
                            }
                        }
                        for opt_id in ordered {
                            if let NodeKind::Element(ref opt) = doc.nodes[opt_id].kind
                                && opt.tag == TagName::Option
                            {
                                let value = opt
                                    .get_attribute("value")
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| doc.text_content(opt_id));
                                let label = doc.text_content(opt_id);
                                let disabled = opt.get_attribute("disabled").is_some();
                                if opt.get_attribute("selected").is_some() {
                                    selected_index = Some(options.len());
                                }
                                options.push(SelectOption {
                                    value,
                                    label,
                                    disabled,
                                });
                            }
                        }
                        if selected_index.is_none() && !options.is_empty() {
                            selected_index = Some(0);
                        }
                        form_manager.add_element(
                            form_id,
                            FormElement::SelectBox {
                                name,
                                options,
                                selected_index,
                                open: false,
                            },
                        );
                    },
                    TagName::Button => {
                        let btype = elem
                            .get_attribute("type")
                            .unwrap_or("submit")
                            .to_ascii_lowercase();
                        match btype.as_str() {
                            "submit" => {
                                let name = elem.get_attribute("name").unwrap_or("").to_string();
                                let value = elem.get_attribute("value").unwrap_or("").to_string();
                                let label = doc.text_content(nid);
                                form_manager.add_element(
                                    form_id,
                                    FormElement::SubmitButton { name, value, label },
                                );
                            },
                            "reset" => {
                                let text = doc.text_content(nid);
                                let label = if text.is_empty() {
                                    "Reset".to_string()
                                } else {
                                    text
                                };
                                form_manager
                                    .add_element(form_id, FormElement::ResetButton { label });
                            },
                            _ => {},
                        }
                    },
                    _ => {},
                }
            }
            let _ = form_nid;
        }
    }

    /// Walk the DOM to build a map of `<a>` element NodeIds to their
    /// `href` attribute values.
    pub fn build_link_map(doc: &html::dom::Document) -> HashMap<NodeId, String> {
        let mut map = HashMap::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::A
                && let Some(href) = elem.get_attribute("href")
            {
                map.insert(id, href.to_string());
            }
        }
        map
    }

    /// Load pending web fonts and resolve `web_font_id` on styles.
    ///
    /// Called from `tick()` when VFS access is available. This triggers
    /// the actual network/VFS fetch of font data, parses it with fontdue,
    /// and then resolves `web_font_id` on each `ComputedStyle`. If any
    /// fonts are loaded, the layout and display list are rebuilt.
    #[cfg(feature = "web-fonts")]
    pub(crate) fn load_web_fonts(&mut self, vfs: &dyn oasis_vfs::Vfs) {
        // Only attempt font loading once per page. After the first call,
        // `pending` is drained inside `load_fonts`, so repeated calls
        // would be harmless but wasteful no-ops.
        if self.fonts_load_attempted || self.font_registry.borrow().has_fonts() {
            return;
        }
        self.fonts_load_attempted = true;

        let base_url = self.nav.current_url().map(|s| s.to_string());
        {
            let mut reg = self.font_registry.borrow_mut();
            reg.load_fonts(base_url.as_deref(), vfs, self.tls.as_deref());
        }

        // If fonts were loaded, resolve web_font_id on styles and
        // trigger relayout.
        let font_count = self.font_registry.borrow().font_count();
        if font_count > 0 {
            self.diag(&format!("[BR] web fonts loaded: {} faces", font_count));
            // Resolve web_font_id on cached styles.
            let reg = self.font_registry.borrow();
            for style in self.styles.iter_mut().flatten() {
                let italic = style.font_style == crate::css::values::FontStyle::Italic;
                let weight = style.font_weight.0;
                if let Some(font_id) = reg.resolve_font(&style.font_family.families, weight, italic)
                {
                    style.web_font_id = Some(font_id.as_raw());
                }
            }
            drop(reg);
            // Force display list rebuild on next paint.
            self.layout_dirty = true;
        }
    }

    /// Walk the DOM to collect text from `<style>` elements and parse
    /// each into a `Stylesheet`. Both `<head>` and `<body>` style blocks
    /// are included.
    ///
    /// `viewport` is threaded into `Stylesheet::parse_with_viewport` so
    /// `@media (min/max-width)` and `@media (prefers-color-scheme)` gate
    /// against the actual window size rather than the 480x272 default —
    /// without this, desktop-sized windows silently get the mobile
    /// breakpoints of any page whose author CSS uses `max-width`.
    fn collect_style_sheets(
        doc: &html::dom::Document,
        viewport: css::parser::MediaViewport,
    ) -> (Vec<css::parser::Stylesheet>, Vec<html::dom::NodeId>) {
        let mut sheets = Vec::new();
        let mut positions = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::Style
            {
                let css_text = doc.text_content(id);
                if !css_text.is_empty() {
                    sheets.push(css::parser::Stylesheet::parse_with_viewport(
                        &css_text, viewport,
                    ));
                    positions.push(id);
                }
            }
        }
        (sheets, positions)
    }

    /// Return `true` if a single comma-separated media-query token
    /// (e.g. `print`, `only print`, `print and (color)`) targets the
    /// print medium exclusively. Used by `<link media="…">` filtering
    /// to drop print-only sheets without swallowing mixed lists like
    /// `print, screen` or queries like `(min-width: 500px)`.
    pub(crate) fn is_print_only_media_query(token: &str) -> bool {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return false;
        }
        // Drop a leading `only` / `not` modifier before inspecting the
        // media type. `not print` is *not* print-only — it matches
        // everything except print — so treat it as non-print.
        let rest = if let Some(r) = trimmed.strip_prefix("only ") {
            r.trim_start()
        } else if let Some(r) = trimmed.strip_prefix("not ") {
            // Only bare `not all` matches zero media types. A query of the
            // form `not all and (feature)` matches media where `(feature)`
            // is false (e.g. `not all and (color)` matches monochrome
            // screens) so must not be filtered here. Any other `not <type>`
            // (e.g. `not screen`, `not print`) still matches *some* media.
            return r.trim() == "all";
        } else {
            trimmed
        };
        // The media type is the first whitespace-delimited word.
        let media_type = rest.split_whitespace().next().unwrap_or("");
        media_type == "print"
    }

    /// Walk the DOM and collect `<link rel="stylesheet" href="...">`
    /// URLs, resolved against `base_url`.
    ///
    /// Skips `rel="alternate stylesheet"` and links whose `media` attr
    /// is a `print`-only query (the layout pipeline renders with
    /// `@media screen` semantics). Data URIs are rejected here because
    /// the async loader expects an HTTP/VFS fetchable resource; inline
    /// `data:` stylesheets should use a `<style>` block anyway.
    ///
    /// The returned vector preserves DOM order so the cascade applies
    /// external sheets in the same sequence the author wrote them. The
    /// parallel `Vec<NodeId>` gives each retained URL its originating
    /// DOM node index so callers can interleave external sheets with
    /// inline `<style>` blocks by DOM order in the cascade.
    fn collect_linked_stylesheet_urls(
        doc: &html::dom::Document,
        base_url: &str,
    ) -> (Vec<String>, Vec<html::dom::NodeId>) {
        const MAX_LINKED_STYLESHEETS: usize = 16;
        let mut urls = Vec::new();
        let mut positions = Vec::new();
        let base_parsed = loader::Url::parse(base_url);
        for (node_id, node) in doc.nodes.iter().enumerate() {
            if urls.len() >= MAX_LINKED_STYLESHEETS {
                log::warn!(
                    "external stylesheet limit ({MAX_LINKED_STYLESHEETS}) \
                     reached; remaining <link rel=\"stylesheet\"> tags dropped"
                );
                break;
            }
            let html::dom::NodeKind::Element(elem) = &node.kind else {
                continue;
            };
            if elem.tag != html::dom::TagName::Link {
                continue;
            }
            let rel = elem.get_attribute("rel").unwrap_or("");
            let is_stylesheet = rel
                .split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"));
            let is_alternate = rel
                .split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("alternate"));
            if !is_stylesheet || is_alternate {
                continue;
            }
            // Skip print-only stylesheets. `media` is a comma-separated
            // list of media queries; if *every* entry targets `print`
            // exclusively, the sheet is irrelevant to screen rendering.
            // A mixed list like `print, screen` still matches screen and
            // must not be skipped.
            if let Some(media) = elem.get_attribute("media") {
                let m = media.trim().to_ascii_lowercase();
                if !m.is_empty() && m.split(',').all(Self::is_print_only_media_query) {
                    continue;
                }
            }
            let Some(href) = elem.get_attribute("href") else {
                continue;
            };
            let href = href.trim();
            if href.is_empty() || href.starts_with("data:") {
                continue;
            }
            let resolved = match &base_parsed {
                Some(base) => base
                    .resolve(href)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string()),
                None => href.to_string(),
            };
            // Reject resolved URLs the loader can't actually fetch.
            // `file://` (used as the base for local fixture pages) can't
            // follow protocol-relative `//cdn/...` references since the
            // network loader rejects the `file` scheme. Silently drop
            // them rather than spamming the I/O thread with certain-fail
            // requests.
            let scheme = loader::Url::parse(&resolved)
                .map(|u| u.scheme.clone())
                .unwrap_or_default();
            match scheme.as_str() {
                "http" | "https" | "vfs" => {},
                _ => continue,
            }
            if urls.iter().any(|u| u == &resolved) {
                continue;
            }
            urls.push(resolved);
            positions.push(node_id);
        }
        (urls, positions)
    }

    /// Submit each linked stylesheet URL to the I/O thread for fetch.
    /// Each slot in `external_stylesheets` is keyed by its index in the
    /// submitted vec; the `poll_io_thread` handler fills the slot when
    /// the CSS bytes arrive.
    ///
    /// VFS-scheme stylesheets (`vfs://…/foo.css`) are loaded synchronously
    /// here rather than round-tripped through the I/O thread — that thread
    /// doesn't hold a VFS handle, so routing a VFS request through it
    /// would always return "unsupported network scheme: vfs" and the
    /// slot would never fill.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    fn submit_external_stylesheets(&mut self, urls: Vec<String>, base_url: &str) {
        let source = if self.config.features.sandbox_only {
            ResourceSource::Vfs
        } else {
            ResourceSource::VfsThenNetwork
        };
        let referrer = loader::strip_referrer(base_url);
        let base = Some(base_url.to_string());
        let mut pending_network: Vec<(usize, ResourceRequest)> = Vec::new();
        for (idx, url) in urls.into_iter().enumerate() {
            let request = ResourceRequest {
                url: url.clone(),
                base_url: base.clone(),
                source,
                method: crate::loader::HttpMethod::Get,
                body: None,
                referrer: referrer.clone(),
            };
            if url.starts_with("vfs://") {
                // VFS lookups require the caller's `Vfs` handle, which
                // we don't hold during `load_html`. Stash them for the
                // next `tick()` to resolve synchronously against the
                // passed-in `vfs`.
                self.pending_vfs_stylesheets.push((idx, request));
                continue;
            }
            pending_network.push((idx, request));
        }

        if !pending_network.is_empty() {
            self.ensure_io_thread();
            if self.io_thread.is_none() {
                log::warn!(
                    "I/O thread unavailable; dropping {} external stylesheet fetch(es). \
                     Page will render without linked CSS.",
                    pending_network.len()
                );
                return;
            }
            // Safe: the `is_none()` check above early-returned, so the
            // I/O thread is guaranteed present for the rest of this call.
            let io = self
                .io_thread
                .as_mut()
                .expect("io_thread checked non-None above");
            for (idx, request) in pending_network {
                let validators = self.cache.peek_validators(&request.url);
                let id = io.send(IoRequestKind::Stylesheet, request, validators, None);
                self.pending_io_stylesheets.insert(id, idx);
            }
        }
    }

    /// WASM/PSP stub: no async I/O path, skip external stylesheet loading.
    #[cfg(any(target_arch = "wasm32", feature = "psp"))]
    fn submit_external_stylesheets(&mut self, _urls: Vec<String>, _base_url: &str) {}

    /// Drain any VFS-scheme stylesheets queued during `load_html` and
    /// parse them against the caller-supplied VFS. Called from `tick()`
    /// so the initial paint can land without waiting.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    pub(crate) fn load_pending_vfs_stylesheets(&mut self, vfs: &dyn Vfs) {
        if self.pending_vfs_stylesheets.is_empty() {
            return;
        }
        let viewport = css::parser::MediaViewport {
            width: self.window_w as f32,
            height: self.window_h as f32,
            dark_mode: false,
            prefers_reduced_motion: false,
            hover: true,
            pointer: "fine",
        };
        let drained: Vec<(usize, ResourceRequest)> =
            std::mem::take(&mut self.pending_vfs_stylesheets);
        for (idx, request) in drained {
            match loader::vfs::load_from_vfs(vfs, &request) {
                Ok(resp) => {
                    let css_text = String::from_utf8_lossy(&resp.body).into_owned();
                    let sheet = css::parser::Stylesheet::parse_with_viewport(&css_text, viewport);
                    if idx < self.external_stylesheets.len() {
                        self.external_stylesheets[idx] = Some(sheet);
                        self.pending_external_css_apply = true;
                    }
                },
                Err(e) => {
                    log::debug!("vfs stylesheet fetch failed: {e}");
                },
            }
        }
    }

    /// Interleave cached inline `<style>` sheets and fetched external
    /// `<link>` sheets back into DOM order. Cascade precedence depends
    /// on source position within the author origin, so a page with a
    /// `<link>` before a `<style>` in `<head>` must apply the link's
    /// rules first — grouping all inline sheets before all external
    /// sheets (or vice-versa) would flip the winner for any rule of
    /// equal specificity.
    fn author_sheets_in_dom_order(&self) -> Vec<&css::parser::Stylesheet> {
        let inline_n = self.cached_author_sheets.len();
        let external_n = self.external_stylesheets.len();
        let mut out: Vec<&css::parser::Stylesheet> = Vec::with_capacity(inline_n + external_n);
        let mut i = 0; // inline cursor
        let mut e = 0; // external cursor
        while i < inline_n || e < external_n {
            let inline_pos = self
                .cached_author_sheet_positions
                .get(i)
                .copied()
                .unwrap_or(usize::MAX);
            let external_pos = self
                .external_stylesheet_positions
                .get(e)
                .copied()
                .unwrap_or(usize::MAX);
            if inline_pos <= external_pos {
                if i < inline_n {
                    out.push(&self.cached_author_sheets[i]);
                }
                i += 1;
            } else {
                if let Some(Some(sheet)) = self.external_stylesheets.get(e) {
                    out.push(sheet);
                }
                e += 1;
            }
        }
        out
    }

    /// Apply any external stylesheets that have arrived since the last
    /// cascade. Rebuilds `self.styles` from UA + cached inline sheets +
    /// arrived external sheets and marks the layout dirty so the next
    /// paint picks up the new visual state.
    ///
    /// Re-runs cascade only — keyframes, @font-face, animations, and
    /// the scripts pipeline are left alone; style changes alone are
    /// sufficient for the visual delta that matters for real-world
    /// sites (old.reddit, MediaWiki) whose external CSS is declarative.
    ///
    /// Known limitation: `@import url(...)` rules inside fetched
    /// external CSS are *not* followed. The parser captures them but
    /// this pass does not chase the transitive closure, so pages whose
    /// top-level stylesheet is a thin `@import` shim render with only
    /// the shim's own rules applied. Acceptable for old.reddit /
    /// MediaWiki (top-level sheets carry the rules directly); revisit
    /// if another real-world target relies on `@import` chains.
    pub(crate) fn apply_external_stylesheets_if_pending(&mut self) {
        if !self.pending_external_css_apply {
            return;
        }
        self.pending_external_css_apply = false;
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in self.author_sheets_in_dom_order() {
            all_sheets.push(sheet);
        }
        // Preserve the container-query lookup captured during `load_html`'s
        // post-layout restyle pass; without it, `@container` rules would
        // silently stop matching on every re-cascade driven by a late-
        // arriving external sheet. `global_layers` is intentionally left
        // `None` — layer ordering is recomputed fresh from the merged
        // sheet list below, matching the initial cascade path.
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
            containers: self.container_lookup.as_ref(),
            global_layers: None,
        };
        let styles = css::cascade::style_tree(doc, &all_sheets, &self.cached_inline_styles, &ctx);
        let selector_index = css::cascade::SelectorIndex::build(&all_sheets);

        #[cfg(feature = "web-fonts")]
        {
            let mut font_reg = self.font_registry.borrow_mut();
            for sheet in &all_sheets {
                font_reg.collect_font_faces(&sheet.font_faces);
            }
            self.fonts_load_attempted = false;
        }

        self.styles = styles;
        self.cached_selector_index = Some(selector_index);
        self.layout_dirty = true;
        self.full_repaint_needed = true;
        self.display_list.clear();
    }

    /// Walk the DOM to collect inline `style=""` attributes and parse
    /// each into a list of declarations keyed by NodeId.
    pub(crate) fn collect_inline_styles(
        doc: &html::dom::Document,
    ) -> Vec<(NodeId, Vec<css::parser::Declaration>)> {
        let mut result = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && let Some(style_attr) = elem.get_attribute("style")
            {
                let decls = css::parser::parse_inline_style(style_attr);
                if !decls.is_empty() {
                    result.push((id, decls));
                }
            }
        }
        result
    }

    /// Walk the DOM to collect inline `<script>` text in document order.
    /// External scripts (`<script src="...">`) and non-JavaScript types
    /// (e.g. `application/ld+json`) are skipped.
    ///
    /// Returns `(immediate, deferred)` where `deferred` contains scripts
    /// with the `defer` or `async` attribute, to be executed after the
    /// first paint.
    #[cfg(feature = "javascript")]
    fn collect_scripts(doc: &html::dom::Document) -> (Vec<String>, Vec<String>) {
        let mut immediate = Vec::new();
        let mut deferred = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::Script
                && elem.get_attribute("src").is_none()
                && Self::is_js_script_type(elem.get_attribute("type"))
            {
                let text = doc.text_content(id);
                if !text.is_empty() {
                    let is_deferred = elem.get_attribute("defer").is_some()
                        || elem.get_attribute("async").is_some();
                    if is_deferred {
                        deferred.push(text);
                    } else {
                        immediate.push(text);
                    }
                }
            }
        }
        (immediate, deferred)
    }

    /// Returns `true` if the script `type` attribute indicates JavaScript
    /// (or is absent/empty, which defaults to JS per the HTML spec).
    #[cfg(feature = "javascript")]
    fn is_js_script_type(type_attr: Option<&str>) -> bool {
        match type_attr {
            None | Some("") => true,
            Some(t) => {
                let t = t.trim().to_ascii_lowercase();
                matches!(
                    t.as_str(),
                    "text/javascript"
                        | "application/javascript"
                        | "text/ecmascript"
                        | "application/ecmascript"
                        | "module"
                )
            },
        }
    }

    /// Execute deferred scripts (those with `defer` or `async` attributes).
    ///
    /// Should be called after the first paint so that initial rendering is
    /// not blocked by script execution.
    #[cfg(feature = "javascript")]
    pub fn execute_deferred_scripts(&mut self) {
        if self.deferred_scripts.is_empty() {
            return;
        }
        let scripts = std::mem::take(&mut self.deferred_scripts);
        if let Some(engine) = &self.js_engine {
            let refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
            engine.eval_all(&refs);
            // Check if deferred scripts triggered any DOM mutations that
            // require relayout.
            let fired = engine.tick_timers(0.0);
            if fired > 0 {
                self.layout_dirty = true;
            }
        }
    }

    /// Console output from JavaScript execution on the current page.
    #[cfg(feature = "javascript")]
    pub fn console_output(&self) -> &[oasis_js::ConsoleEntry] {
        &self.console_output
    }

    /// Load and render a Gemini document.
    pub fn load_gemini(&mut self, source: &str, url: &str) {
        let doc = gemini::parser::GeminiDocument::parse(source);
        let title = doc.title().unwrap_or("Gemini page").to_string();

        // Convert to HTML and render through the HTML pipeline.
        let html = gemini_to_html(&doc);
        self.load_html(&html, url);

        // Override the title with the Gemini document title.
        self.nav.update_title(&title);
    }

    /// Toggle reader mode on/off.
    pub fn toggle_reader_mode(&mut self) {
        if !self.config.features.reader_mode {
            return;
        }

        self.reader_mode = !self.reader_mode;
        self.nav.update_reader_mode(self.reader_mode);

        if self.reader_mode {
            // Extract article and re-render.
            if let Some(doc) = &self.document
                && let Some(article) = reader::extract_article(doc)
            {
                self.reader_html = Some(article.html.clone());
                // Re-parse the reader HTML.
                let url = self.nav.current_url().unwrap_or("about:reader").to_string();
                let tokens = html::tokenizer::Tokenizer::new(&article.html).tokenize();
                let reader_doc = html::tree_builder::TreeBuilder::build(tokens);
                let ua_sheet = css::default::default_stylesheet();
                let reader_ctx = css::cascade::CascadeContext {
                    hover_node: None,
                    visited_urls: Some(&self.visited_urls),
                    focused_node: None,
                    containers: None,
                    global_layers: None,
                };
                let styles = css::cascade::style_tree(&reader_doc, &[ua_sheet], &[], &reader_ctx);
                let href_map = Self::build_link_map(&reader_doc);
                self.cached_author_sheets = Vec::new();
                self.cached_author_sheet_positions = Vec::new();
                self.cached_inline_styles = Vec::new();
                self.external_stylesheets = Vec::new();
                self.external_stylesheet_positions = Vec::new();
                self.document = Some(reader_doc);
                self.styles = styles;
                self.href_map = href_map;
                self.layout_root = None;
                self.link_map.clear();
                self.scroll.reset();
                self.selected_link = -1;
                self.nav.update_title(&format!("Reader: {}", article.title));
                let _ = url; // suppress unused warning
            }
        } else {
            // Restore original page by re-navigating.
            self.reader_html = None;
            if let Some(url) = self.nav.current_url() {
                let url = url.to_string();
                // Re-parse original from cache if available.
                if let Some(entry) = self.cache.get(&url) {
                    let body = entry.response.body.clone();
                    let ct = entry.response.content_type;
                    if ct == ContentType::Html || ct == ContentType::PlainText {
                        let text = String::from_utf8_lossy(&body);
                        self.load_html(&text, &url);
                    }
                }
            }
        }
    }
}

/// rendering through the HTML pipeline.
fn gemini_to_html(doc: &gemini::parser::GeminiDocument) -> String {
    let mut html = String::from("<html><head><title>");
    if let Some(title) = doc.title() {
        html.push_str(title);
    }
    html.push_str("</title></head><body>");

    for line in &doc.lines {
        match line {
            gemini::parser::GeminiLine::Text(text) => {
                html.push_str("<p>");
                push_escaped(&mut html, text);
                html.push_str("</p>");
            },
            gemini::parser::GeminiLine::Link { url, display } => {
                html.push_str("<p><a href=\"");
                push_escaped(&mut html, url);
                html.push_str("\">");
                let label = display.as_deref().unwrap_or(url.as_str());
                push_escaped(&mut html, label);
                html.push_str("</a></p>");
            },
            gemini::parser::GeminiLine::Heading1(text) => {
                html.push_str("<h1>");
                push_escaped(&mut html, text);
                html.push_str("</h1>");
            },
            gemini::parser::GeminiLine::Heading2(text) => {
                html.push_str("<h2>");
                push_escaped(&mut html, text);
                html.push_str("</h2>");
            },
            gemini::parser::GeminiLine::Heading3(text) => {
                html.push_str("<h3>");
                push_escaped(&mut html, text);
                html.push_str("</h3>");
            },
            gemini::parser::GeminiLine::ListItem(text) => {
                html.push_str("<li>");
                push_escaped(&mut html, text);
                html.push_str("</li>");
            },
            gemini::parser::GeminiLine::Quote(text) => {
                html.push_str("<blockquote>");
                push_escaped(&mut html, text);
                html.push_str("</blockquote>");
            },
            gemini::parser::GeminiLine::Preformatted { lines, alt_text: _ } => {
                html.push_str("<pre>");
                for (i, pre_line) in lines.iter().enumerate() {
                    if i > 0 {
                        html.push('\n');
                    }
                    push_escaped(&mut html, pre_line);
                }
                html.push_str("</pre>");
            },
            gemini::parser::GeminiLine::Empty => {
                html.push_str("<br>");
            },
        }
    }

    html.push_str("</body></html>");
    html
}

/// Push HTML-escaped text into a string.
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}
