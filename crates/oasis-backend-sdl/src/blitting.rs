//! Texture blitting operations for the SDL2 backend.
//!
//! Contains sub-rect, tinted, and flipped blit methods.

use sdl2::rect::Rect;

use oasis_core::backend::{BackendErrExt, Color, TextureId, texture_not_found};
use oasis_core::error::Result;

use super::SdlBackend;

impl SdlBackend {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_sub_impl(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        let (tx, ty) = self.translate(dst_x, dst_y);
        let texture = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        let src_rect = Rect::new(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = Rect::new(tx, ty, dst_w, dst_h);
        self.canvas
            .copy(texture, src_rect, dst_rect)
            .backend_err()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_tinted_impl(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        texture.set_color_mod(tint.r, tint.g, tint.b);
        texture.set_alpha_mod(tint.a);
        let dst_rect = Rect::new(tx, ty, w, h);
        self.canvas.copy(texture, None, dst_rect).backend_err()?;
        // Reset modulation on the same texture.
        texture.set_color_mod(255, 255, 255);
        texture.set_alpha_mod(255);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_sub_tinted_impl(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(dst_x, dst_y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        texture.set_color_mod(tint.r, tint.g, tint.b);
        texture.set_alpha_mod(tint.a);
        let src_rect = Rect::new(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = Rect::new(tx, ty, dst_w, dst_h);
        self.canvas
            .copy(texture, src_rect, dst_rect)
            .backend_err()?;
        // Reset modulation on the same texture.
        texture.set_color_mod(255, 255, 255);
        texture.set_alpha_mod(255);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_flipped_impl(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        let dst_rect = Rect::new(tx, ty, w, h);
        self.canvas
            .copy_ex(texture, None, dst_rect, 0.0, None, flip_h, flip_v)
            .backend_err()?;
        Ok(())
    }
}
