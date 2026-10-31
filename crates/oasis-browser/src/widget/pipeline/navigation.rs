//! Navigation entry points, the IO thread hand-off and response
//! dispatch (`navigate_*`, `reload`, `poll_io_thread`,
//! `process_response`).

use super::*;

impl BrowserWidget {
    /// Navigate via HTTP POST to a URL with the given body.
    ///
    /// Used for `<form method="post">` submissions. The encoded form
    /// data is sent as the request body with
    /// `Content-Type: application/x-www-form-urlencoded`.
    pub fn navigate_post(&mut self, url: &str, body: Vec<u8>, vfs: &dyn Vfs) {
        self.reset_for_navigation();
        self.skip_nav_push = false;

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
            // A form post is always page-initiated: cross-site, it gets
            // no `SameSite=Lax` / `Strict` cookies (CSRF).
            initiator: self.nav.current_url().map(String::from),
        };

        // POST requests always go to the network, so offload to IO thread.
        #[cfg(native_net)]
        {
            // `ensure_io_thread` fails only if the OS refuses to spawn a
            // thread; fall through to the synchronous load in that case.
            if self.should_use_io_thread(&request, vfs) && self.ensure_io_thread() {
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

        // Check if we have a cached response for this URL. The fragment
        // is not part of the resource's identity.
        let key = match url.split_once('#') {
            Some((base, _)) if self.cache.get(url).is_none() => base,
            _ => url,
        };
        if let Some(entry) = self.cache.get(key) {
            let body = entry.response.body.clone();
            let ct = entry.response.content_type;
            let csp = entry.csp.clone();
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
                // The re-render runs the page's scripts again: under the
                // policy it was served with, not none.
                self.page_csp = csp;
                self.page_errors.clear();
                self.decoded_images.clear();
                self.mask_image_arcs.clear();
                self.retire_image_textures();
                self.pending_images.clear();
                self.decoded_image_bytes = 0;
                self.decoded_image_lru.clear();
                self.cached_image_info.clear();
                self.image_info_dirty = false;

                let text = html_for_text_body(ct, &body);
                self.load_html(&text, url);

                self.collect_page_image_requests();
                if !self.pending_images.is_empty() {
                    self.state = LoadingState::Loading;
                }
                return;
            }
        }

        // Cache miss or non-HTML content — fetch from network. The
        // history entry already moved (back/forward), so the load must
        // not push a new one: that would clear the forward stack.
        self.navigate_vfs_inner(url, vfs, false);
    }

