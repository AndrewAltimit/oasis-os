//! Texture blitting operations for the SDL3 backend.
//!
//! Contains sub-rect, tinted, and flipped blit methods.

use std::collections::HashMap;

use oasis_core::backend::{BackendErrExt, Color, TextureId, texture_not_found};
use oasis_core::error::Result;
use sdl3::render::Texture;

use super::{SdlBackend, frect};

/// SDL's default texture modulation (no tint, fully opaque).
pub(crate) const NEUTRAL_MOD: (u8, u8, u8, u8) = (255, 255, 255, 255);

/// Ensure `texture`'s color/alpha modulation equals `want`, issuing SDL
/// calls only when the tracked state differs. Modulation is per-texture
/// GPU state, so it is tracked per texture id in `mods` (absent entry =
/// SDL default). Tinted blits leave their tint applied and rely on every
/// other blit path calling this with [`NEUTRAL_MOD`] — that turns the
/// old set/reset pair (4 calls per tinted blit) into zero calls when the
/// same texture is blitted repeatedly with the same tint.
pub(crate) fn ensure_texture_mod(
    mods: &mut HashMap<u64, (u8, u8, u8, u8)>,
    id: u64,
    texture: &mut Texture,
    want: (u8, u8, u8, u8),
) {
    let cur = mods.entry(id).or_insert(NEUTRAL_MOD);
    if (cur.0, cur.1, cur.2) != (want.0, want.1, want.2) {
        texture.set_color_mod(want.0, want.1, want.2);
    }
    if cur.3 != want.3 {
        texture.set_alpha_mod(want.3);
    }
    *cur = want;
}

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
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        ensure_texture_mod(&mut self.texture_mods, tex.0, texture, NEUTRAL_MOD);
        let src_rect = frect(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = frect(tx, ty, dst_w, dst_h);
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
        ensure_texture_mod(
            &mut self.texture_mods,
            tex.0,
            texture,
            (tint.r, tint.g, tint.b, tint.a),
        );
        let dst_rect = frect(tx, ty, w, h);
        self.canvas.copy(texture, None, dst_rect).backend_err()?;
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
        ensure_texture_mod(
            &mut self.texture_mods,
            tex.0,
            texture,
            (tint.r, tint.g, tint.b, tint.a),
        );
        let src_rect = frect(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = frect(tx, ty, dst_w, dst_h);
        self.canvas
            .copy(texture, src_rect, dst_rect)
            .backend_err()?;
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
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        ensure_texture_mod(&mut self.texture_mods, tex.0, texture, NEUTRAL_MOD);
        let dst_rect = frect(tx, ty, w, h);
        self.canvas
            .copy_ex(texture, None, dst_rect, 0.0, None, flip_h, flip_v)
            .backend_err()?;
        Ok(())
    }
}
