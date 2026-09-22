//! `SdiCore` implementation for the SDL3 backend.
//!
//! Contains the 13 required rendering methods: init, clear, blit,
//! fill_rect, draw_text, swap_buffers, load_texture, destroy_texture,
//! set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown.

use sdl3::pixels::PixelFormat;
use sdl3::rect::Rect;
use sdl3::render::Texture;

use oasis_core::backend::{
    BackendErrExt, Color, SdiCore, SdiText, TextureId, texture_not_found, validate_rgba_data,
};
use oasis_core::error::Result;

use super::{SdlBackend, frect};

impl SdiCore for SdlBackend {
    fn init(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        // Goes through set_color so the cached draw-color state stays
        // coherent (clear itself ignores the blend mode).
        self.set_color(color);
        self.canvas.clear();
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        crate::blitting::ensure_texture_mod(
            &mut self.texture_mods,
            tex.0,
            texture,
            crate::blitting::NEUTRAL_MOD,
        );
        self.canvas
            .copy(texture, None, frect(tx, ty, w, h))
            .backend_err()?;
        Ok(())
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> Result<()> {
        self.draw_text_styled(text, x, y, font_size, color, false, false)
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.set_color(color);
        self.canvas.fill_rect(frect(tx, ty, w, h)).backend_err()?;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.canvas.present();
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;

        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormat::ABGR8888, width, height)
            .backend_err()?;

        texture
            .with_lock(None, |buffer: &mut [u8], pitch: usize| {
                copy_rows_to_pitch(buffer, pitch, rgba_data, width);
            })
            .backend_err()?;

        texture.set_blend_mode(sdl3::render::BlendMode::Blend);

        // SAFETY: The texture borrows from self.texture_creator which lives in the
        // same struct. The explicit `Drop` impl clears all textures before
        // texture_creator is dropped. The erased lifetime is therefore always valid.
        let texture: Texture<'static> = unsafe { std::mem::transmute(texture) };

        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, texture);
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.textures.remove(&tex.0);
        self.texture_mods.remove(&tex.0);
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // A degenerate clip (w==0 or h==0) means "intersection collapsed
        // to nothing". `sdl3::rect::Rect::new` would silently clamp the
        // zero to one, leaving a 1-pixel slot that any subsequent
        // `fill_rect` / glyph blit can sneak a sliver of pixels through —
        // that is exactly the source of the dotted-underline leak below
        // an old.reddit.com browser window. Use `ClippingRect::Zero` so
        // SDL3 actually rejects every draw under this clip.
        if w == 0 || h == 0 {
            self.canvas.set_clip_rect(sdl3::render::ClippingRect::Zero);
        } else {
            self.canvas.set_clip_rect(Rect::new(x, y, w, h));
        }
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.canvas.set_clip_rect(None);
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        // Mirror the draw path's per-character font choice (TTF glyph when
        // the skin font has one, bitmap fallback otherwise) with the same
        // whole-pixel advances, so measurement and rendering always agree.
        if let Some(ttf) = &self.ttf_font {
            let px = font_size.max(1) as f32;
            return text
                .chars()
                .map(|ch| {
                    if ttf.has_glyph(ch) {
                        ttf.advance(ch, px).max(0) as u32
                    } else {
                        oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size)
                    }
                })
                .sum();
        }
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let rect = Rect::new(x, y, w, h);
        // SDL returns the renderer's native format (commonly ARGB8888, i.e.
        // B,G,R,A bytes in memory); normalize to RGBA byte order, which is
        // what every caller (screenshots, MCP, tests) expects.
        let surface = self
            .canvas
            .read_pixels(rect)
            .backend_err()?
            .convert_format(PixelFormat::RGBA32)
            .backend_err()?;
        let pitch = surface.pitch() as usize;
        let height = surface.height() as usize;
        let width = surface.width() as usize;
        let bpp = 4usize; // RGBA
        // SAFETY: The surface was just created by read_pixels and is not
        // shared; we only read the pixel data before it goes out of scope.
        let data = unsafe { surface.without_lock() }.ok_or_else(|| {
            oasis_core::error::OasisError::Backend("cannot lock surface pixels".into())
        })?;
        // Copy pixel data row by row (pitch may differ from width * bpp).
        let mut pixels = Vec::with_capacity(width * height * bpp);
        for row in 0..height {
            let start = row * pitch;
            let end = start + width * bpp;
            if end <= data.len() {
                pixels.extend_from_slice(&data[start..end]);
            }
        }
        Ok(pixels)
    }

    fn shutdown(&mut self) -> Result<()> {
        log::info!("SDL3 backend shut down");
        Ok(())
    }
}

