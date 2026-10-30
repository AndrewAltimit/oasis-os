//! Window types and configuration for the window manager.
//!
//! The WM is a consumer of the SDI API. Each `Window` owns a group of SDI
//! objects identified by a naming convention: `"{id}.frame"`, `"{id}.titlebar"`,
//! etc. The WM handles behavior; the skin handles appearance.

mod id;
#[cfg(test)]
mod tests;
mod theme;

use oasis_types::bitmap_font::glyph_advance_scaled;

use crate::snap::SnapZone;

pub use id::WindowId;
pub use theme::WmTheme;

/// The behavioral template of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    /// Draggable, resizable, closable, minimizable, maximizable.
    AppWindow,
    /// Modal, centered, blocks input to other windows.
    Dialog,
    /// Docked to a screen edge, not freely draggable.
    Panel,
    /// Small, always-on-top, draggable, no minimize/maximize.
    FloatingWidget,
    /// No frame, no titlebar, covers entire content area.
    Fullscreen,
}

/// Current display state of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

/// Configuration for creating a new window.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Unique identifier (becomes the SDI object name prefix).
    pub id: String,
    /// Window title displayed in the titlebar.
    pub title: String,
    /// Initial X position. If `None`, the WM cascades automatically.
    pub x: Option<i32>,
    /// Initial Y position. If `None`, the WM cascades automatically.
    pub y: Option<i32>,
    /// Content area width.
    pub width: u32,
    /// Content area height.
    pub height: u32,
    /// Window type (determines available operations).
    pub window_type: WindowType,
    /// Pin this window above normal windows in z-order.
    pub always_on_top: bool,
    /// Block input to all windows below this one.
    pub modal: bool,
}

/// A window's outer rectangle: position plus size (used for saved
/// restore geometry, snap/tile targets and animation endpoints).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Result of [`Window::title_layout`]: where and what to draw in the titlebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleLayout {
    /// X position where the text is drawn.
    pub x: i32,
    /// Width of the region the title may occupy (buttons excluded).
    pub avail_w: u32,
    /// Measured pixel width of `text`.
    pub text_w: u32,
    /// The title, truncated with `"..."` when it does not fit.
    pub text: String,
}

/// Measure the pixel width of `s` with the proportional bitmap-font metrics.
pub fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

/// Truncate `title` so its rendered width fits in `max_px`, appending
/// `"..."` when truncated. Returns an empty string when not even the
/// ellipsis fits.
pub fn truncate_to_width(title: &str, font_size: u16, max_px: i32) -> String {
    if text_px(title, font_size) <= max_px {
        return title.to_string();
    }
    let ellipsis_w = text_px("...", font_size);
    let target = max_px - ellipsis_w;
    if target < 0 {
        return String::new();
    }
    let mut w = 0i32;
    let mut end = 0;
    for (i, c) in title.char_indices() {
        let cw = glyph_advance_scaled(c, font_size) as i32;
        if w + cw > target {
            break;
        }
        w += cw;
        end = i + c.len_utf8();
    }
    format!("{}...", title[..end].trim_end())
}

/// A managed window in the WM.
///
/// Tracks the window's metadata, geometry, state, and the names of its
/// SDI child objects. The WM manipulates the SDI registry through these names.
#[derive(Debug, Clone)]
pub struct Window {
    /// Unique identifier (SDI name prefix).
    pub id: WindowId,
    /// Display title.
    pub title: String,
    /// Window type.
    pub window_type: WindowType,
    /// Current state.
    pub state: WindowState,
    /// Position of the window's outer frame (top-left).
    pub x: i32,
    /// Position of the window's outer frame (top-left).
    pub y: i32,
    /// Total outer width (including borders).
    pub outer_w: u32,
    /// Total outer height (including borders and titlebar).
    pub outer_h: u32,
    /// Saved geometry for restoring from maximized state.
    pub saved_geometry: Option<Geometry>,
    /// Pin this window above normal windows in z-order.
    pub always_on_top: bool,
    /// Block input to all windows below this one.
    pub modal: bool,
    /// Whether this window is in fullscreen kiosk mode (no decorations, full screen).
    pub fullscreen_kiosk: bool,
    /// Geometry saved when entering kiosk mode (separate from maximize/minimize
    /// `saved_geometry` so that kiosk ↔ maximize don't clobber each other).
    pub kiosk_saved_geometry: Option<Geometry>,
    /// Snap zone the window is currently snapped to (drag-to-edge or
    /// keyboard snap), if any. Cleared when the window is unsnapped,
    /// dragged out of the zone, or resized by hand.
    pub snap_zone: Option<SnapZone>,
    /// Geometry to return to when the window is unsnapped.
    pub pre_snap_geometry: Option<Geometry>,
    /// Geometry to return to when tiling is switched off.
    pub pre_tile_geometry: Option<Geometry>,
    /// The window was maximized when it was minimized: restoring it (e.g.
    /// from the taskbar) brings it back maximized, while `saved_geometry`
    /// keeps the pre-maximize geometry for the restore after that.
    pub minimized_from_maximized: bool,
}

