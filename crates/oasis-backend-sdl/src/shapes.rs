//! Shape drawing primitives for the SDL3 backend.
//!
//! Contains all shape-related drawing methods (rounded rects, lines,
//! circles, triangles, polygons, arcs) and helper functions used by both
//! this module and the parent `lib.rs` (gradients, clip intersection).

use oasis_core::backend::{ArcParams, Color, SdiCore, StrokeStyle};
use oasis_core::error::Result;
use sdl3::render::FRect;

use super::{SdlBackend, fpoint, frect};

// -------------------------------------------------------------------
// Inherent shape methods on SdlBackend
// -------------------------------------------------------------------

impl SdlBackend {
    /// Submit the spans collected in `rect_batch` with a single
    /// `fill_rects` FFI call (using the current draw color).
    fn flush_rect_batch(&mut self) {
        if !self.rect_batch.is_empty() {
            let _ = self.canvas.fill_rects(self.rect_batch.as_slice());
        }
    }

    /// Fill a triangle using pre-translated screen coordinates.
    ///
    /// Used by `fill_arc` which translates the center once and computes
    /// all vertices in screen space. Only appends spans to `rect_batch`;
    /// the caller submits them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_triangle_translated(
        &mut self,
        tx1: i32,
        ty1: i32,
        tx2: i32,
        ty2: i32,
        tx3: i32,
        ty3: i32,
    ) {
        triangle_spans((tx1, ty1), (tx2, ty2), (tx3, ty3), &mut self.rect_batch);
    }

    pub(crate) fn shape_fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let (tx, ty) = self.translate(x, y);
        self.set_color(color);
        // Every covered row exactly once (translucent fills blend
        // uniformly), all submitted in one `fill_rects` call.
        self.rect_batch.clear();
        rounded_rect_spans(tx, ty, w, h, radius, &mut self.rect_batch);
        self.flush_rect_batch();
        Ok(())
    }

    pub(crate) fn shape_stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke: StrokeStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.set_color(stroke.color);
        if stroke.width == 1 {
            let _ = self.canvas.draw_rect(frect(tx, ty, w, h));
        } else {
            let sw = stroke.width as u32;
            let _ = self.canvas.fill_rect(frect(tx, ty, w, sw));
            let _ = self
                .canvas
                .fill_rect(frect(tx, ty + h as i32 - sw as i32, w, sw));
            let _ = self
                .canvas
                .fill_rect(frect(tx, ty + sw as i32, sw, h.saturating_sub(sw * 2)));
            let _ = self.canvas.fill_rect(frect(
                tx + w as i32 - sw as i32,
                ty + sw as i32,
                sw,
                h.saturating_sub(sw * 2),
            ));
        }
        Ok(())
    }

    pub(crate) fn shape_draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.set_color(color);
        if width <= 1 {
            let _ = self.canvas.draw_line(fpoint(tx1, ty1), fpoint(tx2, ty2));
        } else {
            // Square brush stamped along the Bresenham path, one span per
            // row (shared with the software rasterizer). The previous
            // "parallel lines along the normal" truncated the unit normal
            // to integers, so every non-axis-aligned thick line collapsed
            // to a 1 px line drawn `width` times.
            self.rect_batch.clear();
            let batch = &mut self.rect_batch;
            oasis_rasterize::thick_line_rows(tx1, ty1, tx2, ty2, width, |y, xs, xe| {
                push_row_span(batch, xs, xe - 1, y);
            });
            self.flush_rect_batch();
        }
        Ok(())
    }

    pub(crate) fn shape_fill_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.set_color(color);
        self.rect_batch.clear();
        circle_spans(tcx, tcy, radius as i32, &mut self.rect_batch);
        self.flush_rect_batch();
        Ok(())
    }

    pub(crate) fn shape_stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke: StrokeStyle,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.set_color(stroke.color);
        // The annulus between `radius` and `radius - width`, one or two
        // spans per row (shared with the software rasterizer). Concentric
        // 1 px midpoint rings, the previous approach, left moire gaps
        // between the rings of a thick stroke.
        self.rect_batch.clear();
        let batch = &mut self.rect_batch;
        oasis_rasterize::stroke_circle_rows(radius, stroke.width, |dy, x0, x1| {
            push_row_span(batch, tcx + x0, tcx + x1 - 1, tcy + dy);
        });
        self.flush_rect_batch();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        let (tx3, ty3) = self.translate(x3, y3);
        self.set_color(color);
        self.rect_batch.clear();
        triangle_spans((tx1, ty1), (tx2, ty2), (tx3, ty3), &mut self.rect_batch);
        self.flush_rect_batch();
        Ok(())
    }

    pub(crate) fn shape_stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke: StrokeStyle,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.shape_stroke_rect(x, y, w, h, stroke);
        }
        let (tx, ty) = self.translate(x, y);
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        self.set_color(stroke.color);
        // Filled rounded rect minus the inset inner one (shared with the
        // software rasterizer): corners have the same shape as
        // `fill_rounded_rect` and thick strokes have no gaps.
        self.rect_batch.clear();
        let batch = &mut self.rect_batch;
        oasis_rasterize::stroke_rounded_rect_rows(
            w as i32,
            h as i32,
            r,
            stroke.width,
            |dy, x0, x1| push_row_span(batch, tx + x0, tx + x1 - 1, ty + dy),
        );
        self.flush_rect_batch();
        Ok(())
    }

    pub(crate) fn shape_fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 {
            return Ok(());
        }
        self.set_color(color);

        // Collect translated vertices into the persistent scratch
        // buffer (`rect_batch` pattern) instead of allocating a fresh
        // Vec on every call. Translation is a pure offset, so applying
        // it once to (0, 0) covers every vertex.
        let (ox, oy) = self.translate(0, 0);
        self.poly_points.clear();
        self.poly_points
            .extend(points.iter().map(|&(x, y)| (x + ox, y + oy)));

        self.rect_batch.clear();
        let batch = &mut self.rect_batch;
        oasis_rasterize::polygon_rows(&self.poly_points, &mut self.poly_xs, |y, x0, x1| {
            push_row_span(batch, x0, x1 - 1, y);
        });
        self.flush_rect_batch();
        Ok(())
    }

    pub(crate) fn shape_fill_arc(&mut self, arc: ArcParams, color: Color) -> Result<()> {
        use oasis_types::backend::{arc_segments, cos_approx_f32, sin_approx_f32};
        let (tcx, tcy) = self.translate(arc.cx, arc.cy);
        self.set_color(color);
        let segments = arc_segments(arc.radius, arc.start_angle, arc.end_angle);
        let r = arc.radius as f32;
        let step = (arc.end_angle - arc.start_angle) / segments as f32;

        // Build triangle fan vertices and scanline-fill each triangle; the
        // spans of the whole fan go to SDL in one `fill_rects` call.
        self.rect_batch.clear();
        let mut prev_x = tcx + (r * cos_approx_f32(arc.start_angle)) as i32;
        let mut prev_y = tcy + (r * sin_approx_f32(arc.start_angle)) as i32;
        for i in 1..=segments {
            let angle = arc.start_angle + step * i as f32;
            let nx = tcx + (r * cos_approx_f32(angle)) as i32;
            let ny = tcy + (r * sin_approx_f32(angle)) as i32;
            self.fill_triangle_translated(tcx, tcy, prev_x, prev_y, nx, ny);
            prev_x = nx;
            prev_y = ny;
        }
        self.flush_rect_batch();
        Ok(())
    }
}

