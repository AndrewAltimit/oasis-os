//! The one definition of how the built-in bitmap font is rasterized.
//!
//! Every desktop-class backend draws the 8x8 bitmap font scaled to the
//! requested pixel size (not to the nearest multiple of 8), with the same
//! faux-bold (1 px double strike) and faux-italic (row shear), advances
//! each glyph by [`glyph_advance_scaled`] and reports the same line
//! metrics. SDL uploads [`glyph_mask`] as a texture, the software
//! rasterizer fills it as spans; both therefore produce identical pixels,
//! and text measured with `measure_text` is exactly as wide as drawn.
//!
//! [`glyph_advance_scaled`]: oasis_types::bitmap_font::glyph_advance_scaled

use oasis_types::bitmap_font;
pub use oasis_types::bitmap_font::{bitmap_ascent, bitmap_line_height};

/// Width and height of the cell [`glyph_mask`] rasterizes `ch` into.
pub fn glyph_cell(ch: char, font_size: u16, bold: bool, italic: bool) -> (u32, u32) {
    let fs = font_size.max(1) as i32;
    let advance = bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
    let bold_extra = i32::from(bold);
    let italic_extra = if italic { fs / 32 + 1 } else { 0 };
    let gw = (advance + bold_extra + italic_extra).max(1) as u32;
    (gw, font_size.max(1) as u32)
}

/// Rasterize `ch` at `font_size` into `mask` (row-major coverage of the
/// [`glyph_cell`], `true` = ink). Returns the cell size.
///
/// Each of the 8x8 source bits covers the scaled rectangle
/// `[col*fs/8, (col+1)*fs/8) x [row*fs/8, (row+1)*fs/8)` (at least one
/// pixel), shifted left by the glyph's left padding; italic shears row
/// `r` right by `(7 - r) * fs / 32`; bold also sets the pixel to the
/// right of every ink pixel. The UI triangles (U+25B2 / U+25BC) are drawn
/// as smooth shapes at the target size instead of scaled bits.
pub fn glyph_mask(
    ch: char,
    font_size: u16,
    bold: bool,
    italic: bool,
    mask: &mut Vec<bool>,
) -> (u32, u32) {
    let (gw, gh) = glyph_cell(ch, font_size, bold, italic);
    mask.clear();
    mask.resize((gw * gh) as usize, false);
    let (w, h) = (gw as i32, gh as i32);
    let mut set = |x: i32, y: i32| {
        if (0..w).contains(&x) && (0..h).contains(&y) {
            mask[(y * w + x) as usize] = true;
        }
    };

    if bitmap_font::is_smooth_triangle(ch) {
        for y in 0..h {
            let Some((x0, x1)) = bitmap_font::smooth_triangle_span(ch, y, w, h) else {
                continue;
            };
            for x in x0..=x1 {
                set(x, y);
                if bold {
                    set(x + 1, y);
                }
            }
        }
        return (gw, gh);
    }

    let fs = font_size.max(1) as i32;
    let bits_of = bitmap_font::glyph(ch);
    let left_pad = bitmap_font::glyph_metrics(ch).0 as i32;
    for row in 0..8i32 {
        let bits = bits_of[row as usize];
        if bits == 0 {
            continue;
        }
        let oy0 = row * fs / 8;
        let oy1 = ((row + 1) * fs / 8).max(oy0 + 1);
        let italic_off = if italic { (7 - row) * fs / 32 } else { 0 };
        for col in 0..8i32 {
            if bits & (0x80 >> col) == 0 {
                continue;
            }
            let src_col = col - left_pad;
            let ox0 = src_col * fs / 8;
            let ox1 = ((src_col + 1) * fs / 8).max(ox0 + 1);
            for py in oy0..oy1 {
                for px in ox0..ox1 {
                    set(px + italic_off, py);
                    if bold {
                        set(px + italic_off + 1, py);
                    }
                }
            }
        }
    }
    (gw, gh)
}

/// Call `f(y, x0, x1)` for every horizontal run of ink in a mask of width
/// `w` (half-open, mask coordinates).
pub fn mask_runs(mask: &[bool], w: u32, mut f: impl FnMut(i32, i32, i32)) {
    if w == 0 {
        return;
    }
    for (y, row) in mask.chunks_exact(w as usize).enumerate() {
        let mut x = 0usize;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            f(y as i32, start as i32, x as i32);
        }
    }
}

/// One horizontal ink run of a glyph cell: row `y`, columns `x0..x1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GlyphRun {
    pub(crate) y: i32,
    pub(crate) x0: i32,
    pub(crate) x1: i32,
}

/// Text styles a [`GlyphRunCache`] keeps at once (UI text uses a handful
/// of sizes; a new style past this evicts the oldest).
const RUN_CACHE_STYLES: usize = 8;

/// Largest font size whose glyph runs are cached (bigger glyphs are rare
/// and would make each entry large).
const RUN_CACHE_MAX_SIZE: u16 = 64;

/// Not-yet-rasterized marker in [`StyleRuns::spans`].
const UNBUILT: (u32, u32) = (u32::MAX, u32::MAX);

