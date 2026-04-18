//! Window types and configuration for the window manager.
//!
//! The WM is a consumer of the SDI API. Each `Window` owns a group of SDI
//! objects identified by a naming convention: `"{id}.frame"`, `"{id}.titlebar"`,
//! etc. The WM handles behavior; the skin handles appearance.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use oasis_types::backend::Color;

/// Shared, reference-counted window identifier.
///
/// Wraps `Rc<str>` for cheap cloning at 60fps. Compares equal with
/// `&str`, `String`, and other `WindowId` values, so existing call
/// sites that do `id == "browser"` keep working.
#[derive(Clone, Eq)]
pub struct WindowId(Rc<str>);

impl WindowId {
    /// Create a new window id from any string-like value.
    pub fn new(s: impl Into<Rc<str>>) -> Self {
        Self(s.into())
    }

    /// Borrow the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for WindowId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for WindowId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &*self.0)
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Hash for WindowId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self.0).hash(state);
    }
}

impl PartialEq for WindowId {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl PartialEq<str> for WindowId {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for WindowId {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for WindowId {
    fn eq(&self, other: &String) -> bool {
        &*self.0 == other.as_str()
    }
}

impl PartialEq<WindowId> for str {
    fn eq(&self, other: &WindowId) -> bool {
        self == &*other.0
    }
}

impl PartialEq<WindowId> for &str {
    fn eq(&self, other: &WindowId) -> bool {
        *self == &*other.0
    }
}

impl PartialEq<WindowId> for String {
    fn eq(&self, other: &WindowId) -> bool {
        self.as_str() == &*other.0
    }
}

impl From<String> for WindowId {
    fn from(s: String) -> Self {
        Self(Rc::from(s))
    }
}

impl From<&str> for WindowId {
    fn from(s: &str) -> Self {
        Self(Rc::from(s))
    }
}

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

/// Visual theme parameters for window rendering.
///
/// In Phase 7 these are hardcoded defaults. Phase 8 (skin system) will
/// populate these from the active skin's configuration.
#[derive(Debug, Clone)]
pub struct WmTheme {
    /// Titlebar height in pixels.
    pub titlebar_height: u32,
    /// Frame border width in pixels.
    pub border_width: u32,
    /// Titlebar background color (active window).
    pub titlebar_active_color: Color,
    /// Titlebar background color (inactive window).
    pub titlebar_inactive_color: Color,
    /// Titlebar text color.
    pub titlebar_text_color: Color,
    /// Frame/border color.
    pub frame_color: Color,
    /// Content area background color.
    pub content_bg_color: Color,
    /// Close button color.
    pub btn_close_color: Color,
    /// Minimize button color.
    pub btn_minimize_color: Color,
    /// Maximize button color.
    pub btn_maximize_color: Color,
    /// Button size (square, width = height).
    pub button_size: u32,
    /// Resize handle hit area size in pixels.
    pub resize_handle_size: u32,
    /// Font size for titlebar text.
    pub titlebar_font_size: u16,

    // -- Extended visual properties --
    /// Titlebar corner radius (top corners only).
    pub titlebar_radius: u16,
    /// Whether the titlebar uses a gradient fill.
    pub titlebar_gradient: bool,
    /// Explicit active titlebar gradient top color (overrides auto-derive).
    pub titlebar_gradient_top: Option<Color>,
    /// Explicit active titlebar gradient bottom color.
    pub titlebar_gradient_bottom: Option<Color>,
    /// Explicit inactive titlebar gradient top color.
    pub titlebar_inactive_gradient_top: Option<Color>,
    /// Explicit inactive titlebar gradient bottom color.
    pub titlebar_inactive_gradient_bottom: Option<Color>,
    /// Shadow elevation for window frames (0 = none).
    pub frame_shadow_level: u8,
    /// Frame corner radius.
    pub frame_border_radius: u16,
    /// Window button corner radius.
    pub button_radius: u16,

    // -- Tier 1: Button layout and title alignment --
    /// Which side the window buttons are on: "right" or "left".
    pub button_side: String,
    /// Close button glyph text.
    pub glyph_close: String,
    /// Minimize button glyph text.
    pub glyph_minimize: String,
    /// Maximize button glyph text.
    pub glyph_maximize: String,
    /// Title text alignment: "left" or "center".
    pub title_align: String,

    // -- Tier 2: Separator and glyph colors --
    /// Whether a 1px separator line is drawn at the titlebar bottom edge.
    pub separator_enabled: bool,
    /// Separator line color.
    pub separator_color: Color,
    /// Close glyph text color.
    pub glyph_close_color: Color,
    /// Minimize glyph text color.
    pub glyph_minimize_color: Color,
    /// Maximize glyph text color.
    pub glyph_maximize_color: Color,
    /// Spacing between window buttons.
    pub button_spacing: i32,

