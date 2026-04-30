//! Lightweight skin presets for PSP.
//!
//! Each preset stores 9 base colors and derives a full [`ActiveTheme`] via
//! [`ActiveTheme::from_base_colors`] — no TOML parser, no embedded skin
//! strings, no `SkinTheme` struct.  Total overhead is ~300 bytes per preset
//! (9 × Color × 4 bytes + enum discriminant).

use std::collections::HashMap;

use oasis_backend_psp::{Color, GradientStops};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::skin::SkinFeatures;
use oasis_core::vector::background::{
    BackgroundLayer, LayerAnimation, LayerKind, LayerPosition,
};
use oasis_shader::ShaderParams;

use crate::theme;

/// Available skin presets for PSP.  Each corresponds to a desktop builtin skin
/// but carries only the 9 base colors needed for derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PspSkinPreset {
    Psix,
    Classic,
    Balatro,
    RetroCga,
    Solarized,
    HighContrast,
    Altimit,
}

impl PspSkinPreset {
    /// All presets in display order.
    pub(crate) const ALL: &[Self] = &[
        Self::Psix,
        Self::Classic,
        Self::Balatro,
        Self::RetroCga,
        Self::Solarized,
        Self::HighContrast,
        Self::Altimit,
    ];

    /// Human-readable display name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Psix => "PSIX",
            Self::Classic => "Classic",
            Self::Balatro => "Balatro",
            Self::RetroCga => "Retro CGA",
            Self::Solarized => "Solarized",
            Self::HighContrast => "High Contrast",
            Self::Altimit => "Altimit",
        }
    }

    /// Config key used in `config.rcfg`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Psix => "psix",
            Self::Classic => "classic",
            Self::Balatro => "balatro",
            Self::RetroCga => "retro-cga",
            Self::Solarized => "solarized",
            Self::HighContrast => "highcontrast",
            Self::Altimit => "altimit",
        }
    }

    /// Resolve a config key to a preset, defaulting to `Psix`.
    pub(crate) fn from_key(key: &str) -> Self {
        Self::ALL
            .iter()
            .find(|p| p.key() == key)
            .copied()
            .unwrap_or(Self::Psix)
    }

    /// The 9 base colors: (background, primary, secondary, text, dim_text,
    /// status_bar, prompt, output, error).
    fn base_colors(self) -> [Color; 9] {
        match self {
            Self::Psix => [
                // PSIX: the original green-tinted PSP shell style
                Color::rgb(0x1A, 0x1A, 0x2D), // background
                Color::rgb(0x32, 0x64, 0xC8), // primary
                Color::rgb(0x50, 0x50, 0x50), // secondary
                Color::WHITE,                 // text
                Color::rgb(0x80, 0x80, 0x80), // dim_text
                Color::rgb(0x28, 0x3C, 0x5A), // status_bar
                Color::rgb(0x00, 0xFF, 0x00), // prompt
                Color::rgb(0xCC, 0xCC, 0xCC), // output
                Color::rgb(0xFF, 0x44, 0x44), // error
            ],
            Self::Classic => [
                Color::rgb(0x0E, 0x0E, 0x1C), // background
                Color::rgb(0x44, 0x88, 0xCC), // primary
                Color::rgb(0x2A, 0x2A, 0x3E), // secondary
                Color::rgb(0xE0, 0xE0, 0xF0), // text
                Color::rgb(0x70, 0x70, 0x90), // dim_text
                Color::rgb(0x18, 0x18, 0x2C), // status_bar
                Color::rgb(0x44, 0xCC, 0x88), // prompt
                Color::rgb(0xC0, 0xC0, 0xD8), // output
                Color::rgb(0xFF, 0x44, 0x66), // error
            ],
            Self::Balatro => [
                Color::rgb(0x0A, 0x0A, 0x14),        // background
                Color::rgb(0x00, 0xF0, 0xFF),        // primary
                Color::rgb(0x1A, 0x1A, 0x2E),        // secondary
                Color::rgb(0xE0, 0xF0, 0xFF),        // text
                Color::rgb(0x50, 0x60, 0x80),        // dim_text
                Color::rgba(0x08, 0x08, 0x10, 0x80), // status_bar
                Color::rgb(0x00, 0xF0, 0xFF),        // prompt
                Color::rgb(0xC0, 0xD8, 0xFF),        // output
                Color::rgb(0xFF, 0x20, 0x60),        // error
            ],
            Self::RetroCga => [
                Color::rgb(0x00, 0x00, 0x00), // background
                Color::rgb(0x55, 0xFF, 0xFF), // primary
                Color::rgb(0xFF, 0x55, 0xFF), // secondary
                Color::rgb(0xFF, 0xFF, 0xFF), // text
                Color::rgb(0x55, 0xFF, 0xFF), // dim_text
                Color::rgb(0x00, 0x00, 0x00), // status_bar
                Color::rgb(0x55, 0xFF, 0xFF), // prompt
                Color::rgb(0xFF, 0xFF, 0xFF), // output
                Color::rgb(0xFF, 0x55, 0xFF), // error
            ],
            Self::Solarized => [
                Color::rgb(0x00, 0x2B, 0x36), // background
                Color::rgb(0x26, 0x8B, 0xD2), // primary
                Color::rgb(0x07, 0x36, 0x42), // secondary
                Color::rgb(0x83, 0x94, 0x96), // text
                Color::rgb(0x58, 0x6E, 0x75), // dim_text
                Color::rgb(0x07, 0x36, 0x42), // status_bar
                Color::rgb(0x85, 0x99, 0x00), // prompt
                Color::rgb(0x83, 0x94, 0x96), // output
                Color::rgb(0xDC, 0x32, 0x2F), // error
            ],
            Self::HighContrast => [
                Color::rgb(0x00, 0x00, 0x00), // background
                Color::rgb(0xFF, 0xFF, 0x00), // primary
                Color::rgb(0x1A, 0x1A, 0x1A), // secondary
                Color::rgb(0xFF, 0xFF, 0xFF), // text
                Color::rgb(0xCC, 0xCC, 0xCC), // dim_text
                Color::rgb(0x1A, 0x1A, 0x1A), // status_bar
                Color::rgb(0xFF, 0xFF, 0x00), // prompt
                Color::rgb(0xFF, 0xFF, 0xFF), // output
                Color::rgb(0xFF, 0x44, 0x44), // error
            ],
            Self::Altimit => [
                Color::rgb(0x08, 0x08, 0x16),        // background
                Color::rgb(0x00, 0xCC, 0x88),        // primary
                Color::rgb(0x1A, 0x1A, 0x2E),        // secondary
                Color::rgb(0xD0, 0xE8, 0xE0),        // text
                Color::rgb(0x50, 0x68, 0x60),        // dim_text
                Color::rgba(0x0A, 0x0A, 0x1A, 0x80), // status_bar
                Color::rgb(0x00, 0xCC, 0x88),        // prompt
                Color::rgb(0xB0, 0xD8, 0xC8),        // output
                Color::rgb(0xFF, 0x44, 0x66),        // error
            ],
        }
    }

    /// Shader configuration for this skin, if any.
    ///
    /// Returns `(shader_name, ShaderParams)` matching the desktop TOML skins.
    pub(crate) fn shader_config(self) -> Option<(&'static str, ShaderParams)> {
        match self {
            Self::Balatro => Some((
                "balatro",
                ShaderParams {
                    colors: vec![
                        hex_to_f4(0x00, 0xF0, 0xFF),
                        hex_to_f4(0x00, 0x6B, 0xB4),
                        hex_to_f4(0x16, 0x23, 0x25),
                    ],
                    floats: HashMap::from([
                        ("speed".into(), 1.0),
                        ("contrast".into(), 3.5),
                        ("spin_speed".into(), 1.0),
                        ("spin_amount".into(), 0.25),
                        ("pixel_filter".into(), 745.0),
                        ("lighting".into(), 0.4),
                        ("spin_ease".into(), 1.0),
                    ]),
                },
            )),
            Self::RetroCga => Some((
                "voronoi",
                ShaderParams {
                    colors: vec![
                        hex_to_f4(0x55, 0xFF, 0x55),
                        hex_to_f4(0xFF, 0x55, 0xFF),
                    ],
                    floats: HashMap::from([
                        ("speed".into(), 0.5),
                        // Size=6: ~6 cells across 11 internal pixels (RENDER_SCALE=3
                        // at 32x32). Larger cells look better at low res and reduce
                        // the number of unique voronoi_pt evaluations.
                        ("size".into(), 6.0),
                    ]),
                },
            )),
            Self::Solarized => Some((
                "ocean_waves",
                ShaderParams {
                    colors: vec![
                        hex_to_f4(0x00, 0x2B, 0x36),
                        hex_to_f4(0x00, 0xE0, 0xE0),
                        hex_to_f4(0x58, 0x6E, 0x75),
                    ],
                    floats: HashMap::from([("speed".into(), 0.6)]),
                },
            )),
            Self::Altimit => Some((
                "starfield",
                ShaderParams {
                    colors: vec![
                        hex_to_f4(0x00, 0xCC, 0x88),
                        hex_to_f4(0x08, 0x08, 0x16),
                    ],
                    floats: HashMap::from([("speed".into(), 0.6)]),
                },
            )),
            _ => None,
        }
    }

    /// Build a full [`ActiveTheme`] from this preset with PSP-specific
    /// geometry overrides applied.
    pub(crate) fn to_active_theme(self) -> ActiveTheme {
        let [
            bg,
            primary,
            secondary,
            text,
            dim_text,
            status_bar,
            prompt,
            output,
            error,
        ] = self.base_colors();
        let mut t = ActiveTheme::from_base_colors(
            bg, primary, secondary, text, dim_text, status_bar, prompt, output, error,
        )
        .with_screen_size(
            oasis_backend_psp::SCREEN_WIDTH,
            oasis_backend_psp::SCREEN_HEIGHT,
        );

        apply_psp_overrides(&mut t);
        self.apply_icon_palette(&mut t);

        // Inject shader background layer *after* PSP overrides (which
        // truncate to 4 layers) so the shader is never silently dropped.
        if let Some((name, params)) = self.shader_config() {
            t.background_layers.push(BackgroundLayer {
                kind: LayerKind::Shader {
                    name: name.to_string(),
                    params,
                },
                color: Color::WHITE,
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            });
        }

        t
    }

    /// Override the derived [`IconTheme`] with a preset-specific palette.
    ///
    /// `ActiveTheme::from_base_colors` derives `body_color` from the theme's
    /// text color, so most themes (which all use a near-white text) end up
    /// with near-identical white "paper" icons. Override here to give each
    /// theme a recognisable look — body / outline / fold / label — while
    /// the per-app accent stripe (`AppEntry::color`) keeps icons distinct
    /// from each other.
    fn apply_icon_palette(self, t: &mut ActiveTheme) {
        match self {
            Self::Psix => {
                // PSIX original: bright paper white, blue-tint shadow.
                t.icon.body_color = Color::rgb(250, 250, 248);
                t.icon.outline_color = Color::rgba(255, 255, 255, 180);
                t.icon.fold_color = Color::rgb(210, 210, 205);
                t.icon.shadow_color = Color::rgba(0, 0, 30, 90);
                t.icon.label_color = Color::rgba(255, 255, 255, 230);
                t.icon.label_shadow = Some(Color::rgba(0, 0, 0, 120));
            },
            Self::Classic => {
                // Slightly cooler paper for the classic theme.
                t.icon.body_color = Color::rgb(220, 225, 240);
                t.icon.outline_color = Color::rgba(140, 160, 200, 200);
                t.icon.fold_color = Color::rgb(150, 160, 180);
                t.icon.shadow_color = Color::rgba(0, 0, 20, 100);
                t.icon.label_color = Color::rgb(220, 230, 240);
                t.icon.label_shadow = Some(Color::rgba(0, 0, 0, 140));
            },
            Self::Balatro => {
                // Cyan-tinted dark icons matching the spinning shader.
                t.icon.body_color = Color::rgb(20, 35, 55);
                t.icon.outline_color = Color::rgba(0, 240, 255, 220);
                t.icon.fold_color = Color::rgb(0, 180, 200);
                t.icon.shadow_color = Color::rgba(0, 240, 255, 60);
                t.icon.label_color = Color::rgb(200, 240, 255);
                t.icon.label_shadow = None;
            },
            Self::RetroCga => {
                // Monitor look: black bezel + cyan/magenta phosphor.
                t.icon.body_color = Color::rgb(0, 0, 0);
                t.icon.outline_color = Color::rgb(85, 255, 255);
                t.icon.fold_color = Color::rgb(255, 85, 255);
                t.icon.shadow_color = Color::rgba(255, 85, 255, 90);
                t.icon.label_color = Color::rgb(85, 255, 255);
                t.icon.label_shadow = None;
            },
            Self::Solarized => {
                // Solarized base02 paper, base1 outline.
                t.icon.body_color = Color::rgb(7, 54, 66);
                t.icon.outline_color = Color::rgb(147, 161, 161);
                t.icon.fold_color = Color::rgb(101, 123, 131);
                t.icon.shadow_color = Color::rgba(0, 30, 40, 120);
                t.icon.label_color = Color::rgb(147, 161, 161);
                t.icon.label_shadow = None;
            },
            Self::HighContrast => {
                // Pure black body with a thick yellow outline.
                t.icon.body_color = Color::rgb(0, 0, 0);
                t.icon.outline_color = Color::rgb(255, 255, 0);
                t.icon.fold_color = Color::rgb(255, 255, 0);
                t.icon.shadow_color = Color::rgba(255, 255, 0, 80);
                t.icon.label_color = Color::rgb(255, 255, 0);
                t.icon.label_shadow = None;
            },
            Self::Altimit => {
                // Deep navy body with mint accents to echo the starfield.
                t.icon.body_color = Color::rgb(10, 18, 28);
                t.icon.outline_color = Color::rgb(0, 204, 136);
                t.icon.fold_color = Color::rgb(0, 150, 100);
                t.icon.shadow_color = Color::rgba(0, 204, 136, 70);
                t.icon.label_color = Color::rgb(180, 232, 210);
                t.icon.label_shadow = None;
            },
        }
    }

    /// Five-stop palette for the static-gradient wallpaper. Used only by
    /// non-shader presets (the others paint a `LayerKind::Shader` over the
    /// gradient and you never see it). Colors are passed to
    /// [`oasis_backend_psp::generate_gradient_with`] which preserves the
    /// PSIX wave-arc shape and just recolors the sweep.
    pub(crate) fn gradient_stops(self) -> GradientStops {
        match self {
            // Original PSIX orange→lime — keep as canonical reference.
            Self::Psix => [
                (245, 110, 15),
                (255, 170, 15),
                (255, 230, 30),
                (230, 245, 40),
                (140, 235, 50),
            ],
            // Cool indigo → cyan sweep so Classic stops looking like PSIX.
            Self::Classic => [
                (24, 32, 90),
                (44, 72, 140),
                (60, 120, 180),
                (80, 170, 210),
                (140, 220, 230),
            ],
            // Cyan-magenta plasma echo (still mostly hidden by Balatro shader).
            Self::Balatro => [
                (10, 20, 36),
                (16, 64, 96),
                (24, 120, 160),
                (60, 200, 220),
                (140, 240, 255),
            ],
            // Stark CGA: pure black → cyan → magenta.
            Self::RetroCga => [
                (0, 0, 0),
                (0, 0, 80),
                (0, 100, 160),
                (170, 90, 200),
                (255, 85, 255),
            ],
            // Solarized base03 → cyan accent.
            Self::Solarized => [
                (0, 30, 38),
                (7, 54, 66),
                (38, 139, 210),
                (42, 161, 152),
                (133, 153, 0),
            ],
            // Black → vivid yellow ramp for unmistakable contrast.
            Self::HighContrast => [
                (0, 0, 0),
                (40, 30, 0),
                (110, 90, 0),
                (200, 170, 20),
                (255, 230, 0),
            ],
            // Deep navy → mint, matching the Altimit accent palette.
            Self::Altimit => [
                (8, 10, 26),
                (16, 30, 44),
                (10, 80, 80),
                (0, 160, 130),
                (140, 230, 200),
            ],
        }
    }

    /// Build the matching [`SkinFeatures`] (grid layout for PSP).
    pub(crate) fn skin_features() -> SkinFeatures {
        let mut f = SkinFeatures::default();
        f.grid_cols = 5;
        f.grid_rows = 3;
        f.icons_per_page = 15;
        // Unified desktop: bottom bar shows taskbar buttons, not media tabs.
        f.show_media_tabs = false;
        f.show_page_dots = false;
        f
    }
}

