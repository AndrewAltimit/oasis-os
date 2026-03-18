//! Extension traits: fine-grained capability groupings.
//!
//! Each extension trait mirrors a subset of the old monolithic `SdiBackend`
//! methods, with `SdiCore` as the only supertrait.  Default implementations
//! use `SdiCore` primitives only, so a type that implements `SdiCore` can
//! opt into any extension trait with an empty `impl` block.
//!
//! `SdiBackend` is now a marker super-trait defined as
//! `SdiCore + SdiShapes + SdiGradients + SdiAlpha + SdiText + SdiTextures
//!  + SdiClipTransform + SdiVector + SdiBatch`
//! with a blanket impl, so any type satisfying all extension traits
//! automatically implements `SdiBackend`.

use super::{
    Color, GradientStyle, SdiCore, TextMetrics, TextureId, arc_segments, cos_approx_f32,
    sin_approx_f32,
};
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

// ---------------------------------------------------------------------------
// SdiGradients
// ---------------------------------------------------------------------------

/// Gradient fill operations.
pub trait SdiGradients: SdiCore {
    /// Draw a filled rectangle with a gradient.
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
        _radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, gradient.primary_color())
    }
}

// ---------------------------------------------------------------------------
// SdiAlpha
// ---------------------------------------------------------------------------

/// Alpha blending and viewport utilities.
pub trait SdiAlpha: SdiCore {
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
}

// ---------------------------------------------------------------------------
// SdiText
// ---------------------------------------------------------------------------

/// Text measurement and drawing helpers.
#[allow(clippy::too_many_arguments)]
pub trait SdiText: SdiCore {
    /// Measure the height of text at the given font size.
    fn measure_text_height(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 6).div_ceil(5)
    }

    /// Measure the font's ascent.
    fn font_ascent(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 17).div_ceil(20)
    }

    /// Return full text metrics (width, height, ascent) for a string.
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
    /// Faux-bold is implemented via double-strike (drawing at x and x+1).
    /// The default implementation ignores `italic` because a true faux-italic
    /// requires per-scanline skew which cannot be achieved with `draw_text`.
    /// Backends that support italic rendering should override this method.
    fn draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        _italic: bool,
    ) -> Result<()> {
        self.draw_text(text, x, y, font_size, color)?;
        if bold {
            self.draw_text(text, x + 1, y, font_size, color)?;
        }
        Ok(())
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
                let tw = self.measure_text(&test, font_size);
                if tw > max_width && !current_line.is_empty() {
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
}

// ---------------------------------------------------------------------------
// SdiTextures
// ---------------------------------------------------------------------------

/// Texture sub-blitting and tinting operations.
#[allow(clippy::too_many_arguments)]
pub trait SdiTextures: SdiCore {
    /// Blit a sub-rectangle from a texture.
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
}

// ---------------------------------------------------------------------------
// SdiClipTransform
// ---------------------------------------------------------------------------

/// Clip rectangle and coordinate translation stack operations.
pub trait SdiClipTransform: SdiCore {
    /// Push a clip rectangle onto the clip stack.
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

    /// Push a coordinate origin translation.
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
}

// ---------------------------------------------------------------------------
// SdiVector
// ---------------------------------------------------------------------------

/// Vector graphics primitives (polygons, arcs, dashed lines).
#[allow(clippy::too_many_arguments)]
pub trait SdiVector: SdiShapes {
    /// Draw a filled convex polygon (triangle-fan decomposition).
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 {
            return Ok(());
        }
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

    /// Draw a filled arc (pie wedge, approximated with triangles).
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

    /// Draw an arc stroke (line segments along arc).
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
    fn fill_polygon_gradient(
        &mut self,
        points: &[(i32, i32)],
        color_start: Color,
        _color_end: Color,
    ) -> Result<()> {
        self.fill_polygon(points, color_start)
    }
}

// ---------------------------------------------------------------------------
// SdiBatch
// ---------------------------------------------------------------------------

/// A text item for batched submission via [`SdiBatch::submit_text_batch`].
#[derive(Debug, Clone)]
pub struct BatchText<'a> {
    /// The text string to render.
    pub text: &'a str,
    /// X position in screen pixels.
    pub x: i32,
    /// Y position in screen pixels.
    pub y: i32,
    /// Fill color.
    pub color: Color,
}

/// A rectangle for batched submission via [`SdiBatch::submit_rect_batch`].
#[derive(Debug, Clone, Copy)]
pub struct BatchRect {
    /// X position in screen pixels.
    pub x: i32,
    /// Y position in screen pixels.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Fill color.
    pub color: Color,
}

