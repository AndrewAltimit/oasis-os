//! Nine-patch (9-slice) rendering for scalable themed borders.

use crate::backend::{SdiBackend, TextureId};
use crate::error::Result;

/// Nine-patch definition for a texture.
///
/// The texture is divided into a 3x3 grid. Corners render at fixed size,
/// edges stretch in one dimension, and the center stretches in both.
pub struct NinePatch {
    pub texture: TextureId,
    pub tex_width: u32,
    pub tex_height: u32,
    pub left: u16,
    pub right: u16,
    pub top: u16,
    pub bottom: u16,
}

impl NinePatch {
    /// Draw the nine-patch at the given screen position and size.
    pub fn draw(&self, backend: &mut dyn SdiBackend, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let l = self.left as u32;
        let r = self.right as u32;
        let t = self.top as u32;
        let b = self.bottom as u32;
        let tw = self.tex_width;
        let th = self.tex_height;
        let mid_w = w.saturating_sub(l + r);
        let mid_h = h.saturating_sub(t + b);
        let src_mid_w = tw.saturating_sub(l + r);
        let src_mid_h = th.saturating_sub(t + b);

        // Corners (fixed size).
        backend.blit_sub(self.texture, 0, 0, l, t, x, y, l, t)?;
        backend.blit_sub(self.texture, tw - r, 0, r, t, x + (w - r) as i32, y, r, t)?;
        backend.blit_sub(self.texture, 0, th - b, l, b, x, y + (h - b) as i32, l, b)?;
        backend.blit_sub(
            self.texture,
            tw - r,
            th - b,
            r,
            b,
            x + (w - r) as i32,
            y + (h - b) as i32,
            r,
            b,
        )?;

        // Edges (stretched in one dimension).
        backend.blit_sub(self.texture, l, 0, src_mid_w, t, x + l as i32, y, mid_w, t)?;
        backend.blit_sub(
            self.texture,
            l,
            th - b,
            src_mid_w,
            b,
            x + l as i32,
            y + (h - b) as i32,
            mid_w,
            b,
        )?;
        backend.blit_sub(self.texture, 0, t, l, src_mid_h, x, y + t as i32, l, mid_h)?;
        backend.blit_sub(
            self.texture,
            tw - r,
            t,
            r,
            src_mid_h,
            x + (w - r) as i32,
            y + t as i32,
            r,
            mid_h,
        )?;

        // Center (stretched in both dimensions).
        backend.blit_sub(
            self.texture,
            l,
            t,
            src_mid_w,
            src_mid_h,
            x + l as i32,
            y + t as i32,
            mid_w,
            mid_h,
        )?;
        Ok(())
    }
}
