//! Factory functions for background layer elements.
//!
//! Each function produces a `Vec<VectorOp>` for a specific [`super::background::LayerKind`].
//! These are the building blocks that [`super::background::BackgroundScene::build_ops()`] dispatches to.

use oasis_types::backend::Color;

use crate::anim::AnimClock;
use crate::op::VectorOp;

/// Saturating `u32` -> `i32` for viewport sizes.
fn sat_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// Saturating `i64` -> `i32` for derived coordinates.
fn clamp_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Interior grid positions `spacing, 2*spacing, ... < len` (none when
/// `spacing` is 0). Stepping in `u32` cannot overflow or run backwards
/// the way an `i32` cursor did for spacings above `i32::MAX`.
fn ticks(len: u32, spacing: u32) -> impl Iterator<Item = i32> {
    let step = usize::try_from(spacing).unwrap_or(usize::MAX).max(1);
    let end = if spacing == 0 { 0 } else { len };
    (spacing..end).step_by(step).map(sat_i32)
}

/// Grid of thin lines at regular spacing.
pub fn grid(w: u32, h: u32, spacing: u32, color: Color) -> Vec<VectorOp> {
    let (wi, hi) = (sat_i32(w), sat_i32(h));
    let vertical = ticks(w, spacing).map(|x| VectorOp::Line {
        x1: x,
        y1: 0,
        x2: x,
        y2: hi,
        width: 1,
        color,
    });
    let horizontal = ticks(h, spacing).map(|y| VectorOp::Line {
        x1: 0,
        y1: y,
        x2: wi,
        y2: y,
        width: 1,
        color,
    });
    vertical.chain(horizontal).collect()
}

/// Dot pattern at regular spacing.
pub fn dot_grid(w: u32, h: u32, spacing: u32, radius: u16, color: Color) -> Vec<VectorOp> {
    ticks(w, spacing)
        .flat_map(|x| {
            ticks(h, spacing).map(move |y| VectorOp::FillCircle {
                cx: x,
                cy: y,
                radius,
                color,
            })
        })
        .collect()
}

/// Wireframe sphere with animated rotation.
pub fn wireframe_sphere(cx: i32, cy: i32, radius: u16, color: Color, angle: f32) -> Vec<VectorOp> {
    let r = radius as i32;
    let full_circle = core::f32::consts::TAU;
    let inner_r = radius / 3;
    let (sin, cos) = (angle.sin(), angle.cos());

    vec![
        // Main circle
        VectorOp::StrokeArc {
            cx,
            cy,
            radius,
            start_angle: 0.0,
            end_angle: full_circle,
            width: 1,
            color,
        },
        // Horizontal cross line
        VectorOp::Line {
            x1: cx.saturating_sub(r),
            y1: cy,
            x2: cx.saturating_add(r),
            y2: cy,
            width: 1,
            color,
        },
        // Vertical cross line
        VectorOp::Line {
            x1: cx,
            y1: cy.saturating_sub(r),
            x2: cx,
            y2: cy.saturating_add(r),
            width: 1,
            color,
        },
        // Rotating longitude line
        VectorOp::Line {
            x1: cx.saturating_add((inner_r as f32 * cos) as i32),
            y1: cy.saturating_sub(r),
            x2: cx.saturating_sub((inner_r as f32 * cos) as i32),
            y2: cy.saturating_add(r),
            width: 1,
            color,
        },
        // Perpendicular rotating line
        VectorOp::Line {
            x1: cx.saturating_add((inner_r as f32 * sin) as i32),
            y1: cy.saturating_sub(r),
            x2: cx.saturating_sub((inner_r as f32 * sin) as i32),
            y2: cy.saturating_add(r),
            width: 1,
            color,
        },
    ]
}

/// Radar sweep arc.
pub fn radar_sweep(
    cx: i32,
    cy: i32,
    radius: u16,
    sweep_angle: f32,
    rotation: f32,
    color: Color,
) -> Vec<VectorOp> {
    vec![VectorOp::FillArc {
        cx,
        cy,
        radius,
        start_angle: rotation,
        end_angle: rotation + sweep_angle,
        color,
    }]
}

/// Concentric ring outlines.
pub fn concentric_rings(
    cx: i32,
    cy: i32,
    count: u8,
    radius: u16,
    stroke_width: u16,
    color: Color,
) -> Vec<VectorOp> {
    let full_circle = core::f32::consts::TAU;
    let mut ops = Vec::with_capacity(count as usize);
    for i in 1..=count {
        let r = radius as u32 * i as u32 / count as u32;
        ops.push(VectorOp::StrokeArc {
            cx,
            cy,
            radius: r.min(u16::MAX as u32) as u16,
            start_angle: 0.0,
            end_angle: full_circle,
            width: stroke_width,
            color,
        });
    }
    ops
}

/// Translucent glass shard polygon (normalized 0..1 coords mapped to viewport).
pub fn glass_shard(
    points: &[(f32, f32)],
    w: u32,
    h: u32,
    color: Color,
    drift_x: f32,
    drift_y: f32,
) -> Vec<VectorOp> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let wf = w as f32;
    let hf = h as f32;
    // Wrap the drift offset itself so the entire polygon moves as a unit.
    // Per-vertex wrapping would tear the shape when it straddles the boundary.
    let wrapped_dx = drift_x.rem_euclid(wf);
    let wrapped_dy = drift_y.rem_euclid(hf);
    let mapped: Vec<(i32, i32)> = points
        .iter()
        .map(|&(px, py)| ((px * wf + wrapped_dx) as i32, (py * hf + wrapped_dy) as i32))
        .collect();
    vec![VectorOp::FillPolygon {
        points: mapped,
        color,
    }]
}

