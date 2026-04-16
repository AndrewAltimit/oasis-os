//! Web font glyph rendering: rasterize glyphs from the [`FontRegistry`]
//! and blit them to the backend using temporary textures.

use std::cell::RefCell;
use std::collections::HashMap;

use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

use super::registry::{FontId, FontRegistry};

/// Cache of uploaded glyph textures keyed by (font_id, codepoint, size_tenths).
///
/// Textures are uploaded once and reused across frames until the cache
/// is cleared (e.g. on navigation or font registry rebuild).
pub struct GlyphTextureCache {
    textures: HashMap<(u32, char, u32), TextureId>,
}

impl GlyphTextureCache {
    pub fn new() -> Self {
        GlyphTextureCache {
            textures: HashMap::new(),
        }
    }

    /// Destroy all cached textures via the backend.
    pub fn clear(&mut self, backend: &mut dyn SdiBackend) {
        for (_, tex_id) in self.textures.drain() {
            let _ = backend.destroy_texture(tex_id);
        }
    }

    /// Number of cached glyph textures.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

impl Default for GlyphTextureCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Render a web font text string to the backend by rasterizing
/// individual glyphs and blitting them as textures.
///
/// This is the core web font rendering path. For each character:
/// 1. Rasterize the glyph (via fontdue) to an alpha bitmap
/// 2. Convert to RGBA using the text color + alpha
/// 3. Upload as a texture (cached in `tex_cache`)
/// 4. Blit at the correct position
#[allow(clippy::too_many_arguments)]
pub fn render_web_font_text(
    backend: &mut dyn SdiBackend,
    registry: &RefCell<FontRegistry>,
    tex_cache: &mut GlyphTextureCache,
    text: &str,
    x: i32,
    y: i32,
    font_size: u16,
    color: Color,
    font_id: FontId,
) -> Result<()> {
    let fs = font_size as f32;
    let mut reg = registry.borrow_mut();

    // Get line metrics for baseline positioning.
    let (ascent, _descent) = reg.line_metrics(font_id, fs);

    let mut cursor_x = x as f32;
    let baseline_y = y as f32 + ascent;

    for ch in text.chars() {
        let size_tenths = (fs * 10.0) as u32;
        let cache_key = (font_id.as_raw(), ch, size_tenths);

        // Get or create glyph texture.
        if let std::collections::hash_map::Entry::Vacant(e) = tex_cache.textures.entry(cache_key) {
            let glyph = reg.rasterize_glyph(font_id, ch, fs);
            if glyph.width > 0 && glyph.height > 0 {
                // Convert alpha bitmap to RGBA.
                let pixel_count = (glyph.width * glyph.height) as usize;
                let mut rgba = Vec::with_capacity(pixel_count * 4);
                for &alpha in &glyph.bitmap {
                    rgba.push(color.r);
                    rgba.push(color.g);
                    rgba.push(color.b);
                    rgba.push(((alpha as u16 * color.a as u16) / 255) as u8);
                }

                if let Ok(tex_id) = backend.load_texture(glyph.width, glyph.height, &rgba) {
                    e.insert(tex_id);
                }
            }
        }

        // Look up advance width from the glyph data.
        let glyph = reg.rasterize_glyph(font_id, ch, fs);
        let advance = glyph.advance_width;

        // Blit the glyph texture if it exists.
        if let Some(&tex_id) = tex_cache.textures.get(&cache_key) {
            let gx = (cursor_x + glyph.x_offset) as i32;
            let gy = (baseline_y - glyph.y_offset) as i32 - glyph.height as i32;
            backend.blit(tex_id, gx, gy, glyph.width, glyph.height)?;
        }

        cursor_x += advance;
    }

    Ok(())
}
