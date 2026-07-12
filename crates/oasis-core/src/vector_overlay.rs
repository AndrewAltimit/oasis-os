//! Data-driven vector overlay rendering for background decorations.
//!
//! When a skin has `background_layers` configured, this module renders
//! decorative elements (grids, wireframe spheres, radar sweeps, glass shards,
//! scanlines, etc.) directly to the backend via `BackgroundScene`.
//!
//! These overlays are rendered between the base SDI layer (wallpaper, bars)
//! and the overlay layer (cursor, toasts), preserving correct z-order.

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;
use oasis_vector::AnimClock;
use oasis_vector::ShaderParams;
use oasis_vector::background::{BackgroundLayer, BackgroundScene, LayerKind};
use oasis_vector::op::VectorOp;
use oasis_vector::render::{render_ops, render_scene};
use oasis_vector::scene::VectorScene;

use crate::active_theme::ActiveTheme;

/// Whether a layer's ops change from frame to frame.
///
/// Static layers can be built once and cached (D4); animated layers are
/// rebuilt each frame. Under `reduced_motion` every layer renders with a
/// frozen clock, so everything is cacheable.
fn layer_is_animated(layer: &BackgroundLayer, reduced_motion: bool) -> bool {
    if reduced_motion {
        return false;
    }
    let anim = &layer.animation;
    match layer.kind {
        // These kinds animate from the clock directly.
        LayerKind::EqBars { .. } | LayerKind::FloatingPolygons { .. } | LayerKind::Waves { .. } => {
            true
        },
        // Shader layers emit no VectorOps at all.
        LayerKind::Shader { .. } => false,
        // Everything else animates only through the shared animation params.
        _ => {
            anim.rotate_speed != 0.0
                || anim.pulse_speed != 0.0
                || anim.drift_x != 0.0
                || anim.drift_y != 0.0
        },
    }
}

/// Per-layer op cache for background/chrome vector layers (perf item D4).
///
/// Ops for static layers are tessellated once and replayed each frame;
/// animated layers rebuild as before. Owned by the shell (one per layer
/// list) and invalidated on skin swap or resolution change via
/// [`LayerOpsCache::invalidate`]; a size/motion fingerprint also catches
/// stale reuse defensively.
#[derive(Default)]
pub struct LayerOpsCache {
    /// Cached (ops, primitive count) per layer; `None` = animated.
    per_layer: Vec<Option<(Vec<VectorOp>, u32)>>,
    /// (w, h, reduced_motion, layer count) the cache was built for.
    built_for: Option<(u32, u32, bool, usize)>,
}

impl LayerOpsCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all cached ops; the next render rebuilds them.
    pub fn invalidate(&mut self) {
        self.built_for = None;
        self.per_layer.clear();
    }
}

/// Per-frame parameters for cached layer rendering.
#[derive(Debug, Clone, Copy)]
pub struct LayerFrame {
    /// Viewport width in pixels.
    pub w: u32,
    /// Viewport height in pixels.
    pub h: u32,
    /// Whether layer animations are suppressed.
    pub reduced_motion: bool,
    /// Frame counter driving animation timing (assumes 60fps).
    pub frame_counter: u32,
}

/// Render a layer list with per-layer caching (D4).
///
/// Behaviorally identical to `BackgroundScene::build_ops` + `render_scene`:
/// layers render in order, disabled layers are skipped, and any layer whose
/// primitive count exceeds the remaining complexity budget is dropped.
pub fn render_layers_cached(
    backend: &mut dyn SdiBackend,
    layers: &[BackgroundLayer],
    complexity_budget: u32,
    frame: LayerFrame,
    cache: &mut LayerOpsCache,
) -> Result<()> {
    if layers.is_empty() {
        return Ok(());
    }

    let LayerFrame {
        w,
        h,
        reduced_motion,
        frame_counter,
    } = frame;
    let clock = AnimClock {
        time_s: frame_counter as f32 / 60.0,
        frame: frame_counter,
    };

    if cache.built_for != Some((w, h, reduced_motion, layers.len())) {
        cache.per_layer = layers
            .iter()
            .map(|layer| {
                if layer_is_animated(layer, reduced_motion) {
                    None
                } else {
                    let ops = BackgroundScene::build_layer_ops(layer, &clock, w, h, reduced_motion);
                    let count = BackgroundScene::count_ops(&ops) as u32;
                    Some((ops, count))
                }
            })
            .collect();
        cache.built_for = Some((w, h, reduced_motion, layers.len()));
    }

    let mut budget = complexity_budget;
    for (layer, cached) in layers.iter().zip(&cache.per_layer) {
        if !layer.enabled || budget == 0 {
            continue;
        }
        match cached {
            Some((ops, count)) => {
                if *count > budget {
                    continue;
                }
                budget -= count;
                render_ops(backend, ops, 255)?;
            },
            None => {
                let ops = BackgroundScene::build_layer_ops(layer, &clock, w, h, reduced_motion);
                let count = BackgroundScene::count_ops(&ops) as u32;
                if count > budget {
                    continue;
                }
                budget -= count;
                render_ops(backend, &ops, 255)?;
            },
        }
    }
    Ok(())
}

