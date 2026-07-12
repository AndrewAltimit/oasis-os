//! Skin theme -- color scheme and visual properties.
//!
//! The theme defines the color palette and optional WM visual overrides
//! for a skin. Loaded from `theme.toml`.

mod conversion;
pub mod overrides;

use serde::Deserialize;

use oasis_types::backend::Color;

pub use conversion::{ContrastWarning, contrast_ratio};
pub use overrides::resolve_easing;
pub use overrides::{
    AnimationPreset, AppOverrides, BackgroundLayerConfig, BackgroundPerformanceConfig,
    BarOverrides, BrowserOverrides, CursorConfig, GeometryOverrides, GradientPreset, IconOverrides,
    LayerAnimationConfig, LayerPositionConfig, NinePatchDef, OskOverrides, ScrollbarOverrides,
    StartMenuOverrides, TransitionOverrides, WallpaperConfig, WmThemeOverrides,
};

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
    /// Accent color override (default: same as primary). Drives the UI
    /// accent family (hover/pressed/subtle) when set, letting a skin use
    /// a highlight color distinct from its primary.
    #[serde(default)]
    pub accent: Option<String>,
    /// Accent hover color override (default: derived from the accent).
    #[serde(default)]
    pub accent_hover: Option<String>,
    /// Success/positive color override (toasts, status badges).
    #[serde(default)]
    pub success: Option<String>,
    /// Warning/caution color override (toasts, status badges).
    #[serde(default)]
    pub warning: Option<String>,
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

    /// Per-element color overrides for app screens.
    #[serde(default)]
    pub app_overrides: Option<AppOverrides>,

    /// Per-element color overrides for the on-screen keyboard.
    #[serde(default)]
    pub osk_overrides: Option<OskOverrides>,

    /// Per-element color overrides for the start menu popup.
    #[serde(default)]
    pub start_menu_overrides: Option<StartMenuOverrides>,

    /// Wallpaper generation configuration.
    #[serde(default)]
    pub wallpaper: Option<WallpaperConfig>,

    /// Software mouse cursor theming (texture + hotspot).
    #[serde(default)]
    pub cursor: Option<CursorConfig>,

    /// Geometry overrides (bar heights, icon sizes, font sizes).
    #[serde(default)]
    pub geometry: Option<GeometryOverrides>,

    /// Transition effect overrides.
    #[serde(default)]
    pub transition: Option<TransitionOverrides>,

    /// Per-element overrides for scrollbar appearance.
    #[serde(default)]
    pub scrollbar_overrides: Option<ScrollbarOverrides>,

    /// Background decoration layers for the dashboard.
    ///
    /// ```toml
    /// [[background_layers]]
    /// kind = "grid"
    /// spacing = 30
    /// color = "#FFFFFF12"
    /// ```
    #[serde(default)]
    pub background_layers: Option<Vec<BackgroundLayerConfig>>,

    /// Chrome decoration layers rendered in the overlay pass — on top of
    /// bars, tabs, and windows — for procedurally shaped chrome accents
    /// (notch lines, corner brackets, HUD reticles) without shipping art.
    /// Same schema as `background_layers`; `"image"` and `"shader"` kinds
    /// are not supported here.
    ///
    /// ```toml
    /// [[chrome_layers]]
    /// kind = "crosshair"
    /// size = 12
    /// color = "#FFFFFF30"
    /// position = { anchor = "top_right", offset_x = -0.05, offset_y = 0.04 }
    /// ```
    #[serde(default)]
    pub chrome_layers: Option<Vec<BackgroundLayerConfig>>,

    /// Performance settings for background layers.
    #[serde(default)]
    pub background_performance: Option<BackgroundPerformanceConfig>,

    /// Per-app color overrides keyed by app name.
    ///
    /// Each entry maps color keys to hex color strings.
    /// Apps query `ActiveTheme::app_color("app_name", "key")` with
    /// fallback to their default palette.
    ///
    /// ```toml
    /// [app_themes.tv_guide]
    /// bg = "#0A1628"
    /// grid_line = "#1A3A5C"
    /// ```
    #[serde(default)]
    pub app_themes:
        Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,

    /// Named gradient presets reusable across components.
    ///
    /// ```toml
    /// [gradients.primary]
    /// from = "#0066FF"
    /// to = "#0044AA"
    /// ```
    #[serde(default)]
    pub gradients: Option<std::collections::HashMap<String, GradientPreset>>,

    /// Named animation timing presets.
    ///
    /// ```toml
    /// [animations.button_press]
    /// duration_ms = 100
    /// easing = "ease_out_quad"
    /// ```
    #[serde(default)]
    pub animations: Option<std::collections::HashMap<String, AnimationPreset>>,

    /// Per-widget state color overrides.
    ///
    /// ```toml
    /// [widget_states.button]
    /// normal_bg = "#505050"
    /// hover_bg = "#656565"
    /// pressed_bg = "#353535"
    /// disabled_bg = "#3A3A3A"
    /// disabled_text = "#555555"
    /// ```
    #[serde(default)]
    pub widget_states:
        Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,
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
            accent: None,
            accent_hover: None,
            success: None,
            warning: None,
            border_radius: None,
            shadow_intensity: None,
            gradient_enabled: None,
            wm_theme: None,
            bar_overrides: None,
            icon_overrides: None,
            browser_overrides: None,
            app_overrides: None,
            osk_overrides: None,
            start_menu_overrides: None,
            wallpaper: None,
            cursor: None,
            geometry: None,
            transition: None,
            scrollbar_overrides: None,
            background_layers: None,
            chrome_layers: None,
            background_performance: None,
            app_themes: None,
            gradients: None,
            animations: None,
            widget_states: None,
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
}

