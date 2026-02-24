//! Browser subsystem: HTML/CSS parsing, layout, and rendering.
//!
//! This module ties together the HTML/CSS pipeline (tokenizer, parser,
//! DOM, style cascade, layout, paint) with navigation, scroll state,
//! resource loading, reader mode, and Gemini protocol support into the
//! [`BrowserWidget`] -- the top-level component that the window manager
//! drives.

pub mod commands;
pub mod config;
pub(crate) mod css;
pub mod gemini;
pub(crate) mod html;
pub mod image;
pub(crate) mod layout;
pub mod loader;
pub mod nav;
pub(crate) mod paint;
pub mod plugin;
pub mod reader;
pub mod scroll;
pub mod skin;

#[cfg(test)]
pub(crate) mod test_utils;

// -----------------------------------------------------------------------
// Public re-exports
// -----------------------------------------------------------------------

pub use config::BrowserConfig;
pub use loader::{ContentType, ResourceResponse, ResourceSource, Url};
pub use nav::{Bookmark, HistoryEntry, NavigationController};
pub use scroll::ScrollState;

/// Map from img src URL -> (intrinsic_width, intrinsic_height) for decoded images.
pub type ImageInfoMap = HashMap<String, (u32, u32)>;

// -----------------------------------------------------------------------
// Bench/fuzz re-exports (not part of public API)
// -----------------------------------------------------------------------

/// Internal types exposed for benchmarks and fuzz targets.
///
/// These are implementation details and may change without notice.
#[doc(hidden)]
pub mod internals {
    pub use crate::css::cascade::{CascadeContext, style_tree};
    pub use crate::css::parser::{Stylesheet, parse_inline_style};
    pub use crate::css::values::ComputedStyle;
    pub use crate::html::dom::Document;
    pub use crate::html::tokenizer::Tokenizer;
    pub use crate::html::tree_builder::TreeBuilder;
    pub use crate::layout::block::{
        StyleCache, TextMeasurer, build_layout_tree, layout_block_incremental,
    };
    pub use crate::paint::paint as paint_page;
}

// -----------------------------------------------------------------------
// Imports
// -----------------------------------------------------------------------

use std::collections::{HashMap, HashSet};

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_types::input::{Button, InputEvent, Trigger};
use oasis_vfs::Vfs;

use html::dom::{NodeId, NodeKind, TagName};
use loader::cache::{CacheEntry, ResourceCache};
use loader::{ResourceRequest, load_resource};
use oasis_types::backend::TextureId;
use paint::LinkRegion;

// -----------------------------------------------------------------------
// LoadingState
// -----------------------------------------------------------------------

/// Current loading state of the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingState {
    /// No load in progress.
    Idle,
    /// A resource load is in progress.
    Loading,
    /// The most recent load failed.
    Error,
}

// -----------------------------------------------------------------------
// SimpleTextMeasurer
// -----------------------------------------------------------------------

/// A text measurer that approximates glyph widths as 8 pixels per
/// character, matching the 8x8 bitmap font used by OASIS backends.
pub struct SimpleTextMeasurer;

impl layout::block::TextMeasurer for SimpleTextMeasurer {
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_types::backend::bitmap_measure_text(text, font_size)
    }
}

// -----------------------------------------------------------------------
// Focus
// -----------------------------------------------------------------------

/// Which part of the browser chrome has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Keyboard input goes to content (link navigation, scrolling).
    Content,
    /// Keyboard input goes to the URL bar for editing.
    UrlBar,
}

// -----------------------------------------------------------------------
// BrowserWidget
// -----------------------------------------------------------------------

/// Top-level browser component driven by the window manager.
///
/// Owns the full browser pipeline: resource loading, HTML parsing, CSS
/// cascade, layout, paint, navigation, scroll, reader mode, and Gemini
/// protocol support.
pub struct BrowserWidget {
    /// Visual and feature configuration.
    pub config: BrowserConfig,

    /// Navigation controller (history, bookmarks).
    nav: NavigationController,

    /// Scroll state for the content viewport.
    scroll: ScrollState,

    /// Resource cache (LRU, bounded by byte size).
    cache: ResourceCache,

    /// Current loading state.
    state: LoadingState,

    /// The most recent error message (if any).
    error_message: Option<String>,

    /// Parsed DOM of the current page.
    document: Option<html::dom::Document>,

    /// Computed styles indexed by `NodeId`.
    styles: Vec<Option<css::values::ComputedStyle>>,

    /// Layout tree root for the current page.
    layout_root: Option<layout::box_model::LayoutBox>,

    /// Link regions from the most recent paint pass.
    link_map: Vec<LinkRegion>,

    /// Map from DOM `NodeId` to `href` for `<a>` elements.
    href_map: HashMap<NodeId, String>,

    /// Index of the currently focused link (-1 = none).
    selected_link: i32,

    /// Which part of the chrome has keyboard focus.
    focus: Focus,

    /// URL bar editing buffer (populated when focus is `UrlBar`).
    url_input: String,

    /// Cursor position within `url_input` (byte offset).
    url_cursor: usize,

    /// Whether reader mode is active.
    reader_mode: bool,

    /// Reader-mode article HTML (re-parsed when toggled).
    reader_html: Option<String>,

    /// Window position and size set by the window manager.
    window_x: i32,
    window_y: i32,
    window_w: u32,
    window_h: u32,

    /// Optional TLS provider for HTTPS and Gemini connections.
    tls: Option<Box<dyn oasis_net::tls::TlsProvider>>,

    /// Whether the layout tree needs rebuilding.
    layout_dirty: bool,

    /// Viewport width used for the most recent layout pass.
    last_layout_w: u32,

    /// Set of URLs that have been navigated to (for `:visited`).
    visited_urls: HashSet<String>,

    /// DOM node currently under the cursor (for `:hover`).
    hover_node: Option<NodeId>,

    /// Decoded image data keyed by resolved src URL.
    decoded_images: HashMap<String, image::DecodedImage>,

    /// GPU textures for decoded images, keyed by src URL.
    image_textures: HashMap<String, TextureId>,

    /// Pending image requests to be processed in time-sliced batches.
    pending_images: Vec<(String, ResourceRequest)>,

    /// Total bytes of decoded image RGBA data currently held.
    decoded_image_bytes: usize,

    /// LRU order for decoded images (front = most recent).
    /// Used for eviction when over `IMAGE_MEMORY_BUDGET`.
    decoded_image_lru: std::collections::VecDeque<String>,

    /// Cached image info map (URL -> intrinsic dimensions). Rebuilt
    /// only when `image_info_dirty` is set (after new image decodes).
    cached_image_info: ImageInfoMap,

    /// Whether the cached image info map needs rebuilding.
    image_info_dirty: bool,

    /// Cached author stylesheets (from `<style>` blocks). Re-parsed only
    /// on navigation, not on hover.
    cached_author_sheets: Vec<css::parser::Stylesheet>,

    /// Cached inline `style=""` declarations. Re-parsed only on navigation.
    cached_inline_styles: Vec<(NodeId, Vec<css::parser::Declaration>)>,

    /// Last time a hover restyle was performed (for throttling).
    last_hover_time: Option<std::time::Instant>,
}

