#![allow(clippy::too_many_arguments)]
//! Shared software rasterization primitives for OASIS_OS backends.
//!
//! This crate provides a [`SoftwareBuffer`] that implements pixel-level RGBA
//! rendering operations (alpha blending, spans, gradients, Bresenham lines,
//! circles, rounded rects, triangles, text). Both the UE5 and WASM backends
//! can use these primitives as a software fallback or primary renderer.
//!
//! The crate also provides [`GlyphCacheKey`] for packing glyph parameters into
//! a compact hash key suitable for any glyph cache implementation.
//!
//! [`TextureDedup`] provides content-addressed texture deduplication with LRU
//! eviction and reference counting, shared by SDL and WASM backends.

mod texture_dedup;
pub use texture_dedup::TextureDedup;

use oasis_types::backend::{Color, GradientStyle};
use oasis_types::color::lerp_color_ratio;
use oasis_types::geometry::ClipRect;
use oasis_types::rasterize::{self, PixelSink};

// ---------------------------------------------------------------------------
// GlyphCacheKey
// ---------------------------------------------------------------------------

/// Packs `(char, font_size, rgba, bold, italic)` into a `u64` for hashing.
///
/// This is useful for any backend that wants to cache pre-rendered glyph
/// bitmaps or canvas elements keyed by character + style parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey(pub u64);

impl GlyphCacheKey {
    /// Create a new glyph cache key from character and style parameters.
    ///
    /// Layout (LSB to MSB):
    /// - bits  0..20: char code point (21 bits, covers all Unicode)
    /// - bits 21..36: font_size (16 bits)
    /// - bits 37..41: red (5 bits, quantized)
    /// - bits 42..46: green (5 bits, quantized)
    /// - bits 47..51: blue (5 bits, quantized)
    /// - bits 52..59: alpha (8 bits)
    /// - bit  60:     bold flag
    /// - bit  61:     italic flag
    pub const fn new(ch: char, font_size: u16, color: Color, bold: bool, italic: bool) -> Self {
        let c = ch as u64 & 0x1F_FFFF; // 21 bits
        let fs = (font_size as u64) & 0xFFFF; // 16 bits
        let r5 = (color.r as u64 >> 3) & 0x1F; // 5 bits
        let g5 = (color.g as u64 >> 3) & 0x1F; // 5 bits
        let b5 = (color.b as u64 >> 3) & 0x1F; // 5 bits
        let a = color.a as u64; // 8 bits
        let flags = (bold as u64) | ((italic as u64) << 1); // 2 bits
        Self(c | (fs << 21) | (r5 << 37) | (g5 << 42) | (b5 << 47) | (a << 52) | (flags << 60))
    }

    /// Return the inner packed `u64` value.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// SoftwareBuffer
// ---------------------------------------------------------------------------

/// An RGBA pixel buffer with software rasterization operations.
///
/// All rendering methods perform clipping, bounds checking, and source-over
/// alpha blending. The buffer uses RGBA byte order (R at offset 0, A at
/// offset 3) with 4 bytes per pixel.
pub struct SoftwareBuffer {
    width: u32,
    height: u32,
    buffer: Vec<u8>,
    clip: Option<ClipRect>,
}

impl SoftwareBuffer {
    /// Create a new buffer with the given resolution, filled with transparent
    /// black.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 4) as usize;
        Self {
            width,
            height,
            buffer: vec![0; size],
            clip: None,
        }
    }