// -------------------------------------------------------------------
// Free helper functions
// -------------------------------------------------------------------

// `edge_x` is now shared via oasis_types.
pub(crate) use oasis_types::rasterize::edge_x;

/// Append the inclusive span `x0..=x1` on row `y` to `out`, growing the
/// previous rect instead when it is the same span on an adjacent row (so
/// straight-sided runs such as a rounded rect's body become one rect).
pub(crate) fn push_row_span(out: &mut Vec<FRect>, x0: i32, x1: i32, y: i32) {
    if x0 > x1 {
        return;
    }
    let w = x1 - x0 + 1;
    if let Some(last) = out.last_mut()
        && last.x as i32 == x0
        && last.w as i32 == w
    {
        let (top, h) = (last.y as i32, last.h as i32);
        if top + h == y {
            last.h += 1.0;
            return;
        }
        if top - 1 == y {
            last.y -= 1.0;
            last.h += 1.0;
            return;
        }
    }
    out.push(frect(x0, y, w as u32, 1));
}

/// Scanline spans of a filled rounded rect at screen position `(tx, ty)`.
///
/// Every covered row is emitted exactly once, so a translucent fill blends
/// each pixel once. (The old path overlapped the body rect, the top and
/// bottom strips and repeated midpoint rows, darkening parts of the shape.)
pub(crate) fn rounded_rect_spans(
    tx: i32,
    ty: i32,
    w: u32,
    h: u32,
    radius: u16,
    out: &mut Vec<FRect>,
) {
    let r = (radius as u32).min(w / 2).min(h / 2) as i32;
    oasis_rasterize::rounded_rect_rows(w as i32, h as i32, r, |dy, x0, x1| {
        push_row_span(out, tx + x0, tx + x1 - 1, ty + dy);
    });
}