    // -- Tier 3: Hover, shadow, stroke, insets --
    /// Close button hover color.
    pub btn_close_hover: Color,
    /// Minimize button hover color.
    pub btn_minimize_hover: Color,
    /// Maximize button hover color.
    pub btn_maximize_hover: Color,
    /// Whether title text has a drop shadow.
    pub title_text_shadow: bool,
    /// Title text shadow color.
    pub title_text_shadow_color: Color,
    /// Content area stroke width (0 = none).
    pub content_stroke_width: u16,
    /// Content area stroke color.
    pub content_stroke_color: Color,
    /// Top inset when maximized (for status bar awareness).
    pub maximize_top_inset: u32,
    /// Bottom inset when maximized (for bottom bar awareness).
    pub maximize_bottom_inset: u32,
    /// Color for the semi-transparent modal backdrop overlay.
    pub modal_overlay_color: Color,
    /// Alpha applied to inactive window frames (default 180; 255 = no dim).
    pub inactive_frame_alpha: u8,
}

impl Default for WmTheme {
    fn default() -> Self {
        Self {
            titlebar_height: 24,
            border_width: 1,
            titlebar_active_color: Color::rgb(50, 80, 140),
            titlebar_inactive_color: Color::rgb(80, 80, 80),
            titlebar_text_color: Color::WHITE,
            frame_color: Color::rgb(40, 40, 40),
            content_bg_color: Color::rgb(30, 30, 30),
            btn_close_color: Color::rgb(200, 60, 60),
            btn_minimize_color: Color::rgb(200, 180, 60),
            btn_maximize_color: Color::rgb(60, 180, 60),
            button_size: 16,
            resize_handle_size: 6,
            titlebar_font_size: 12,
            titlebar_radius: 0,
            titlebar_gradient: false,
            titlebar_gradient_top: None,
            titlebar_gradient_bottom: None,
            titlebar_inactive_gradient_top: None,
            titlebar_inactive_gradient_bottom: None,
            frame_shadow_level: 0,
            frame_border_radius: 0,
            button_radius: 0,
            button_side: "right".to_string(),
            glyph_close: "x".to_string(),
            glyph_minimize: "-".to_string(),
            glyph_maximize: "\u{25A1}".to_string(),
            title_align: "left".to_string(),
            separator_enabled: false,
            separator_color: Color::rgba(255, 255, 255, 30),
            glyph_close_color: Color::WHITE,
            glyph_minimize_color: Color::WHITE,
            glyph_maximize_color: Color::WHITE,
            button_spacing: 2,
            btn_close_hover: Color::rgb(220, 80, 80),
            btn_minimize_hover: Color::rgb(220, 200, 80),
            btn_maximize_hover: Color::rgb(80, 200, 80),
            title_text_shadow: false,
            title_text_shadow_color: Color::rgba(0, 0, 0, 150),
            content_stroke_width: 0,
            content_stroke_color: Color::rgba(255, 255, 255, 20),
            maximize_top_inset: 0,
            maximize_bottom_inset: 0,
            modal_overlay_color: Color::rgba(0, 0, 0, 100),
            inactive_frame_alpha: 180,
        }
    }
}

/// Stored geometry for restore-from-maximize.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
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
        }
    }