/// Batch rendering operations (begin/flush command queues).
pub trait SdiBatch: SdiCore {
    /// Begin recording draw commands into a batch.
    fn begin_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Flush and execute all batched draw commands.
    fn flush_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Submit a batch of solid colored rectangles in a single call.
    ///
    /// Backends can override this to submit all rectangles as GPU geometry
    /// (e.g. `SDL_RenderGeometry`, PSP `sceGumDrawArray`) reducing draw call
    /// overhead and command buffer usage. The default issues individual
    /// `fill_rect` calls which is correct but not batched.
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        for r in rects {
            self.fill_rect(r.x, r.y, r.w, r.h, r.color)?;
        }
        Ok(())
    }

    /// Submit a batch of text items sharing the same font style.
    ///
    /// All items in the batch share `font_size`, `bold`, and `italic`
    /// but may have different positions and colors. Backends can override
    /// this to coalesce glyph atlas lookups. The default issues individual
    /// `draw_text` calls with faux-bold double-strike.
    fn submit_text_batch(
        &mut self,
        texts: &[BatchText<'_>],
        font_size: u16,
        bold: bool,
        _italic: bool,
    ) -> Result<()> {
        for t in texts {
            self.draw_text(t.text, t.x, t.y, font_size, t.color)?;
            if bold {
                self.draw_text(t.text, t.x + 1, t.y, font_size, t.color)?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SdiRenderTarget
// ---------------------------------------------------------------------------

/// Offscreen render target operations for compositing and tile caching.
///
/// Backends that support GPU render targets (SDL3, WebGL) should override
/// these methods. The default implementation is a no-op that renders
/// directly to the screen, preserving current behavior for backends
/// without render target support (PSP, UE5 software).
pub trait SdiRenderTarget: SdiCore {
    /// Create an offscreen render target texture of the given size.
    ///
    /// Returns a [`TextureId`] that can be passed to
    /// [`SdiRenderTarget::set_render_target`] and later blitted to the
    /// screen with [`blit`](SdiCore::blit).
    fn create_render_target(&mut self, _w: u32, _h: u32) -> Result<TextureId> {
        Err(crate::error::OasisError::Backend(
            "render targets not supported".into(),
        ))
    }

    /// Redirect all subsequent draw calls to the given render target.
    ///
    /// Pass `None` to restore drawing to the default screen target.
    fn set_render_target(&mut self, _target: Option<TextureId>) -> Result<()> {
        Ok(())
    }

    /// Query whether this backend supports offscreen render targets.
    fn supports_render_targets(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// SdiGeometry
// ---------------------------------------------------------------------------

/// Raw geometry submission for GPU-accelerated rendering.
///
/// Enables arbitrary textured/colored triangles for diagonal gradients,
/// CSS transforms, and custom shapes. Maps to `SDL_RenderGeometry` on
/// SDL3 backends.
pub trait SdiGeometry: SdiCore {
    /// Submit raw triangle geometry to the GPU.
    ///
    /// `vertices` contains position + color + optional UV data.
    /// `indices` indexes into `vertices` to form triangles (3 per tri).
    /// `texture` is an optional texture to sample; `None` uses vertex colors.
    fn render_geometry(
        &mut self,
        _vertices: &[GeometryVertex],
        _indices: &[u32],
        _texture: Option<TextureId>,
    ) -> Result<()> {
        // Default: no-op. Backends without geometry support fall back to
        // fill_rect-based approximations in the caller.
        Ok(())
    }

    /// Query whether this backend supports raw geometry submission.
    fn supports_geometry(&self) -> bool {
        false
    }
}

/// A vertex for [`SdiGeometry::render_geometry`].
#[derive(Debug, Clone, Copy)]
pub struct GeometryVertex {
    /// X position in screen pixels.
    pub x: f32,
    /// Y position in screen pixels.
    pub y: f32,
    /// Texture U coordinate (0.0..1.0). Ignored if no texture.
    pub u: f32,
    /// Texture V coordinate (0.0..1.0). Ignored if no texture.
    pub v: f32,
    /// Vertex color (premultiplied alpha).
    pub color: Color,
}

// ---------------------------------------------------------------------------
// SdiBlendMode
// ---------------------------------------------------------------------------

/// Alpha blending mode control for compositing layers.
pub trait SdiBlendMode: SdiCore {
    /// Set the active blend mode for subsequent draw operations.
    fn set_blend_mode(&mut self, _mode: BlendMode) -> Result<()> {
        Ok(())
    }

    /// Query the current blend mode.
    fn current_blend_mode(&self) -> BlendMode {
        BlendMode::Blend
    }
}

/// Blend modes for compositing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// No blending — source overwrites destination.
    None,
    /// Standard alpha blending (src * alpha + dst * (1 - alpha)).
    #[default]
    Blend,
    /// Additive blending (src + dst).
    Add,
    /// Multiplicative blending (src * dst).
    Multiply,
}
