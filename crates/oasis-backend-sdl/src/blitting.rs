//! Texture blitting for the SDL3 backend.
//!
//! `blit`, `blit_sub` and `blit_tinted` are all one SDL `copy` with an
//! optional source rectangle and a color modulation; [`SdlBackend::copy_texture`]
//! is that one path.

use std::collections::HashMap;

use oasis_types::backend::{BackendErrExt, Color, TextureId, texture_not_found};
use oasis_types::error::Result;
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

/// A texture source rectangle `(x, y, w, h)` in texture pixels.
pub(crate) type SrcRect = (u32, u32, u32, u32);

impl SdlBackend {
    /// Copy texture `tex` (the `src` part of it, or all of it) to the
    /// translated destination rect, modulated by `tint` (`Color::WHITE`
    /// draws it unchanged).
    pub(crate) fn copy_texture(
        &mut self,
        tex: TextureId,
        src: Option<SrcRect>,
        (x, y, w, h): (i32, i32, u32, u32),
        tint: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| texture_not_found(tex))?;
        ensure_texture_mod(
            &mut self.texture_mods,
            tex.0,
            texture,
            (tint.r, tint.g, tint.b, tint.a),
        );
        let src_rect = src.map(|(sx, sy, sw, sh)| frect(sx as i32, sy as i32, sw, sh));
        self.canvas
            .copy(texture, src_rect, frect(tx, ty, w, h))
            .backend_err()
    }
}