/// Apply PSP hardware-specific overrides to any `ActiveTheme`.
///
/// These ensure opaque bars (semi-transparent alpha darkens window content on
/// the PSP GE), compact icon grid for 480×272, and correct bar heights.
pub(crate) fn apply_psp_overrides(t: &mut ActiveTheme) {
    // Opaque bar backgrounds (semi-transparent alpha=80 looks muddy on PSP).
    t.bar.statusbar_bg = Color::rgba(
        t.bar.statusbar_bg.r,
        t.bar.statusbar_bg.g,
        t.bar.statusbar_bg.b,
        255,
    );
    t.bar.bg = Color::rgba(t.bar.bg.r, t.bar.bg.g, t.bar.bg.b, 255);

    // Bar geometry matching PSP layout.
    t.statusbar_height = theme::STATUSBAR_H;
    t.tab_row_height = 0;
    t.bottombar_height = theme::BOTTOMBAR_H;

    // Compact icons for 4×3 grid on 480×272.
    t.icon_width = theme::ICON_W;
    t.icon_height = theme::ICON_H;
    t.icon_stripe_h = theme::ICON_STRIPE_H as u32;
    t.icon_fold_size = theme::ICON_FOLD_SIZE as u32;
    t.grid_padding_x = theme::GRID_PAD_X as u16;
    t.grid_padding_y = theme::GRID_PAD_Y as u16;
    t.cursor_pad = theme::CURSOR_PAD;

    // Background layer guardrails for PSP performance.
    // Filter out expensive layer types that strain the PSP GE.
    // Shader layers are allowed — rendered via CPU software renderer.
    t.background_layers.retain(|layer| {
        !matches!(
            layer.kind,
            LayerKind::FloatingPolygons { .. }
                | LayerKind::EqBars { .. }
                | LayerKind::Waves { .. }
        )
    });
    // Cap at 4 base layers max on PSP hardware (shader layer appended separately).
    t.background_layers.truncate(4);
    // Tighter complexity budget for 333MHz MIPS.
    t.background_max_layers = 4;
    t.background_complexity_budget = t.background_complexity_budget.min(100);
    t.background_reduced_motion = true;
}

