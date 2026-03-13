//! Software RGBA framebuffer renderer.
//!
//! Implements `SdiBackend` by drawing into a `Vec<u8>` RGBA buffer. UE5 reads
//! the buffer via `oasis_get_buffer()` and copies it to a `UTexture2D`.
//!
//! All extended primitives (rounded rects, lines, circles, triangles,
//! gradients, sub-rect blits, clip/transform stacks) are software-rasterized
//! into the pixel buffer.

use std::rc::Rc;

use oasis_core::backend::{
    Color, GradientStyle, SdiBackend, TextureId, texture_not_found, validate_rgba_data,
};
use oasis_core::error::Result;
use oasis_types::backend::SdiCore;
use oasis_types::color::lerp_color_ratio;
use oasis_types::geometry::ClipRect;
use oasis_types::rasterize::{self, PixelSink};

use crate::font;

/// A stored texture for later blitting.
struct Texture {
    width: u32,
    height: u32,
    data: Rc<Vec<u8>>,
}

/// Software RGBA framebuffer renderer for UE5 integration.
///
/// All rendering operations write directly to an RGBA pixel buffer.
/// The buffer is exposed to UE5 via the FFI layer. A dirty flag tracks
/// whether the buffer has changed since the last read.
pub struct Ue5Backend {
    width: u32,
    height: u32,
    buffer: Vec<u8>,
    dirty: bool,
    textures: Vec<Option<Texture>>,
    clip: Option<ClipRect>,
    clip_stack: Vec<ClipRect>,
    translate_stack: Vec<(i32, i32)>,
    cumulative_translate: (i32, i32),
}

/// Compute the intersection of two clip rectangles (convenience wrapper).
fn intersect_clip(a: &ClipRect, b: &ClipRect) -> Option<ClipRect> {
    a.intersect(b)
}

