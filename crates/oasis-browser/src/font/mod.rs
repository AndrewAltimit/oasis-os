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