    /// Compute the content area rectangle (position and size within the frame).
    pub fn content_rect(&self, theme: &WmTheme) -> (i32, i32, u32, u32) {
        if self.fullscreen_kiosk {
            return (self.x, self.y, self.outer_w, self.outer_h);
        }

        let has_titlebar = self.window_type != WindowType::Fullscreen;
        let has_border = matches!(self.window_type, WindowType::AppWindow | WindowType::Dialog);

        let border = if has_border { theme.border_width } else { 0 };
        let titlebar_h = if has_titlebar {
            theme.titlebar_height
        } else {
            0
        };

        let cx = self.x + border as i32;
        let cy = self.y + titlebar_h as i32 + border as i32;
        let cw = self.outer_w.saturating_sub(border * 2);
        let ch = self
            .outer_h
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
    /// Indices are assigned so that physical left-to-right order is always
    /// minimize (leftmost), maximize (middle), close (rightmost), regardless
    /// of whether `button_side` is "left" or "right". `idx` counts from the
    /// edge indicated by `button_side` inward.
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

    /// Index of the minimize button (leftmost when present).
    fn minimize_btn_idx(&self, theme: &WmTheme) -> i32 {
        if theme.button_side == "left" { 0 } else { 2 }
    }

    /// Index of the maximize button (middle when present).
    fn maximize_btn_idx(&self) -> i32 {
        1
    }

    /// Index of the close button (rightmost when present).
    ///
    /// When close is the only button on the titlebar (Dialog, FloatingWidget),
    /// put it at the edge so it doesn't float out in empty space.
    fn close_btn_idx(&self, theme: &WmTheme) -> i32 {
        let alone = !self.has_minimize_button() && !self.has_maximize_button();
        if alone {
            0
        } else if theme.button_side == "left" {
            2
        } else {
            0
        }
    }

    /// Compute close button rectangle.
    pub fn close_btn_rect(&self, theme: &WmTheme) -> Option<(i32, i32, u32, u32)> {
        let (tx, ty, tw, th) = self.titlebar_rect(theme)?;
        if !self.has_close_button() {
            return None;
        }
        let btn_size = theme.button_size.min(th);
        let bx = self.button_x(theme, tx, tw, self.close_btn_idx(theme));
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
        let bx = self.button_x(theme, tx, tw, self.maximize_btn_idx());
        let by = ty + (th as i32 - btn_size as i32) / 2;
        Some((bx, by, btn_size, btn_size))
    }

    /// Compute the title text X position and available width.
    pub fn title_text_x(&self, theme: &WmTheme) -> Option<(i32, u32)> {
        let (tx, _ty, tw, _th) = self.titlebar_rect(theme)?;
        let btn_size = theme.button_size.min(theme.titlebar_height) as i32;
        let sp = theme.button_spacing;
        // Count how many buttons this window type has.
        let btn_count = [
            self.has_close_button(),
            self.has_minimize_button(),
            self.has_maximize_button(),
        ]
        .iter()
        .filter(|&&v| v)
        .count() as i32;
        let inset = Self::button_inset(theme);
        let text_inset = inset * 2; // padding on each side
        let buttons_w = if btn_count > 0 {
            btn_count * btn_size + (btn_count - 1) * sp + text_inset
        } else {
            0
        };
        let margin = buttons_w as u32 + text_inset as u32 * 2;
        let (text_x, avail_w) = if theme.title_align == "center" {
            (tx + text_inset, tw.saturating_sub(margin))
        } else if theme.button_side == "left" {
            (tx + buttons_w + text_inset, tw.saturating_sub(margin))
        } else {
            (tx + text_inset, tw.saturating_sub(margin))
        };
        Some((text_x, avail_w))
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
    pub fn sdi_suffixes(&self) -> Vec<&'static str> {
        match self.window_type {
            WindowType::Fullscreen => vec!["content"],
            WindowType::FloatingWidget => vec![
                "frame",
                "titlebar",
                "title_text",
                "title_shadow",
                "separator",
                "btn_close",
                "btn_close_glyph",
                "content",
                "content_stroke",
            ],
            WindowType::Panel => vec![
                "frame",
                "titlebar",
                "title_text",
                "title_shadow",
                "separator",
                "content",
                "content_stroke",
            ],
            WindowType::Dialog => vec![
                "frame",
                "titlebar",
                "title_text",
                "title_shadow",
                "separator",
                "btn_close",
                "btn_close_glyph",
                "content",
                "content_stroke",
            ],
            WindowType::AppWindow => vec![
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
            ],
        }
    }

    /// Build the full SDI object name for a suffix.
    pub fn sdi_name(&self, suffix: &str) -> String {
        format!("{}.{suffix}", self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> WindowConfig {
        WindowConfig {
            id: "test_win".to_string(),
            title: "Test Window".to_string(),
            x: None,
            y: None,
            width: 200,
            height: 150,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        }
    }

    #[test]
    fn window_new_computes_outer_dimensions() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 10, 20, &theme);
        // outer_w = content_w + 2*border = 200 + 2 = 202
        assert_eq!(win.outer_w, 200 + theme.border_width * 2);
        // outer_h = content_h + titlebar + 2*border = 150 + 24 + 2 = 176
        assert_eq!(
            win.outer_h,
            150 + theme.titlebar_height + theme.border_width * 2
        );
    }

    #[test]
    fn fullscreen_has_no_border_or_titlebar() {
        let theme = WmTheme::default();
        let config = WindowConfig {
            id: "fs".to_string(),
            title: "Full".to_string(),
            x: None,
            y: None,
            width: 480,
            height: 272,
            window_type: WindowType::Fullscreen,
            always_on_top: false,
            modal: false,
        };
        let win = Window::new(&config, 0, 0, &theme);
        assert_eq!(win.outer_w, 480);
        assert_eq!(win.outer_h, 272);
        assert!(win.titlebar_rect(&theme).is_none());
    }

    #[test]
    fn content_rect_app_window() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 10, 20, &theme);
        let (cx, cy, cw, ch) = win.content_rect(&theme);
        assert_eq!(cx, 10 + theme.border_width as i32);
        assert_eq!(
            cy,
            20 + theme.titlebar_height as i32 + theme.border_width as i32
        );
        assert_eq!(cw, 200);
        assert_eq!(ch, 150);
    }

    #[test]
    fn titlebar_rect_inside_border() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 10, 20, &theme);
        let (tx, ty, tw, th) = win.titlebar_rect(&theme).unwrap();
        assert_eq!(tx, 10 + theme.border_width as i32);
        assert_eq!(ty, 20 + theme.border_width as i32);
        assert_eq!(tw, 200); // outer_w - 2*border
        assert_eq!(th, theme.titlebar_height);
    }

    #[test]
    fn close_button_top_right() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 0, 0, &theme);
        let (bx, _by, bw, _bh) = win.close_btn_rect(&theme).unwrap();
        let (tx, _ty, tw, _th) = win.titlebar_rect(&theme).unwrap();
        // Close button is near the right edge of the titlebar.
        assert!(bx + bw as i32 <= tx + tw as i32);
        assert!(bx > tx + tw as i32 / 2); // Right half.
    }

    #[test]
    fn dialog_has_no_minimize_maximize() {
        let theme = WmTheme::default();
        let config = WindowConfig {
            id: "dlg".to_string(),
            title: "Dialog".to_string(),
            x: None,
            y: None,
            width: 300,
            height: 100,
            window_type: WindowType::Dialog,
            always_on_top: false,
            modal: false,
        };
        let win = Window::new(&config, 0, 0, &theme);
        assert!(win.has_close_button());
        assert!(!win.has_minimize_button());
        assert!(!win.has_maximize_button());
        assert!(!win.is_resizable());
    }

    #[test]
    fn floating_widget_draggable_no_resize() {
        let theme = WmTheme::default();
        let config = WindowConfig {
            id: "widget".to_string(),
            title: "Clock".to_string(),
            x: None,
            y: None,
            width: 80,
            height: 40,
            window_type: WindowType::FloatingWidget,
            always_on_top: false,
            modal: false,
        };
        let win = Window::new(&config, 0, 0, &theme);
        assert!(win.is_draggable());
        assert!(!win.is_resizable());
        assert!(win.has_close_button());
        assert!(!win.has_minimize_button());
    }

    #[test]
    fn sdi_suffixes_by_type() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 0, 0, &theme);
        let suffixes = win.sdi_suffixes();
        assert!(suffixes.contains(&"frame"));
        assert!(suffixes.contains(&"titlebar"));
        assert!(suffixes.contains(&"btn_close"));
        assert!(suffixes.contains(&"btn_minimize"));
        assert!(suffixes.contains(&"btn_maximize"));
        assert!(suffixes.contains(&"content"));
    }

    #[test]
    fn sdi_name_formatting() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 0, 0, &theme);
        assert_eq!(win.sdi_name("frame"), "test_win.frame");
        assert_eq!(win.sdi_name("titlebar"), "test_win.titlebar");
    }

    #[test]
    fn panel_not_draggable() {
        let config = WindowConfig {
            id: "taskbar".to_string(),
            title: "Taskbar".to_string(),
            x: None,
            y: None,
            width: 480,
            height: 32,
            window_type: WindowType::Panel,
            always_on_top: false,
            modal: false,
        };
        let theme = WmTheme::default();
        let win = Window::new(&config, 0, 0, &theme);
        assert!(!win.is_draggable());
        assert!(!win.is_resizable());
        assert!(!win.has_close_button());
    }

    #[test]
    fn default_theme_reasonable() {
        let theme = WmTheme::default();
        assert!(theme.titlebar_height > 0);
        assert!(theme.button_size > 0);
        assert!(theme.button_size <= theme.titlebar_height);
        assert!(theme.resize_handle_size > 0);
    }

    #[test]
    fn kiosk_mode_content_rect_is_full_bounds() {
        let theme = WmTheme::default();
        let config = test_config();
        let mut win = Window::new(&config, 0, 0, &theme);
        win.fullscreen_kiosk = true;
        win.x = 0;
        win.y = 0;
        win.outer_w = 480;
        win.outer_h = 272;
        let (cx, cy, cw, ch) = win.content_rect(&theme);
        assert_eq!((cx, cy, cw, ch), (0, 0, 480, 272));
    }

    #[test]
    fn kiosk_mode_titlebar_is_none() {
        let theme = WmTheme::default();
        let config = test_config();
        let mut win = Window::new(&config, 10, 20, &theme);
        assert!(win.titlebar_rect(&theme).is_some());
        win.fullscreen_kiosk = true;
        assert!(win.titlebar_rect(&theme).is_none());
    }

    #[test]
    fn kiosk_mode_default_false() {
        let theme = WmTheme::default();
        let config = test_config();
        let win = Window::new(&config, 0, 0, &theme);
        assert!(!win.fullscreen_kiosk);
    }
}