impl Ue5Backend {
    /// Create a new backend with the given resolution.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 4) as usize;
        Self {
            width,
            height,
            buffer: vec![0; size],
            dirty: true,
            textures: Vec::new(),
            clip: None,
            clip_stack: Vec::new(),
            translate_stack: Vec::new(),
            cumulative_translate: (0, 0),
        }
    }

    /// Get a read-only reference to the RGBA pixel buffer.
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Whether the buffer has been modified since the last `clear_dirty()`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag (called after UE5 reads the buffer).
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Buffer dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Blit raw RGBA pixels into the framebuffer at the given position.
    pub fn blit_rgba(&mut self, x: u32, y: u32, w: u32, h: u32, pixels: &[u8]) {
        let stride = (self.width * 4) as usize;
        let src_stride = (w * 4) as usize;
        for row in 0..h {
            let dy = y + row;
            if dy >= self.height {
                break;
            }
            let dst_start = (dy as usize) * stride + (x as usize) * 4;
            let src_start = (row as usize) * src_stride;
            let copy_w = w.min(self.width.saturating_sub(x)) as usize * 4;
            if dst_start + copy_w <= self.buffer.len() && src_start + copy_w <= pixels.len() {
                self.buffer[dst_start..dst_start + copy_w]
                    .copy_from_slice(&pixels[src_start..src_start + copy_w]);
            }
        }
        self.dirty = true;
    }

    /// Apply cumulative translation to coordinates.
    fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x + self.cumulative_translate.0,
            y + self.cumulative_translate.1,
        )
    }

    /// Set a single pixel. Performs bounds and clip checking.
    fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 {
            return;
        }
        let (ux, uy) = (x as u32, y as u32);
        if ux >= self.width || uy >= self.height {
            return;
        }
        // Clip check.
        if let Some(clip) = &self.clip
            && (x < clip.x
                || y < clip.y
                || ux >= (clip.x as u32).saturating_add(clip.w)
                || uy >= (clip.y as u32).saturating_add(clip.h))
        {
            return;
        }
        let offset = ((uy * self.width + ux) * 4) as usize;
        // Alpha blending (source over).
        if color.a == 255 {
            self.buffer[offset] = color.r;
            self.buffer[offset + 1] = color.g;
            self.buffer[offset + 2] = color.b;
            self.buffer[offset + 3] = 255;
        } else if color.a > 0 {
            let sa = color.a as u16;
            let da = 255 - sa;
            self.buffer[offset] =
                ((color.r as u16 * sa + self.buffer[offset] as u16 * da + 127) / 255) as u8;
            self.buffer[offset + 1] =
                ((color.g as u16 * sa + self.buffer[offset + 1] as u16 * da + 127) / 255) as u8;
            self.buffer[offset + 2] =
                ((color.b as u16 * sa + self.buffer[offset + 2] as u16 * da + 127) / 255) as u8;
            self.buffer[offset + 3] = 255;
        }
    }

    /// Fill a horizontal span of pixels. Clips to bounds and active clip rect.
    /// Much faster than per-pixel `set_pixel` for solid-color fills.
    fn fill_span(&mut self, y: i32, x_start: i32, x_end: i32, color: Color) {
        if y < 0 || y >= self.height as i32 {
            return;
        }
        // Determine effective x range after clipping.
        let mut xs = x_start.max(0);
        let mut xe = x_end.min(self.width as i32);
        if let Some(clip) = &self.clip {
            xs = xs.max(clip.x);
            xe = xe.min(clip.x + clip.w as i32);
            if y < clip.y || y >= clip.y + clip.h as i32 {
                return;
            }
        }
        if xs >= xe {
            return;
        }
        let row_offset = (y as usize * self.width as usize) * 4;
        if color.a == 255 {
            // Opaque: direct write, no blending.
            for x in xs..xe {
                let offset = row_offset + x as usize * 4;
                self.buffer[offset] = color.r;
                self.buffer[offset + 1] = color.g;
                self.buffer[offset + 2] = color.b;
                self.buffer[offset + 3] = 255;
            }
        } else if color.a > 0 {
            let sa = color.a as u16;
            let da = 255 - sa;
            for x in xs..xe {
                let offset = row_offset + x as usize * 4;
                self.buffer[offset] =
                    ((color.r as u16 * sa + self.buffer[offset] as u16 * da + 127) / 255) as u8;
                self.buffer[offset + 1] =
                    ((color.g as u16 * sa + self.buffer[offset + 1] as u16 * da + 127) / 255) as u8;
                self.buffer[offset + 2] =
                    ((color.b as u16 * sa + self.buffer[offset + 2] as u16 * da + 127) / 255) as u8;
                self.buffer[offset + 3] = 255;
            }
        }
    }

    /// Draw a horizontal span (faster than pixel-by-pixel for solid fills).
    fn hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        let start = x1.min(x2);
        let end = x1.max(x2) + 1; // fill_span uses exclusive end
        self.fill_span(y, start, end, color);
    }

    /// Get texture data via `Rc::clone` (O(1) refcount bump, no data copy).
    fn get_texture_data(&self, tex: TextureId) -> Result<(u32, u32, Rc<Vec<u8>>)> {
        let idx = tex.0 as usize;
        let texture = self
            .textures
            .get(idx)
            .and_then(|t| t.as_ref())
            .ok_or_else(|| texture_not_found(tex.0))?;
        Ok((texture.width, texture.height, Rc::clone(&texture.data)))
    }
}

impl PixelSink for Ue5Backend {
    fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        self.hline(x1, x2, y, color);
    }
}

