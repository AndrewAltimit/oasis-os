//! Shared software rasterization algorithms.
//!
//! These functions implement platform-agnostic scanline rasterization. Each
//! backend provides a [`PixelSink`] implementation that maps horizontal spans
//! to its native drawing primitive (SDL3 draw_line, UE5 hline, etc.).

use crate::backend::Color;

/// Abstraction for horizontal-span pixel output.
///
/// Backends implement this to bridge shared rasterization algorithms to their
/// native drawing primitives.
pub trait PixelSink {
    /// Fill a horizontal span from `x1` to `x2` (inclusive) at row `y`.
    fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, color: Color);
}

/// Compute the x coordinate along an edge at a given y (linear interpolation).
pub fn edge_x(x0: i32, y0: i32, x1: i32, y1: i32, y: i32) -> i32 {
    if y1 == y0 {
        return x0;
    }
    x0 + (x1 - x0) * (y - y0) / (y1 - y0)
}

/// Integer square root (floor) using Newton's method.
pub fn isqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Rasterize a filled triangle using edge-walking scanlines.
pub fn rasterize_triangle(
    sink: &mut impl PixelSink,
    v0: (i32, i32),
    v1: (i32, i32),
    v2: (i32, i32),
    color: Color,
) {
    let mut verts = [v0, v1, v2];
    verts.sort_by_key(|v| v.1);
    let (vx0, vy0) = verts[0];
    let (vx1, vy1) = verts[1];
    let (vx2, vy2) = verts[2];

    if vy0 == vy2 {
        let min_x = vx0.min(vx1).min(vx2);
        let max_x = vx0.max(vx1).max(vx2);
        sink.draw_hline(min_x, max_x, vy0, color);
        return;
    }

    for y in vy0..=vy2 {
        let mut x_min = i32::MAX;
        let mut x_max = i32::MIN;

        let x_02 = edge_x(vx0, vy0, vx2, vy2, y);
        x_min = x_min.min(x_02);
        x_max = x_max.max(x_02);

        if y <= vy1 && vy0 != vy1 {
            let x_01 = edge_x(vx0, vy0, vx1, vy1, y);
            x_min = x_min.min(x_01);
            x_max = x_max.max(x_01);
        }
        if y >= vy1 && vy1 != vy2 {
            let x_12 = edge_x(vx1, vy1, vx2, vy2, y);
            x_min = x_min.min(x_12);
            x_max = x_max.max(x_12);
        }
        if y == vy1 {
            x_min = x_min.min(vx1);
            x_max = x_max.max(vx1);
        }

        if x_min <= x_max {
            sink.draw_hline(x_min, x_max, y, color);
        }
    }
}

// `rasterize_circle` / `rasterize_rounded_rect` used to live here. They were
// unused anywhere in the workspace (every software path goes through
// `oasis-rasterize`) and emitted some scanlines more than once, which
// double-blends translucent fills -- the bug fixed in `oasis-rasterize` by
// `midpoint_row_extents` / `rounded_rect_rows`. They were removed rather
// than fixed so there is a single, correct implementation to reuse.

/// Compute the perpendicular unit normal for a line direction vector.
///
/// Used by thick-line rendering in SDL and PSP backends. Returns `(nx, ny)`
/// such that the normal is perpendicular to `(dx, dy)` with unit length.
/// Returns `(0.0, 0.0)` if the input is a zero-length vector.
pub fn perpendicular_normal_f32(dx: f32, dy: f32) -> (f32, f32) {
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        return (0.0, 0.0);
    }
    // Use Newton's method for sqrt to avoid libm dependency.
    let mut est = len_sq;
    for _ in 0..8 {
        est = 0.5 * (est + len_sq / est);
    }
    let len = est.max(1.0);
    (-dy / len, dx / len)
}

/// Compute the horizontal extent of a circle at a given scanline offset.
///
/// For a circle of radius `r` centered at the origin, returns the x-extent
/// at vertical offset `dy` from center: `floor(sqrt(r^2 - dy^2))`.
/// Returns 0 when `|dy| > r`. Used by stroke-circle rendering (UE5 backend).
pub fn radial_extent(r: i32, dy: i32) -> i32 {
    let sq = r * r - dy * dy;
    if sq < 0 { 0 } else { isqrt(sq as u32) as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SpanRecorder {
        spans: Vec<(i32, i32, i32)>,
    }

    impl SpanRecorder {
        fn new() -> Self {
            Self { spans: Vec::new() }
        }
    }

    impl PixelSink for SpanRecorder {
        fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, _color: Color) {
            self.spans.push((x1, x2, y));
        }
    }

    #[test]
    fn edge_x_at_endpoints() {
        assert_eq!(edge_x(10, 20, 50, 80, 20), 10);
        assert_eq!(edge_x(10, 20, 50, 80, 80), 50);
    }

    #[test]
    fn edge_x_horizontal_returns_x0() {
        assert_eq!(edge_x(10, 50, 30, 50, 50), 10);
    }

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(10), 3);
        assert_eq!(isqrt(100), 10);
    }

    #[test]
    fn triangle_degenerate_horizontal() {
        let mut rec = SpanRecorder::new();
        rasterize_triangle(&mut rec, (0, 5), (10, 5), (5, 5), Color::WHITE);
        assert_eq!(rec.spans.len(), 1);
        assert_eq!(rec.spans[0], (0, 10, 5));
    }

    #[test]
    fn triangle_produces_spans() {
        let mut rec = SpanRecorder::new();
        rasterize_triangle(&mut rec, (5, 0), (0, 10), (10, 10), Color::WHITE);
        assert!(!rec.spans.is_empty());
        // All spans should be between y=0 and y=10.
        for &(_, _, y) in &rec.spans {
            assert!(y >= 0 && y <= 10);
        }
    }

    #[test]
    fn perpendicular_normal_unit_length() {
        let (nx, ny) = perpendicular_normal_f32(3.0, 4.0);
        let len = (nx * nx + ny * ny).sqrt();
        assert!(
            (len - 1.0).abs() < 0.01,
            "normal should be unit length: {len}"
        );
    }

    #[test]
    fn perpendicular_normal_is_perpendicular() {
        let (dx, dy) = (3.0f32, 4.0);
        let (nx, ny) = perpendicular_normal_f32(dx, dy);
        let dot = dx * nx + dy * ny;
        assert!(
            dot.abs() < 0.01,
            "normal should be perpendicular: dot={dot}"
        );
    }

    #[test]
    fn perpendicular_normal_zero_vec() {
        let (nx, ny) = perpendicular_normal_f32(0.0, 0.0);
        assert_eq!(nx, 0.0);
        assert_eq!(ny, 0.0);
    }

    #[test]
    fn radial_extent_at_center() {
        assert_eq!(radial_extent(10, 0), 10);
    }

    #[test]
    fn radial_extent_at_edge() {
        assert_eq!(radial_extent(10, 10), 0);
    }

    #[test]
    fn radial_extent_beyond_radius() {
        assert_eq!(radial_extent(10, 15), 0);
    }

    #[test]
    fn radial_extent_midpoint() {
        // At dy=6 with r=10: sqrt(100-36) = sqrt(64) = 8
        assert_eq!(radial_extent(10, 6), 8);
    }
}
