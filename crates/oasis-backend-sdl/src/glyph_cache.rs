//! Glyph cache and text rendering for the SDL3 backend.
//!
//! Manages a fixed-size LRU cache of pre-rendered glyph textures. Each
//! unique (character, font_size, color, bold, italic) combination is
//! rasterized once into an RGBA buffer, uploaded as an SDL streaming
//! texture, and reused on subsequent draws.

use oasis_core::backend::{BackendErrExt, Color, SdiText};
use oasis_core::error::Result;
use oasis_rasterize::GlyphCacheKey;
use sdl3::pixels::PixelFormat;
use sdl3::render::Texture;

use super::{SdlBackend, font, frect};

/// Maximum number of cached glyph textures before LRU eviction kicks in.
pub(crate) const MAX_GLYPH_CACHE_SIZE: usize = 2048;

// -------------------------------------------------------------------
// Glyph rendering helpers
// -------------------------------------------------------------------

impl SdlBackend {
    /// Render a single glyph to an RGBA buffer and load it as an SDL
    /// texture. Returns the texture ID stored in `self.textures`.
    pub(crate) fn render_glyph_texture(
        &mut self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<u64> {
        let fs = font_size.max(1) as i32;
        let advance = oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
        let bold_extra = if bold { 1 } else { 0 };
        let italic_extra = if italic { fs / 32 + 1 } else { 0 };
        let gw = (advance + bold_extra + italic_extra).max(1) as u32;
        let gh = font_size.max(1) as u32;
        let mut rgba = vec![0u8; (gw * gh * 4) as usize];

        // Triangle glyphs (▲ / ▼) scale up chunky through the bitmap
        // path because the 8×8 source only has six rows of triangle
        // data. Render them directly at the target resolution.
        if oasis_types::bitmap_font::is_smooth_triangle(ch) {
            for y in 0..gh as i32 {
                let Some((x0, x1)) =
                    oasis_types::bitmap_font::smooth_triangle_span(ch, y, gw as i32, gh as i32)
                else {
                    continue;
                };
                for x in x0..=x1 {
                    Self::set_glyph_pixel(&mut rgba, gw, gh, x, y, color);
                    if bold {
                        Self::set_glyph_pixel(&mut rgba, gw, gh, x + 1, y, color);
                    }
                }
            }
        } else {
            let glyph_data = font::glyph(ch);
            let (left_pad, _) = font::glyph_metrics(ch);
            let left_pad = left_pad as i32;

            for row in 0..8i32 {
                let bits = glyph_data[row as usize];
                if bits == 0 {
                    continue;
                }
                let oy0 = row * fs / 8;
                let oy1 = (row + 1) * fs / 8;
                let italic_off = if italic { (7 - row) * fs / 32 } else { 0 };

                for col in 0..8i32 {
                    if bits & (0x80 >> col) == 0 {
                        continue;
                    }
                    let src_col = col - left_pad;
                    let ox0 = src_col * fs / 8;
                    let ox1 = (src_col + 1) * fs / 8;
                    // Fill the scaled rectangle in the buffer.
                    for py in oy0..oy1.max(oy0 + 1) {
                        for px in ox0..ox1.max(ox0 + 1) {
                            let bx = px + italic_off;
                            Self::set_glyph_pixel(&mut rgba, gw, gh, bx, py, color);
                            if bold {
                                Self::set_glyph_pixel(&mut rgba, gw, gh, bx + 1, py, color);
                            }
                        }
                    }
                }
            }
        }

        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormat::ABGR8888, gw, gh)
            .backend_err()?;
        texture
            .with_lock(None, |buf: &mut [u8], pitch: usize| {
                let row_bytes = (gw as usize) * 4;
                for y in 0..gh as usize {
                    let src_start = y * row_bytes;
                    let dst_start = y * pitch;
                    buf[dst_start..dst_start + row_bytes]
                        .copy_from_slice(&rgba[src_start..src_start + row_bytes]);
                }
            })
            .backend_err()?;
        texture.set_blend_mode(sdl3::render::BlendMode::Blend);

        // SAFETY: The texture borrows from self.texture_creator
        // which lives in the same struct. The explicit `Drop` impl
        // clears all textures before texture_creator is dropped.
        let texture: Texture<'static> = unsafe { std::mem::transmute(texture) };

        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, texture);
        Ok(id)
    }

    /// Write a single pixel into a glyph RGBA buffer, with bounds
    /// checking.
    fn set_glyph_pixel(buf: &mut [u8], w: u32, h: u32, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return;
        }
        let offset = (y as usize * w as usize + x as usize) * 4;
        if offset + 3 < buf.len() {
            buf[offset] = color.r;
            buf[offset + 1] = color.g;
            buf[offset + 2] = color.b;
            buf[offset + 3] = color.a;
        }
    }
}

// -------------------------------------------------------------------
// SdiText: Text system (glyph-cache backed)
// -------------------------------------------------------------------

impl SdiText for SdlBackend {
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
        if text.is_empty() || color.a == 0 || font_size == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let mut cx = tx;

        for ch in text.chars() {
            let key = GlyphCacheKey::new(ch, font_size, color, bold, italic);
            if self.glyph_cache.contains_key(&key) {
                // Cache hit: update LRU access counter.
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
            } else {
                // Evict LRU entry when cache is full.
                while self.glyph_cache.len() >= MAX_GLYPH_CACHE_SIZE {
                    if let Some((&oldest_key, _)) =
                        self.glyph_access.iter().min_by_key(|&(_, &ts)| ts)
                    {
                        if let Some(tex_id) = self.glyph_cache.remove(&oldest_key) {
                            self.textures.remove(&tex_id);
                        }
                        self.glyph_access.remove(&oldest_key);
                    } else {
                        break;
                    }
                }
                // Render the glyph to a small RGBA buffer.
                let tex_id = self.render_glyph_texture(ch, font_size, color, bold, italic)?;
                self.glyph_cache.insert(key, tex_id);
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
            }
            // Blit the cached glyph texture.
            if let Some(&tex_id) = self.glyph_cache.get(&key)
                && let Some(texture) = self.textures.get(&tex_id)
            {
                let query = texture.query();
                let _ = self
                    .canvas
                    .copy(texture, None, frect(cx, ty, query.width, query.height));
            }
            cx += oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
        }
        Ok(())
    }

    fn measure_text_height(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 1.2 (the actual rendered row height).
        (f64::from(font_size.max(8)) * 1.2).ceil() as u32
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 0.85 (baseline offset from top).
        (f64::from(font_size.max(8)) * 0.85).ceil() as u32
    }
}
