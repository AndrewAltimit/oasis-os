//! Derived skin variants -- Dark / Light / High-contrast.
//!
//! A variant transforms the 9 base palette colors of an existing skin
//! (flipping lightness for dark<->light, pushing contrast ratios for
//! high-contrast) and then relies on the existing derivation engine
//! (`ActiveTheme::from_skin`) to rebuild every UI color from the new
//! palette.
//!
//! Fine-grained *color* overrides from the source skin are intentionally
//! dropped -- they encode literal hex values tuned for the original palette
//! and would clash with the transformed base colors. Structural overrides
//! (geometry, WM titlebar metrics, icon style / animation settings, start
//! menu layout) are preserved so the variant keeps the skin's shape.

use oasis_types::backend::Color;

use crate::loader::Skin;
use crate::theme::{
    GeometryOverrides, IconOverrides, SkinTheme, StartMenuOverrides, WmThemeOverrides,
};

/// Prefix for skin-swap request names that ask for a derived variant of the
/// *currently active* skin instead of a named skin (e.g. `"@variant:dark"`).
///
/// Used by the `skin variant` terminal command; resolved via
/// [`resolve_skin_request`].
pub const VARIANT_REQUEST_PREFIX: &str = "@variant:";

/// A derived variant of a skin's base palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinVariant {
    /// Dark backgrounds, light text.
    Dark,
    /// Light backgrounds, dark text.
    Light,
    /// Near-black background, near-white text, saturated accents; pushes
    /// text/background contrast to at least WCAG AAA (7:1).
    HighContrast,
}

impl SkinVariant {
    /// All variants, for UI listings.
    pub const ALL: [SkinVariant; 3] = [
        SkinVariant::Dark,
        SkinVariant::Light,
        SkinVariant::HighContrast,
    ];

    /// Parse a variant from a user-facing name.
    ///
    /// Accepts `"dark"`, `"light"`, `"high-contrast"` (also
    /// `"highcontrast"` / `"high_contrast"`), case-insensitively.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "high-contrast" | "highcontrast" | "high_contrast" => Some(Self::HighContrast),
            _ => None,
        }
    }

    /// Canonical user-facing name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::HighContrast => "high-contrast",
        }
    }

    /// Human-readable label for UI listings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::HighContrast => "High Contrast",
        }
    }
}

// ---------------------------------------------------------------------------
// HSL color math
// ---------------------------------------------------------------------------

