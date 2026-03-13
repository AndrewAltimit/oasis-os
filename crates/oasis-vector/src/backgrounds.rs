//! Factory functions for background layer elements.
//!
//! Each function produces a `Vec<VectorOp>` for a specific [`super::background::LayerKind`].
//! These are the building blocks that [`super::background::BackgroundScene::build_ops()`] dispatches to.

use oasis_types::backend::Color;

use crate::anim::AnimClock;
use crate::op::VectorOp;

/// Grid of thin lines at regular spacing.
pub fn grid(w: u32, h: u32, spacing: u32, color: Color) -> Vec<VectorOp> {
    if spacing == 0 {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let mut x = spacing as i32;
    while x < w as i32 {
        ops.push(VectorOp::Line {
            x1: x,
            y1: 0,
            x2: x,
            y2: h as i32,
            width: 1,
            color,
        });
        x += spacing as i32;
    }
    let mut y = spacing as i32;
    while y < h as i32 {
        ops.push(VectorOp::Line {
            x1: 0,
            y1: y,
            x2: w as i32,
            y2: y,
            width: 1,
            color,
        });
        y += spacing as i32;
    }
    ops
}

/// Dot pattern at regular spacing.
pub fn dot_grid(w: u32, h: u32, spacing: u32, radius: u16, color: Color) -> Vec<VectorOp> {
    if spacing == 0 {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let mut x = spacing as i32;
    while x < w as i32 {
        let mut y = spacing as i32;
        while y < h as i32 {
            ops.push(VectorOp::FillCircle {
                cx: x,
                cy: y,
                radius,
                color,
            });
            y += spacing as i32;
        }
        x += spacing as i32;
    }
    ops
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
            x1: cx - r,
            y1: cy,
            x2: cx + r,
            y2: cy,
            width: 1,
            color,
        },
        // Vertical cross line
        VectorOp::Line {
            x1: cx,
            y1: cy - r,
            x2: cx,
            y2: cy + r,
            width: 1,
            color,
        },
        // Rotating longitude line
        VectorOp::Line {
            x1: cx + (inner_r as f32 * cos) as i32,
            y1: cy - r,
            x2: cx - (inner_r as f32 * cos) as i32,
            y2: cy + r,
            width: 1,
            color,
        },
        // Perpendicular rotating line
        VectorOp::Line {
            x1: cx + (inner_r as f32 * sin) as i32,
            y1: cy - r,
            x2: cx - (inner_r as f32 * sin) as i32,
            y2: cy + r,
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
    let wi = w as i32;
    let hi = h as i32;
    let mapped: Vec<(i32, i32)> = points
        .iter()
        .map(|&(px, py)| {
            (
                ((px * w as f32 + drift_x) as i32).rem_euclid(wi),
                ((py * h as f32 + drift_y) as i32).rem_euclid(hi),
            )
        })
        .collect();
    vec![VectorOp::FillPolygon {
        points: mapped,
        color,
    }]
}

/// Horizontal scanlines across the viewport.
pub fn scanlines(w: u32, h: u32, spacing: u16, color: Color) -> Vec<VectorOp> {
    if spacing == 0 {
        return Vec::new();
    }
    let mut ops = Vec::new();
    let mut y = spacing as i32;
    while y < h as i32 {
        ops.push(VectorOp::Line {
            x1: 0,
            y1: y,
            x2: w as i32,
            y2: y,
            width: 1,
            color,
        });
        y += spacing as i32;
    }
    ops
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
    let total_w = count as i32 * (bar_width as i32 + 2) - 2;
    let start_x = cx - total_w / 2;
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
        let x = start_x + i as i32 * (bar_width as i32 + 2);
        let y = cy - h as i32;
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
            x1: cx - s,
            y1: cy,
            x2: cx + s,
            y2: cy,
            width: 1,
            color,
        },
        // Vertical
        VectorOp::Line {
            x1: cx,
            y1: cy - s,
            x2: cx,
            y2: cy + s,
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
        let base_x = (w as f32 * ((i as f32 * 0.37 + 0.1) % 1.0)) as i32;
        let base_y = (h as f32 * ((i as f32 * 0.53 + 0.2) % 1.0)) as i32;
        let dx = (clock.time_s * drift_x + phase * 10.0) as i32;
        let dy = (clock.time_s * drift_y + phase * 7.0) as i32;
        let cx = (base_x + dx).rem_euclid(w as i32);
        let cy = (base_y + dy).rem_euclid(h as i32);
        let radius = 8.0 + (i as f32 * 3.7) % 12.0;

        let mut points = Vec::with_capacity(sides);
        for s in 0..sides {
            let angle =
                core::f32::consts::TAU * s as f32 / sides as f32 + clock.time_s * 0.3 + phase;
            points.push((
                cx + (radius * angle.cos()) as i32,
                cy + (radius * angle.sin()) as i32,
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
            radius: radius + 4,
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
    let time = if reduced_motion {
        0.0
    } else {
        clock.time_s * speed
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
    fn floating_polygons_count() {
        let clock = AnimClock::new();
        let ops = floating_polygons(480, 272, 3, 4, Color::WHITE, &clock, 5.0, 3.0, 0.0);
        assert_eq!(ops.len(), 3);
    }
}
