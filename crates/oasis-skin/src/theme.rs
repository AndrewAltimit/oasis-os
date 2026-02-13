//! Skin theme -- color scheme and visual properties.
//!
//! The theme defines the color palette and optional WM visual overrides
//! for a skin. Loaded from `theme.toml`.

use serde::Deserialize;

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};
use oasis_types::shadow::Shadow;
use oasis_ui::theme::Theme;
use oasis_wm::WmTheme;

/// Color scheme for a skin.
#[derive(Debug, Clone, Deserialize)]
pub struct SkinTheme {
    /// Main background color.
    #[serde(default = "default_bg")]
    pub background: String,
    /// Primary accent color (active elements, highlights).
    #[serde(default = "default_primary")]
    pub primary: String,
    /// Secondary color (borders, separators).
    #[serde(default = "default_secondary")]
    pub secondary: String,
    /// Default text color.
    #[serde(default = "default_text")]
    pub text: String,
    /// Dimmed/secondary text color.
    #[serde(default = "default_dim_text")]
    pub dim_text: String,
    /// Status bar background color.
    #[serde(default = "default_status_bar")]
    pub status_bar: String,
    /// Terminal prompt color.
    #[serde(default = "default_prompt")]
    pub prompt: String,
    /// Terminal output text color.
    #[serde(default = "default_output")]
    pub output: String,
    /// Terminal error text color.
    #[serde(default = "default_error")]
    pub error: String,

    // -- Extended visual fields (optional, for modern rendering) --
    /// Surface color override (default: derived from background).
    #[serde(default)]
    pub surface: Option<String>,
    /// Accent hover color override (default: derived from primary).
    #[serde(default)]
    pub accent_hover: Option<String>,
    /// Default border radius for UI elements (pixels).
    #[serde(default)]
    pub border_radius: Option<u16>,
    /// Shadow intensity (0 = none, 1 = subtle, 2 = medium, 3 = heavy).
    #[serde(default)]
    pub shadow_intensity: Option<u8>,
    /// Whether gradient fills are enabled for this skin.
    #[serde(default)]
    pub gradient_enabled: Option<bool>,

    /// Whether the WM is visually themed by this skin.
    #[serde(default)]
    pub wm_theme: Option<WmThemeOverrides>,

    /// Per-element color overrides for status/bottom bars.
    #[serde(default)]
    pub bar_overrides: Option<BarOverrides>,

    /// Per-element color overrides for dashboard icons.
    #[serde(default)]
    pub icon_overrides: Option<IconOverrides>,

    /// Per-element color overrides for browser chrome.
    #[serde(default)]
    pub browser_overrides: Option<BrowserOverrides>,

    /// Per-element color overrides for the start menu popup.
    #[serde(default)]
    pub start_menu_overrides: Option<StartMenuOverrides>,

    /// Wallpaper generation configuration.
    #[serde(default)]
    pub wallpaper: Option<WallpaperConfig>,

    /// Geometry overrides (bar heights, icon sizes, font sizes).
    #[serde(default)]
    pub geometry: Option<GeometryOverrides>,
}

