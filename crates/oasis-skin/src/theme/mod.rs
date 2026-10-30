//! Skin theme -- color scheme and visual properties.
//!
//! The theme defines the color palette and optional WM visual overrides
//! for a skin. Loaded from `theme.toml`.

mod conversion;
pub mod overrides;
mod presets;

use serde::Deserialize;

use oasis_types::backend::Color;

pub use conversion::{ContrastWarning, composite_over, contrast_ratio};
pub(crate) use conversion::{bar_backdrop, readable_on};
pub use overrides::{
    AnimationPreset, AppOverrides, BackgroundLayerConfig, BackgroundPerformanceConfig,
    BarOverrides, BootOverrides, BrowserOverrides, CursorConfig, Density, ElevationLayerConfig,
    ElevationOverrides, GeometryOverrides, GradientPreset, IconOverrides, LayerAnimationConfig,
    LayerPositionConfig, NinePatchDef, OskOverrides, PaletteOverrides, ScrollbarOverrides,
    StartMenuOverrides, TransitionOverrides, TypographyOverrides, WallpaperConfig,
    WmThemeOverrides,
};
pub use overrides::{EASING_NAMES, resolve_easing};

/// Color scheme for a skin.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// Accent color override (default: same as primary). Drives the UI
    /// accent family (hover/pressed/subtle) when set, letting a skin use
    /// a highlight color distinct from its primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Accent hover color override (default: derived from the accent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_hover: Option<String>,
    /// Success/positive color override (toasts, status badges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<String>,
    /// Warning/caution color override (toasts, status badges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Default border radius for UI elements (pixels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u16>,
    /// Shadow intensity (0 = none, 1 = subtle, 2 = medium, 3 = heavy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_intensity: Option<u8>,
    /// Whether gradient fills are enabled for this skin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_enabled: Option<bool>,

    /// Global accessibility text-size multiplier applied to the resolved UI
    /// font-size ladder and text metrics. `1.0` = no change (pixel-identical);
    /// values above `1.0` enlarge all text. Clamped to `0.5..=3.0` when applied.
    ///
    /// This is orthogonal to `geometry.font_scale`: it scales whatever the
    /// resolved font sizes are, so the two compose multiplicatively.
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,

    /// UI density preset applied to spacing tokens only: `"compact"`,
    /// `"comfortable"` (default), or `"spacious"`. Unset / unknown values are
    /// treated as `comfortable`, which is an exact pixel-identity no-op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<String>,

    /// Semantic elevation/shadow ladder overrides (`[elevation]` table).
    ///
    /// Levels left unset fall back to the built-in shadow ladder, so an empty
    /// table renders shadows byte-for-byte identically to today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation: Option<ElevationOverrides>,

    /// Whether the WM is visually themed by this skin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wm_theme: Option<WmThemeOverrides>,

    /// Per-element color overrides for status/bottom bars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_overrides: Option<BarOverrides>,

    /// Per-element color overrides for dashboard icons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_overrides: Option<IconOverrides>,

    /// Per-element color overrides for browser chrome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_overrides: Option<BrowserOverrides>,

    /// Per-element color overrides for app screens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_overrides: Option<AppOverrides>,

    /// Per-element color overrides for the on-screen keyboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osk_overrides: Option<OskOverrides>,

    /// Per-element color overrides for the start menu popup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_menu_overrides: Option<StartMenuOverrides>,

    /// Wallpaper generation configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<WallpaperConfig>,

    /// Software mouse cursor theming (texture + hotspot + procedural
    /// arrow fill/outline colors).
    ///
    /// ```toml
    /// [cursor]
    /// fill = "#FFFFFF"
    /// outline = "#000000"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CursorConfig>,

    /// Geometry overrides (bar heights, icon sizes, font sizes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometryOverrides>,

    /// Typography scale for the widget toolkit (font-size ladder + spacing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography: Option<TypographyOverrides>,

    /// Transition effect overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionOverrides>,

    /// Per-element overrides for scrollbar appearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollbar_overrides: Option<ScrollbarOverrides>,

    /// 16-slot ANSI terminal palette overrides.
    ///
    /// ```toml
    /// [palette]
    /// red = "#FF5555"
    /// bright_green = "#69FF94"
    /// ```
    ///
    /// Unset slots derive from the skin's base colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<PaletteOverrides>,

    /// Boot splash color overrides.
    ///
    /// ```toml
    /// [boot]
    /// banner_border = "#AA88FF"
    /// sky_stops = ["#02001A", "#050044", "#150088", "#5500CC", "#AA55FF", "#FFFFFF"]
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot: Option<BootOverrides>,

    /// Background decoration layers for the dashboard.
    ///
    /// ```toml
    /// [[background_layers]]
    /// kind = "grid"
    /// spacing = 30
    /// color = "#FFFFFF12"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_layers: Option<Vec<BackgroundLayerConfig>>,

    /// Performance settings for background layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_themes:
        Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,

    /// Named gradient presets reusable across components.
    ///
    /// ```toml
    /// [gradients.primary]
    /// from = "#0066FF"
    /// to = "#0044AA"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradients: Option<std::collections::HashMap<String, GradientPreset>>,

    /// Named animation timing presets.
    ///
    /// ```toml
    /// [animations.button_press]
    /// duration_ms = 100
    /// easing = "ease_out_quad"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_states:
        Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,

    /// UI sound theme: WAV assets played on interface events.
    ///
    /// ```toml
    /// [sounds]
    /// click = "assets/click.wav"
    /// nav = "assets/nav.wav"
    /// volume = 0.8
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sounds: Option<SkinSounds>,
}

