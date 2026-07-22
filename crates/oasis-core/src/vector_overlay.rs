//! Data-driven vector overlay rendering for background decorations.
//!
//! When a skin has `background_layers` configured, this module renders
//! decorative elements (grids, wireframe spheres, radar sweeps, glass shards,
//! scanlines, etc.) directly to the backend via `BackgroundScene`.
//!
//! These overlays are rendered between the base SDI layer (wallpaper, bars)
//! and the overlay layer (cursor, toasts), preserving correct z-order.

use oasis_types::backend::{Color, RenderTargetId, SdiBackend};
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

/// Per-layer op cache for background/chrome vector layers (perf item D4)
/// plus baked offscreen textures for static layers.
///
/// Ops for static layers are tessellated once and replayed each frame;
/// animated layers rebuild as before. On backends with render-target
/// support, each static layer's ops are additionally rasterized once
/// into an offscreen texture and replayed as a single composite per
/// frame instead of re-running every primitive. Owned by the shell (one
/// per layer list) and invalidated on skin swap or resolution change via
/// [`LayerOpsCache::invalidate`]; a size/motion fingerprint also catches
/// stale reuse defensively.
#[derive(Default)]
pub struct LayerOpsCache {
    /// Cached (ops, primitive count) per layer; `None` = animated.
    per_layer: Vec<Option<(Vec<VectorOp>, u32)>>,
    /// (w, h, reduced_motion, layer count) the cache was built for.
    built_for: Option<(u32, u32, bool, usize)>,
    /// Baked offscreen texture per layer, parallel to `per_layer`.
    /// `None` = animated layer, empty ops, or baking unavailable.
    baked: Vec<Option<RenderTargetId>>,
    /// Fingerprint the baked textures were built for (same key as
    /// `built_for`).
    baked_for: Option<(u32, u32, bool, usize)>,
    /// Targets orphaned by [`invalidate`](Self::invalidate) or a
    /// fingerprint change, destroyed at the next render call
    /// (invalidation sites have no backend access).
    stale_targets: Vec<RenderTargetId>,
    /// Set once the backend rejects baking (no render-target support,
    /// or premultiplied composite unavailable) so we stop retrying and
    /// permanently use the immediate-mode path.
    bake_disabled: bool,
}

impl LayerOpsCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all cached ops; the next render rebuilds them.
    ///
    /// Baked textures are retired here and destroyed on the next
    /// render call (the invalidation sites — skin swap, resolution
    /// change — have no backend handle to destroy them with).
    pub fn invalidate(&mut self) {
        self.built_for = None;
        self.per_layer.clear();
        self.retire_baked();
    }

    /// Destroy any baked textures owned by this cache.
    ///
    /// Call before dropping a short-lived cache whose backend outlives
    /// it (e.g. per-screenshot caches); long-lived caches are cleaned
    /// up by the backend's own teardown.
    pub fn release_targets(&mut self, backend: &mut dyn SdiBackend) {
        self.retire_baked();
        self.destroy_stale(backend);
    }

    /// Move all baked textures to the stale list for later destruction.
    fn retire_baked(&mut self) {
        self.stale_targets.extend(self.baked.drain(..).flatten());
        self.baked_for = None;
    }

    /// Destroy retired targets now that a backend is available.
    fn destroy_stale(&mut self, backend: &mut dyn SdiBackend) {
        for id in self.stale_targets.drain(..) {
            // Best-effort: opt-out backends have a no-op destroy.
            let _ = backend.destroy_render_target(id);
        }
    }
}

