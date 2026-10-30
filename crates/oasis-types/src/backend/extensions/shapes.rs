//! `SdiShapes` extension trait.

use super::spans::SpanFill;
use crate::backend::{Color, SdiCore};
use crate::error::Result;
use crate::rasterize::{
    midpoint_row_extents, rounded_rect_rows, stroke_circle_rows, stroke_rounded_rect_rows,
    thick_line_rows, triangle_rows,
};

// ---------------------------------------------------------------------------
// SdiShapes
// ---------------------------------------------------------------------------

/// Shape drawing primitives (rounded rects, lines, circles,
/// triangles).
///
/// The span-based defaults cover exactly the pixels the desktop backends
/// draw: they feed the shared span generators in [`crate::rasterize`] into
/// `fill_rect` calls (one per span, with spans that continue straight down
/// merged into one rectangle), and every pixel is covered once, so
/// translucent colours blend uniformly. PSP and all test mocks rely on
/// them.
#[allow(clippy::too_many_arguments)]
pub trait SdiShapes: SdiCore {
    /// Draw a filled rectangle with rounded corners.
    ///
    /// `radius` is clamped to half the shorter side.
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        if radius == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;
        let mut fill = SpanFill::new(self, color);
        rounded_rect_rows(w as i32, h as i32, r, |dy, x0, x1| {
            fill.span(y + dy, x + x0, x + x1);
        });
        fill.finish()
    }

    /// Draw the outline of a rectangle, `stroke_width` pixels thick and
    /// inside the rectangle.
    ///
    /// A stroke at least half as wide as the shorter side fills the whole
    /// rectangle (the four edges would otherwise overlap and double-blend).
    fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let sw = stroke_width as u32;
        if w == 0 || h == 0 || sw == 0 {
            return Ok(());
        }
        if sw * 2 >= w || sw * 2 >= h {
            return self.fill_rect(x, y, w, h, color);
        }
        let inner_h = h - sw * 2;
        self.fill_rect(x, y, w, sw, color)?;
        self.fill_rect(x, y + (h - sw) as i32, w, sw, color)?;
        self.fill_rect(x, y + sw as i32, sw, inner_h, color)?;
        self.fill_rect(x + (w - sw) as i32, y + sw as i32, sw, inner_h, color)?;
        Ok(())
    }

    /// Draw the outline of a rounded rectangle.
    ///
    /// `radius` is clamped to half the shorter side; a zero radius strokes
    /// a plain rectangle.
    fn stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        if radius == 0 {
            return self.stroke_rect(x, y, w, h, stroke_width, color);
        }
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        let mut fill = SpanFill::new(self, color);
        stroke_rounded_rect_rows(w as i32, h as i32, r, stroke_width, |dy, x0, x1| {
            fill.span(y + dy, x + x0, x + x1);
        });
        fill.finish()
    }

    /// Draw a line between two points (both endpoints included).
    ///
    /// A square `width` x `width` brush is stamped at every Bresenham step
    /// ([`thick_line_rows`]), centred on the step; widths `<= 1` draw the
    /// single-pixel Bresenham line.
    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let mut fill = SpanFill::new(self, color);
        thick_line_rows(x1, y1, x2, y2, width, |y, xs, xe| fill.span(y, xs, xe));
        fill.finish()
    }

    /// Draw a filled circle using the midpoint circle algorithm, one span
    /// per row ([`midpoint_row_extents`]).
    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let mut fill = SpanFill::new(self, color);
        midpoint_row_extents(radius as i32, |o, ext| {
            fill.span(cy + o, cx - ext, cx + ext + 1);
            if o != 0 {
                fill.span(cy - o, cx - ext, cx + ext + 1);
            }
        });
        fill.finish()
    }

    /// Draw a filled triangle, one span per row ([`triangle_rows`]).
    fn fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        let mut fill = SpanFill::new(self, color);
        triangle_rows((x1, y1), (x2, y2), (x3, y3), |y, x0, x1| {
            fill.span(y, x0, x1);
        });
        fill.finish()
    }

    /// Draw a box shadow with blur, spread, and offset.
    ///
    /// The default implementation approximates the shadow with concentric
    /// rectangles at decreasing opacity (matching the current browser behavior).
    /// GPU backends can override with a Gaussian blur shader for better quality.
    fn fill_shadow(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        blur: f32,
        spread: f32,
        offset_x: f32,
        offset_y: f32,
        color: Color,
        radius: f32,
    ) -> Result<()> {
        let bx = (x as f32 + offset_x - spread) as i32;
        let by = (y as f32 + offset_y - spread) as i32;
        let bw = (w as f32 + spread * 2.0) as u32;
        let bh = (h as f32 + spread * 2.0) as u32;
        let steps = (blur as i32).max(1);
        for i in (0..steps).rev() {
            let t = i as f32 / steps as f32;
            let alpha = ((color.a as f32) * (1.0 - t) * 0.4) as u8;
            if alpha == 0 {
                continue;
            }
            let expand = i;
            let c = Color::rgba(color.r, color.g, color.b, alpha);
            let rx = bx - expand;
            let ry = by - expand;
            let rw = bw.saturating_add(expand as u32 * 2);
            let rh = bh.saturating_add(expand as u32 * 2);
            if radius > 0.0 {
                let r = (radius + expand as f32) as u16;
                self.fill_rounded_rect(rx, ry, rw, rh, r, c)?;
            } else {
                self.fill_rect(rx, ry, rw, rh, c)?;
            }
        }
        Ok(())
    }

    /// Draw the outline of a circle: the ring between `radius` and
    /// `radius - stroke_width` ([`stroke_circle_rows`]; a zero width counts
    /// as 1). The centre stays untouched.
    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let mut fill = SpanFill::new(self, color);
        stroke_circle_rows(radius, stroke_width, |dy, x0, x1| {
            fill.span(cy + dy, cx + x0, cx + x1);
        });
        fill.finish()
    }
}