/// Optional overrides for the window manager theme.
#[derive(Debug, Clone, Deserialize)]
pub struct WmThemeOverrides {
    pub titlebar_height: Option<u32>,
    pub border_width: Option<u32>,
    pub titlebar_active: Option<String>,
    pub titlebar_inactive: Option<String>,
    pub titlebar_text: Option<String>,
    pub frame_color: Option<String>,
    pub content_bg: Option<String>,
    pub btn_close: Option<String>,
    pub btn_minimize: Option<String>,
    pub btn_maximize: Option<String>,
    pub button_size: Option<u32>,
    pub resize_handle_size: Option<u32>,
    pub titlebar_font_size: Option<u16>,
    // Extended visual properties.
    #[serde(default)]
    pub titlebar_radius: Option<u16>,
    #[serde(default)]
    pub titlebar_gradient: Option<bool>,
    #[serde(default)]
    pub titlebar_gradient_top: Option<String>,
    #[serde(default)]
    pub titlebar_gradient_bottom: Option<String>,
    #[serde(default)]
    pub titlebar_inactive_gradient_top: Option<String>,
    #[serde(default)]
    pub titlebar_inactive_gradient_bottom: Option<String>,
    #[serde(default)]
    pub frame_shadow_level: Option<u8>,
    #[serde(default)]
    pub frame_border_radius: Option<u16>,
    #[serde(default)]
    pub button_radius: Option<u16>,
    // Tier 1
    #[serde(default)]
    pub button_side: Option<String>,
    #[serde(default)]
    pub glyph_close: Option<String>,
    #[serde(default)]
    pub glyph_minimize: Option<String>,
    #[serde(default)]
    pub glyph_maximize: Option<String>,
    #[serde(default)]
    pub title_align: Option<String>,
    // Tier 2
    #[serde(default)]
    pub separator_enabled: Option<bool>,
    #[serde(default)]
    pub separator_color: Option<String>,
    #[serde(default)]
    pub glyph_close_color: Option<String>,
    #[serde(default)]
    pub glyph_minimize_color: Option<String>,
    #[serde(default)]
    pub glyph_maximize_color: Option<String>,
    #[serde(default)]
    pub button_spacing: Option<i32>,
    // Tier 3
    #[serde(default)]
    pub btn_close_hover: Option<String>,
    #[serde(default)]
    pub btn_minimize_hover: Option<String>,
    #[serde(default)]
    pub btn_maximize_hover: Option<String>,
    #[serde(default)]
    pub title_text_shadow: Option<bool>,
    #[serde(default)]
    pub title_text_shadow_color: Option<String>,
    #[serde(default)]
    pub content_stroke_width: Option<u16>,
    #[serde(default)]
    pub content_stroke_color: Option<String>,
    #[serde(default)]
    pub maximize_top_inset: Option<u32>,
    #[serde(default)]
    pub maximize_bottom_inset: Option<u32>,
}

/// Per-element overrides for status bar and bottom bar colors.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BarOverrides {
    pub bar_bg: Option<String>,
    pub statusbar_bg: Option<String>,
    pub separator_color: Option<String>,
    pub battery_color: Option<String>,
    pub version_color: Option<String>,
    pub clock_color: Option<String>,
    pub url_color: Option<String>,
    pub usb_color: Option<String>,
    pub tab_active_fill: Option<String>,
    pub tab_active_alpha: Option<u8>,
    pub tab_inactive_alpha: Option<u8>,
    pub media_tab_active: Option<String>,
    pub media_tab_inactive: Option<String>,
    pub pipe_color: Option<String>,
    pub r_hint_color: Option<String>,
    pub category_label_color: Option<String>,
    pub page_dot_active: Option<String>,
    pub page_dot_inactive: Option<String>,
    pub statusbar_gradient_top: Option<String>,
    pub statusbar_gradient_bottom: Option<String>,
    pub bar_gradient_top: Option<String>,
    pub bar_gradient_bottom: Option<String>,
}

/// Per-element overrides for dashboard icon rendering.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IconOverrides {
    pub body_color: Option<String>,
    pub fold_color: Option<String>,
    pub outline_color: Option<String>,
    pub shadow_color: Option<String>,
    pub label_color: Option<String>,
    pub cursor_color: Option<String>,
    pub icon_border_radius: Option<u16>,
    pub cursor_border_radius: Option<u16>,
    pub cursor_stroke_width: Option<u16>,
    /// Icon style variant: "document" (default), "card", or "circle".
    #[serde(default)]
    pub icon_style: Option<String>,
    /// Cursor style variant: "stroke" (default), "fill", or "underline".
    #[serde(default)]
    pub cursor_style: Option<String>,
}

/// Wallpaper generation configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WallpaperConfig {
    /// Style: "gradient" (default), "solid", or "none".
    pub style: Option<String>,
    /// Hex color stops for gradient wallpaper.
    pub color_stops: Option<Vec<String>>,
    /// Whether PSIX arc ripple waves are enabled (default true).
    pub wave_enabled: Option<bool>,
    /// Wave intensity 0.0-1.0 (default 1.0).
    pub wave_intensity: Option<f32>,
    /// Gradient angle in degrees: 0=horizontal, 90=vertical (default 0).
    pub angle: Option<f32>,
}

/// Geometry overrides for bar heights, icon sizes, and font sizes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeometryOverrides {
    pub statusbar_height: Option<u32>,
    pub bottombar_height: Option<u32>,
    pub tab_row_height: Option<u32>,
    pub icon_width: Option<u32>,
    pub icon_height: Option<u32>,
    pub font_small: Option<u16>,
}

