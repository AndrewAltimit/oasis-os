//! Full rendering backend trait with extended primitives.

use super::{
    Color, GradientStyle, SdiCore, TextMetrics, TextureId, arc_segments, cos_approx_f32,
    sin_approx_f32,
};
use crate::error::Result;

/// Full rendering backend trait with extended primitives.
///
/// Extends [`SdiCore`] with 30 optional methods for shapes, gradients,
/// text system, texture operations, clip/transform stacks, and batching.
/// All have default implementations that approximate using [`SdiCore`]
/// methods, so backends can progressively override for native acceleration.
///
/// # For backend implementors
///
/// 1. Implement [`SdiCore`] with the 13 required methods
/// 2. Implement `SdiBackend` and override only the methods you can accelerate
/// 3. For test mocks: `impl SdiBackend for MyMock {}` (empty) is sufficient
#[allow(clippy::too_many_arguments)]
pub trait SdiBackend: SdiCore {
    // -----------------------------------------------------------------------
    // Extended: Shape Primitives
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with rounded corners.
    ///
    /// `radius` specifies the corner radius in pixels. If `radius` exceeds
    /// half the smaller dimension, it is clamped. A radius of 0 is equivalent
    /// to `fill_rect`.
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        color: Color,
    ) -> Result<()> {
        // Default: fall back to sharp-cornered fill_rect.
        self.fill_rect(x, y, w, h, color)
    }

    /// Draw the outline of a rectangle.
    ///
    /// `stroke_width` is drawn inward from the given bounds.
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
        // Default: fall back to sharp stroke_rect.
        self.stroke_rect(x, y, w, h, stroke_width, color)
    }

    /// Draw a line between two points.
    ///
    /// `width` is the line thickness in pixels. The default implementation
    /// handles horizontal, vertical, and diagonal lines using Bresenham's
    /// algorithm.
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
            // Bresenham's line algorithm for diagonal lines.
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
            // Fill horizontal spans for each octant pair.
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

    /// Draw a filled triangle defined by three vertices (scanline fill).
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
        // Sort vertices by y-coordinate.
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
            // Degenerate: all on same scanline.
            let lx = v0.0.min(v1.0).min(v2.0);
            let rx = v0.0.max(v1.0).max(v2.0);
            return self.fill_rect(lx, v0.1, (rx - lx).max(1) as u32, 1, color);
        }
        // Scanline fill.
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

    // -----------------------------------------------------------------------
    // Extended: Gradient Fills (Phase 2)
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with a gradient.
    ///
    /// The [`GradientStyle`] enum specifies direction and colors.
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, gradient.primary_color())
    }

    /// Draw a filled rounded rectangle with a gradient.
    fn fill_rounded_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.fill_rounded_rect(x, y, w, h, radius, gradient.primary_color())
    }

    // -----------------------------------------------------------------------
    // Extended: Alpha Utilities (Phase 2)
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with explicit alpha override.
    fn fill_rect_alpha(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        alpha: u8,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, color.with_alpha(alpha))
    }

    /// Return the current viewport dimensions `(width, height)`.
    ///
    /// The default returns the PSP native resolution (480x272). Backends
    /// should override this to return their actual canvas size.
    fn viewport_size(&self) -> (u32, u32) {
        (
            super::DEFAULT_VIEWPORT_WIDTH,
            super::DEFAULT_VIEWPORT_HEIGHT,
        )
    }

    /// Dim the entire viewport with a semi-transparent overlay.
    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        let (w, h) = self.viewport_size();
        self.fill_rect(0, 0, w, h, Color::rgba(0, 0, 0, alpha))
    }

    // -----------------------------------------------------------------------
    // Extended: Text System (Phase 3)
    // -----------------------------------------------------------------------

    /// Measure the height of text at the given font size.
    ///
    /// The default uses `ceil(font_size * 1.2)`. Backends with their own
    /// font system should override this.
    fn measure_text_height(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 6).div_ceil(5) // ceil(fs * 1.2)
    }

    /// Measure the font's ascent (baseline to top of tallest glyph).
    ///
    /// The default uses `ceil(font_size * 0.85)`, which is coordinated with
    /// `measure_text_height` so that `ascent < height` always holds.
    fn font_ascent(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 17).div_ceil(20) // ceil(fs * 0.85)
    }

    /// Return full text metrics (width, height, ascent) for a string.
    ///
    /// The default delegates to `measure_text`, `measure_text_height`, and
    /// `font_ascent`. Backends that override any of those individual methods
    /// get correct results automatically.
    fn text_metrics(&self, text: &str, font_size: u16) -> TextMetrics {
        TextMetrics {
            width: self.measure_text(text, font_size),
            height: self.measure_text_height(font_size),
            ascent: self.font_ascent(font_size),
        }
    }

    /// Measure both width and height of a text string.
    fn measure_text_extents(&self, text: &str, font_size: u16) -> (u32, u32) {
        let m = self.text_metrics(text, font_size);
        (m.width, m.height)
    }

    /// Draw text truncated with "..." if it exceeds `max_width`.
    ///
    /// Returns the actual drawn width in pixels.
    fn draw_text_ellipsis(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
    ) -> Result<u32> {
        let text_w = self.measure_text(text, font_size);
        if text_w <= max_width {
            self.draw_text(text, x, y, font_size, color)?;
            return Ok(text_w);
        }
        let ellipsis_w = self.measure_text("...", font_size);
        let target = max_width.saturating_sub(ellipsis_w);
        let mut drawn_w = 0u32;
        let mut end_byte = 0;
        for (i, ch) in text.char_indices() {
            let ch_w = self.measure_text(&text[i..i + ch.len_utf8()], font_size);
            if drawn_w + ch_w > target {
                break;
            }
            drawn_w += ch_w;
            end_byte = i + ch.len_utf8();
        }
        let truncated = format!("{}...", &text[..end_byte]);
        self.draw_text(&truncated, x, y, font_size, color)?;
        Ok(drawn_w + ellipsis_w)
    }

    /// Draw text with bold and italic style hints.
    ///
    /// Backends with bitmap fonts implement faux-bold via double-strike
    /// (drawing at x and x+1) and faux-italic via row-skewing.
    fn draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        let _ = (bold, italic);
        self.draw_text(text, x, y, font_size, color)
    }

    /// Draw multiline word-wrapped text within a bounding box.
    ///
    /// Returns the total height used in pixels.
    fn draw_text_wrapped(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
        line_height: u32,
    ) -> Result<u32> {
        let lh = if line_height > 0 {
            line_height
        } else {
            self.measure_text_height(font_size)
        };
        let mut cy = y;
        for line in text.split('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.is_empty() {
                cy += lh as i32;
                continue;
            }
            let mut current_line = String::new();
            for word in words {
                let test = if current_line.is_empty() {
                    word.to_string()
                } else {
                    format!("{current_line} {word}")
                };
                if self.measure_text(&test, font_size) > max_width && !current_line.is_empty() {
                    self.draw_text(&current_line, x, cy, font_size, color)?;
                    cy += lh as i32;
                    current_line = word.to_string();
                } else {
                    current_line = test;
                }
            }
            if !current_line.is_empty() {
                self.draw_text(&current_line, x, cy, font_size, color)?;
                cy += lh as i32;
            }
        }
        Ok((cy - y) as u32)
    }

    // -----------------------------------------------------------------------
    // Extended: Texture Operations (Phase 4)
    // -----------------------------------------------------------------------

    /// Blit a sub-rectangle from a texture (sprite sheet / atlas support).
    fn blit_sub(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        let _ = (src_x, src_y, src_w, src_h);
        self.blit(tex, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with a multiplicative color tint.
    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let _ = tint;
        self.blit(tex, x, y, w, h)
    }

    /// Blit a texture sub-rectangle with a color tint.
    fn blit_sub_tinted(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) -> Result<()> {
        let _ = tint;
        self.blit_sub(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with horizontal and/or vertical flip.
    fn blit_flipped(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        let _ = (flip_h, flip_v);
        self.blit(tex, x, y, w, h)
    }

    // -----------------------------------------------------------------------
    // Extended: Clip and Transform Stack (Phase 5)
    // -----------------------------------------------------------------------

    /// Push a clip rectangle onto the clip stack.
    ///
    /// The effective clip is the intersection of all pushed rects.
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.set_clip_rect(x, y, w, h)
    }

    /// Pop the most recently pushed clip rectangle.
    fn pop_clip_rect(&mut self) -> Result<()> {
        self.reset_clip_rect()
    }

    /// Query the current effective clip rectangle.
    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        None
    }

    /// Push a coordinate origin translation onto the transform stack.
    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        let _ = (dx, dy);
        Ok(())
    }

    /// Pop the most recently pushed translation.
    fn pop_translate(&mut self) -> Result<()> {
        Ok(())
    }

    /// Query the current cumulative translation offset.
    fn current_translate(&self) -> (i32, i32) {
        (0, 0)
    }

    /// Push a rendering region (translate + clip).
    fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.push_translate(x, y)?;
        self.push_clip_rect(0, 0, w, h)
    }

    /// Pop a previously pushed region.
    fn pop_region(&mut self) -> Result<()> {
        self.pop_clip_rect()?;
        self.pop_translate()
    }

    // -----------------------------------------------------------------------
    // Extended: Vector Graphics Primitives (Phase 7)
    // -----------------------------------------------------------------------

    /// Draw a filled convex polygon defined by 3 or more vertices.
    ///
    /// The polygon is filled using fan triangulation from the first vertex.
    /// For correct results, the polygon should be convex. Concave polygons
    /// may produce visual artifacts but will not crash.
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 {
            return Ok(());
        }
        // Fan triangulation from vertex 0.
        let v0 = points[0];
        for i in 1..points.len() - 1 {
            let v1 = points[i];
            let v2 = points[i + 1];
            self.fill_triangle(v0.0, v0.1, v1.0, v1.1, v2.0, v2.1, color)?;
        }
        Ok(())
    }

    /// Draw the outline of a polygon.
    fn stroke_polygon(&mut self, points: &[(i32, i32)], width: u16, color: Color) -> Result<()> {
        if points.len() < 2 {
            return Ok(());
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            self.draw_line(
                points[i].0,
                points[i].1,
                points[j].0,
                points[j].1,
                width,
                color,
            )?;
        }
        Ok(())
    }

    /// Draw a filled arc (pie wedge) from `start_angle` to `end_angle`.
    ///
    /// Angles are in radians, measured clockwise from the positive X axis
    /// (3 o'clock position). A full circle is `0.0..TAU`.
    fn fill_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) -> Result<()> {
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;
        let mut prev_x = cx + (r * cos_approx_f32(start_angle)) as i32;
        let mut prev_y = cy + (r * sin_approx_f32(start_angle)) as i32;
        for i in 1..=segments {
            let angle = start_angle + step * i as f32;
            let nx = cx + (r * cos_approx_f32(angle)) as i32;
            let ny = cy + (r * sin_approx_f32(angle)) as i32;
            self.fill_triangle(cx, cy, prev_x, prev_y, nx, ny, color)?;
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    /// Draw an arc stroke (partial circle outline) from `start_angle`
    /// to `end_angle`. Angles in radians, clockwise from 3 o'clock.
    fn stroke_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;
        let mut prev_x = cx + (r * cos_approx_f32(start_angle)) as i32;
        let mut prev_y = cy + (r * sin_approx_f32(start_angle)) as i32;
        for i in 1..=segments {
            let angle = start_angle + step * i as f32;
            let nx = cx + (r * cos_approx_f32(angle)) as i32;
            let ny = cy + (r * sin_approx_f32(angle)) as i32;
            self.draw_line(prev_x, prev_y, nx, ny, width, color)?;
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    /// Draw a dashed line between two points.
    ///
    /// `dash` is the length of each drawn segment in pixels.
    /// `gap` is the length of each space between segments.
    fn stroke_line_dashed(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
        dash: u16,
        gap: u16,
    ) -> Result<()> {
        let dx = (x2 - x1) as f32;
        let dy = (y2 - y1) as f32;
        let total_len = (dx * dx + dy * dy).sqrt();
        if total_len < 1.0 {
            return Ok(());
        }
        let ux = dx / total_len;
        let uy = dy / total_len;
        let cycle = dash as f32 + gap as f32;
        let mut t = 0.0f32;
        while t < total_len {
            let seg_end = (t + dash as f32).min(total_len);
            let sx = x1 + (ux * t) as i32;
            let sy = y1 + (uy * t) as i32;
            let ex = x1 + (ux * seg_end) as i32;
            let ey = y1 + (uy * seg_end) as i32;
            self.draw_line(sx, sy, ex, ey, width, color)?;
            t += cycle;
        }
        Ok(())
    }

    /// Draw a filled polygon with a per-vertex linear gradient.
    ///
    /// `color_start` is applied at the topmost vertex, `color_end` at the
    /// bottommost. Intermediate vertices are interpolated by Y position.
    /// Falls back to solid `color_start` fill by default.
    fn fill_polygon_gradient(
        &mut self,
        points: &[(i32, i32)],
        color_start: Color,
        _color_end: Color,
    ) -> Result<()> {
        self.fill_polygon(points, color_start)
    }

    // -----------------------------------------------------------------------
    // Extended: Batch Rendering (Phase 6)
    // -----------------------------------------------------------------------

    /// Begin recording draw commands into a batch.
    fn begin_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Flush and execute all batched draw commands.
    fn flush_batch(&mut self) -> Result<()> {
        Ok(())
    }
}