    /// Reinitialize the buffer with a new resolution.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.buffer = vec![0; (width * height * 4) as usize];
        self.clip = None;
    }

    /// Buffer width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Buffer height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get a read-only reference to the raw RGBA pixel data.
    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    /// Get a mutable reference to the raw RGBA pixel data.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    /// Set the active clip rectangle. Pass `None` to disable clipping.
    pub fn set_clip(&mut self, clip: Option<ClipRect>) {
        self.clip = clip;
    }

    /// Get the current clip rectangle.
    pub fn clip(&self) -> Option<ClipRect> {
        self.clip
    }

    // -----------------------------------------------------------------------
    // Pixel operations
    // -----------------------------------------------------------------------

    /// Set a single pixel with source-over alpha blending.
    ///
    /// Performs bounds and clip checking. Out-of-bounds writes are silently
    /// ignored.
    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
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
        blend_pixel(&mut self.buffer, offset, color);
    }

    /// Fill a horizontal span of pixels with source-over alpha blending.
    ///
    /// `x_start` is inclusive, `x_end` is exclusive. Clips to bounds and
    /// active clip rect.
    pub fn fill_span(&mut self, y: i32, x_start: i32, x_end: i32, color: Color) {
        if y < 0 || y >= self.height as i32 {
            return;
        }
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

    /// Draw a horizontal line (inclusive endpoints). Wrapper around
    /// [`fill_span`](Self::fill_span).
    pub fn hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        let start = x1.min(x2);
        let end = x1.max(x2) + 1;
        self.fill_span(y, start, end, color);
    }

    /// Clear the entire buffer to the given color (no alpha blending).
    pub fn clear(&mut self, color: Color) {
        for pixel in self.buffer.chunks_exact_mut(4) {
            pixel[0] = color.r;
            pixel[1] = color.g;
            pixel[2] = color.b;
            pixel[3] = color.a;
        }
    }

    /// Fill a rectangle.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        if w == 0 || h == 0 || color.a == 0 {
            return;
        }
        for dy in 0..h as i32 {
            self.fill_span(y + dy, x, x + w as i32, color);
        }
    }

    /// Composite `src_pixels` (a `src_w * src_h * 4` RGBA8 buffer) over
    /// the destination rect at `(dst_x, dst_y, dst_w, dst_h)`,
    /// stretching 1:1 (no scaling) and applying per-pixel src-over
    /// alpha blending multiplied by `opacity`.
    ///
    /// This is the fallback compositor path for backends without
    /// hardware blend (UE5, PSP, and SDL non-native blend modes).
    /// `opacity` is clamped to `[0.0, 1.0]`.
    pub fn composite_rgba(
        &mut self,
        dst_x: i32,
        dst_y: i32,
        src_w: u32,
        src_h: u32,
        src_pixels: &[u8],
        opacity: f32,
    ) {
        if src_w == 0 || src_h == 0 || src_pixels.len() < (src_w * src_h * 4) as usize {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let op_u16 = (opacity * 256.0).round() as u16;
        if op_u16 == 0 {
            return;
        }
        let dst_stride = (self.width * 4) as usize;
        let src_stride = (src_w * 4) as usize;
        for row in 0..src_h as i32 {
            let dy = dst_y + row;
            if dy < 0 || dy as u32 >= self.height {
                continue;
            }
            for col in 0..src_w as i32 {
                let dx = dst_x + col;
                if dx < 0 || dx as u32 >= self.width {
                    continue;
                }
                let src_off = (row as usize) * src_stride + (col as usize) * 4;
                let dst_off = (dy as usize) * dst_stride + (dx as usize) * 4;
                let sr = src_pixels[src_off];
                let sg = src_pixels[src_off + 1];
                let sb = src_pixels[src_off + 2];
                let sa = src_pixels[src_off + 3];
                // Apply layer opacity to source alpha (256-scale).
                let a = ((sa as u16 * op_u16) >> 8) as u8;
                if a == 0 {
                    continue;
                }
                let inv = 255 - a as u16;
                let dr = self.buffer[dst_off];
                let dg = self.buffer[dst_off + 1];
                let db = self.buffer[dst_off + 2];
                let da = self.buffer[dst_off + 3];
                // Standard src-over.
                self.buffer[dst_off] = ((sr as u16 * a as u16 + dr as u16 * inv) / 255) as u8;
                self.buffer[dst_off + 1] = ((sg as u16 * a as u16 + dg as u16 * inv) / 255) as u8;
                self.buffer[dst_off + 2] = ((sb as u16 * a as u16 + db as u16 * inv) / 255) as u8;
                self.buffer[dst_off + 3] = (a as u16 + ((da as u16 * inv) / 255)) as u8;
            }
        }
    }

    /// Blit raw RGBA pixels into the buffer at the given position (no alpha
    /// blending, direct copy).
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
    }

    /// Read pixels from the buffer into a new RGBA vec.
    pub fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        for row in 0..h {
            let sy = (y as u32).wrapping_add(row) as usize;
            if sy >= self.height as usize {
                continue;
            }
            for col in 0..w {
                let sx = (x as u32).wrapping_add(col) as usize;
                if sx >= self.width as usize {
                    continue;
                }
                let src_idx = (sy * self.width as usize + sx) * 4;
                let dst_idx = (row as usize * w as usize + col as usize) * 4;
                out[dst_idx..dst_idx + 4].copy_from_slice(&self.buffer[src_idx..src_idx + 4]);
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Shape primitives
    // -----------------------------------------------------------------------

    /// Draw a filled rounded rectangle using the midpoint circle algorithm for
    /// corners.
    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, radius: u16, color: Color) {
        if w == 0 || h == 0 || color.a == 0 {
            return;
        }
        if radius == 0 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;

        // Center rect.
        for dy in r..(h as i32 - r) {
            self.hline(x, x + w as i32 - 1, y + dy, color);
        }

        // Corner arcs via midpoint circle.
        let mut cx = 0i32;
        let mut cy = r;
        let mut d = 1 - r;
        while cx <= cy {
            self.hline(x + r - cy, x + w as i32 - 1 - r + cy, y + r - cx, color);
            if cx != 0 {
                self.hline(
                    x + r - cy,
                    x + w as i32 - 1 - r + cy,
                    y + h as i32 - 1 - r + cx,
                    color,
                );
            }
            if cx != cy {
                self.hline(x + r - cx, x + w as i32 - 1 - r + cx, y + r - cy, color);
                self.hline(
                    x + r - cx,
                    x + w as i32 - 1 - r + cx,
                    y + h as i32 - 1 - r + cy,
                    color,
                );
            } else {
                self.hline(
                    x + r - cx,
                    x + w as i32 - 1 - r + cx,
                    y + h as i32 - 1 - r + cy,
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
    }

    /// Stroke a rectangle outline.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, stroke_width: u16, color: Color) {
        let sw = stroke_width as u32;
        // Top.
        self.fill_rect(x, y, w, sw, color);
        // Bottom.
        self.fill_rect(x, y + h as i32 - sw as i32, w, sw, color);
        // Left.
        self.fill_rect(x, y + sw as i32, sw, h.saturating_sub(sw * 2), color);
        // Right.
        self.fill_rect(
            x + w as i32 - sw as i32,
            y + sw as i32,
            sw,
            h.saturating_sub(sw * 2),
            color,
        );
    }

    /// Draw a line using Bresenham's algorithm.
    pub fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, width: u16, color: Color) {
        if color.a == 0 {
            return;
        }
        let w = width as i32;
        let dx = (x2 - x1).abs();
        let dy = -(y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x1;
        let mut cy = y1;

        loop {
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

    /// Fill a circle using the midpoint circle algorithm.
    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) {
        if color.a == 0 {
            return;
        }
        let r = radius as i32;
        let mut x = 0i32;
        let mut y = r;
        let mut d = 1 - r;

        while x <= y {
            self.hline(cx - y, cx + y, cy + x, color);
            if x != 0 {
                self.hline(cx - y, cx + y, cy - x, color);
            }
            if x != y {
                self.hline(cx - x, cx + x, cy + y, color);
                self.hline(cx - x, cx + x, cy - y, color);
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
    }

    /// Stroke a circle outline.
    pub fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) {
        if color.a == 0 || radius == 0 {
            return;
        }
        let r_outer = radius as i32;
        let r_inner = (radius as i32 - stroke_width as i32).max(0);

        for dy in -r_outer..=r_outer {
            let y = cy + dy;
            let outer_sq = r_outer * r_outer - dy * dy;
            if outer_sq < 0 {
                continue;
            }
            let outer_x = rasterize::isqrt(outer_sq as u32) as i32;

            if r_inner > 0 {
                let inner_sq = r_inner * r_inner - dy * dy;
                if inner_sq > 0 {
                    let inner_x = rasterize::isqrt(inner_sq as u32) as i32;
                    self.hline(cx - outer_x, cx - inner_x, y, color);
                    self.hline(cx + inner_x, cx + outer_x, y, color);
                    continue;
                }
            }
            self.hline(cx - outer_x, cx + outer_x, y, color);
        }
    }

    /// Fill a triangle using the shared scanline rasterizer.
    pub fn fill_triangle(&mut self, v0: (i32, i32), v1: (i32, i32), v2: (i32, i32), color: Color) {
        if color.a == 0 {
            return;
        }
        rasterize::rasterize_triangle(self, v0, v1, v2, color);
    }

    // -----------------------------------------------------------------------
    // Gradient fills
    // -----------------------------------------------------------------------

    /// Fill a rectangle with a vertical gradient.
    pub fn fill_rect_vertical_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top: Color,
        bottom: Color,
    ) {
        let h_max = h.saturating_sub(1).max(1);
        for dy in 0..h as i32 {
            let color = lerp_color_ratio(top, bottom, dy as u32, h_max);
            self.fill_span(y + dy, x, x + w as i32, color);
        }
    }

    /// Fill a rectangle with a horizontal gradient.
    pub fn fill_rect_horizontal_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        left: Color,
        right: Color,
    ) {
        let w_max = w.saturating_sub(1).max(1);
        for dx in 0..w as i32 {
            let color = lerp_color_ratio(left, right, dx as u32, w_max);
            for dy in 0..h as i32 {
                self.set_pixel(x + dx, y + dy, color);
            }
        }
    }

    /// Fill a rectangle with a four-corner bilinear gradient.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect_four_corner_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top_left: Color,
        top_right: Color,
        bottom_left: Color,
        bottom_right: Color,
    ) {
        let h_max = h.saturating_sub(1).max(1);
        let w_max = w.saturating_sub(1).max(1);
        for dy in 0..h as i32 {
            let left = lerp_color_ratio(top_left, bottom_left, dy as u32, h_max);
            let right = lerp_color_ratio(top_right, bottom_right, dy as u32, h_max);
            for dx in 0..w as i32 {
                let color = lerp_color_ratio(left, right, dx as u32, w_max);
                self.set_pixel(x + dx, y + dy, color);
            }
        }
    }

    /// Fill a rectangle with a gradient, dispatching on [`GradientStyle`].
    ///
    /// This is a convenience method that delegates to the appropriate
    /// variant-specific gradient fill.
    pub fn fill_rect_gradient(&mut self, x: i32, y: i32, w: u32, h: u32, gradient: &GradientStyle) {
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                self.fill_rect_vertical_gradient(x, y, w, h, top, bottom);
            },
            GradientStyle::Horizontal { left, right } => {
                self.fill_rect_horizontal_gradient(x, y, w, h, left, right);
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                self.fill_rect_four_corner_gradient(
                    x,
                    y,
                    w,
                    h,
                    top_left,
                    top_right,
                    bottom_left,
                    bottom_right,
                );
            },
        }
    }

    // -----------------------------------------------------------------------
    // Text rendering
    // -----------------------------------------------------------------------

    /// Render bitmap font text into the buffer.
    ///
    /// Uses the shared `oasis_types::bitmap_font` glyph data. The `glyph_fn`
    /// and `metrics_fn` parameters allow callers to provide their own glyph
    /// lookup (typically `font::glyph` and `font::glyph_metrics`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_bitmap_text<F, M>(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        glyph_fn: F,
        metrics_fn: M,
    ) where
        F: Fn(char) -> &'static [u8; 8],
        M: Fn(char) -> (u8, u8),
    {
        if text.is_empty() || color.a == 0 || font_size == 0 {
            return;
        }
        let scale = if font_size >= 8 {
            (font_size / 8) as i32
        } else {
            1
        };

        let mut cx = x;
        for ch in text.chars() {
            let glyph_data: &[u8; 8] = glyph_fn(ch);
            let (left_pad, advance) = metrics_fn(ch);
            let left_pad = left_pad as i32;
            for row in 0..8i32 {
                let bits = glyph_data[row as usize];
                for col in 0..8i32 {
                    if bits & (0x80 >> col) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.set_pixel(
                                    cx + (col - left_pad) * scale + sx,
                                    y + row * scale + sy,
                                    color,
                                );
                            }
                        }
                    }
                }
            }
            cx += advance as i32 * scale;
        }
    }

    // -----------------------------------------------------------------------
    // Texture blit helpers
    // -----------------------------------------------------------------------

    /// Blit RGBA texture data with scaling and alpha blending.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_texture(
        &mut self,
        tex_data: &[u8],
        tex_w: u32,
        tex_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) {
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let src_x = (dx * tex_w / dst_w) as usize;
                let src_y = (dy * tex_h / dst_h) as usize;
                let src_offset = (src_y * tex_w as usize + src_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        tex_data[src_offset],
                        tex_data[src_offset + 1],
                        tex_data[src_offset + 2],
                        tex_data[src_offset + 3],
                    );
                    self.set_pixel(dst_x + dx as i32, dst_y + dy as i32, color);
                }
            }
        }
    }

    /// Blit a sub-region of RGBA texture data with scaling and alpha blending.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_texture_sub(
        &mut self,
        tex_data: &[u8],
        tex_w: u32,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) {
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
                    self.set_pixel(dst_x + dx as i32, dst_y + dy as i32, color);
                }
            }
        }
    }

    /// Blit RGBA texture data with a tint color applied (multiply blend).
    #[allow(clippy::too_many_arguments)]
    pub fn blit_texture_tinted(
        &mut self,
        tex_data: &[u8],
        tex_w: u32,
        tex_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) {
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let src_x = (dx * tex_w / dst_w) as usize;
                let src_y = (dy * tex_h / dst_h) as usize;
                let src_offset = (src_y * tex_w as usize + src_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        ((tex_data[src_offset] as u16 * tint.r as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 1] as u16 * tint.g as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 2] as u16 * tint.b as u16 + 127) / 255) as u8,
                        ((tex_data[src_offset + 3] as u16 * tint.a as u16 + 127) / 255) as u8,
                    );
                    self.set_pixel(dst_x + dx as i32, dst_y + dy as i32, color);
                }
            }
        }
    }

    /// Blit a sub-region of RGBA texture data with tint (multiply blend).
    #[allow(clippy::too_many_arguments)]
    pub fn blit_texture_sub_tinted(
        &mut self,
        tex_data: &[u8],
        tex_w: u32,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) {
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
                    self.set_pixel(dst_x + dx as i32, dst_y + dy as i32, color);
                }
            }
        }
    }

    /// Blit RGBA texture data with horizontal and/or vertical flip.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_texture_flipped(
        &mut self,
        tex_data: &[u8],
        tex_w: u32,
        tex_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        flip_h: bool,
        flip_v: bool,
    ) {
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let sample_x = if flip_h {
                    ((dst_w - 1 - dx) * tex_w / dst_w) as usize
                } else {
                    (dx * tex_w / dst_w) as usize
                };
                let sample_y = if flip_v {
                    ((dst_h - 1 - dy) * tex_h / dst_h) as usize
                } else {
                    (dy * tex_h / dst_h) as usize
                };
                let src_offset = (sample_y * tex_w as usize + sample_x) * 4;
                if src_offset + 3 < tex_data.len() {
                    let color = Color::rgba(
                        tex_data[src_offset],
                        tex_data[src_offset + 1],
                        tex_data[src_offset + 2],
                        tex_data[src_offset + 3],
                    );
                    self.set_pixel(dst_x + dx as i32, dst_y + dy as i32, color);
                }
            }
        }
    }
}