/// Per-element overrides for the start menu popup and button.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StartMenuOverrides {
    pub panel_bg: Option<String>,
    pub panel_gradient_top: Option<String>,
    pub panel_gradient_bottom: Option<String>,
    pub panel_border: Option<String>,
    pub item_text: Option<String>,
    pub item_text_active: Option<String>,
    pub highlight_color: Option<String>,
    pub button_bg: Option<String>,
    pub button_text: Option<String>,
    pub panel_border_radius: Option<u16>,
    pub panel_shadow_level: Option<u8>,
    // Tier 1: Layout
    #[serde(default)]
    pub layout_mode: Option<String>,
    #[serde(default)]
    pub button_label: Option<String>,
    #[serde(default)]
    pub button_width: Option<u32>,
    #[serde(default)]
    pub button_height: Option<u32>,
    #[serde(default)]
    pub button_shape: Option<String>,
    #[serde(default)]
    pub panel_width: Option<u32>,
    #[serde(default)]
    pub columns: Option<usize>,
    // Tier 2: Header/footer
    #[serde(default)]
    pub header_text: Option<String>,
    #[serde(default)]
    pub header_bg: Option<String>,
    #[serde(default)]
    pub header_text_color: Option<String>,
    #[serde(default)]
    pub header_height: Option<u32>,
    #[serde(default)]
    pub footer_enabled: Option<bool>,
    #[serde(default)]
    pub footer_bg: Option<String>,
    #[serde(default)]
    pub footer_text_color: Option<String>,
    #[serde(default)]
    pub footer_height: Option<u32>,
    // Tier 2: Item geometry + button gradient
    #[serde(default)]
    pub item_icon_size: Option<u32>,
    #[serde(default)]
    pub item_row_height: Option<i32>,
    #[serde(default)]
    pub button_gradient: Option<bool>,
    #[serde(default)]
    pub button_gradient_top: Option<String>,
    #[serde(default)]
    pub button_gradient_bottom: Option<String>,
}

/// Per-element overrides for browser chrome colors.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrowserOverrides {
    pub chrome_bg: Option<String>,
    pub chrome_text: Option<String>,
    pub chrome_button_bg: Option<String>,
    pub url_bar_bg: Option<String>,
    pub url_bar_text: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_bar_text: Option<String>,
    pub link_color: Option<String>,
}

fn default_bg() -> String {
    "#1A1A2D".to_string()
}
fn default_primary() -> String {
    "#3264C8".to_string()
}
fn default_secondary() -> String {
    "#505050".to_string()
}
fn default_text() -> String {
    "#FFFFFF".to_string()
}
fn default_dim_text() -> String {
    "#808080".to_string()
}
fn default_status_bar() -> String {
    "#283C5A".to_string()
}
fn default_prompt() -> String {
    "#00FF00".to_string()
}
fn default_output() -> String {
    "#CCCCCC".to_string()
}
fn default_error() -> String {
    "#FF4444".to_string()
}

impl Default for SkinTheme {
    fn default() -> Self {
        Self {
            background: default_bg(),
            primary: default_primary(),
            secondary: default_secondary(),
            text: default_text(),
            dim_text: default_dim_text(),
            status_bar: default_status_bar(),
            prompt: default_prompt(),
            output: default_output(),
            error: default_error(),
            surface: None,
            accent_hover: None,
            border_radius: None,
            shadow_intensity: None,
            gradient_enabled: None,
            wm_theme: None,
            bar_overrides: None,
            icon_overrides: None,
            browser_overrides: None,
            start_menu_overrides: None,
            wallpaper: None,
            geometry: None,
        }
    }
}

impl SkinTheme {
    /// Parse the background color string to a `Color`.
    pub fn background_color(&self) -> Color {
        parse_hex_color(&self.background).unwrap_or(Color::BLACK)
    }

    /// Parse the primary color string to a `Color`.
    pub fn primary_color(&self) -> Color {
        parse_hex_color(&self.primary).unwrap_or(Color::WHITE)
    }

    /// Parse the text color string to a `Color`.
    pub fn text_color(&self) -> Color {
        parse_hex_color(&self.text).unwrap_or(Color::WHITE)
    }