/// UI sound theme (`[sounds]` in theme.toml): each field references a WAV
/// asset by skin-relative path (e.g. `"assets/click.wav"`). Omitted events
/// stay silent; a skin without a `[sounds]` table is fully silent.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct SkinSounds {
    /// Button / interactive element click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click: Option<String>,
    /// App launch / window open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    /// App / window close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<String>,
    /// Error feedback (error toasts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Non-error toast notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toast: Option<String>,
    /// Cursor / d-pad navigation between icons (rate-limited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav: Option<String>,
    /// Master volume for UI sounds, 0.0–1.0 (default 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
}

impl SkinSounds {
    /// Iterate the configured `(event, asset_path)` pairs.
    pub fn entries(&self) -> [(&'static str, Option<&str>); 6] {
        [
            ("click", self.click.as_deref()),
            ("open", self.open.as_deref()),
            ("close", self.close.as_deref()),
            ("error", self.error.as_deref()),
            ("toast", self.toast.as_deref()),
            ("nav", self.nav.as_deref()),
        ]
    }

    /// The asset path configured for a named event, if any.
    pub fn path_for(&self, event: &str) -> Option<&str> {
        self.entries()
            .into_iter()
            .find(|(name, _)| *name == event)
            .and_then(|(_, path)| path)
    }

    /// Master volume clamped to 0.0–1.0 (default 1.0).
    pub fn effective_volume(&self) -> f32 {
        self.volume.unwrap_or(1.0).clamp(0.0, 1.0)
    }
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
fn default_text_scale() -> f32 {
    1.0
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
            text_scale: default_text_scale(),
            density: None,
            elevation: None,
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
            typography: None,
            transition: None,
            scrollbar_overrides: None,
            palette: None,
            boot: None,
            background_layers: None,
            chrome_layers: None,
            background_performance: None,
            app_themes: None,
            gradients: None,
            animations: None,
            widget_states: None,
            sounds: None,
        }
    }
}

impl SkinTheme {
    /// Parse a theme from a TOML string (the body of a `theme.toml`).
    pub fn from_toml_str(toml_str: &str) -> oasis_types::error::Result<Self> {
        toml::from_str(toml_str)
            .map_err(|e| oasis_types::error::OasisError::Config(format!("theme: {e}").into()))
    }

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

    /// Parse the status_bar color string to a `Color`.
    pub fn status_bar_color(&self) -> Color {
        parse_hex_color(&self.status_bar).unwrap_or(Color::rgb(40, 60, 90))
    }

    /// The resolved UI density preset (spacing multiplier). Defaults to
    /// `comfortable`, an exact pixel-identity no-op.
    pub fn density_preset(&self) -> Density {
        Density::parse(self.density.as_deref())
    }

    /// Clamped global accessibility text-size multiplier. `1.0` = no change.
    ///
    /// The raw `text_scale` field is clamped to `0.5..=3.0` so a malformed
    /// skin cannot shrink text to nothing or blow it up unboundedly.
    pub fn text_scale_factor(&self) -> f32 {
        self.text_scale.clamp(0.5, 3.0)
    }

    /// Scale a base font size by the accessibility `text_scale`, rounded and
    /// clamped to at least 1px. With `text_scale == 1.0` the input is returned
    /// unchanged (pixel-identical to pre-`text_scale` behavior).
    pub fn scale_font(&self, base: u16) -> u16 {
        (f32::from(base) * self.text_scale_factor())
            .round()
            .max(1.0) as u16
    }

    /// Build the semantic elevation ladder from the `[elevation]` table, or a
    /// default ladder (built-in shadows) when the table is absent.
    pub fn elevation_ladder(&self) -> oasis_types::shadow::ElevationLadder {
        self.elevation
            .as_ref()
            .map(ElevationOverrides::to_ladder)
            .unwrap_or_default()
    }
}

/// Parse "#RRGGBB" or "#RRGGBBAA" into a `Color`.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    oasis_types::color::parse_hex_color(s)
}

#[cfg(test)]
mod tests;
