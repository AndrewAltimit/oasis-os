//! Extension traits: fine-grained capability groupings.
//!
//! Each extension trait mirrors a subset of `SdiBackend` methods with
//! `SdiCore` as the only supertrait. Default implementations use
//! `SdiCore` primitives only, so types that do *not* implement
//! `SdiBackend` can still satisfy these traits.
//!
//! A blanket impl ensures that every `SdiBackend` implementor
//! automatically satisfies all extension traits by delegating to the
//! corresponding `SdiBackend` methods (which may be overridden).

use super::{
    Color, GradientStyle, SdiBackend, SdiCore, TextureId, arc_segments, cos_approx_f32,
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
    fn ext_fill_rounded_rect(
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
    fn ext_stroke_rect(
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
    fn ext_stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        self.ext_stroke_rect(x, y, w, h, stroke_width, color)
    }

    /// Draw a line between two points.
    fn ext_draw_line(
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
    fn ext_fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
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
    fn ext_fill_triangle(
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

    /// Draw the outline of a circle.
    fn ext_stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let _ = stroke_width;
        self.ext_fill_circle(cx, cy, radius, color)
    }
}

/// Blanket: every `SdiBackend` automatically implements `SdiShapes`.
impl<T: SdiBackend + ?Sized> SdiShapes for T {
    fn ext_fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::fill_rounded_rect(self, x, y, w, h, radius, color)
    }
    fn ext_stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::stroke_rect(self, x, y, w, h, stroke_width, color)
    }
    fn ext_stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::stroke_rounded_rect(self, x, y, w, h, radius, stroke_width, color)
    }
    fn ext_draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::draw_line(self, x1, y1, x2, y2, width, color)
    }
    fn ext_fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        SdiBackend::fill_circle(self, cx, cy, radius, color)
    }
    fn ext_fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        SdiBackend::fill_triangle(self, x1, y1, x2, y2, x3, y3, color)
    }
    fn ext_stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::stroke_circle(self, cx, cy, radius, stroke_width, color)
    }
}

// ---------------------------------------------------------------------------
// SdiGradients
// ---------------------------------------------------------------------------

/// Gradient fill operations.
pub trait SdiGradients: SdiCore {
    /// Draw a filled rectangle with a gradient.
    fn ext_fill_rect_gradient(
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
    fn ext_fill_rounded_rect_gradient(
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

/// Blanket: every `SdiBackend` automatically implements
/// `SdiGradients`.
impl<T: SdiBackend + ?Sized> SdiGradients for T {
    fn ext_fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        SdiBackend::fill_rect_gradient(self, x, y, w, h, gradient)
    }
    fn ext_fill_rounded_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        SdiBackend::fill_rounded_rect_gradient(self, x, y, w, h, radius, gradient)
    }
}

// ---------------------------------------------------------------------------
// SdiAlpha
// ---------------------------------------------------------------------------

/// Alpha blending and viewport utilities.
pub trait SdiAlpha: SdiCore {
    /// Draw a filled rectangle with explicit alpha override.
    fn ext_fill_rect_alpha(
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
    fn ext_viewport_size(&self) -> (u32, u32) {
        (480, 272)
    }

    /// Dim the entire viewport with a semi-transparent overlay.
    fn ext_dim_screen(&mut self, alpha: u8) -> Result<()> {
        let (w, h) = self.ext_viewport_size();
        self.fill_rect(0, 0, w, h, Color::rgba(0, 0, 0, alpha))
    }
}

/// Blanket: every `SdiBackend` automatically implements `SdiAlpha`.
impl<T: SdiBackend + ?Sized> SdiAlpha for T {
    fn ext_fill_rect_alpha(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        alpha: u8,
    ) -> Result<()> {
        SdiBackend::fill_rect_alpha(self, x, y, w, h, color, alpha)
    }
    fn ext_viewport_size(&self) -> (u32, u32) {
        SdiBackend::viewport_size(self)
    }
    fn ext_dim_screen(&mut self, alpha: u8) -> Result<()> {
        SdiBackend::dim_screen(self, alpha)
    }
}

// ---------------------------------------------------------------------------
// SdiText
// ---------------------------------------------------------------------------

/// Text measurement and drawing helpers.
#[allow(clippy::too_many_arguments)]
pub trait SdiText: SdiCore {
    /// Measure the height of text at the given font size.
    fn ext_measure_text_height(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 6).div_ceil(5)
    }

