//! Conversion from `SkinTheme` to `ui::Theme` and `WmTheme`.

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};
use oasis_types::shadow::Shadow;
use oasis_ui::theme::Theme;
use oasis_wm::WmTheme;

use super::overrides::WmThemeOverrides;
use super::{SkinTheme, parse_hex_color};

impl SkinTheme {
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

        // Accent variants: explicit `accent` override, else primary.
        let accent = self
            .accent
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or(primary);
        let accent_hover = self
            .accent_hover
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| lighten(accent, 0.15));
        let accent_pressed = darken(accent, 0.85);
        let accent_subtle = with_alpha(accent, 30);

        // Border radius and shadow from extended fields.
        let radius = self.border_radius.unwrap_or(4);
        let shadow_level = self.shadow_intensity.unwrap_or(1);

        let success = self
            .success
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| Color::rgb(80, 200, 120));
        let warning = self
            .warning
            .as_ref()
            .and_then(|s| parse_hex_color(s))
            .unwrap_or_else(|| Color::rgb(255, 180, 50));

        let mut theme = Theme {
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

            success,
            warning,
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
            toggle_track_off: Color::rgba(255, 255, 255, 10),
            toggle_track_on: accent,
            toggle_thumb: text,
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

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: oasis_types::text_direction::TextDirection::Ltr,
        };

        if let Some(ref typo) = self.typography {
            apply_typography(&mut theme, typo);
        }
        if let Some(ref states) = self.widget_states {
            apply_widget_states(&mut theme, states);
        }
        theme
    }

    /// Build a `WmTheme` from the defaults plus any overrides.
    pub fn build_wm_theme(&self) -> WmTheme {
        let mut theme = WmTheme::default();
        if let Some(ref ov) = self.wm_theme {
            apply_wm_overrides(&mut theme, ov);
        }
        // Default inactive title text to the active text color when unset,
        // so a custom `titlebar_text` carries over to unfocused windows.
        if self
            .wm_theme
            .as_ref()
            .and_then(|o| o.titlebar_text_inactive.as_ref())
            .is_none()
        {
            theme.titlebar_text_inactive_color = theme.titlebar_text_color;
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

/// Apply the `[typography]` scale onto a derived `Theme`.
///
/// Unset fields keep the derivation's defaults, so skins written before this
/// existed produce a byte-identical `Theme`.
fn apply_typography(theme: &mut Theme, typo: &super::TypographyOverrides) {
    for (slot, value) in [
        (&mut theme.font_size_xs, typo.font_size_xs),
        (&mut theme.font_size_sm, typo.font_size_sm),
        (&mut theme.font_size_md, typo.font_size_md),
        (&mut theme.font_size_lg, typo.font_size_lg),
        (&mut theme.font_size_xl, typo.font_size_xl),
        (&mut theme.font_size_xxl, typo.font_size_xxl),
        (&mut theme.spacing_xs, typo.spacing_xs),
        (&mut theme.spacing_sm, typo.spacing_sm),
        (&mut theme.spacing_md, typo.spacing_md),
        (&mut theme.spacing_lg, typo.spacing_lg),
        (&mut theme.spacing_xl, typo.spacing_xl),
    ] {
        if let Some(v) = value {
            *slot = v;
        }
    }
}

/// Apply `[widget_states.*]` color overrides onto a derived `Theme`.
///
/// Recognized slots (unknown widgets/keys are ignored; `skin lint` warns):
/// - `button`: `normal_bg`, `hover_bg`, `pressed_bg`, `disabled_bg`,
///   `disabled_text`
/// - `input`: `bg`, `border`, `focus_border`
/// - `toggle`: `track_off`, `track_on`, `thumb`
/// - `scrollbar`: `track`, `thumb`, `thumb_hover`
fn apply_widget_states(
    theme: &mut Theme,
    states: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) {
    let get = |widget: &str, key: &str| {
        states
            .get(widget)
            .and_then(|m| m.get(key))
            .and_then(|hex| parse_hex_color(hex))
    };
    for (slot, key, widget_key) in [
        (&mut theme.button_bg, "button", "normal_bg"),
        (&mut theme.button_bg_hover, "button", "hover_bg"),
        (&mut theme.button_bg_pressed, "button", "pressed_bg"),
        (&mut theme.button_bg_disabled, "button", "disabled_bg"),
        (&mut theme.text_disabled, "button", "disabled_text"),
        (&mut theme.input_bg, "input", "bg"),
        (&mut theme.input_border, "input", "border"),
        (&mut theme.input_border_focus, "input", "focus_border"),
        (&mut theme.toggle_track_off, "toggle", "track_off"),
        (&mut theme.toggle_track_on, "toggle", "track_on"),
        (&mut theme.toggle_thumb, "toggle", "thumb"),
        (&mut theme.scrollbar_track, "scrollbar", "track"),
        (&mut theme.scrollbar_thumb, "scrollbar", "thumb"),
        (&mut theme.scrollbar_thumb_hover, "scrollbar", "thumb_hover"),
    ] {
        if let Some(c) = get(key, widget_key) {
            *slot = c;
        }
    }
}

/// Apply all WM theme overrides to a `WmTheme`.
fn apply_wm_overrides(theme: &mut WmTheme, ov: &WmThemeOverrides) {
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
    // `titlebar_text_active` is a synonym that takes precedence.
    if let Some(ref c) = ov.titlebar_text_active
        && let Some(parsed) = parse_hex_color(c)
    {
        theme.titlebar_text_color = parsed;
    }
    if let Some(ref c) = ov.titlebar_text_inactive
        && let Some(parsed) = parse_hex_color(c)
    {
        theme.titlebar_text_inactive_color = parsed;
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
    if let Some(ref c) = ov.modal_overlay_color
        && let Some(parsed) = parse_hex_color(c)
    {
        theme.modal_overlay_color = parsed;
    }
    if let Some(a) = ov.inactive_frame_alpha {
        theme.inactive_frame_alpha = a;
    }
    if let Some(ref np) = ov.titlebar_nine_patch {
        theme.titlebar_nine_patch = Some((np.image.clone(), np.insets));
    }
    if let Some(ref np) = ov.frame_nine_patch {
        theme.frame_nine_patch = Some((np.image.clone(), np.insets));
    }
}

/// Compute the WCAG 2.0 relative luminance of a color.
///
/// See <https://www.w3.org/TR/WCAG20/#relativeluminancedef>.
fn relative_luminance(c: Color) -> f64 {
    fn linearize(val: u8) -> f64 {
        let s = val as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b)
}

/// Compute the WCAG 2.0 contrast ratio between two colors.
///
/// Returns a value between 1.0 (identical) and 21.0 (black vs white).
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// A contrast warning for a text/background color pair.
#[derive(Debug, Clone)]
pub struct ContrastWarning {
    /// Human-readable label for the pair (e.g. "text on background").
    pub pair: String,
    /// The computed contrast ratio.
    pub ratio: f64,
    /// WCAG AA minimum (4.5 for normal text, 3.0 for large text).
    pub required: f64,
}

impl SkinTheme {
    /// Validate key text/background pairs against WCAG AA contrast minimums.
    ///
    /// Returns a list of warnings for pairs that fail the 4.5:1 ratio.
    pub fn validate_contrast(&self) -> Vec<ContrastWarning> {
        let bg = self.background_color();
        let pairs: &[(&str, Color, f64)] = &[
            ("text on background", self.text_color(), 4.5),
            ("dim_text on background", self.dim_text_color(), 3.0),
            ("prompt on background", self.prompt_color(), 3.0),
            ("error on background", self.error_color(), 3.0),
        ];

        let mut warnings = Vec::new();
        for &(label, fg, required) in pairs {
            let ratio = contrast_ratio(fg, bg);
            if ratio < required {
                warnings.push(ContrastWarning {
                    pair: label.to_string(),
                    ratio,
                    required,
                });
            }
        }
        warnings
    }
}