/// Convert RGB hex values to `[f32; 4]` RGBA (alpha = 1.0).
fn hex_to_f4(r: u8, g: u8, b: u8) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

#[cfg(test)]
impl PspSkinPreset {
    /// Cycle to the next preset in the list.
    pub(crate) fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Cycle to the previous preset in the list.
    pub(crate) fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Apply a skin preset at runtime: rebuild [`ActiveTheme`], refresh the
/// dashboard config to use the new colors, and persist the choice to
/// `config.rcfg`. Returns `true` if the preset actually changed.
///
/// Used by the terminal `skin NAME` command, the Settings kiosk app, and
/// the TCP cmd_server's remote skin-change channel.
pub(crate) fn apply_skin_preset(
    preset: PspSkinPreset,
    current_preset: &mut PspSkinPreset,
    active_theme: &mut oasis_core::active_theme::ActiveTheme,
    skin_features: &oasis_core::skin::SkinFeatures,
    dashboard: &mut oasis_core::dashboard::DashboardState,
    config: &mut psp::config::Config,
) -> bool {
    if preset == *current_preset {
        return false;
    }
    *current_preset = preset;
    *active_theme = preset.to_active_theme();
    dashboard.config =
        oasis_core::dashboard::DashboardConfig::from_features(skin_features, active_theme);
    config.set(
        "skin",
        psp::config::ConfigValue::Str(preset.key().to_string()),
    );
    let _ = config.save(crate::theme::CONFIG_PATH);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::vector::background::LayerKind;

    // -----------------------------------------------------------------------
    // hex_to_f4 helper
    // -----------------------------------------------------------------------

    #[test]
    fn hex_to_f4_black() {
        let c = hex_to_f4(0, 0, 0);
        assert_eq!(c, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn hex_to_f4_white() {
        let c = hex_to_f4(255, 255, 255);
        assert!((c[0] - 1.0).abs() < 1e-5);
        assert!((c[1] - 1.0).abs() < 1e-5);
        assert!((c[2] - 1.0).abs() < 1e-5);
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn hex_to_f4_known_color() {
        // Balatro cyan: #00F0FF
        let c = hex_to_f4(0x00, 0xF0, 0xFF);
        assert!((c[0] - 0.0).abs() < 1e-5);
        assert!((c[1] - 0xF0 as f32 / 255.0).abs() < 1e-5);
        assert!((c[2] - 1.0).abs() < 1e-5);
    }

    // -----------------------------------------------------------------------
    // shader_config correctness
    // -----------------------------------------------------------------------

    #[test]
    fn shader_skins_return_correct_names() {
        assert_eq!(PspSkinPreset::Balatro.shader_config().unwrap().0, "balatro");
        assert_eq!(PspSkinPreset::RetroCga.shader_config().unwrap().0, "voronoi");
        assert_eq!(
            PspSkinPreset::Solarized.shader_config().unwrap().0,
            "ocean_waves"
        );
        assert_eq!(
            PspSkinPreset::Altimit.shader_config().unwrap().0,
            "starfield"
        );
    }

    #[test]
    fn non_shader_skins_return_none() {
        assert!(PspSkinPreset::Psix.shader_config().is_none());
        assert!(PspSkinPreset::Classic.shader_config().is_none());
        assert!(PspSkinPreset::HighContrast.shader_config().is_none());
    }

    #[test]
    fn shader_params_have_colors() {
        for preset in PspSkinPreset::ALL {
            if let Some((_, params)) = preset.shader_config() {
                assert!(
                    !params.colors.is_empty(),
                    "{:?} shader should have at least one color",
                    preset,
                );
                for color in &params.colors {
                    for &c in &color[..3] {
                        assert!(
                            (0.0..=1.0).contains(&c),
                            "{:?} color component {c} out of range",
                            preset,
                        );
                    }
                    assert_eq!(color[3], 1.0, "{:?} alpha must be 1.0", preset);
                }
            }
        }
    }

    #[test]
    fn shader_params_speed_is_positive() {
        for preset in PspSkinPreset::ALL {
            if let Some((_, params)) = preset.shader_config() {
                if let Some(&speed) = params.floats.get("speed") {
                    assert!(
                        speed > 0.0,
                        "{:?} shader speed must be positive, got {speed}",
                        preset,
                    );
                }
            }
        }
    }

    #[test]
    fn all_shader_names_are_registered() {
        // Every shader name returned by shader_config must be recognized
        // by the software renderer (dispatches to a render function).
        let known = [
            "balatro",
            "voronoi",
            "city_lights",
            "ocean_waves",
            "calm_waves",
            "starfield",
            "plasma",
            "matrix_rain",
        ];
        for preset in PspSkinPreset::ALL {
            if let Some((name, _)) = preset.shader_config() {
                assert!(
                    known.contains(&name),
                    "{:?} uses unknown shader '{name}'",
                    preset,
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Theme derivation with shader layers
    // -----------------------------------------------------------------------

    #[test]
    fn shader_skins_inject_shader_layer() {
        for preset in PspSkinPreset::ALL {
            let theme = preset.to_active_theme();
            let has_shader = theme.background_layers.iter().any(|l| {
                matches!(l.kind, LayerKind::Shader { .. })
            });
            let expects_shader = preset.shader_config().is_some();
            assert_eq!(
                has_shader, expects_shader,
                "{:?}: shader layer mismatch (expected={expects_shader}, found={has_shader})",
                preset,
            );
        }
    }

    #[test]
    fn shader_layer_name_matches_config() {
        for preset in PspSkinPreset::ALL {
            if let Some((expected_name, _)) = preset.shader_config() {
                let theme = preset.to_active_theme();
                let layer = theme
                    .background_layers
                    .iter()
                    .find_map(|l| match &l.kind {
                        LayerKind::Shader { name, .. } => Some(name.as_str()),
                        _ => None,
                    });
                assert_eq!(
                    layer,
                    Some(expected_name),
                    "{:?}: shader layer name mismatch",
                    preset,
                );
            }
        }
    }

    #[test]
    fn non_shader_skins_have_no_shader_layer() {
        for preset in &[
            PspSkinPreset::Psix,
            PspSkinPreset::Classic,
            PspSkinPreset::HighContrast,
        ] {
            let theme = preset.to_active_theme();
            let has_shader = theme.background_layers.iter().any(|l| {
                matches!(l.kind, LayerKind::Shader { .. })
            });
            assert!(!has_shader, "{:?} should not have a shader layer", preset);
        }
    }

    // -----------------------------------------------------------------------
    // apply_psp_overrides allows shader layers
    // -----------------------------------------------------------------------

    #[test]
    fn overrides_retain_shader_layers() {
        let theme = PspSkinPreset::Balatro.to_active_theme();
        // Balatro should have a shader layer that survived overrides.
        let count = theme
            .background_layers
            .iter()
            .filter(|l| matches!(l.kind, LayerKind::Shader { .. }))
            .count();
        assert_eq!(count, 1, "Balatro theme should have exactly 1 shader layer");
    }

    #[test]
    fn overrides_still_filter_expensive_layers() {
        // Manually inject an expensive layer, then apply overrides.
        let mut theme = PspSkinPreset::Psix.to_active_theme();
        theme.background_layers.push(BackgroundLayer {
            kind: LayerKind::FloatingPolygons { count: 5, sides: 6 },
            color: Color::WHITE,
            position: LayerPosition::default(),
            animation: LayerAnimation::default(),
            enabled: true,
        });
        theme.background_layers.push(BackgroundLayer {
            kind: LayerKind::EqBars {
                count: 8,
                bar_width: 10,
                max_height: 100,
            },
            color: Color::WHITE,
            position: LayerPosition::default(),
            animation: LayerAnimation::default(),
            enabled: true,
        });
        apply_psp_overrides(&mut theme);
        let has_expensive = theme.background_layers.iter().any(|l| {
            matches!(
                l.kind,
                LayerKind::FloatingPolygons { .. }
                    | LayerKind::EqBars { .. }
                    | LayerKind::Waves { .. }
            )
        });
        assert!(
            !has_expensive,
            "apply_psp_overrides should filter expensive layers"
        );
    }

    #[test]
    fn overrides_cap_at_four_layers() {
        let mut theme = PspSkinPreset::Psix.to_active_theme();
        for _ in 0..10 {
            theme.background_layers.push(BackgroundLayer {
                kind: LayerKind::Grid { spacing: 30 },
                color: Color::WHITE,
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            });
        }
        apply_psp_overrides(&mut theme);
        assert!(
            theme.background_layers.len() <= 4,
            "PSP should cap at 4 background layers, got {}",
            theme.background_layers.len(),
        );
    }

    // -----------------------------------------------------------------------
    // get_shader_layer integration
    // -----------------------------------------------------------------------

    #[test]
    fn get_shader_layer_finds_shader_skins() {
        for preset in PspSkinPreset::ALL {
            let theme = preset.to_active_theme();
            let found = oasis_core::vector_overlay::get_shader_layer(&theme);
            let expects = preset.shader_config().is_some();
            assert_eq!(
                found.is_some(),
                expects,
                "{:?}: get_shader_layer mismatch",
                preset,
            );
        }
    }

    #[test]
    fn get_shader_layer_returns_correct_params() {
        let theme = PspSkinPreset::Balatro.to_active_theme();
        let info =
            oasis_core::vector_overlay::get_shader_layer(&theme).unwrap();
        assert_eq!(info.name, "balatro");
        assert_eq!(info.params.colors.len(), 3);
        assert!(info.params.floats.contains_key("speed"));
        assert!(info.params.floats.contains_key("contrast"));
        assert!(info.params.floats.contains_key("spin_speed"));
    }

    // -----------------------------------------------------------------------
    // Software renderer produces valid output for all PSP shaders
    // -----------------------------------------------------------------------

    #[test]
    fn all_psp_shaders_render_without_panic() {
        use oasis_shader::software::SoftwareShaderRenderer;
        let mut renderer = SoftwareShaderRenderer::new(64, 64);
        for preset in PspSkinPreset::ALL {
            if let Some((name, params)) = preset.shader_config() {
                // Render at three time points to catch time-dependent panics.
                for &t in &[0.0, 1.0, 10.0] {
                    let pixels = renderer.render_shader(name, t, &params);
                    assert_eq!(
                        pixels.len(),
                        64 * 64 * 4,
                        "{:?} ({name}) at t={t}: wrong pixel count",
                        preset,
                    );
                }
            }
        }
    }

    #[test]
    fn all_psp_shaders_produce_non_black_output() {
        use oasis_shader::software::SoftwareShaderRenderer;
        let mut renderer = SoftwareShaderRenderer::new(64, 64);
        for preset in PspSkinPreset::ALL {
            if let Some((name, params)) = preset.shader_config() {
                let pixels = renderer.render_shader(name, 1.0, &params);
                let has_color = pixels
                    .chunks(4)
                    .any(|px| px[0] > 10 || px[1] > 10 || px[2] > 10);
                assert!(
                    has_color,
                    "{:?} ({name}): rendered all-black output",
                    preset,
                );
            }
        }
    }

    #[test]
    fn shader_output_changes_over_time() {
        use oasis_shader::software::SoftwareShaderRenderer;
        let mut renderer = SoftwareShaderRenderer::new(64, 64);
        for preset in PspSkinPreset::ALL {
            if let Some((name, params)) = preset.shader_config() {
                let pixels_t0 = renderer.render_shader(name, 0.0, &params).to_vec();
                let pixels_t5 = renderer.render_shader(name, 5.0, &params).to_vec();
                assert_ne!(
                    pixels_t0, pixels_t5,
                    "{:?} ({name}): output should differ between t=0 and t=5",
                    preset,
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Performance: shader render stays within budget
    // -----------------------------------------------------------------------

    #[test]
    fn shader_render_time_under_budget() {
        use oasis_shader::software::SoftwareShaderRenderer;
        use std::time::Instant;

        let mut renderer = SoftwareShaderRenderer::new(64, 64);
        for preset in PspSkinPreset::ALL {
            if let Some((name, params)) = preset.shader_config() {
                // Warm up.
                let _ = renderer.render_shader(name, 0.0, &params);

                // Measure 10 frames.
                let start = Instant::now();
                for i in 0..10 {
                    let _ = renderer.render_shader(
                        name,
                        i as f32 / 30.0,
                        &params,
                    );
                }
                let elapsed = start.elapsed();
                let per_frame_us = elapsed.as_micros() / 10;

                // Budget: 16ms per frame at 60fps. Shader runs at 30fps
                // (every other frame), so 33ms budget. On PSP's 333MHz
                // MIPS, multiply host time by ~10x for rough estimate.
                // We check that host time is under 2ms (-> ~20ms on PSP).
                assert!(
                    per_frame_us < 2000,
                    "{:?} ({name}): {per_frame_us}us/frame exceeds 2ms budget",
                    preset,
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Skin cycling preserves shader/non-shader transition
    // -----------------------------------------------------------------------

    #[test]
    fn skin_cycle_shader_transitions() {
        // Cycle through all presets in order and verify the shader state
        // toggles correctly as we pass through shader/non-shader skins.
        let mut saw_shader = false;
        let mut saw_no_shader = false;
        for preset in PspSkinPreset::ALL {
            let theme = preset.to_active_theme();
            let info = oasis_core::vector_overlay::get_shader_layer(&theme);

            match preset.shader_config() {
                Some((name, _)) => {
                    let found = info.as_ref().map(|i| i.name.as_str());
                    assert_eq!(found, Some(name), "{:?} should have shader", preset);
                    saw_shader = true;
                },
                None => {
                    assert!(info.is_none(), "{:?} should have no shader", preset);
                    saw_no_shader = true;
                },
            }
        }
        // Ensure we tested at least one of each.
        assert!(saw_shader, "should have tested at least one shader skin");
        assert!(saw_no_shader, "should have tested at least one non-shader skin");
    }
}
