//! Glyph cache and text rendering for the SDL3 backend.
//!
//! Manages a fixed-size LRU cache of pre-rendered glyph textures.
//!
//! Opaque text — which is nearly all of it — is cached **color-independently**:
//! each (character, font_size, bold, italic) is rasterized white once and
//! tinted at blit time with SDL's texture color modulation. One texture then
//! serves every color the shell draws that character in, so re-skinning, a
//! hover color lerp, or an accent animation costs nothing. The tint is exact:
//! SDL modulates by `src * mod / 255`, a white source leaves `mod` unchanged,
//! and blending a fully opaque source yields the source.
//!
//! Translucent text keeps a color-keyed entry with the color baked into the
//! pixels — see `SdlBackend::tints_at_blit`.

use oasis_core::backend::{BackendErrExt, Color, SdiText};
use oasis_core::error::{OasisError, Result};
use oasis_rasterize::GlyphCacheKey;
use oasis_rasterize::ttf::TtfFont;
use sdl3::pixels::PixelFormat;
use sdl3::render::Texture;

use super::{SdlBackend, font, frect};

/// Maximum number of cached glyph textures before LRU eviction kicks in.
pub(crate) const MAX_GLYPH_CACHE_SIZE: usize = 2048;

/// Fraction of the cache evicted when it fills, as a divisor: the oldest
/// `MAX_GLYPH_CACHE_SIZE / EVICT_DIVISOR` entries go at once. Evicting in
/// batches amortizes the O(n log n) sort over many insertions instead of
/// paying an O(n) min-scan per new glyph.
const EVICT_DIVISOR: usize = 4;

/// A cached glyph: its texture plus the dimensions it was rasterized at.
///
/// The dimensions are stored here so the draw path never calls
/// `Texture::query()` (an FFI round-trip) per glyph per frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphEntry {
    /// Texture id in `SdlBackend::textures`. `0` means "no pixels" (e.g. a
    /// TTF space glyph): the id counter starts at 1, so the blit lookup
    /// misses and only the advance is applied.
    pub(crate) texture: u64,
    /// Rasterized width in pixels.
    pub(crate) w: u32,
    /// Rasterized height in pixels.
    pub(crate) h: u32,
    /// Blit offset from the pen position (bitmap glyphs: 0).
    pub(crate) off_x: i32,
    /// Blit offset from the top of the line box (bitmap glyphs: 0).
    pub(crate) off_y: i32,
    /// Pen advance in pixels. For bitmap glyphs this is exactly
    /// `glyph_advance_scaled`, so the no-font path stays pixel-identical;
    /// for TTF glyphs it is the font's rounded advance. `measure_text`
    /// accumulates the same integers, keeping layout and drawing in
    /// agreement.
    pub(crate) advance: i32,
    /// Value of `glyph_access_counter` at the last hit (LRU timestamp).
    pub(crate) last_used: u64,
    /// Color modulation currently set on the texture (SDL texture state
    /// persists across frames). Tracked so the draw path only issues
    /// `set_color_mod` when the tint actually changes for this glyph.
    pub(crate) mod_rgb: (u8, u8, u8),
}

// -------------------------------------------------------------------
// Glyph rendering helpers
// -------------------------------------------------------------------