impl SdiCore for Ue5Backend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.buffer = vec![0; (width * height * 4) as usize];
        self.dirty = true;
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        for pixel in self.buffer.chunks_exact_mut(4) {
            pixel[0] = color.r;
            pixel[1] = color.g;
            pixel[2] = color.b;
            pixel[3] = color.a;
        }
        self.dirty = true;
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        for dy in 0..h as i32 {
            self.fill_span(ty + dy, tx, tx + w as i32, color);
        }
        self.dirty = true;
        Ok(())
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let scale = if font_size >= 8 {
            (font_size / 8) as i32
        } else {
            1
        };

        let mut cx = tx;
        for ch in text.chars() {
            let glyph_data = font::glyph(ch);
            let (left_pad, advance) = font::glyph_metrics(ch);
            let left_pad = left_pad as i32;
            for row in 0..8i32 {
                let bits = glyph_data[row as usize];
                for col in 0..8i32 {
                    if bits & (0x80 >> col) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.set_pixel(
                                    cx + (col - left_pad) * scale + sx,
                                    ty + row * scale + sy,
                                    color,
                                );
                            }
                        }
                    }
                }
            }
            cx += advance as i32 * scale;
        }
        self.dirty = true;
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        for dy in 0..h {
            for dx in 0..w {
                let src_x = (dx * tex_w / w) as usize;
                let src_y = (dy * tex_h / h) as usize;
                let src_offset = (src_y * tex_w as usize + src_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        tex_data[src_offset],
                        tex_data[src_offset + 1],
                        tex_data[src_offset + 2],
                        tex_data[src_offset + 3],
                    );
                    self.set_pixel(tx + dx as i32, ty + dy as i32, color);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;

        let texture = Texture {
            width,
            height,
            data: Rc::new(rgba_data.to_vec()),
        };

        for (i, slot) in self.textures.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(texture);
                return Ok(TextureId(i as u64));
            }
        }
        let id = self.textures.len();
        self.textures.push(Some(texture));
        Ok(TextureId(id as u64))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        let idx = tex.0 as usize;
        if idx < self.textures.len() {
            self.textures[idx] = None;
        }
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.clip = Some(ClipRect { x, y, w, h });
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.clip = None;
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        for row in 0..h {
            let sy = (y as u32 + row) as usize;
            if sy >= self.height as usize {
                continue;
            }
            for col in 0..w {
                let sx = (x as u32 + col) as usize;
                if sx >= self.width as usize {
                    continue;
                }
                let src_idx = (sy * self.width as usize + sx) * 4;
                let dst_idx = (row as usize * w as usize + col as usize) * 4;
                out[dst_idx..dst_idx + 4].copy_from_slice(&self.buffer[src_idx..src_idx + 4]);
            }
        }
        Ok(out)
    }

    fn shutdown(&mut self) -> Result<()> {
        self.buffer.clear();
        self.textures.clear();
        self.clip_stack.clear();
        self.translate_stack.clear();
        self.cumulative_translate = (0, 0);
        log::info!("UE5 backend shut down");
        Ok(())
    }
}

impl SdiBackend for Ue5Backend {
    // -------------------------------------------------------------------
    // Extended: Shape Primitives
    // -------------------------------------------------------------------