    /// Parse the prompt color string to a `Color`.
    pub fn prompt_color(&self) -> Color {
        parse_hex_color(&self.prompt).unwrap_or(Color::rgb(0, 255, 0))
    }

    /// Parse the output color string to a `Color`.
    pub fn output_color(&self) -> Color {
        parse_hex_color(&self.output).unwrap_or(Color::rgb(204, 204, 204))
    }

    /// Parse the error color string to a `Color`.
    pub fn error_color(&self) -> Color {
        parse_hex_color(&self.error).unwrap_or(Color::rgb(255, 68, 68))
    }

    /// Parse the secondary color string to a `Color`.
    pub fn secondary_color(&self) -> Color {
        parse_hex_color(&self.secondary).unwrap_or(Color::rgb(80, 80, 80))
    }

    /// Parse the dim_text color string to a `Color`.
    pub fn dim_text_color(&self) -> Color {
        parse_hex_color(&self.dim_text).unwrap_or(Color::rgb(128, 128, 128))
    }

    /// Convert the 9-color skin palette into a full `ui::Theme`.
    ///
    /// Derives all 50+ fields from the base colors using lighten/darken.
    /// Optional extended fields (`surface`, `accent_hover`, etc.) override
    /// the derived values when present.
    pub fn to_ui_theme(&self) -> Theme {
        let bg = self.background_color();
        let primary = self.primary_color();
        let secondary = self.secondary_color();
        let text = self.text_color();
        let dim = self.dim_text_color();
        let err = self.error_color();

        // Surface variants: lighten background by 5% and 10%.
        let surface = self
            .surface
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| lighten(bg, 0.05));
        let surface_variant = lighten(bg, 0.10);