impl SdlBackend {
    /// Render a single glyph into an RGBA buffer and load it as an SDL texture.
    /// Returns the texture ID stored in `self.textures` plus the rasterized
    /// dimensions.
    ///
    /// `color` is white for cache entries that are tinted at blit time, and the
    /// text color itself for the translucent entries that bake it in.
    pub(crate) fn render_glyph_texture(
        &mut self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<(u64, u32, u32)> {
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

        let id = self.upload_glyph_rgba(&rgba, gw, gh)?;
        Ok((id, gw, gh))
    }

    /// Upload a glyph RGBA buffer as a blended SDL texture and return its id.
    fn upload_glyph_rgba(&mut self, rgba: &[u8], gw: u32, gh: u32) -> Result<u64> {
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

    /// Rasterize a glyph from the active TTF font into a texture. Returns
    /// `(texture_id, w, h, off_x, off_y, advance)`; empty glyphs (spaces)
    /// return texture id 0 with only the advance populated.
    ///
    /// Faux-bold is baked into the coverage as a 1px double-strike so the
    /// draw loop stays identical for both font paths; faux-italic is not
    /// applied (a real italic face belongs in the font file).
    fn render_ttf_glyph(
        &mut self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
    ) -> Result<(u64, u32, u32, i32, i32, i32)> {
        let px = font_size.max(1) as f32;
        let Some(ttf) = self.ttf_font.as_ref() else {
            return Err(OasisError::Backend(
                "render_ttf_glyph without a font".into(),
            ));
        };
        let glyph = ttf.rasterize(ch, px);
        let ascent = ttf.ascent(px);
        let off_x = glyph.xmin;
        let off_y = ascent - glyph.ymin - glyph.height as i32;
        let advance = glyph.advance;

        if glyph.width == 0 || glyph.height == 0 {
            return Ok((0, 0, 0, off_x, off_y, advance));
        }

        let (coverage, gw) = if bold {
            // Double-strike: max of the coverage with itself shifted 1px.
            let w2 = glyph.width + 1;
            let mut out = vec![0u8; (w2 * glyph.height) as usize];
            for y in 0..glyph.height as usize {
                for x in 0..glyph.width as usize {
                    let v = glyph.coverage[y * glyph.width as usize + x];
                    let i = y * w2 as usize + x;
                    out[i] = out[i].max(v);
                    out[i + 1] = v;
                }
            }
            (out, w2)
        } else {
            (glyph.coverage, glyph.width)
        };
        let gh = glyph.height;

        let mut rgba = Vec::with_capacity(coverage.len() * 4);
        for &cov in &coverage {
            rgba.push(color.r);
            rgba.push(color.g);
            rgba.push(color.b);
            rgba.push(((cov as u16 * color.a as u16) / 255) as u8);
        }

        let id = self.upload_glyph_rgba(&rgba, gw, gh)?;
        Ok((id, gw, gh, off_x, off_y, advance))
    }

    /// Look up a glyph, rendering and caching it on miss. Returns the entry
    /// plus whether the caller must re-apply `set_color_mod` before blitting
    /// (i.e. the requested tint differs from what the texture last carried).
    ///
    /// Opaque text gets a color-independent entry (white pixels, tinted at
    /// blit); translucent text gets a color-keyed entry with the color baked
    /// in — see [`Self::tints_at_blit`] for why.
    fn glyph_entry(
        &mut self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<(GlyphEntry, bool)> {
        let tinted = Self::tints_at_blit(color);
        let rgb = (color.r, color.g, color.b);
        let key = if tinted {
            GlyphCacheKey::colorless(ch, font_size, bold, italic)
        } else {
            GlyphCacheKey::new(ch, font_size, color, bold, italic)
        };
        self.glyph_access_counter += 1;
        let stamp = self.glyph_access_counter;

        if let Some(entry) = self.glyph_cache.get_mut(&key) {
            // Cache hit: touch the in-entry LRU stamp (no second HashMap).
            entry.last_used = stamp;
            let needs_mod = tinted && entry.mod_rgb != rgb;
            if needs_mod {
                entry.mod_rgb = rgb;
            }
            return Ok((*entry, needs_mod));
        }

        if self.glyph_cache.len() >= MAX_GLYPH_CACHE_SIZE {
            self.evict_oldest_glyphs();
        }

        let raster = if tinted {
            Color::rgba(255, 255, 255, 255)
        } else {
            color
        };
        // A skin TTF font takes over any character it has a glyph for;
        // everything else (e.g. the ▲/▼ UI triangles) falls back to the
        // bitmap font. `measure_text` makes the same per-character choice.
        let use_ttf = self.ttf_font.as_ref().is_some_and(|f| f.has_glyph(ch));
        let (texture, w, h, off_x, off_y, advance) = if use_ttf {
            self.render_ttf_glyph(ch, font_size, raster, bold)?
        } else {
            let (texture, w, h) = self.render_glyph_texture(ch, font_size, raster, bold, italic)?;
            let advance = oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
            (texture, w, h, 0, 0, advance)
        };
        // Fresh textures carry SDL's default modulation (white).
        let mut entry = GlyphEntry {
            texture,
            w,
            h,
            off_x,
            off_y,
            advance,
            last_used: stamp,
            mod_rgb: (255, 255, 255),
        };
        let needs_mod = tinted && entry.mod_rgb != rgb;
        if needs_mod {
            entry.mod_rgb = rgb;
        }
        self.glyph_cache.insert(key, entry);
        Ok((entry, needs_mod))
    }

    /// Whether a glyph in this color is served from the color-independent
    /// cache and tinted at blit time.
    ///
    /// Opaque text is: SDL modulates a white source as `255 * mod / 255 == mod`
    /// and, with a fully opaque source, the blend result *is* the source — so a
    /// tinted white glyph is byte-for-byte what rasterizing in that color used
    /// to produce, and every color of a character shares one texture.
    ///
    /// Translucent text is not. Modulating alpha moves the blit onto SDL's
    /// modulated blend path, whose rounding differs from the per-pixel-alpha
    /// path by up to 2/255 against a contrasting background. That is invisible
    /// but it is not identical, and there is little to win: a glyph that is
    /// fading has a different alpha — and so a different key — on the next
    /// frame regardless of how the cache is keyed.
    fn tints_at_blit(color: Color) -> bool {
        color.a == 255
    }

    /// Drop the oldest quarter of the cache in one pass.
    fn evict_oldest_glyphs(&mut self) {
        let victims = (MAX_GLYPH_CACHE_SIZE / EVICT_DIVISOR).max(1);
        let mut stamps: Vec<(u64, GlyphCacheKey)> = self
            .glyph_cache
            .iter()
            .map(|(&key, entry)| (entry.last_used, key))
            .collect();
        if stamps.is_empty() {
            return;
        }
        // Partial sort: we only need the `victims` oldest.
        let pivot = victims.min(stamps.len()).saturating_sub(1);
        stamps.select_nth_unstable_by_key(pivot, |&(stamp, _)| stamp);
        for &(_, key) in stamps.iter().take(victims) {
            if let Some(entry) = self.glyph_cache.remove(&key) {
                self.textures.remove(&entry.texture);
            }
        }
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
            let (entry, set_mod) = self.glyph_entry(ch, font_size, color, bold, italic)?;
            // Dimensions come from the cache entry, not a `texture.query()`
            // FFI round-trip per glyph per frame.
            if let Some(texture) = self.textures.get_mut(&entry.texture) {
                if set_mod {
                    // White glyph, tinted to the requested color by SDL.
                    // Texture modulation persists, so this only fires when
                    // the glyph's tracked tint actually changes.
                    texture.set_color_mod(color.r, color.g, color.b);
                }
                let _ = self.canvas.copy(
                    texture,
                    None,
                    frect(cx + entry.off_x, ty + entry.off_y, entry.w, entry.h),
                );
            }
            // Bitmap entries carry `glyph_advance_scaled` here, so the
            // no-font path advances exactly as before.
            cx += entry.advance;
        }
        Ok(())
    }

    fn measure_text_height(&self, font_size: u16) -> u32 {
        if let Some(ttf) = &self.ttf_font {
            // Same pixel size the rasterizer uses, so line boxes and
            // baseline placement come from one set of metrics.
            return ttf.line_height(font_size.max(1) as f32);
        }
        // Match WASM: font_size * 1.2 (the actual rendered row height).
        (f64::from(font_size.max(8)) * 1.2).ceil() as u32
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        if let Some(ttf) = &self.ttf_font {
            return ttf.ascent(font_size.max(1) as f32).max(0) as u32;
        }
        // Match WASM: font_size * 0.85 (baseline offset from top).
        (f64::from(font_size.max(8)) * 0.85).ceil() as u32
    }

    fn set_font(&mut self, font: Option<&[u8]>) {
        if font.is_none() && self.ttf_font.is_none() {
            return; // Bitmap → bitmap: keep the warm glyph cache.
        }
        // Cached glyph textures belong to the outgoing font.
        for (_, entry) in self.glyph_cache.drain() {
            self.textures.remove(&entry.texture);
        }
        self.ttf_font = font.and_then(|bytes| match TtfFont::from_bytes(bytes) {
            Ok(f) => Some(f),
            Err(e) => {
                log::warn!("skin font rejected, keeping bitmap font: {e}");
                None
            },
        });
    }
}
