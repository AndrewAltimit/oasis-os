//! `SdiTextures` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore, TextureId};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiTextures
// ---------------------------------------------------------------------------

/// Texture sub-blitting and tinting operations.
#[allow(clippy::too_many_arguments)]
pub trait SdiTextures: SdiCore {
    /// Blit a sub-rectangle from a texture.
    fn blit_sub(
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
        let _ = (src_x, src_y, src_w, src_h);
        self.blit(tex, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with a multiplicative color tint.
    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let _ = tint;
        self.blit(tex, x, y, w, h)
    }

    /// Blit a texture sub-rectangle with a color tint.
    fn blit_sub_tinted(
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
        let _ = tint;
        self.blit_sub(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with horizontal and/or vertical flip.
    fn blit_flipped(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        let _ = (flip_h, flip_v);
        self.blit(tex, x, y, w, h)
    }
}