    /// Measure the font's ascent.
    fn ext_font_ascent(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 17).div_ceil(20)
    }

    /// Draw text with bold and italic style hints.
    fn ext_draw_text_styled(
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
    fn ext_draw_text_wrapped(
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
            self.ext_measure_text_height(font_size)
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

    /// Draw text truncated with "..." if it exceeds `max_width`.
    fn ext_draw_text_ellipsis(
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
}

/// Blanket: every `SdiBackend` automatically implements `SdiText`.
impl<T: SdiBackend + ?Sized> SdiText for T {
    fn ext_measure_text_height(&self, font_size: u16) -> u32 {
        SdiBackend::measure_text_height(self, font_size)
    }
    fn ext_font_ascent(&self, font_size: u16) -> u32 {
        SdiBackend::font_ascent(self, font_size)
    }
    fn ext_draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        SdiBackend::draw_text_styled(self, text, x, y, font_size, color, bold, italic)
    }
    fn ext_draw_text_wrapped(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
        line_height: u32,
    ) -> Result<u32> {
        SdiBackend::draw_text_wrapped(self, text, x, y, font_size, color, max_width, line_height)
    }
    fn ext_draw_text_ellipsis(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
    ) -> Result<u32> {
        SdiBackend::draw_text_ellipsis(self, text, x, y, font_size, color, max_width)
    }
}

// ---------------------------------------------------------------------------
// SdiTextures
// ---------------------------------------------------------------------------

/// Texture sub-blitting and tinting operations.
#[allow(clippy::too_many_arguments)]
pub trait SdiTextures: SdiCore {
    /// Blit a sub-rectangle from a texture.
    fn ext_blit_sub(
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
    fn ext_blit_tinted(
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
    fn ext_blit_sub_tinted(
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
        self.ext_blit_sub(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with horizontal and/or vertical flip.
    fn ext_blit_flipped(
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

/// Blanket: every `SdiBackend` automatically implements
/// `SdiTextures`.
impl<T: SdiBackend + ?Sized> SdiTextures for T {
    fn ext_blit_sub(
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
        SdiBackend::blit_sub(
            self, tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h,
        )
    }
    fn ext_blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        SdiBackend::blit_tinted(self, tex, x, y, w, h, tint)
    }
    fn ext_blit_sub_tinted(
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
        SdiBackend::blit_sub_tinted(
            self, tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h, tint,
        )
    }
    fn ext_blit_flipped(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        SdiBackend::blit_flipped(self, tex, x, y, w, h, flip_h, flip_v)
    }
}

// ---------------------------------------------------------------------------
// SdiClipTransform
// ---------------------------------------------------------------------------

/// Clip rectangle and coordinate translation stack operations.
pub trait SdiClipTransform: SdiCore {
    /// Push a clip rectangle onto the clip stack.
    fn ext_push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.set_clip_rect(x, y, w, h)
    }

    /// Pop the most recently pushed clip rectangle.
    fn ext_pop_clip_rect(&mut self) -> Result<()> {
        self.reset_clip_rect()
    }

    /// Push a coordinate origin translation.
    fn ext_push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        let _ = (dx, dy);
        Ok(())
    }

    /// Pop the most recently pushed translation.
    fn ext_pop_translate(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Blanket: every `SdiBackend` automatically implements
/// `SdiClipTransform`.
impl<T: SdiBackend + ?Sized> SdiClipTransform for T {
    fn ext_push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        SdiBackend::push_clip_rect(self, x, y, w, h)
    }
    fn ext_pop_clip_rect(&mut self) -> Result<()> {
        SdiBackend::pop_clip_rect(self)
    }
    fn ext_push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        SdiBackend::push_translate(self, dx, dy)
    }
    fn ext_pop_translate(&mut self) -> Result<()> {
        SdiBackend::pop_translate(self)
    }
}

// ---------------------------------------------------------------------------
// SdiVector
// ---------------------------------------------------------------------------

/// Vector graphics primitives (polygons, arcs, dashed lines).
#[allow(clippy::too_many_arguments)]
pub trait SdiVector: SdiShapes {
    /// Draw a filled convex polygon (triangle-fan decomposition).
    fn ext_fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 {
            return Ok(());
        }
        let v0 = points[0];
        for i in 1..points.len() - 1 {
            let v1 = points[i];
            let v2 = points[i + 1];
            self.ext_fill_triangle(v0.0, v0.1, v1.0, v1.1, v2.0, v2.1, color)?;
        }
        Ok(())
    }

    /// Draw the outline of a polygon.
    fn ext_stroke_polygon(
        &mut self,
        points: &[(i32, i32)],
        width: u16,
        color: Color,
    ) -> Result<()> {
        if points.len() < 2 {
            return Ok(());
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            self.ext_draw_line(
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
    fn ext_fill_arc(
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
            self.ext_fill_triangle(cx, cy, prev_x, prev_y, nx, ny, color)?;
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    /// Draw an arc stroke (line segments along arc).
    fn ext_stroke_arc(
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
            self.ext_draw_line(prev_x, prev_y, nx, ny, width, color)?;
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    /// Draw a dashed line between two points.
    fn ext_stroke_line_dashed(
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
            self.ext_draw_line(sx, sy, ex, ey, width, color)?;
            t += cycle;
        }
        Ok(())
    }

    /// Draw a filled polygon with a per-vertex linear gradient.
    fn ext_fill_polygon_gradient(
        &mut self,
        points: &[(i32, i32)],
        color_start: Color,
        _color_end: Color,
    ) -> Result<()> {
        self.ext_fill_polygon(points, color_start)
    }
}

/// Blanket: every `SdiBackend` automatically implements `SdiVector`.
impl<T: SdiBackend + ?Sized> SdiVector for T {
    fn ext_fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        SdiBackend::fill_polygon(self, points, color)
    }
    fn ext_stroke_polygon(
        &mut self,
        points: &[(i32, i32)],
        width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::stroke_polygon(self, points, width, color)
    }
    fn ext_fill_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) -> Result<()> {
        SdiBackend::fill_arc(self, cx, cy, radius, start_angle, end_angle, color)
    }
    fn ext_stroke_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        SdiBackend::stroke_arc(self, cx, cy, radius, start_angle, end_angle, width, color)
    }
    fn ext_stroke_line_dashed(
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
        SdiBackend::stroke_line_dashed(self, x1, y1, x2, y2, width, color, dash, gap)
    }
    fn ext_fill_polygon_gradient(
        &mut self,
        points: &[(i32, i32)],
        color_start: Color,
        color_end: Color,
    ) -> Result<()> {
        SdiBackend::fill_polygon_gradient(self, points, color_start, color_end)
    }
}

// ---------------------------------------------------------------------------
// SdiBatch
// ---------------------------------------------------------------------------

/// Batch rendering operations (begin/flush command queues).
pub trait SdiBatch: SdiCore {
    /// Begin recording draw commands into a batch.
    fn ext_begin_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Flush and execute all batched draw commands.
    fn ext_flush_batch(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Blanket: every `SdiBackend` automatically implements `SdiBatch`.
impl<T: SdiBackend + ?Sized> SdiBatch for T {
    fn ext_begin_batch(&mut self) -> Result<()> {
        SdiBackend::begin_batch(self)
    }
    fn ext_flush_batch(&mut self) -> Result<()> {
        SdiBackend::flush_batch(self)
    }
}
