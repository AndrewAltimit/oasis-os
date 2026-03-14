//! Data-driven background layer system.
//!
//! [`BackgroundLayer`] describes a single decorative background element
//! (grids, spheres, radar sweeps, etc.) and [`BackgroundScene`] composes
//! them into a renderable set of [`VectorOp`]s driven by an [`AnimClock`].
//!
//! Skins configure layers via `[[background_layers]]` TOML sections.

use oasis_types::backend::Color;
use oasis_types::shader::ShaderParams;

use crate::anim::AnimClock;
use crate::backgrounds;
use crate::op::VectorOp;

/// A single background decoration layer.
#[derive(Debug, Clone)]
pub struct BackgroundLayer {
    /// What kind of element to draw.
    pub kind: LayerKind,
    /// Element color (typically white at low alpha).
    pub color: Color,
    /// Positioning within the viewport.
    pub position: LayerPosition,
    /// Animation parameters.
    pub animation: LayerAnimation,
    /// Whether this layer is active.
    pub enabled: bool,
}

/// The type of background decoration.
#[derive(Debug, Clone)]
pub enum LayerKind {
    /// Thin grid lines at regular spacing.
    Grid { spacing: u32 },
    /// Dot pattern at regular spacing.
    DotGrid { spacing: u32, radius: u16 },
    /// Wireframe globe with cross lines.
    WireframeSphere { radius: u16 },
    /// Rotating pie-wedge sweep.
    RadarSweep { radius: u16, sweep_angle: f32 },
    /// Concentric stroke circles.
    ConcentricRings {
        count: u8,
        radius: u16,
        stroke_width: u16,
    },
    /// Translucent polygon shard (normalized 0..1 coordinates).
    GlassShard { points: Vec<(f32, f32)> },
    /// Horizontal scan lines.
    Scanlines { spacing: u16 },
    /// Audio equalizer bars.
    EqBars {
        count: u8,
        bar_width: u32,
        max_height: u32,
    },
    /// Crosshair reticle.
    Crosshair { size: u16 },
    /// Drifting polygon shapes.
    FloatingPolygons { count: u8, sides: u8 },
    /// Pulsing circle at center.
    PulsingCore { radius: u16 },
    /// Undulating wave rows across the full screen (Vanta Waves style).
    Waves {
        rows: u8,
        amplitude: u16,
        frequency: f32,
    },
    /// GPU fragment shader (rendered by backend, not as VectorOps).
    ///
    /// Shader layers are skipped by `build_ops` and handled directly by each
    /// backend's render loop via `oasis-shader`.
    Shader { name: String, params: ShaderParams },
}

/// Where a layer is anchored within the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    /// Resolve an anchor point to pixel coordinates within a viewport.
    pub fn resolve(self, w: u32, h: u32) -> (i32, i32) {
        let hw = w as i32 / 2;
        let hh = h as i32 / 2;
        match self {
            Self::TopLeft => (0, 0),
            Self::TopCenter => (hw, 0),
            Self::TopRight => (w as i32, 0),
            Self::CenterLeft => (0, hh),
            Self::Center => (hw, hh),
            Self::CenterRight => (w as i32, hh),
            Self::BottomLeft => (0, h as i32),
            Self::BottomCenter => (hw, h as i32),
            Self::BottomRight => (w as i32, h as i32),
        }
    }
}

/// Positioning for a background layer.
#[derive(Debug, Clone)]
pub struct LayerPosition {
    /// Which screen edge/corner to anchor to.
    pub anchor: Anchor,
    /// Horizontal offset as fraction of screen width.
    pub offset_x: f32,
    /// Vertical offset as fraction of screen height.
    pub offset_y: f32,
}

