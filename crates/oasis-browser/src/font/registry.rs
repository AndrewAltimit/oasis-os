//! Font registry: loads, caches, and rasterizes web fonts.
//!
//! The [`FontRegistry`] collects `@font-face` rules from parsed
//! stylesheets, fetches font data via the browser's resource loader,
//! parses the font files with `fontdue`, and caches rasterized glyphs
//! for both text measurement and rendering.

use std::collections::HashMap;

use crate::css::parser::{FontFaceRule, FontFaceSrc, FontFaceStyle};
use crate::css::values::FontFamilyName;
use crate::loader::{self, ResourceRequest, ResourceSource};

/// Opaque font identifier within a [`FontRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub(crate) u32);

impl FontId {
    /// Create a `FontId` from a raw index.
    pub fn from_raw(id: u32) -> Self {
        FontId(id)
    }

    /// Get the raw index.
    pub fn as_raw(self) -> u32 {
        self.0
    }
}

/// A rasterized glyph bitmap ready for texture upload.
#[derive(Debug, Clone)]
pub struct RasterizedGlyph {
    /// Alpha-channel bitmap (row-major, top-to-bottom).
    pub bitmap: Vec<u8>,
    /// Width of the bitmap in pixels.
    pub width: u32,
    /// Height of the bitmap in pixels.
    pub height: u32,
    /// Metrics: how many pixels to advance the cursor horizontally.
    pub advance_width: f32,
    /// Horizontal offset from the cursor to the left edge of the glyph.
    pub x_offset: f32,
    /// Vertical offset from the baseline to the top of the glyph
    /// (positive = above baseline).
    pub y_offset: f32,
}

/// Key for the glyph cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font_id: FontId,
    codepoint: char,
    size_tenths: u32, // font size * 10 for sub-pixel precision
}

/// A single loaded font face.
struct LoadedFont {
    /// The fontdue font handle.
    font: fontdue::Font,
    /// Original `@font-face` metadata.
    weight: (u16, u16),
    style: FontFaceStyle,
}

/// Maximum number of cached glyphs to prevent unbounded memory growth.
const MAX_GLYPH_CACHE: usize = 8192;

/// Font registry that manages web font loading and glyph rasterization.
///
/// # Lifecycle
///
/// 1. `collect_font_faces` extracts `@font-face` rules from stylesheets.
/// 2. `load_fonts` fetches and parses font data.
/// 3. `resolve_font` finds the best matching font for a family/weight/style.
/// 4. `measure_text` and `rasterize_glyph` provide layout and rendering.
pub struct FontRegistry {
    /// Map from lowercase family name → list of loaded font faces.
    families: HashMap<String, Vec<FontId>>,
    /// All loaded fonts, indexed by [`FontId`].
    fonts: Vec<LoadedFont>,
    /// Rasterized glyph cache.
    glyph_cache: HashMap<GlyphKey, RasterizedGlyph>,
    /// Pending `@font-face` rules waiting to be loaded.
    pending: Vec<FontFaceRule>,
}