/// Rasterize a static layer's ops into a fresh offscreen target.
///
/// The target is cleared to transparent so the layer's alpha is
/// preserved. Drawing straight-alpha primitives over transparent black
/// leaves premultiplied pixels, which
/// `composite_render_target_premultiplied` composes back onto the
/// framebuffer exactly like the original immediate-mode draws.
fn bake_layer_ops(
    backend: &mut dyn SdiBackend,
    ops: &[VectorOp],
    w: u32,
    h: u32,
) -> Result<RenderTargetId> {
    let id = backend.create_render_target(w, h)?;
    let result = match backend.bind_render_target(id) {
        Ok(()) => {
            let draw = backend
                .clear(Color::TRANSPARENT)
                .and_then(|()| render_ops(backend, ops, 255));
            // Always pop the bind, then surface the first error.
            let unbind = backend.unbind_render_target();
            draw.and(unbind)
        },
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => Ok(id),
        Err(e) => {
            let _ = backend.destroy_render_target(id);
            Err(e)
        },
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

/// Render a layer list with per-layer caching (D4) and static-layer
/// texture baking.
///
/// Behaviorally identical to `BackgroundScene::build_ops` + `render_scene`:
/// layers render in order, disabled layers are skipped, and any layer whose
/// primitive count exceeds the remaining complexity budget is dropped.
///
/// On backends that support offscreen render targets, each static
/// layer's ops are rasterized once into a transparent-cleared target
/// and replayed as a single premultiplied composite per frame. Animated
/// layers keep the immediate path. If the backend lacks render-target
/// support — or the premultiplied composite fails (some renderers can
/// draw offscreen but not blend premultiplied sources) — baking is
/// disabled and every layer renders through the immediate-mode path,
/// so opt-out backends (WASM/UE5/PSP) degrade instead of breaking.
pub fn render_layers_cached(
    backend: &mut dyn SdiBackend,
    layers: &[BackgroundLayer],
    complexity_budget: u32,
    frame: LayerFrame,
    cache: &mut LayerOpsCache,
) -> Result<()> {
    // Destroy targets orphaned by `invalidate()` (skin swap has no
    // backend access) now that one is on hand. Runs before the empty
    // check so a swap to a layer-less skin still releases textures.
    cache.destroy_stale(backend);
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
    let key = (w, h, reduced_motion, layers.len());

    if cache.built_for != Some(key) {
        cache.retire_baked();
        cache.destroy_stale(backend);
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
        cache.built_for = Some(key);
    }

    // Bake static layers to offscreen textures (once per fingerprint).
    if !cache.bake_disabled && cache.baked_for != Some(key) {
        if backend.supports_render_targets() {
            cache.retire_baked();
            cache.destroy_stale(backend);
            let mut baked: Vec<Option<RenderTargetId>> = Vec::with_capacity(cache.per_layer.len());
            let mut failed = false;
            for entry in &cache.per_layer {
                let target = match entry {
                    Some((ops, _)) if !ops.is_empty() && !failed => {
                        match bake_layer_ops(backend, ops, w, h) {
                            Ok(id) => Some(id),
                            Err(_) => {
                                failed = true;
                                None
                            },
                        }
                    },
                    _ => None,
                };
                baked.push(target);
            }
            if failed {
                // Backend advertises render targets but baking failed
                // (e.g. out of texture memory): release the partial
                // set and stop retrying.
                cache.bake_disabled = true;
                for id in baked.into_iter().flatten() {
                    let _ = backend.destroy_render_target(id);
                }
            } else {
                cache.baked = baked;
                cache.baked_for = Some(key);
            }
        } else {
            cache.bake_disabled = true;
        }
    }

    let mut budget = complexity_budget;
    let mut composite_failed = false;
    for (i, (layer, cached)) in layers.iter().zip(&cache.per_layer).enumerate() {
        if !layer.enabled || budget == 0 {
            continue;
        }
        match cached {
            Some((ops, count)) => {
                if *count > budget {
                    continue;
                }
                budget -= count;
                if !composite_failed && let Some(id) = cache.baked.get(i).copied().flatten() {
                    match backend.composite_render_target_premultiplied(id, 0, 0, w, h) {
                        Ok(()) => continue,
                        // Renderer cannot composite premultiplied
                        // sources: fall through to immediate mode and
                        // disable baking after the loop (the borrow on
                        // `ops` blocks retiring the cache here).
                        Err(_) => composite_failed = true,
                    }
                }
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
    if composite_failed {
        cache.bake_disabled = true;
        cache.retire_baked();
        // Retired textures are destroyed at the start of the next call.
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
        // Render targets are disabled so this exercises the
        // immediate-mode fallback (the baked path is covered below).
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
        rec.disable_render_targets();
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
        assert!(cache.bake_disabled, "unsupported backend disables baking");
        assert!(cache.baked.is_empty());
    }

    #[test]
    fn static_layers_bake_and_composite() {
        use oasis_types::backend::DrawCommand;

        // One static grid + one animated sweep. On a backend with
        // render-target support the static layer bakes once (create /
        // bind / clear / ops / unbind) and every frame replays as a
        // single premultiplied composite; the animated layer keeps
        // drawing immediate-mode primitives.
        let mut sweep = grid_layer();
        sweep.kind = LayerKind::RadarSweep {
            radius: 40,
            sweep_angle: 0.8,
        };
        sweep.animation.rotate_speed = 1.0;
        let layers = vec![grid_layer(), sweep];

        let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
        let mut cache = LayerOpsCache::new();
        let lf = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 7,
        };

        render_layers_cached(&mut rec, &layers, 500, lf, &mut cache).expect("cold render");
        let cold = rec.commands();
        let find = |pat: fn(&DrawCommand) -> bool| cold.iter().position(pat);
        let create = find(|c| matches!(c, DrawCommand::CreateRenderTarget { .. }))
            .expect("static layer creates a target");
        let bind = find(|c| matches!(c, DrawCommand::BindRenderTarget { .. }))
            .expect("bake binds the target");
        // RecordingBackend records `clear` as a full-viewport FillRect.
        let clear = find(|c| {
            matches!(
                c,
                DrawCommand::FillRect {
                    x: 0,
                    y: 0,
                    color: Color::TRANSPARENT,
                    ..
                }
            )
        })
        .expect("bake clears the target to transparent");
        let unbind =
            find(|c| matches!(c, DrawCommand::UnbindRenderTarget)).expect("bake pops the bind");
        let composite =
            find(|c| matches!(c, DrawCommand::CompositeRenderTargetPremultiplied { .. }))
                .expect("baked layer composites back");
        assert!(create < bind && bind < clear && clear < unbind && unbind < composite);
        assert_eq!(
            rec.live_render_target_count(),
            1,
            "one target for one static layer"
        );
        assert_eq!(rec.render_target_bind_depth(), 0);

        // Warm frame: composite only — no target churn, no re-baking,
        // no immediate-mode ops for the static layer.
        rec.clear_commands();
        render_layers_cached(&mut rec, &layers, 500, lf, &mut cache).expect("warm render");
        let warm = rec.commands();
        assert!(
            warm.iter()
                .any(|c| matches!(c, DrawCommand::CompositeRenderTargetPremultiplied { .. })),
            "warm frame replays the baked texture"
        );
        assert!(
            !warm
                .iter()
                .any(|c| matches!(c, DrawCommand::CreateRenderTarget { .. })),
            "warm frame must not re-bake"
        );
        assert!(
            warm.iter()
                .any(|c| !matches!(c, DrawCommand::CompositeRenderTargetPremultiplied { .. })),
            "animated layer still draws immediate-mode"
        );
        assert_eq!(rec.live_render_target_count(), 1);
    }

    #[test]
    fn invalidate_destroys_baked_targets_on_next_render() {
        use oasis_types::backend::DrawCommand;

        let layers = vec![grid_layer()];
        let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
        let mut cache = LayerOpsCache::new();
        let lf = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 0,
        };

        render_layers_cached(&mut rec, &layers, 500, lf, &mut cache).expect("cold render");
        assert_eq!(rec.live_render_target_count(), 1);

        // Skin swap: retire now, destroy + re-bake at the next render.
        cache.invalidate();
        assert_eq!(cache.stale_targets.len(), 1);
        rec.clear_commands();
        render_layers_cached(&mut rec, &layers, 500, lf, &mut cache).expect("rebake render");
        assert!(
            rec.commands()
                .iter()
                .any(|c| matches!(c, DrawCommand::DestroyRenderTarget { .. })),
            "stale target destroyed on next render"
        );
        assert_eq!(
            rec.live_render_target_count(),
            1,
            "old target destroyed, fresh one baked"
        );

        // Explicit release for short-lived caches.
        cache.release_targets(&mut rec);
        assert_eq!(rec.live_render_target_count(), 0);
    }

    #[test]
    fn resize_rebakes_at_new_dimensions() {
        use oasis_types::backend::DrawCommand;

        let layers = vec![grid_layer()];
        let mut rec = oasis_test_backend::RecordingBackend::new(1024, 768);
        let mut cache = LayerOpsCache::new();
        let small = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 0,
        };
        let big = LayerFrame {
            w: 1024,
            h: 768,
            reduced_motion: false,
            frame_counter: 0,
        };

        render_layers_cached(&mut rec, &layers, 500, small, &mut cache).expect("small render");
        rec.clear_commands();
        render_layers_cached(&mut rec, &layers, 500, big, &mut cache).expect("big render");
        let created: Vec<(u32, u32)> = rec
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::CreateRenderTarget { w, h, .. } => Some((*w, *h)),
                _ => None,
            })
            .collect();
        assert_eq!(created, vec![(1024, 768)], "rebaked at the new size");
        assert_eq!(rec.live_render_target_count(), 1, "old target destroyed");
    }

    #[test]
    fn over_budget_static_layer_is_not_composited() {
        use oasis_types::backend::DrawCommand;

        // A dense grid whose primitive count exceeds the budget must be
        // dropped entirely — no composite, matching the uncached path.
        let mut dense = grid_layer();
        dense.kind = LayerKind::Grid { spacing: 10 };
        let layers = vec![dense];
        let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
        let mut cache = LayerOpsCache::new();
        let lf = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 0,
        };
        render_layers_cached(&mut rec, &layers, 5, lf, &mut cache).expect("render");
        assert!(
            !rec.commands()
                .iter()
                .any(|c| matches!(c, DrawCommand::CompositeRenderTargetPremultiplied { .. })),
            "over-budget layer must not composite"
        );
    }

    #[test]
    fn mock_backend_without_targets_falls_back() {
        // MockSdiCore keeps the default `SdiRenderTarget` impl
        // (`supports_render_targets` = false): rendering must succeed
        // through the immediate path with baking disabled.
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let layers = vec![grid_layer()];
        let mut cache = LayerOpsCache::new();
        let lf = LayerFrame {
            w: 480,
            h: 272,
            reduced_motion: false,
            frame_counter: 0,
        };
        render_layers_cached(&mut backend, &layers, 500, lf, &mut cache).expect("render");
        render_layers_cached(&mut backend, &layers, 500, lf, &mut cache).expect("warm render");
        assert!(cache.bake_disabled);
        assert!(cache.baked.is_empty());
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