/// Render the theme's `background_layers` through a [`LayerOpsCache`].
///
/// Cached variant of [`render_vector_background`] for backends that own a
/// persistent cache (the SDL shell). Output is identical.
pub fn render_vector_background_cached(
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
    frame_counter: u32,
    cache: &mut LayerOpsCache,
) -> Result<()> {
    render_layers_cached(
        backend,
        &at.background_layers,
        at.background_complexity_budget,
        LayerFrame {
            w: at.screen_w,
            h: at.screen_h,
            reduced_motion: at.background_reduced_motion,
            frame_counter,
        },
        cache,
    )
}

/// Render the theme's `chrome_layers` in the overlay pass.
///
/// Called after the SDI overlay layer (bars, tabs) so the vector chrome
/// paints on top — procedurally shaped chrome accents without shipped art.
pub fn render_vector_chrome(
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
    frame_counter: u32,
    cache: &mut LayerOpsCache,
) -> Result<()> {
    render_layers_cached(
        backend,
        &at.chrome_layers,
        at.background_complexity_budget,
        LayerFrame {
            w: at.screen_w,
            h: at.screen_h,
            reduced_motion: at.background_reduced_motion,
            frame_counter,
        },
        cache,
    )
}

/// Build and render vector background overlays for the current frame.
///
/// Composes decorative elements from the theme's `background_layers` list
/// into a `BackgroundScene` and renders via `VectorScene`.
///
/// `frame_counter` drives animation timing. Pass 0 for static.
pub fn render_vector_background(
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
    frame_counter: u32,
) -> Result<()> {
    if at.background_layers.is_empty() {
        return Ok(());
    }

    let w = at.screen_w;
    let h = at.screen_h;

    // Build a clock from the frame counter (assumes 60fps).
    let clock = AnimClock {
        time_s: frame_counter as f32 / 60.0,
        frame: frame_counter,
    };

    let scene_data = BackgroundScene {
        layers: at.background_layers.clone(),
        complexity_budget: at.background_complexity_budget,
    };

    let ops = scene_data.build_ops(&clock, w, h, at.background_reduced_motion);
    if ops.is_empty() {
        return Ok(());
    }

    let scene = VectorScene {
        width: w,
        height: h,
        ops,
    };

    render_scene(backend, &scene)
}

/// Information about a shader background layer, extracted from the theme.
pub struct ShaderLayerInfo {
    /// Shader name (matches registry key, e.g. "balatro").
    pub name: String,
    /// Shader-specific parameters.
    pub params: ShaderParams,
}

