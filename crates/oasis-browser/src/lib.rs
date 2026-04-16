//! Browser subsystem: HTML/CSS parsing, layout, and rendering.
//!
//! This module ties together the HTML/CSS pipeline (tokenizer, parser,
//! DOM, style cascade, layout, paint) with navigation, scroll state,
//! resource loading, reader mode, and Gemini protocol support into the
//! [`BrowserWidget`] -- the top-level component that the window manager
//! drives.
//!
//! See `docs/` (in the crate root) for architecture, CSS coverage, and
//! contributor guides. Start with `docs/README.md`.
//!
//! # Feature flags
//!
//! - `javascript` — enables QuickJS-NG via `oasis-js` for inline
//!   `<script>` execution, `fetch`, `setTimeout`, event dispatch, and
//!   DOM bindings. The PSP backend now compiles the same QuickJS C
//!   sources via pspdev's `psp-gcc` (wiring in
//!   `crates/oasis-backend-psp/.cargo/config.toml`), so `oasis-js` and
//!   its full `JsEngine` API are available on `mipsel-sony-psp` too.
//!   The `javascript` feature is still off on PSP by default while
//!   `js_dom.rs` is audited for the mipsel target; once that's done
//!   the PSP browser will pick up inline `<script>` and DOM bindings
//!   unchanged from desktop/WASM/UE5. Standalone evaluation is already
//!   reachable via `cmd_server.rs`'s `js <code>` TCP command.
//! - `webp` — enables WebP image decoding via the `image` crate.
//! - `parallel-style` — parallel style cascade via rayon (desktop only).
//! - `psp` — enables PSP-specific shrink-the-footprint code paths and
//!   disables features that rely on desktop-only crates.
//!
//! # Compositor (overhaul epic)
//!
//! Desktop backends (SDL3, WASM, UE5) support offscreen compositing
//! via `SdiRenderTarget`. This unlocks, on the backends that opt in:
//!
//! - `mix-blend-mode` — WASM renders all 16 CSS blend modes natively
//!   via Canvas2D `globalCompositeOperation`. SDL3 currently uses
//!   `Blend`/`Mod` approximations for Normal/Multiply and degrades
//!   the other 14 modes to plain alpha over; the software path lands
//!   in a follow-up.
//! - `backdrop-filter` — parsed and plumbed through `PushCompositingLayer`;
//!   the backdrop-sample step is scaffolded on top of the filter-chain
//!   readback pipeline.
//! - `isolation: isolate` — creates a stacking context and a real
//!   compositing layer.
//! - box-level `filter` — opacity, grayscale, invert, brightness,
//!   contrast, sepia, saturate, hue-rotate, and blur (separable
//!   3-pass box blur) run on CPU via read_render_target → apply →
//!   upload as texture → blit.
//! - `mask-*` (8 longhands) — parsed and stored on `ComputedStyle`;
//!   the destination-in composite path lives on top of the existing
//!   readback pipeline in a follow-up.
//!
//! PSP opts out: the backend reports `supports_render_targets() = false`
//! so replay falls back to plain opacity stacking — pages still render,
//! but blend modes, filters, and masks do not apply. See
//! `docs/compositor-overhaul-plan.md` §3.6 for the phased revisit plan.

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
pub mod search;
pub mod skin;
pub mod svg;
pub(crate) mod transform;

pub mod canvas;
pub mod font;

pub(crate) mod image_atlas;
mod widget_images;
mod widget_input;
mod widget_paint;
mod widget_pipeline;

#[cfg(feature = "javascript")]
mod js_dom;

