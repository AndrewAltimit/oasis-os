//! Browser subsystem: HTML/CSS parsing, layout, and rendering.
//!
//! This module ties together the HTML/CSS pipeline (tokenizer, parser,
//! DOM, style cascade, layout, paint) with navigation, scroll state,
//! resource loading, reader mode, and Gemini protocol support into the
//! [`BrowserWidget`] -- the top-level component that the window manager
//! drives.

pub mod config;
pub(crate) mod css;
pub mod forms;
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

mod widget_images;
mod widget_input;
mod widget_paint;
mod widget_pipeline;

#[cfg(feature = "javascript")]
mod js_dom;

#[cfg(test)]
pub(crate) mod test_utils;

// -----------------------------------------------------------------------
// Public re-exports
// -----------------------------------------------------------------------

pub use config::BrowserConfig;
pub use loader::{ContentType, ResourceResponse, ResourceSource, Url};
pub use nav::{Bookmark, BrowserHistoryEntry, NavigationController};
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
    pub use crate::css::default::default_stylesheet;
    pub use crate::css::parser::{MediaViewport, Stylesheet, parse_inline_style};
    pub use crate::css::values::{ComputedStyle, Display, TextDecoration};
    pub use crate::html::dom::{Document, NodeKind, TagName};
    pub use crate::html::tokenizer::Tokenizer;
    pub use crate::html::tree_builder::TreeBuilder;
    pub use crate::layout::block::{
        StyleCache, TextMeasurer, build_layout_tree, layout_block_incremental,
    };
    pub use crate::layout::box_model::{BoxType, LayoutBox};
    pub use crate::layout::text_cache::CachingMeasurer;
    pub use crate::paint::PaintViewport;
    pub use crate::paint::paint as paint_page;
}

// -----------------------------------------------------------------------
// Imports
// -----------------------------------------------------------------------

use std::collections::{HashMap, HashSet};

use html::dom::NodeId;
use loader::ResourceRequest;
use loader::cache::ResourceCache;
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

    /// Buffered JavaScript console output from the most recent page load.
    #[cfg(feature = "javascript")]
    console_output: Vec<oasis_js::ConsoleEntry>,

    /// Retained JS engine for event dispatch after page load.
    #[cfg(feature = "javascript")]
    js_engine: Option<oasis_js::JsEngine>,

    /// Shared document reference held by the JS engine's closures.
    /// Kept alive so the engine can access the DOM for event handlers.
    #[cfg(feature = "javascript")]
    js_doc: Option<js_dom::SharedDoc>,
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
            #[cfg(feature = "javascript")]
            console_output: Vec::new(),
            #[cfg(feature = "javascript")]
            js_engine: None,
            #[cfg(feature = "javascript")]
            js_doc: None,
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
        let doc = self
            .document
            .as_ref()
            .expect("guarded by is_none() early return above");
        let content_h = self.config.content_height(self.window_h);
        let base_url = self.nav.current_url().map(String::from);
        let measurer = layout::text_cache::CachingMeasurer::new(&SimpleTextMeasurer);
        let layout_root = layout::block::build_layout_tree(
            doc,
            &self.styles,
            &measurer,
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

    // Navigation/loading methods → widget_pipeline.rs
    // Image loading methods → widget_images.rs
    // Paint methods → widget_paint.rs
    // Input handling methods → widget_input.rs

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

    /// Get the window width (set by the WM).
    pub fn window_w(&self) -> u32 {
        self.window_w
    }

    /// Get the window height (set by the WM).
    pub fn window_h(&self) -> u32 {
        self.window_h
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
// Tests -- extracted to browser_tests.rs to keep lib.rs focused on the
// BrowserWidget struct definition, accessors, and module wiring.
// -----------------------------------------------------------------------

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
