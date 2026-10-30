//! Background / chrome / image layer derivation: `[[background_layers]]` and
//! `[[chrome_layers]]` TOML entries to `oasis-vector` layers and image decals.

use oasis_types::backend::Color;

use crate::SkinTheme;
use crate::theme::parse_hex_color;

use super::derive::{LAYER_COLOR_DEFAULT, ov};
use super::{ActiveTheme, ImageLayerTheme};

impl ActiveTheme {
    /// Derive image decal layers (`kind = "image"`) from skin configuration.
    ///
    /// Image layers are not vector layers -- they carry an asset key that the
    /// shell resolves against `Skin::assets` and uploads as a texture, so
    /// they live in a separate list from `background_layers`.
    pub(super) fn derive_image_layers(skin: &SkinTheme) -> Vec<ImageLayerTheme> {
        skin.background_layers
            .as_ref()
            .map(|layers| {
                layers
                    .iter()
                    .filter(|cfg| cfg.kind == "image")
                    .filter_map(|cfg| {
                        let source = cfg.source.clone()?;
                        Some(ImageLayerTheme {
                            source,
                            position: convert_layer_position(cfg),
                            animation: convert_layer_animation(cfg),
                            alpha: cfg.alpha.unwrap_or(255),
                            enabled: cfg.enabled.unwrap_or(true),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Derive background layers from skin configuration.
    pub(super) fn derive_background_layers(skin: &SkinTheme) -> Vec<oasis_vector::BackgroundLayer> {
        Self::derive_layer_list(skin, skin.background_layers.as_ref())
    }

    /// Derive chrome layers (overlay-pass vector decorations) from skin
    /// configuration. Same conversion as background layers; `"image"` and
    /// `"shader"` kinds fall out naturally (the converter drops "image" and
    /// the overlay renderer emits no ops for "shader").
    pub(super) fn derive_chrome_layers(skin: &SkinTheme) -> Vec<oasis_vector::BackgroundLayer> {
        Self::derive_layer_list(skin, skin.chrome_layers.as_ref())
    }

    /// Convert a TOML layer list to runtime layers, honoring the
    /// `background_performance.max_layers` cap.
    pub(super) fn derive_layer_list(
        skin: &SkinTheme,
        layers: Option<&Vec<crate::theme::BackgroundLayerConfig>>,
    ) -> Vec<oasis_vector::BackgroundLayer> {
        let bg_perf = skin.background_performance.as_ref();
        let bg_max_layers = bg_perf.and_then(|p| p.max_layers).unwrap_or(8);
        let default_color = ov(
            bg_perf.and_then(|p| p.default_layer_color.as_ref()),
            LAYER_COLOR_DEFAULT,
        );

        layers
            .map(|layers| {
                layers
                    .iter()
                    .take(bg_max_layers as usize)
                    .filter_map(|cfg| Self::convert_background_layer(cfg, default_color, &ov))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Convert a TOML background layer config to a runtime `BackgroundLayer`.
    fn convert_background_layer(
        cfg: &crate::theme::BackgroundLayerConfig,
        default_color: Color,
        ov_fn: &dyn Fn(Option<&String>, Color) -> Color,
    ) -> Option<oasis_vector::BackgroundLayer> {
        use oasis_vector::background::{BackgroundLayer, LayerKind};

        let color = ov_fn(cfg.color.as_ref(), default_color);

        let kind = match cfg.kind.as_str() {
            "grid" => LayerKind::Grid {
                spacing: cfg.spacing.unwrap_or(30),
            },
            "dot_grid" => LayerKind::DotGrid {
                spacing: cfg.spacing.unwrap_or(20),
                radius: cfg.dot_radius.unwrap_or(1),
            },
            "wireframe_sphere" => LayerKind::WireframeSphere {
                radius: cfg.radius.unwrap_or(60),
            },
            "radar_sweep" => LayerKind::RadarSweep {
                radius: cfg.radius.unwrap_or(65),
                sweep_angle: cfg.sweep_angle.unwrap_or(0.8),
            },
            "concentric_rings" => LayerKind::ConcentricRings {
                count: cfg.count.unwrap_or(3),
                radius: cfg.radius.unwrap_or(60),
                stroke_width: cfg.stroke_width.unwrap_or(1),
            },
            "glass_shard" => {
                let pts: Vec<(f32, f32)> = cfg
                    .points
                    .as_ref()
                    .map(|p| p.iter().map(|a| (a[0], a[1])).collect())
                    .unwrap_or_default();
                if pts.is_empty() {
                    return None;
                }
                LayerKind::GlassShard { points: pts }
            },
            "scanlines" => LayerKind::Scanlines {
                spacing: cfg.spacing.unwrap_or(2) as u16,
            },
            "eq_bars" => LayerKind::EqBars {
                count: cfg.count.unwrap_or(5),
                bar_width: cfg.bar_width.unwrap_or(8),
                max_height: cfg.max_height.unwrap_or(30),
            },
            "crosshair" => LayerKind::Crosshair {
                size: cfg.size.unwrap_or(20),
            },
            "floating_polygons" => LayerKind::FloatingPolygons {
                count: cfg.count.unwrap_or(3),
                sides: cfg.sides.unwrap_or(4),
            },
            "pulsing_core" => LayerKind::PulsingCore {
                radius: cfg.radius.unwrap_or(10),
            },
            "waves" => LayerKind::Waves {
                rows: cfg.count.unwrap_or(16),
                amplitude: cfg.max_height.unwrap_or(20) as u16,
                frequency: cfg.frequency.unwrap_or(3.0),
            },
            "shader" => {
                let name = cfg.shader.clone().unwrap_or_else(|| "balatro".to_string());
                let params = parse_shader_params(cfg);
                LayerKind::Shader { name, params }
            },
            _ => return None,
        };

        Some(BackgroundLayer {
            kind,
            color,
            position: convert_layer_position(cfg),
            animation: convert_layer_animation(cfg),
            enabled: cfg.enabled.unwrap_or(true),
        })
    }
}

/// Convert a layer's position sub-table to the runtime type.
fn convert_layer_position(
    cfg: &crate::theme::BackgroundLayerConfig,
) -> oasis_vector::background::LayerPosition {
    use oasis_vector::background::{Anchor, LayerPosition};
    cfg.position
        .as_ref()
        .map_or_else(LayerPosition::default, |p| {
            let anchor = match p.anchor.as_deref().unwrap_or("center") {
                "top_left" => Anchor::TopLeft,
                "top_center" => Anchor::TopCenter,
                "top_right" => Anchor::TopRight,
                "center_left" => Anchor::CenterLeft,
                "center_right" => Anchor::CenterRight,
                "bottom_left" => Anchor::BottomLeft,
                "bottom_center" => Anchor::BottomCenter,
                "bottom_right" => Anchor::BottomRight,
                _ => Anchor::Center,
            };
            LayerPosition {
                anchor,
                offset_x: p.offset_x.unwrap_or(0.0),
                offset_y: p.offset_y.unwrap_or(0.0),
            }
        })
}

/// Convert a layer's animation sub-table to the runtime type.
fn convert_layer_animation(
    cfg: &crate::theme::BackgroundLayerConfig,
) -> oasis_vector::background::LayerAnimation {
    use oasis_vector::background::LayerAnimation;
    cfg.animation
        .as_ref()
        .map_or_else(LayerAnimation::default, |a| LayerAnimation {
            rotate_speed: a.rotate_speed.unwrap_or(0.0),
            pulse_speed: a.pulse_speed.unwrap_or(0.0),
            pulse_min_alpha: a.pulse_min_alpha.unwrap_or(0.5),
            drift_x: a.drift_x.unwrap_or(0.0),
            drift_y: a.drift_y.unwrap_or(0.0),
            phase_offset: a.phase_offset.unwrap_or(0.0),
        })
}

/// Parse shader-specific parameters from a `BackgroundLayerConfig`.
fn parse_shader_params(cfg: &crate::theme::BackgroundLayerConfig) -> oasis_vector::ShaderParams {
    let mut params = oasis_vector::ShaderParams::default();

    if let Some(ref table) = cfg.shader_params {
        // Extract color_1, color_2, color_3 as hex -> [f32; 4].
        for key in &["color_1", "color_2", "color_3"] {
            if let Some(toml::Value::String(hex)) = table.get(*key)
                && let Some(c) = hex_to_f32_color(hex)
            {
                params.colors.push(c);
            }
        }

        // Extract named float parameters.
        for (key, value) in table {
            if key.starts_with("color_") {
                continue;
            }
            match value {
                toml::Value::Float(f) => {
                    params.floats.insert(key.clone(), *f as f32);
                },
                toml::Value::Integer(i) => {
                    params.floats.insert(key.clone(), *i as f32);
                },
                toml::Value::Boolean(b) => {
                    params
                        .floats
                        .insert(key.clone(), if *b { 1.0 } else { 0.0 });
                },
                _ => {},
            }
        }
    }

    params
}

/// Parse a hex color string (e.g. "#DE4440") to `[f32; 4]` (0.0-1.0 RGBA).
fn hex_to_f32_color(hex: &str) -> Option<[f32; 4]> {
    let c = parse_hex_color(hex)?;
    Some([
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    ])
}