impl Window {
    /// Create a new window from configuration and theme.
    pub fn new(config: &WindowConfig, x: i32, y: i32, theme: &WmTheme) -> Self {
        let has_titlebar = config.window_type != WindowType::Fullscreen;
        let has_border = matches!(
            config.window_type,
            WindowType::AppWindow | WindowType::Dialog
        );

        let border = if has_border { theme.border_width } else { 0 };
        let titlebar_h = if has_titlebar {
            theme.titlebar_height
        } else {
            0
        };

        let outer_w = config.width + border * 2;
        let outer_h = config.height + titlebar_h + border * 2;

        Self {
            id: WindowId::from(config.id.as_str()),
            title: config.title.clone(),
            window_type: config.window_type,
            state: WindowState::Normal,
            x,
            y,
            outer_w,
            outer_h,
            saved_geometry: None,
            always_on_top: config.always_on_top,
            modal: config.modal,
            fullscreen_kiosk: false,
            kiosk_saved_geometry: None,
            snap_zone: None,
            pre_snap_geometry: None,
            pre_tile_geometry: None,
            minimized_from_maximized: false,
        }
    }

    /// The window's current outer rectangle.
    pub fn geometry(&self) -> Geometry {
        Geometry {
            x: self.x,
            y: self.y,
            w: self.outer_w,
            h: self.outer_h,
        }
    }

    /// Move and resize the outer frame to `g` (logical geometry only; the
    /// manager re-lays out the SDI objects).
    pub fn set_geometry(&mut self, g: Geometry) {
        self.x = g.x;
        self.y = g.y;
        self.outer_w = g.w;
        self.outer_h = g.h;
    }

    /// Compute the content area rectangle (position and size within the frame).
    pub fn content_rect(&self, theme: &WmTheme) -> (i32, i32, u32, u32) {
        self.content_rect_at(theme, self.geometry())
    }

    /// Content area rectangle if the window's outer frame were at `outer`
    /// (used to draw windows at their animated geometry).
    pub fn content_rect_at(&self, theme: &WmTheme, outer: Geometry) -> (i32, i32, u32, u32) {
        if self.fullscreen_kiosk {
            return (outer.x, outer.y, outer.w, outer.h);
        }

        let has_titlebar = self.window_type != WindowType::Fullscreen;
        let has_border = matches!(self.window_type, WindowType::AppWindow | WindowType::Dialog);

        let border = if has_border { theme.border_width } else { 0 };
        let titlebar_h = if has_titlebar {
            theme.titlebar_height
        } else {
            0
        };

        let cx = outer.x + border as i32;
        let cy = outer.y + titlebar_h as i32 + border as i32;
        let cw = outer.w.saturating_sub(border * 2);
        let ch = outer
            .h
            .saturating_sub(titlebar_h)
            .saturating_sub(border * 2);
        (cx, cy, cw, ch)
    }

    /// Compute the titlebar rectangle.
    pub fn titlebar_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        if self.fullscreen_kiosk || self.window_type == WindowType::Fullscreen {
            return None;
        }
        let has_border = matches!(self.window_type, WindowType::AppWindow | WindowType::Dialog);
        let border = if has_border { theme.border_width } else { 0 };

