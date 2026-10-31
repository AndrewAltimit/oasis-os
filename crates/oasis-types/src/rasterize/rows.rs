//! Scanline span generators shared by every backend and by the
//! `SdiShapes` / `SdiVector` trait defaults.
//!
//! Each generator reports the pixels a shape covers as horizontal spans,
//! **one call per covered row** (two for rows a ring passes through twice),
//! so a translucent fill blends every pixel exactly once. The software
//! rasterizer (`oasis-rasterize::SoftwareBuffer`) fills the spans directly,
//! the SDL backend submits them as one batched `fill_rects` call and the
//! trait defaults (used by PSP and every mock) emit one `fill_rect(.., 1)`
//! per span. Sharing the generators is what keeps the backends
//! pixel-identical: there is exactly one definition of which pixels a
//! circle, a rounded rect, a thick line or a polygon covers.
//!
//! The generators are pure integer math with no backend state, which is
//! why they live here rather than in `oasis-rasterize` (which re-exports
//! them unchanged).

use crate::geometry::{ClipRect, isqrt_i64};

/// Walk the midpoint circle of radius `r` and report every row offset
/// `o` in `0..=r` **exactly once** together with the half-width `ext` of
/// the filled disc on that row (the span is `-ext..=ext`).
///
/// The classic midpoint fill emits the same row several times (the
/// `(x, y)` octant pair revisits a row while `y` stays constant), which
/// double-blends translucent fills. This walk emits the same union of
/// pixels -- the widest extent seen for each row -- but only once.
pub fn midpoint_row_extents(r: i32, mut f: impl FnMut(i32, i32)) {
    if r < 0 {
        return;
    }
    let mut cx = 0i32;
    let mut cy = r;
    let mut d = 1 - r;
    while cx <= cy {
        // Row `cx` is visited exactly once, with its final extent `cy`.
        f(cx, cy);
        let steps_down = d >= 0;
        // Row `cy` is revisited while `cy` stays constant, with a growing
        // `cx`; its widest extent is the `cx` of the step that leaves it.
        // `cx < cy` guarantees row `cy` is never reached as a `cx` row.
        if steps_down && cx < cy {
            f(cy, cx);
        }
        cx += 1;
        if steps_down {
            cy -= 1;
            d += 2 * (cx - cy) + 1;
        } else {
            d += 2 * cx + 1;
        }
    }
}

/// Enumerate the rows of a filled `w x h` rounded rect with corner radius
/// `r` (already clamped to `w/2`, `h/2`). Calls `f(dy, x0, x1)` exactly
/// once per covered row, where the row covers columns `x0..x1` (relative
/// to the rect's left edge, half-open).
///
/// The covered pixels match the historical midpoint-based fill exactly;
/// only the repeated rows are gone.
pub fn rounded_rect_rows(w: i32, h: i32, r: i32, mut f: impl FnMut(i32, i32, i32)) {
    if w <= 0 || h <= 0 {
        return;
    }
    if r <= 0 {
        for dy in 0..h {
            f(dy, 0, w);
        }
        return;
    }
    // Columns of a row inset by `r - ext`. When `w == 2r` the pole row
    // (ext 0) keeps the two center pixels, as the inclusive-endpoint
    // `hline` always drew it.
    let span = |ext: i32| {
        let inset = r - ext;
        if w - 2 * inset > 0 {
            (inset, w - inset)
        } else {
            (inset - 1, inset + 1)
        }
    };
    midpoint_row_extents(r, |o, ext| {
        let (x0, x1) = span(ext);
        // Top arc: offset 0 is the first full-width row.
        f(r - o, x0, x1);
        // Bottom arc: offset 0 is covered by the body (or, when h == 2r,
        // coincides with a top-arc row), so only o >= 1 rows below the
        // top half are emitted.
        let dy = h - 1 - r + o;
        if o >= 1 && dy > r {
            f(dy, x0, x1);
        }
    });
    // Body rows strictly between the two arcs.
    for dy in (r + 1)..(h - r) {
        f(dy, 0, w);
    }
}

