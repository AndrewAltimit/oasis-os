//! Browser subsystem: HTML/CSS parsing, layout, and rendering.
//!
//! This module ties together the HTML/CSS pipeline (tokenizer, parser,
//! DOM, style cascade, layout, paint) with navigation, scroll state,
//! resource loading, reader mode, and Gemini protocol support into the
//! [`BrowserWidget`] -- the top-level component that the window manager
//! drives.

pub mod commands;
pub mod config;
pub mod css;
pub mod gemini;
pub mod html;
pub mod image;
pub mod layout;
pub mod loader;
pub mod nav;
pub mod paint;
pub mod plugin;
pub mod reader;
pub mod scroll;
pub mod skin;

// -----------------------------------------------------------------------
// Public re-exports
// -----------------------------------------------------------------------

pub use config::BrowserConfig;
pub use loader::{ContentType, ResourceResponse, ResourceSource, Url};
pub use nav::{Bookmark, HistoryEntry, NavigationController};
pub use scroll::ScrollState;

// -----------------------------------------------------------------------
// Imports
// -----------------------------------------------------------------------

use std::collections::HashMap;

use crate::backend::{Color, SdiBackend};
use crate::error::Result;
use crate::input::{Button, InputEvent, Trigger};
use crate::vfs::Vfs;

use html::dom::NodeId;
use loader::cache::{CacheEntry, ResourceCache};
use loader::{ResourceRequest, load_resource};
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
    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        text.len() as u32 * 8
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
        }
    }

    /// Update the window position and size (called by the WM).
    pub fn set_window(&mut self, x: i32, y: i32, w: u32, h: u32) {
        self.window_x = x;
        self.window_y = y;
        self.window_w = w;
        self.window_h = h;
        let vh = self.config.content_height(h) as i32;
        self.scroll.set_viewport_height(vh);
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

        match load_resource(vfs, &request) {
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

        // 3. CSS cascade with default stylesheet.
        let ua_sheet = css::default::default_stylesheet();
        let styles = css::cascade::style_tree(&doc, &[&ua_sheet], &[]);

        // 4. Build link href map from DOM.
        let href_map = Self::build_link_map(&doc);

        // 5. Build layout tree.
        let content_h = self.config.content_height(self.window_h);
        let layout_root = layout::block::build_layout_tree(
            &doc,
            &styles,
            &SimpleTextMeasurer,
            self.window_w as f32,
            content_h as f32,
        );

        // 6. Store results.
        self.document = Some(doc);
        self.styles = styles;
        self.href_map = href_map;
        self.layout_root = Some(layout_root);
        self.link_map.clear();
        self.scroll.reset();
        self.state = LoadingState::Idle;

        // 7. Update navigation.
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
                let styles = css::cascade::style_tree(&reader_doc, &[&ua_sheet], &[]);
                let href_map = Self::build_link_map(&reader_doc);
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
    pub fn paint(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
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

        // Paint status bar.
        self.paint_status_bar(backend)?;

        backend.reset_clip_rect()?;
        Ok(())
    }

    /// Paint the URL bar and navigation buttons.
    pub fn paint_chrome(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let h = self.config.url_bar_height;
        let bw = self.config.button_width;

        // Chrome background.
        backend.fill_rect(
            self.window_x,
            self.window_y,
            self.window_w,
            h,
            self.config.chrome_bg,
        )?;

        // Back button.
        let back_color = if self.nav.can_go_back() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        backend.fill_rect(self.window_x, self.window_y, bw, h, back_color)?;
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
        backend.fill_rect(self.window_x + bw as i32, self.window_y, bw, h, fwd_color)?;
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
        backend.fill_rect(url_x, self.window_y + 2, url_w, h - 4, bar_bg)?;

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
                    h - 6,
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
        backend.fill_rect(home_x, self.window_y, bw, h, self.config.chrome_button_bg)?;
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
        let text_w = scroll_text.len() as i32 * 8;
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
    use crate::vfs::MemoryVfs;

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
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "hello", 12,),
            40
        );
        assert_eq!(layout::block::TextMeasurer::measure_text(&m, "", 12,), 0);
        assert_eq!(layout::block::TextMeasurer::measure_text(&m, "a", 16,), 8);
        // Width is character count * 8, independent of font size.
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "test", 8,),
            32
        );
        assert_eq!(
            layout::block::TextMeasurer::measure_text(&m, "test", 24,),
            32
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
}