/// Convert an sRGB color to (hue, saturation, lightness), each in `0.0..=1.0`
/// (hue is a fraction of a full turn).
fn rgb_to_hsl(c: Color) -> (f32, f32, f32) {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// Convert (hue, saturation, lightness) back to an sRGB color (alpha 255).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    let (h, s, l) = (h.rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    if s < f32::EPSILON {
        let v = (l * 255.0).round() as u8;
        return Color::rgb(v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_channel(p, q, h + 1.0 / 3.0);
    let g = hue_to_channel(p, q, h);
    let b = hue_to_channel(p, q, h - 1.0 / 3.0);
    Color::rgb(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

/// Replace a color's lightness, keeping hue and saturation.
fn with_lightness(c: Color, l: f32) -> Color {
    let (h, s, _) = rgb_to_hsl(c);
    hsl_to_rgb(h, s, l)
}

/// Flip a color's lightness (`l' = 1 - l`), keeping hue and saturation.
fn flip_lightness(c: Color) -> Color {
    let (h, s, l) = rgb_to_hsl(c);
    hsl_to_rgb(h, s, 1.0 - l)
}

/// Lightness component of a color.
fn lightness(c: Color) -> f32 {
    rgb_to_hsl(c).2
}

/// Step a foreground color's lightness away from the background until the
/// WCAG contrast ratio reaches `min_ratio` (or the lightness range is
/// exhausted). Hue and saturation are preserved.
fn ensure_contrast(fg: Color, bg: Color, min_ratio: f64) -> Color {
    if crate::theme::contrast_ratio(fg, bg) >= min_ratio {
        return fg;
    }
    let (h, s, mut l) = rgb_to_hsl(fg);
    // Move away from the background's lightness pole.
    let step = if lightness(bg) < 0.5 { 0.05 } else { -0.05 };
    let mut best = fg;
    for _ in 0..20 {
        l = (l + step).clamp(0.0, 1.0);
        best = hsl_to_rgb(h, s, l);
        if crate::theme::contrast_ratio(best, bg) >= min_ratio {
            return best;
        }
    }
    best
}

fn to_hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn parse(s: &str, fallback: Color) -> Color {
    crate::theme::parse_hex_color(s).unwrap_or(fallback)
}

// ---------------------------------------------------------------------------
// Override stripping (keep structure, drop literal colors)
// ---------------------------------------------------------------------------

/// Keep WM geometry (titlebar height, button metrics, glyphs, alignment)
/// but drop every literal color so the derivation engine recolors the WM
/// from the transformed palette.
fn strip_wm_colors(wm: &WmThemeOverrides) -> WmThemeOverrides {
    let mut out = wm.clone();
    out.titlebar_active = None;
    out.titlebar_inactive = None;
    out.titlebar_text = None;
    out.frame_color = None;
    out.content_bg = None;
    out.btn_close = None;
    out.btn_minimize = None;
    out.btn_maximize = None;
    out.titlebar_gradient = None;
    out.titlebar_gradient_top = None;
    out.titlebar_gradient_bottom = None;
    out.titlebar_inactive_gradient_top = None;
    out.titlebar_inactive_gradient_bottom = None;
    out.separator_color = None;
    out.glyph_close_color = None;
    out.glyph_minimize_color = None;
    out.glyph_maximize_color = None;
    out.btn_close_hover = None;
    out.btn_minimize_hover = None;
    out.btn_maximize_hover = None;
    out.title_text_shadow_color = None;
    out.content_stroke_color = None;
    out.modal_overlay_color = None;
    out
}

/// Keep icon style / container / animation settings but drop literal colors.
fn strip_icon_colors(icons: &IconOverrides) -> IconOverrides {
    let mut out = icons.clone();
    out.body_color = None;
    out.fold_color = None;
    out.outline_color = None;
    out.shadow_color = None;
    out.label_color = None;
    out.cursor_color = None;
    out.focus_glow_color = None;
    out
}

/// Keep start menu layout (mode, dimensions, labels) but drop literal colors.
fn strip_start_menu_colors(menu: &StartMenuOverrides) -> StartMenuOverrides {
    let mut out = menu.clone();
    out.panel_bg = None;
    out.panel_gradient_top = None;
    out.panel_gradient_bottom = None;
    out.panel_border = None;
    out.item_text = None;
    out.item_text_active = None;
    out.highlight_color = None;
    out.button_bg = None;
    out.button_text = None;
    out.header_bg = None;
    out.header_text_color = None;
    out.footer_bg = None;
    out.footer_text_color = None;
    out.button_gradient = None;
    out.button_gradient_top = None;
    out.button_gradient_bottom = None;
    out.item_colors = None;
    out.item_separator_color = None;
    out
}

/// Keep geometry metrics but drop the focus ring color.
fn strip_geometry_colors(geo: &GeometryOverrides) -> GeometryOverrides {
    let mut out = geo.clone();
    out.focus_ring_color = None;
    out
}

// ---------------------------------------------------------------------------
// Variant derivation
// ---------------------------------------------------------------------------

impl SkinTheme {
    /// Derive a Dark / Light / High-contrast variant of this theme.
    ///
    /// Transforms the 9 base palette colors and preserves structural
    /// configuration (geometry, WM metrics, icon style, start menu layout,
    /// border radius, shadow intensity). Literal color overrides are dropped
    /// so the derivation engine rebuilds all component colors from the
    /// transformed palette.
    pub fn derive_variant(&self, variant: SkinVariant) -> SkinTheme {
        let bg = parse(&self.background, Color::rgb(26, 26, 45));
        let primary = parse(&self.primary, Color::rgb(50, 100, 200));
        let secondary = parse(&self.secondary, Color::rgb(80, 80, 80));
        let text = parse(&self.text, Color::WHITE);
        let dim_text = parse(&self.dim_text, Color::rgb(128, 128, 128));
        let status_bar = parse(&self.status_bar, Color::rgb(40, 60, 90));
        let prompt = parse(&self.prompt, Color::rgb(0, 255, 0));
        let output = parse(&self.output, Color::rgb(204, 204, 204));
        let error = parse(&self.error, Color::rgb(255, 68, 68));

        let (bg, primary, secondary, text, dim_text, status_bar, prompt, output, error) =
            match variant {
                SkinVariant::Dark => {
                    // Background roles: force into the dark half.
                    let bg = if lightness(bg) > 0.5 {
                        flip_lightness(bg)
                    } else {
                        bg
                    };
                    let status_bar = if lightness(status_bar) > 0.5 {
                        flip_lightness(status_bar)
                    } else {
                        status_bar
                    };
                    let secondary = if lightness(secondary) > 0.65 {
                        flip_lightness(secondary)
                    } else {
                        secondary
                    };
                    // Text roles: force into the light half, then guarantee
                    // readability against the new background.
                    let flip_text = |c: Color| {
                        if lightness(c) < 0.5 {
                            flip_lightness(c)
                        } else {
                            c
                        }
                    };
                    let text = ensure_contrast(flip_text(text), bg, 4.5);
                    let dim_text = ensure_contrast(flip_text(dim_text), bg, 3.0);
                    let output = ensure_contrast(flip_text(output), bg, 4.5);
                    // Accents keep their hue but must stay visible.
                    let primary = ensure_contrast(primary, bg, 3.0);
                    let prompt = ensure_contrast(prompt, bg, 3.0);
                    let error = ensure_contrast(error, bg, 3.0);
                    (
                        bg, primary, secondary, text, dim_text, status_bar, prompt, output, error,
                    )
                },
                SkinVariant::Light => {
                    let bg = if lightness(bg) < 0.5 {
                        flip_lightness(bg)
                    } else {
                        bg
                    };
                    let status_bar = if lightness(status_bar) < 0.5 {
                        flip_lightness(status_bar)
                    } else {
                        status_bar
                    };
                    let secondary = if lightness(secondary) < 0.35 {
                        flip_lightness(secondary)
                    } else {
                        secondary
                    };
                    let flip_text = |c: Color| {
                        if lightness(c) > 0.5 {
                            flip_lightness(c)
                        } else {
                            c
                        }
                    };
                    let text = ensure_contrast(flip_text(text), bg, 4.5);
                    let dim_text = ensure_contrast(flip_text(dim_text), bg, 3.0);
                    let output = ensure_contrast(flip_text(output), bg, 4.5);
                    let primary = ensure_contrast(primary, bg, 3.0);
                    let prompt = ensure_contrast(prompt, bg, 3.0);
                    let error = ensure_contrast(error, bg, 3.0);
                    (
                        bg, primary, secondary, text, dim_text, status_bar, prompt, output, error,
                    )
                },
                SkinVariant::HighContrast => {
                    // Near-black backgrounds keeping a hint of the original
                    // hue; near-white text; saturated, bright accents.
                    let dark_of = |c: Color, l: f32| {
                        let (h, s, _) = rgb_to_hsl(c);
                        hsl_to_rgb(h, s * 0.3, l)
                    };
                    let bg = dark_of(bg, 0.03);
                    let status_bar = dark_of(status_bar, 0.06);
                    let secondary = with_lightness(secondary, 0.55);
                    let text = ensure_contrast(with_lightness(text, 0.97), bg, 7.0);
                    let dim_text = ensure_contrast(with_lightness(dim_text, 0.8), bg, 4.5);
                    let output = ensure_contrast(with_lightness(output, 0.9), bg, 7.0);
                    let boost = |c: Color| {
                        let (h, s, _) = rgb_to_hsl(c);
                        ensure_contrast(hsl_to_rgb(h, (s * 1.5).clamp(0.0, 1.0), 0.6), bg, 4.5)
                    };
                    (
                        bg,
                        boost(primary),
                        secondary,
                        text,
                        dim_text,
                        status_bar,
                        boost(prompt),
                        output,
                        boost(error),
                    )
                },
            };

        SkinTheme {
            background: to_hex(bg),
            primary: to_hex(primary),
            secondary: to_hex(secondary),
            text: to_hex(text),
            dim_text: to_hex(dim_text),
            status_bar: to_hex(status_bar),
            prompt: to_hex(prompt),
            output: to_hex(output),
            error: to_hex(error),
            // Structural carry-overs.
            border_radius: match variant {
                SkinVariant::HighContrast => Some(0),
                _ => self.border_radius,
            },
            shadow_intensity: match variant {
                SkinVariant::HighContrast => Some(0),
                _ => self.shadow_intensity,
            },
            gradient_enabled: match variant {
                SkinVariant::HighContrast => Some(false),
                _ => self.gradient_enabled,
            },
            wm_theme: self.wm_theme.as_ref().map(strip_wm_colors),
            icon_overrides: self.icon_overrides.as_ref().map(strip_icon_colors),
            start_menu_overrides: self
                .start_menu_overrides
                .as_ref()
                .map(strip_start_menu_colors),
            geometry: self.geometry.as_ref().map(strip_geometry_colors),
            // Everything below is a literal-color override tuned for the
            // original palette -- drop it and let derivation recolor.
            ..SkinTheme::default()
        }
    }
}

impl Skin {
    /// Derive a Dark / Light / High-contrast variant of this skin.
    ///
    /// The variant keeps layout, features, and strings; the theme palette is
    /// transformed via [`SkinTheme::derive_variant`] and the manifest name is
    /// suffixed (e.g. `"classic-dark"`).
    pub fn derive_variant(&self, variant: SkinVariant) -> Skin {
        let mut out = self.clone();
        out.theme = self.theme.derive_variant(variant);
        // Avoid stacking suffixes when re-deriving from an existing variant.
        let base = SkinVariant::ALL
            .iter()
            .find_map(|v| out.manifest.name.strip_suffix(&format!("-{}", v.name())))
            .unwrap_or(&out.manifest.name)
            .to_string();
        out.manifest.name = format!("{base}-{}", variant.name());
        out
    }
}

/// Resolve a skin-swap request that may be either a skin name/path or a
/// variant request (`"@variant:dark"`) against the currently active skin.
///
/// Used by the app layers that handle `CommandSignal::SkinSwap` so the
/// `skin variant <v>` terminal command can derive from whatever skin is
/// live without a new signal type.
pub fn resolve_skin_request(name: &str, current: &Skin) -> oasis_types::error::Result<Skin> {
    resolve_variant_request(name, current).unwrap_or_else(|| crate::resolve_skin(name))
}

/// If `name` is a variant request, derive it from `current`; otherwise
/// `None` (caller falls through to normal skin resolution).
fn resolve_variant_request(name: &str, current: &Skin) -> Option<oasis_types::error::Result<Skin>> {
    let variant_name = name.strip_prefix(VARIANT_REQUEST_PREFIX)?;
    Some(match SkinVariant::from_name(variant_name) {
        Some(variant) => Ok(current.derive_variant(variant)),
        None => Err(oasis_types::error::OasisError::Config(
            format!("unknown skin variant '{variant_name}' (dark|light|high-contrast)").into(),
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;
    use crate::theme::contrast_ratio;

    fn colors(theme: &SkinTheme) -> (Color, Color) {
        (theme.background_color(), theme.text_color())
    }

    #[test]
    fn hsl_round_trip() {
        for c in [
            Color::rgb(255, 0, 0),
            Color::rgb(0, 128, 255),
            Color::rgb(26, 26, 45),
            Color::rgb(250, 248, 240),
            Color::rgb(128, 128, 128),
            Color::BLACK,
            Color::WHITE,
        ] {
            let (h, s, l) = rgb_to_hsl(c);
            let back = hsl_to_rgb(h, s, l);
            assert!(
                (back.r as i32 - c.r as i32).abs() <= 2
                    && (back.g as i32 - c.g as i32).abs() <= 2
                    && (back.b as i32 - c.b as i32).abs() <= 2,
                "HSL round trip drifted: {c:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn variant_from_name() {
        assert_eq!(SkinVariant::from_name("dark"), Some(SkinVariant::Dark));
        assert_eq!(SkinVariant::from_name("LIGHT"), Some(SkinVariant::Light));
        assert_eq!(
            SkinVariant::from_name("high-contrast"),
            Some(SkinVariant::HighContrast)
        );
        assert_eq!(
            SkinVariant::from_name("highcontrast"),
            Some(SkinVariant::HighContrast)
        );
        assert_eq!(SkinVariant::from_name("nope"), None);
    }

    #[test]
    fn dark_variant_of_light_theme_is_dark() {
        // Paper is a light skin (background #FAF8F0).
        let paper = builtin::load_builtin("paper").expect("paper loads");
        let dark = paper.theme.derive_variant(SkinVariant::Dark);
        let (bg, text) = colors(&dark);
        assert!(
            lightness(bg) < 0.5,
            "dark variant background should be dark, got {bg:?}"
        );
        assert!(
            contrast_ratio(text, bg) >= 4.5,
            "dark variant text/bg contrast too low: {}",
            contrast_ratio(text, bg)
        );
    }

    #[test]
    fn light_variant_of_dark_theme_is_light() {
        let classic = builtin::load_builtin("classic").expect("classic loads");
        let light = classic.theme.derive_variant(SkinVariant::Light);
        let (bg, text) = colors(&light);
        assert!(
            lightness(bg) > 0.5,
            "light variant background should be light, got {bg:?}"
        );
        assert!(
            contrast_ratio(text, bg) >= 4.5,
            "light variant text/bg contrast too low: {}",
            contrast_ratio(text, bg)
        );
    }

    #[test]
    fn high_contrast_variant_increases_contrast() {
        for name in ["classic", "paper", "vaporwave", "solarized"] {
            let skin = builtin::load_builtin(name).expect("builtin loads");
            let (bg0, text0) = colors(&skin.theme);
            let hc = skin.theme.derive_variant(SkinVariant::HighContrast);
            let (bg1, text1) = colors(&hc);
            let before = contrast_ratio(text0, bg0);
            let after = contrast_ratio(text1, bg1);
            assert!(
                after >= before || after >= 7.0,
                "skin '{name}': high-contrast did not raise contrast \
                 ({before:.2} -> {after:.2})"
            );
            assert!(
                after >= 7.0,
                "skin '{name}': high-contrast text/bg ratio {after:.2} < 7.0"
            );
        }
    }

    #[test]
    fn variant_preserves_structure_drops_colors() {
        let xp = builtin::load_builtin("xp").expect("xp loads");
        let dark = xp.theme.derive_variant(SkinVariant::Dark);
        // WM titlebar geometry survives; its colors do not.
        if let Some(ref src_wm) = xp.theme.wm_theme {
            let wm = dark.wm_theme.as_ref().expect("wm overrides preserved");
            assert_eq!(wm.titlebar_height, src_wm.titlebar_height);
            assert!(wm.titlebar_active.is_none());
            assert!(wm.btn_close.is_none());
        }
        // Literal-color override groups are dropped.
        assert!(dark.bar_overrides.is_none());
        assert!(dark.app_overrides.is_none());
    }

    #[test]
    fn skin_variant_renames_manifest() {
        let classic = builtin::load_builtin("classic").expect("classic loads");
        let dark = classic.derive_variant(SkinVariant::Dark);
        assert_eq!(dark.manifest.name, "classic-dark");
        // Re-deriving does not stack suffixes.
        let light = dark.derive_variant(SkinVariant::Light);
        assert_eq!(light.manifest.name, "classic-light");
    }

    #[test]
    fn resolve_skin_request_variant() {
        let classic = builtin::load_builtin("classic").expect("classic loads");
        let derived = resolve_skin_request("@variant:dark", &classic).expect("variant resolves");
        assert_eq!(derived.manifest.name, "classic-dark");
        assert!(resolve_skin_request("@variant:bogus", &classic).is_err());
        // Plain names fall through to normal resolution.
        let named = resolve_skin_request("classic", &classic).expect("name resolves");
        assert_eq!(named.manifest.name, "classic");
    }

    #[test]
    fn variant_theme_derives_active_theme() {
        // The transformed palette must survive the full derivation engine.
        let classic = builtin::load_builtin("classic").expect("classic loads");
        let hc = classic.theme.derive_variant(SkinVariant::HighContrast);
        let active = crate::active_theme::ActiveTheme::from_skin(&hc);
        // Smoke check: derived app screen text is readable on its background.
        let ratio = contrast_ratio(active.app.text, active.app.bg);
        assert!(ratio >= 4.5, "derived app text/bg ratio {ratio:.2} too low");
    }
}