#[cfg(test)]
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
    pub use crate::css::cascade::{
        CascadeContext, set_cascade_progress_hook, set_cascade_yield_hook, style_tree,
    };
    pub use crate::css::default::default_stylesheet;
    pub use crate::css::parser::{MediaViewport, Stylesheet, parse_inline_style};
    pub use crate::css::values::{ComputedStyle, Display, TextDecoration, TextDecorationLine};
    pub use crate::html::dom::{Document, NodeKind, TagName};
    pub use crate::html::tokenizer::{
        Tokenizer, set_tokenize_progress_hook, set_tokenize_yield_hook,
    };
    pub use crate::html::tree_builder::{
        TreeBuilder, set_tree_builder_progress_hook, set_tree_builder_raw_log_hook,
        set_tree_builder_yield_hook,
    };
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
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
use loader::cookies::CookieJar;
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
// BrowserError
// -----------------------------------------------------------------------

/// An error recorded during page loading, parsing, or rendering.
#[derive(Debug, Clone)]
pub struct BrowserError {
    /// The category of error.
    pub kind: BrowserErrorKind,
    /// Human-readable description.
    pub message: String,
}

/// Category of a [`BrowserError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserErrorKind {
    /// Network or resource loading error.
    Network,
    /// HTML/CSS parse error.
    Parse,
    /// JavaScript execution error.
    Script,
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

    /// Session-scoped cookie jar for HTTP requests.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    cookie_jar: CookieJar,

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

    /// Whether the layout tree needs rebuilding.
    layout_dirty: bool,

    /// When true, the next `load_html` call skips pushing to the
    /// navigation history. Used by back/forward cache restore.
    skip_nav_push: bool,

    /// Viewport width used for the most recent layout pass.
    last_layout_w: u32,

    /// Set of URLs that have been navigated to (for `:visited`).
    visited_urls: HashSet<String>,

    /// DOM node currently under the cursor (for `:hover`).
    hover_node: Option<NodeId>,

    /// DOM node that currently has keyboard/tab focus (for `:focus`).
    focused_node: Option<NodeId>,

    /// DOM node ID of the `<body>` element (fallback target for key events).
    body_node_id: Option<NodeId>,

    /// Decoded image data keyed by resolved src URL.
    decoded_images: HashMap<String, image::DecodedImage>,

    /// Arc-wrapped views of `decoded_images` entries used as
    /// `mask-image: url(...)` sources. Lazily populated by
    /// `ensure_image_textures` so each unique mask URL clones its
    /// pixel buffer exactly once, then layout boxes cheaply carry a
    /// shared reference to the decoded bytes without round-tripping
    /// through the GPU.
    mask_image_arcs: HashMap<String, std::sync::Arc<image::DecodedImage>>,

    /// Background I/O thread for non-blocking HTTP requests.
    /// Lazily created on first network request.
    ///
    /// **Drop order**: `io_thread` is declared before `tls` so it is
    /// dropped first. `IoThread::drop()` closes the sender channel and
    /// joins the worker thread, ensuring it has fully exited before the
    /// `TlsProvider` is freed.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    io_thread: Option<loader::io_thread::IoThread>,

    /// Optional TLS provider for HTTPS and Gemini connections.
    ///
    /// **Drop order**: Must be declared after `io_thread` so it outlives
    /// the I/O worker thread (see `SharedTlsProvider` safety invariant).
    tls: Option<Box<dyn oasis_net::tls::TlsProvider>>,

    /// Pending page load request ID (in-flight on the I/O thread).
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    pending_page_load: Option<loader::io_thread::IoRequestId>,

    /// In-flight image requests on the I/O thread, keyed by request ID
    /// mapped to the resolved image URL.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    pending_io_images: std::collections::HashMap<loader::io_thread::IoRequestId, String>,

    /// Channel to send `(url, raw_bytes)` to the background decode thread.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    image_decode_tx: Option<std::sync::mpsc::Sender<(String, Vec<u8>)>>,

    /// Channel to receive `(url, DecodedImage)` from the background decode thread.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    image_decode_rx: Option<std::sync::mpsc::Receiver<(String, image::DecodedImage)>>,

    /// Number of images sent to the decode thread but not yet received back.
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    image_decode_in_flight: usize,

    /// GPU textures for decoded images, keyed by src URL.
    image_textures: HashMap<String, TextureId>,

    /// Texture atlas for packing small images (reduces GPU bind switches).
    image_atlas: image_atlas::ImageAtlas,

    /// Pending image requests to be processed in time-sliced batches.
    pending_images: Vec<(String, ResourceRequest)>,

    /// Images deferred because they are far below the viewport.
    /// Re-evaluated on scroll events, not every tick.
    deferred_images: Vec<(String, ResourceRequest)>,

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

    /// Cached selector index built from UA + author sheets. Reused
    /// across hover/focus restyles to avoid rebuilding per mouse move.
    /// Invalidated on navigation when stylesheets change.
    cached_selector_index: Option<css::cascade::SelectorIndex>,

    /// Snapshot of post-layout container sizes used to evaluate
    /// `@container` rules during cascade. Built from the layout tree
    /// after the first layout pass and reused by hover/focus restyles
    /// until the next full reload.
    container_lookup: Option<css::cascade::ContainerLookup>,

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

    /// Scripts with the `defer` attribute, executed after the first paint
    /// rather than blocking the initial render.
    #[cfg(feature = "javascript")]
    deferred_scripts: Vec<String>,

    /// Shared computed styles for `getComputedStyle()` in JS.
    /// Updated after CSS cascade so event handlers see current values.
    #[cfg(feature = "javascript")]
    js_styles: js_dom::SharedStyles,

    /// Pending navigation actions from JavaScript (`location.assign`,
    /// `history.back`/`forward`).
    #[cfg(feature = "javascript")]
    js_nav_actions: js_dom::SharedNavActions,

    /// Persistent localStorage data shared across page navigations.
    #[cfg(feature = "javascript")]
    js_local_storage: js_dom::SharedLocalStorage,

    /// Shared canvas states keyed by DOM `NodeId`. Populated during
    /// layout for `<canvas>` elements, accessed by JS canvas bindings.
    #[cfg(feature = "javascript")]
    canvas_states: canvas::SharedCanvasMap,

    /// CSS transition engine for smooth property interpolation.
    transition_engine: css::transition::TransitionEngine,

    /// CSS animation engine for `@keyframes` animations.
    animation_engine: css::animation::AnimationEngine,

    /// Timestamp of the last `tick()` call for computing animation deltas.
    last_tick_time: Option<std::time::Instant>,

    /// Content Security Policy for the current page (parsed from HTTP
    /// response headers).
    page_csp: Option<loader::csp::CspPolicy>,

    /// Timestamp when loading started (for progress bar animation).
    #[allow(dead_code)]
    loading_start: Option<std::time::Instant>,

    /// In-page search state: the current search query (empty = inactive).
    #[allow(dead_code)]
    search_state: String,

    /// Ordered list of focusable DOM node IDs (tabindex order).
    #[allow(dead_code)]
    tab_order: Vec<NodeId>,

    /// Current index into `tab_order` (-1 = none).
    #[allow(dead_code)]
    tab_focus_index: i32,

    /// Persistent text measurement cache shared across layout passes.
    /// Cleared only when the effective font size changes (e.g. zoom).
    text_cache: layout::text_cache::SharedTextCache,

    /// Web font registry: loaded @font-face fonts for the current page.
    #[cfg(feature = "web-fonts")]
    font_registry: std::cell::RefCell<font::FontRegistry>,

    /// Cached glyph textures for web font rendering.
    /// Used by `render_web_font_text` during display list replay.
    #[cfg(feature = "web-fonts")]
    #[allow(dead_code)]
    glyph_tex_cache: font::GlyphTextureCache,

    /// Whether `load_web_fonts` has already been attempted for the
    /// current page. Prevents re-issuing network fetches every tick
    /// when font loading fails (404, parse error).
    #[cfg(feature = "web-fonts")]
    fonts_load_attempted: bool,

    /// Last font size used for layout (base * zoom). When this changes
    /// the text cache is invalidated.
    last_effective_font_size: f32,

    /// Accumulated errors from page loading, parsing, and JS execution
    /// for the current page. Cleared on each new navigation.
    page_errors: Vec<BrowserError>,

    /// Form state manager for HTML `<form>` elements on the current page.
    form_manager: forms::FormManager,

    /// Cached display list for the current page content.
    /// Rebuilt only when layout changes; replayed on each frame.
    display_list: paint::display_list::DisplayList,

    /// Scroll Y position at which the display list was last recorded.
    /// When scroll changes, we replay with adjusted offsets instead of
    /// rebuilding. A full rebuild is forced when layout changes.
    display_list_scroll_y: i32,

    /// Scroll X position at which the display list was last recorded.
    display_list_scroll_x: i32,

    /// Scroll Y position that link_map regions are currently adjusted to.
    /// Used to compute per-frame deltas when shifting link regions during
    /// scroll-only replay (without re-recording the display list).
    link_map_scroll_y: i32,

    /// Scroll X position that link_map regions are currently adjusted to.
    link_map_scroll_x: i32,

    /// Per-element scroll offsets for nested scroll containers.
    /// Elements with `overflow: auto/scroll` whose content exceeds their
    /// box dimensions get an entry here, keyed by DOM node ID.
    nested_scroll_offsets: HashMap<NodeId, (f32, f32)>,

    /// Dirty rectangles that need repainting (e.g. hover/focus changes).
    /// When non-empty and no layout change occurred, only these regions
    /// are replayed via `replay_dirty()` instead of a full `replay()`.
    dirty_rects: Vec<layout::box_model::Rect>,

    /// When true, the entire viewport needs repainting even though
    /// the layout may not have changed (e.g. visual-only style change
    /// without known dirty rects).
    full_repaint_needed: bool,

    /// Tile grid for tracking visible/dirty regions of the page.
    /// Infrastructure for future GPU tile caching (render tiles as
    /// GPU textures, only re-render newly visible tiles on scroll).
    #[allow(dead_code)]
    tile_grid: Option<paint::tiling::TileGrid>,

    /// Optional diagnostic log hook. When set, the browser fires it
    /// at key milestones during navigation and image loading: page
    /// fetch start/end, response processing, image fetch start/end
    /// with sizes, etc. PSP wires this to its on-disk `eboot.log` so
    /// remote diagnostics can capture where a synchronous
    /// `navigate_vfs` is spending its time (or hanging).
    pub(crate) diag_log: Option<DiagLogFn>,
}