    fn fill_rounded_rect(
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
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;

        // Center rect (full width, excluding top/bottom radius strips).
        for dy in r..(h as i32 - r) {
            self.hline(tx, tx + w as i32 - 1, ty + dy, color);
        }

        // Top and bottom strips with rounded corners (midpoint circle).
        let mut cx = 0i32;
        let mut cy = r;
        let mut d = 1 - r;
        while cx <= cy {
            // Top-left to top-right scanlines.
            self.hline(tx + r - cy, tx + w as i32 - 1 - r + cy, ty + r - cx, color);
            if cx != 0 {
                self.hline(
                    tx + r - cy,
                    tx + w as i32 - 1 - r + cy,
                    ty + h as i32 - 1 - r + cx,
                    color,
                );
            }
            if cx != cy {
                self.hline(tx + r - cx, tx + w as i32 - 1 - r + cx, ty + r - cy, color);
                self.hline(
                    tx + r - cx,
                    tx + w as i32 - 1 - r + cx,
                    ty + h as i32 - 1 - r + cy,
                    color,
                );
            } else {
                self.hline(
                    tx + r - cx,
                    tx + w as i32 - 1 - r + cx,
                    ty + h as i32 - 1 - r + cy,
                    color,
                );
            }

            cx += 1;
            if d < 0 {
                d += 2 * cx + 1;
            } else {
                cy -= 1;
                d += 2 * (cx - cy) + 1;
            }
        }
        self.dirty = true;
        Ok(())
    }

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
        // Top.
        self.fill_rect(x, y, w, sw, color)?;
        // Bottom.
        self.fill_rect(x, y + h as i32 - sw as i32, w, sw, color)?;
        // Left.
        self.fill_rect(x, y + sw as i32, sw, h.saturating_sub(sw * 2), color)?;
        // Right.
        self.fill_rect(
            x + w as i32 - sw as i32,
            y + sw as i32,
            sw,
            h.saturating_sub(sw * 2),
            color,
        )?;
        Ok(())
    }

    fn draw_line(
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
        let w = width as i32;

        // Bresenham's line algorithm.
        let dx = (tx2 - tx1).abs();
        let dy = -(ty2 - ty1).abs();
        let sx = if tx1 < tx2 { 1 } else { -1 };
        let sy = if ty1 < ty2 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = tx1;
        let mut cy = ty1;

        loop {
            // Draw a block of pixels for line width.
            if w <= 1 {
                self.set_pixel(cx, cy, color);
            } else {
                let half = w / 2;
                for wy in -half..=(w - half - 1) {
                    for wx in -half..=(w - half - 1) {
                        self.set_pixel(cx + wx, cy + wy, color);
                    }
                }
            }

            if cx == tx2 && cy == ty2 {
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
        self.dirty = true;
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        let r = radius as i32;

        // Midpoint circle algorithm with horizontal fill spans.
        let mut x = 0i32;
        let mut y = r;
        let mut d = 1 - r;

        while x <= y {
            self.hline(tcx - y, tcx + y, tcy + x, color);
            if x != 0 {
                self.hline(tcx - y, tcx + y, tcy - x, color);
            }
            if x != y {
                self.hline(tcx - x, tcx + x, tcy + y, color);
                self.hline(tcx - x, tcx + x, tcy - y, color);
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        let r_outer = radius as i32;
        let r_inner = (radius as i32 - stroke_width as i32).max(0);

        // Scanline approach: for each row, compute outer and inner x extents.
        for dy in -r_outer..=r_outer {
            let y = tcy + dy;
            // Outer circle extent at this row.
            let outer_sq = r_outer * r_outer - dy * dy;
            if outer_sq < 0 {
                continue;
            }
            let outer_x = isqrt(outer_sq as u32) as i32;

            if r_inner > 0 {
                let inner_sq = r_inner * r_inner - dy * dy;
                if inner_sq > 0 {
                    let inner_x = isqrt(inner_sq as u32) as i32;
                    // Draw left arc.
                    self.hline(tcx - outer_x, tcx - inner_x, y, color);
                    // Draw right arc.
                    self.hline(tcx + inner_x, tcx + outer_x, y, color);
                    continue;
                }
            }
            // Full span (inner circle doesn't reach this row).
            self.hline(tcx - outer_x, tcx + outer_x, y, color);
        }
        self.dirty = true;
        Ok(())
    }

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
        let v0 = self.translate(x1, y1);
        let v1 = self.translate(x2, y2);
        let v2 = self.translate(x3, y3);
        rasterize::rasterize_triangle(self, v0, v1, v2, color);
        self.dirty = true;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Gradient Fills
    // -------------------------------------------------------------------

    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                let h_max = h.saturating_sub(1).max(1);
                for dy in 0..h as i32 {
                    let color = lerp_color_ratio(top, bottom, dy as u32, h_max);
                    self.fill_span(ty + dy, tx, tx + w as i32, color);
                }
            },
            GradientStyle::Horizontal { left, right } => {
                let w_max = w.saturating_sub(1).max(1);
                for dx in 0..w as i32 {
                    let color = lerp_color_ratio(left, right, dx as u32, w_max);
                    for dy in 0..h as i32 {
                        self.set_pixel(tx + dx, ty + dy, color);
                    }
                }
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                let h_max = h.saturating_sub(1).max(1);
                let w_max = w.saturating_sub(1).max(1);
                for dy in 0..h as i32 {
                    let left = lerp_color_ratio(top_left, bottom_left, dy as u32, h_max);
                    let right = lerp_color_ratio(top_right, bottom_right, dy as u32, h_max);
                    for dx in 0..w as i32 {
                        let color = lerp_color_ratio(left, right, dx as u32, w_max);
                        self.set_pixel(tx + dx, ty + dy, color);
                    }
                }
            },
        }
        self.dirty = true;
        Ok(())
    }

    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(0, 0, self.width, self.height, Color::rgba(0, 0, 0, alpha))
    }

    // -------------------------------------------------------------------
    // Extended: Text System
    // -------------------------------------------------------------------

    fn measure_text_height(&self, font_size: u16) -> u32 {
        // 8x8 bitmap font: line height matches the scaled glyph height.
        let scale = if font_size >= 8 {
            (font_size / 8) as u32
        } else {
            1
        };
        8 * scale
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        // Bitmap font ascent is the full glyph height (no descenders).
        let scale = if font_size >= 8 {
            (font_size / 8) as u32
        } else {
            1
        };
        8 * scale
    }

    // -------------------------------------------------------------------
    // Extended: Texture Operations
    // -------------------------------------------------------------------

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
        let (tex_w, _tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let sx = src_x + (dx * src_w / dst_w.max(1));
                let sy = src_y + (dy * src_h / dst_h.max(1));
                let src_offset = (sy as usize * tex_w as usize + sx as usize) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        tex_data[src_offset],
                        tex_data[src_offset + 1],
                        tex_data[src_offset + 2],
                        tex_data[src_offset + 3],
                    );
                    self.set_pixel(tx + dx as i32, ty + dy as i32, color);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        for dy in 0..h {
            for dx in 0..w {
                let src_x = (dx * tex_w / w) as usize;
                let src_y = (dy * tex_h / h) as usize;
                let src_offset = (src_y * tex_w as usize + src_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        ((tex_data[src_offset] as u16 * tint.r as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 1] as u16 * tint.g as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 2] as u16 * tint.b as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 3] as u16 * tint.a as u16 + 127) / 255) as u8,
                    );
                    self.set_pixel(tx + dx as i32, ty + dy as i32, color);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

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
        let (tex_w, _tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let sx = src_x + (dx * src_w / dst_w.max(1));
                let sy = src_y + (dy * src_h / dst_h.max(1));
                let src_offset = (sy as usize * tex_w as usize + sx as usize) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        ((tex_data[src_offset] as u16 * tint.r as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 1] as u16 * tint.g as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 2] as u16 * tint.b as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 3] as u16 * tint.a as u16 + 127) / 255) as u8,
                    );
                    self.set_pixel(tx + dx as i32, ty + dy as i32, color);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

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
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        for dy in 0..h {
            for dx in 0..w {
                let sample_x = if flip_h {
                    ((w - 1 - dx) * tex_w / w) as usize
                } else {
                    (dx * tex_w / w) as usize
                };
                let sample_y = if flip_v {
                    ((h - 1 - dy) * tex_h / h) as usize
                } else {
                    (dy * tex_h / h) as usize
                };
                let src_offset = (sample_y * tex_w as usize + sample_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        tex_data[src_offset],
                        tex_data[src_offset + 1],
                        tex_data[src_offset + 2],
                        tex_data[src_offset + 3],
                    );
                    self.set_pixel(tx + dx as i32, ty + dy as i32, color);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Clip and Transform Stack
    // -------------------------------------------------------------------

    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let new_clip = ClipRect { x: tx, y: ty, w, h };
        if let Some(current) = self.clip {
            self.clip_stack.push(current);
            self.clip = intersect_clip(&current, &new_clip).or(Some(ClipRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }));
        } else {
            self.clip_stack.push(ClipRect {
                x: 0,
                y: 0,
                w: self.width,
                h: self.height,
            });
            self.clip = Some(new_clip);
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        if let Some(previous) = self.clip_stack.pop() {
            if previous.x == 0
                && previous.y == 0
                && previous.w == self.width
                && previous.h == self.height
            {
                // Was the sentinel for "no clip active".
                self.clip = None;
            } else {
                self.clip = Some(previous);
            }
        } else {
            self.clip = None;
        }
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.clip.map(|c| (c.x, c.y, c.w, c.h))
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(self.cumulative_translate);
        self.cumulative_translate.0 += dx;
        self.cumulative_translate.1 += dy;
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        if let Some(prev) = self.translate_stack.pop() {
            self.cumulative_translate = prev;
        }
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.cumulative_translate
    }
}