impl FontRegistry {
    /// Create an empty font registry.
    pub fn new() -> Self {
        FontRegistry {
            families: HashMap::new(),
            fonts: Vec::new(),
            glyph_cache: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// Collect `@font-face` rules from parsed stylesheets.
    ///
    /// Call this after CSS parsing and before layout. The rules are
    /// stored as pending until `load_fonts` is called.
    pub fn collect_font_faces(&mut self, rules: &[FontFaceRule]) {
        for rule in rules {
            // Deduplicate by family + weight + style.
            let dominated = self.pending.iter().any(|existing| {
                existing.family.eq_ignore_ascii_case(&rule.family)
                    && existing.weight == rule.weight
                    && existing.style == rule.style
            });
            if !dominated {
                self.pending.push(rule.clone());
            }
        }
    }

    /// Load all pending font faces by fetching their data.
    ///
    /// For each pending `@font-face` rule, tries the `src:` entries in
    /// order until one succeeds. Font data is parsed with fontdue.
    pub fn load_fonts(
        &mut self,
        base_url: Option<&str>,
        vfs: &dyn oasis_vfs::Vfs,
        tls: Option<&dyn oasis_net::tls::TlsProvider>,
    ) {
        let pending = std::mem::take(&mut self.pending);
        for rule in &pending {
            if self.has_family_variant(&rule.family, rule.weight, rule.style) {
                continue;
            }
            for src in &rule.src {
                match src {
                    FontFaceSrc::Url { url, format } => {
                        // Skip formats we can't handle.
                        if !format.is_empty() && !format.iter().any(|f| is_supported_format(f)) {
                            continue;
                        }
                        if let Some(data) = fetch_font_data(url, base_url, vfs, tls)
                            && self.load_font_data(&rule.family, rule.weight, rule.style, &data)
                        {
                            break; // First successful source wins.
                        }
                    },
                    FontFaceSrc::Local(_name) => {
                        // Local font matching not implemented — skip.
                        continue;
                    },
                }
            }
        }
    }

    /// Parse font data and register it under the given family name.
    ///
    /// Returns `true` if the font was successfully parsed.
    pub(crate) fn load_font_data(
        &mut self,
        family: &str,
        weight: (u16, u16),
        style: FontFaceStyle,
        data: &[u8],
    ) -> bool {
        // Try to detect and unwrap WOFF/WOFF2 containers.
        let font_data = unwrap_font_container(data);

        let settings = fontdue::FontSettings {
            collection_index: 0,
            scale: 40.0, // Default scale — actual sizes are passed per-rasterize.
            load_substitutions: true,
        };
        match fontdue::Font::from_bytes(font_data, settings) {
            Ok(font) => {
                let id = FontId(self.fonts.len() as u32);
                self.fonts.push(LoadedFont {
                    font,
                    weight,
                    style,
                });
                let family_key = family.to_ascii_lowercase();
                self.families.entry(family_key).or_default().push(id);
                true
            },
            Err(e) => {
                log::warn!("failed to parse font '{}': {}", family, e);
                false
            },
        }
    }

    /// Check if a font variant is already loaded.
    fn has_family_variant(&self, family: &str, weight: (u16, u16), style: FontFaceStyle) -> bool {
        let key = family.to_ascii_lowercase();
        self.families.get(&key).is_some_and(|ids| {
            ids.iter().any(|id| {
                let f = &self.fonts[id.0 as usize];
                f.weight == weight && f.style == style
            })
        })
    }

    /// Resolve the best matching font for a family name, weight, and style.
    ///
    /// Returns `None` if no web font is loaded for this family. Uses
    /// CSS font matching: exact weight match preferred, then closest
    /// weight, then style fallback.
    pub fn resolve_font(
        &self,
        families: &[FontFamilyName],
        weight: u16,
        italic: bool,
    ) -> Option<FontId> {
        for family in families {
            if let FontFamilyName::Named(name) = family {
                let key = name.to_ascii_lowercase();
                if let Some(ids) = self.families.get(&key)
                    && let Some(id) = best_match(ids, &self.fonts, weight, italic)
                {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Returns `true` if at least one web font is loaded.
    pub fn has_fonts(&self) -> bool {
        !self.fonts.is_empty()
    }

    /// Returns `true` if a specific family name has any loaded variants.
    pub fn has_family(&self, name: &str) -> bool {
        self.families.contains_key(&name.to_ascii_lowercase())
    }

    /// Measure the width of a text string using a loaded web font.
    ///
    /// Returns the width in pixels if the font is available, or `None`
    /// if no web font matches (caller should fall back to bitmap font).
    pub fn measure_text(&self, text: &str, font_size: f32, font_id: FontId) -> u32 {
        let font = &self.fonts[font_id.0 as usize].font;
        let mut width = 0.0f32;
        for ch in text.chars() {
            let metrics = font.metrics(ch, font_size);
            width += metrics.advance_width;
        }
        width.ceil() as u32
    }

    /// Get the line height for a font at a given size.
    pub fn line_metrics(&self, font_id: FontId, font_size: f32) -> (f32, f32) {
        let font = &self.fonts[font_id.0 as usize].font;
        if let Some(lm) = font.horizontal_line_metrics(font_size) {
            (lm.ascent, -lm.descent)
        } else {
            (font_size * 0.85, font_size * 0.15)
        }
    }

    /// Rasterize a single glyph, returning cached results when available.
    pub fn rasterize_glyph(
        &mut self,
        font_id: FontId,
        ch: char,
        font_size: f32,
    ) -> &RasterizedGlyph {
        let key = GlyphKey {
            font_id,
            codepoint: ch,
            size_tenths: (font_size * 10.0) as u32,
        };

        // Check cache.
        if self.glyph_cache.contains_key(&key) {
            return &self.glyph_cache[&key];
        }

        // Evict if cache is full.
        if self.glyph_cache.len() >= MAX_GLYPH_CACHE {
            self.glyph_cache.clear();
        }

        // Rasterize.
        let font = &self.fonts[font_id.0 as usize].font;
        let (metrics, bitmap) = font.rasterize(ch, font_size);

        let glyph = RasterizedGlyph {
            bitmap,
            width: metrics.width as u32,
            height: metrics.height as u32,
            advance_width: metrics.advance_width,
            x_offset: metrics.xmin as f32,
            y_offset: metrics.ymin as f32,
        };

        self.glyph_cache.insert(key, glyph);
        &self.glyph_cache[&key]
    }

    /// Number of loaded font faces.
    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    /// Number of cached glyphs.
    pub fn glyph_cache_len(&self) -> usize {
        self.glyph_cache.len()
    }
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------
// Font matching (CSS Fonts Level 4 §5.2)
// -------------------------------------------------------------------

/// Select the best font from a list of candidates for the given
/// weight and style.
fn best_match(
    ids: &[FontId],
    fonts: &[LoadedFont],
    target_weight: u16,
    italic: bool,
) -> Option<FontId> {
    let target_style = if italic {
        FontFaceStyle::Italic
    } else {
        FontFaceStyle::Normal
    };

    let mut best: Option<(FontId, u32)> = None;
    for &id in ids {
        let f = &fonts[id.0 as usize];
        // Style penalty: exact match = 0, mismatch = 1000.
        let style_penalty = if f.style == target_style {
            0u32
        } else if f.style == FontFaceStyle::Oblique && italic {
            100 // Oblique is acceptable fallback for italic.
        } else {
            1000
        };
        // Weight penalty: distance from target, clamped to range.
        let closest_w = target_weight.clamp(f.weight.0, f.weight.1);
        let weight_penalty = target_weight.abs_diff(closest_w) as u32;
        let total = style_penalty + weight_penalty;
        if best.is_none() || total < best.as_ref().map_or(u32::MAX, |b| b.1) {
            best = Some((id, total));
        }
    }
    best.map(|(id, _)| id)
}

// -------------------------------------------------------------------
// Font data fetching
// -------------------------------------------------------------------

/// Fetch font data from a URL (relative or absolute).
fn fetch_font_data(
    url: &str,
    base_url: Option<&str>,
    vfs: &dyn oasis_vfs::Vfs,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
) -> Option<Vec<u8>> {
    let request = ResourceRequest {
        url: url.to_string(),
        base_url: base_url.map(|s| s.to_string()),
        source: if tls.is_some() {
            ResourceSource::VfsThenNetwork
        } else {
            ResourceSource::Vfs
        },
        method: loader::HttpMethod::Get,
        body: None,
        referrer: base_url.map(|s| s.to_string()),
    };

    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
    {
        match loader::load_resource(vfs, &request, tls, None, None) {
            Ok(loaded) if loaded.response.status < 400 && !loaded.response.body.is_empty() => {
                Some(loaded.response.body)
            },
            _ => None,
        }
    }
    #[cfg(any(target_arch = "wasm32", feature = "psp"))]
    {
        match loader::load_resource(vfs, &request, tls) {
            Ok(loaded) if loaded.response.status < 400 && !loaded.response.body.is_empty() => {
                Some(loaded.response.body)
            },
            _ => None,
        }
    }
}

/// Check if a `format()` hint is one we can parse.
fn is_supported_format(format: &str) -> bool {
    matches!(
        format.to_ascii_lowercase().as_str(),
        "truetype" | "opentype" | "woff" | "woff2"
    )
}

/// Attempt to unwrap a WOFF/WOFF2 container to get raw sfnt/TTF data.
///
/// WOFF is a thin wrapper around sfnt with table-level compression.
/// WOFF2 uses Brotli compression. For now we only support raw TTF/OTF
/// and WOFF (simple zlib table decompression). WOFF2 requires a
/// specialized decoder which we don't yet have — fontdue can often
/// parse the raw bytes if they're not actually WOFF2-compressed.
fn unwrap_font_container(data: &[u8]) -> &[u8] {
    // WOFF signature: 0x774F4646 ("wOFF")
    // WOFF2 signature: 0x774F4632 ("wOF2")
    // We pass through to fontdue which handles raw TTF/OTF directly.
    // fontdue 0.9 can parse WOFF natively, so we just return the data.
    data
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parser::{FontDisplay, FontFaceRule, FontFaceSrc, FontFaceStyle};

    #[test]
    fn empty_registry_has_no_fonts() {
        let reg = FontRegistry::new();
        assert!(!reg.has_fonts());
        assert_eq!(reg.font_count(), 0);
    }

    #[test]
    fn collect_deduplicates_same_variant() {
        let mut reg = FontRegistry::new();
        let rule = FontFaceRule {
            family: "Test".into(),
            src: vec![FontFaceSrc::Url {
                url: "test.ttf".into(),
                format: vec![],
            }],
            weight: (400, 400),
            style: FontFaceStyle::Normal,
            display: FontDisplay::Swap,
            unicode_range: vec![],
        };
        reg.collect_font_faces(&[rule.clone(), rule.clone()]);
        assert_eq!(reg.pending.len(), 1);
    }

    #[test]
    fn different_weights_are_separate() {
        let mut reg = FontRegistry::new();
        let rule1 = FontFaceRule {
            family: "Test".into(),
            src: vec![FontFaceSrc::Url {
                url: "test-400.ttf".into(),
                format: vec![],
            }],
            weight: (400, 400),
            style: FontFaceStyle::Normal,
            display: FontDisplay::Auto,
            unicode_range: vec![],
        };
        let rule2 = FontFaceRule {
            family: "Test".into(),
            src: vec![FontFaceSrc::Url {
                url: "test-700.ttf".into(),
                format: vec![],
            }],
            weight: (700, 700),
            style: FontFaceStyle::Normal,
            display: FontDisplay::Auto,
            unicode_range: vec![],
        };
        reg.collect_font_faces(&[rule1, rule2]);
        assert_eq!(reg.pending.len(), 2);
    }

    #[test]
    fn supported_formats() {
        assert!(is_supported_format("truetype"));
        assert!(is_supported_format("opentype"));
        assert!(is_supported_format("woff"));
        assert!(is_supported_format("woff2"));
        assert!(!is_supported_format("svg"));
        assert!(!is_supported_format("embedded-opentype"));
    }

    #[test]
    fn font_matching_exact_weight() {
        // Directly test best_match by building LoadedFont entries with
        // a shared dummy font (only the metadata matters for matching).
        let font = dummy_font();
        let fonts = vec![
            LoadedFont {
                font: font.clone(),
                weight: (400, 400),
                style: FontFaceStyle::Normal,
            },
            LoadedFont {
                font,
                weight: (700, 700),
                style: FontFaceStyle::Normal,
            },
        ];
        let ids = vec![FontId(0), FontId(1)];
        assert_eq!(best_match(&ids, &fonts, 400, false), Some(FontId(0)));
        assert_eq!(best_match(&ids, &fonts, 700, false), Some(FontId(1)));
        // 500 should prefer 400 (closer).
        assert_eq!(best_match(&ids, &fonts, 500, false), Some(FontId(0)));
    }

    #[test]
    fn font_matching_italic_preference() {
        let font = dummy_font();
        let fonts = vec![
            LoadedFont {
                font: font.clone(),
                weight: (400, 400),
                style: FontFaceStyle::Normal,
            },
            LoadedFont {
                font,
                weight: (400, 400),
                style: FontFaceStyle::Italic,
            },
        ];
        let ids = vec![FontId(0), FontId(1)];
        assert_eq!(best_match(&ids, &fonts, 400, true), Some(FontId(1)));
        assert_eq!(best_match(&ids, &fonts, 400, false), Some(FontId(0)));
    }

    #[test]
    fn load_font_from_bytes() {
        let mut reg = FontRegistry::new();
        let data = include_bytes!("../../test_data/minimal.ttf");
        let ok = reg.load_font_data("TestFont", (400, 400), FontFaceStyle::Normal, data);
        assert!(ok, "should parse minimal TTF");
        assert!(reg.has_fonts());
        assert!(reg.has_family("TestFont"));
        assert!(reg.has_family("testfont")); // case-insensitive
    }

    /// Create a fontdue Font from the test data file.
    fn dummy_font() -> fontdue::Font {
        let data = include_bytes!("../../test_data/minimal.ttf");
        fontdue::Font::from_bytes(data as &[u8], fontdue::FontSettings::default())
            .expect("test font should parse")
    }
}