/// The part of the plane a span generator reports: `bounds` as half-open
/// `i64` ranges, cut to the `i32` coordinate range.
///
/// The generators that take bounds (lines, polygons, triangles: shapes
/// whose extent is not limited by a `u16` radius) only visit the rows
/// inside it and clip every span to its columns, so their cost is
/// proportional to the visible part of the shape. A line from `-1e9` to
/// `1e9` costs as much as one across the screen.
#[derive(Clone, Copy)]
struct Window {
    x0: i64,
    x1: i64,
    y0: i64,
    y1: i64,
}

impl Window {
    fn new(bounds: ClipRect) -> Self {
        let max = i32::MAX as i64;
        Self {
            x0: bounds.x as i64,
            x1: bounds.right().min(max),
            y0: bounds.y as i64,
            y1: bounds.bottom().min(max),
        }
    }

    /// Rows `y0..=y1` (inclusive) cut to the window, as a half-open range.
    fn rows(&self, y0: i64, y1: i64) -> core::ops::Range<i64> {
        y0.max(self.y0)..(y1 + 1).min(self.y1)
    }

    /// Report columns `a..b` of row `y` (inside the window's rows), cut to
    /// the window's columns; nothing when the cut span is empty.
    fn emit(&self, f: &mut impl FnMut(i32, i32, i32), y: i64, a: i64, b: i64) {
        let (a, b) = (a.max(self.x0), b.min(self.x1));
        if a < b {
            // In range: the window lies inside the i32 plane.
            f(y as i32, a as i32, b as i32);
        }
    }
}

