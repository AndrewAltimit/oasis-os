//! Core rendering trait that every backend must implement.

use super::{Color, TextureId};
use crate::error::Result;

/// Core rendering methods that every backend must implement.
///
/// These 13 methods are the minimum surface for a rendering backend.
/// Test mocks and null backends only need to implement this trait,
/// then add an empty `impl SdiBackend for T {}` to pick up all
/// extended methods with their default implementations.
pub trait SdiCore {
    /// Initialize the rendering subsystem.
    fn init(&mut self, width: u32, height: u32) -> Result<()>;

    /// Clear the screen to a solid color.
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

    /// Set the clipping rectangle (for window manager content clipping).
    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;

    /// Reset clipping to the full screen.
    fn reset_clip_rect(&mut self) -> Result<()>;

    /// Measure the width of a text string at the given font size.
    /// Returns width in pixels. Used by inline layout for line breaking.
    fn measure_text(&self, text: &str, font_size: u16) -> u32;

    /// Read the current framebuffer as RGBA pixel data.
    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>>;

    /// Shut down the rendering subsystem and release resources.
    fn shutdown(&mut self) -> Result<()>;
}