/// Horizontal scanlines across the viewport.
pub fn scanlines(w: u32, h: u32, spacing: u16, color: Color) -> Vec<VectorOp> {
    let wi = sat_i32(w);
    ticks(h, u32::from(spacing))
        .map(|y| VectorOp::Line {
            x1: 0,
            y1: y,
            x2: wi,
            y2: y,
            width: 1,
            color,
        })
        .collect()
}

/// Audio equalizer bar visualization.
#[allow(clippy::too_many_arguments)]
pub fn eq_visualizer(
    cx: i32,
    cy: i32,
    count: u8,
    bar_width: u32,
    max_height: u32,
    color: Color,
    clock: &AnimClock,
    reduced_motion: bool,
) -> Vec<VectorOp> {
    // i64: a large `bar_width` / `max_height` must not overflow i32.
    let pitch = i64::from(bar_width) + 2;
    let total_w = i64::from(count) * pitch - 2;
    let start_x = i64::from(cx) - total_w / 2;
    let mut ops = Vec::with_capacity(count as usize);
    for i in 0..count {
        let h = if reduced_motion {
            max_height / 2
        } else {
            let phase = i as f32 * 1.2;
            let norm = clock.sine_norm(1.5, phase);
            (norm * max_height as f32) as u32
        };
        let h = h.max(2);
        let x = clamp_i32(start_x + i64::from(i) * pitch);
        let y = clamp_i32(i64::from(cy) - i64::from(h));
        ops.push(VectorOp::FillRect {
            x,
            y,
            w: bar_width,
            h,
            color,
        });
    }
    ops
}

/// Crosshair reticle.
pub fn crosshair(cx: i32, cy: i32, size: u16, color: Color) -> Vec<VectorOp> {
    let s = size as i32;
    vec![
        // Horizontal
        VectorOp::Line {
            x1: cx.saturating_sub(s),
            y1: cy,
            x2: cx.saturating_add(s),
            y2: cy,
            width: 1,
            color,
        },
        // Vertical
        VectorOp::Line {
            x1: cx,
            y1: cy.saturating_sub(s),
            x2: cx,
            y2: cy.saturating_add(s),
            width: 1,
            color,
        },
        // Center dot
        VectorOp::FillCircle {
            cx,
            cy,
            radius: 2,
            color,
        },
    ]
}

/// Floating drifting polygons.
#[allow(clippy::too_many_arguments)]
pub fn floating_polygons(
    w: u32,
    h: u32,
    count: u8,
    sides: u8,
    color: Color,
    clock: &AnimClock,
    drift_x: f32,
    drift_y: f32,
    base_phase: f32,
) -> Vec<VectorOp> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let sides = sides.max(3) as usize;

    for i in 0..count {
        let phase = base_phase + i as f32 * 2.3;
        // Position: deterministic based on index, drifting over time
        let (wf, hf) = (w as f32, h as f32);
        let base_x = (wf * ((i as f32 * 0.37 + 0.1) % 1.0)).floor();
        let base_y = (hf * ((i as f32 * 0.53 + 0.2) % 1.0)).floor();
        // The drift is wrapped to the viewport in f64 first: `time * drift`
        // in f32 lost precision with uptime and overflowed the i32 add.
        let dx = clock.wrapped(drift_x, wf) + phase * 10.0;
        let dy = clock.wrapped(drift_y, hf) + phase * 7.0;
        let cx = (base_x + dx.trunc()).rem_euclid(wf) as i32;
        let cy = (base_y + dy.trunc()).rem_euclid(hf) as i32;
        let radius = 8.0 + (i as f32 * 3.7) % 12.0;

        let mut points = Vec::with_capacity(sides);
        for s in 0..sides {
            let angle = core::f32::consts::TAU * s as f32 / sides as f32 + clock.angle(0.3, phase);
            points.push((
                cx.saturating_add((radius * angle.cos()) as i32),
                cy.saturating_add((radius * angle.sin()) as i32),
            ));
        }
        ops.push(VectorOp::FillPolygon { points, color });
    }
    ops
}

