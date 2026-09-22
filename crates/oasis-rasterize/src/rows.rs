//! Scanline span generators shared by every backend.
//!
//! Each generator reports the pixels a shape covers as horizontal spans,
//! **one call per covered row** (two for rows a ring passes through twice),
//! so a translucent fill blends every pixel exactly once. The software
//! rasterizer fills the spans directly; the SDL backend submits them as
//! one batched `fill_rects` call. Sharing the generators is what keeps the
//! backends pixel-identical: there is exactly one definition of which
//! pixels a thick line, a stroked circle or a polygon covers.

use crate::rounded_rect_rows;
use oasis_types::rasterize;

/// Rows of a line from `(x1, y1)` to `(x2, y2)` stamped with a square
/// `width` x `width` brush at every Bresenham step (the brush spans
/// `-width/2 ..= width - width/2 - 1` around the step, so even widths lean
/// towards negative coordinates).
///
/// The union of the brush stamps is reported as one half-open span
/// `f(y, x0, x1)` per row: the steps covering a row are consecutive and
/// each moves at most one pixel, so that union is always contiguous.
/// Widths `<= 1` report the single-pixel Bresenham line.
pub fn thick_line_rows(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    width: u16,
    mut f: impl FnMut(i32, i32, i32),
) {
    let w = (width as i32).max(1);
    let lo = -(w / 2);
    let hi = w - w / 2 - 1;
    let y_min = y1.min(y2) + lo;
    let rows = (y1.max(y2) + hi - y_min + 1) as usize;
    let mut spans = vec![(i32::MAX, i32::MIN); rows];

    let dx = (x2 - x1).abs();
    let dy = -(y2 - y1).abs();
    let sx = if x1 < x2 { 1 } else { -1 };
    let sy = if y1 < y2 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut cx, mut cy) = (x1, y1);
    loop {
        for yy in (cy + lo)..=(cy + hi) {
            let s = &mut spans[(yy - y_min) as usize];
            s.0 = s.0.min(cx + lo);
            s.1 = s.1.max(cx + hi);
        }
        if cx == x2 && cy == y2 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
    for (i, &(a, b)) in spans.iter().enumerate() {
        if a <= b {
            f(y_min + i as i32, a, b + 1);
        }
    }
}

/// Rows of a circle outline centred on the origin: the annulus between
/// `radius` and `radius - stroke_width` (a width of 0 counts as 1).
///
/// Calls `f(dy, x0, x1)` with half-open spans; rows the ring crosses twice
/// produce two disjoint spans.
pub fn stroke_circle_rows(radius: u16, stroke_width: u16, mut f: impl FnMut(i32, i32, i32)) {
    if radius == 0 {
        return;
    }
    let r_outer = radius as i32;
    let r_inner = (r_outer - (stroke_width as i32).max(1)).max(0);
    for dy in -r_outer..=r_outer {
        let outer_x = rasterize::isqrt((r_outer * r_outer - dy * dy) as u32) as i32;
        if r_inner > 0 {
            let inner_sq = r_inner * r_inner - dy * dy;
            if inner_sq > 0 {
                let inner_x = rasterize::isqrt(inner_sq as u32) as i32;
                f(dy, -outer_x, -inner_x + 1);
                f(dy, inner_x, outer_x + 1);
                continue;
            }
        }
        f(dy, -outer_x, outer_x + 1);
    }
}

/// Rows of a `w` x `h` rounded-rect outline `stroke_width` pixels thick
/// (a width of 0 counts as 1): the filled rounded rect with radius `r`
/// (already clamped to `w/2`, `h/2`) minus the filled inner rounded rect
/// inset by the stroke with radius `r - stroke_width`.
///
/// Calls `f(dy, x0, x1)` with half-open spans relative to the rect's top
/// left; rows crossing the hole produce two spans.
pub fn stroke_rounded_rect_rows(
    w: i32,
    h: i32,
    r: i32,
    stroke_width: u16,
    mut f: impl FnMut(i32, i32, i32),
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let sw = (stroke_width as i32).max(1);
    let (iw, ih) = (w - 2 * sw, h - 2 * sw);
    if iw <= 0 || ih <= 0 {
        // Stroke wider than half the rect: the outline is solid.
        rounded_rect_rows(w, h, r, f);
        return;
    }
    let ir = (r - sw).max(0).min(iw / 2).min(ih / 2);
    let mut hole = vec![None; ih as usize];
    rounded_rect_rows(iw, ih, ir, |dy, x0, x1| {
        hole[dy as usize] = Some((x0 + sw, x1 + sw));
    });
    rounded_rect_rows(w, h, r, |dy, x0, x1| {
        let inner = usize::try_from(dy - sw)
            .ok()
            .and_then(|i| hole.get(i).copied().flatten());
        match inner {
            Some((ix0, ix1)) => {
                if x0 < ix0 {
                    f(dy, x0, ix0);
                }
                if ix1 < x1 {
                    f(dy, ix1, x1);
                }
            },
            None => f(dy, x0, x1),
        }
    });
}

