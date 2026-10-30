//! Shared software rasterization algorithms.
//!
//! Pure, allocation-light span math lives here so the `SdiShapes` /
//! `SdiVector` trait defaults (and therefore PSP and every test mock) can
//! use exactly the same pixel coverage as the desktop backends. Pixel
//! buffers, glyph masks and TTF rasterization live in `oasis-rasterize`,
//! which re-exports the span generators.
//!
//! - [`PixelSink`] + [`rasterize_triangle`]: edge-walking triangle fill.
//! - The `*_rows` span generators ([`midpoint_row_extents`],
//!   [`rounded_rect_rows`], [`thick_line_rows`], [`stroke_circle_rows`],
//!   [`stroke_rounded_rect_rows`], [`polygon_rows`]).

use crate::backend::Color;

mod rows;
pub use rows::{
    midpoint_row_extents, polygon_rows, rounded_rect_rows, stroke_circle_rows,
    stroke_rounded_rect_rows, thick_line_rows,
};

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
            assert!((0..=10).contains(&y));
        }
    }
}