    /// Reload the current page from its source (VFS or network),
    /// keeping the history entry and the scroll position.
    pub fn reload(&mut self, vfs: &dyn Vfs) {
        let Some(url) = self.nav.current_url().map(str::to_string) else {
            return;
        };
        let scroll_y = self.scroll.scroll_y;
        self.nav.update_scroll(scroll_y);
        self.navigate_vfs_inner(&url, vfs, false);
        // Synchronous (VFS) reloads are laid out already; restore the
        // offset. Network reloads land at the top like a fresh load.
        if self.document.is_some() {
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Drop the page's image and atlas textures, queueing their GPU
    /// handles in `retired_textures`: navigation has no backend, so
    /// `ensure_image_textures` destroys them on the next paint.
    pub(super) fn retire_image_textures(&mut self) {
        self.retired_textures
            .extend(self.image_textures.drain().map(|(_, tex)| tex));
        self.image_atlas.clear_into(&mut self.retired_textures);
    }

    /// Reset browser state in preparation for a new navigation.
    ///
    /// Clears the parsed document and layout tree along with all
    /// per-page image/atlas caches so callers can rely on a blank slate
    /// whether or not a fresh document ends up being loaded (error
    /// pages, iframe-overlay mode, …).
    pub(super) fn reset_for_navigation(&mut self) {
        self.state = LoadingState::Loading;
        self.selected_link = -1;
        self.clear_node_references();
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
        self.retire_image_textures();
        self.pending_images.clear();
        self.decoded_image_bytes = 0;
        self.decoded_image_lru.clear();
        self.cached_image_info.clear();
        self.image_info_dirty = false;
        #[cfg(native_net)]
        {
            self.pending_page_load = None;
            self.pending_page_url = None;
            self.pending_io_images.clear();
        }
    }

    /// Drop every piece of state that names a node of the current
    /// document by index. `NodeId`s are arena indices, so after the
    /// document is replaced they point at unrelated nodes of the new
    /// page — or past its end, which panics in the `:hover` / `:focus`
    /// ancestor walks. Call whenever `self.document` is swapped.
    pub(super) fn clear_node_references(&mut self) {
        self.hover_node = None;
        self.focused_node = None;
        self.transition_engine = css::transition::TransitionEngine::new();
    }

    /// Navigate to a URL using the VFS as the resource source.
    pub fn navigate_vfs(&mut self, url: &str, vfs: &dyn Vfs) {
        self.navigate_vfs_inner(url, vfs, true);
    }

    /// [`Self::navigate_vfs`]; `push_history` is false for loads of the
    /// current history entry (reload, back/forward cache misses).
    pub(super) fn navigate_vfs_inner(&mut self, url: &str, vfs: &dyn Vfs, push_history: bool) {
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
        // Consumed by `load_html` when this load (sync or from the I/O
        // thread) finishes; a later navigation overrides it.
        self.skip_nav_push = !push_history;

        // Internal pages: serve directly without hitting the VFS or
        // network. Only `vfs://bookmarks` is wired up for now — the
        // bookmarks button in the chrome navigates here. History is
        // available through `nav.history_page_html()` if we ever wire
        // a second button for it.
        let internal = match url {
            "vfs://bookmarks" | "oasis://bookmarks" => Some(self.nav.bookmarks_page_html()),
            "vfs://history" | "oasis://history" => Some(self.nav.history_page_html()),
            _ => None,
        };
        if let Some(html) = internal {
            let body = html.into_bytes();
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
            // GET navigations can't be told apart from typed URLs here,
            // so they count as user-initiated (same-site): only the
            // `Strict`-on-cross-site-link-click case is lost.
            initiator: None,
        };

        // Determine if this is a network request that can be offloaded.
        #[cfg(native_net)]
        {
            // `ensure_io_thread` fails only if the OS refuses to spawn a
            // thread; fall through to the synchronous load in that case.
            if self.should_use_io_thread(&request, vfs) && self.ensure_io_thread() {
                self.submit_page_load_to_io_thread(request);
                return;
            }
        }

        // Synchronous path: VFS-only or fallback.
        self.execute_sync_load(url, vfs, request);
    }

    /// Synchronous resource load and processing (VFS or fallback).
    pub(super) fn execute_sync_load(&mut self, url: &str, vfs: &dyn Vfs, request: ResourceRequest) {
        self.diag(&format!("[BR] sync_load fetch start: {url}"));
        match load_resource(
            vfs,
            &request,
            self.tls.as_deref(),
            #[cfg(native_net)]
            Some(&mut self.cookie_jar),
            #[cfg(native_net)]
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
    #[cfg(native_net)]
    pub(super) fn should_use_io_thread(&self, request: &ResourceRequest, vfs: &dyn Vfs) -> bool {
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

    /// Ensure the I/O thread is running. Returns `false` when it could
    /// not be spawned (the error is logged; callers fall back to
    /// synchronous loading or skip the request).
    ///
    /// The worker receives its own `Arc` clone of the TLS provider, so the
    /// provider stays alive for as long as the thread can use it -- even if
    /// [`BrowserWidget::set_tls_provider`] replaces `self.tls` meanwhile.
    #[cfg(native_net)]
    pub(crate) fn ensure_io_thread(&mut self) -> bool {
        if let Some(io) = self.io_thread.as_mut() {
            // Every submit comes through here: keep the limit current.
            io.set_max_redirects(self.config.max_redirects);
            return true;
        }
        let tls = self.tls.clone();
        let cookie_jar = self.cookie_jar.clone();
        match IoThread::spawn(tls, cookie_jar) {
            Ok(mut io) => {
                io.set_max_redirects(self.config.max_redirects);
                self.io_thread = Some(io);
                true
            },
            Err(e) => {
                log::error!("browser: failed to spawn I/O thread: {e}");
                false
            },
        }
    }

    /// Submit a page load request to the I/O thread.
    #[cfg(native_net)]
    pub(super) fn submit_page_load_to_io_thread(&mut self, request: ResourceRequest) {
        self.ensure_io_thread();

        // Extract cache validators before sending.
        let validators = self.cache.peek_validators(&request.url);

        if let Some(ref mut io) = self.io_thread {
            let url = request.url.clone();
            let id = io.send(IoRequestKind::PageLoad, request, validators, None);
            self.pending_page_load = Some(id);
            self.pending_page_url = Some(url);
        }
    }

    /// Submit an image load request to the I/O thread.
    #[cfg(native_net)]
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
    #[cfg(native_net)]
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
                let requested_url = self.pending_page_url.take();
                match result.result {
                    Ok(mut loaded) => {
                        // `304 Not Modified` answers our conditional
                        // request: the body to show is the cached one.
                        if loaded.response.status == 304
                            && let Some(cached) = self.cache.peek_response(&loaded.response.url)
                        {
                            // A 304 may omit the CSP header: keep the
                            // stored response's policy unless it sends one.
                            if loaded.csp.is_none() {
                                loaded.csp = self.cache.peek_csp(&loaded.response.url);
                            }
                            loaded.response = cached;
                        }
                        let url_str = loaded.response.url.clone();
                        let etag = loaded.etag.clone();
                        let last_modified = loaded.last_modified.clone();
                        self.page_csp = loaded.csp;
                        self.process_response(loaded.response);
                        self.cache.set_validators(&url_str, etag, last_modified);
                    },
                    Err(e) => {
                        let err_msg = e.to_string();
                        // Render the error page under the URL that
                        // failed so it stays in the URL bar / history
                        // and can be retried with a reload.
                        let url = requested_url.as_deref().unwrap_or("about:error");
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
                    // A 304 revalidation carries no body: use the cached one.
                    let cached = (loaded.response.status == 304)
                        .then(|| self.cache.peek_response(&loaded.response.url))
                        .flatten();
                    let body = match cached {
                        Some(cached) => cached.body,
                        None => loaded.response.body,
                    };
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
                        let decoded = match crate::image::decode_image_with_limits(
                            &body,
                            self.config.image_limits(),
                        ) {
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
        // `(slot, sheet url, css)` of arrived sheets, for `@import`s.
        let mut arrived: Vec<(usize, String, String)> = Vec::new();
        for result in stylesheet_results {
            let Some(idx) = self.pending_io_stylesheets.remove(&result.id) else {
                continue;
            };
            let sheet = match result.result {
                Ok(loaded) => {
                    let sheet_url = loaded.response.url;
                    let body = loaded.response.body;
                    if body.len() > MAX_STYLESHEET_BYTES {
                        log::warn!(
                            "external stylesheet {} too large ({} bytes), skipping",
                            sheet_url,
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
                    let sheet = css::parser::Stylesheet::parse_with_viewport(&css_text, viewport);
                    (sheet, css_text, sheet_url)
                },
                Err(e) => {
                    log::debug!("external stylesheet fetch failed: {e}");
                    self.pending_external_css_apply = true;
                    continue;
                },
            };
            if idx < self.external_stylesheets.len() {
                let (sheet, css_text, sheet_url) = sheet;
                self.external_stylesheets[idx] = Some(sheet);
                arrived.push((idx, sheet_url, css_text.clone()));
                self.record_external_source(idx, css_text);
                self.pending_external_css_apply = true;
            }
        }
        self.queue_stylesheet_imports(arrived);
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
                // Callers set `page_csp` from this response's headers
                // before handing it over.
                csp: self.page_csp.clone(),
            },
        );

        match content_type {
            ContentType::Html
            | ContentType::PlainText
            | ContentType::Css
            | ContentType::Unknown => {
                // A stylesheet opened directly is shown as escaped text:
                // it used to be wrapped in `<pre>` raw, so a
                // `</pre><script>` in it ran as page script.
                let body = html_for_text_body(content_type, &response.body);
                self.load_html(&body, &url);
            },
            ContentType::GeminiText => {
                let body = String::from_utf8_lossy(&response.body);
                self.load_gemini(&body, &url);
            },
            _ if content_type.is_image() => {
                // Decode the bytes we already have so the wrapping
                // `<img>` doesn't trigger a second network round-trip
                // (and inherit any cookie / redirect / cache failure
                // path that the page-load fetch already cleared).
                if let Some(decoded) = crate::image::decode_image_with_limits(
                    &response.body,
                    self.config.image_limits(),
                ) {
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
}
