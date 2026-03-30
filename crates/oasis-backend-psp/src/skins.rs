//! Lightweight skin presets for PSP.
//!
//! Each preset stores 9 base colors and derives a full [`ActiveTheme`] via
//! [`ActiveTheme::from_base_colors`] — no TOML parser, no embedded skin
//! strings, no `SkinTheme` struct.  Total overhead is ~300 bytes per preset
//! (9 × Color × 4 bytes + enum discriminant).

use oasis_backend_psp::Color;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::skin::SkinFeatures;
use oasis_core::vector::background::LayerKind;

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
    Terminal,
    Altimit,
    Tactical,
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
        Self::Terminal,
        Self::Altimit,
        Self::Tactical,
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
            Self::Terminal => "Terminal",
            Self::Altimit => "Altimit",
            Self::Tactical => "Tactical",
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
            Self::Terminal => "terminal",
            Self::Altimit => "altimit",
            Self::Tactical => "tactical",
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
            Self::Terminal => [
                Color::rgb(0x00, 0x00, 0x00), // background
                Color::rgb(0x00, 0xFF, 0x00), // primary
                Color::rgb(0x00, 0x33, 0x00), // secondary
                Color::rgb(0x00, 0xCC, 0x00), // text
                Color::rgb(0x00, 0x66, 0x00), // dim_text
                Color::rgb(0x00, 0x1A, 0x00), // status_bar
                Color::rgb(0x00, 0xFF, 0x00), // prompt
                Color::rgb(0x00, 0xCC, 0x00), // output
                Color::rgb(0xFF, 0x33, 0x33), // error
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
            Self::Tactical => [
                Color::rgb(0x0A, 0x0A, 0x0A), // background
                Color::rgb(0xCC, 0x88, 0x00), // primary
                Color::rgb(0x33, 0x33, 0x33), // secondary
                Color::rgb(0xAA, 0xAA, 0xAA), // text
                Color::rgb(0x66, 0x66, 0x66), // dim_text
                Color::rgb(0x1A, 0x1A, 0x1A), // status_bar
                Color::rgb(0xCC, 0x88, 0x00), // prompt
                Color::rgb(0xAA, 0xAA, 0xAA), // output
                Color::rgb(0xCC, 0x33, 0x33), // error
            ],
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
        t
    }

    /// Build the matching [`SkinFeatures`] (grid layout for PSP).
    pub(crate) fn skin_features() -> SkinFeatures {
        let mut f = SkinFeatures::default();
        f.grid_cols = 4;
        f.grid_rows = 3;
        f.icons_per_page = 12;
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
    t.background_layers.retain(|layer| {
        !matches!(
            layer.kind,
            LayerKind::FloatingPolygons { .. }
                | LayerKind::EqBars { .. }
                | LayerKind::Waves { .. }
                | LayerKind::Shader { .. }
        )
    });
    // Cap at 4 layers max on PSP hardware.
    t.background_layers.truncate(4);
    // Tighter complexity budget for 333MHz MIPS.
    t.background_max_layers = 4;
    t.background_complexity_budget = t.background_complexity_budget.min(100);
    t.background_reduced_motion = true;
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
