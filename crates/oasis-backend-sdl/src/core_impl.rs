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
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
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
            .with_lock(None, |buffer: &mut [u8], _pitch: usize| {
                buffer[..rgba_data.len()].copy_from_slice(rgba_data);
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
                        oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as u32
                    }
                })
                .sum();
        }
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let rect = Rect::new(x, y, w, h);
        let surface = self.canvas.read_pixels(rect).backend_err()?;
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
            .with_lock(None, |buffer: &mut [u8], _pitch: usize| {
                buffer[..rgba_data.len()].copy_from_slice(rgba_data);
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
