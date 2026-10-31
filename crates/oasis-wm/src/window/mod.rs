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

use crate::hit_test::ButtonKind;
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

impl WindowType {
    /// Frame border width a window of this type gets from `theme` (only
    /// app windows and dialogs have a border).
    pub fn border(self, theme: &WmTheme) -> u32 {
        match self {
            WindowType::AppWindow | WindowType::Dialog => theme.border_width,
            _ => 0,
        }
    }

    /// Titlebar height a window of this type gets from `theme`
    /// (everything but [`WindowType::Fullscreen`] has a titlebar).
    pub fn titlebar_height(self, theme: &WmTheme) -> u32 {
        match self {
            WindowType::Fullscreen => 0,
            _ => theme.titlebar_height,
        }
    }
}

/// `v` as an `i32`, saturating at `i32::MAX`.
fn to_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
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
        let border2 = config.window_type.border(theme).saturating_mul(2);
        let titlebar_h = config.window_type.titlebar_height(theme);

        let outer_w = config.width.saturating_add(border2);
        let outer_h = config
            .height
            .saturating_add(titlebar_h)
            .saturating_add(border2);

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

        let border = self.window_type.border(theme);
        let titlebar_h = self.window_type.titlebar_height(theme);

        // Saturating: a frame at the far end of the coordinate space must
        // not overflow (debug builds panic on it).
        let cx = outer.x.saturating_add(to_i32(border));
        let cy = outer
            .y
            .saturating_add(to_i32(titlebar_h.saturating_add(border)));
        let cw = outer.w.saturating_sub(border.saturating_mul(2));
        let ch = outer
            .h
            .saturating_sub(titlebar_h)
            .saturating_sub(border.saturating_mul(2));
        (cx, cy, cw, ch)
    }

    /// Compute the titlebar rectangle.
    pub fn titlebar_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        if self.fullscreen_kiosk || self.window_type == WindowType::Fullscreen {
            return None;
        }
        let border = self.window_type.border(theme);
        let tx = self.x.saturating_add(to_i32(border));
        let ty = self.y.saturating_add(to_i32(border));
        let tw = self.outer_w.saturating_sub(border.saturating_mul(2));
        let th = theme.titlebar_height;
        Some((tx, ty, tw, th))
    }

    /// Compute button inset from the titlebar edge, derived from
    /// the titlebar height so buttons stay proportional.
    fn button_inset(theme: &WmTheme) -> i32 {
        ((theme.titlebar_height / 8) as i32).max(1)
    }

    /// Compute a button's X position given its index.
    ///
    /// `idx` counts from the edge indicated by `button_side` inward. With
    /// `button_side == "right"` the physical left-to-right order is minimize,
    /// maximize, close (Windows / GNOME convention: close at the corner).
    /// With `button_side == "left"` it is close, minimize, maximize (macOS
    /// traffic-light convention: close at the corner).
    ///
    /// Computed in `i64` and saturated to `i32`, so a titlebar at the far
    /// end of the coordinate space cannot overflow.
    fn button_x(theme: &WmTheme, tx: i32, tw: u32, idx: i32) -> i32 {
        let btn_size = i64::from(theme.button_size.min(theme.titlebar_height));
        let sp = i64::from(theme.button_spacing);
        let inset = i64::from(Self::button_inset(theme));
        let (tx, tw, idx) = (i64::from(tx), i64::from(tw), i64::from(idx));
        let x = if theme.button_side == "left" {
            tx + inset + idx * (btn_size + sp)
        } else {
            tx + tw - (idx + 1) * btn_size - idx * sp - inset
        };
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }

    /// Index of a titlebar button, counted from the button edge.
    ///
    /// Close is always at the edge, on either side (and when it is the only
    /// button, e.g. Dialog/FloatingWidget). Left side: close, minimize,
    /// maximize from the edge inward. Right side: minimize is furthest from
    /// the edge (leftmost of the group), maximize in the middle.
    fn button_idx(theme: &WmTheme, kind: ButtonKind) -> i32 {
        let left = theme.button_side == "left";
        match kind {
            ButtonKind::Close => 0,
            ButtonKind::Minimize => {
                if left {
                    1
                } else {
                    2
                }
            },
            ButtonKind::Maximize => {
                if left {
                    2
                } else {
                    1
                }
            },
        }
    }

    /// Whether this window type shows the titlebar button `kind`.
    pub fn has_button(&self, kind: ButtonKind) -> bool {
        match kind {
            ButtonKind::Close => self.has_close_button(),
            ButtonKind::Minimize => self.has_minimize_button(),
            ButtonKind::Maximize => self.has_maximize_button(),
        }
    }

    /// The rectangle of titlebar button `kind`, or `None` when the window
    /// has no titlebar or no such button.
    pub fn button_rect(&self, theme: &WmTheme, kind: ButtonKind) -> Option<(i32, i32, u32, u32)> {
        let (tx, ty, tw, th) = self.titlebar_rect(theme)?;
        if !self.has_button(kind) {
            return None;
        }
        let btn_size = theme.button_size.min(th);
        let bx = Self::button_x(theme, tx, tw, Self::button_idx(theme, kind));
        let by = ty.saturating_add(((th - btn_size) / 2) as i32);
        Some((bx, by, btn_size, btn_size))
    }

    /// Compute close button rectangle.
    pub fn close_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        self.button_rect(theme, ButtonKind::Close)
    }

    /// Compute minimize button rectangle.
    pub fn minimize_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        self.button_rect(theme, ButtonKind::Minimize)
    }

    /// Compute maximize button rectangle.
    pub fn maximize_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        self.button_rect(theme, ButtonKind::Maximize)
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
        // In i64 (saturated back to i32 at the end), so a titlebar at the
        // far end of the coordinate space cannot overflow.
        let btn_size = i64::from(theme.button_size.min(theme.titlebar_height));
        let sp = i64::from(theme.button_spacing);
        let inset = i64::from(Self::button_inset(theme));
        let text_inset = inset * 2;
        let btn_count = i64::from(self.button_count());
        // Pixels reserved on the button side: edge inset + buttons + gap.
        let buttons_w = if btn_count > 0 {
            inset + btn_count * btn_size + (btn_count - 1) * sp + text_inset
        } else {
            text_inset
        };
        let (tx, tw) = (i64::from(tx), i64::from(tw));
        let font = theme.titlebar_font_size;

        let (region_x, region_w) = if theme.title_align == "center" {
            (tx + buttons_w, tw - buttons_w * 2)
        } else if theme.button_side == "left" {
            (tx + buttons_w, tw - buttons_w - text_inset)
        } else {
            (tx + text_inset, tw - buttons_w - text_inset)
        };
        let region_w = region_w.clamp(0, i64::from(i32::MAX)) as i32;

        let text = truncate_to_width(&self.title, font, region_w);
        let text_w = text_px(&text, font).min(region_w);
        let (rw, text_w64) = (i64::from(region_w), i64::from(text_w));
        let x = if theme.title_align == "center" {
            // Center on the full titlebar; the symmetric reservation keeps
            // the result inside [region_x, region_x + region_w].
            (tx + (tw - text_w64) / 2).clamp(region_x, region_x + rw - text_w64)
        } else {
            region_x
        };
        let x = x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
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
