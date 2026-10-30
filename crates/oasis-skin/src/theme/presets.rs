//! Named presets and scales: gradient / animation presets, easing names,
//! the UI density scale and the semantic elevation ladder.

use serde::Deserialize;

/// A reusable gradient preset (two-color linear gradient).
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct GradientPreset {
    /// Start color (hex).
    pub from: String,
    /// End color (hex).
    pub to: String,
}

/// A named animation timing preset.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct AnimationPreset {
    /// Duration in milliseconds.
    pub duration_ms: u32,
    /// Easing function name: one of [`EASING_NAMES`] (`skin lint` warns on
    /// anything else; [`resolve_easing`] falls back to linear).
    #[serde(default = "default_easing")]
    pub easing: String,
}

fn default_easing() -> String {
    "linear".to_string()
}

/// Easing curve names [`resolve_easing`] understands.
pub const EASING_NAMES: &[&str] = &[
    "linear",
    "ease_in_quad",
    "ease_out_quad",
    "ease_in_out_quad",
    "ease_out_cubic",
    "ease_in_out_cubic",
    "ease_out_elastic",
    "ease_out_bounce",
];

/// Resolve an easing function name (one of [`EASING_NAMES`]) to a function
/// pointer. Returns `linear` for unknown names (`skin lint` flags them).
pub fn resolve_easing(name: &str) -> fn(f32) -> f32 {
    use oasis_ui::animation::easing;
    match name {
        "linear" => easing::linear,
        "ease_in_quad" => easing::ease_in_quad,
        "ease_out_quad" => easing::ease_out_quad,
        "ease_in_out_quad" => easing::ease_in_out_quad,
        "ease_out_cubic" => easing::ease_out_cubic,
        "ease_in_out_cubic" => easing::ease_in_out_cubic,
        "ease_out_elastic" => easing::ease_out_elastic,
        "ease_out_bounce" => easing::ease_out_bounce,
        _ => easing::linear,
    }
}

// ---------------------------------------------------------------------------
// Density scale + semantic elevation ladder ("layout as data")
// ---------------------------------------------------------------------------

/// UI density preset, applied as a global multiplier to spacing tokens only.
///
/// `comfortable` (the default) is `×1.0` -- no change, pixel-identical to the
/// legacy layout. `compact` is `×0.85` and `spacious` is `×1.15`. Font sizes
/// are unaffected; only the derived `ui::Theme` `spacing_*` ladder is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    /// Tighter layout (×0.85 spacing).
    Compact,
    /// Default layout (×1.0 spacing).
    #[default]
    Comfortable,
    /// Looser layout (×1.15 spacing).
    Spacious,
}

impl Density {
    /// Parse a density name. Unknown / unset values fall back to `comfortable`.
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("compact") => Self::Compact,
            Some("spacious") => Self::Spacious,
            _ => Self::Comfortable,
        }
    }

    /// The spacing multiplier for this density.
    pub fn multiplier(self) -> f32 {
        match self {
            Self::Compact => 0.85,
            Self::Comfortable => 1.0,
            Self::Spacious => 1.15,
        }
    }

    /// Scale a base spacing token by this density's multiplier.
    ///
    /// `Comfortable` returns the value unchanged (integer identity), so the
    /// default preset never perturbs existing pixel measurements. Other
    /// densities round to the nearest pixel.
    pub fn scale(self, base: u16) -> u16 {
        match self {
            Self::Comfortable => base,
            other => (f32::from(base) * other.multiplier()).round() as u16,
        }
    }
}

/// A single shadow layer within an `[elevation]` override.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct ElevationLayerConfig {
    /// Horizontal offset in pixels.
    pub offset_x: i32,
    /// Vertical offset in pixels.
    pub offset_y: i32,
    /// Size expansion beyond the element bounds in pixels (default 0).
    #[serde(default)]
    pub spread: u16,
    /// Layer opacity (0-255).
    pub alpha: u8,
    /// Optional tint color (hex). Defaults to black.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Semantic elevation/shadow ladder overrides (`[elevation]` table).
