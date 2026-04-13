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
    fn reset_for_navigation(&mut self) {
        self.state = LoadingState::Loading;
        self.selected_link = -1;
        self.reader_mode = false;
        self.reader_html = None;
        self.error_message = None;
        self.page_csp = None;
        self.page_errors.clear();
        self.decoded_images.clear();
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

            if let Ok(loaded) = result.result {
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
                } else if let Some(decoded) = crate::image::decode_image(&body) {
                    // Sync fallback.
                    let img_bytes = decoded.width as usize * decoded.height as usize * 4;
                    self.decoded_image_bytes += img_bytes;
                    self.decoded_image_lru.push_front(resolved.clone());
                    self.decoded_images.insert(resolved, decoded);
                    self.image_info_dirty = true;
                    self.layout_dirty = true;
                }
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
                let wrapped = format!(
                    "<html><body>\
                     <img src=\"{}\"></body></html>",
                    url
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
                    if let Err(e) = engine.with_context(|ctx| {
                        js_dom::install_document_global_with_csp(
                            &ctx,
                            &s,
                            url,
                            &nav,
                            &js_sty,
                            self.page_csp.as_ref(),
                            Some(&ls),
                        )
                    }) {
                        log::warn!("JS DOM install failed: {}", e.message);
                        self.record_error(
                            crate::BrowserErrorKind::Script,
                            format!("JS DOM install: {}", e.message),
                        );
                    }
                    // Install canvas 2D context bindings.
                    let cm = std::rc::Rc::clone(&self.canvas_states);
                    if let Err(e) =
                        engine.with_context(|ctx| js_dom::install_canvas_bindings(&ctx, &cm))
                    {
                        log::warn!("Canvas bindings install failed: {}", e.message);
                    }
                    if !scripts.is_empty() {
                        let script_refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
                        engine.eval_all(&script_refs);
                    }
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
        let author_sheets = Self::collect_style_sheets(&doc);
        let inline_styles = Self::collect_inline_styles(&doc);
        self.diag(&format!(
            "[BR] collect stylesheets done: {} sheets, {} inline",
            author_sheets.len(),
            inline_styles.len()
        ));

        // 4. CSS cascade: user-agent + author stylesheets + inline styles.
        self.diag("[BR] cascade start");
        let ua_sheet = css::default::default_stylesheet();
        let (styles, selector_index) = {
            let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
            for sheet in &author_sheets {
                all_sheets.push(sheet);
            }
            let ctx = css::cascade::CascadeContext {
                hover_node: self.hover_node,
                visited_urls: Some(&self.visited_urls),
                focused_node: None,
            };
            let styles = css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx);
            self.diag("[BR] selector index start");
            // Build selector index while all_sheets is alive.
            let idx = css::cascade::SelectorIndex::build(&all_sheets);
            (styles, idx)
        };
        self.diag(&format!("[BR] cascade done: {} styles", styles.len()));

        // Update shared computed styles for JS getComputedStyle().
        #[cfg(feature = "javascript")]
        {
            *self.js_styles.borrow_mut() = styles.clone();
        }

        // Cache parsed sheets and selector index for hover restyles.
        self.cached_author_sheets = author_sheets;
        self.cached_inline_styles = inline_styles;
        self.cached_selector_index = Some(selector_index);

        // 4b. Register CSS animations with the animation engine.
        //     Collect @keyframes from all stylesheets, then register any
        //     node that declares `animation-name`.
        {
            let mut all_keyframes: Vec<&css::parser::KeyframesRule> = Vec::new();
            all_keyframes.extend(ua_sheet.keyframes.iter());
            for sheet in &self.cached_author_sheets {
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
        let layout_root = layout::block::build_layout_tree(
            &doc,
            &styles,
            &measurer,
            viewport_w,
            viewport_h,
            Some(url),
            &self.cached_image_info,
        );
        self.diag("[BR] layout done");

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

    /// Walk the DOM to collect text from `<style>` elements and parse
    /// each into a `Stylesheet`. Both `<head>` and `<body>` style blocks
    /// are included.
    fn collect_style_sheets(doc: &html::dom::Document) -> Vec<css::parser::Stylesheet> {
        let mut sheets = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::Style
            {
                let css_text = doc.text_content(id);
                if !css_text.is_empty() {
                    sheets.push(css::parser::Stylesheet::parse(&css_text));
                }
            }
        }
        sheets
    }

    /// Walk the DOM to collect inline `style=""` attributes and parse
    /// each into a list of declarations keyed by NodeId.
    fn collect_inline_styles(
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
                };
                let styles = css::cascade::style_tree(&reader_doc, &[ua_sheet], &[], &reader_ctx);
                let href_map = Self::build_link_map(&reader_doc);
                self.cached_author_sheets = Vec::new();
                self.cached_inline_styles = Vec::new();
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
