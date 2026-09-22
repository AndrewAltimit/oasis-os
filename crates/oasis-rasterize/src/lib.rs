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

#[cfg(feature = "ttf")]
pub mod ttf;

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

    /// Create a *color-independent* key from character and style parameters.
    ///
    /// Backends that rasterize glyphs white and tint them at blit time (SDL's
    /// texture color/alpha modulation, for instance) must not key the cache on
    /// color: doing so re-rasterizes the same glyph for every theme accent,
    /// hover tint, or animated fade. The color bits are left zero, so these
    /// keys never collide with [`Self::new`] keys carrying a nonzero color.
    pub const fn colorless(ch: char, font_size: u16, bold: bool, italic: bool) -> Self {
        Self::new(ch, font_size, Color::rgba(0, 0, 0, 0), bold, italic)
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
    /// Reused per-blit source column map (avoids a per-call allocation).
    col_map: Vec<usize>,
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
            col_map: Vec::new(),
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
        let b = self.visible_bounds();
        if x < b.x0 || x >= b.x1 || y < b.y0 || y >= b.y1 {
            return;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        blend_pixel(&mut self.buffer, offset, color);
    }

    /// The drawable region: the buffer bounds intersected with the active
    /// clip rect, as half-open pixel ranges. Computed once per primitive so
    /// the inner loops can run on plain row slices with no per-pixel checks.
    #[inline]
    fn visible_bounds(&self) -> Bounds {
        let mut x0 = 0i64;
        let mut y0 = 0i64;
        let mut x1 = self.width as i64;
        let mut y1 = self.height as i64;
        if let Some(clip) = &self.clip {
            x0 = x0.max(clip.x as i64);
            y0 = y0.max(clip.y as i64);
            x1 = x1.min(clip.x as i64 + clip.w as i64);
            y1 = y1.min(clip.y as i64 + clip.h as i64);
        }
        // `x0`/`y0` are >= 0 and `x1`/`y1` <= width/height (clamped up to
        // the start), so an empty region has `x0 == x1` or `y0 == y1`.
        Bounds {
            x0: x0 as i32,
            y0: y0 as i32,
            x1: x1.max(x0) as i32,
            y1: y1.max(y0) as i32,
        }
    }

    /// Mutable RGBA bytes of row `y`, columns `xs..xe` (already clipped).
    #[inline]
    fn row_mut(&mut self, y: i32, xs: i32, xe: i32) -> &mut [u8] {
        let row = y as usize * self.width as usize * 4;
        &mut self.buffer[row + xs as usize * 4..row + xe as usize * 4]
    }

    /// Fill a horizontal span of pixels with source-over alpha blending.
    ///
    /// `x_start` is inclusive, `x_end` is exclusive. Clips to bounds and
    /// active clip rect.
    pub fn fill_span(&mut self, y: i32, x_start: i32, x_end: i32, color: Color) {
        if color.a == 0 {
            return;
        }
        let b = self.visible_bounds();
        if y < b.y0 || y >= b.y1 {
            return;
        }
        let xs = x_start.max(b.x0);
        let xe = x_end.min(b.x1);
        if xs >= xe {
            return;
        }
        let row = self.row_mut(y, xs, xe);
        if color.a == 255 {
            let px = [color.r, color.g, color.b, 255];
            for dst in row.as_chunks_mut::<4>().0 {
                *dst = px;
            }
        } else {
            let sa = color.a as u16;
            let da = 255 - sa;
            let (r, g, bl) = (
                color.r as u16 * sa,
                color.g as u16 * sa,
                color.b as u16 * sa,
            );
            for dst in row.as_chunks_mut::<4>().0 {
                dst[0] = ((r + dst[0] as u16 * da + 127) / 255) as u8;
                dst[1] = ((g + dst[1] as u16 * da + 127) / 255) as u8;
                dst[2] = ((bl + dst[2] as u16 * da + 127) / 255) as u8;
                dst[3] = 255;
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
        for pixel in self.buffer.as_chunks_mut::<4>().0.iter_mut() {
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
        // Every row is filled exactly once, so translucent colors blend
        // uniformly (no darker bands where corner scanlines used to repeat).
        rounded_rect_rows(w as i32, h as i32, r, |dy, x0, x1| {
            self.fill_span(y + dy, x + x0, x + x1, color);
        });
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
        // One span per row (the midpoint walk used to repeat rows near the
        // poles, double-blending translucent circles).
        midpoint_row_extents(radius as i32, |o, ext| {
            self.hline(cx - ext, cx + ext, cy + o, color);
            if o != 0 {
                self.hline(cx - ext, cx + ext, cy - o, color);
            }
        });
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
        let Some((xs, xe, ys, ye)) = self.clip_rect_to_visible(x, y, w, h) else {
            return;
        };
        // Colors depend only on the column: interpolate a chunk of columns
        // once, then blend that chunk into every visible row.
        const CHUNK: usize = 64;
        let mut colors = [Color::rgba(0, 0, 0, 0); CHUNK];
        let mut cs = xs;
        while cs < xe {
            let ce = (cs + CHUNK as i32).min(xe);
            let n = (ce - cs) as usize;
            for (i, c) in colors[..n].iter_mut().enumerate() {
                *c = lerp_color_ratio(left, right, (cs - x) as u32 + i as u32, w_max);
            }
            for py in ys..ye {
                let row = self.row_mut(py, cs, ce);
                for (px, &c) in row.as_chunks_mut::<4>().0.iter_mut().zip(&colors[..n]) {
                    blend_px(px, c);
                }
            }
            cs = ce;
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
        let Some((xs, xe, ys, ye)) = self.clip_rect_to_visible(x, y, w, h) else {
            return;
        };
        for py in ys..ye {
            let dy = (py - y) as u32;
            let left = lerp_color_ratio(top_left, bottom_left, dy, h_max);
            let right = lerp_color_ratio(top_right, bottom_right, dy, h_max);
            let row = self.row_mut(py, xs, xe);
            for (i, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let dx = (xs - x) as u32 + i as u32;
                blend_px(px, lerp_color_ratio(left, right, dx, w_max));
            }
        }
    }

    /// Intersect the rect `(x, y, w, h)` with the visible region, returning
    /// half-open `(xs, xe, ys, ye)` or `None` when nothing is visible.
    #[inline]
    fn clip_rect_to_visible(&self, x: i32, y: i32, w: u32, h: u32) -> Option<(i32, i32, i32, i32)> {
        let b = self.visible_bounds();
        let xs = (x as i64).max(b.x0 as i64);
        let ys = (y as i64).max(b.y0 as i64);
        let xe = (x as i64 + w as i64).min(b.x1 as i64);
        let ye = (y as i64 + h as i64).min(b.y1 as i64);
        (xs < xe && ys < ye).then_some((xs as i32, xe as i32, ys as i32, ye as i32))
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
                // Emit each run of set bits as one scaled span per sub-row.
                let mut col = 0i32;
                while col < 8 {
                    if bits & (0x80 >> col) == 0 {
                        col += 1;
                        continue;
                    }
                    let start = col;
                    while col < 8 && bits & (0x80 >> col) != 0 {
                        col += 1;
                    }
                    let xs = cx + (start - left_pad) * scale;
                    let xe = cx + (col - left_pad) * scale;
                    for sy in 0..scale {
                        self.fill_span(y + row * scale + sy, xs, xe, color);
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
        self.blit_core(&BlitParams {
            tex: tex_data,
            tex_w,
            src: (0, 0, tex_w, tex_h),
            dst: (dst_x, dst_y, dst_w, dst_h),
            flip_h: false,
            flip_v: false,
            tint: None,
        });
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
        self.blit_core(&BlitParams {
            tex: tex_data,
            tex_w,
            src: (src_x, src_y, src_w, src_h),
            dst: (dst_x, dst_y, dst_w, dst_h),
            flip_h: false,
            flip_v: false,
            tint: None,
        });
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
        self.blit_core(&BlitParams {
            tex: tex_data,
            tex_w,
            src: (0, 0, tex_w, tex_h),
            dst: (dst_x, dst_y, dst_w, dst_h),
            flip_h: false,
            flip_v: false,
            tint: Some(tint),
        });
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
        self.blit_core(&BlitParams {
            tex: tex_data,
            tex_w,
            src: (src_x, src_y, src_w, src_h),
            dst: (dst_x, dst_y, dst_w, dst_h),
            flip_h: false,
            flip_v: false,
            tint: Some(tint),
        });
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
        self.blit_core(&BlitParams {
            tex: tex_data,
            tex_w,
            src: (0, 0, tex_w, tex_h),
            dst: (dst_x, dst_y, dst_w, dst_h),
            flip_h,
            flip_v,
            tint: None,
        });
    }

    /// Shared nearest-neighbour blit.
    ///
    /// Destination column `k` (row `j`) samples source column
    /// `src.x + m * src.w / dst.w` where `m` is `k`, or `dst.w - 1 - k` when
    /// flipped -- exactly the per-pixel formula the blits have always used.
    /// The destination rect is clipped once up front; each row then runs on
    /// a plain slice. An unscaled, untinted row blends straight from the
    /// contiguous source run (opaque runs become a single `copy_from_slice`).
    /// Otherwise the source column of every destination column is computed
    /// once per blit with an exact integer DDA (quotient + remainder, so no
    /// per-pixel division), and a fully opaque row that repeats the previous
    /// source row is duplicated with `copy_within`. Source pixels whose
    /// offset falls outside `tex` are skipped, as before.
    fn blit_core(&mut self, p: &BlitParams<'_>) {
        let (sx0, _, sw, _) = p.src;
        let (dst_x, dst_y, dw, dh) = p.dst;
        if dw == 0 || dh == 0 {
            return;
        }
        let Some((xs, xe, ys, ye)) = self.clip_rect_to_visible(dst_x, dst_y, dw, dh) else {
            return;
        };
        // Destination-local visible columns `[k0, k1)`.
        let k0 = (xs as i64 - dst_x as i64) as u64;
        let k1 = (xe as i64 - dst_x as i64) as u64;
        let n = (k1 - k0) as usize;
        let (dw64, sw64) = (dw as u64, sw as u64);
        // First sample index `m` in ascending order. With a horizontal flip
        // the ascending `m` walk fills the row right-to-left.
        let m0 = if p.flip_h { dw64 - k1 } else { k0 };
        let q0 = (m0 * sw64 / dw64) as usize;
        let tex = p.tex;
        let tex_stride = p.tex_w as usize * 4;

        if !p.flip_h && p.tint.is_none() && sw == dw {
            // Unscaled horizontally: each row is one contiguous source run.
            for py in ys..ye {
                let sy = Self::sample_row(p, py);
                let off = sy * tex_stride + (sx0 as usize + q0) * 4;
                let cnt = n.min(tex.len().saturating_sub(off) / 4);
                if cnt > 0 {
                    let dst = self.row_mut(py, xs, xe);
                    blend_row(&mut dst[..cnt * 4], &tex[off..off + cnt * 4]);
                }
            }
            return;
        }

        // Source texel column for every visible destination column, in
        // destination order, via an exact integer DDA (no division per
        // pixel). The map is computed once and reused for every row.
        let mut cols = std::mem::take(&mut self.col_map);
        cols.clear();
        let (q_step, r_step) = ((sw64 / dw64) as usize, sw64 % dw64);
        let (mut q, mut r) = (q0, m0 * sw64 % dw64);
        for _ in 0..n {
            cols.push(sx0 as usize + q);
            q += q_step;
            r += r_step;
            if r >= dw64 {
                r -= dw64;
                q += 1;
            }
        }
        if p.flip_h {
            cols.reverse();
        }

        // `Some(sy)` of the previous row when every one of its samples was
        // opaque: a repeated source row (vertical upscale) then produces
        // identical pixels, so the destination row is simply duplicated.
        let mut prev_opaque_row: Option<usize> = None;
        for py in ys..ye {
            let sy = Self::sample_row(p, py);
            let row_off = xs as usize * 4;
            let stride = self.width as usize * 4;
            if prev_opaque_row == Some(sy) {
                let cur = py as usize * stride + row_off;
                let len = n * 4;
                self.buffer
                    .copy_within(cur - stride..cur - stride + len, cur);
                continue;
            }
            let src_px = tex
                .get(sy * tex_stride..)
                .map_or(&[][..], |s| s.as_chunks::<4>().0);
            let dst_px = self.row_mut(py, xs, xe).as_chunks_mut::<4>().0;
            let mut opaque = true;
            match p.tint {
                None => {
                    for (d, &c) in dst_px.iter_mut().zip(&cols) {
                        match src_px.get(c) {
                            Some(&s) if s[3] == 255 => *d = s,
                            Some(&s) => {
                                opaque = false;
                                blend_px(d, Color::rgba(s[0], s[1], s[2], s[3]));
                            },
                            None => opaque = false,
                        }
                    }
                },
                Some(t) => {
                    opaque = false;
                    for (d, &c) in dst_px.iter_mut().zip(&cols) {
                        if let Some(&s) = src_px.get(c) {
                            let color = Color::rgba(
                                ((s[0] as u16 * t.r as u16 + 127) / 255) as u8,
                                ((s[1] as u16 * t.g as u16 + 127) / 255) as u8,
                                ((s[2] as u16 * t.b as u16 + 127) / 255) as u8,
                                ((s[3] as u16 * t.a as u16 + 127) / 255) as u8,
                            );
                            blend_px(d, color);
                        }
                    }
                },
            }
            prev_opaque_row = opaque.then_some(sy);
        }
        self.col_map = cols;
    }

    /// Source texel row sampled by destination row `py` of a blit.
    #[inline]
    fn sample_row(p: &BlitParams<'_>, py: i32) -> usize {
        let (_, sy0, _, sh) = p.src;
        let (_, dst_y, _, dh) = p.dst;
        let j = (py as i64 - dst_y as i64) as u64;
        let my = if p.flip_v { dh as u64 - 1 - j } else { j };
        (sy0 as u64 + my * sh as u64 / dh as u64) as usize
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
    if let Some(px) = buffer[offset..offset + 4]
        .as_chunks_mut::<4>()
        .0
        .first_mut()
    {
        blend_px(px, color);
    }
}

/// Source-over blend `color` into one RGBA pixel. The destination alpha is
/// always forced to 255 (the buffers are treated as opaque surfaces).
#[inline]
fn blend_px(px: &mut [u8; 4], color: Color) {
    if color.a == 255 {
        *px = [color.r, color.g, color.b, 255];
    } else if color.a > 0 {
        let sa = color.a as u16;
        let da = 255 - sa;
        px[0] = ((color.r as u16 * sa + px[0] as u16 * da + 127) / 255) as u8;
        px[1] = ((color.g as u16 * sa + px[1] as u16 * da + 127) / 255) as u8;
        px[2] = ((color.b as u16 * sa + px[2] as u16 * da + 127) / 255) as u8;
        px[3] = 255;
    }
}

/// Blend an RGBA source row over an equally long destination row.
///
/// Runs of fully opaque source pixels are copied with one
/// `copy_from_slice`; everything else goes through [`blend_px`], so the
/// result is identical to blending pixel by pixel.
#[inline]
fn blend_row(dst: &mut [u8], src: &[u8]) {
    let d = dst.as_chunks_mut::<4>().0;
    let s = src.as_chunks::<4>().0;
    let n = d.len().min(s.len());
    let (d, s) = (&mut d[..n], &s[..n]);
    // Fully opaque rows (the common case for images and icons) are one
    // memcpy; the branch-free AND-reduction vectorizes well.
    if s.iter()
        .fold(u32::MAX, |acc, p| acc & u32::from_le_bytes(*p))
        >> 24
        == 255
    {
        d.copy_from_slice(s);
        return;
    }
    let mut i = 0;
    while i < n {
        if s[i][3] == 255 {
            let start = i;
            i += 1;
            while i < n && s[i][3] == 255 {
                i += 1;
            }
            d[start..i].copy_from_slice(&s[start..i]);
        } else {
            let p = s[i];
            blend_px(&mut d[i], Color::rgba(p[0], p[1], p[2], p[3]));
            i += 1;
        }
    }
}

/// Half-open visible pixel region (`x0..x1`, `y0..y1`).
#[derive(Clone, Copy)]
struct Bounds {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// Arguments of the shared blit routine.
struct BlitParams<'a> {
    tex: &'a [u8],
    tex_w: u32,
    /// Source rect `(x, y, w, h)` in texels.
    src: (u32, u32, u32, u32),
    /// Destination rect `(x, y, w, h)` in pixels.
    dst: (i32, i32, u32, u32),
    flip_h: bool,
    flip_v: bool,
    tint: Option<Color>,
}

// ---------------------------------------------------------------------------
// Scanline helpers
// ---------------------------------------------------------------------------

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

    #[test]
    fn colorless_key_ignores_color() {
        let k1 = GlyphCacheKey::colorless('A', 12, false, false);
        let k2 = GlyphCacheKey::colorless('A', 12, false, false);
        assert_eq!(k1, k2);
        // Style still matters.
        assert_ne!(k1, GlyphCacheKey::colorless('A', 12, true, false));
        assert_ne!(k1, GlyphCacheKey::colorless('A', 12, false, true));
        assert_ne!(k1, GlyphCacheKey::colorless('A', 16, false, false));
        assert_ne!(k1, GlyphCacheKey::colorless('B', 12, false, false));
    }

    #[test]
    fn colorless_key_never_collides_with_colored_key() {
        // A colored key with a visible color always has nonzero color bits,
        // so the two key spaces can safely share one cache map.
        let visible = Color::rgba(255, 255, 255, 255);
        assert_ne!(
            GlyphCacheKey::colorless('A', 12, false, false),
            GlyphCacheKey::new('A', 12, visible, false, false)
        );
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

    // -----------------------------------------------------------------------
    // Golden tests: fast paths vs. the original per-pixel algorithms
    // -----------------------------------------------------------------------

    /// Verbatim copies of the pre-optimization per-pixel implementations,
    /// kept as the reference the span/row-based fast paths must match.
    mod reference {
        use super::*;

        pub fn set_pixel(buf: &mut SoftwareBuffer, x: i32, y: i32, color: Color) {
            if x < 0 || y < 0 {
                return;
            }
            let (ux, uy) = (x as u32, y as u32);
            if ux >= buf.width() || uy >= buf.height() {
                return;
            }
            if let Some(clip) = buf.clip()
                && (x < clip.x
                    || y < clip.y
                    || ux >= (clip.x as u32).saturating_add(clip.w)
                    || uy >= (clip.y as u32).saturating_add(clip.h))
            {
                return;
            }
            let offset = ((uy * buf.width() + ux) * 4) as usize;
            blend_pixel(buf.data_mut(), offset, color);
        }

        fn hline(buf: &mut SoftwareBuffer, x1: i32, x2: i32, y: i32, color: Color) {
            for x in x1.min(x2)..=x1.max(x2) {
                set_pixel(buf, x, y, color);
            }
        }

        fn sample(tex: &[u8], off: usize) -> Option<Color> {
            (off + 3 < tex.len())
                .then(|| Color::rgba(tex[off], tex[off + 1], tex[off + 2], tex[off + 3]))
        }

        fn tinted(c: Color, t: Color) -> Color {
            Color::rgba(
                ((c.r as u16 * t.r as u16 + 127) / 255) as u8,
                ((c.g as u16 * t.g as u16 + 127) / 255) as u8,
                ((c.b as u16 * t.b as u16 + 127) / 255) as u8,
                ((c.a as u16 * t.a as u16 + 127) / 255) as u8,
            )
        }

        pub fn blit_sub(
            buf: &mut SoftwareBuffer,
            tex: &[u8],
            tex_w: u32,
            src: (u32, u32, u32, u32),
            dst: (i32, i32, u32, u32),
            flip: (bool, bool),
            tint: Option<Color>,
        ) {
            let (src_x, src_y, src_w, src_h) = src;
            let (dst_x, dst_y, dst_w, dst_h) = dst;
            for dy in 0..dst_h {
                for dx in 0..dst_w {
                    let mx = if flip.0 { dst_w - 1 - dx } else { dx };
                    let my = if flip.1 { dst_h - 1 - dy } else { dy };
                    let sx = src_x + (mx * src_w / dst_w.max(1));
                    let sy = src_y + (my * src_h / dst_h.max(1));
                    let off = (sy as usize * tex_w as usize + sx as usize) * 4;
                    if let Some(c) = sample(tex, off) {
                        let c = tint.map_or(c, |t| tinted(c, t));
                        set_pixel(buf, dst_x + dx as i32, dst_y + dy as i32, c);
                    }
                }
            }
        }

        pub fn hgrad(buf: &mut SoftwareBuffer, r: (i32, i32, u32, u32), l: Color, rt: Color) {
            let (x, y, w, h) = r;
            let w_max = w.saturating_sub(1).max(1);
            for dx in 0..w as i32 {
                let color = lerp_color_ratio(l, rt, dx as u32, w_max);
                for dy in 0..h as i32 {
                    set_pixel(buf, x + dx, y + dy, color);
                }
            }
        }

        pub fn four_corner(buf: &mut SoftwareBuffer, r: (i32, i32, u32, u32), c: [Color; 4]) {
            let (x, y, w, h) = r;
            let h_max = h.saturating_sub(1).max(1);
            let w_max = w.saturating_sub(1).max(1);
            for dy in 0..h as i32 {
                let left = lerp_color_ratio(c[0], c[2], dy as u32, h_max);
                let right = lerp_color_ratio(c[1], c[3], dy as u32, h_max);
                for dx in 0..w as i32 {
                    let color = lerp_color_ratio(left, right, dx as u32, w_max);
                    set_pixel(buf, x + dx, y + dy, color);
                }
            }
        }

        pub fn text(buf: &mut SoftwareBuffer, text: &str, x: i32, y: i32, fs: u16, c: Color) {
            let scale = if fs >= 8 { (fs / 8) as i32 } else { 1 };
            let mut cx = x;
            for ch in text.chars() {
                let glyph_data = oasis_types::bitmap_font::glyph(ch);
                let (left_pad, advance) = oasis_types::bitmap_font::glyph_metrics(ch);
                let left_pad = left_pad as i32;
                for row in 0..8i32 {
                    let bits = glyph_data[row as usize];
                    for col in 0..8i32 {
                        if bits & (0x80 >> col) != 0 {
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    set_pixel(
                                        buf,
                                        cx + (col - left_pad) * scale + sx,
                                        y + row * scale + sy,
                                        c,
                                    );
                                }
                            }
                        }
                    }
                }
                cx += advance as i32 * scale;
            }
        }

        pub fn rounded_rect(
            buf: &mut SoftwareBuffer,
            rect: (i32, i32, u32, u32),
            radius: u16,
            c: Color,
        ) {
            let (x, y, w, h) = rect;
            if w == 0 || h == 0 {
                return;
            }
            let r = (radius as u32).min(w / 2).min(h / 2) as i32;
            let (wi, hi) = (w as i32, h as i32);
            for dy in r..(hi - r) {
                hline(buf, x, x + wi - 1, y + dy, c);
            }
            let mut cx = 0i32;
            let mut cy = r;
            let mut d = 1 - r;
            while cx <= cy {
                hline(buf, x + r - cy, x + wi - 1 - r + cy, y + r - cx, c);
                if cx != 0 {
                    hline(buf, x + r - cy, x + wi - 1 - r + cy, y + hi - 1 - r + cx, c);
                }
                if cx != cy {
                    hline(buf, x + r - cx, x + wi - 1 - r + cx, y + r - cy, c);
                }
                hline(buf, x + r - cx, x + wi - 1 - r + cx, y + hi - 1 - r + cy, c);
                cx += 1;
                if d < 0 {
                    d += 2 * cx + 1;
                } else {
                    cy -= 1;
                    d += 2 * (cx - cy) + 1;
                }
            }
        }

        pub fn circle(buf: &mut SoftwareBuffer, cx: i32, cy: i32, radius: u16, c: Color) {
            let r = radius as i32;
            let (mut x, mut y, mut d) = (0i32, r, 1 - r);
            while x <= y {
                hline(buf, cx - y, cx + y, cy + x, c);
                if x != 0 {
                    hline(buf, cx - y, cx + y, cy - x, c);
                }
                if x != y {
                    hline(buf, cx - x, cx + x, cy + y, c);
                    hline(buf, cx - x, cx + x, cy - y, c);
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
    }

    /// Tiny deterministic xorshift PRNG for reproducible randomized tests.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// Uniform-ish integer in `lo..hi`.
        fn range(&mut self, lo: i64, hi: i64) -> i64 {
            lo + (self.next() % (hi - lo) as u64) as i64
        }
        /// True with probability `1/n`.
        fn one_in(&mut self, n: u64) -> bool {
            self.next().is_multiple_of(n)
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
        fn color(&mut self, opaque: bool) -> Color {
            let v = self.next().to_le_bytes();
            Color::rgba(v[0], v[1], v[2], if opaque { 255 } else { v[3] })
        }
    }

    /// Two identical buffers (random contents + clip): one for the fast
    /// path, one for the reference. Clip origins are non-negative -- the
    /// only domain where the old `set_pixel` clip test was well defined.
    fn buffer_pair(rng: &mut Rng) -> (SoftwareBuffer, SoftwareBuffer) {
        let (w, h) = (64u32, 48u32);
        let pixels = rng.bytes((w * h * 4) as usize);
        let clip = (!rng.one_in(3)).then(|| ClipRect {
            x: rng.range(0, 70) as i32,
            y: rng.range(0, 52) as i32,
            w: rng.range(0, 70) as u32,
            h: rng.range(0, 52) as u32,
        });
        let mut a = SoftwareBuffer::new(w, h);
        let mut b = SoftwareBuffer::new(w, h);
        for buf in [&mut a, &mut b] {
            buf.data_mut().copy_from_slice(&pixels);
            buf.set_clip(clip);
        }
        (a, b)
    }

    fn random_rect(rng: &mut Rng) -> (i32, i32, u32, u32) {
        (
            rng.range(-40, 70) as i32,
            rng.range(-40, 55) as i32,
            rng.range(0, 100) as u32,
            rng.range(0, 80) as u32,
        )
    }

    /// Random texture; alpha is opaque, translucent, or mixed; sometimes the
    /// slice is truncated to exercise the out-of-range sample skip.
    fn random_texture(rng: &mut Rng) -> (Vec<u8>, u32, u32) {
        let tw = rng.range(1, 40) as u32;
        let th = rng.range(1, 40) as u32;
        let mut tex = rng.bytes((tw * th * 4) as usize);
        match rng.next() % 3 {
            0 => tex.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255),
            1 => tex
                .iter_mut()
                .skip(3)
                .step_by(4)
                .for_each(|a| *a = if *a < 128 { 255 } else { *a }),
            _ => {},
        }
        if rng.one_in(5) {
            let keep = rng.range(0, tex.len() as i64 + 1) as usize;
            tex.truncate(keep);
        }
        (tex, tw, th)
    }

    #[test]
    fn golden_blits_match_reference() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for iter in 0..3000 {
            let (mut fast, mut slow) = buffer_pair(&mut rng);
            let (tex, tw, th) = random_texture(&mut rng);
            let dst = random_rect(&mut rng);
            let (dx, dy, dw, dh) = dst;
            let opaque_tint = rng.one_in(2);
            let tint = rng.color(opaque_tint);
            let variant = iter % 5;
            match variant {
                0 => {
                    // Bias toward the unscaled fast path.
                    let (dw, dh) = if rng.one_in(2) { (tw, th) } else { (dw, dh) };
                    fast.blit_texture(&tex, tw, th, dx, dy, dw, dh);
                    let d = (dx, dy, dw, dh);
                    reference::blit_sub(
                        &mut slow,
                        &tex,
                        tw,
                        (0, 0, tw, th),
                        d,
                        (false, false),
                        None,
                    );
                },
                1 | 3 => {
                    let sx = rng.range(0, tw as i64) as u32;
                    let sy = rng.range(0, th as i64) as u32;
                    let sw = rng.range(0, (tw - sx) as i64 + 1) as u32;
                    let sh = rng.range(0, (th - sy) as i64 + 1) as u32;
                    let (dw, dh) = if rng.one_in(2) { (sw, sh) } else { (dw, dh) };
                    let t = (variant == 3).then_some(tint);
                    match t {
                        None => fast.blit_texture_sub(&tex, tw, sx, sy, sw, sh, dx, dy, dw, dh),
                        Some(t) => fast
                            .blit_texture_sub_tinted(&tex, tw, sx, sy, sw, sh, dx, dy, dw, dh, t),
                    }
                    let (s, d) = ((sx, sy, sw, sh), (dx, dy, dw, dh));
                    reference::blit_sub(&mut slow, &tex, tw, s, d, (false, false), t);
                },
                2 => {
                    fast.blit_texture_tinted(&tex, tw, th, dx, dy, dw, dh, tint);
                    let s = (0, 0, tw, th);
                    reference::blit_sub(&mut slow, &tex, tw, s, dst, (false, false), Some(tint));
                },
                _ => {
                    let flip = (rng.one_in(2), rng.one_in(2));
                    fast.blit_texture_flipped(&tex, tw, th, dx, dy, dw, dh, flip.0, flip.1);
                    reference::blit_sub(&mut slow, &tex, tw, (0, 0, tw, th), dst, flip, None);
                },
            }
            assert!(
                fast.data() == slow.data(),
                "blit variant {variant} diverged at iter {iter}"
            );
        }
    }

    #[test]
    fn golden_gradients_and_text_match_reference() {
        let mut rng = Rng(0x0123_4567_89AB_CDEF);
        for iter in 0..1500 {
            let (mut fast, mut slow) = buffer_pair(&mut rng);
            let rect = random_rect(&mut rng);
            let (x, y, w, h) = rect;
            let opaque = rng.one_in(2);
            let c = [
                rng.color(opaque),
                rng.color(opaque),
                rng.color(opaque),
                rng.color(opaque),
            ];
            match iter % 3 {
                0 => {
                    fast.fill_rect_horizontal_gradient(x, y, w, h, c[0], c[1]);
                    reference::hgrad(&mut slow, rect, c[0], c[1]);
                },
                1 => {
                    fast.fill_rect_four_corner_gradient(x, y, w, h, c[0], c[1], c[2], c[3]);
                    reference::four_corner(&mut slow, rect, c);
                },
                _ => {
                    let fs = rng.range(0, 33) as u16;
                    let text = "Hi! gjpq {Oasis} 0123 _|~";
                    fast.draw_bitmap_text(
                        text,
                        x,
                        y,
                        fs,
                        c[0],
                        oasis_types::bitmap_font::glyph,
                        oasis_types::bitmap_font::glyph_metrics,
                    );
                    if fs != 0 {
                        reference::text(&mut slow, text, x, y, fs, c[0]);
                    }
                },
            }
            assert!(
                fast.data() == slow.data(),
                "variant {} diverged at iter {iter}",
                iter % 3
            );
        }
    }

    #[test]
    fn golden_opaque_rounded_rect_and_circle_match_reference() {
        let mut rng = Rng(0xDEAD_BEEF_F00D_CAFE);
        for iter in 0..2000u32 {
            let (mut fast, mut slow) = buffer_pair(&mut rng);
            let rect = random_rect(&mut rng);
            let radius = rng.range(0, 50) as u16;
            let c = rng.color(true);
            if iter.is_multiple_of(2) {
                fast.fill_rounded_rect(rect.0, rect.1, rect.2, rect.3, radius, c);
                reference::rounded_rect(&mut slow, rect, radius, c);
            } else {
                fast.fill_circle(rect.0, rect.1, radius, c);
                reference::circle(&mut slow, rect.0, rect.1, radius, c);
            }
            assert!(
                fast.data() == slow.data(),
                "shape diverged at iter {iter}: {rect:?} r={radius}"
            );
        }
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

    /// Every covered pixel of a translucent shape must be blended exactly
    /// once: over a uniform background they all end up the same value.
    fn assert_uniform_single_blend(buf: &SoftwareBuffer, bg: Color, fill: Color) {
        let mut once = SoftwareBuffer::new(1, 1);
        once.clear(bg);
        once.set_pixel(0, 0, fill);
        let expected = [
            once.data()[0],
            once.data()[1],
            once.data()[2],
            once.data()[3],
        ];
        let bg_px = [bg.r, bg.g, bg.b, bg.a];
        let mut covered = 0;
        for px in buf.data().as_chunks::<4>().0 {
            if *px != bg_px {
                assert_eq!(*px, expected, "pixel blended more than once");
                covered += 1;
            }
        }
        assert!(covered > 0);
    }

    #[test]
    fn translucent_rounded_rect_blends_each_pixel_once() {
        let bg = Color::rgb(10, 20, 30);
        let fill = Color::rgba(200, 100, 50, 128);
        for (w, h, r) in [
            (40, 30, 8),
            (40, 30, 15),
            (30, 30, 15),
            (31, 17, 8),
            (9, 40, 4),
        ] {
            let mut buf = SoftwareBuffer::new(50, 50);
            buf.clear(bg);
            buf.fill_rounded_rect(3, 4, w, h, r, fill);
            assert_uniform_single_blend(&buf, bg, fill);
        }
    }

    #[test]
    fn translucent_circle_blends_each_pixel_once() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgba(255, 255, 255, 100);
        for r in [0, 1, 2, 5, 12, 20] {
            let mut buf = SoftwareBuffer::new(50, 50);
            buf.clear(bg);
            buf.fill_circle(25, 25, r, fill);
            assert_uniform_single_blend(&buf, bg, fill);
        }
    }

    #[test]
    fn negative_clip_origin_bounds_right_edge() {
        // A clip rect hanging off the left edge still bounds the right side.
        let mut buf = SoftwareBuffer::new(10, 1);
        buf.set_clip(Some(ClipRect {
            x: -5,
            y: 0,
            w: 8,
            h: 1,
        }));
        buf.blit_texture(&[255u8; 40], 10, 1, 0, 0, 10, 1);
        buf.set_pixel(5, 0, Color::WHITE);
        assert_eq!(&buf.data()[..12], &[255u8; 12]);
        assert!(buf.data()[12..].iter().all(|&b| b == 0));
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