/// Rows of a filled polygon under the even-odd rule, sampled at integer
/// row coordinates.
///
/// Each edge covers rows `min(y0, y1) ..< max(y0, y1)` (horizontal edges
/// none); the crossings of a row are sorted and paired into spans, and
/// each pair `(a, b)` is reported as `f(y, a, b + 1)` (both crossings
/// inclusive). `xs` is scratch storage reused across calls.
pub fn polygon_rows(points: &[(i32, i32)], xs: &mut Vec<i32>, mut f: impl FnMut(i32, i32, i32)) {
    if points.len() < 3 {
        return;
    }
    let y_min = points.iter().map(|v| v.1).min().unwrap_or(0);
    let y_max = points.iter().map(|v| v.1).max().unwrap_or(0);
    let n = points.len();
    for y in y_min..=y_max {
        xs.clear();
        for i in 0..n {
            let (x0, y0) = points[i];
            let (x1, y1) = points[(i + 1) % n];
            if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                let t = (y - y0) as f32 / (y1 - y0) as f32;
                xs.push(x0 + (t * (x1 - x0) as f32) as i32);
            }
        }
        xs.sort_unstable();
        for pair in xs.as_chunks::<2>().0 {
            if pair[0] <= pair[1] {
                f(y, pair[0], pair[1] + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect spans into a coverage-count grid of `w` x `h` with the
    /// origin at `(ox, oy)`.
    fn grid(
        w: usize,
        h: usize,
        (ox, oy): (i32, i32),
        emit: impl FnOnce(&mut dyn FnMut(i32, i32, i32)),
    ) -> Vec<u32> {
        let mut g = vec![0u32; w * h];
        let mut put = |y: i32, x0: i32, x1: i32| {
            assert!(x0 < x1, "empty span {x0}..{x1} on row {y}");
            for x in x0..x1 {
                let (gx, gy) = (x + ox, y + oy);
                if (0..w as i32).contains(&gx) && (0..h as i32).contains(&gy) {
                    g[gy as usize * w + gx as usize] += 1;
                }
            }
        };
        emit(&mut put);
        g
    }

    #[test]
    fn thick_line_covers_each_pixel_once() {
        for (x1, y1, x2, y2, w) in [
            (2, 2, 40, 2, 3u16),
            (5, 5, 5, 30, 4),
            (3, 3, 30, 30, 3),
            (40, 4, 4, 20, 5),
            (10, 30, 35, 2, 2),
            (7, 7, 7, 7, 3),
        ] {
            let g = grid(50, 40, (0, 0), |put| {
                thick_line_rows(x1, y1, x2, y2, w, put)
            });
            assert!(
                g.iter().all(|&n| n <= 1),
                "{:?}: double coverage",
                (x1, y1, x2, y2)
            );
            assert!(g.contains(&1));
        }
    }

    #[test]
    fn thick_diagonal_is_thick() {
        let mut widths = Vec::new();
        thick_line_rows(0, 0, 30, 30, 3, |_, a, b| widths.push(b - a));
        // Interior rows of a 45-degree 3px square-brush line are 5 px wide.
        assert!(
            widths[3..widths.len() - 3].iter().all(|&w| w == 5),
            "{widths:?}"
        );
    }

    #[test]
    fn thin_line_matches_bresenham_pixels() {
        let g = grid(40, 40, (0, 0), |put| thick_line_rows(1, 1, 30, 12, 1, put));
        let n: u32 = g.iter().sum();
        assert_eq!(n, 30, "one pixel per Bresenham step along the major axis");
    }

    #[test]
    fn stroke_circle_is_a_ring() {
        let g = grid(41, 41, (20, 20), |put| stroke_circle_rows(18, 4, put));
        assert!(g.iter().all(|&n| n <= 1));
        assert_eq!(g[20 * 41 + 20], 0, "center is hollow");
        assert_eq!(g[20 * 41 + 38], 1, "rightmost ring pixel");
        // Width 0 draws the same ring as width 1.
        let a = grid(41, 41, (20, 20), |put| stroke_circle_rows(10, 0, put));
        let b = grid(41, 41, (20, 20), |put| stroke_circle_rows(10, 1, put));
        assert_eq!(a, b);
    }

    #[test]
    fn stroke_rounded_rect_is_outer_minus_inner() {
        for (w, h, r, sw) in [
            (50, 36, 10, 1u16),
            (50, 36, 12, 3),
            (30, 30, 15, 2),
            (9, 9, 4, 5),
        ] {
            let g = grid(w as usize, h as usize, (0, 0), |put| {
                stroke_rounded_rect_rows(w, h, r, sw, put)
            });
            assert!(g.iter().all(|&n| n <= 1), "{w}x{h} r{r} sw{sw}");
            let outer = grid(w as usize, h as usize, (0, 0), |put| {
                rounded_rect_rows(w, h, r, put)
            });
            // Every stroked pixel lies inside the filled shape; edges are
            // `sw` thick in the middle of each side.
            for (i, (&s, &o)) in g.iter().zip(&outer).enumerate() {
                assert!(s <= o, "pixel {i} stroked outside the fill");
            }
            let sw = (sw as i32).min(w / 2);
            let mid_row = (h / 2) as usize * w as usize;
            let left: u32 = g[mid_row..mid_row + w as usize / 2].iter().sum();
            assert_eq!(left as i32, sw, "{w}x{h} r{r}: left edge thickness");
        }
    }

    #[test]
    fn polygon_rows_even_odd() {
        // A self-intersecting star leaves its center pentagon unfilled.
        let star = [(30, 0), (38, 40), (10, 14), (50, 14), (22, 40)];
        let mut xs = Vec::new();
        let g = grid(60, 45, (0, 0), |put| polygon_rows(&star, &mut xs, put));
        assert_eq!(g[20 * 60 + 30], 0, "even-odd hole");
        assert_eq!(g[5 * 60 + 30], 1, "star tip");
        // Degenerate input draws nothing.
        let g = grid(10, 10, (0, 0), |put| {
            polygon_rows(&[(1, 1), (5, 5)], &mut xs, put)
        });
        assert!(g.iter().all(|&n| n == 0));
    }
}