/// Pulsing core circle (typically at center).
pub fn pulsing_core(cx: i32, cy: i32, radius: u16, color: Color) -> Vec<VectorOp> {
    let full_circle = core::f32::consts::TAU;
    vec![
        // Outer glow ring
        VectorOp::StrokeArc {
            cx,
            cy,
            radius: radius.saturating_add(4),
            start_angle: 0.0,
            end_angle: full_circle,
            width: 1,
            color: Color::rgba(color.r, color.g, color.b, color.a / 3),
        },
        // Main circle
        VectorOp::FillCircle {
            cx,
            cy,
            radius,
            color,
        },
    ]
}

/// Compute a wave Y offset for a given x position, row parameters, and time.
fn wave_y(x: f32, w: f32, frequency: f32, time: f32, row_phase: f32, row_amp: f32) -> f32 {
    let phase = x * frequency / w * core::f32::consts::TAU + time + row_phase;
    row_amp * phase.sin() + row_amp * 0.3 * (phase * 2.3 + row_phase * 0.5).sin()
}

/// Undulating wave surface across the full viewport (Vanta Waves style).
///
/// Draws filled bands between adjacent wave rows to create a 3D surface
/// illusion with perspective depth. Rows near the bottom are brighter and
/// have larger amplitude. A bright stroke on each wave crest adds shininess.
#[allow(clippy::too_many_arguments)]
pub fn waves(
    w: u32,
    h: u32,
    rows: u8,
    amplitude: u16,
    frequency: f32,
    speed: f32,
    color: Color,
    clock: &AnimClock,
    reduced_motion: bool,
) -> Vec<VectorOp> {
    let rows = rows.max(4) as usize;
    let amp = amplitude as f32;
    // Use fewer segments for a smoother look without excessive ops.
    let segments = (w / 12).max(12) as usize;
    let seg_w = w as f32 / segments as f32;
    let mut ops = Vec::with_capacity(rows * segments);
    // Both harmonics of `wave_y` (phase and 2.3 x phase) repeat after
    // 10 turns, so wrapping the time term there is seamless -- and keeps
    // the f32 sine arguments small for any uptime.
    let time = if reduced_motion {
        0.0
    } else {
        clock.wrapped(speed, 10.0 * core::f32::consts::TAU)
    };

    // Pre-compute wave points for each row.
    let row_data: Vec<(Vec<(i32, i32)>, f32)> = (0..rows)
        .map(|row| {
            let t = row as f32 / (rows - 1).max(1) as f32;
            let base_y = h as f32 * 0.12 + t * (h as f32 * 0.82);
            let row_phase = row as f32 * 0.8;
            let row_amp = amp * (0.3 + 0.7 * t);
            let points: Vec<(i32, i32)> = (0..=segments)
                .map(|s| {
                    let x = s as f32 * seg_w;
                    let y_off = wave_y(x, w as f32, frequency, time, row_phase, row_amp);
                    (x as i32, (base_y + y_off) as i32)
                })
                .collect();
            (points, t)
        })
        .collect();

    // Draw filled quads between adjacent rows (back to front).
    for pair in row_data.windows(2) {
        let (top_pts, _t_top) = &pair[0];
        let (bot_pts, t_bot) = &pair[1];

        // Band fill alpha: darker at top, brighter at bottom.
        let fill_alpha = ((color.a as f32 * (0.25 + 0.75 * t_bot)) as u8).max(1);
        let fill_color = Color::rgba(color.r, color.g, color.b, fill_alpha);

        // Draw filled trapezoids between each segment pair.
        for s in 0..segments {
            let tl = top_pts[s];
            let tr = top_pts[s + 1];
            let br = bot_pts[s + 1];
            let bl = bot_pts[s];
            ops.push(VectorOp::FillPolygon {
                points: vec![tl, tr, br, bl],
                color: fill_color,
            });
        }
    }

    // Draw bright wave crests (stroke lines on each row).
    for (points, t) in &row_data {
        let stroke_alpha = ((color.a as f32 * (0.5 + 0.5 * t)) as u8).max(1);
        let stroke_color = Color::rgba(
            color.r.saturating_add(60),
            color.g.saturating_add(60),
            color.b.saturating_add(60),
            stroke_alpha,
        );
        let width = if *t > 0.6 { 2 } else { 1 };
        for pair in points.windows(2) {
            ops.push(VectorOp::Line {
                x1: pair[0].0,
                y1: pair[0].1,
                x2: pair[1].0,
                y2: pair[1].1,
                width,
                color: stroke_color,
            });
        }
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_produces_lines() {
        let ops = grid(100, 100, 30, Color::WHITE);
        // 3 vertical (30,60,90) + 3 horizontal (30,60,90) = 6
        assert_eq!(ops.len(), 6);
    }

    #[test]
    fn pulsing_core_max_radius_does_not_overflow() {
        let ops = pulsing_core(0, 0, u16::MAX, Color::WHITE);
        let VectorOp::StrokeArc { radius, .. } = &ops[0] else {
            panic!("expected StrokeArc, got {:?}", ops[0]);
        };
        assert_eq!(*radius, u16::MAX);
    }

    #[test]
    fn grid_zero_spacing() {
        let ops = grid(100, 100, 0, Color::WHITE);
        assert!(ops.is_empty());
    }

    #[test]
    fn dot_grid_produces_circles() {
        let ops = dot_grid(100, 100, 30, 2, Color::WHITE);
        // 3x3 = 9 dots
        assert_eq!(ops.len(), 9);
    }

    #[test]
    fn wireframe_sphere_ops() {
        let ops = wireframe_sphere(100, 100, 60, Color::WHITE, 0.0);
        assert_eq!(ops.len(), 5);
    }

    #[test]
    fn radar_sweep_single_arc() {
        let ops = radar_sweep(100, 100, 60, 0.8, 0.0, Color::WHITE);
        assert_eq!(ops.len(), 1);
    }

    #[test]
    fn concentric_rings_count() {
        let ops = concentric_rings(100, 100, 3, 60, 1, Color::WHITE);
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn glass_shard_maps_coords() {
        let ops = glass_shard(
            &[(0.0, 1.0), (0.2, 0.75), (0.125, 1.0)],
            480,
            272,
            Color::WHITE,
            0.0,
            0.0,
        );
        assert_eq!(ops.len(), 1);
    }

    #[test]
    fn scanlines_spacing() {
        let ops = scanlines(480, 272, 4, Color::WHITE);
        assert!(!ops.is_empty());
        // 272 / 4 - 1 = 67 lines
        assert_eq!(ops.len(), 67);
    }

    #[test]
    fn crosshair_produces_three_ops() {
        let ops = crosshair(100, 100, 20, Color::WHITE);
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn eq_visualizer_count() {
        let clock = AnimClock::new();
        let ops = eq_visualizer(100, 100, 5, 8, 30, Color::WHITE, &clock, false);
        assert_eq!(ops.len(), 5);
    }

    #[test]
    fn pulsing_core_two_ops() {
        let ops = pulsing_core(100, 100, 10, Color::WHITE);
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn huge_spacing_terminates_without_lines() {
        // Regression: `spacing as i32` went negative above i32::MAX and the
        // cursor ran backwards forever (overflow panic in debug builds).
        assert!(grid(480, 272, u32::MAX, Color::WHITE).is_empty());
        assert!(dot_grid(480, 272, 3_000_000_000, 1, Color::WHITE).is_empty());
        // Positions still match the old loop for normal sizes.
        let ops = scanlines(10, 10, 3, Color::WHITE);
        let ys: Vec<i32> = ops
            .iter()
            .map(|op| match op {
                VectorOp::Line { y1, .. } => *y1,
                _ => panic!("expected lines"),
            })
            .collect();
        assert_eq!(ys, vec![3, 6, 9]);
    }

    #[test]
    fn extreme_layer_parameters_do_not_overflow() {
        let clock = AnimClock::new();
        let ops = eq_visualizer(0, 0, 255, u32::MAX, u32::MAX, Color::WHITE, &clock, true);
        assert_eq!(ops.len(), 255);
        assert_eq!(
            crosshair(i32::MAX, i32::MIN, u16::MAX, Color::WHITE).len(),
            3
        );
        assert_eq!(
            wireframe_sphere(i32::MIN, 0, u16::MAX, Color::WHITE, 1.0).len(),
            5
        );
    }

    #[test]
    fn floating_polygons_stay_on_screen_after_long_uptime() {
        // Regression: `time * drift` saturated to i32::MAX and the `+ base`
        // overflowed after enough uptime with a fast drift.
        let clock = AnimClock::from_frame(u32::MAX);
        let ops = floating_polygons(480, 272, 4, 5, Color::WHITE, &clock, 1.0e6, -3.0e5, 0.5);
        assert_eq!(ops.len(), 4);
    }

    #[test]
    fn floating_polygons_count() {
        let clock = AnimClock::new();
        let ops = floating_polygons(480, 272, 3, 4, Color::WHITE, &clock, 5.0, 3.0, 0.0);
        assert_eq!(ops.len(), 3);
    }
}
