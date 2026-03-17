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

impl BrowserWidget {
    /// Navigate via HTTP POST to a URL with the given body.
    ///
    /// Used for `<form method="post">` submissions. The encoded form
    /// data is sent as the request body with
    /// `Content-Type: application/x-www-form-urlencoded`.
    pub fn navigate_post(&mut self, url: &str, body: Vec<u8>, vfs: &dyn Vfs) {
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

        match load_resource(
            vfs,
            &request,
            self.tls.as_deref(),
            #[cfg(not(target_arch = "wasm32"))]
            Some(&mut self.cookie_jar),
            #[cfg(not(target_arch = "wasm32"))]
            Some(&self.cache),
        ) {
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

    /// Navigate to a URL using the VFS as the resource source.
    pub fn navigate_vfs(&mut self, url: &str, vfs: &dyn Vfs) {
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

        match load_resource(
            vfs,
            &request,
            self.tls.as_deref(),
            #[cfg(not(target_arch = "wasm32"))]
            Some(&mut self.cookie_jar),
            #[cfg(not(target_arch = "wasm32"))]
            Some(&self.cache),
        ) {
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
                let err_resp = loader::vfs::error_page(url, &err_msg);
                self.process_response(err_resp);
                self.state = LoadingState::Error;
                self.error_message = Some(err_msg.clone());
                self.record_error(crate::BrowserErrorKind::Network, err_msg);
            },
        }

        // Collect image requests for time-sliced loading across frames.
        // Page text renders immediately; images stream in via
        // `load_next_image_batch()` called from `paint()`.
        self.collect_page_image_requests();
        if !self.pending_images.is_empty() {
            self.state = LoadingState::Loading;
        }
    }

    /// Process a loaded resource response.
    pub fn process_response(&mut self, response: ResourceResponse) {
        let url = response.url.clone();
        let content_type = response.content_type;

        // Cache the response.
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
                // CSS files are not directly renderable.
                let wrapped = format!(
                    "<html><body><pre>{}</pre></body></html>",
                    String::from_utf8_lossy(&response.body)
                );
                self.load_html(&wrapped, &url);
            },
            _ if content_type.is_image() => {
                // Wrap image in a simple HTML page.
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

        // 1. Tokenize and build DOM.
        let tokens = html::tokenizer::Tokenizer::new(source).tokenize();
        let doc = html::tree_builder::TreeBuilder::build(tokens);

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
            let scripts = Self::collect_scripts(&doc);
            let shared: js_dom::SharedDoc = std::rc::Rc::new(std::cell::RefCell::new(doc));
            match oasis_js::JsEngine::new(8 * 1024 * 1024) {
                Ok(engine) => {
                    let s = std::rc::Rc::clone(&shared);
                    let nav = std::rc::Rc::clone(&self.js_nav_actions);
                    let js_sty = std::rc::Rc::clone(&self.js_styles);
                    if let Err(e) = engine.with_context(|ctx| {
                        js_dom::install_document_global_with_csp(
                            &ctx,
                            &s,
                            url,
                            &nav,
                            &js_sty,
                            self.page_csp.as_ref(),
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
            // Clone the (possibly mutated) document for layout/paint.
            shared.borrow().clone()
        };

        // 2. Extract page title.
        let title = doc.title().unwrap_or_else(|| url.to_string());

        // 3. Collect <style> blocks and inline style="" attributes from DOM.
        //    Cache them so hover restyles don't re-parse.
        let author_sheets = Self::collect_style_sheets(&doc);
        let inline_styles = Self::collect_inline_styles(&doc);

        // 4. CSS cascade: user-agent + author stylesheets + inline styles.
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![&ua_sheet];
        for sheet in &author_sheets {
            all_sheets.push(sheet);
        }
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: None,
        };
        let styles = css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx);

        // Update shared computed styles for JS getComputedStyle().
        #[cfg(feature = "javascript")]
        {
            *self.js_styles.borrow_mut() = styles.clone();
        }

        // Cache parsed sheets for hover restyles.
        self.cached_author_sheets = author_sheets;
        self.cached_inline_styles = inline_styles;

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
        let content_h = self.config.content_height(self.window_h);
        let image_info = self.build_image_info_map();
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
            &image_info,
        );

        // 7a. Collect canvas states from layout tree.
        #[cfg(feature = "javascript")]
        {
            self.canvas_states.borrow_mut().clear();
            crate::canvas::collect_canvas_states(&layout_root, &self.canvas_states);
        }

        // 7. Store results.
        self.document = Some(doc);
        self.styles = styles;
        self.href_map = href_map;
        self.layout_root = Some(layout_root);
        self.link_map.clear();
        self.scroll.reset();
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
    #[cfg(feature = "javascript")]
    fn collect_scripts(doc: &html::dom::Document) -> Vec<String> {
        let mut scripts = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::Script
                && elem.get_attribute("src").is_none()
                && Self::is_js_script_type(elem.get_attribute("type"))
            {
                let text = doc.text_content(id);
                if !text.is_empty() {
                    scripts.push(text);
                }
            }
        }
        scripts
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
                let styles = css::cascade::style_tree(&reader_doc, &[&ua_sheet], &[], &reader_ctx);
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
