//! `SdiShapes` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiShapes
// ---------------------------------------------------------------------------

/// Shape drawing primitives (rounded rects, lines, circles,
/// triangles).
#[allow(clippy::too_many_arguments)]
pub trait SdiShapes: SdiCore {
    /// Draw a filled rectangle with rounded corners.
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        color: Color,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, color)
    }

    /// Draw the outline of a rectangle.
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
        self.fill_rect(x, y, w, sw, color)?;
        self.fill_rect(x, y + h as i32 - sw as i32, w, sw, color)?;
        self.fill_rect(x, y + sw as i32, sw, h.saturating_sub(sw * 2), color)?;
        self.fill_rect(
            x + w as i32 - sw as i32,
            y + sw as i32,
            sw,
            h.saturating_sub(sw * 2),
            color,
        )?;
        Ok(())
    }

    /// Draw the outline of a rounded rectangle.
    fn stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        self.stroke_rect(x, y, w, h, stroke_width, color)
    }

    /// Draw a line between two points.
    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        if y1 == y2 {
            let lx = x1.min(x2);
            let w = (x1 - x2).unsigned_abs();
            self.fill_rect(lx, y1, w.max(1), width as u32, color)?;
        } else if x1 == x2 {
            let ly = y1.min(y2);
            let h = (y1 - y2).unsigned_abs();
            self.fill_rect(x1, ly, width as u32, h.max(1), color)?;
        } else {
            let w = width as u32;
            let dx = (x2 - x1).abs();
            let dy = -(y2 - y1).abs();
            let sx: i32 = if x1 < x2 { 1 } else { -1 };
            let sy: i32 = if y1 < y2 { 1 } else { -1 };
            let mut err = dx + dy;
            let (mut cx, mut cy) = (x1, y1);
            loop {
                self.fill_rect(cx, cy, w, w, color)?;
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
        }
        Ok(())
    }

    /// Draw a filled circle using the midpoint circle algorithm.
    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let r = radius as i32;
        let mut x = r;
        let mut y: i32 = 0;
        let mut err = 1 - r;
        while x >= y {
            self.fill_rect(cx - x, cy + y, (x * 2 + 1) as u32, 1, color)?;
            self.fill_rect(cx - x, cy - y, (x * 2 + 1) as u32, 1, color)?;
            self.fill_rect(cx - y, cy + x, (y * 2 + 1) as u32, 1, color)?;
            self.fill_rect(cx - y, cy - x, (y * 2 + 1) as u32, 1, color)?;
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
        Ok(())
    }

    /// Draw a filled triangle (scanline fill).
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
        let (mut v0, mut v1, mut v2) = ((x1, y1), (x2, y2), (x3, y3));
        if v0.1 > v1.1 {
            core::mem::swap(&mut v0, &mut v1);
        }
        if v0.1 > v2.1 {
            core::mem::swap(&mut v0, &mut v2);
        }
        if v1.1 > v2.1 {
            core::mem::swap(&mut v1, &mut v2);
        }
        let total_h = v2.1 - v0.1;
        if total_h == 0 {
            let lx = v0.0.min(v1.0).min(v2.0);
            let rx = v0.0.max(v1.0).max(v2.0);
            return self.fill_rect(lx, v0.1, (rx - lx).max(1) as u32, 1, color);
        }
        for y in v0.1..=v2.1 {
            let second_half = y >= v1.1;
            let seg_h = if second_half {
                v2.1 - v1.1
            } else {
                v1.1 - v0.1
            };
            let alpha = (y - v0.1) as f32 / total_h as f32;
            let beta = if seg_h == 0 {
                0.0
            } else if second_half {
                (y - v1.1) as f32 / seg_h as f32
            } else {
                (y - v0.1) as f32 / seg_h as f32
            };
            let mut ax = v0.0 + ((v2.0 - v0.0) as f32 * alpha) as i32;
            let mut bx = if second_half {
                v1.0 + ((v2.0 - v1.0) as f32 * beta) as i32
            } else {
                v0.0 + ((v1.0 - v0.0) as f32 * beta) as i32
            };
            if ax > bx {
                core::mem::swap(&mut ax, &mut bx);
            }
            self.fill_rect(ax, y, (bx - ax + 1) as u32, 1, color)?;
        }
        Ok(())
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
            let rw = bw + expand as u32 * 2;
            let rh = bh + expand as u32 * 2;
            if radius > 0.0 {
                let r = (radius + expand as f32) as u16;
                self.fill_rounded_rect(rx, ry, rw, rh, r, c)?;
            } else {
                self.fill_rect(rx, ry, rw, rh, c)?;
            }
        }
        Ok(())
    }

    /// Draw the outline of a circle.
    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let _ = stroke_width;
        self.fill_circle(cx, cy, radius, color)
    }
}