/// Rows of a line from `(x1, y1)` to `(x2, y2)` stamped with a square
/// `width` x `width` brush at every Bresenham step (the brush spans
/// `-width/2 ..= width - width/2 - 1` around the step, so even widths lean
/// towards negative coordinates), limited to `bounds`.
///
/// The union of the brush stamps is reported as one half-open span
/// `f(y, x0, x1)` per row: the steps covering a row are consecutive and
/// each moves at most one pixel, so that union is always contiguous.
/// Widths `<= 1` report the single-pixel Bresenham line.
///
/// The Bresenham position of every step has a closed form (below), so the
/// span of a row is computed directly from the steps that reach it rather
/// than by walking the line: only rows inside `bounds` are visited, spans
/// are clipped to its columns, and the cost does not depend on how far the
/// line extends off-screen. Pass [`ClipRect::UNBOUNDED`] for every row.
pub fn thick_line_rows(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    width: u16,
    bounds: ClipRect,
    mut f: impl FnMut(i32, i32, i32),
) {
    let win = Window::new(bounds);
    let w = (width as i64).max(1);
    let (lo, hi) = (-(w / 2), w - w / 2 - 1);
    let (x1, y1, x2, y2) = (x1 as i64, y1 as i64, x2 as i64, y2 as i64);
    // `a`/`b`: the major/minor axis deltas. The Bresenham walk (error
    // `dx - dy`, step x on `2e >= -dy`, step y on `2e <= dx`) takes
    // exactly one major step per iteration and puts step `i` at minor
    // offset `floor((2 * b * i + a) / (2 * a))` (ties step the minor
    // axis). `minor_at` is that formula; `first_step` inverts it.
    let (dx, dy) = ((x2 - x1).abs(), (y2 - y1).abs());
    let (sx, sy) = (if x1 < x2 { 1 } else { -1 }, if y1 < y2 { 1 } else { -1 });
    let x_major = dx >= dy;
    let (a, b) = if x_major { (dx, dy) } else { (dy, dx) };
    let minor_at = |i: i64| -> i64 {
        if a == 0 {
            0
        } else {
            ((2 * b as i128 * i as i128 + a as i128) / (2 * a as i128)) as i64
        }
    };
    // The first step whose minor offset is at least `m`.
    let first_step = |m: i64| -> i64 {
        if m <= 0 {
            0
        } else if m > b {
            a + 1
        } else {
            // ceil(a * (2m - 1) / (2b)); b > 0 here since 0 < m <= b.
            let num = a as i128 * (2 * m as i128 - 1);
            let den = 2 * b as i128;
            ((num + den - 1) / den) as i64
        }
    };
    let (y_top, y_bot) = (y1.min(y2), y1.max(y2));
    for y in win.rows(y_top + lo, y_bot + hi) {
        // The steps whose brush reaches row `y` sit on rows
        // `y - hi ..= y - lo`; as offsets from `y1` along `sy`:
        let (ca, cb) = ((y - hi).max(y_top), (y - lo).min(y_bot));
        let (m0, m1) = if sy > 0 {
            (ca - y1, cb - y1)
        } else {
            (y1 - cb, y1 - ca)
        };
        // Those steps are consecutive (`i0..=i1`), and x is monotonic in
        // the step, so the row's extent comes from the two ends.
        let (i0, i1) = if x_major {
            (first_step(m0), first_step(m1 + 1) - 1)
        } else {
            (m0, m1)
        };
        let x_at = |i: i64| {
            if x_major {
                x1 + sx * i
            } else {
                x1 + sx * minor_at(i)
            }
        };
        let (xa, xb) = (x_at(i0), x_at(i1));
        win.emit(&mut f, y, xa.min(xb) + lo, xa.max(xb) + hi + 1);
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
    // i64: a u16 radius squared does not fit i32.
    let r_outer = radius as i64;
    let r_inner = (r_outer - (stroke_width as i64).max(1)).max(0);
    for dy in -r_outer..=r_outer {
        let outer_x = isqrt_i64(r_outer * r_outer - dy * dy) as i32;
        let inner_sq = r_inner * r_inner - dy * dy;
        let dy = dy as i32;
        if r_inner > 0 && inner_sq >= 0 {
            // The inner disc covers `-inner_x..=inner_x`; the ring is what
            // lies outside it.
            let inner_x = isqrt_i64(inner_sq) as i32;
            if inner_x < outer_x {
                f(dy, -outer_x, -inner_x);
                f(dy, inner_x + 1, outer_x + 1);
            }
            continue;
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
/// row coordinates and limited to `bounds` (only rows inside it are
/// visited and spans are clipped to its columns; see [`thick_line_rows`]).
///
/// Each edge covers rows `min(y0, y1) ..< max(y0, y1)` (horizontal edges
/// none); the crossings of a row are sorted and paired into spans, and
/// each pair `(a, b)` is reported as `f(y, a, b + 1)` (both crossings
/// inclusive). `xs` is scratch storage reused across calls.
pub fn polygon_rows(
    points: &[(i32, i32)],
    xs: &mut Vec<i32>,
    bounds: ClipRect,
    mut f: impl FnMut(i32, i32, i32),
) {
    if points.len() < 3 {
        return;
    }
    let win = Window::new(bounds);
    let y_min = points.iter().map(|v| v.1).min().unwrap_or(0) as i64;
    let y_max = points.iter().map(|v| v.1).max().unwrap_or(0) as i64;
    // Crossings are clamped to one column either side of the window: the
    // order (and so the even-odd pairing) is kept, and a pair entirely
    // outside collapses to an empty span.
    let (cx0, cx1) = ((win.x0 - 1).max(i32::MIN as i64), win.x1);
    let n = points.len();
    for y in win.rows(y_min, y_max) {
        xs.clear();
        for i in 0..n {
            let (x0, y0) = (points[i].0 as i64, points[i].1 as i64);
            let (x1, y1) = (points[(i + 1) % n].0 as i64, points[(i + 1) % n].1 as i64);
            if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                let t = (y - y0) as f32 / (y1 - y0) as f32;
                let x = x0 + (t * (x1 - x0) as f32) as i64;
                xs.push(x.clamp(cx0, cx1) as i32);
            }
        }
        xs.sort_unstable();
        for pair in xs.as_chunks::<2>().0 {
            if pair[0] <= pair[1] {
                win.emit(&mut f, y, pair[0] as i64, pair[1] as i64 + 1);
            }
        }
    }
}

/// Rows of a filled triangle, one call per covered row: `f(y, x0, x1)`
/// covers columns `x0..x1` (half-open) of row `y`. Only rows inside
/// `bounds` are visited and spans are clipped to its columns (see
/// [`thick_line_rows`]).
///
/// Edges are walked with [`edge_x`](crate::rasterize::edge_x) at integer
/// rows, both outermost crossings included. A triangle whose vertices all
/// share one row covers the span between its leftmost and rightmost
/// vertex. `SoftwareBuffer`, the SDL backend and the `SdiShapes` default
/// all fill these spans.
pub fn triangle_rows(
    v0: (i32, i32),
    v1: (i32, i32),
    v2: (i32, i32),
    bounds: ClipRect,
    mut f: impl FnMut(i32, i32, i32),
) {
    use crate::rasterize::edge_x;
    let win = Window::new(bounds);
    let mut verts = [v0, v1, v2];
    verts.sort_by_key(|v| v.1);
    let [(vx0, vy0), (vx1, vy1), (vx2, vy2)] = verts;

    if vy0 == vy2 {
        let min_x = vx0.min(vx1).min(vx2) as i64;
        let max_x = vx0.max(vx1).max(vx2) as i64;
        for y in win.rows(vy0 as i64, vy0 as i64) {
            win.emit(&mut f, y, min_x, max_x + 1);
        }
        return;
    }

    for y in win.rows(vy0 as i64, vy2 as i64) {
        // Inside the window, so inside the i32 range.
        let y = y as i32;
        let x_02 = edge_x(vx0, vy0, vx2, vy2, y);
        let (mut x_min, mut x_max) = (x_02, x_02);
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
        win.emit(&mut f, y as i64, x_min as i64, x_max as i64 + 1);
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
                thick_line_rows(x1, y1, x2, y2, w, ClipRect::UNBOUNDED, put)
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
        thick_line_rows(0, 0, 30, 30, 3, ClipRect::UNBOUNDED, |_, a, b| {
            widths.push(b - a)
        });
        // Interior rows of a 45-degree 3px square-brush line are 5 px wide.
        assert!(
            widths[3..widths.len() - 3].iter().all(|&w| w == 5),
            "{widths:?}"
        );
    }

    #[test]
    fn thin_line_matches_bresenham_pixels() {
        let g = grid(40, 40, (0, 0), |put| {
            thick_line_rows(1, 1, 30, 12, 1, ClipRect::UNBOUNDED, put)
        });
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

    /// Regression: the ring kept the inner disc's edge column, so a 1 px
    /// stroke was 2 px wide on the rows through the centre.
    #[test]
    fn stroke_circle_width_is_exact_on_the_axes() {
        for (r, sw) in [(10u16, 1u16), (10, 3), (18, 4)] {
            let g = grid(41, 41, (20, 20), |put| stroke_circle_rows(r, sw, put));
            let row: u32 = (0..41).map(|x| g[20 * 41 + x]).sum();
            let col: u32 = (0..41).map(|y| g[y * 41 + 20]).sum();
            assert_eq!(row, 2 * sw as u32, "r={r} sw={sw} centre row");
            assert_eq!(col, 2 * sw as u32, "r={r} sw={sw} centre column");
        }
    }

    #[test]
    fn stroke_circle_max_radius_does_not_overflow() {
        let mut rows = 0u32;
        stroke_circle_rows(u16::MAX, 2, |_, x0, x1| {
            assert!(x0 < x1);
            rows += 1;
        });
        assert!(rows > 2 * u16::MAX as u32);
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
        let g = grid(60, 45, (0, 0), |put| {
            polygon_rows(&star, &mut xs, ClipRect::UNBOUNDED, put)
        });
        assert_eq!(g[20 * 60 + 30], 0, "even-odd hole");
        assert_eq!(g[5 * 60 + 30], 1, "star tip");
        // Degenerate input draws nothing.
        let g = grid(10, 10, (0, 0), |put| {
            polygon_rows(&[(1, 1), (5, 5)], &mut xs, ClipRect::UNBOUNDED, put)
        });
        assert!(g.iter().all(|&n| n == 0));
    }

    #[test]
    fn rounded_rect_rows_visits_each_row_once() {
        for w in 1..24 {
            for h in 1..24 {
                for r in 0..=(w.min(h) / 2) {
                    let mut seen = vec![0u32; h as usize];
                    rounded_rect_rows(w, h, r, |dy, x0, x1| {
                        assert!((0..h).contains(&dy), "row {dy} outside 0..{h}");
                        assert!(0 <= x0 && x0 < x1 && x1 <= w, "w={w} r={r}: {x0}..{x1}");
                        seen[dy as usize] += 1;
                    });
                    assert!(seen.iter().all(|&n| n == 1), "w={w} h={h} r={r}: {seen:?}");
                }
            }
        }
    }

    #[test]
    fn midpoint_row_extents_visits_each_offset_once() {
        for r in 0..200 {
            let mut ext_of = vec![-1i32; r as usize + 1];
            midpoint_row_extents(r, |o, ext| {
                assert!(ext >= 0 && ext <= r);
                assert_eq!(ext_of[o as usize], -1, "r={r}: offset {o} visited twice");
                ext_of[o as usize] = ext;
            });
            assert!(ext_of.iter().all(|&e| e >= 0), "r={r}: {ext_of:?}");
            // The extent never grows with distance from the center.
            assert!(ext_of.windows(2).all(|p| p[0] >= p[1]), "r={r}: {ext_of:?}");
        }
    }

    #[test]
    fn triangle_rows_one_span_per_row_and_flat_case() {
        let mut rows = Vec::new();
        triangle_rows(
            (10, 2),
            (2, 18),
            (18, 18),
            ClipRect::UNBOUNDED,
            |y, x0, x1| rows.push((y, x0, x1)),
        );
        assert_eq!(rows.len(), 17);
        assert_eq!(rows.first(), Some(&(2, 10, 11)));
        assert_eq!(rows.last(), Some(&(18, 2, 19)));

        // All vertices on one row: leftmost to rightmost, inclusive, even
        // when the extreme vertex sorts last.
        rows.clear();
        triangle_rows((5, 5), (0, 5), (10, 5), ClipRect::UNBOUNDED, |y, x0, x1| {
            rows.push((y, x0, x1))
        });
        assert_eq!(rows, vec![(5, 0, 11)]);
    }

    #[test]
    fn triangle_rows_huge_coordinates_do_not_overflow() {
        // `edge_x` multiplies two coordinate deltas; with i32 math this
        // overflowed (a debug-build panic).
        let mut n = 0;
        triangle_rows(
            (-2_000_000_000, 0),
            (2_000_000_000, 0),
            (0, 3),
            ClipRect::UNBOUNDED,
            |_, x0, x1| {
                assert!(x0 < x1);
                n += 1;
            },
        );
        assert_eq!(n, 4);
    }

    /// The walking Bresenham the closed form in `thick_line_rows` replaced:
    /// stamp the brush at every step and union the spans per row.
    fn walked_line(x1: i32, y1: i32, x2: i32, y2: i32, width: u16) -> Vec<(i32, i32, i32)> {
        let w = (width as i32).max(1);
        let (lo, hi) = (-(w / 2), w - w / 2 - 1);
        let y_min = y1.min(y2) + lo;
        let mut spans = vec![(i32::MAX, i32::MIN); (y1.max(y2) + hi - y_min + 1) as usize];
        let (dx, dy) = ((x2 - x1).abs(), -(y2 - y1).abs());
        let (sx, sy) = (if x1 < x2 { 1 } else { -1 }, if y1 < y2 { 1 } else { -1 });
        let (mut err, mut cx, mut cy) = (dx + dy, x1, y1);
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
        spans
            .iter()
            .enumerate()
            .filter(|(_, s)| s.0 <= s.1)
            .map(|(i, s)| (y_min + i as i32, s.0, s.1 + 1))
            .collect()
    }

    #[test]
    fn thick_line_closed_form_matches_the_walk() {
        // Every direction and slope in a small box, several widths.
        for x2 in -9..=9 {
            for y2 in -9..=9 {
                for w in [0u16, 1, 2, 3, 4, 7] {
                    let mut got = Vec::new();
                    thick_line_rows(3, -2, 3 + x2, -2 + y2, w, ClipRect::UNBOUNDED, |y, a, b| {
                        got.push((y, a, b))
                    });
                    assert_eq!(
                        got,
                        walked_line(3, -2, 3 + x2, -2 + y2, w),
                        "{x2},{y2} w{w}"
                    );
                }
            }
        }
        // Long shallow and steep lines.
        for (x1, y1, x2, y2) in [(0, 0, 997, 13), (5, 400, -31, -600), (-80, 7, 1000, 6)] {
            let mut got = Vec::new();
            thick_line_rows(x1, y1, x2, y2, 3, ClipRect::UNBOUNDED, |y, a, b| {
                got.push((y, a, b))
            });
            assert_eq!(got, walked_line(x1, y1, x2, y2, 3));
        }
    }

    #[test]
    fn thick_line_bounds_clip_rows_and_columns() {
        let bounds = ClipRect::new(10, 5, 20, 8);
        let mut all = Vec::new();
        thick_line_rows(0, 0, 60, 30, 3, ClipRect::UNBOUNDED, |y, a, b| {
            all.push((y, a, b))
        });
        let want: Vec<_> = all
            .iter()
            .filter(|s| (5..13).contains(&s.0))
            .map(|&(y, a, b)| (y, a.max(10), b.min(30)))
            .filter(|s| s.1 < s.2)
            .collect();
        let mut got = Vec::new();
        thick_line_rows(0, 0, 60, 30, 3, bounds, |y, a, b| got.push((y, a, b)));
        assert_eq!(got, want);
        assert!(!got.is_empty());
    }

    /// Regression: the generators walked (and allocated) every row and
    /// step of the shape, so a line or polygon spanning +-1e9 effectively
    /// hung. Bounded to a screen they visit only its rows.
    #[test]
    fn huge_coordinates_finish_fast() {
        let screen = ClipRect::new(0, 0, 480, 272);
        let start = std::time::Instant::now();
        let mut rows = 0u32;
        let mut count = |y: i32, a: i32, b: i32| {
            assert!((0..272).contains(&y) && 0 <= a && a < b && b <= 480);
            rows += 1;
        };
        for (x1, y1, x2, y2) in [
            (-1_000_000_000, 100, 1_000_000_000, 100),
            (200, -1_000_000_000, 200, 1_000_000_000),
            (-1_000_000_000, -1_000_000_000, 1_000_000_000, 1_000_000_000),
            (i32::MIN, i32::MAX, i32::MAX, i32::MIN),
        ] {
            thick_line_rows(x1, y1, x2, y2, 5, screen, &mut count);
        }
        let mut xs = Vec::new();
        let big = [
            (-1_000_000_000, -1_000_000_000),
            (1_000_000_000, -1_000_000_000),
            (0, 1_000_000_000),
        ];
        polygon_rows(&big, &mut xs, screen, &mut count);
        triangle_rows(big[0], big[1], big[2], screen, &mut count);
        assert!(rows > 0);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn polygon_and_triangle_bounds_clip() {
        let tri = [(2, 2), (40, 6), (12, 30)];
        let bounds = ClipRect::new(8, 4, 10, 10);
        let inside = |s: &(i32, i32, i32)| (4..14).contains(&s.0);
        let clip = |&(y, a, b): &(i32, i32, i32)| (y, a.max(8), b.min(18));
        let mut xs = Vec::new();
        for poly in [true, false] {
            let (mut all, mut got) = (Vec::new(), Vec::new());
            if poly {
                polygon_rows(&tri, &mut xs, ClipRect::UNBOUNDED, |y, a, b| {
                    all.push((y, a, b))
                });
                polygon_rows(&tri, &mut xs, bounds, |y, a, b| got.push((y, a, b)));
            } else {
                triangle_rows(tri[0], tri[1], tri[2], ClipRect::UNBOUNDED, |y, a, b| {
                    all.push((y, a, b))
                });
                triangle_rows(tri[0], tri[1], tri[2], bounds, |y, a, b| {
                    got.push((y, a, b))
                });
            }
            let want: Vec<_> = all
                .iter()
                .filter(|s| inside(s))
                .map(clip)
                .filter(|s| s.1 < s.2)
                .collect();
            assert_eq!(got, want, "poly={poly}");
        }
    }
}