impl PixelSink for SoftwareBuffer {
    fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        self.hline(x1, x2, y, color);
    }
}

// ---------------------------------------------------------------------------
// Alpha blending helper
// ---------------------------------------------------------------------------

/// Blend a source color into a buffer at the given byte offset using
/// source-over compositing.
#[inline]
fn blend_pixel(buffer: &mut [u8], offset: usize, color: Color) {
    if color.a == 255 {
        buffer[offset] = color.r;
        buffer[offset + 1] = color.g;
        buffer[offset + 2] = color.b;
        buffer[offset + 3] = 255;
    } else if color.a > 0 {
        let sa = color.a as u16;
        let da = 255 - sa;
        buffer[offset] = ((color.r as u16 * sa + buffer[offset] as u16 * da + 127) / 255) as u8;
        buffer[offset + 1] =
            ((color.g as u16 * sa + buffer[offset + 1] as u16 * da + 127) / 255) as u8;
        buffer[offset + 2] =
            ((color.b as u16 * sa + buffer[offset + 2] as u16 * da + 127) / 255) as u8;
        buffer[offset + 3] = 255;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_buffer() {
        let buf = SoftwareBuffer::new(480, 272);
        assert_eq!(buf.data().len(), 480 * 272 * 4);
        assert_eq!(buf.width(), 480);
        assert_eq!(buf.height(), 272);
    }

    #[test]
    fn clear_fills_buffer() {
        let mut buf = SoftwareBuffer::new(4, 4);
        buf.clear(Color::rgb(255, 0, 0));
        assert_eq!(buf.data()[0], 255);
        assert_eq!(buf.data()[1], 0);
        assert_eq!(buf.data()[2], 0);
        assert_eq!(buf.data()[3], 255);
        let last = buf.data().len() - 4;
        assert_eq!(buf.data()[last], 255);
    }

    #[test]
    fn fill_rect_draws_pixels() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        buf.fill_rect(2, 2, 3, 3, Color::rgb(0, 255, 0));
        let offset = (2 * 10 + 2) * 4;
        assert_eq!(buf.data()[offset], 0);
        assert_eq!(buf.data()[offset + 1], 255);
        assert_eq!(buf.data()[0], 0);
        assert_eq!(buf.data()[1], 0);
    }

    #[test]
    fn set_pixel_alpha_blend() {
        let mut buf = SoftwareBuffer::new(1, 1);
        buf.clear(Color::WHITE);
        buf.set_pixel(0, 0, Color::rgba(255, 0, 0, 128));
        assert!(buf.data()[0] > 200); // R stays high
        assert!(buf.data()[1] > 100 && buf.data()[1] < 140); // G blended
        assert_eq!(buf.data()[3], 255); // A always 255
    }

    #[test]
    fn set_pixel_out_of_bounds_no_crash() {
        let mut buf = SoftwareBuffer::new(4, 4);
        buf.clear(Color::BLACK);
        let before: Vec<u8> = buf.data().to_vec();
        buf.set_pixel(-1, 0, Color::WHITE);
        buf.set_pixel(0, -1, Color::WHITE);
        buf.set_pixel(4, 0, Color::WHITE);
        buf.set_pixel(0, 4, Color::WHITE);
        assert_eq!(buf.data(), before.as_slice());
    }

    #[test]
    fn clip_restricts_drawing() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        buf.set_clip(Some(ClipRect {
            x: 2,
            y: 2,
            w: 3,
            h: 3,
        }));
        buf.fill_rect(0, 0, 10, 10, Color::rgb(255, 0, 0));
        // (0,0) should be black.
        assert_eq!(buf.data()[0], 0);
        // (3,3) should be red.
        let offset = (3 * 10 + 3) * 4;
        assert_eq!(buf.data()[offset], 255);
    }

    #[test]
    fn fill_rounded_rect_draws_center() {
        let mut buf = SoftwareBuffer::new(20, 20);
        buf.clear(Color::BLACK);
        buf.fill_rounded_rect(2, 2, 16, 16, 4, Color::rgb(0, 255, 0));
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(buf.data()[offset + 1], 255);
    }

    #[test]
    fn draw_line_horizontal() {
        let mut buf = SoftwareBuffer::new(20, 10);
        buf.clear(Color::BLACK);
        buf.draw_line(2, 5, 18, 5, 1, Color::rgb(255, 0, 0));
        let offset = (5 * 20 + 10) * 4;
        assert_eq!(buf.data()[offset], 255);
    }

    #[test]
    fn fill_circle_draws_center() {
        let mut buf = SoftwareBuffer::new(30, 30);
        buf.clear(Color::BLACK);
        buf.fill_circle(15, 15, 10, Color::rgb(255, 0, 0));
        let offset = (15 * 30 + 15) * 4;
        assert_eq!(buf.data()[offset], 255);
    }

    #[test]
    fn stroke_circle_hollow_center() {
        let mut buf = SoftwareBuffer::new(30, 30);
        buf.clear(Color::BLACK);
        buf.stroke_circle(15, 15, 10, 2, Color::rgb(0, 255, 0));
        let center = (15 * 30 + 15) * 4;
        assert_eq!(buf.data()[center], 0);
        let edge = (15 * 30 + 25) * 4;
        assert_eq!(buf.data()[edge + 1], 255);
    }

    #[test]
    fn fill_triangle_draws() {
        let mut buf = SoftwareBuffer::new(20, 20);
        buf.clear(Color::BLACK);
        buf.fill_triangle((10, 2), (2, 18), (18, 18), Color::rgb(0, 255, 0));
        let offset = (14 * 20 + 10) * 4;
        assert_eq!(buf.data()[offset + 1], 255);
    }

    #[test]
    fn vertical_gradient_fills() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        buf.fill_rect_vertical_gradient(0, 0, 10, 10, Color::WHITE, Color::BLACK);
        assert_eq!(buf.data()[0], 255);
        let last_row = (9 * 10) * 4;
        assert_eq!(buf.data()[last_row], 0);
    }

    #[test]
    fn horizontal_gradient_fills() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        buf.fill_rect_horizontal_gradient(0, 0, 10, 10, Color::WHITE, Color::BLACK);
        assert_eq!(buf.data()[0], 255);
        let right = 9 * 4;
        assert_eq!(buf.data()[right], 0);
    }

    #[test]
    fn read_pixels_roundtrip() {
        let mut buf = SoftwareBuffer::new(4, 4);
        buf.clear(Color::rgb(42, 84, 126));
        let pixels = buf.read_pixels(0, 0, 4, 4);
        assert_eq!(pixels.len(), 64);
        assert_eq!(pixels[0], 42);
        assert_eq!(pixels[1], 84);
        assert_eq!(pixels[2], 126);
    }

    #[test]
    fn resize_clears_buffer() {
        let mut buf = SoftwareBuffer::new(4, 4);
        buf.clear(Color::WHITE);
        buf.resize(8, 8);
        assert_eq!(buf.data().len(), 8 * 8 * 4);
        assert_eq!(buf.data()[0], 0);
    }

    // -----------------------------------------------------------------------
    // GlyphCacheKey tests
    // -----------------------------------------------------------------------

    #[test]
    fn glyph_key_unique_for_different_chars() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('B', 12, c, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_different_sizes() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('A', 16, c, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_different_colors() {
        let c1 = Color::rgba(255, 0, 0, 255);
        let c2 = Color::rgba(0, 255, 0, 255);
        let k1 = GlyphCacheKey::new('A', 12, c1, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c2, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_bold_italic() {
        let c = Color::rgba(255, 255, 255, 255);
        let keys: Vec<GlyphCacheKey> = vec![
            GlyphCacheKey::new('X', 10, c, false, false),
            GlyphCacheKey::new('X', 10, c, true, false),
            GlyphCacheKey::new('X', 10, c, false, true),
            GlyphCacheKey::new('X', 10, c, true, true),
        ];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "keys[{i}] == keys[{j}]");
            }
        }
    }

    #[test]
    fn glyph_key_equal_for_same_params() {
        let c = Color::rgba(128, 64, 32, 200);
        let k1 = GlyphCacheKey::new('Z', 24, c, true, true);
        let k2 = GlyphCacheKey::new('Z', 24, c, true, true);
        assert_eq!(k1, k2);
    }

    #[test]
    fn glyph_key_alpha_distinction() {
        let c1 = Color::rgba(128, 128, 128, 100);
        let c2 = Color::rgba(128, 128, 128, 200);
        let k1 = GlyphCacheKey::new('A', 12, c1, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c2, false, false);
        assert_ne!(k1, k2);
    }

    // -----------------------------------------------------------------------
    // Texture blit tests
    // -----------------------------------------------------------------------

    #[test]
    fn blit_texture_draws() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        let tex_data = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        buf.blit_texture(&tex_data, 2, 2, 1, 1, 2, 2);
        let offset = (1 * 10 + 1) * 4;
        assert_eq!(buf.data()[offset], 255);
        assert_eq!(buf.data()[offset + 1], 0);
    }

    #[test]
    fn blit_texture_tinted_applies_tint() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        let tex_data = vec![255u8; 4]; // white pixel
        buf.blit_texture_tinted(&tex_data, 1, 1, 0, 0, 1, 1, Color::rgb(255, 0, 0));
        assert_eq!(buf.data()[0], 255); // R
        assert_eq!(buf.data()[1], 0); // G
        assert_eq!(buf.data()[2], 0); // B
    }

    #[test]
    fn blit_texture_flipped_horizontal() {
        let mut buf = SoftwareBuffer::new(10, 10);
        buf.clear(Color::BLACK);
        let tex_data = vec![255, 0, 0, 255, 0, 0, 255, 255]; // red, blue
        buf.blit_texture_flipped(&tex_data, 2, 1, 0, 0, 2, 1, true, false);
        // With flip: left=blue, right=red.
        assert_eq!(buf.data()[0], 0); // B at left
        assert_eq!(buf.data()[2], 255);
        assert_eq!(buf.data()[4], 255); // R at right
        assert_eq!(buf.data()[6], 0);
    }
}
