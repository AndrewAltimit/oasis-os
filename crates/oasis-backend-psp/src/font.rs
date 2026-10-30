//! Font rendering: system TrueType fonts (via `psp::font`) with 8x8 bitmap
//! fallback.
//!
//! The bitmap fallback is the shared 8x8 table in
//! `oasis_core::bitmap_font` (`oasis-types`); `render.rs` builds its glyph
//! atlas from it.
//!
//! `SystemFont` wraps the PSP's built-in TrueType font library, providing
//! anti-aliased rendering via a VRAM glyph atlas. Falls back to bitmap if
//! system fonts are unavailable (e.g. PPSSPP emulator).

use std::pin::Pin;

use psp::font::{Font, FontLib, FontRenderer};
use psp::sys::{SceFontFamilyCode, SceFontLanguageCode, SceFontStyleCode};

/// System font wrapper (FontLib + Font + FontRenderer with VRAM atlas).
///
/// Manages the self-referential FontRenderer<'a> -> &'a Font relationship
/// via a `Pin<Box<Font>>` that guarantees the heap address is stable.
///
/// Drop order matters: `renderer` is declared first and therefore dropped
/// first, before `_font` and `_fontlib` (Rust drops fields in declaration
/// order).
pub struct SystemFont {
    // Drop order: renderer first, then font, then fontlib.
    renderer: Option<FontRenderer<'static>>,
    _font: Pin<Box<Font>>,
    _fontlib: FontLib,
}

impl SystemFont {
    /// Try to initialize system fonts. Returns `None` if unavailable.
    pub fn try_init(atlas_vram: *mut u8) -> Option<Self> {
        let fontlib = FontLib::new(1).ok()?;
        let font = fontlib
            .find_optimum(
                SceFontFamilyCode::SansSerif,
                SceFontStyleCode::Regular,
                SceFontLanguageCode::Latin,
            )
            .ok()?;
        let font = Box::pin(font);

        // SAFETY: The Font is behind Pin<Box<_>>, guaranteeing its heap
        // address is stable for the lifetime of `_font`. The renderer is
        // dropped before _font (struct field declaration order), so the
        // reference remains valid for the entire lifetime of `renderer`.
        let font_ref: &'static Font = unsafe { &*(font.as_ref().get_ref() as *const Font) };
        let renderer = FontRenderer::new(font_ref, atlas_vram, 12.0);

        Some(Self {
            renderer: Some(renderer),
            _font: font,
            _fontlib: fontlib,
        })
    }

    /// Queue text for drawing. Call `flush()` after all text is queued.
    pub fn draw_text(&mut self, x: f32, y: f32, color_abgr: u32, text: &str) {
        if let Some(r) = &mut self.renderer {
            r.draw_text(x, y, color_abgr, text);
        }
    }

    /// Measure the width of a text string in pixels.
    pub fn measure_text(&self, text: &str) -> f32 {
        self.renderer.as_ref().map_or(0.0, |r| r.measure_text(text))
    }

    /// Get the line height in pixels.
    pub fn line_height(&self) -> f32 {
        self.renderer.as_ref().map_or(12.0, |r| r.line_height())
    }

    /// Submit all queued glyph sprites to the GU.
    ///
    /// # Safety
    /// Must be called within an active GU display list.
    pub unsafe fn flush(&mut self) {
        if let Some(r) = &mut self.renderer {
            // SAFETY: Caller guarantees we're in an active GU display list.
            unsafe { r.flush() };
        }
    }
}
