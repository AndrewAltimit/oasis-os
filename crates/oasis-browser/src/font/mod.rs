//! Web font loading and rendering.
//!
//! This module implements `@font-face` support: parsing font data,
//! caching rasterized glyphs, and providing font-aware text measurement
//! for the layout engine.
//!
//! Feature-gated behind `web-fonts` (depends on `fontdue`).

#[cfg(feature = "web-fonts")]
mod registry;

#[cfg(feature = "web-fonts")]
pub use registry::{FontId, FontRegistry, RasterizedGlyph};

#[cfg(feature = "web-fonts")]
mod measurer;
#[cfg(feature = "web-fonts")]
mod render;

#[cfg(feature = "web-fonts")]
pub use measurer::FontAwareTextMeasurer;
#[cfg(feature = "web-fonts")]
pub use render::{GlyphTextureCache, render_web_font_text};

/// Adapter implementing `WebFontRenderer` by delegating to the
/// font registry and glyph texture cache.
#[cfg(feature = "web-fonts")]
pub struct BrowserWebFontRenderer<'a> {
    pub registry: &'a std::cell::RefCell<FontRegistry>,
    pub tex_cache: &'a mut GlyphTextureCache,
}

#[cfg(feature = "web-fonts")]
impl crate::paint::display_list::WebFontRenderer for BrowserWebFontRenderer<'_> {
    fn render(
        &mut self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: oasis_types::backend::Color,
        font_id: u32,
    ) -> oasis_types::error::Result<()> {
        render_web_font_text(
            backend,
            self.registry,
            self.tex_cache,
            text,
            x,
            y,
            font_size,
            color,
            FontId::from_raw(font_id),
        )
    }
}