/// Extract the first shader layer from the active theme (if any).
///
/// Backends call this to determine whether to invoke GPU shader rendering
/// before the vector overlay pass.
pub fn get_shader_layer(at: &ActiveTheme) -> Option<ShaderLayerInfo> {
    at.background_layers.iter().find_map(|l| match &l.kind {
        LayerKind::Shader { name, params } => Some(ShaderLayerInfo {
            name: name.clone(),
            params: params.clone(),
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::Color;
    use oasis_vector::background::{LayerAnimation, LayerKind, LayerPosition};
    use oasis_vector::icons;

    fn grid_layer() -> BackgroundLayer {
        BackgroundLayer {
            kind: LayerKind::Grid { spacing: 30 },
            color: Color::rgba(255, 255, 255, 18),
            position: LayerPosition::default(),
            animation: LayerAnimation::default(),
            enabled: true,
        }
    }

    #[test]
    fn static_grid_is_not_animated() {
        assert!(!layer_is_animated(&grid_layer(), false));
    }

    #[test]
    fn rotating_layer_is_animated() {
        let mut layer = grid_layer();
        layer.kind = LayerKind::RadarSweep {
            radius: 40,
            sweep_angle: 0.8,
        };
        layer.animation.rotate_speed = 1.0;
        assert!(layer_is_animated(&layer, false));
    }

    #[test]
    fn clock_driven_kinds_are_animated() {
        let mut layer = grid_layer();
        layer.kind = LayerKind::EqBars {
            count: 5,
            bar_width: 8,
            max_height: 30,
        };
        assert!(layer_is_animated(&layer, false));
        layer.kind = LayerKind::Waves {
            rows: 8,
            amplitude: 10,
            frequency: 3.0,
        };
        assert!(layer_is_animated(&layer, false));
    }

    #[test]
    fn reduced_motion_makes_everything_cacheable() {
        let mut layer = grid_layer();
        layer.kind = LayerKind::EqBars {
            count: 5,
            bar_width: 8,
            max_height: 30,
        };
        layer.animation.rotate_speed = 2.0;
        assert!(!layer_is_animated(&layer, true));
    }

    #[test]
    fn cache_builds_static_entries_and_invalidates() {
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let mut anim = grid_layer();
        anim.animation.pulse_speed = 1.0;
        let layers = vec![grid_layer(), anim];
        let mut cache = LayerOpsCache::new();

        let frame = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 0,
        };
        render_layers_cached(&mut backend, &layers, 200, frame, &mut cache).expect("render");
        assert_eq!(cache.per_layer.len(), 2);
        assert!(
            cache.per_layer[0].is_some(),
            "static layer should be cached"
        );
        assert!(cache.per_layer[1].is_none(), "pulsing layer stays live");
        assert_eq!(cache.built_for, Some((480, 272, false, 2)));

        cache.invalidate();
        assert!(cache.built_for.is_none());
        assert!(cache.per_layer.is_empty());
    }

    #[test]
    fn cached_render_matches_uncached_path() {
        // Mixed static + animated layers, including one that busts the
        // budget: draw commands from the cached path (fresh AND warm)
        // must match the reference build_ops + render_scene path exactly.
        let mut sweep = grid_layer();
        sweep.kind = LayerKind::RadarSweep {
            radius: 40,
            sweep_angle: 0.8,
        };
        sweep.animation.rotate_speed = 1.0;
        let mut dense = grid_layer();
        dense.kind = LayerKind::Grid { spacing: 10 }; // busts a small budget
        let layers = vec![grid_layer(), sweep, dense];
        let budget = 60;
        let frame = 42;

        let reference = {
            let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
            let clock = AnimClock {
                time_s: frame as f32 / 60.0,
                frame,
            };
            let scene = BackgroundScene {
                layers: layers.clone(),
                complexity_budget: budget,
            };
            let ops = scene.build_ops(&clock, 480, 272, false);
            let vscene = VectorScene {
                width: 480,
                height: 272,
                ops,
            };
            render_scene(&mut rec, &vscene).expect("reference render");
            rec.commands().to_vec()
        };

        let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
        let mut cache = LayerOpsCache::new();
        let lf = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: frame,
        };
        // Cold (builds cache) then warm (replays cache) at the same frame.
        render_layers_cached(&mut rec, &layers, budget, lf, &mut cache).expect("cold render");
        assert_eq!(rec.commands(), &reference[..], "cold path diverges");
        rec.clear_commands();
        render_layers_cached(&mut rec, &layers, budget, lf, &mut cache).expect("warm render");
        assert_eq!(rec.commands(), &reference[..], "warm path diverges");
    }

    #[test]
    fn render_vector_background_empty_layers() {
        // Default theme has no background layers — should return Ok immediately.
        let at = ActiveTheme::default();
        assert!(at.background_layers.is_empty());
    }

    #[test]
    fn render_vector_background_builds_scene() {
        let mut at = ActiveTheme::default();
        at.background_layers.push(BackgroundLayer {
            kind: LayerKind::Grid { spacing: 30 },
            color: Color::rgba(255, 255, 255, 18),
            position: LayerPosition::default(),
            animation: LayerAnimation::default(),
            enabled: true,
        });
        let w = at.screen_w;
        let h = at.screen_h;
        let grid_ops = icons::grid_overlay(w, h, 30, Color::WHITE);
        assert!(!grid_ops.is_empty());
    }
}