/// Boxed callback type for [`BrowserWidget::set_diag_log`].
pub type DiagLogFn = Box<dyn Fn(&str) + Send + Sync>;

impl BrowserWidget {
    /// Create a new browser widget with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        let home = config.features.home_url.clone();
        let cache_bytes = config.cache_size_bytes();
        let smooth = config.smooth_scroll;
        let effective_font = config.default_font_size * config.zoom_level;
        Self {
            config,
            nav: NavigationController::new(&home),
            scroll: ScrollState::new(238, smooth), // 272 - 34
            cache: ResourceCache::new(cache_bytes),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            cookie_jar: CookieJar::new(),
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
            layout_dirty: false,
            skip_nav_push: false,
            last_layout_w: 480,
            visited_urls: HashSet::new(),
            hover_node: None,
            focused_node: None,
            body_node_id: None,
            decoded_images: HashMap::new(),
            mask_image_arcs: HashMap::new(),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            io_thread: None,
            tls: None,
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            pending_page_load: None,
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            pending_io_images: HashMap::new(),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            image_decode_tx: None,
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            image_decode_rx: None,
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            image_decode_in_flight: 0,
            image_textures: HashMap::new(),
            image_atlas: image_atlas::ImageAtlas::new(),
            pending_images: Vec::new(),
            deferred_images: Vec::new(),
            decoded_image_bytes: 0,
            decoded_image_lru: std::collections::VecDeque::new(),
            cached_image_info: HashMap::new(),
            image_info_dirty: false,
            cached_author_sheets: Vec::new(),
            cached_inline_styles: Vec::new(),
            cached_selector_index: None,
            container_lookup: None,
            last_hover_time: None,
            #[cfg(feature = "javascript")]
            console_output: Vec::new(),
            #[cfg(feature = "javascript")]
            js_engine: None,
            #[cfg(feature = "javascript")]
            js_doc: None,
            #[cfg(feature = "javascript")]
            deferred_scripts: Vec::new(),
            #[cfg(feature = "javascript")]
            js_styles: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            #[cfg(feature = "javascript")]
            js_nav_actions: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            #[cfg(feature = "javascript")]
            js_local_storage: std::rc::Rc::new(std::cell::RefCell::new(HashMap::new())),
            #[cfg(feature = "javascript")]
            canvas_states: std::rc::Rc::new(std::cell::RefCell::new(HashMap::new())),
            transition_engine: css::transition::TransitionEngine::new(),
            animation_engine: css::animation::AnimationEngine::new(),
            last_tick_time: None,
            page_csp: None,
            loading_start: None,
            search_state: String::new(),
            tab_order: Vec::new(),
            tab_focus_index: -1,
            text_cache: layout::text_cache::new_shared_cache(),
            #[cfg(feature = "web-fonts")]
            font_registry: std::cell::RefCell::new(font::FontRegistry::new()),
            #[cfg(feature = "web-fonts")]
            glyph_tex_cache: font::GlyphTextureCache::new(),
            #[cfg(feature = "web-fonts")]
            fonts_load_attempted: false,
            last_effective_font_size: effective_font,
            page_errors: Vec::new(),
            form_manager: forms::FormManager::new(),
            display_list: paint::display_list::DisplayList::new(),
            display_list_scroll_y: 0,
            display_list_scroll_x: 0,
            link_map_scroll_y: 0,
            link_map_scroll_x: 0,
            nested_scroll_offsets: HashMap::new(),
            dirty_rects: Vec::new(),
            full_repaint_needed: true,
            tile_grid: None,
            diag_log: None,
        }
    }

    /// Attach a TLS provider for HTTPS and Gemini support.
    pub fn set_tls_provider(&mut self, provider: Box<dyn oasis_net::tls::TlsProvider>) {
        self.tls = Some(provider);
    }

    /// Install a diagnostic log hook. The browser fires it at key
    /// milestones during navigation and image loading so an embedder
    /// can capture where a synchronous `navigate_vfs` is spending its
    /// time. Used by the PSP backend to write to its on-disk
    /// `eboot.log` for remote diagnostics.
    pub fn set_diag_log(&mut self, hook: DiagLogFn) {
        self.diag_log = Some(hook);
    }

    /// Internal: fire the diagnostic log hook if installed.
    pub(crate) fn diag(&self, msg: &str) {
        if let Some(ref hook) = self.diag_log {
            hook(msg);
        }
    }

    /// Update the window position and size (called by the WM).
    ///
    /// Marks layout dirty if the viewport width changed, since layout
    /// depends on the available width for line breaking and block sizing.
    pub fn set_window(&mut self, x: i32, y: i32, w: u32, h: u32) {
        if w != self.window_w || h != self.window_h {
            self.layout_dirty = true;
        }
        // Position change requires display list rebuild (absolute coords).
        if x != self.window_x || y != self.window_y {
            self.full_repaint_needed = true;
        }
        self.window_x = x;
        self.window_y = y;
        self.window_w = w;
        self.window_h = h;
        let vh = self.config.content_height(h) as i32;
        self.scroll.set_viewport_height(vh);
        self.scroll.set_viewport_width(w as i32);
    }

    /// Returns whether the layout tree needs rebuilding.
    pub fn is_layout_dirty(&self) -> bool {
        self.layout_dirty
    }

    /// Mark a screen-space rectangle as needing repaint.
    ///
    /// On the next `paint()` call, only display items intersecting
    /// dirty rectangles will be replayed (via `replay_dirty()`),
    /// skipping items outside the dirty region.
    pub fn mark_dirty(&mut self, rect: layout::box_model::Rect) {
        self.dirty_rects.push(rect);
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

        self.refresh_image_info();
        let doc = self
            .document
            .as_ref()
            .expect("guarded by is_none() early return above");
        let content_h = self.config.content_height(self.window_h);
        let base_url = self.nav.current_url().map(String::from);
        let shared = std::rc::Rc::clone(&self.text_cache);
        let measurer =
            layout::text_cache::CachingMeasurer::with_shared(&SimpleTextMeasurer, shared);
        let viewport_w = self.window_w as f32 / self.config.zoom_level;
        let viewport_h = content_h as f32 / self.config.zoom_level;
        let layout_root = layout::block::build_layout_tree(
            doc,
            &self.styles,
            &measurer,
            viewport_w,
            viewport_h,
            base_url.as_deref(),
            &self.cached_image_info,
        );

        #[cfg(feature = "javascript")]
        {
            self.canvas_states.borrow_mut().clear();
            canvas::collect_canvas_states(&layout_root, &self.canvas_states);
        }

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

    /// Accumulated errors from the current page (network, parse, JS).
    ///
    /// Cleared on each new navigation. Useful for developer tooling
    /// or debugging pages that fail to load correctly.
    pub fn errors(&self) -> &[BrowserError] {
        &self.page_errors
    }

    /// Record a browser error for the current page.
    pub(crate) fn record_error(&mut self, kind: BrowserErrorKind, message: impl Into<String>) {
        self.page_errors.push(BrowserError {
            kind,
            message: message.into(),
        });
    }

    /// Get the scroll state.
    pub fn scroll(&self) -> &ScrollState {
        &self.scroll
    }

    /// Get a mutable reference to the scroll state.
    pub fn scroll_mut(&mut self) -> &mut ScrollState {
        &mut self.scroll
    }

    // ---------------------------------------------------------------
    // Zoom / text scaling
    // ---------------------------------------------------------------

    /// Current zoom level (1.0 = 100%).
    pub fn zoom_level(&self) -> f32 {
        self.config.zoom_level
    }

    /// Effective font size after applying zoom.
    pub fn effective_font_size(&self) -> f32 {
        self.config.default_font_size * self.config.zoom_level
    }

    /// Zoom in by 25% (max 3.0x).
    pub fn zoom_in(&mut self) {
        let new_zoom = (self.config.zoom_level * 1.25).min(3.0);
        self.set_zoom(new_zoom);
    }

    /// Zoom out by 20% (min 0.5x).
    pub fn zoom_out(&mut self) {
        let new_zoom = (self.config.zoom_level * 0.8).max(0.5);
        self.set_zoom(new_zoom);
    }

    /// Reset zoom to 1.0x.
    pub fn reset_zoom(&mut self) {
        self.set_zoom(1.0);
    }

    /// Set an explicit zoom level, clamped to 0.5..=3.0.
    ///
    /// Invalidates the text measurement cache and marks layout dirty.
    fn set_zoom(&mut self, level: f32) {
        let clamped = level.clamp(0.5, 3.0);
        if (clamped - self.config.zoom_level).abs() < f32::EPSILON {
            return;
        }
        self.config.zoom_level = clamped;
        let new_effective = self.effective_font_size();
        if (new_effective - self.last_effective_font_size).abs() > f32::EPSILON {
            // Font size changed -- clear persistent text cache.
            self.text_cache.borrow_mut().clear();
            self.last_effective_font_size = new_effective;
        }
        self.layout_dirty = true;
    }
}

// -----------------------------------------------------------------------
// Tests -- extracted to browser_tests.rs to keep lib.rs focused on the
// BrowserWidget struct definition, accessors, and module wiring.
// -----------------------------------------------------------------------

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