///
/// Each level (0..=5) may be overridden with an explicit list of layers.
/// Levels left unset resolve to the built-in
/// [`oasis_types::shadow::Shadow::elevation`] ladder, so an empty table
/// reproduces today's shadow appearance byte-for-byte.
///
/// ```toml
/// [[elevation.level_2]]
/// offset_x = 2
/// offset_y = 4
/// spread = 2
/// alpha = 50
/// color = "#000000"
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct ElevationOverrides {
    /// Level 0 layers (default: no shadow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_0: Option<Vec<ElevationLayerConfig>>,
    /// Level 1 layers (default: built-in subtle shadow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_1: Option<Vec<ElevationLayerConfig>>,
    /// Level 2 layers (default: built-in medium shadow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_2: Option<Vec<ElevationLayerConfig>>,
    /// Level 3 layers (default: built-in prominent shadow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_3: Option<Vec<ElevationLayerConfig>>,
    /// Level 4 layers (default: same as level 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_4: Option<Vec<ElevationLayerConfig>>,
    /// Level 5 layers (default: same as level 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_5: Option<Vec<ElevationLayerConfig>>,
}

impl ElevationOverrides {
    /// Build a runtime [`ElevationLadder`](oasis_types::shadow::ElevationLadder)
    /// from these overrides. Levels without an override fall back to the
    /// built-in ladder, so an empty table yields a default ladder.
    pub fn to_ladder(&self) -> oasis_types::shadow::ElevationLadder {
        use oasis_types::backend::Color;
        use oasis_types::shadow::{ElevationLadder, ShadowLayer};

        let mut ladder = ElevationLadder::default();
        let levels = [
            &self.level_0,
            &self.level_1,
            &self.level_2,
            &self.level_3,
            &self.level_4,
            &self.level_5,
        ];
        for (level, spec) in levels.iter().enumerate() {
            if let Some(layer_defs) = spec {
                let layers = layer_defs
                    .iter()
                    .map(|l| ShadowLayer {
                        offset_x: l.offset_x,
                        offset_y: l.offset_y,
                        spread: l.spread,
                        alpha: l.alpha,
                        color: l
                            .color
                            .as_deref()
                            .and_then(oasis_types::color::parse_hex_color)
                            .unwrap_or(Color::BLACK),
                    })
                    .collect();
                ladder.set_level(level as u8, layers);
            }
        }
        ladder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_parse_defaults_to_comfortable() {
        assert_eq!(Density::parse(None), Density::Comfortable);
        assert_eq!(Density::parse(Some("bogus")), Density::Comfortable);
        assert_eq!(Density::parse(Some("comfortable")), Density::Comfortable);
        assert_eq!(Density::parse(Some("compact")), Density::Compact);
        assert_eq!(Density::parse(Some("spacious")), Density::Spacious);
    }

    #[test]
    fn density_comfortable_is_integer_identity() {
        for base in [0u16, 1, 2, 4, 8, 12, 16, 255] {
            assert_eq!(Density::Comfortable.scale(base), base);
        }
    }

    #[test]
    fn density_compact_and_spacious_scale() {
        // 0.85 / 1.15 rounded to nearest pixel.
        assert_eq!(Density::Compact.scale(8), 7); // 6.8 -> 7
        assert_eq!(Density::Compact.scale(16), 14); // 13.6 -> 14
        assert_eq!(Density::Spacious.scale(8), 9); // 9.2 -> 9
        assert_eq!(Density::Spacious.scale(16), 18); // 18.4 -> 18
    }

    #[test]
    fn elevation_empty_yields_default_ladder() {
        let ov = ElevationOverrides::default();
        assert!(ov.to_ladder().is_default());
    }

    #[test]
    fn elevation_override_builds_ladder() {
        let toml = r##"
[[level_2]]
offset_x = 2
offset_y = 4
spread = 2
alpha = 50
color = "#FF0000"
"##;
        let ov: ElevationOverrides = toml::from_str(toml).unwrap();
        let ladder = ov.to_ladder();
        assert!(!ladder.is_default());
        let s = ladder.resolve(2);
        assert_eq!(s.layers.len(), 1);
        assert_eq!(s.layers[0].alpha, 50);
        assert_eq!(
            s.layers[0].color,
            oasis_types::backend::Color::rgb(255, 0, 0)
        );
        // A non-overridden level still matches the built-in ladder.
        assert_eq!(
            ladder.resolve(1).layers.len(),
            oasis_types::shadow::Shadow::elevation(1).layers.len()
        );
    }
}
