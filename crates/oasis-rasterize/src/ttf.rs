//! TTF/OTF glyph rasterization for per-skin fonts (feature `ttf`).
//!
//! Wraps `fontdue` behind a small integer-metric API so backends can swap
//! their bitmap-font text path for a skin-provided TrueType font without
//! measurement drift: [`TtfFont::advance`] rounds each glyph advance to whole
//! pixels once, and both `measure_text` and the draw loop accumulate those
//! same integers, so layout and rendering always agree.
//!
//! Glyphs are rasterized as coverage (alpha-only) bitmaps; callers tint at
//! blit time (see [`TtfGlyph::to_rgba`]), matching the color-independent
//! glyph cache design used by the SDL backend.

use oasis_types::backend::Color;
use oasis_types::error::{OasisError, Result};

/// A parsed TrueType/OpenType font ready for glyph rasterization.
pub struct TtfFont {
    font: fontdue::Font,
}

/// A rasterized glyph: coverage bitmap plus integer placement metrics.
///
/// `xmin`/`ymin` are fontdue's glyph bounding-box offsets: `xmin` is the
/// horizontal offset from the pen position to the left edge, `ymin` the
/// vertical offset from the baseline to the *bottom* edge (positive = above
/// baseline).
pub struct TtfGlyph {
    /// Alpha coverage, row-major top-to-bottom (`width * height` bytes).
    pub coverage: Vec<u8>,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Left side bearing in pixels.
    pub xmin: i32,
    /// Offset from baseline to bitmap bottom (positive = above baseline).
    pub ymin: i32,
    /// Horizontal pen advance in whole pixels (already rounded).
    pub advance: i32,
}

impl TtfGlyph {
    /// Expand the coverage bitmap to RGBA using `color` for RGB and
    /// `coverage * color.a / 255` for alpha.
    pub fn to_rgba(&self, color: Color) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.coverage.len() * 4);
        for &cov in &self.coverage {
            rgba.push(color.r);
            rgba.push(color.g);
            rgba.push(color.b);
            rgba.push(((cov as u16 * color.a as u16) / 255) as u8);
        }
        rgba
    }
}

impl TtfFont {
    /// Parse raw TTF/OTF bytes. WOFF/WOFF2 containers are not supported.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let settings = fontdue::FontSettings {
            collection_index: 0,
            scale: 40.0, // Default only -- actual sizes are passed per call.
            load_substitutions: true,
        };
        let font = fontdue::Font::from_bytes(bytes, settings)
            .map_err(|e| OasisError::Config(format!("font: {e}").into()))?;
        Ok(Self { font })
    }

    /// Whether the font has a real glyph (not `.notdef`) for `ch`.
    ///
    /// Backends use this to fall back to their bitmap font for symbols a
    /// skin font doesn't cover (e.g. the ▲/▼ UI triangles).
    pub fn has_glyph(&self, ch: char) -> bool {
        self.font.lookup_glyph_index(ch) != 0
    }

    /// Horizontal advance for `ch` at `px` pixels, rounded to whole pixels.
    pub fn advance(&self, ch: char, px: f32) -> i32 {
        self.font.metrics(ch, px).advance_width.round().max(0.0) as i32
    }

    /// Width of `text` at `px` pixels: the sum of whole-pixel advances.
    ///
    /// Only covers glyphs the font actually has; callers mixing in a bitmap
    /// fallback should accumulate per character via [`Self::advance`] and
    /// [`Self::has_glyph`] instead.
    pub fn measure_text(&self, text: &str, px: f32) -> u32 {
        text.chars()
            .map(|ch| self.advance(ch, px).max(0) as u32)
            .sum()
    }

    /// Baseline offset from the top of the line box, in pixels.
    pub fn ascent(&self, px: f32) -> i32 {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent.ceil() as i32)
            .unwrap_or_else(|| (px * 0.85).ceil() as i32)
    }

    /// Line box height in pixels.
    pub fn line_height(&self, px: f32) -> u32 {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| m.new_line_size.ceil().max(1.0) as u32)
            .unwrap_or_else(|| (px * 1.2).ceil() as u32)
    }

    /// Rasterize `ch` at `px` pixels to a coverage bitmap.
    pub fn rasterize(&self, ch: char, px: f32) -> TtfGlyph {
        let (metrics, coverage) = self.font.rasterize(ch, px);
        TtfGlyph {
            coverage,
            width: metrics.width as u32,
            height: metrics.height as u32,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            advance: metrics.advance_width.round().max(0.0) as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT: &[u8] = include_bytes!("../../oasis-browser/test_data/minimal.ttf");

    #[test]
    fn parse_valid_font() {
        assert!(TtfFont::from_bytes(TEST_FONT).is_ok());
    }

    #[test]
    fn parse_garbage_fails() {
        assert!(TtfFont::from_bytes(b"definitely not a font").is_err());
        assert!(TtfFont::from_bytes(&[]).is_err());
    }

    #[test]
    fn measure_empty_is_zero() {
        let font = TtfFont::from_bytes(TEST_FONT).expect("test font");
        assert_eq!(font.measure_text("", 16.0), 0);
    }

    #[test]
    fn measure_matches_summed_advances() {
        let font = TtfFont::from_bytes(TEST_FONT).expect("test font");
        // Whatever ASCII glyphs the fixture has, the string measurement must
        // equal the sum of per-char integer advances (draw/measure agreement).
        let text: String = (' '..='~').filter(|&c| font.has_glyph(c)).collect();
        let summed: u32 = text
            .chars()
            .map(|ch| font.advance(ch, 16.0).max(0) as u32)
            .sum();
        assert_eq!(font.measure_text(&text, 16.0), summed);
    }

    #[test]
    fn rasterize_advance_matches_metrics_advance() {
        let font = TtfFont::from_bytes(TEST_FONT).expect("test font");
        for ch in (' '..='~').filter(|&c| font.has_glyph(c)) {
            let glyph = font.rasterize(ch, 16.0);
            assert_eq!(glyph.advance, font.advance(ch, 16.0), "char {ch:?}");
            assert_eq!(
                glyph.coverage.len(),
                (glyph.width * glyph.height) as usize,
                "char {ch:?}"
            );
        }
    }

    #[test]
    fn line_metrics_are_positive() {
        let font = TtfFont::from_bytes(TEST_FONT).expect("test font");
        assert!(font.ascent(16.0) > 0);
        assert!(font.line_height(16.0) > 0);
    }

    #[test]
    fn to_rgba_expands_coverage() {
        let glyph = TtfGlyph {
            coverage: vec![0, 128, 255],
            width: 3,
            height: 1,
            xmin: 0,
            ymin: 0,
            advance: 4,
        };
        let rgba = glyph.to_rgba(Color::rgba(10, 20, 30, 255));
        assert_eq!(rgba.len(), 12);
        assert_eq!(&rgba[0..4], &[10, 20, 30, 0]);
        assert_eq!(&rgba[4..8], &[10, 20, 30, 128]);
        assert_eq!(&rgba[8..12], &[10, 20, 30, 255]);
    }
}
