//! [`WmTheme`]: the visual parameters window chrome is built from.

use oasis_types::backend::Color;
use oasis_types::nine_patch::NinePatchSlices;

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
    /// Titlebar text color (active window).
    pub titlebar_text_color: Color,
    /// Titlebar text color (inactive window).
    pub titlebar_text_inactive_color: Color,
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

    // -- Nine-patch chrome --
    /// Titlebar nine-patch from the skin: (asset key, [l, t, r, b] insets).
    /// The shell resolves it into `titlebar_patch` once a backend exists.
    pub titlebar_nine_patch: Option<(String, [u16; 4])>,
    /// Frame nine-patch from the skin: (asset key, insets).
    pub frame_nine_patch: Option<(String, [u16; 4])>,
    /// Uploaded titlebar texture + slices (set by the shell on skin swap).
    pub titlebar_patch: Option<(oasis_types::backend::TextureId, NinePatchSlices)>,
    /// Uploaded frame texture + slices.
    pub frame_patch: Option<(oasis_types::backend::TextureId, NinePatchSlices)>,
}

impl Default for WmTheme {
    fn default() -> Self {
        Self {
            titlebar_height: 24,
            border_width: 1,
            titlebar_active_color: Color::rgb(50, 80, 140),
            titlebar_inactive_color: Color::rgb(80, 80, 80),
            titlebar_text_color: Color::WHITE,
            titlebar_text_inactive_color: Color::WHITE,
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
            titlebar_nine_patch: None,
            frame_nine_patch: None,
            titlebar_patch: None,
            frame_patch: None,
        }
    }
}