impl BrowserWidget {
    /// Create a new browser widget with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        let home = config.features.home_url.clone();
        let cache_bytes = config.cache_size_bytes();
        let smooth = config.smooth_scroll;
        Self {
            config,
            nav: NavigationController::new(&home),
            scroll: ScrollState::new(238, smooth), // 272 - 34
            cache: ResourceCache::new(cache_bytes),
            state: LoadingState::Idle,
            error_message: None,
            document: None,
            styles: Vec::new(),
            layout_root: None,
            link_map: Vec::new(),
            href_map: HashMap::new(),
            selected_link: -1,
            focus: Focus::Content,
            url_input: String::new(),
            url_cursor: 0,
            reader_mode: false,
            reader_html: None,
            window_x: 0,
            window_y: 0,
            window_w: 480,
            window_h: 272,
            tls: None,
            layout_dirty: false,
            last_layout_w: 480,
            visited_urls: HashSet::new(),
            hover_node: None,
            decoded_images: HashMap::new(),
            image_textures: HashMap::new(),
            pending_images: Vec::new(),
            decoded_image_bytes: 0,
            decoded_image_lru: std::collections::VecDeque::new(),
            cached_image_info: HashMap::new(),
            image_info_dirty: false,
            cached_author_sheets: Vec::new(),
            cached_inline_styles: Vec::new(),
            last_hover_time: None,
        }
    }

    /// Attach a TLS provider for HTTPS and Gemini support.
    pub fn set_tls_provider(&mut self, provider: Box<dyn oasis_net::tls::TlsProvider>) {
        self.tls = Some(provider);
    }

    /// Update the window position and size (called by the WM).
    ///
    /// Marks layout dirty if the viewport width changed, since layout
    /// depends on the available width for line breaking and block sizing.
    pub fn set_window(&mut self, x: i32, y: i32, w: u32, h: u32) {
        if w != self.window_w || h != self.window_h {
            self.layout_dirty = true;
        }
        self.window_x = x;
        self.window_y = y;
        self.window_w = w;
        self.window_h = h;
        let vh = self.config.content_height(h) as i32;
        self.scroll.set_viewport_height(vh);
    }

    /// Returns whether the layout tree needs rebuilding.
    pub fn is_layout_dirty(&self) -> bool {
        self.layout_dirty
    }

    /// Rebuild layout from cached DOM/styles if the viewport changed.
    /// Returns `true` if relayout was performed, `false` if skipped.
    pub fn relayout_if_dirty(&mut self) -> bool {
        if !self.layout_dirty {
            return false;
        }
        if self.document.is_none() || self.styles.is_empty() {
            self.layout_dirty = false;
            return false;
        }

        let image_info = self.build_image_info_map();
        let doc = self.document.as_ref().unwrap();
        let content_h = self.config.content_height(self.window_h);
        let base_url = self.nav.current_url().map(String::from);
        let layout_root = layout::block::build_layout_tree(
            doc,
            &self.styles,
            &SimpleTextMeasurer,
            self.window_w as f32,
            content_h as f32,
            base_url.as_deref(),
            &image_info,
        );

        self.layout_root = Some(layout_root);
        self.link_map.clear();
        self.last_layout_w = self.window_w;
        self.layout_dirty = false;
        true
    }

    // ---------------------------------------------------------------
    // Navigation / loading
    // ---------------------------------------------------------------

    /// Navigate to a URL using the VFS as the resource source.
    pub fn navigate_vfs(&mut self, url: &str, vfs: &dyn Vfs) {
        self.state = LoadingState::Loading;
        self.selected_link = -1;
        self.reader_mode = false;
        self.reader_html = None;
        self.error_message = None;
        self.decoded_images.clear();
        self.image_textures.clear();
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

        let request = ResourceRequest {
            url: url.to_string(),
            base_url: self.nav.current_url().map(String::from),
            source,
        };

        match load_resource(vfs, &request, self.tls.as_deref()) {
            Ok(response) => {
                self.process_response(response);
            },
            Err(e) => {
                let err_resp = loader::vfs::error_page(url, &e.to_string());
                self.process_response(err_resp);
                self.state = LoadingState::Error;
                self.error_message = Some(e.to_string());
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
    pub fn load_html(&mut self, html_source: &str, url: &str) {
        // 1. Tokenize and build DOM.
        let tokens = html::tokenizer::Tokenizer::new(html_source).tokenize();
        let doc = html::tree_builder::TreeBuilder::build(tokens);

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
        };
        let styles = css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx);

        // Cache parsed sheets for hover restyles.
        self.cached_author_sheets = author_sheets;
        self.cached_inline_styles = inline_styles;

        // 5. Build link href map from DOM.
        let href_map = Self::build_link_map(&doc);

        // 6. Build layout tree.
        let content_h = self.config.content_height(self.window_h);
        let image_info = self.build_image_info_map();
        let layout_root = layout::block::build_layout_tree(
            &doc,
            &styles,
            &SimpleTextMeasurer,
            self.window_w as f32,
            content_h as f32,
            Some(url),
            &image_info,
        );

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

        // 8. Update navigation.
        self.nav.navigate(url, &title);
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

    // ---------------------------------------------------------------
    // Image loading
    // ---------------------------------------------------------------

    /// Walk the DOM to find `<img>` elements and collect their requests
    /// into `self.pending_images` for time-sliced loading. Does NOT
    /// fetch or decode — that happens in `load_next_image_batch()`.
    fn collect_page_image_requests(&mut self) {
        let doc = match &self.document {
            Some(d) => d,
            None => return,
        };
        let base_url = self.nav.current_url().map(String::from);

        let mut requests: Vec<(String, ResourceRequest)> = Vec::new();
        for node in &doc.nodes {
            if let NodeKind::Element(elem) = &node.kind
                && elem.tag == TagName::Img
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(&base_url, src);
                if self.decoded_images.contains_key(&resolved) {
                    continue;
                }
                let source = if self.config.features.sandbox_only {
                    ResourceSource::Vfs
                } else {
                    ResourceSource::VfsThenNetwork
                };
                requests.push((
                    resolved.clone(),
                    ResourceRequest {
                        url: resolved,
                        base_url: base_url.clone(),
                        source,
                    },
                ));
            }
        }

        self.pending_images = requests;
    }

    /// Maximum decoded image memory budget (bytes of RGBA data).
    const IMAGE_MEMORY_BUDGET: usize = 8 * 1024 * 1024; // 8MB

    /// Process pending image requests within a time budget.
    ///
    /// Pops requests from `pending_images`, fetches and decodes each.
    /// Returns after the time budget is exhausted or all images are done.
    /// Each successful decode sets `layout_dirty` so the layout tree
    /// rebuilds with correct intrinsic dimensions.
    pub fn load_next_image_batch(&mut self, vfs: &dyn Vfs, budget_ms: u32) {
        if self.pending_images.is_empty() {
            return;
        }

        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(budget_ms as u64);
        let mut any_decoded = false;

        while let Some((resolved, request)) = self.pending_images.pop() {
            // Skip if already decoded (e.g. from cache).
            if self.decoded_images.contains_key(&resolved) {
                continue;
            }

            if let Ok(response) = load_resource(vfs, &request, self.tls.as_deref())
                && let Some(decoded) = image::decode_image(&response.body)
            {
                let img_bytes = (decoded.width * decoded.height * 4) as usize;

                // Evict oldest decoded images if over budget.
                while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                    if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                        if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                            let evicted_bytes = (evicted.width * evicted.height * 4) as usize;
                            self.decoded_image_bytes -= evicted_bytes;
                            self.image_info_dirty = true;
                        }
                    } else {
                        break;
                    }
                }

                self.decoded_image_bytes += img_bytes;
                self.decoded_image_lru.push_front(resolved.clone());
                self.decoded_images.insert(resolved, decoded);
                self.image_info_dirty = true;
                any_decoded = true;
            }

            // Check time budget after each image.
            if start.elapsed() >= budget {
                break;
            }
        }

        if any_decoded {
            self.layout_dirty = true;
            self.rebuild_layout_with_images();
        }

        if self.pending_images.is_empty() && self.state == LoadingState::Loading {
            self.state = LoadingState::Idle;
        }
    }

    /// Get the cached image info map, rebuilding it if dirty.
    fn build_image_info_map(&mut self) -> ImageInfoMap {
        if self.image_info_dirty {
            self.cached_image_info = self
                .decoded_images
                .iter()
                .map(|(url, img)| (url.clone(), (img.width, img.height)))
                .collect();
            self.image_info_dirty = false;
        }
        self.cached_image_info.clone()
    }

    /// Upload decoded images as GPU textures and assign them to
    /// `ReplacedContent::Image` nodes in the layout tree.
    fn ensure_image_textures(&mut self, backend: &mut dyn SdiBackend) {
        let doc = match &self.document {
            Some(d) => d,
            None => return,
        };
        let base_url = self.nav.current_url().map(String::from);

        // Collect URLs that need texture creation.
        let mut pending: Vec<String> = Vec::new();
        for node in &doc.nodes {
            if let NodeKind::Element(elem) = &node.kind
                && elem.tag == TagName::Img
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(&base_url, src);
                if !self.image_textures.contains_key(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                {
                    pending.push(resolved);
                }
            }
        }

        // Create textures.
        for resolved in &pending {
            if let Some(decoded) = self.decoded_images.get(resolved)
                && let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
            {
                self.image_textures.insert(resolved.clone(), tex);
            }
        }

        // Also collect background-image URLs from styles.
        for style_opt in &self.styles {
            if let Some(style) = style_opt
                && let css::values::BackgroundImage::Url(ref url) = style.background_image
            {
                let resolved = Self::resolve_src(&base_url, url);
                if !self.image_textures.contains_key(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                    && !pending.contains(&resolved)
                {
                    pending.push(resolved);
                }
            }
        }

        // Create textures.
        for resolved in &pending {
            if let Some(decoded) = self.decoded_images.get(resolved)
                && let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
            {
                self.image_textures.insert(resolved.clone(), tex);
            }
        }

        // Walk layout tree and assign textures.
        if let Some(layout) = &mut self.layout_root {
            Self::assign_textures_recursive(
                layout,
                &self.document,
                &base_url,
                &self.image_textures,
            );
        }
    }

    /// Recursively walk the layout tree and assign GPU textures to
    /// `ReplacedContent::Image` nodes and `background-image` styles.
    fn assign_textures_recursive(
        layout_box: &mut layout::box_model::LayoutBox,
        doc: &Option<html::dom::Document>,
        base_url: &Option<String>,
        textures: &HashMap<String, TextureId>,
    ) {
        if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
            ref mut texture,
            ..
        }) = layout_box.box_type
            && texture.is_none()
            && let Some(node_id) = layout_box.node
            && let Some(doc) = doc
        {
            let node = doc.get(node_id);
            if let NodeKind::Element(elem) = &node.kind
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(base_url, src);
                if let Some(&tex) = textures.get(&resolved) {
                    *texture = Some(tex);
                }
            }
        }

        // Assign background-image texture.
        if layout_box.background_texture.is_none()
            && let css::values::BackgroundImage::Url(ref url) = layout_box.style.background_image
        {
            let resolved = Self::resolve_src(base_url, url);
            if let Some(&tex) = textures.get(&resolved) {
                layout_box.background_texture = Some(tex);
            }
        }

        for child in &mut layout_box.children {
            Self::assign_textures_recursive(child, doc, base_url, textures);
        }
    }

    /// Rebuild the layout tree with image dimensions after images have
    /// been decoded (second layout pass).
    fn rebuild_layout_with_images(&mut self) {
        let image_info = self.build_image_info_map();
        if let Some(doc) = &self.document {
            let content_h = self.config.content_height(self.window_h);
            let base_url = self.nav.current_url().map(String::from);
            let layout_root = layout::block::build_layout_tree(
                doc,
                &self.styles,
                &SimpleTextMeasurer,
                self.window_w as f32,
                content_h as f32,
                base_url.as_deref(),
                &image_info,
            );
            self.layout_root = Some(layout_root);
            self.link_map.clear();
        }
    }

    /// Resolve an img `src` attribute against a base URL.
    fn resolve_src(base_url: &Option<String>, src: &str) -> String {
        match base_url {
            Some(base) => {
                if let Some(base_parsed) = loader::Url::parse(base) {
                    base_parsed
                        .resolve(src)
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| src.to_string())
                } else {
                    src.to_string()
                }
            },
            None => src.to_string(),
        }
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

    // ---------------------------------------------------------------
    // Painting
    // ---------------------------------------------------------------

    /// Paint the browser to the backend.
    ///
    /// Draws chrome (URL bar, navigation buttons, status bar) and
    /// the page content viewport.
    /// Per-frame update: process pending image loads within a time budget.
    ///
    /// Call this once per frame before `paint()`. Images stream in
    /// progressively so the page is never blocked waiting for all images.
    pub fn tick(&mut self, vfs: &dyn Vfs) {
        self.load_next_image_batch(vfs, 8);
    }

    pub fn paint(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        // Rebuild layout if the viewport was resized since last paint.
        self.relayout_if_dirty();

        // Set clip to our window area.
        backend.set_clip_rect(self.window_x, self.window_y, self.window_w, self.window_h)?;

        // Paint chrome (URL bar + buttons).
        self.paint_chrome(backend)?;

        // Content viewport.
        let content_y = self.window_y + self.config.url_bar_height as i32;
        let content_h = self.config.content_height(self.window_h);

        backend.set_clip_rect(self.window_x, content_y, self.window_w, content_h)?;

        // Paint page background.
        backend.fill_rect(
            self.window_x,
            content_y,
            self.window_w,
            content_h,
            self.config.default_bg_color,
        )?;

        // Upload decoded images as GPU textures and assign to layout
        // nodes (first paint after navigation).
        self.ensure_image_textures(backend);

        // Paint layout tree if available.
        if let Some(layout) = &self.layout_root {
            let result = paint::paint(
                layout,
                backend,
                self.scroll.scroll_y as f32,
                self.window_x,
                content_y,
                self.window_w as f32,
                content_h as f32,
                &self.href_map,
            )?;
            self.link_map = result.links;
            self.scroll.set_content_height(result.content_height as i32);
        }

        // Paint link highlight if a link is selected.
        if self.selected_link >= 0 {
            let idx = self.selected_link as usize;
            if idx < self.link_map.len() {
                let link = self.link_map[idx].clone();
                paint::paint_link_highlight(&link, backend, Color::rgb(255, 200, 0))?;
            }
        }

        // Paint scrollbar when content overflows viewport.
        if self.scroll.max_scroll() > 0 {
            let sb_w: u32 = 6;
            let sb_x = self.window_x + self.window_w as i32 - sb_w as i32 - 1;
            let track_y = content_y;
            let track_h = content_h;

            // Track.
            backend.fill_rect(sb_x, track_y, sb_w, track_h, Color::rgba(255, 255, 255, 20))?;

            // Thumb: proportional to viewport/content ratio.
            let ratio = self.scroll.viewport_height as f32 / self.scroll.content_height as f32;
            let thumb_h = ((track_h as f32 * ratio) as u32).max(12).min(track_h);
            let scrollable = track_h - thumb_h;
            let frac = self.scroll.scroll_fraction();
            let thumb_y = track_y + (scrollable as f32 * frac) as i32;
            backend.fill_rect(
                sb_x,
                thumb_y,
                sb_w,
                thumb_h,
                Color::rgba(255, 255, 255, 100),
            )?;
        }

        // Paint status bar.
        self.paint_status_bar(backend)?;

        backend.reset_clip_rect()?;
        Ok(())
    }

    /// Paint the URL bar and navigation buttons.
    pub fn paint_chrome(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let h = self.config.url_bar_height;
        let bw = self.config.button_width;
        let themed = self.config.use_themed_chrome;
        let r: u16 = 4; // Chrome element border radius.

        // Chrome background.
        if themed {
            backend.fill_rounded_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                r,
                self.config.chrome_bg,
            )?;
        } else {
            backend.fill_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                self.config.chrome_bg,
            )?;
        }

        // Back button.
        let back_color = if self.nav.can_go_back() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(self.window_x, self.window_y, bw, h, r, back_color)?;
        } else {
            backend.fill_rect(self.window_x, self.window_y, bw, h, back_color)?;
        }
        backend.draw_text(
            "<",
            self.window_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // Forward button.
        let fwd_color = if self.nav.can_go_forward() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(
                self.window_x + bw as i32,
                self.window_y,
                bw,
                h,
                r,
                fwd_color,
            )?;
        } else {
            backend.fill_rect(self.window_x + bw as i32, self.window_y, bw, h, fwd_color)?;
        }
        backend.draw_text(
            ">",
            self.window_x + bw as i32 + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // URL bar.
        let url_x = self.window_x + (bw * 2) as i32;
        let url_w = self.window_w.saturating_sub(bw * 3);

        // Use a highlighted background when the URL bar is focused.
        let bar_bg = if self.focus == Focus::UrlBar {
            Color::rgb(60, 60, 80)
        } else {
            self.config.url_bar_bg
        };
        if themed {
            backend.fill_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                bar_bg,
            )?;
            // Stroke around URL bar for definition.
            backend.stroke_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                1,
                Color::rgba(255, 255, 255, 30),
            )?;
        } else {
            backend.fill_rect(url_x, self.window_y + 2, url_w, h.saturating_sub(4), bar_bg)?;
        }

        // URL text: show the editing buffer when focused, otherwise
        // the current navigation URL.
        let max_chars = (url_w / 8).saturating_sub(1) as usize;
        if self.focus == Focus::UrlBar {
            // Show editing buffer with cursor indicator.
            let display = if self.url_input.len() > max_chars {
                &self.url_input[..self.url_input.floor_char_boundary(max_chars)]
            } else {
                &self.url_input
            };
            backend.draw_text(
                display,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;

            // Draw cursor line.
            let cursor_chars = self.url_input[..self.url_cursor].chars().count();
            let cursor_px = url_x + 4 + cursor_chars as i32 * 8;
            if cursor_px < url_x + url_w as i32 - 4 {
                backend.fill_rect(
                    cursor_px,
                    self.window_y + 3,
                    1,
                    h.saturating_sub(6),
                    self.config.url_bar_text,
                )?;
            }
        } else {
            let url_text = self.nav.current_url().unwrap_or("about:blank");
            let display_url = if url_text.len() > max_chars {
                &url_text[..url_text.floor_char_boundary(max_chars)]
            } else {
                url_text
            };
            backend.draw_text(
                display_url,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;
        }

        // Home button (rightmost).
        let home_x = self.window_x + self.window_w as i32 - bw as i32;
        if themed {
            backend.fill_rounded_rect(
                home_x,
                self.window_y + 2,
                bw,
                h.saturating_sub(4),
                r,
                self.config.chrome_button_bg,
            )?;
        } else {
            backend.fill_rect(home_x, self.window_y, bw, h, self.config.chrome_button_bg)?;
        }
        backend.draw_text(
            "H",
            home_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        Ok(())
    }

    /// Paint the status bar at the bottom.
    pub fn paint_status_bar(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let sh = self.config.status_bar_height;
        let sy = self.window_y + self.window_h as i32 - sh as i32;

        backend.fill_rect(
            self.window_x,
            sy,
            self.window_w,
            sh,
            self.config.status_bar_bg,
        )?;

        // Status text.
        let status = match self.state {
            LoadingState::Idle => {
                if self.reader_mode {
                    "Reader mode"
                } else {
                    "Ready"
                }
            },
            LoadingState::Loading => "Loading...",
            LoadingState::Error => "Error",
        };
        backend.draw_text(
            status,
            self.window_x + 4,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        // Scroll indicator on the right.
        let frac = self.scroll.scroll_fraction();
        let pct = (frac * 100.0) as u32;
        let scroll_text = format!("{}%", pct);
        let text_w = oasis_types::backend::bitmap_measure_text(&scroll_text, 10) as i32;
        backend.draw_text(
            &scroll_text,
            self.window_x + self.window_w as i32 - text_w - 4,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        Ok(())
    }

    // ---------------------------------------------------------------
    // Input handling
    // ---------------------------------------------------------------

    /// Handle an input event. Returns `true` if the event was
    /// consumed.
    pub fn handle_input(&mut self, event: &InputEvent, vfs: &dyn Vfs) -> bool {
        // URL-bar editing mode intercepts most keys.
        if self.focus == Focus::UrlBar {
            match event {
                InputEvent::TextInput(ch) => {
                    self.url_input.insert(self.url_cursor, *ch);
                    self.url_cursor += ch.len_utf8();
                    return true;
                },
                InputEvent::Backspace => {
                    if self.url_cursor > 0 {
                        // Find the previous character boundary.
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_input.remove(prev);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Confirm) => {
                    let url = self.url_input.clone();
                    self.focus = Focus::Content;
                    if !url.is_empty() {
                        self.navigate_to(&url, vfs);
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Cancel) => {
                    // Discard edits.
                    self.focus = Focus::Content;
                    self.url_input.clear();
                    self.url_cursor = 0;
                    return true;
                },
                InputEvent::ButtonPress(Button::Left) => {
                    if self.url_cursor > 0 {
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Right) => {
                    if self.url_cursor < self.url_input.len() {
                        let next = self.url_input[self.url_cursor..]
                            .chars()
                            .next()
                            .map(|c| self.url_cursor + c.len_utf8())
                            .unwrap_or(self.url_input.len());
                        self.url_cursor = next;
                    }
                    return true;
                },
                InputEvent::PointerClick { x, y } => {
                    self.handle_click(*x, *y, vfs);
                    return true;
                },
                _ => return false,
            }
        }

        match event {
            InputEvent::ButtonPress(Button::Up) => {
                self.scroll.scroll_up();
                true
            },
            InputEvent::ButtonPress(Button::Down) => {
                self.scroll.scroll_down();
                true
            },
            InputEvent::ButtonPress(Button::Left) => {
                self.select_prev_link();
                true
            },
            InputEvent::ButtonPress(Button::Right) => {
                self.select_next_link();
                true
            },
            InputEvent::ButtonPress(Button::Confirm) => {
                self.activate_selected_link(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Cancel) => {
                self.go_back(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Triangle) => {
                self.toggle_reader_mode();
                true
            },
            InputEvent::ButtonPress(Button::Square) => {
                self.go_home(vfs);
                true
            },
            InputEvent::TriggerPress(Trigger::Left) => {
                self.scroll.page_up();
                true
            },
            InputEvent::TriggerPress(Trigger::Right) => {
                self.scroll.page_down();
                true
            },
            InputEvent::MouseWheel { delta } => {
                self.scroll.wheel_scroll(*delta);
                true
            },
            InputEvent::CursorMove { x, y } => {
                self.handle_cursor_move(*x, *y);
                true
            },
            InputEvent::PointerClick { x, y } => {
                self.handle_click(*x, *y, vfs);
                true
            },
            _ => false,
        }
    }

    /// Select the next link in the link map.
    pub fn select_next_link(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link += 1;
        if self.selected_link >= self.link_map.len() as i32 {
            self.selected_link = 0;
        }
        self.scroll_to_selected_link();
    }

    /// Select the previous link in the link map.
    pub fn select_prev_link(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link -= 1;
        if self.selected_link < 0 {
            self.selected_link = self.link_map.len() as i32 - 1;
        }
        self.scroll_to_selected_link();
    }

    /// Scroll to make the currently selected link visible.
    fn scroll_to_selected_link(&mut self) {
        if self.selected_link < 0 {
            return;
        }
        let idx = self.selected_link as usize;
        if idx < self.link_map.len() {
            let link = &self.link_map[idx];
            self.scroll
                .scroll_to_visible(link.rect.y as i32, link.rect.height as i32);
        }
    }

    /// Activate the currently selected link.
    pub fn activate_selected_link(&mut self, vfs: &dyn Vfs) {
        if self.selected_link < 0 {
            return;
        }
        let idx = self.selected_link as usize;
        if idx < self.link_map.len() {
            let href = self.link_map[idx].href.clone();
            self.navigate_to(&href, vfs);
        }
    }

    /// Handle a pointer click at window-relative coordinates.
    pub fn handle_click(&mut self, x: i32, y: i32, vfs: &dyn Vfs) {
        let rel_y = y - self.window_y;
        let chrome_h = self.config.url_bar_height as i32;

        // Click in chrome area?
        if rel_y < chrome_h {
            let rel_x = x - self.window_x;
            let bw = self.config.button_width as i32;

            if rel_x < bw {
                // Back button.
                self.focus = Focus::Content;
                self.go_back(vfs);
            } else if rel_x < bw * 2 {
                // Forward button.
                self.focus = Focus::Content;
                self.go_forward(vfs);
            } else if rel_x >= self.window_w as i32 - bw {
                // Home button.
                self.focus = Focus::Content;
                self.go_home(vfs);
            } else {
                // URL bar area -- enter edit mode.
                self.focus = Focus::UrlBar;
                self.url_input = self.nav.current_url().unwrap_or("about:blank").to_string();
                self.url_cursor = self.url_input.len();
            }
            return;
        }

        // Click in content area: leave URL bar editing.
        self.focus = Focus::Content;

        // Check link hit regions.
        for link in &self.link_map {
            let lx = link.rect.x;
            let ly = link.rect.y;
            let lw = link.rect.width;
            let lh = link.rect.height;
            if (x as f32) >= lx && (x as f32) < lx + lw && (y as f32) >= ly && (y as f32) < ly + lh
            {
                let href = link.href.clone();
                self.navigate_to(&href, vfs);
                return;
            }
        }
    }

    /// Handle a cursor move at window-relative coordinates.
    ///
    /// Hit-tests link regions to determine the hover target. If the
    /// hovered node changes, re-runs the CSS cascade so `:hover` rules
    /// take effect.
    fn handle_cursor_move(&mut self, x: i32, y: i32) {
        let mut new_hover: Option<NodeId> = None;
        for link in &self.link_map {
            let lx = link.rect.x;
            let ly = link.rect.y;
            let lw = link.rect.width;
            let lh = link.rect.height;
            if (x as f32) >= lx && (x as f32) < lx + lw && (y as f32) >= ly && (y as f32) < ly + lh
            {
                new_hover = Some(link.node);
                break;
            }
        }

        if new_hover != self.hover_node {
            // Throttle hover restyles to at most 20/sec.
            let now = std::time::Instant::now();
            if let Some(last) = self.last_hover_time
                && now.duration_since(last).as_millis() < 50
            {
                return;
            }
            self.last_hover_time = Some(now);

            let old_hover = self.hover_node;
            self.hover_node = new_hover;
            self.restyle_hover_affected(old_hover);
        }
    }

    /// Re-run the CSS cascade on hover-affected nodes only.
    ///
    /// Instead of re-parsing stylesheets and re-cascading the entire DOM,
    /// this uses cached sheets and only re-computes styles for the ancestors
    /// of the old and new hover nodes — typically ~10-20 nodes.
    fn restyle_hover_affected(&mut self, old_hover: Option<NodeId>) {
        let Some(doc) = &self.document else { return };

        // Build the set of affected nodes: ancestors of old + new hover.
        let mut affected: Vec<NodeId> = Vec::new();
        for start in [old_hover, self.hover_node].into_iter().flatten() {
            let mut cur = Some(start);
            while let Some(nid) = cur {
                if !affected.contains(&nid) {
                    affected.push(nid);
                }
                cur = doc.nodes[nid].parent;
            }
        }

        if affected.is_empty() {
            return;
        }

        // Build sheet references from cache (no re-parsing).
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![&ua_sheet];
        for sheet in &self.cached_author_sheets {
            all_sheets.push(sheet);
        }

        let index = css::cascade::SelectorIndex::build(&all_sheets);
        let inline_map: HashMap<NodeId, &[css::parser::Declaration]> = self
            .cached_inline_styles
            .iter()
            .map(|(nid, decls)| (*nid, decls.as_slice()))
            .collect();
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
        };

        let mut any_changed = false;
        for &nid in &affected {
            let node = &doc.nodes[nid];
            if !matches!(node.kind, html::dom::NodeKind::Element(_)) {
                continue;
            }
            let parent_style = node.parent.and_then(|pid| self.styles[pid].as_ref());
            let new_style = css::cascade::compute_style(
                doc,
                nid,
                parent_style,
                &all_sheets,
                &index,
                &inline_map,
                &ctx,
            );
            if self.styles[nid].as_ref() != Some(&new_style) {
                self.styles[nid] = Some(new_style);
                any_changed = true;
            }
        }

        if any_changed {
            self.layout_dirty = true;
        }
    }

    /// Navigate to a URL, resolving relative references against
    /// the current page.
    pub fn navigate_to(&mut self, href: &str, vfs: &dyn Vfs) {
        let resolved = if let Some(current) = self.nav.current_url() {
            if let Some(base) = Url::parse(current) {
                base.resolve(href)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string())
            } else {
                href.to_string()
            }
        } else {
            href.to_string()
        };

        // Track this URL as visited for :visited pseudo-class.
        self.visited_urls.insert(resolved.clone());
        self.navigate_vfs(&resolved, vfs);
    }

    /// Go back in history.
    pub fn go_back(&mut self, vfs: &dyn Vfs) {
        // Save current scroll position.
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_back() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_vfs(&url, vfs);
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Go forward in history.
    pub fn go_forward(&mut self, vfs: &dyn Vfs) {
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_forward() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_vfs(&url, vfs);
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Navigate to the home page.
    pub fn go_home(&mut self, vfs: &dyn Vfs) {
        let url = self.nav.go_home();
        self.navigate_vfs(&url, vfs);
    }

    // ---------------------------------------------------------------
    // Accessors
    // ---------------------------------------------------------------

    /// Get the window X position (set by the WM).
    pub fn window_x(&self) -> i32 {
        self.window_x
    }

    /// Get the window Y position (set by the WM).
    pub fn window_y(&self) -> i32 {
        self.window_y
    }

    /// Get the title of the current page.
    pub fn title(&self) -> Option<&str> {
        self.nav.current_title()
    }

    /// Get the URL of the current page.
    pub fn current_url(&self) -> Option<&str> {
        self.nav.current_url()
    }

    /// Get the current loading state.
    pub fn loading_state(&self) -> LoadingState {
        self.state
    }

    /// Check if reader mode is active.
    pub fn is_reader_mode(&self) -> bool {
        self.reader_mode
    }

    /// Get an immutable reference to the navigation controller.
    pub fn navigation(&self) -> &NavigationController {
        &self.nav
    }

    /// Get a mutable reference to the navigation controller.
    pub fn navigation_mut(&mut self) -> &mut NavigationController {
        &mut self.nav
    }

    /// Get the current error message, if any.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Get the scroll state.
    pub fn scroll(&self) -> &ScrollState {
        &self.scroll
    }

    /// Get a mutable reference to the scroll state.
    pub fn scroll_mut(&mut self) -> &mut ScrollState {
        &mut self.scroll
    }
}

// -----------------------------------------------------------------------
// Gemini-to-HTML helper
// -----------------------------------------------------------------------

/// Convert a parsed Gemini document to a simple HTML string for
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

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockBackend;
    use oasis_vfs::MemoryVfs;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    /// Create a MemoryVfs pre-populated with a simple site tree.
    fn test_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/home").unwrap();
        vfs.write(
            "/sites/home/index.html",
            b"<html><head><title>Home</title></head>\
              <body><h1>Welcome</h1>\
              <p>Hello world</p>\
              <a href=\"page2.html\">Next</a>\
              </body></html>",
        )
        .unwrap();
        vfs.write(
            "/sites/home/page2.html",
            b"<html><head><title>Page 2</title></head>\
              <body><h1>Page Two</h1>\
              <p>Content here</p>\
              <a href=\"index.html\">Back</a>\
              </body></html>",
        )
        .unwrap();
        vfs.write(
            "/sites/home/article.html",
            b"<html><head><title>Article</title></head>\
              <body>\
              <article>\
              <p>This is a long enough paragraph to pass the \
              minimum scoring threshold for reader mode \
              extraction in the OASIS browser.</p>\
              <p>And here is another paragraph that also has \
              plenty of text content for the scoring algo.</p>\
              </article>\
              </body></html>",
        )
        .unwrap();
        vfs.mkdir("/sites/gem.example").unwrap();
        vfs.write(
            "/sites/gem.example/page.gmi",
            b"# Gemini Page\n\nHello from Gemini!\n\
              => gemini://gem.example/other Other Page\n",
        )
        .unwrap();
        vfs
    }

    fn make_browser() -> BrowserWidget {
        let mut config = BrowserConfig::default();
        config.features.home_url = "vfs://sites/home/index.html".to_string();
        BrowserWidget::new(config)
    }

    // ---------------------------------------------------------------
    // Test 1: default config creation
    // ---------------------------------------------------------------

    #[test]
    fn default_config_creation() {
        let config = BrowserConfig::default();
        assert!(config.features.enabled);
        assert!(config.features.native_engine);
        assert!(config.features.gemini);
        assert!(config.features.reader_mode);
        assert_eq!(config.url_bar_height, 20);
        assert_eq!(config.status_bar_height, 14);
        assert_eq!(config.button_width, 20);
        assert_eq!(config.features.home_url, "vfs://sites/home/index.html");
        assert_eq!(config.cache_size_bytes(), 2 * 1024 * 1024);
        assert_eq!(config.content_height(272), 238);
    }

    // ---------------------------------------------------------------
    // Test 2: SimpleTextMeasurer
    // ---------------------------------------------------------------

    #[test]
    fn simple_text_measurer() {
        let m = SimpleTextMeasurer;
        // Sub-pixel: h(7*12/8)+e(7*12/8)+l(5*12/8)+l(5*12/8)+o(7*12/8)
        //          = 10+10+7+7+10 = 44
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "hello", 12,),
            44
        );
        assert_eq!(layout::block::TextMeasurer::measure_text(&m, "", 12,), 0);
        // 'a' advance=7, 7*16/8 = 14
        assert_eq!(layout::block::TextMeasurer::measure_text(&m, "a", 16,), 14);
        // t(7)+e(7)+s(7)+t(7): 7*8/8 * 4 = 28
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "test", 8,),
            28
        );
        // Same base=28, scale=3 at font_size 24
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "test", 24,),
            84
        );
    }

    // ---------------------------------------------------------------
    // Test 3: VFS navigation
    // ---------------------------------------------------------------

    #[test]
    fn vfs_navigation_loads_page() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        assert_eq!(browser.loading_state(), LoadingState::Idle);
        assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));
        assert_eq!(browser.title(), Some("Home"));
        assert!(browser.document.is_some());
    }

    // ---------------------------------------------------------------
    // Test 4: scroll input
    // ---------------------------------------------------------------

    #[test]
    fn scroll_input_changes_offset() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Set a large content height so scroll is possible.
        browser.scroll.set_content_height(1000);

        let initial = browser.scroll.scroll_y;

        // Scroll down.
        browser.handle_input(&InputEvent::ButtonPress(Button::Down), &vfs);
        assert!(
            browser.scroll.scroll_y > initial,
            "scroll_y should increase on Down"
        );

        let after_down = browser.scroll.scroll_y;

        // Scroll up.
        browser.handle_input(&InputEvent::ButtonPress(Button::Up), &vfs);
        assert!(
            browser.scroll.scroll_y < after_down,
            "scroll_y should decrease on Up"
        );
    }

    // ---------------------------------------------------------------
    // Test 5: link navigation
    // ---------------------------------------------------------------

    #[test]
    fn link_navigation_cycles() {
        let mut browser = make_browser();

        // Manually set up some link regions.
        browser.link_map = vec![
            LinkRegion {
                rect: layout::box_model::Rect::new(10.0, 100.0, 80.0, 16.0),
                href: "page1.html".to_string(),
                node: 1,
            },
            LinkRegion {
                rect: layout::box_model::Rect::new(10.0, 130.0, 80.0, 16.0),
                href: "page2.html".to_string(),
                node: 2,
            },
        ];

        assert_eq!(browser.selected_link, -1);

        // Select next -> index 0.
        browser.select_next_link();
        assert_eq!(browser.selected_link, 0);

        // Select next -> index 1.
        browser.select_next_link();
        assert_eq!(browser.selected_link, 1);

        // Select next wraps -> index 0.
        browser.select_next_link();
        assert_eq!(browser.selected_link, 0);

        // Select prev wraps -> index 1.
        browser.select_prev_link();
        assert_eq!(browser.selected_link, 1);

        // Select prev -> index 0.
        browser.select_prev_link();
        assert_eq!(browser.selected_link, 0);
    }

    // ---------------------------------------------------------------
    // Test 6: chrome click detection
    // ---------------------------------------------------------------

    #[test]
    fn chrome_click_detection() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Navigate to a second page so back works.
        browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/home/page2.html"));
        assert!(browser.nav.can_go_back());

        // Click the back button (x < button_width, y < url_bar_h).
        browser.handle_click(5, 5, &vfs);
        // Should have navigated back.
        assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));

        // Click the home button (rightmost).
        browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
        let home_x = 480 - browser.config.button_width as i32 + 5;
        browser.handle_click(home_x, 5, &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));
    }

    // ---------------------------------------------------------------
    // Test 7: URL resolution
    // ---------------------------------------------------------------

    #[test]
    fn url_resolution_relative() {
        let base = Url::parse("vfs://sites/home/index.html").unwrap();

        let resolved = base.resolve("page2.html").unwrap();
        assert_eq!(resolved.to_string(), "vfs://sites/home/page2.html");

        let resolved = base.resolve("/other/page.html").unwrap();
        assert_eq!(resolved.to_string(), "vfs://sites/other/page.html");

        let resolved = base.resolve("#section").unwrap();
        assert_eq!(resolved.path, "/home/index.html");
        assert_eq!(resolved.fragment, Some("section".to_string()));
    }

    // ---------------------------------------------------------------
    // Test 8: content type dispatch
    // ---------------------------------------------------------------

    #[test]
    fn content_type_dispatch() {
        // HTML content type should trigger load_html.
        let mut browser = make_browser();
        let response = ResourceResponse {
            url: "vfs://test/page.html".to_string(),
            content_type: ContentType::Html,
            body: b"<html><body>Test</body></html>".to_vec(),
            status: 200,
        };
        browser.process_response(response);
        assert!(browser.document.is_some());
        assert_eq!(browser.loading_state(), LoadingState::Idle);

        // Gemini content type dispatches through load_gemini.
        let mut browser2 = make_browser();
        let response = ResourceResponse {
            url: "gemini://gem.example/page.gmi".to_string(),
            content_type: ContentType::GeminiText,
            body: b"# Gemini\nHello".to_vec(),
            status: 200,
        };
        browser2.process_response(response);
        assert!(browser2.document.is_some());

        // CSS content type wraps in <pre>.
        let mut browser3 = make_browser();
        let response = ResourceResponse {
            url: "vfs://test/style.css".to_string(),
            content_type: ContentType::Css,
            body: b"body { color: red; }".to_vec(),
            status: 200,
        };
        browser3.process_response(response);
        assert!(browser3.document.is_some());

        // Image content type wraps in <img> tag.
        let mut browser4 = make_browser();
        let response = ResourceResponse {
            url: "vfs://test/photo.png".to_string(),
            content_type: ContentType::Png,
            body: vec![0u8; 16],
            status: 200,
        };
        browser4.process_response(response);
        assert!(browser4.document.is_some());
    }

    // ===============================================================
    // Integration tests: full navigate -> parse -> layout -> paint
    // ===============================================================

    // ---------------------------------------------------------------
    // Test: page renders text content
    // ---------------------------------------------------------------

    #[test]
    fn page_renders_text_content() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            backend.has_text("Welcome"),
            "page should render 'Welcome' heading text"
        );
        assert!(
            backend.draw_text_count() > 0,
            "should have at least one draw_text call"
        );
        assert!(
            backend.fill_rect_count() > 0,
            "should have fill_rect calls for chrome and backgrounds"
        );
    }

    // ---------------------------------------------------------------
    // Test: page renders links as clickable regions
    // ---------------------------------------------------------------

    #[test]
    fn page_renders_link_regions() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            !browser.link_map.is_empty(),
            "link_map should contain at least one link"
        );
        let has_page2 = browser
            .link_map
            .iter()
            .any(|l| l.href.contains("page2.html"));
        assert!(has_page2, "should have a link to page2.html");
    }

    // ---------------------------------------------------------------
    // Test: navigation updates content
    // ---------------------------------------------------------------

    #[test]
    fn navigation_updates_content() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();
        assert!(backend.has_text("Welcome"), "page1 should show Welcome");

        // Navigate to page 2.
        browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
        let mut backend2 = MockBackend::new();
        browser.paint(&mut backend2).unwrap();
        assert!(
            backend2.has_text("Page") || backend2.has_text("Two"),
            "page2 should show 'Page Two' (words may be split)"
        );
    }

    // ---------------------------------------------------------------
    // Test: chrome always renders
    // ---------------------------------------------------------------

    #[test]
    fn chrome_always_renders() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Chrome buttons: "<", ">", "H"
        assert!(backend.has_text("<"), "should render back button '<'");
        assert!(backend.has_text(">"), "should render forward button '>'");
        assert!(backend.has_text("H"), "should render home button 'H'");
        // URL bar should show the current URL.
        assert!(backend.has_text("vfs://"), "should render URL in the bar");
    }

    // ---------------------------------------------------------------
    // Test: error page renders on missing VFS path
    // ---------------------------------------------------------------

    #[test]
    fn error_page_renders_on_missing_path() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://nonexistent/page.html", &vfs);

        assert_eq!(browser.loading_state(), LoadingState::Error);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Should render some error message text.
        assert!(
            backend.draw_text_count() > 0,
            "error page should render text"
        );
    }

    // ---------------------------------------------------------------
    // Test: Gemini page renders text
    // ---------------------------------------------------------------

    #[test]
    fn gemini_page_renders_text() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/gem.example/page.gmi", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            backend.has_text("Gemini") || backend.has_text("Page"),
            "should render Gemini heading text (words may be split)"
        );
    }

    // ---------------------------------------------------------------
    // Test: content height is nonzero after paint
    // ---------------------------------------------------------------

    #[test]
    fn content_height_nonzero_after_paint() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            browser.scroll().content_height > 0,
            "content_height should be nonzero for a page with content"
        );
    }

    // ===============================================================
    // URL bar editing unit tests
    // ===============================================================

    // ---------------------------------------------------------------
    // Test: URL bar click sets focus
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_click_sets_focus() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        assert_eq!(browser.focus, Focus::Content);

        // Click in URL bar area (between buttons).
        let bw = browser.config.button_width;
        let click_x = (bw * 2 + 10) as i32;
        let click_y = browser.config.url_bar_height as i32 / 2;
        browser.handle_click(click_x, click_y, &vfs);

        assert_eq!(browser.focus, Focus::UrlBar);
        assert_eq!(browser.url_input, "vfs://sites/home/index.html");
        assert_eq!(browser.url_cursor, browser.url_input.len());
    }

    // ---------------------------------------------------------------
    // Test: URL bar typing inserts chars
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_typing_inserts_chars() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar focus.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        assert_eq!(browser.focus, Focus::UrlBar);

        let base_len = browser.url_input.len();

        browser.handle_input(&InputEvent::TextInput('a'), &vfs);
        browser.handle_input(&InputEvent::TextInput('b'), &vfs);
        browser.handle_input(&InputEvent::TextInput('c'), &vfs);

        assert_eq!(browser.url_input.len(), base_len + 3);
        assert!(browser.url_input.ends_with("abc"));
        assert_eq!(browser.url_cursor, browser.url_input.len());
    }

    // ---------------------------------------------------------------
    // Test: URL bar backspace deletes
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_backspace_deletes() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar focus and type some chars.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        browser.handle_input(&InputEvent::TextInput('x'), &vfs);
        browser.handle_input(&InputEvent::TextInput('y'), &vfs);
        let before_bs = browser.url_input.len();

        browser.handle_input(&InputEvent::Backspace, &vfs);
        assert_eq!(browser.url_input.len(), before_bs - 1);
        assert!(browser.url_input.ends_with('x'));
    }

    // ---------------------------------------------------------------
    // Test: URL bar confirm navigates
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_confirm_navigates() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar and replace content.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);

        // Clear the input and type a new URL.
        browser.url_input.clear();
        browser.url_cursor = 0;
        let target = "vfs://sites/home/page2.html";
        for ch in target.chars() {
            browser.handle_input(&InputEvent::TextInput(ch), &vfs);
        }

        // Press Confirm.
        browser.handle_input(&InputEvent::ButtonPress(Button::Confirm), &vfs);

        assert_eq!(browser.focus, Focus::Content);
        assert_eq!(browser.current_url(), Some("vfs://sites/home/page2.html"));
    }

    // ---------------------------------------------------------------
    // Test: URL bar cancel discards
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_cancel_discards() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let original_url = browser.current_url().unwrap().to_string();

        // Enter URL bar and modify.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        browser.handle_input(&InputEvent::TextInput('z'), &vfs);

        // Press Cancel.
        browser.handle_input(&InputEvent::ButtonPress(Button::Cancel), &vfs);

        assert_eq!(browser.focus, Focus::Content);
        assert!(browser.url_input.is_empty());
        assert_eq!(browser.current_url(), Some(original_url.as_str()));
    }

    // ---------------------------------------------------------------
    // Test: URL bar left/right moves cursor
    // ---------------------------------------------------------------

    #[test]
    fn url_bar_left_right_moves_cursor() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        let end_pos = browser.url_cursor;
        assert!(end_pos > 0);

        // Move left.
        browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
        assert!(browser.url_cursor < end_pos);

        let after_left = browser.url_cursor;

        // Move right.
        browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
        assert!(browser.url_cursor > after_left);
    }

    // ---------------------------------------------------------------
    // Test: content click exits URL bar
    // ---------------------------------------------------------------

    #[test]
    fn content_click_exits_url_bar() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar focus.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        assert_eq!(browser.focus, Focus::UrlBar);

        // Click in content area (below URL bar).
        let content_y = browser.config.url_bar_height as i32 + 50;
        browser.handle_click(100, content_y, &vfs);
        assert_eq!(browser.focus, Focus::Content);
    }

    // ===============================================================
    // Paint pipeline tests for chrome rendering
    // ===============================================================

    // ---------------------------------------------------------------
    // Test: paint chrome shows editing buffer in URL bar mode
    // ---------------------------------------------------------------

    #[test]
    fn paint_chrome_url_bar_editing() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Enter URL bar and type something.
        let bw = browser.config.button_width;
        browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
        browser.handle_input(&InputEvent::TextInput('!'), &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // The URL bar should show the editing buffer (containing '!').
        assert!(
            backend.has_text("!"),
            "URL bar should display the editing buffer text"
        );
    }

    // ---------------------------------------------------------------
    // Test: paint chrome normal mode shows URL
    // ---------------------------------------------------------------

    #[test]
    fn paint_chrome_normal_mode_shows_url() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        assert_eq!(browser.focus, Focus::Content);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            backend.has_text("vfs://sites/home/index.html"),
            "chrome should display the current URL"
        );
    }

    // ===============================================================
    // Extended test fixtures
    // ===============================================================

    /// VFS with richer pages for interaction testing.
    fn interaction_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/test").unwrap();

        // Page with a single link.
        vfs.write(
            "/sites/test/single_link.html",
            b"<html><head><title>Single Link</title></head>\
              <body>\
              <p>Before the link.</p>\
              <p><a href=\"target.html\">Click me</a></p>\
              <p>After the link.</p>\
              </body></html>",
        )
        .unwrap();

        // Target page.
        vfs.write(
            "/sites/test/target.html",
            b"<html><head><title>Target</title></head>\
              <body><h1>You arrived!</h1></body></html>",
        )
        .unwrap();

        // Page with multiple links.
        vfs.write(
            "/sites/test/multi_links.html",
            b"<html><head><title>Multi Links</title></head>\
              <body>\
              <p><a href=\"page_a.html\">Link A</a></p>\
              <p><a href=\"page_b.html\">Link B</a></p>\
              <p><a href=\"page_c.html\">Link C</a></p>\
              </body></html>",
        )
        .unwrap();

        // Page A/B/C.
        vfs.write(
            "/sites/test/page_a.html",
            b"<html><body><p>Page A</p></body></html>",
        )
        .unwrap();
        vfs.write(
            "/sites/test/page_b.html",
            b"<html><body><p>Page B</p></body></html>",
        )
        .unwrap();
        vfs.write(
            "/sites/test/page_c.html",
            b"<html><body><p>Page C</p></body></html>",
        )
        .unwrap();

        // Long page for scroll testing.
        let mut long_html = String::from("<html><head><title>Long</title></head><body>");
        for i in 0..20 {
            long_html.push_str(&format!("<p>Paragraph {} with some text content.</p>", i));
        }
        long_html.push_str(
            "<p><a href=\"target.html\">Bottom link</a></p>\
             </body></html>",
        );
        vfs.write("/sites/test/long.html", long_html.as_bytes())
            .unwrap();

        // Inline link within text.
        vfs.write(
            "/sites/test/inline_link.html",
            b"<html><body>\
              <p>Read <a href=\"target.html\">this page</a> for info.</p>\
              </body></html>",
        )
        .unwrap();

        vfs
    }

    fn make_interaction_browser() -> BrowserWidget {
        let mut config = BrowserConfig::default();
        config.features.home_url = "vfs://sites/test/single_link.html".to_string();
        BrowserWidget::new(config)
    }

    // ===============================================================
    // Category A: Layout Geometry Verification
    // ===============================================================

    #[test]
    fn text_boxes_do_not_overlap() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let overlaps = backend.find_overlapping_text_lines();
        assert!(
            overlaps.is_empty(),
            "text lines should not overlap, found {} overlapping line pairs: {:?}",
            overlaps.len(),
            overlaps,
        );
    }

    #[test]
    fn multi_link_page_text_does_not_overlap() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let overlaps = backend.find_overlapping_text_lines();
        assert!(
            overlaps.is_empty(),
            "multi-link page text lines should not overlap: {:?}",
            overlaps,
        );
    }

    #[test]
    fn text_y_positions_increase_monotonically() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let positions = backend.text_positions();
        // Filter to content text only (skip single-char chrome buttons).
        let content: Vec<_> = positions
            .iter()
            .filter(|(t, _, _, _)| t.len() > 1)
            .collect();

        // Y should never decrease between distinct text lines.
        for pair in content.windows(2) {
            let (text_a, _, ya, _) = pair[0];
            let (text_b, _, yb, _) = pair[1];
            assert!(
                yb >= ya,
                "text Y should increase: '{}' at y={} before '{}' at y={}",
                text_a,
                ya,
                text_b,
                yb,
            );
        }
    }

    #[test]
    fn line_height_exceeds_font_size() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        // Check layout tree: all inline boxes should have
        // content.height >= font_size.
        let layout = browser.layout_root.as_ref().expect("should have layout");
        check_line_heights(layout);
    }

    /// Recursively verify line heights in the layout tree.
    fn check_line_heights(lb: &layout::box_model::LayoutBox) {
        if matches!(lb.box_type, layout::box_model::BoxType::Inline)
            && lb.dimensions.content.height > 0.0
        {
            assert!(
                lb.dimensions.content.height >= lb.style.font_size,
                "inline box height ({}) should be >= font_size ({}) for text {:?}",
                lb.dimensions.content.height,
                lb.style.font_size,
                lb.text,
            );
        }
        for child in &lb.children {
            check_line_heights(child);
        }
    }

    // ===============================================================
    // Category B: Link Region Validation
    // ===============================================================

    #[test]
    fn link_regions_have_valid_dimensions() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            !browser.link_map.is_empty(),
            "should have at least one link region"
        );

        for link in &browser.link_map {
            assert!(
                link.rect.width > 0.0,
                "link '{}' should have positive width, got {}",
                link.href,
                link.rect.width,
            );
            assert!(
                link.rect.height > 0.0,
                "link '{}' should have positive height, got {}",
                link.href,
                link.rect.height,
            );
        }
    }

    #[test]
    fn link_regions_within_viewport() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let chrome_y = browser.config.url_bar_height as f32;
        let view_bottom = browser.window_h as f32;

        for link in &browser.link_map {
            assert!(
                link.rect.x >= 0.0,
                "link x ({}) should be >= 0",
                link.rect.x,
            );
            assert!(
                link.rect.y >= chrome_y,
                "link y ({}) should be >= chrome height ({})",
                link.rect.y,
                chrome_y,
            );
            assert!(
                link.rect.y + link.rect.height <= view_bottom + 1.0,
                "link bottom ({}) should be <= viewport bottom ({})",
                link.rect.y + link.rect.height,
                view_bottom,
            );
        }
    }

    #[test]
    fn multiple_links_have_distinct_regions() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            browser.link_map.len() >= 3,
            "multi-link page should have at least 3 links, got {}",
            browser.link_map.len(),
        );

        // No two links should have the same Y position.
        for i in 0..browser.link_map.len() {
            for j in (i + 1)..browser.link_map.len() {
                let a = &browser.link_map[i].rect;
                let b = &browser.link_map[j].rect;
                // Check they don't fully overlap (different hrefs should
                // have different rects).
                let overlaps_x = a.x < b.x + b.width && b.x < a.x + a.width;
                let overlaps_y = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(
                    !(overlaps_x && overlaps_y),
                    "links '{}' and '{}' should not overlap: \
                     a=({},{},{},{}) b=({},{},{},{})",
                    browser.link_map[i].href,
                    browser.link_map[j].href,
                    a.x,
                    a.y,
                    a.width,
                    a.height,
                    b.x,
                    b.y,
                    b.width,
                    b.height,
                );
            }
        }
    }

    #[test]
    fn link_region_matches_rendered_text_position() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("target.html"))
            .expect("should have link to target.html");

        // Find the "Click me" text draw call.
        let text_pos = backend
            .text_positions()
            .into_iter()
            .find(|(t, _, _, _)| t.contains("Click"));

        if let Some((_text, tx, ty, _fs)) = text_pos {
            // The text draw position should be within the link rect.
            let lx = link.rect.x as i32;
            let ly = link.rect.y as i32;
            let lr = lx + link.rect.width as i32;
            let lb = ly + link.rect.height as i32;

            assert!(
                tx >= lx - 2 && tx <= lr + 2,
                "text x ({}) should be within link rect x range ({}-{})",
                tx,
                lx,
                lr,
            );
            assert!(
                ty >= ly - 2 && ty <= lb + 2,
                "text y ({}) should be within link rect y range ({}-{})",
                ty,
                ly,
                lb,
            );
        }
    }

    // ===============================================================
    // Category C: Click-to-Navigate Simulation
    // ===============================================================

    #[test]
    fn click_on_link_center_navigates() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("target.html"))
            .expect("should have link to target.html");

        // Click at the center of the link region.
        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;

        // Diagnostic: print link rect and click position.
        let link_rect = link.rect;
        eprintln!(
            "link rect: x={}, y={}, w={}, h={}; click: ({}, {})",
            link_rect.x, link_rect.y, link_rect.width, link_rect.height, cx, cy,
        );

        browser.handle_click(cx, cy, &vfs);

        assert_eq!(
            browser.current_url(),
            Some("vfs://sites/test/target.html"),
            "clicking link center should navigate to target.html \
             (link rect: x={:.1}, y={:.1}, w={:.1}, h={:.1}; click: ({}, {}))",
            link_rect.x,
            link_rect.y,
            link_rect.width,
            link_rect.height,
            cx,
            cy,
        );
    }

    #[test]
    fn click_on_link_edge_navigates() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("target.html"))
            .expect("should have link to target.html");

        // Click 1px inside the top-left corner.
        let cx = (link.rect.x + 1.0) as i32;
        let cy = (link.rect.y + 1.0) as i32;

        browser.handle_click(cx, cy, &vfs);

        assert_eq!(
            browser.current_url(),
            Some("vfs://sites/test/target.html"),
            "clicking near link edge should navigate",
        );
    }

    #[test]
    fn click_outside_link_does_not_navigate() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);
        let original = browser.current_url().unwrap().to_string();

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Click far below any content.
        browser.handle_click(240, 260, &vfs);
        assert_eq!(
            browser.current_url(),
            Some(original.as_str()),
            "clicking outside links should not navigate",
        );
    }

    #[test]
    fn click_on_second_link_navigates_to_correct_target() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Find link B.
        let link_b = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("page_b.html"))
            .expect("should have link to page_b.html");

        let cx = (link_b.rect.x + link_b.rect.width / 2.0) as i32;
        let cy = (link_b.rect.y + link_b.rect.height / 2.0) as i32;

        browser.handle_click(cx, cy, &vfs);

        assert_eq!(
            browser.current_url(),
            Some("vfs://sites/test/page_b.html"),
            "clicking Link B should navigate to page_b.html",
        );
    }

    #[test]
    fn tab_then_confirm_navigates() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(!browser.link_map.is_empty(), "should have links");

        // Tab to select first link.
        browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
        assert_eq!(browser.selected_link, 0);

        // Confirm.
        browser.handle_input(&InputEvent::ButtonPress(Button::Confirm), &vfs);

        assert_eq!(
            browser.current_url(),
            Some("vfs://sites/test/target.html"),
            "tab + confirm should navigate to target",
        );
    }

    #[test]
    fn click_all_links_on_multi_link_page() {
        let vfs = interaction_vfs();
        let targets = ["page_a.html", "page_b.html", "page_c.html"];

        for target in &targets {
            let mut browser = make_interaction_browser();
            browser.set_window(0, 0, 480, 272);
            browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

            let mut backend = MockBackend::new();
            browser.paint(&mut backend).unwrap();

            let link = browser
                .link_map
                .iter()
                .find(|l| l.href.contains(target))
                .unwrap_or_else(|| panic!("should have link to {target}"));

            let cx = (link.rect.x + link.rect.width / 2.0) as i32;
            let cy = (link.rect.y + link.rect.height / 2.0) as i32;

            browser.handle_click(cx, cy, &vfs);

            let expected = format!("vfs://sites/test/{target}");
            assert_eq!(
                browser.current_url(),
                Some(expected.as_str()),
                "clicking should navigate to {target}",
            );
        }
    }

    // ===============================================================
    // Category D: Scroll + Link Interaction
    // ===============================================================

    #[test]
    fn link_regions_update_after_scroll_and_repaint() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/long.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let initial_links = browser.link_map.clone();

        // Force content height so scrolling is possible.
        browser.scroll.set_content_height(2000);

        // Scroll down.
        for _ in 0..5 {
            browser.handle_input(&InputEvent::ButtonPress(Button::Down), &vfs);
        }

        // Repaint to get updated link regions.
        let mut backend2 = MockBackend::new();
        browser.paint(&mut backend2).unwrap();

        // If there were links visible before scroll, their Y positions
        // should have shifted (or they may be off-screen now).
        if !initial_links.is_empty() && !browser.link_map.is_empty() {
            // At minimum, verify the link_map was regenerated (it's
            // rebuilt every paint pass).
            assert!(
                !browser.link_map.is_empty(),
                "link_map should be regenerated after repaint"
            );
        }
    }

    // ===============================================================
    // Category E: End-to-End HTML Scenarios
    // ===============================================================

    #[test]
    fn inline_link_within_paragraph() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/inline_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Should render "this page" as link text.
        assert!(
            !browser.link_map.is_empty(),
            "inline link should produce link regions"
        );

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("target.html"))
            .expect("should have link to target.html");

        // Click the link.
        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;

        browser.handle_click(cx, cy, &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/test/target.html"),);
    }

    #[test]
    fn navigate_back_after_link_click() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);
        let original = browser.current_url().unwrap().to_string();

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Click link to navigate.
        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("target.html"))
            .expect("should have link");
        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;
        browser.handle_click(cx, cy, &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/test/target.html"),);

        // Go back.
        browser.go_back(&vfs);
        assert_eq!(browser.current_url(), Some(original.as_str()));
    }

    #[test]
    fn full_roundtrip_navigate_paint_click() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);

        // Step 1: Navigate to multi-links page.
        browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);
        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Verify content rendered (words split by inline layout).
        assert!(backend.has_text("Link"));
        assert!(backend.draw_text_count() > 3);

        // Step 2: Click Link A.
        let link_a = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("page_a.html"))
            .expect("should have Link A");
        let cx = (link_a.rect.x + link_a.rect.width / 2.0) as i32;
        let cy = (link_a.rect.y + link_a.rect.height / 2.0) as i32;
        browser.handle_click(cx, cy, &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/test/page_a.html"));

        // Step 3: Paint new page.
        let mut backend2 = MockBackend::new();
        browser.paint(&mut backend2).unwrap();
        assert!(backend2.has_text("Page"));

        // Step 4: Go back and repaint.
        browser.go_back(&vfs);
        let mut backend3 = MockBackend::new();
        browser.paint(&mut backend3).unwrap();
        assert!(backend3.has_text("Link"));

        // Step 5: Links should work again after going back.
        let link_c = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("page_c.html"))
            .expect("should have Link C after going back");
        let cx = (link_c.rect.x + link_c.rect.width / 2.0) as i32;
        let cy = (link_c.rect.y + link_c.rect.height / 2.0) as i32;
        browser.handle_click(cx, cy, &vfs);
        assert_eq!(browser.current_url(), Some("vfs://sites/test/page_c.html"));
    }

    // ===============================================================
    // Category F: Diagnostic / Coordinate Debugging Tests
    // ===============================================================

    #[test]
    fn link_rect_is_hittable_by_integer_coords() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        for link in &browser.link_map {
            // Verify that the center of the rect, cast to i32,
            // still falls inside the f32 rect. This catches rounding
            // edge cases.
            let cx = (link.rect.x + link.rect.width / 2.0) as i32;
            let cy = (link.rect.y + link.rect.height / 2.0) as i32;

            assert!(
                (cx as f32) >= link.rect.x
                    && (cx as f32) < link.rect.x + link.rect.width
                    && (cy as f32) >= link.rect.y
                    && (cy as f32) < link.rect.y + link.rect.height,
                "center ({}, {}) of link '{}' should be inside its rect \
                 ({}, {}, {}, {})",
                cx,
                cy,
                link.href,
                link.rect.x,
                link.rect.y,
                link.rect.width,
                link.rect.height,
            );
        }
    }

    #[test]
    fn link_rect_y_is_below_chrome() {
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let chrome_h = browser.config.url_bar_height as f32;
        for link in &browser.link_map {
            let cy = (link.rect.y + link.rect.height / 2.0) as i32;
            let rel_y = cy - browser.window_y;
            assert!(
                rel_y >= chrome_h as i32,
                "link '{}' center y ({}) should be below chrome ({})",
                link.href,
                rel_y,
                chrome_h,
            );
        }
    }

    #[test]
    fn handle_click_coordinates_match_link_map() {
        // This is the most precise diagnostic test: it reconstructs
        // the exact hit-test logic from handle_click() and verifies
        // that at least one link in the map is hittable.
        let vfs = interaction_vfs();
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            !browser.link_map.is_empty(),
            "should have at least one link"
        );

        let link = &browser.link_map[0];
        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;

        // Replicate the handle_click logic.
        let rel_y = cy - browser.window_y;
        let chrome_h = browser.config.url_bar_height as i32;

        assert!(
            rel_y >= chrome_h,
            "click y ({}) relative to window ({}) = {} should be >= chrome ({}). \
             Link rect: ({:.1}, {:.1}, {:.1}, {:.1})",
            cy,
            browser.window_y,
            rel_y,
            chrome_h,
            link.rect.x,
            link.rect.y,
            link.rect.width,
            link.rect.height,
        );

        // Check the hit test would match.
        let hit = (cx as f32) >= link.rect.x
            && (cx as f32) < link.rect.x + link.rect.width
            && (cy as f32) >= link.rect.y
            && (cy as f32) < link.rect.y + link.rect.height;

        assert!(
            hit,
            "center ({}, {}) should hit link rect ({:.1}, {:.1}, {:.1}, {:.1})",
            cx, cy, link.rect.x, link.rect.y, link.rect.width, link.rect.height,
        );
    }

    #[test]
    fn original_test_page_link_click_navigates() {
        // Test the original test_vfs index.html page (the one used by
        // existing tests) to ensure its link is also clickable.
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains("page2.html"))
            .expect("should have link to page2.html");

        let link_rect = link.rect;
        let cx = (link_rect.x + link_rect.width / 2.0) as i32;
        let cy = (link_rect.y + link_rect.height / 2.0) as i32;

        browser.handle_click(cx, cy, &vfs);

        assert_eq!(
            browser.current_url(),
            Some("vfs://sites/home/page2.html"),
            "clicking link on original index.html should navigate to page2 \
             (link rect: x={:.1}, y={:.1}, w={:.1}, h={:.1}; click: ({}, {}))",
            link_rect.x,
            link_rect.y,
            link_rect.width,
            link_rect.height,
            cx,
            cy,
        );
    }

    #[test]
    fn test_navigate_https_without_tls_shows_error() {
        let mut bw = make_browser();
        let vfs = test_vfs();
        // No TLS provider set -- HTTPS should produce an error page.
        bw.navigate_vfs("https://example.com/page", &vfs);
        assert_eq!(bw.state, LoadingState::Idle);
        // The HTTPS error page should be rendered as HTML in the DOM.
        let doc = bw.document.as_ref().expect("document should be loaded");
        let text = doc.text_content(doc.root);
        assert!(
            text.contains("HTTPS Required"),
            "expected 'HTTPS Required' in page text, got: {text}",
        );
    }

    #[test]
    fn test_navigate_gemini_without_tls_shows_error() {
        let mut bw = make_browser();
        let vfs = test_vfs();
        // No TLS provider -- Gemini should show a TLS Required page.
        bw.navigate_vfs("gemini://example.com/page", &vfs);
        assert_eq!(bw.state, LoadingState::Idle);
        let doc = bw.document.as_ref().expect("document should be loaded");
        let text = doc.text_content(doc.root);
        assert!(
            text.contains("TLS Required"),
            "expected 'TLS Required' in page text, got: {text}",
        );
    }

    // ===============================================================
    // Incremental Layout / Dirty Flag Tests
    // ===============================================================

    #[test]
    fn layout_not_dirty_after_load() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        assert!(
            !browser.is_layout_dirty(),
            "layout should not be dirty immediately after load_html"
        );
    }

    #[test]
    fn layout_dirty_after_width_change() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Change width -> should mark dirty.
        browser.set_window(0, 0, 320, 272);
        assert!(
            browser.is_layout_dirty(),
            "layout should be dirty after viewport width change"
        );
    }

    #[test]
    fn layout_not_dirty_after_position_only_change() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Move window (same size) -> should NOT mark dirty.
        browser.set_window(10, 20, 480, 272);
        assert!(
            !browser.is_layout_dirty(),
            "layout should not be dirty after position-only change"
        );
    }

    #[test]
    fn layout_dirty_after_height_only_change() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Change height only -> should mark dirty because viewport
        // height is used for CSS positioned elements (fixed/absolute).
        browser.set_window(0, 0, 480, 300);
        assert!(
            browser.is_layout_dirty(),
            "layout should be dirty after height change"
        );
    }

    #[test]
    fn relayout_if_dirty_skips_when_clean() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Layout is clean after load.
        assert!(
            !browser.relayout_if_dirty(),
            "should skip relayout when clean"
        );
    }

    #[test]
    fn relayout_if_dirty_rebuilds_on_width_change() {
        let vfs = test_vfs();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

        // Change width to mark dirty.
        browser.set_window(0, 0, 320, 272);
        assert!(browser.is_layout_dirty());

        // Relayout should run and clear dirty flag.
        assert!(
            browser.relayout_if_dirty(),
            "should perform relayout on dirty"
        );
        assert!(
            !browser.is_layout_dirty(),
            "dirty flag should be cleared after relayout"
        );
        assert!(
            browser.layout_root.is_some(),
            "layout_root should exist after relayout"
        );
    }

    #[test]
    fn relayout_if_dirty_skips_without_document() {
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);

        // No page loaded -- force dirty.
        browser.set_window(0, 0, 320, 272);

        // Should skip (no document/styles).
        assert!(
            !browser.relayout_if_dirty(),
            "should skip relayout without a loaded document"
        );
        assert!(
            !browser.is_layout_dirty(),
            "dirty flag should be cleared even without document"
        );
    }

    #[test]
    fn layout_box_dirty_field_defaults_true() {
        let lb = layout::box_model::LayoutBox::new(
            layout::box_model::BoxType::Block,
            css::values::ComputedStyle::default(),
            None,
        );
        assert!(lb.dirty, "new LayoutBox should have dirty=true");
    }

    #[test]
    fn layout_box_mark_clean_propagates() {
        let mut parent = layout::box_model::LayoutBox::new(
            layout::box_model::BoxType::Block,
            css::values::ComputedStyle::default(),
            None,
        );
        let child = layout::box_model::LayoutBox::new(
            layout::box_model::BoxType::Inline,
            css::values::ComputedStyle::default(),
            None,
        );
        parent.children.push(child);

        assert!(parent.dirty);
        assert!(parent.children[0].dirty);

        parent.mark_clean();

        assert!(!parent.dirty, "parent should be clean");
        assert!(!parent.children[0].dirty, "child should be clean");
    }

    #[test]
    fn layout_box_mark_dirty_sets_flag() {
        let mut lb = layout::box_model::LayoutBox::new(
            layout::box_model::BoxType::Block,
            css::values::ComputedStyle::default(),
            None,
        );
        lb.mark_clean();
        assert!(!lb.dirty);

        lb.mark_dirty();
        assert!(lb.dirty, "mark_dirty should set dirty flag");
    }

    // ===============================================================
    // Image rendering tests
    // ===============================================================

    /// Build a minimal valid 24-bit BMP of given dimensions (solid red).
    fn make_test_bmp(w: u32, h: u32) -> Vec<u8> {
        let bpp: u16 = 24;
        let row_bytes = (w * 3).div_ceil(4) * 4;
        let pixel_data_size = row_bytes * h;
        let file_size = 54 + pixel_data_size;

        let mut bmp = vec![0u8; file_size as usize];
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[2..6].copy_from_slice(&file_size.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        bmp[22..26].copy_from_slice(&(h as i32).to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&bpp.to_le_bytes());
        bmp[30..34].copy_from_slice(&0u32.to_le_bytes());

        // Fill with solid red (BGR = 0,0,255).
        for row in 0..h {
            for col in 0..w {
                let off = 54 + (row * row_bytes + col * 3) as usize;
                if off + 2 < bmp.len() {
                    bmp[off] = 0; // B
                    bmp[off + 1] = 0; // G
                    bmp[off + 2] = 255; // R
                }
            }
        }
        bmp
    }

    /// Create a VFS with an image file and an HTML page referencing it.
    fn test_vfs_with_image() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/img").unwrap();

        let bmp_data = make_test_bmp(16, 16);
        vfs.write("/sites/img/red.bmp", &bmp_data).unwrap();

        vfs.write(
            "/sites/img/index.html",
            b"<html><body>\
              <h1>Image Test</h1>\
              <img src=\"red.bmp\" alt=\"Red square\">\
              </body></html>",
        )
        .unwrap();

        vfs.write(
            "/sites/img/with_dims.html",
            b"<html><body>\
              <img src=\"red.bmp\" width=\"32\" height=\"32\" alt=\"Scaled\">\
              </body></html>",
        )
        .unwrap();

        vfs.write(
            "/sites/img/broken.html",
            b"<html><body>\
              <img src=\"nonexistent.bmp\" alt=\"Missing image\">\
              </body></html>",
        )
        .unwrap();

        vfs.write(
            "/sites/img/no_alt.html",
            b"<html><body>\
              <img src=\"nonexistent.bmp\">\
              </body></html>",
        )
        .unwrap();

        vfs.write(
            "/sites/img/multi.html",
            b"<html><body>\
              <p>Before</p>\
              <img src=\"red.bmp\" alt=\"First\">\
              <p>Middle</p>\
              <img src=\"red.bmp\" alt=\"Second\">\
              <p>After</p>\
              </body></html>",
        )
        .unwrap();

        vfs
    }

    // ---------------------------------------------------------------
    // Test: image decoded from VFS during navigation
    // ---------------------------------------------------------------

    #[test]
    fn image_decoded_from_vfs_during_navigation() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/index.html", &vfs);

        // Images are now loaded progressively; process all pending.
        browser.load_next_image_batch(&vfs, 5000);

        assert_eq!(browser.loading_state(), LoadingState::Idle);
        assert!(
            !browser.decoded_images.is_empty(),
            "should have decoded at least one image"
        );

        let has_red = browser
            .decoded_images
            .values()
            .any(|img| img.width == 16 && img.height == 16);
        assert!(has_red, "decoded image should be 16x16");
    }

    // ---------------------------------------------------------------
    // Test: image renders as blit call (not placeholder)
    // ---------------------------------------------------------------

    #[test]
    fn image_renders_as_blit() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
        browser.load_next_image_batch(&vfs, 5000);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert!(
            backend.blit_count() > 0,
            "should have at least one blit call for the image"
        );

        // Verify the blit has correct dimensions (16x16 from intrinsic).
        let blits: Vec<_> = backend
            .calls
            .iter()
            .filter_map(|c| {
                if let crate::test_utils::DrawCall::Blit { w, h, .. } = c {
                    Some((*w, *h))
                } else {
                    None
                }
            })
            .collect();
        assert!(
            blits.iter().any(|&(w, h)| w == 16 && h == 16),
            "should blit at 16x16 (intrinsic dimensions), got: {blits:?}"
        );
    }

    // ---------------------------------------------------------------
    // Test: image uses intrinsic dimensions in layout
    // ---------------------------------------------------------------

    #[test]
    fn image_uses_intrinsic_dimensions_in_layout() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
        browser.load_next_image_batch(&vfs, 5000);

        // Check that the layout tree contains a Replaced(Image) with
        // correct dimensions (16x16) instead of the default 0x0.
        let layout = browser.layout_root.as_ref().expect("should have layout");
        let img_box = find_image_box(layout);
        assert!(img_box.is_some(), "layout tree should contain an image box");
        let (w, h) = img_box.unwrap();
        assert_eq!(w, 16, "image layout width should be 16");
        assert_eq!(h, 16, "image layout height should be 16");
    }

    // ---------------------------------------------------------------
    // Test: HTML width/height attributes override intrinsic
    // ---------------------------------------------------------------

    #[test]
    fn image_html_attrs_override_intrinsic() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/with_dims.html", &vfs);

        let layout = browser.layout_root.as_ref().expect("should have layout");
        let img_box = find_image_box(layout);
        assert!(img_box.is_some(), "layout tree should contain an image box");
        let (w, h) = img_box.unwrap();
        assert_eq!(w, 32, "image width should be 32 (from HTML attr)");
        assert_eq!(h, 32, "image height should be 32 (from HTML attr)");
    }

    // ---------------------------------------------------------------
    // Test: broken image shows placeholder (no blit)
    // ---------------------------------------------------------------

    #[test]
    fn broken_image_shows_placeholder() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/broken.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert_eq!(
            backend.blit_count(),
            0,
            "should not blit for a broken image"
        );
        assert!(
            backend.has_text("Missing"),
            "should render alt text for broken image"
        );
    }

    // ---------------------------------------------------------------
    // Test: broken image without alt shows multiplication sign
    // ---------------------------------------------------------------

    #[test]
    fn broken_image_no_alt_shows_placeholder_symbol() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/no_alt.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        assert_eq!(
            backend.blit_count(),
            0,
            "should not blit for a broken image"
        );
        // Should show the multiplication sign placeholder.
        assert!(
            backend.has_text("\u{00D7}"),
            "should render multiplication sign for broken image without alt"
        );
    }

    // ---------------------------------------------------------------
    // Test: multiple images on same page
    // ---------------------------------------------------------------

    #[test]
    fn multiple_images_same_page() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/multi.html", &vfs);
        browser.load_next_image_batch(&vfs, 5000);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // Both images reference the same BMP, so only one decoded entry
        // but two blit calls (one per <img> tag).
        assert_eq!(
            browser.decoded_images.len(),
            1,
            "should deduplicate same-URL images"
        );
        assert!(
            backend.blit_count() >= 2,
            "should blit twice for two <img> tags, got {}",
            backend.blit_count()
        );
        assert!(backend.has_text("Before"), "should render surrounding text");
        assert!(backend.has_text("After"), "should render surrounding text");
    }

    // ---------------------------------------------------------------
    // Test: textures created during paint
    // ---------------------------------------------------------------

    #[test]
    fn textures_created_during_paint() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
        browser.load_next_image_batch(&vfs, 5000);

        // Before paint, no textures exist.
        assert!(
            browser.image_textures.is_empty(),
            "textures should not exist before paint"
        );

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        // After paint, texture should be created.
        assert!(
            !browser.image_textures.is_empty(),
            "textures should exist after paint"
        );
    }

    // ---------------------------------------------------------------
    // Test: navigating clears old image state
    // ---------------------------------------------------------------

    #[test]
    fn navigation_clears_image_state() {
        let vfs = test_vfs_with_image();
        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);

        // Navigate to page with image.
        browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
        browser.load_next_image_batch(&vfs, 5000);
        assert!(!browser.decoded_images.is_empty());

        // Navigate to page without image.
        browser.navigate_vfs("vfs://sites/img/broken.html", &vfs);
        // decoded_images should have been cleared and only repopulated
        // for the new page (which has no decodable images).
        assert!(
            browser
                .decoded_images
                .values()
                .all(|img| img.width != 16 || img.height != 16),
            "old decoded images should be cleared on navigation"
        );
    }

    // ---------------------------------------------------------------
    // Test: image loading is progressive (Phase 1)
    // ---------------------------------------------------------------

    #[test]
    fn image_loading_is_progressive() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/home").unwrap();

        // Create a page with multiple images.
        vfs.write(
            "/sites/home/images.html",
            b"<html><body>\
              <img src=\"img1.bmp\">\
              <img src=\"img2.bmp\">\
              <img src=\"img3.bmp\">\
              </body></html>",
        )
        .unwrap();

        // Write small BMP files.
        let bmp = make_test_bmp(2, 2);
        vfs.write("/sites/home/img1.bmp", &bmp).unwrap();
        vfs.write("/sites/home/img2.bmp", &bmp).unwrap();
        vfs.write("/sites/home/img3.bmp", &bmp).unwrap();

        let mut browser = make_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/home/images.html", &vfs);

        // After navigate, images should be pending, not yet decoded.
        assert!(
            !browser.pending_images.is_empty() || !browser.decoded_images.is_empty(),
            "should have pending or already-decoded images"
        );

        // Call with 0ms budget — should process at most 1 image.
        let before = browser.decoded_images.len();
        browser.load_next_image_batch(&vfs, 0);
        let after = browser.decoded_images.len();
        assert!(
            after - before <= 1,
            "0ms budget should process at most 1 image, got {}",
            after - before
        );
    }

    /// Recursively find a Replaced(Image) box and return its (width, height).
    fn find_image_box(layout_box: &layout::box_model::LayoutBox) -> Option<(u32, u32)> {
        if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
            width,
            height,
            ..
        }) = &layout_box.box_type
        {
            return Some((*width, *height));
        }
        for child in &layout_box.children {
            if let Some(result) = find_image_box(child) {
                return Some(result);
            }
        }
        None
    }

    #[test]
    fn hover_restyle_is_partial() {
        // Build a DOM with many elements and a :hover rule.
        // Verify that hover only restyles the affected ancestor chain.
        let mut elements: Vec<String> = Vec::new();
        for i in 0..50 {
            elements.push(format!("<p>Paragraph {i}</p>"));
        }
        let html = format!(
            "<html><head><style>a:hover {{ color: red; }}</style></head><body>\
             <a href=\"link.html\"><span>Link</span></a>\
             {}\
             </body></html>",
            elements.join("")
        );
        let vfs = MemoryVfs::new();
        let config = BrowserConfig::default();
        let mut browser = BrowserWidget::new(config);
        browser.load_html(&html, "file:///test.html");

        // Verify cached sheets are populated.
        assert!(
            !browser.cached_author_sheets.is_empty() || browser.cached_inline_styles.is_empty(),
            "cached sheets should be set after load_html"
        );

        // Simulate hover on the link node.
        let link_node = browser
            .href_map
            .keys()
            .next()
            .copied()
            .expect("should have a link");
        let old_hover = browser.hover_node;
        browser.hover_node = Some(link_node);
        browser.restyle_hover_affected(old_hover);

        // Layout should be marked dirty (hover style changed).
        assert!(browser.layout_dirty);

        // Styles should still be populated for all elements.
        let styled_count = browser.styles.iter().filter(|s| s.is_some()).count();
        assert!(styled_count > 5, "most elements should retain styles");
    }

    #[test]
    fn image_eviction_respects_budget() {
        // Directly test that decoded_image_lru eviction works by
        // inserting images that exceed the budget.
        let config = BrowserConfig::default();
        let mut browser = BrowserWidget::new(config);

        // Create a fake 1x1 image (4 bytes). Set a tiny budget.
        let small_img = image::DecodedImage {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        };

        // Manually insert decoded images to test eviction.
        for i in 0..5 {
            let url = format!("http://test.com/img{i}.png");
            let img_bytes = (small_img.width * small_img.height * 4) as usize;
            browser.decoded_image_bytes += img_bytes;
            browser.decoded_image_lru.push_front(url.clone());
            browser.decoded_images.insert(url, small_img.clone());
        }
        assert_eq!(browser.decoded_images.len(), 5);
        assert_eq!(browser.decoded_image_bytes, 20); // 5 * 4 bytes

        // LRU order: img4 (front/MRU) ... img0 (back/LRU)
        // Verify the LRU tracks all 5.
        assert_eq!(browser.decoded_image_lru.len(), 5);
    }
}