/// Parse "#RRGGBB" or "#RRGGBBAA" into a `Color`.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    oasis_types::color::parse_hex_color(s)
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
    fn wm_theme_nine_patch_overrides() {
        let toml = r##"
[wm_theme]
titlebar_nine_patch = { image = "assets/tb.png", insets = [4, 4, 4, 4] }
frame_nine_patch = { image = "assets/frame.png", insets = [6, 6, 6, 6] }
"##;
        let theme: SkinTheme = toml::from_str(toml).unwrap();
        let wm = theme.build_wm_theme();
        assert_eq!(
            wm.titlebar_nine_patch,
            Some(("assets/tb.png".to_string(), [4, 4, 4, 4]))
        );
        assert_eq!(
            wm.frame_nine_patch,
            Some(("assets/frame.png".to_string(), [6, 6, 6, 6]))
        );
        // Runtime patches are resolved by the shell, never at parse time.
        assert!(wm.titlebar_patch.is_none());
        assert!(wm.frame_patch.is_none());
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

    #[test]
    fn to_ui_theme_accent_override() {
        let toml = r##"
primary = "#FF71CE"
accent = "#01CDFE"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let ui = skin.to_ui_theme();
        // Accent family derives from the explicit accent, not primary.
        assert_eq!(ui.accent, Color::rgb(0x01, 0xCD, 0xFE));
        assert_ne!(ui.accent, skin.primary_color());
        assert_eq!(ui.accent_subtle.r, 0x01);
        assert_eq!(ui.info, ui.accent);
    }

    #[test]
    fn build_wm_theme_titlebar_text_active_inactive() {
        let toml = r##"
[wm_theme]
titlebar_text_active = "#FFFFFF"
titlebar_text_inactive = "#888888"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let wm = skin.build_wm_theme();
        assert_eq!(wm.titlebar_text_color, Color::rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(
            wm.titlebar_text_inactive_color,
            Color::rgb(0x88, 0x88, 0x88)
        );
    }

    #[test]
    fn build_wm_theme_inactive_text_defaults_to_active() {
        let toml = r##"
[wm_theme]
titlebar_text = "#FF00FF"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let wm = skin.build_wm_theme();
        assert_eq!(wm.titlebar_text_color, Color::rgb(0xFF, 0x00, 0xFF));
        assert_eq!(wm.titlebar_text_inactive_color, wm.titlebar_text_color);
    }

    // -- resolve_easing tests --

    #[test]
    fn resolve_easing_known_names() {
        let linear = resolve_easing("linear");
        assert!((linear(0.5) - 0.5).abs() < f32::EPSILON);

        let ease_out = resolve_easing("ease_out_quad");
        assert!(ease_out(0.5) > 0.5);

        let ease_in = resolve_easing("ease_in_quad");
        assert!(ease_in(0.5) < 0.5);
    }

    #[test]
    fn resolve_easing_unknown_returns_linear() {
        let f = resolve_easing("unknown_easing");
        assert!((f(0.5) - 0.5).abs() < f32::EPSILON);
    }

    // -- GradientPreset / AnimationPreset deserialization tests --

    #[test]
    fn gradient_preset_deserialize() {
        let toml = r##"
[gradients.primary]
from = "#0066FF"
to = "#0044AA"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let grads = skin.gradients.unwrap();
        let g = &grads["primary"];
        assert_eq!(g.from, "#0066FF");
        assert_eq!(g.to, "#0044AA");
    }

    #[test]
    fn animation_preset_deserialize() {
        let toml = r##"
[animations.button_press]
duration_ms = 100
easing = "ease_out_quad"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let anims = skin.animations.unwrap();
        let a = &anims["button_press"];
        assert_eq!(a.duration_ms, 100);
        assert_eq!(a.easing, "ease_out_quad");
    }

    #[test]
    fn animation_preset_default_easing() {
        let toml = r##"
[animations.fast]
duration_ms = 50
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let anims = skin.animations.unwrap();
        assert_eq!(anims["fast"].easing, "linear");
    }

    // -- widget_states deserialization tests --

    #[test]
    fn widget_states_deserialize() {
        let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"
pressed_bg = "#353535"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let states = skin.widget_states.unwrap();
        let button = &states["button"];
        assert_eq!(button["hover_bg"], "#656565");
    }

    #[test]
    fn widget_states_override_ui_theme() {
        let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"

[widget_states.toggle]
track_on = "#FF8C1E"
thumb = "#101010"

[widget_states.input]
focus_border = "#00FFAA"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let theme = skin.to_ui_theme();
        assert_eq!(theme.button_bg, Color::rgb(0x50, 0x50, 0x50));
        assert_eq!(theme.button_bg_hover, Color::rgb(0x65, 0x65, 0x65));
        assert_eq!(theme.toggle_track_on, Color::rgb(0xFF, 0x8C, 0x1E));
        assert_eq!(theme.toggle_thumb, Color::rgb(0x10, 0x10, 0x10));
        assert_eq!(theme.input_border_focus, Color::rgb(0x00, 0xFF, 0xAA));
        // Slots not overridden keep their derived values.
        let plain = SkinTheme::default().to_ui_theme();
        assert_eq!(theme.button_bg_pressed, plain.button_bg_pressed);
        assert_eq!(theme.toggle_track_off, plain.toggle_track_off);
    }

    #[test]
    fn ui_theme_toggle_defaults_derive_from_accent() {
        let skin = SkinTheme::default();
        let theme = skin.to_ui_theme();
        // Without widget_states, the toggle derives from the accent family
        // exactly as the widget hardcoded before.
        assert_eq!(theme.toggle_track_off, Color::rgba(255, 255, 255, 10));
        assert_eq!(theme.toggle_track_on, theme.accent);
        assert_eq!(theme.toggle_thumb, theme.text_on_accent);
    }

    #[test]
    fn success_warning_override_toast_and_ui() {
        let toml = r##"
success = "#00CC66"
warning = "#FFAA00"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let theme = skin.to_ui_theme();
        assert_eq!(theme.success, Color::rgb(0x00, 0xCC, 0x66));
        assert_eq!(theme.warning, Color::rgb(0xFF, 0xAA, 0x00));
        // Defaults are the historical literals.
        let plain = SkinTheme::default().to_ui_theme();
        assert_eq!(plain.success, Color::rgb(80, 200, 120));
        assert_eq!(plain.warning, Color::rgb(255, 180, 50));
    }

    // -- Phase 12: expanded SkinTheme tests --

    #[test]
    fn default_theme_background_is_dark() {
        let skin = SkinTheme::default();
        let bg = skin.background_color();
        assert!(bg.r < 50 && bg.g < 50 && bg.b < 50);
    }

    #[test]
    fn default_theme_text_is_green() {
        let skin = SkinTheme::default();
        let text = skin.text_color();
        assert_eq!(text.g, 255);
    }

    #[test]
    fn parse_hex_color_three_byte() {
        assert_eq!(
            parse_hex_color("#AABBCC"),
            Some(Color::rgb(0xAA, 0xBB, 0xCC))
        );
    }

    #[test]
    fn parse_hex_color_four_byte() {
        assert_eq!(
            parse_hex_color("#AABBCC80"),
            Some(Color::rgba(0xAA, 0xBB, 0xCC, 0x80))
        );
    }

    #[test]
    fn parse_hex_color_lowercase() {
        assert_eq!(
            parse_hex_color("#aabbcc"),
            Some(Color::rgb(0xAA, 0xBB, 0xCC))
        );
    }

    #[test]
    fn parse_hex_color_missing_hash() {
        assert_eq!(parse_hex_color("AABBCC"), None);
    }

    #[test]
    fn to_ui_theme_surface_derived_from_background() {
        let skin = SkinTheme::default();
        let ui = skin.to_ui_theme();
        // Surface should be lighter than background (derived via lighten).
        let bg = skin.background_color();
        assert!(ui.surface.r >= bg.r || ui.surface.g >= bg.g || ui.surface.b >= bg.b);
    }

    #[test]
    fn to_ui_theme_border_derived() {
        let skin = SkinTheme::default();
        let ui = skin.to_ui_theme();
        // Border strong should match primary.
        assert_eq!(ui.border_strong, skin.primary_color());
    }

    #[test]
    fn to_ui_theme_font_sizes_are_reasonable() {
        let skin = SkinTheme::default();
        let ui = skin.to_ui_theme();
        assert!(ui.font_size_sm > 0);
        assert!(ui.font_size_md > 0);
        assert!(ui.font_size_lg > ui.font_size_sm);
    }

    #[test]
    fn contrast_ratio_black_white() {
        let ratio = contrast_ratio(Color::BLACK, Color::WHITE);
        assert!((ratio - 21.0).abs() < 0.1);
    }

    #[test]
    fn contrast_ratio_same_color() {
        let ratio = contrast_ratio(Color::rgb(128, 128, 128), Color::rgb(128, 128, 128));
        assert!((ratio - 1.0).abs() < 0.01);
    }

    #[test]
    fn default_theme_passes_contrast() {
        let skin = SkinTheme::default();
        let warnings = skin.validate_contrast();
        // Default theme (light text on dark bg) should pass.
        assert!(
            warnings.is_empty(),
            "default theme has contrast warnings: {warnings:?}"
        );
    }

    #[test]
    fn low_contrast_theme_warns() {
        let toml = r##"
background = "#808080"
text = "#909090"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let warnings = skin.validate_contrast();
        assert!(!warnings.is_empty(), "should warn on low contrast");
        assert!(warnings.iter().any(|w| w.pair.contains("text")));
    }

    #[test]
    fn bar_overrides_deserialize() {
        let toml = r##"
[bar_overrides]
statusbar_bg = "#112233"
clock_color = "#FFFFFF"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let bars = skin.bar_overrides.unwrap();
        assert_eq!(bars.statusbar_bg.as_deref(), Some("#112233"));
        assert_eq!(bars.clock_color.as_deref(), Some("#FFFFFF"));
    }

    #[test]
    fn icon_overrides_deserialize() {
        let toml = r##"
[icon_overrides]
body_color = "#FF0000"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let icons = skin.icon_overrides.unwrap();
        assert_eq!(icons.body_color.as_deref(), Some("#FF0000"));
    }

    #[test]
    fn default_no_overrides() {
        let skin = SkinTheme::default();
        assert!(skin.surface.is_none());
        assert!(skin.accent_hover.is_none());
        assert!(skin.border_radius.is_none());
        assert!(skin.shadow_intensity.is_none());
        assert!(skin.wm_theme.is_none());
        assert!(skin.bar_overrides.is_none());
        assert!(skin.icon_overrides.is_none());
    }
}