/// Integer square root (delegates to shared rasterizer).
fn isqrt(n: u32) -> u32 {
    rasterize::isqrt(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_buffer() {
        let backend = Ue5Backend::new(480, 272);
        assert_eq!(backend.buffer().len(), 480 * 272 * 4);
        assert_eq!(backend.dimensions(), (480, 272));
    }

    #[test]
    fn clear_fills_buffer() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[1], 0);
        assert_eq!(backend.buffer()[2], 0);
        assert_eq!(backend.buffer()[3], 255);
        let last = backend.buffer().len() - 4;
        assert_eq!(backend.buffer()[last], 255);
    }

    #[test]
    fn fill_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(2, 2, 3, 3, Color::rgb(0, 255, 0))
            .unwrap();
        let offset = (2 * 10 + 2) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        assert_eq!(backend.buffer()[offset + 1], 255);
        assert_eq!(backend.buffer()[0], 0);
        assert_eq!(backend.buffer()[1], 0);
    }

    #[test]
    fn fill_rect_clips_negative() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(-2, -2, 5, 5, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn draw_text_renders_characters() {
        let mut backend = Ue5Backend::new(100, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_text("A", 0, 0, 8, Color::rgb(255, 255, 255))
            .unwrap();
        let has_white = backend
            .buffer()
            .chunks_exact(4)
            .any(|px| px[0] == 255 && px[1] == 255 && px[2] == 255);
        assert!(has_white);
    }

    #[test]
    fn draw_text_scaled() {
        let mut backend = Ue5Backend::new(100, 40);
        backend.clear(Color::BLACK).unwrap();
        backend.draw_text("X", 0, 0, 16, Color::WHITE).unwrap();
        let white_count = backend
            .buffer()
            .chunks_exact(4)
            .filter(|px| px[0] == 255)
            .count();
        assert!(white_count > 20);
    }

    #[test]
    fn load_and_blit_texture() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        let tex_data = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.blit(tex_id, 1, 1, 2, 2).unwrap();
        let offset = (10 + 1) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        assert_eq!(backend.buffer()[offset + 1], 0);
    }

    #[test]
    fn destroy_texture_invalidates() {
        let mut backend = Ue5Backend::new(10, 10);
        let tex_data = vec![0u8; 2 * 2 * 4];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.destroy_texture(tex_id).unwrap();
        assert!(backend.blit(tex_id, 0, 0, 2, 2).is_err());
    }

    #[test]
    fn texture_data_size_mismatch() {
        let mut backend = Ue5Backend::new(10, 10);
        assert!(backend.load_texture(2, 2, &[0; 8]).is_err());
    }

    #[test]
    fn dirty_flag_tracking() {
        let mut backend = Ue5Backend::new(4, 4);
        assert!(backend.is_dirty());
        backend.clear_dirty();
        assert!(!backend.is_dirty());
        backend.clear(Color::BLACK).unwrap();
        assert!(backend.is_dirty());
    }

    #[test]
    fn clip_rect_restricts_drawing() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend.set_clip_rect(2, 2, 3, 3).unwrap();
        backend
            .fill_rect(0, 0, 10, 10, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 0);
        let offset = (3 * 10 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        backend.reset_clip_rect().unwrap();
        backend.fill_rect(0, 0, 1, 1, Color::WHITE).unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn shutdown_clears_state() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.shutdown().unwrap();
        assert!(backend.buffer().is_empty());
    }

    #[test]
    fn texture_slot_reuse() {
        let mut backend = Ue5Backend::new(4, 4);
        let data = vec![0u8; 4];
        let id0 = backend.load_texture(1, 1, &data).unwrap();
        let id1 = backend.load_texture(1, 1, &data).unwrap();
        backend.destroy_texture(id0).unwrap();
        let id2 = backend.load_texture(1, 1, &data).unwrap();
        assert_eq!(id2.0, id0.0);
        assert_ne!(id1.0, id2.0);
    }

    // -------------------------------------------------------------------
    // Extended primitive tests
    // -------------------------------------------------------------------

    #[test]
    fn fill_rounded_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rounded_rect(2, 2, 16, 16, 4, Color::rgb(0, 255, 0))
            .unwrap();
        // Center pixel should be green.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
        // Corner pixel (2,2) should NOT be filled (inside the radius).
        let corner = (2 * 20 + 2) * 4;
        assert_eq!(backend.buffer()[corner], 0);
    }

    #[test]
    fn draw_line_horizontal() {
        let mut backend = Ue5Backend::new(20, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(2, 5, 18, 5, 1, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (10, 5) should be red.
        let offset = (5 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn draw_line_diagonal() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(0, 0, 19, 19, 1, Color::rgb(0, 0, 255))
            .unwrap();
        // Pixel at (10, 10) should be blue.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 2], 255);
    }

    #[test]
    fn fill_circle_draws() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_circle(15, 15, 10, Color::rgb(255, 0, 0))
            .unwrap();
        // Center should be red.
        let offset = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn fill_triangle_draws() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_triangle(10, 2, 2, 18, 18, 18, Color::rgb(0, 255, 0))
            .unwrap();
        // A point inside the triangle should be green.
        let offset = (14 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
    }

    #[test]
    fn gradient_v_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Vertical {
                    top: Color::WHITE,
                    bottom: Color::BLACK,
                },
            )
            .unwrap();
        // Top pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Bottom pixel should be black.
        let last_row = (9 * 10) * 4;
        assert_eq!(backend.buffer()[last_row], 0);
    }

    #[test]
    fn gradient_h_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Horizontal {
                    left: Color::WHITE,
                    right: Color::BLACK,
                },
            )
            .unwrap();
        // Left pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Right pixel should be black.
        let right = 9 * 4;
        assert_eq!(backend.buffer()[right], 0);
    }

    #[test]
    fn clip_stack_nesting() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        // Push outer clip.
        backend.push_clip_rect(2, 2, 16, 16).unwrap();
        // Push inner clip.
        backend.push_clip_rect(5, 5, 10, 10).unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (0,0) should be black (outside both clips).
        assert_eq!(backend.buffer()[0], 0);
        // Pixel at (3,3) should be black (outside inner clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        // Pixel at (7,7) should be red (inside both clips).
        let offset = (7 * 20 + 7) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        // Pop inner clip.
        backend.pop_clip_rect().unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(0, 255, 0))
            .unwrap();
        // Pixel at (3,3) should now be green (inside outer clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);

        // Pop outer clip.
        backend.pop_clip_rect().unwrap();
    }

    #[test]
    fn translate_stack_offsets() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend.push_translate(5, 5).unwrap();
        // fill_rect at (0,0) should actually draw at (5,5).
        backend
            .fill_rect(0, 0, 2, 2, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (5,5) should be red.
        let offset = (5 * 20 + 5) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        // Pixel at (0,0) should be black.
        assert_eq!(backend.buffer()[0], 0);

        backend.push_translate(3, 3).unwrap();
        assert_eq!(backend.current_translate(), (8, 8));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (5, 5));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (0, 0));
    }

    #[test]
    fn blit_sub_draws_subregion() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 4x4 texture: top-left 2x2 is red, rest is blue.
        let mut tex_data = vec![0u8; 4 * 4 * 4];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let off = ((y * 4 + x) * 4) as usize;
                if x < 2 && y < 2 {
                    tex_data[off] = 255; // R
                    tex_data[off + 3] = 255; // A
                } else {
                    tex_data[off + 2] = 255; // B
                    tex_data[off + 3] = 255; // A
                }
            }
        }
        let tex_id = backend.load_texture(4, 4, &tex_data).unwrap();
        // Blit only the top-left 2x2 subregion.
        backend.blit_sub(tex_id, 0, 0, 2, 2, 0, 0, 2, 2).unwrap();
        // Pixel (0,0) should be red.
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[2], 0);
    }

    #[test]
    fn blit_tinted_applies_color() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 1x1 white texture.
        let tex_data = vec![255u8; 4];
        let tex_id = backend.load_texture(1, 1, &tex_data).unwrap();
        // Tint with red.
        backend
            .blit_tinted(tex_id, 0, 0, 1, 1, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255); // R
        assert_eq!(backend.buffer()[1], 0); // G
        assert_eq!(backend.buffer()[2], 0); // B
    }

    #[test]
    fn blit_flipped_horizontal() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 2x1 texture: left=red, right=blue.
        let tex_data = vec![255, 0, 0, 255, 0, 0, 255, 255];
        let tex_id = backend.load_texture(2, 1, &tex_data).unwrap();
        backend
            .blit_flipped(tex_id, 0, 0, 2, 1, true, false)
            .unwrap();
        // With horizontal flip: left should be blue, right should be red.
        assert_eq!(backend.buffer()[0], 0); // B became left
        assert_eq!(backend.buffer()[2], 255);
        assert_eq!(backend.buffer()[4], 255); // R became right
        assert_eq!(backend.buffer()[6], 0);
    }

    #[test]
    fn stroke_circle_draws_ring() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .stroke_circle(15, 15, 10, 2, Color::rgb(0, 255, 0))
            .unwrap();
        // Center should be black (hollow).
        let center = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[center], 0);
        // Edge pixel should be green.
        let edge = (15 * 30 + 25) * 4;
        assert_eq!(backend.buffer()[edge + 1], 255);
    }

    #[test]
    fn dim_screen_covers_viewport() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::WHITE).unwrap();
        backend.dim_screen(128).unwrap();
        // All pixels should be dimmed (not fully white anymore).
        assert!(backend.buffer()[0] < 255);
        assert!(backend.buffer()[0] > 0);
    }

    #[test]
    fn text_measurement() {
        let backend = Ue5Backend::new(10, 10);
        assert_eq!(backend.measure_text_height(8), 8);
        assert_eq!(backend.measure_text_height(16), 16);
        assert_eq!(backend.font_ascent(8), 8);
        let (w, h) = backend.measure_text_extents("AB", 8);
        assert_eq!(w, 14); // proportional: A(7)+B(7) = 14
        assert_eq!(h, 8);
    }

    // ---------------------------------------------------------------
    // RGBA pixel format / set_pixel color tests
    // ---------------------------------------------------------------

    #[test]
    fn rgba_buffer_layout_red() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255); // R
        assert_eq!(buf[1], 0); // G
        assert_eq!(buf[2], 0); // B
        assert_eq!(buf[3], 255); // A
    }

    #[test]
    fn rgba_buffer_layout_green() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 255, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_blue() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 0, 255)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_white() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::WHITE).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_black() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::BLACK).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_transparent_clear() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgba(0, 0, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn set_pixel_alpha_blend() {
        let mut backend = Ue5Backend::new(1, 1);
        // Start with white background.
        backend.clear(Color::WHITE).unwrap();
        // Draw 50% transparent red over it.
        backend.set_pixel(0, 0, Color::rgba(255, 0, 0, 128));
        let buf = backend.buffer();
        // Red channel: (255*128 + 255*127 + 127) / 255 ~= 255
        // Green channel: (0*128 + 255*127 + 127) / 255 ~= 127
        assert!(buf[0] > 200); // R stays high
        assert!(buf[1] > 100 && buf[1] < 140); // G blended ~127
        assert!(buf[2] > 100 && buf[2] < 140); // B blended ~127
        assert_eq!(buf[3], 255); // A always 255 after blend
    }

    #[test]
    fn set_pixel_fully_transparent_no_change() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(42, 84, 126)).unwrap();
        backend.set_pixel(0, 0, Color::rgba(255, 255, 255, 0));
        let buf = backend.buffer();
        assert_eq!(buf[0], 42);
        assert_eq!(buf[1], 84);
        assert_eq!(buf[2], 126);
    }

    #[test]
    fn set_pixel_out_of_bounds_no_crash() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::BLACK).unwrap();
        // Save buffer state after clear.
        let before: Vec<u8> = backend.buffer().to_vec();
        // Negative coordinates.
        backend.set_pixel(-1, 0, Color::WHITE);
        backend.set_pixel(0, -1, Color::WHITE);
        // Beyond bounds.
        backend.set_pixel(4, 0, Color::WHITE);
        backend.set_pixel(0, 4, Color::WHITE);
        // Buffer should be unchanged from the clear state.
        assert_eq!(backend.buffer(), before.as_slice());
    }

    #[test]
    fn rgba_round_trip_encode_decode() {
        // Encode Color into buffer, read it back.
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(123, 45, 67, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        let decoded = Color::rgba(buf[0], buf[1], buf[2], buf[3]);
        assert_eq!(decoded, c);
    }

    #[test]
    fn rgba_round_trip_all_channels_max() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(255, 255, 255, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }

    #[test]
    fn rgba_round_trip_all_channels_min() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(0, 0, 0, 0);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }
}
