//! Core rendering trait that every backend must implement.

use super::{Color, TextureId};
use crate::error::Result;

/// Core rendering methods that every backend must implement.
///
/// These 13 methods are the minimum surface for a rendering backend.
/// Test mocks and null backends implement this trait plus empty impls
/// for the nine extension traits (`SdiShapes`, `SdiGradients`,
/// `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`, `SdiVector`,
/// `SdiBatch`, `SdiRenderTarget`) -- one
/// [`impl_sdi_defaults!`](crate::impl_sdi_defaults) line writes them --
/// to satisfy `SdiBackend` via its blanket impl.
///
/// # Drawing-state contract
///
/// Every backend, and every wrapper that forwards to one, behaves the same
/// way here. `oasis_test_backend::conformance` pins it on SDL and the
/// software framebuffer, and ADR-003 explains the choices.
///
/// - **Coordinates.** Draw calls take coordinates relative to the current
///   translation ([`SdiClipTransform::push_translate`]) on the currently
///   bound surface: the framebuffer, or the render target bound with
///   [`SdiRenderTarget::bind_render_target`].
/// - **[`set_clip_rect`](Self::set_clip_rect) is absolute and replaces.**
///   The rectangle is in surface pixels and is *not* translated. It
///   replaces whatever clip is active instead of intersecting with it. A
///   zero-area rectangle rejects every draw. Callers that need nesting
///   either intersect themselves (the browser display list does) or use
///   [`SdiClipTransform::push_clip_rect`].
/// - **[`reset_clip_rect`](Self::reset_clip_rect)** removes the clip. It is
///   always safe to call, including when no clip is set.
/// - **[`clear`](Self::clear) ignores the clip and does not blend.** It
///   writes `color`, alpha included, to every pixel of the bound surface,
///   so `clear(Color::TRANSPARENT)` on a render target really makes it
///   transparent.
///
/// [`SdiClipTransform::push_translate`]: crate::backend::SdiClipTransform::push_translate
/// [`SdiClipTransform::push_clip_rect`]: crate::backend::SdiClipTransform::push_clip_rect
/// [`SdiRenderTarget::bind_render_target`]: crate::backend::SdiRenderTarget::bind_render_target
pub trait SdiCore {
    /// Initialize the rendering subsystem.
    fn init(&mut self, width: u32, height: u32) -> Result<()>;

    /// Overwrite every pixel of the bound surface with `color`, ignoring
    /// the clip and without blending (see the drawing-state contract).
    fn clear(&mut self, color: Color) -> Result<()>;

    /// Blit a texture at the given position and size.
    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()>;

    /// Draw a filled rectangle (used when no texture is assigned).
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()>;

    /// Draw text at the given position. The backend chooses its available font.
    /// `font_size` is a hint in pixels; backends may approximate.
    fn draw_text(&mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color)
    -> Result<()>;

    /// Present the current frame to the display.
    fn swap_buffers(&mut self) -> Result<()>;

    /// Load raw RGBA pixel data as a texture. Returns a handle for later blit.
    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId>;

    /// Destroy a previously loaded texture.
    fn destroy_texture(&mut self, tex: TextureId) -> Result<()>;

    /// Replace the clip with an absolute, untranslated rectangle in surface
    /// pixels (see the drawing-state contract).
    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;

    /// Remove the clip, so the whole bound surface is drawable again.
    fn reset_clip_rect(&mut self) -> Result<()>;

    /// Measure the width of a text string at the given font size.
    /// Returns width in pixels. Used by inline layout for line breaking.
    fn measure_text(&self, text: &str, font_size: u16) -> u32;

    /// Read the current framebuffer as RGBA pixel data.
    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>>;

    /// Shut down the rendering subsystem and release resources.
    fn shutdown(&mut self) -> Result<()>;
}