impl Default for LayerPosition {
    fn default() -> Self {
        Self {
            anchor: Anchor::Center,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

/// Animation parameters for a background layer.
#[derive(Debug, Clone)]
pub struct LayerAnimation {
    /// Rotation speed in radians per second (0 = static).
    pub rotate_speed: f32,
    /// Pulse frequency in Hz (0 = no pulse).
    pub pulse_speed: f32,
    /// Minimum alpha when pulsing (0..1).
    pub pulse_min_alpha: f32,
    /// Horizontal drift in pixels per second.
    pub drift_x: f32,
    /// Vertical drift in pixels per second.
    pub drift_y: f32,
    /// Phase offset for staggering multiple instances.
    pub phase_offset: f32,
}

impl Default for LayerAnimation {
    fn default() -> Self {
        Self {
            rotate_speed: 0.0,
            pulse_speed: 0.0,
            pulse_min_alpha: 0.5,
            drift_x: 0.0,
            drift_y: 0.0,
            phase_offset: 0.0,
        }
    }
}

/// A collection of background layers composable into a renderable scene.
#[derive(Debug, Clone)]
pub struct BackgroundScene {
    /// Layers in paint order (back to front).
    pub layers: Vec<BackgroundLayer>,
    /// Maximum number of VectorOps to emit (performance budget).
    pub complexity_budget: u32,
}

impl BackgroundScene {
    /// Build `VectorOp`s for all enabled layers at the current clock time.
    ///
    /// Stops emitting ops once `complexity_budget` is reached.
    pub fn build_ops(
        &self,
        clock: &AnimClock,
        w: u32,
        h: u32,
        reduced_motion: bool,
    ) -> Vec<VectorOp> {
        let mut ops = Vec::new();
        let mut budget = self.complexity_budget;

        for layer in &self.layers {
            if !layer.enabled || budget == 0 {
                continue;
            }

            let mut layer_ops = Self::build_layer_ops(layer, clock, w, h, reduced_motion);
            let count = Self::count_ops(&layer_ops) as u32;
            if count > budget {
                // Skip this layer if it exceeds remaining budget.
                continue;
            }
            budget = budget.saturating_sub(count);
            ops.append(&mut layer_ops);
        }

        ops
    }

    /// Count the number of primitive ops (recursing into groups).
    fn count_ops(ops: &[VectorOp]) -> usize {
        let mut count = 0;
        for op in ops {
            match op {
                VectorOp::Group { ops: inner, .. } => count += Self::count_ops(inner),
                _ => count += 1,
            }
        }
        count
    }

    /// Build ops for a single layer.
    fn build_layer_ops(
        layer: &BackgroundLayer,
        clock: &AnimClock,
        w: u32,
        h: u32,
        reduced_motion: bool,
    ) -> Vec<VectorOp> {
        let anim = &layer.animation;
        let rotation = if reduced_motion || anim.rotate_speed == 0.0 {
            0.0
        } else {
            clock.time_s * anim.rotate_speed + anim.phase_offset
        };

        let pulse_alpha = if reduced_motion || anim.pulse_speed == 0.0 {
            1.0
        } else {
            let norm = clock.sine_norm(anim.pulse_speed, anim.phase_offset);
            anim.pulse_min_alpha + norm * (1.0 - anim.pulse_min_alpha)
        };

        let (ax, ay) = layer.position.anchor.resolve(w, h);
        let cx = ax + (layer.position.offset_x * w as f32) as i32;
        let cy = ay + (layer.position.offset_y * h as f32) as i32;

        let mut color = layer.color;
        color.a = ((color.a as f32 * pulse_alpha) as u8).max(1);

        match &layer.kind {
            LayerKind::Grid { spacing } => backgrounds::grid(w, h, *spacing, color),
            LayerKind::DotGrid { spacing, radius } => {
                backgrounds::dot_grid(w, h, *spacing, *radius, color)
            },
            LayerKind::WireframeSphere { radius } => {
                backgrounds::wireframe_sphere(cx, cy, *radius, color, rotation)
            },
            LayerKind::RadarSweep {
                radius,
                sweep_angle,
            } => backgrounds::radar_sweep(cx, cy, *radius, *sweep_angle, rotation, color),
            LayerKind::ConcentricRings {
                count,
                radius,
                stroke_width,
            } => backgrounds::concentric_rings(cx, cy, *count, *radius, *stroke_width, color),
            LayerKind::GlassShard { points } => {
                if w == 0 || h == 0 {
                    return Vec::new();
                }
                let drift_x = if reduced_motion {
                    0.0
                } else {
                    clock.time_s * anim.drift_x
                };
                let drift_y = if reduced_motion {
                    0.0
                } else {
                    clock.time_s * anim.drift_y
                };
                backgrounds::glass_shard(points, w, h, color, drift_x, drift_y)
            },
            LayerKind::Scanlines { spacing } => backgrounds::scanlines(w, h, *spacing, color),
            LayerKind::EqBars {
                count,
                bar_width,
                max_height,
            } => backgrounds::eq_visualizer(
                cx,
                cy,
                *count,
                *bar_width,
                *max_height,
                color,
                clock,
                reduced_motion,
            ),
            LayerKind::Crosshair { size } => backgrounds::crosshair(cx, cy, *size, color),
            LayerKind::FloatingPolygons { count, sides } => {
                if reduced_motion {
                    Vec::new()
                } else {
                    backgrounds::floating_polygons(
                        w,
                        h,
                        *count,
                        *sides,
                        color,
                        clock,
                        anim.drift_x,
                        anim.drift_y,
                        anim.phase_offset,
                    )
                }
            },
            LayerKind::PulsingCore { radius } => backgrounds::pulsing_core(cx, cy, *radius, color),
            LayerKind::Waves {
                rows,
                amplitude,
                frequency,
            } => backgrounds::waves(
                w,
                h,
                *rows,
                *amplitude,
                *frequency,
                anim.rotate_speed,
                color,
                clock,
                reduced_motion,
            ),
            // Shader layers produce no VectorOps — rendered by each backend directly.
            LayerKind::Shader { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scene_produces_no_ops() {
        let scene = BackgroundScene {
            layers: vec![],
            complexity_budget: 200,
        };
        let clock = AnimClock::new();
        let ops = scene.build_ops(&clock, 480, 272, false);
        assert!(ops.is_empty());
    }

    #[test]
    fn disabled_layer_skipped() {
        let scene = BackgroundScene {
            layers: vec![BackgroundLayer {
                kind: LayerKind::Grid { spacing: 30 },
                color: Color::rgba(255, 255, 255, 18),
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: false,
            }],
            complexity_budget: 200,
        };
        let clock = AnimClock::new();
        let ops = scene.build_ops(&clock, 480, 272, false);
        assert!(ops.is_empty());
    }

    #[test]
    fn grid_layer_produces_ops() {
        let scene = BackgroundScene {
            layers: vec![BackgroundLayer {
                kind: LayerKind::Grid { spacing: 30 },
                color: Color::rgba(255, 255, 255, 18),
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            }],
            complexity_budget: 200,
        };
        let clock = AnimClock::new();
        let ops = scene.build_ops(&clock, 480, 272, false);
        assert!(!ops.is_empty());
    }

    #[test]
    fn budget_limits_output() {
        let scene = BackgroundScene {
            layers: vec![BackgroundLayer {
                kind: LayerKind::Grid { spacing: 10 },
                color: Color::rgba(255, 255, 255, 18),
                position: LayerPosition::default(),
                animation: LayerAnimation::default(),
                enabled: true,
            }],
            complexity_budget: 5,
        };
        let clock = AnimClock::new();
        let ops = scene.build_ops(&clock, 480, 272, false);
        // Grid with spacing=10 at 480x272 produces many ops, should be skipped
        // due to exceeding the budget of 5
        assert!(ops.is_empty() || BackgroundScene::count_ops(&ops) <= 5);
    }

    #[test]
    fn anchor_resolve_corners() {
        assert_eq!(Anchor::TopLeft.resolve(480, 272), (0, 0));
        assert_eq!(Anchor::Center.resolve(480, 272), (240, 136));
        assert_eq!(Anchor::BottomRight.resolve(480, 272), (480, 272));
        assert_eq!(Anchor::CenterRight.resolve(480, 272), (480, 136));
    }

    #[test]
    fn reduced_motion_static() {
        let scene = BackgroundScene {
            layers: vec![BackgroundLayer {
                kind: LayerKind::WireframeSphere { radius: 60 },
                color: Color::rgba(255, 255, 255, 50),
                position: LayerPosition {
                    anchor: Anchor::CenterRight,
                    offset_x: -0.1,
                    offset_y: 0.0,
                },
                animation: LayerAnimation {
                    rotate_speed: 1.0,
                    ..LayerAnimation::default()
                },
                enabled: true,
            }],
            complexity_budget: 200,
        };
        let mut clock = AnimClock::new();
        clock.tick_dt(1000);
        let ops_normal = scene.build_ops(&clock, 480, 272, false);
        let ops_reduced = scene.build_ops(&clock, 480, 272, true);
        // Both should produce ops, but reduced motion should have no rotation
        assert!(!ops_normal.is_empty());
        assert!(!ops_reduced.is_empty());
    }
}