/// Scanline spans of a filled circle, one per row.
pub(crate) fn circle_spans(cx: i32, cy: i32, r: i32, out: &mut Vec<FRect>) {
    oasis_rasterize::midpoint_row_extents(r, |o, ext| {
        push_row_span(out, cx - ext, cx + ext, cy + o);
        if o != 0 {
            push_row_span(out, cx - ext, cx + ext, cy - o);
        }
    });
}

/// Scanline spans of a filled triangle (screen coordinates), one per row.
pub(crate) fn triangle_spans(v0: (i32, i32), v1: (i32, i32), v2: (i32, i32), out: &mut Vec<FRect>) {
    let mut verts = [v0, v1, v2];
    verts.sort_by_key(|v| v.1);
    let (vx0, vy0) = verts[0];
    let (vx1, vy1) = verts[1];
    let (vx2, vy2) = verts[2];
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
        push_row_span(out, x_min, x_max, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_rasterize::SoftwareBuffer;

    /// Rasterize `rects` into a per-pixel hit-count grid of `w x h`.
    fn coverage(rects: &[FRect], w: usize, h: usize) -> Vec<u32> {
        let mut grid = vec![0u32; w * h];
        for r in rects {
            let (x0, y0) = (r.x as i32, r.y as i32);
            for y in y0..y0 + r.h as i32 {
                for x in x0..x0 + r.w as i32 {
                    if (0..w as i32).contains(&x) && (0..h as i32).contains(&y) {
                        grid[y as usize * w + x as usize] += 1;
                    }
                }
            }
        }
        grid
    }

    /// Pixels an opaque fill covers in the reference software rasterizer.
    fn reference_mask(w: usize, h: usize, draw: impl FnOnce(&mut SoftwareBuffer)) -> Vec<bool> {
        let mut buf = SoftwareBuffer::new(w as u32, h as u32);
        draw(&mut buf);
        buf.data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3] != 0)
            .collect()
    }

    #[test]
    fn rounded_rect_spans_cover_each_pixel_once() {
        let white = Color::rgb(255, 255, 255);
        for (w, h, r) in [
            (40u32, 30u32, 8u16),
            (40, 30, 15),
            (30, 30, 15),
            (31, 17, 8),
            (9, 40, 4),
            (2, 12, 5),
            (60, 4, 30),
        ] {
            let mut rects = Vec::new();
            rounded_rect_spans(3, 4, w, h, r, &mut rects);
            let grid = coverage(&rects, 70, 50);
            assert!(
                grid.iter().all(|&n| n <= 1),
                "{w}x{h} r{r}: pixel covered twice"
            );
            let mask = reference_mask(70, 50, |b| b.fill_rounded_rect(3, 4, w, h, r, white));
            let covered: Vec<bool> = grid.iter().map(|&n| n == 1).collect();
            assert_eq!(
                covered, mask,
                "{w}x{h} r{r}: coverage differs from rasterizer"
            );
        }
    }

    #[test]
    fn rounded_rect_spans_merge_body_rows() {
        let mut rects = Vec::new();
        rounded_rect_spans(0, 0, 200, 150, 10, &mut rects);
        // The straight body collapses into one rect; only the ~2r arc rows
        // remain individual spans (vs. ~150 draw calls before).
        assert!(rects.len() <= 2 * 10 + 2, "{} rects", rects.len());
        assert!(rects.iter().any(|r| r.h as i32 >= 150 - 2 * 10 - 2));
    }

    #[test]
    fn circle_spans_cover_each_pixel_once() {
        let white = Color::rgb(255, 255, 255);
        for r in [0, 1, 2, 5, 12, 20] {
            let mut rects = Vec::new();
            circle_spans(25, 25, r, &mut rects);
            let grid = coverage(&rects, 50, 50);
            assert!(grid.iter().all(|&n| n <= 1), "r{r}: pixel covered twice");
            let mask = reference_mask(50, 50, |b| b.fill_circle(25, 25, r as u16, white));
            let covered: Vec<bool> = grid.iter().map(|&n| n == 1).collect();
            assert_eq!(covered, mask, "r{r}: coverage differs from rasterizer");
        }
    }

    #[test]
    fn triangle_spans_one_per_row() {
        let mut rects = Vec::new();
        triangle_spans((10, 2), (2, 18), (18, 18), &mut rects);
        let grid = coverage(&rects, 20, 20);
        assert!(grid.iter().all(|&n| n <= 1));
        let rows: u32 = rects.iter().map(|r| r.h as u32).sum();
        assert_eq!(rows, 17);
    }

    /// SDL readback: a translucent rounded rect over a uniform background
    /// must come back with every covered pixel the same (blended once).
    /// Needs a display (or `SDL_VIDEO_DRIVER=dummy`/`offscreen`); run with
    /// `cargo test -p oasis-backend-sdl -- --ignored`.
    #[test]
    #[ignore]
    fn render_translucent_rounded_rect_uniform() {
        let Ok(mut backend) = SdlBackend::new("test", 64, 64) else {
            return;
        };
        backend.clear(Color::rgb(0, 0, 0)).expect("clear");
        backend
            .shape_fill_rounded_rect(4, 4, 56, 40, 12, Color::rgba(255, 255, 255, 128))
            .expect("fill");
        let pixels = backend.read_pixels(0, 0, 64, 64).expect("read_pixels");
        let mut values: Vec<u8> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0].max(p[1]).max(p[2]))
            .filter(|&v| v != 0)
            .collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), 1, "non-uniform blend: {values:?}");
    }

    #[test]
    fn push_row_span_skips_empty_and_merges_adjacent_rows() {
        let mut rects = Vec::new();
        push_row_span(&mut rects, 5, 4, 0);
        assert!(rects.is_empty());
        push_row_span(&mut rects, 2, 6, 10);
        push_row_span(&mut rects, 2, 6, 11);
        push_row_span(&mut rects, 2, 6, 9);
        assert_eq!(rects.len(), 1);
        assert_eq!((rects[0].y as i32, rects[0].h as i32), (9, 3));
        push_row_span(&mut rects, 1, 6, 12);
        assert_eq!(rects.len(), 2);
    }
}