// -------------------------------------------------------------------
// Inherent texture helpers (not part of SdiCore)
// -------------------------------------------------------------------

impl SdlBackend {
    /// Update the pixels of an existing streaming texture in place.
    ///
    /// Reuses the texture created by `load_texture` instead of the
    /// destroy + create churn (GPU texture allocation, HashMap
    /// insert/remove, unbounded id growth) that per-frame callers like
    /// the shader wallpaper bridge would otherwise incur. The texture
    /// dimensions must match `width` x `height`; a mismatch returns an
    /// error so the caller can destroy and re-create at the new size.
    pub fn update_texture(
        &mut self,
        tex: TextureId,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> Result<()> {
        validate_rgba_data(width, height, rgba_data)?;

        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;

        let query = texture.query();
        if query.width != width || query.height != height {
            return Err(oasis_core::error::OasisError::Backend(
                format!(
                    "update_texture: size mismatch (texture is {}x{}, data is {width}x{height})",
                    query.width, query.height
                )
                .into(),
            ));
        }

        texture
            .with_lock(None, |buffer: &mut [u8], pitch: usize| {
                copy_rows_to_pitch(buffer, pitch, rgba_data, width);
            })
            .backend_err()?;

        Ok(())
    }

    /// Show or hide the host OS mouse pointer over the window.
    ///
    /// Skins that enable `features.software_cursor` draw their own themed
    /// cursor, so the host pointer is hidden to avoid a double cursor.
    pub fn set_host_cursor_visible(&mut self, visible: bool) {
        // SAFETY: SDL_ShowCursor/SDL_HideCursor are global SDL calls with
        // no preconditions beyond SDL_Init, which ran in `new()`.
        unsafe {
            if visible {
                sdl3::sys::mouse::SDL_ShowCursor();
            } else {
                sdl3::sys::mouse::SDL_HideCursor();
            }
        }
    }
}

/// Copy tightly packed RGBA rows (`width * 4` bytes each) into a locked
/// texture buffer whose rows are `pitch` bytes apart.
///
/// Hardware renderers pad rows (Direct3D 11/12 use 256-byte alignment),
/// so a flat `copy_from_slice` shears every texture whose row is not
/// already a multiple of the padding into diagonal stripes.
fn copy_rows_to_pitch(dst: &mut [u8], pitch: usize, rgba_data: &[u8], width: u32) {
    let row_bytes = width as usize * 4;
    if row_bytes == 0 {
        return;
    }
    if pitch == row_bytes {
        let n = rgba_data.len().min(dst.len());
        dst[..n].copy_from_slice(&rgba_data[..n]);
        return;
    }
    for (src_row, dst_row) in rgba_data.chunks_exact(row_bytes).zip(dst.chunks_mut(pitch)) {
        let n = row_bytes.min(dst_row.len());
        dst_row[..n].copy_from_slice(&src_row[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::copy_rows_to_pitch;

    #[test]
    fn copy_rows_honours_padded_pitch() {
        // 3x2 image, 12-byte rows, into a 16-byte pitch buffer.
        let src: Vec<u8> = (0..24).collect();
        let mut dst = vec![0xAA; 32];
        copy_rows_to_pitch(&mut dst, 16, &src, 3);
        assert_eq!(&dst[..12], &src[..12]);
        assert_eq!(&dst[12..16], &[0xAA; 4], "row padding untouched");
        assert_eq!(&dst[16..28], &src[12..24]);
    }

    #[test]
    fn copy_rows_tight_pitch_is_flat_copy() {
        let src: Vec<u8> = (0..24).collect();
        let mut dst = vec![0; 24];
        copy_rows_to_pitch(&mut dst, 12, &src, 3);
        assert_eq!(dst, src);
    }
}
