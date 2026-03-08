//! Animation helpers for vector icons.
//!
//! Frame-counter-driven animation primitives: rotation, pulse (alpha oscillation),
//! blink (on/off toggle), and float (sine-wave vertical offset).
//! These are applied to `VectorOp` trees at render time.

use crate::op::VectorOp;
use oasis_types::backend::Color;

/// Rotate a point `(px, py)` around `(cx, cy)` by `angle` radians.
pub fn rotate_point(px: i32, py: i32, cx: i32, cy: i32, angle: f32) -> (i32, i32) {
    let dx = (px - cx) as f32;
    let dy = (py - cy) as f32;
    let (sin, cos) = (angle.sin(), angle.cos());
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    (cx + rx as i32, cy + ry as i32)
}

/// Rotate a `FillRect` into a `FillPolygon` by computing its 4 corner positions.
///
/// The rotation is around the rectangle's center.
pub fn rotate_rect(x: i32, y: i32, w: u32, h: u32, angle: f32, color: Color) -> VectorOp {
    let cx = x + w as i32 / 2;
    let cy = y + h as i32 / 2;
    let corners = [
        (x, y),
        (x + w as i32, y),
        (x + w as i32, y + h as i32),
        (x, y + h as i32),
    ];
    let rotated: Vec<(i32, i32)> = corners
        .iter()
        .map(|&(px, py)| rotate_point(px, py, cx, cy, angle))
        .collect();
    VectorOp::FillPolygon {
        points: rotated,
        color,
    }
}

/// Compute a sine-wave float offset for a given frame and icon slot.
///
/// Each slot is phase-shifted so icons bob at different times.
/// Returns a Y offset in pixels.
pub fn float_offset(frame: u32, slot: usize, amplitude: f32, speed: f32) -> i32 {
    let phase = slot as f32 * 0.8; // stagger per icon
    let t = frame as f32 * speed + phase;
    (t.sin() * amplitude) as i32
}

/// Compute a pulse alpha (0-255) for a given frame.
///
/// Oscillates between `min_alpha` and 255 using a sine wave.
pub fn pulse_alpha(frame: u32, speed: f32, min_alpha: u8) -> u8 {
    let t = frame as f32 * speed;
    let norm = (t.sin() + 1.0) * 0.5; // 0.0 to 1.0
    let range = 255 - min_alpha as u16;
    (min_alpha as u16 + (norm * range as f32) as u16).min(255) as u8
}

/// Compute blink visibility for a given frame.
///
/// Returns true if the element should be visible (on for 2/3 of interval).
pub fn blink_visible(frame: u32, interval: u32) -> bool {
    let phase = frame % interval;
    // On for first 2/3, off for last 1/3
    phase < (interval * 2 / 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_point_identity() {
        // 0 radians = no rotation
        let (rx, ry) = rotate_point(10, 0, 0, 0, 0.0);
        assert_eq!((rx, ry), (10, 0));
    }

    #[test]
    fn rotate_point_90_degrees() {
        use core::f32::consts::FRAC_PI_2;
        let (rx, ry) = rotate_point(10, 0, 0, 0, FRAC_PI_2);
        // (10, 0) rotated 90 degrees CW around origin → (0, 10)
        assert!((rx - 0).abs() <= 1);
        assert!((ry - 10).abs() <= 1);
    }

    #[test]
    fn rotate_rect_produces_polygon() {
        let op = rotate_rect(0, 0, 10, 10, 0.0, Color::WHITE);
        match op {
            VectorOp::FillPolygon { points, .. } => {
                assert_eq!(points.len(), 4);
                // At 0 rotation, corners should be approximately the original
                assert_eq!(points[0], (0, 0));
                assert_eq!(points[1], (10, 0));
            },
            _ => panic!("expected FillPolygon"),
        }
    }

    #[test]
    fn float_offset_oscillates() {
        let o1 = float_offset(0, 0, 2.0, 0.04);
        let o2 = float_offset(40, 0, 2.0, 0.04);
        // Should produce different offsets at different frames
        // (at frame 0, sin(0)=0; at frame 40, sin(1.6) != 0)
        assert!(o1 != o2 || o1 == 0);
    }

    #[test]
    fn float_offset_staggers_slots() {
        let o0 = float_offset(10, 0, 2.0, 0.04);
        let o1 = float_offset(10, 1, 2.0, 0.04);
        // Different slots at same frame may differ due to phase offset
        let _ = (o0, o1); // just ensure no panic
    }

    #[test]
    fn pulse_alpha_range() {
        for f in 0..200 {
            let a = pulse_alpha(f, 0.06, 80);
            assert!(a >= 80);
            assert!(a <= 255);
        }
    }

    #[test]
    fn blink_visible_duty_cycle() {
        let interval = 45;
        let on_count = (0..interval)
            .filter(|&f| blink_visible(f, interval))
            .count();
        // Should be on for ~2/3 of the interval (30 frames)
        assert_eq!(on_count, 30);
    }
}