        // Accent variants: derived from primary.
        let accent = primary;
        let accent_hover = self
            .accent_hover
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| lighten(primary, 0.15));
        let accent_pressed = darken(primary, 0.85);
        let accent_subtle = with_alpha(primary, 30);

        // Border radius and shadow from extended fields.
        let radius = self.border_radius.unwrap_or(4);
        let shadow_level = self.shadow_intensity.unwrap_or(1);

        Theme {
            background: bg,
            surface,
            surface_variant,
            overlay: Color::rgba(0, 0, 0, 180),

            text_primary: text,
            text_secondary: dim,
            text_disabled: darken(dim, 0.6),
            text_on_accent: text,

            accent,
            accent_hover,
            accent_pressed,
            accent_subtle,

            success: Color::rgb(80, 200, 120),
            warning: Color::rgb(255, 180, 50),
            error: err,
            info: accent,

            border: secondary,
            border_subtle: darken(secondary, 0.7),
            border_strong: primary,

            button_bg: secondary,
            button_bg_hover: lighten(secondary, 0.15),
            button_bg_pressed: darken(secondary, 0.85),
            button_bg_disabled: darken(secondary, 0.5),
            input_bg: darken(bg, 0.8),
            input_border: secondary,
            input_border_focus: primary,
            scrollbar_track: Color::rgba(255, 255, 255, 10),
            scrollbar_thumb: Color::rgba(255, 255, 255, 40),
            scrollbar_thumb_hover: Color::rgba(255, 255, 255, 80),
            tooltip_bg: lighten(bg, 0.15),
            tooltip_text: text,

            font_size_xs: 8,
            font_size_sm: 8,
            font_size_md: 8,
            font_size_lg: 16,
            font_size_xl: 16,
            font_size_xxl: 24,

            spacing_xs: 2,
            spacing_sm: 4,
            spacing_md: 8,
            spacing_lg: 12,
            spacing_xl: 16,

            border_radius_sm: (radius / 2).max(1),
            border_radius_md: radius,
            border_radius_lg: radius * 2,
            border_radius_xl: radius * 3,

            shadow_card: Shadow::elevation(shadow_level.min(1)),
            shadow_dropdown: Shadow::elevation(shadow_level.min(2)),
            shadow_modal: Shadow::elevation(shadow_level.min(3)),
            shadow_tooltip: Shadow::elevation(shadow_level.min(2)),
        }
    }

    /// Build a `WmTheme` from the defaults plus any overrides.
    pub fn build_wm_theme(&self) -> WmTheme {
        let mut theme = WmTheme::default();
        if let Some(ref ov) = self.wm_theme {
            if let Some(h) = ov.titlebar_height {
                theme.titlebar_height = h;
            }
            if let Some(w) = ov.border_width {
                theme.border_width = w;
            }
            if let Some(ref c) = ov.titlebar_active
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_active_color = parsed;
            }
            if let Some(ref c) = ov.titlebar_inactive
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_inactive_color = parsed;
            }
            if let Some(ref c) = ov.titlebar_text
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_text_color = parsed;
            }
            if let Some(ref c) = ov.frame_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.frame_color = parsed;
            }
            if let Some(ref c) = ov.content_bg
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.content_bg_color = parsed;
            }
            if let Some(ref c) = ov.btn_close
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_close_color = parsed;
            }
            if let Some(ref c) = ov.btn_minimize
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_minimize_color = parsed;
            }
            if let Some(ref c) = ov.btn_maximize
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_maximize_color = parsed;
            }
            if let Some(s) = ov.button_size {
                theme.button_size = s;
            }
            if let Some(s) = ov.resize_handle_size {
                theme.resize_handle_size = s;
            }
            if let Some(s) = ov.titlebar_font_size {
                theme.titlebar_font_size = s;
            }
            // Extended visual properties.
            if let Some(r) = ov.titlebar_radius {
                theme.titlebar_radius = r;
            }
            if let Some(g) = ov.titlebar_gradient {
                theme.titlebar_gradient = g;
            }
            if let Some(ref c) = ov.titlebar_gradient_top
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_gradient_top = Some(parsed);
            }
            if let Some(ref c) = ov.titlebar_gradient_bottom
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_gradient_bottom = Some(parsed);
            }
            if let Some(ref c) = ov.titlebar_inactive_gradient_top
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_inactive_gradient_top = Some(parsed);
            }
            if let Some(ref c) = ov.titlebar_inactive_gradient_bottom
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.titlebar_inactive_gradient_bottom = Some(parsed);
            }
            if let Some(s) = ov.frame_shadow_level {
                theme.frame_shadow_level = s;
            }
            if let Some(r) = ov.frame_border_radius {
                theme.frame_border_radius = r;
            }
            if let Some(r) = ov.button_radius {
                theme.button_radius = r;
            }
            // Tier 1
            if let Some(ref s) = ov.button_side {
                theme.button_side = s.clone();
            }
            if let Some(ref s) = ov.glyph_close {
                theme.glyph_close = s.clone();
            }
            if let Some(ref s) = ov.glyph_minimize {
                theme.glyph_minimize = s.clone();
            }
            if let Some(ref s) = ov.glyph_maximize {
                theme.glyph_maximize = s.clone();
            }
            if let Some(ref s) = ov.title_align {
                theme.title_align = s.clone();
            }
            // Tier 2
            if let Some(v) = ov.separator_enabled {
                theme.separator_enabled = v;
            }
            if let Some(ref c) = ov.separator_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.separator_color = parsed;
            }
            if let Some(ref c) = ov.glyph_close_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.glyph_close_color = parsed;
            }
            if let Some(ref c) = ov.glyph_minimize_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.glyph_minimize_color = parsed;
            }
            if let Some(ref c) = ov.glyph_maximize_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.glyph_maximize_color = parsed;
            }
            if let Some(s) = ov.button_spacing {
                theme.button_spacing = s;
            }
            // Tier 3
            if let Some(ref c) = ov.btn_close_hover
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_close_hover = parsed;
            }
            if let Some(ref c) = ov.btn_minimize_hover
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_minimize_hover = parsed;
            }
            if let Some(ref c) = ov.btn_maximize_hover
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.btn_maximize_hover = parsed;
            }
            if let Some(v) = ov.title_text_shadow {
                theme.title_text_shadow = v;
            }
            if let Some(ref c) = ov.title_text_shadow_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.title_text_shadow_color = parsed;
            }
            if let Some(w) = ov.content_stroke_width {
                theme.content_stroke_width = w;
            }
            if let Some(ref c) = ov.content_stroke_color
                && let Some(parsed) = parse_hex_color(c)
            {
                theme.content_stroke_color = parsed;
            }
            if let Some(v) = ov.maximize_top_inset {
                theme.maximize_top_inset = v;
            }
            if let Some(v) = ov.maximize_bottom_inset {
                theme.maximize_bottom_inset = v;
            }
        }
        // Default glyph colors to titlebar_text_color if not explicitly set.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_close_color.as_ref())
            .is_none()
        {
            theme.glyph_close_color = theme.titlebar_text_color;
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_minimize_color.as_ref())
            .is_none()
        {
            theme.glyph_minimize_color = theme.titlebar_text_color;
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.glyph_maximize_color.as_ref())
            .is_none()
        {
            theme.glyph_maximize_color = theme.titlebar_text_color;
        }
        // Default hover colors to lighten(btn_color, 0.15) if not explicitly set.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_close_hover.as_ref())
            .is_none()
        {
            theme.btn_close_hover = lighten(theme.btn_close_color, 0.15);
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_minimize_hover.as_ref())
            .is_none()
        {
            theme.btn_minimize_hover = lighten(theme.btn_minimize_color, 0.15);
        }
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.btn_maximize_hover.as_ref())
            .is_none()
        {
            theme.btn_maximize_hover = lighten(theme.btn_maximize_color, 0.15);
        }
        theme
    }
}

