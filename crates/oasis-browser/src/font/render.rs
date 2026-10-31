//! Web font glyph rendering: rasterize glyphs from the [`FontRegistry`]
//! and blit them to the backend using temporary textures.

use std::cell::RefCell;
use std::collections::HashMap;

use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

use super::registry::{FontId, FontRegistry};

/// Glyph cache key: (font_id, codepoint, size_tenths, r, g, b, a).
type GlyphTexKey = (u32, char, u32, u8, u8, u8, u8);

/// Most glyph textures [`GlyphTextureCache`] keeps.
pub const MAX_GLYPH_TEXTURES: usize = 4096;

/// How many least recently used textures a full cache evicts at once
/// (an eighth: eviction scans the cache, so it should be rare).
const GLYPH_EVICT_BATCH: usize = MAX_GLYPH_TEXTURES / 8;

/// Cache of uploaded glyph textures keyed by font, codepoint, size, and color.
///
/// Textures are uploaded once and reused across frames until the cache
/// is cleared (navigation, font registry rebuild). Keys include the
/// colour, so an animated text colour mints a texture per glyph per
/// step: at [`MAX_GLYPH_TEXTURES`] the least recently drawn eighth is
/// destroyed. (Emptying the whole cache instead re-rasterized every
/// glyph on the page, the ones drawn this frame included.)
pub struct GlyphTextureCache {
    /// Texture and the use stamp of its last draw.
    textures: HashMap<GlyphTexKey, (TextureId, u64)>,
    /// Monotonic use counter (the stamp source).
    clock: u64,
}

impl GlyphTextureCache {
    pub fn new() -> Self {
        GlyphTextureCache {
            textures: HashMap::new(),
            clock: 0,
        }
    }

    /// Destroy all cached textures via the backend.
    pub fn clear(&mut self, backend: &mut dyn SdiBackend) {
        for (_, (tex_id, _)) in self.textures.drain() {
            let _ = backend.destroy_texture(tex_id);
        }
    }

    /// The cached texture for `key`, marking it used.
    fn get(&mut self, key: &GlyphTexKey) -> Option<TextureId> {
        self.clock += 1;
        let clock = self.clock;
        self.textures.get_mut(key).map(|(tex, used)| {
            *used = clock;
            *tex
        })
    }

    /// Cache `tex` under `key`, evicting the least recently used
    /// [`GLYPH_EVICT_BATCH`] textures first when the cache is full.
    fn insert(&mut self, key: GlyphTexKey, tex: TextureId, backend: &mut dyn SdiBackend) {
        if self.textures.len() >= MAX_GLYPH_TEXTURES {
            let mut stamps: Vec<u64> = self.textures.values().map(|&(_, used)| used).collect();
            let nth = GLYPH_EVICT_BATCH.min(stamps.len()) - 1;
            let (_, &mut cutoff, _) = stamps.select_nth_unstable(nth);
            self.textures.retain(|_, &mut (tex, used)| {
                let keep = used > cutoff;
                if !keep {
                    let _ = backend.destroy_texture(tex);
                }
                keep
            });
        }
        self.clock += 1;
        self.textures.insert(key, (tex, self.clock));
    }

    /// Empty the cache, handing its textures to `out` for destruction
    /// later (navigation has no backend). Called when a page load
    /// rebuilds the font registry: font ids restart at 0, so entries
    /// keyed by the old ids would draw the previous page's glyphs.
    pub fn clear_into(&mut self, out: &mut Vec<TextureId>) {
        out.extend(self.textures.drain().map(|(_, (tex, _))| tex));
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

    let size_tenths = (fs * 10.0) as u32;
    for ch in text.chars() {
        let cache_key = (
            font_id.as_raw(),
            ch,
            size_tenths,
            color.r,
            color.g,
            color.b,
            color.a,
        );

        // Rasterize glyph (cached inside FontRegistry).
        let Some(glyph) = reg.rasterize_glyph(font_id, ch, fs) else {
            continue;
        };
        let advance = glyph.advance_width;
        let glyph_w = glyph.width;
        let glyph_h = glyph.height;
        let x_off = glyph.x_offset;
        let y_off = glyph.y_offset;

        // Get or create glyph texture.
        let mut cached = tex_cache.get(&cache_key);
        if cached.is_none() && glyph_w > 0 && glyph_h > 0 {
            // Convert alpha bitmap to RGBA.
            let pixel_count = (glyph_w * glyph_h) as usize;
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for &alpha in &glyph.bitmap {
                rgba.push(color.r);
                rgba.push(color.g);
                rgba.push(color.b);
                rgba.push(((alpha as u16 * color.a as u16) / 255) as u8);
            }

            if let Ok(tex_id) = backend.load_texture(glyph_w, glyph_h, &rgba) {
                tex_cache.insert(cache_key, tex_id, backend);
                cached = Some(tex_id);
            }
        }

        // Blit the glyph texture if it exists.
        if let Some(tex_id) = cached {
            let gx = (cursor_x + x_off) as i32;
            let gy = (baseline_y - y_off) as i32 - glyph_h as i32;
            backend.blit(tex_id, gx, gy, glyph_w, glyph_h)?;
        }

        cursor_x += advance;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the glyph cache was never emptied, so after a page
    /// load rebuilt the font registry (ids restart at 0) the next page
    /// drew the previous page's glyphs, and textures leaked.
    #[test]
    fn clear_into_hands_back_every_texture() {
        let mut cache = GlyphTextureCache::new();
        cache
            .textures
            .insert((0, 'a', 120, 0, 0, 0, 255), (TextureId(7), 0));
        cache
            .textures
            .insert((0, 'b', 120, 0, 0, 0, 255), (TextureId(8), 0));
        let mut retired = Vec::new();
        cache.clear_into(&mut retired);
        assert!(cache.is_empty());
        retired.sort_by_key(|t| t.0);
        assert_eq!(retired, vec![TextureId(7), TextureId(8)]);
    }

    /// Regression: a full cache emptied itself, destroying (and then
    /// re-rasterizing) every glyph, the ones just drawn included. Now
    /// only the least recently used eighth goes.
    #[test]
    fn full_cache_evicts_least_recently_used_glyphs() {
        let mut backend = oasis_test_backend::NullBackend::new(64, 64);
        let mut cache = GlyphTextureCache::new();
        let key = |i: usize| (0, 'a', i as u32, 0, 0, 0, 255);
        for i in 0..MAX_GLYPH_TEXTURES {
            cache.insert(key(i), TextureId(i as u64), &mut backend);
        }
        // Touch the oldest glyph: it is now the most recently used.
        assert_eq!(cache.get(&key(0)), Some(TextureId(0)));
        cache.insert(key(MAX_GLYPH_TEXTURES), TextureId(9999), &mut backend);

        assert_eq!(cache.len(), MAX_GLYPH_TEXTURES - GLYPH_EVICT_BATCH + 1);
        assert!(cache.get(&key(0)).is_some(), "recently drawn glyph kept");
        assert!(cache.get(&key(1)).is_none(), "least recently used evicted");
        assert!(cache.get(&key(GLYPH_EVICT_BATCH)).is_none());
        assert!(cache.get(&key(GLYPH_EVICT_BATCH + 1)).is_some());
        assert!(
            cache.get(&key(MAX_GLYPH_TEXTURES)).is_some(),
            "new glyph cached"
        );
    }
}