        let tx = self.x + border as i32;
        let ty = self.y + border as i32;
        let tw = self.outer_w.saturating_sub(border * 2);
        let th = theme.titlebar_height;
        Some((tx, ty, tw, th))
    }

    /// Compute button inset from the titlebar edge, derived from
    /// the titlebar height so buttons stay proportional.
    fn button_inset(theme: &WmTheme) -> i32 {
        (theme.titlebar_height as i32 / 8).max(1)
    }

    /// Compute a button's X position given its index.
    ///
    /// `idx` counts from the edge indicated by `button_side` inward. With
    /// `button_side == "right"` the physical left-to-right order is minimize,
    /// maximize, close (Windows / GNOME convention: close at the corner).
    /// With `button_side == "left"` it is close, minimize, maximize (macOS
    /// traffic-light convention: close at the corner).
    fn button_x(&self, theme: &WmTheme, tx: i32, tw: u32, idx: i32) -> i32 {
        let btn_size = theme.button_size.min(theme.titlebar_height) as i32;
        let sp = theme.button_spacing;
        let inset = Self::button_inset(theme);
        if theme.button_side == "left" {
            tx + inset + idx * (btn_size + sp)
        } else {
            tx + tw as i32 - (idx + 1) * btn_size - idx * sp - inset
        }
    }

    /// Index of the minimize button (counted from the button edge).
    ///
    /// Left side: second from the edge (after close). Right side: furthest
    /// from the edge (leftmost of the group).
    fn minimize_btn_idx(&self, theme: &WmTheme) -> i32 {
        if theme.button_side == "left" { 1 } else { 2 }
    }

    /// Index of the maximize button (counted from the button edge).
    ///
    /// Left side: furthest from the edge. Right side: middle.
    fn maximize_btn_idx(&self, theme: &WmTheme) -> i32 {
        if theme.button_side == "left" { 2 } else { 1 }
    }

    /// Index of the close button: always at the titlebar edge, on either
    /// side (and when it is the only button, e.g. Dialog/FloatingWidget).
    fn close_btn_idx(&self) -> i32 {
        0
    }

    /// Compute close button rectangle.
    pub fn close_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        let (tx, ty, tw, th) = self.titlebar_rect(theme)?;
        if !self.has_close_button() {
            return None;
        }
        let btn_size = theme.button_size.min(th);
        let bx = self.button_x(theme, tx, tw, self.close_btn_idx());
        let by = ty + (th as i32 - btn_size as i32) / 2;
        Some((bx, by, btn_size, btn_size))
    }

    /// Compute minimize button rectangle.
    pub fn minimize_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        let (tx, ty, tw, th) = self.titlebar_rect(theme)?;
        if !self.has_minimize_button() {
            return None;
        }
        let btn_size = theme.button_size.min(th);
        let bx = self.button_x(theme, tx, tw, self.minimize_btn_idx(theme));
        let by = ty + (th as i32 - btn_size as i32) / 2;
        Some((bx, by, btn_size, btn_size))
    }

    /// Compute maximize button rectangle.
    pub fn maximize_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        let (tx, ty, tw, th) = self.titlebar_rect(theme)?;
        if !self.has_maximize_button() {
            return None;
        }
        let btn_size = theme.button_size.min(th);
        let bx = self.button_x(theme, tx, tw, self.maximize_btn_idx(theme));
        let by = ty + (th as i32 - btn_size as i32) / 2;
        Some((bx, by, btn_size, btn_size))
    }

    /// Number of titlebar buttons this window shows.
    fn button_count(&self) -> i32 {
        [
            self.has_close_button(),
            self.has_minimize_button(),
            self.has_maximize_button(),
        ]
        .iter()
        .filter(|&&v| v)
        .count() as i32
    }

    /// Lay out the title text: where it starts, the space available to it,
    /// and the (possibly ellipsized) string that fits in that space.
    ///
    /// The button group plus a gap is reserved on the button side. For
    /// `title_align == "center"` the same reservation is mirrored on the
    /// opposite side, so the title is centered on the whole titlebar (using
    /// the measured glyph width) yet can never run under a button. Titles
    /// wider than the available space are truncated with `"..."`.
    pub fn title_layout(&self, theme: &WmTheme) -> Option<TitleLayout> {
        let (tx, _ty, tw, _th) = self.titlebar_rect(theme)?;
        let btn_size = theme.button_size.min(theme.titlebar_height) as i32;
        let sp = theme.button_spacing;
        let inset = Self::button_inset(theme);
        let text_inset = inset * 2;
        let btn_count = self.button_count();
        // Pixels reserved on the button side: edge inset + buttons + gap.
        let buttons_w = if btn_count > 0 {
            inset + btn_count * btn_size + (btn_count - 1) * sp + text_inset
        } else {
            text_inset
        };
        let tw_i = tw as i32;
        let font = theme.titlebar_font_size;

        let (region_x, region_w) = if theme.title_align == "center" {
            (tx + buttons_w, tw_i - buttons_w * 2)
        } else if theme.button_side == "left" {
            (tx + buttons_w, tw_i - buttons_w - text_inset)
        } else {
            (tx + text_inset, tw_i - buttons_w - text_inset)
        };
        let region_w = region_w.max(0);

        let text = truncate_to_width(&self.title, font, region_w);
        let text_w = text_px(&text, font).min(region_w);
        let x = if theme.title_align == "center" {
            // Center on the full titlebar; the symmetric reservation keeps
            // the result inside [region_x, region_x + region_w].
            (tx + (tw_i - text_w) / 2).clamp(region_x, region_x + region_w - text_w)
        } else {
            region_x
        };
        Some(TitleLayout {
            x,
            avail_w: region_w as u32,
            text_w: text_w as u32,
            text,
        })
    }

    /// Whether this window type has a close button.
    pub fn has_close_button(&self) -> bool {
        matches!(
            self.window_type,
            WindowType::AppWindow | WindowType::Dialog | WindowType::FloatingWidget
        )
    }

    /// Whether this window type has a minimize button.
    pub fn has_minimize_button(&self) -> bool {
        self.window_type == WindowType::AppWindow
    }

    /// Whether this window type has a maximize button.
    pub fn has_maximize_button(&self) -> bool {
        self.window_type == WindowType::AppWindow
    }

    /// Whether this window type is resizable.
    pub fn is_resizable(&self) -> bool {
        !self.fullscreen_kiosk && self.window_type == WindowType::AppWindow
    }

    /// Whether this window type is draggable.
    pub fn is_draggable(&self) -> bool {
        matches!(
            self.window_type,
            WindowType::AppWindow | WindowType::FloatingWidget
        )
    }

    /// The list of SDI object suffixes this window creates.
    ///
    /// Returns a borrowed static slice: the render path calls this once per
    /// window per frame, so it must not allocate.
    pub fn sdi_suffixes(&self) -> &'static [&'static str] {
        const FULLSCREEN: &[&str] = &["content"];
        const FLOATING_WIDGET: &[&str] = &[
            "frame",
            "titlebar",
            "title_text",
            "title_shadow",
            "separator",
            "btn_close",
            "btn_close_glyph",
            "content",
            "content_stroke",
        ];
        const PANEL: &[&str] = &[
            "frame",
            "titlebar",
            "title_text",
            "title_shadow",
            "separator",
            "content",
            "content_stroke",
        ];
        const DIALOG: &[&str] = &[
            "frame",
            "titlebar",
            "title_text",
            "title_shadow",
            "separator",
            "btn_close",
            "btn_close_glyph",
            "content",
            "content_stroke",
        ];
        const APP_WINDOW: &[&str] = &[
            "frame",
            "titlebar",
            "title_text",
            "title_shadow",
            "separator",
            "btn_close",
            "btn_close_glyph",
            "btn_minimize",
            "btn_minimize_glyph",
            "btn_maximize",
            "btn_maximize_glyph",
            "content",
            "content_stroke",
        ];
        match self.window_type {
            WindowType::Fullscreen => FULLSCREEN,
            WindowType::FloatingWidget => FLOATING_WIDGET,
            WindowType::Panel => PANEL,
            WindowType::Dialog => DIALOG,
            WindowType::AppWindow => APP_WINDOW,
        }
    }

    /// Build the full SDI object name for a suffix.
    pub fn sdi_name(&self, suffix: &str) -> String {
        format!("{}.{suffix}", self.id)
    }
}