/// Parse "#RRGGBB" or "#RRGGBBAA" into a `Color`.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::rgb(r, g, b))
    } else if s.len() == 8 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        let a = u8::from_str_radix(&s[6..8], 16).ok()?;
        Some(Color::rgba(r, g, b, a))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_parses() {
        let theme = SkinTheme::default();
        assert_ne!(theme.background_color(), Color::WHITE);
        assert_eq!(theme.prompt_color(), Color::rgb(0, 255, 0));
    }

    #[test]
    fn parse_hex_colors() {
        assert_eq!(parse_hex_color("#FF0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            parse_hex_color("#00FF0080"),
            Some(Color::rgba(0, 255, 0, 128))
        );
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color("#GG0000"), None);
    }

    #[test]
    fn deserialize_from_toml() {
        let toml = r##"
background = "#000000"
primary = "#00FF00"
text = "#00FF00"
prompt = "#00FF00"
output = "#00CC00"
error = "#FF0000"
"##;
        let theme: SkinTheme = toml::from_str(toml).unwrap();
        assert_eq!(theme.background_color(), Color::rgb(0, 0, 0));
        assert_eq!(theme.text_color(), Color::rgb(0, 255, 0));
    }

    #[test]
    fn wm_theme_overrides() {
        let toml = r##"
[wm_theme]
titlebar_height = 32
titlebar_active = "#0000FF"
button_size = 20
"##;
        let theme: SkinTheme = toml::from_str(toml).unwrap();
        let wm = theme.build_wm_theme();
        assert_eq!(wm.titlebar_height, 32);
        assert_eq!(wm.titlebar_active_color, Color::rgb(0, 0, 255));
        assert_eq!(wm.button_size, 20);
        // Non-overridden values remain default.
        assert_eq!(wm.border_width, 1);
    }

    #[test]
    fn no_wm_overrides_returns_default() {
        let theme = SkinTheme::default();
        let wm = theme.build_wm_theme();
        assert_eq!(wm.titlebar_height, 24);
    }

    #[test]
    fn to_ui_theme_derives_from_base_colors() {
        let skin = SkinTheme::default();
        let ui = skin.to_ui_theme();
        // Background should match.
        assert_eq!(ui.background, skin.background_color());
        // Accent should match primary.
        assert_eq!(ui.accent, skin.primary_color());
        // Error should match.
        assert_eq!(ui.error, skin.error_color());
        // Text primary should match text.
        assert_eq!(ui.text_primary, skin.text_color());
        // Border radii should be reasonable.
        assert!(ui.border_radius_md > 0);
    }

    #[test]
    fn to_ui_theme_respects_extended_fields() {
        let toml = r##"
background = "#000000"
primary = "#FF0000"
surface = "#111111"
accent_hover = "#FF5555"
border_radius = 8
shadow_intensity = 2
gradient_enabled = true
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let ui = skin.to_ui_theme();
        assert_eq!(ui.surface, Color::rgb(0x11, 0x11, 0x11));
        assert_eq!(ui.accent_hover, Color::rgb(0xFF, 0x55, 0x55));
        assert_eq!(ui.border_radius_md, 8);
    }

    #[test]
    fn to_ui_theme_roundtrip() {
        let skin = SkinTheme::default();
        let ui = skin.to_ui_theme();
        assert_eq!(ui.accent, skin.primary_color());
    }
}