/// The [`mask_runs`] of every ASCII glyph drawn so far in one style.
struct StyleRuns {
    key: (u16, bool, bool),
    /// `runs[start..end]` per ASCII code ([`UNBUILT`] until first drawn).
    spans: [(u32, u32); 128],
    runs: Vec<GlyphRun>,
}

/// Cache of bitmap-glyph ink runs, so drawing the same characters again
/// skips [`glyph_mask`] and the run scan. The runs are exactly those of
/// [`mask_runs`] over [`glyph_mask`], so cached text draws the same pixels.
/// Only ASCII up to [`RUN_CACHE_MAX_SIZE`] px is cached.
#[derive(Default)]
pub(crate) struct GlyphRunCache {
    styles: Vec<StyleRuns>,
    /// Next slot to evict once `styles` is full (round robin).
    next_evict: usize,
    mask: Vec<bool>,
}

impl GlyphRunCache {
    /// The ink runs of `ch` in the given style, or `None` when the glyph
    /// is not cacheable (non-ASCII or too large).
    pub(crate) fn runs(
        &mut self,
        ch: char,
        font_size: u16,
        bold: bool,
        italic: bool,
    ) -> Option<&[GlyphRun]> {
        if !ch.is_ascii() || font_size > RUN_CACHE_MAX_SIZE {
            return None;
        }
        let key = (font_size, bold, italic);
        let si = match self.styles.iter().position(|s| s.key == key) {
            Some(i) => i,
            None => {
                let fresh = StyleRuns {
                    key,
                    spans: [UNBUILT; 128],
                    runs: Vec::new(),
                };
                if self.styles.len() < RUN_CACHE_STYLES {
                    self.styles.push(fresh);
                    self.styles.len() - 1
                } else {
                    let i = self.next_evict;
                    self.next_evict = (i + 1) % RUN_CACHE_STYLES;
                    self.styles[i] = fresh;
                    i
                }
            },
        };
        let style = &mut self.styles[si];
        let code = ch as usize;
        if style.spans[code] == UNBUILT {
            let (gw, _) = glyph_mask(ch, font_size, bold, italic, &mut self.mask);
            let start = style.runs.len() as u32;
            mask_runs(&self.mask, gw, |y, x0, x1| {
                style.runs.push(GlyphRun { y, x0, x1 });
            });
            style.spans[code] = (start, style.runs.len() as u32);
        }
        let (start, end) = style.spans[code];
        Some(&style.runs[start as usize..end as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_cache_matches_mask_runs() {
        let mut cache = GlyphRunCache::default();
        let mut mask = Vec::new();
        // More styles than the cache holds, twice, so eviction is hit.
        for _ in 0..2 {
            for fs in [6u16, 8, 10, 12, 13, 16, 20, 24, 32, 64] {
                for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                    for ch in [' ', 'A', 'g', 'W', '~', '\u{7f}'] {
                        let (gw, _) = glyph_mask(ch, fs, bold, italic, &mut mask);
                        let mut want = Vec::new();
                        mask_runs(&mask, gw, |y, x0, x1| want.push(GlyphRun { y, x0, x1 }));
                        let got = cache.runs(ch, fs, bold, italic).unwrap();
                        assert_eq!(got, &want[..], "{ch:?} {fs}px b={bold} i={italic}");
                    }
                }
            }
        }
        assert!(cache.runs('\u{25B2}', 8, false, false).is_none());
        assert!(cache.runs('A', 65, false, false).is_none());
    }

    #[test]
    fn cell_matches_advance() {
        for fs in [6u16, 8, 10, 12, 16, 24] {
            let (w, h) = glyph_cell('A', fs, false, false);
            assert_eq!(w, bitmap_font::glyph_advance_scaled('A', fs).max(1));
            assert_eq!(h, fs as u32);
        }
    }

    #[test]
    fn fractional_sizes_scale_to_the_requested_height() {
        // At 12 px the glyph is 12 px tall, not an 8 px glyph at scale 1.
        let mut m = Vec::new();
        let (w, h) = glyph_mask('I', 12, false, false, &mut m);
        let mut rows = vec![false; h as usize];
        mask_runs(&m, w, |y, _, _| rows[y as usize] = true);
        let inked = rows.iter().filter(|&&r| r).count();
        assert!(inked > 8, "only {inked} rows inked at 12 px");
    }

    #[test]
    fn bold_widens_every_run() {
        let (mut plain, mut bold) = (Vec::new(), Vec::new());
        let (w, _) = glyph_mask('l', 16, false, false, &mut plain);
        let (wb, _) = glyph_mask('l', 16, true, false, &mut bold);
        assert_eq!(wb, w + 1);
        let count = |m: &[bool]| m.iter().filter(|&&b| b).count();
        assert!(count(&bold) > count(&plain));
    }

    #[test]
    fn metrics_follow_the_shared_formula() {
        assert_eq!(bitmap_line_height(8), 10);
        assert_eq!(bitmap_line_height(12), 15);
        assert_eq!(bitmap_line_height(6), 10);
        assert_eq!(bitmap_ascent(16), 14);
        assert_eq!(bitmap_ascent(4), 7);
    }
}
