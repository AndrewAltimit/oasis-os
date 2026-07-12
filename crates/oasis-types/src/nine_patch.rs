//! Nine-patch (9-slice) rendering for scalable themed borders.
//!
//! Lives in `oasis-types` (rather than `oasis-ui`, which re-exports it)
//! so the SDI registry can draw nine-patch objects at render time without
//! a widget-crate dependency.

use crate::backend::{SdiBackend, TextureId};
use crate::error::Result;

/// Nine-patch slicing metadata (insets + source texture dimensions).
///
/// Attached to a textured SDI object; the registry combines it with the
/// object's `texture` into a [`NinePatch`] at draw time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NinePatchSlices {
    /// Source texture width in pixels.
    pub tex_width: u32,
    /// Source texture height in pixels.
    pub tex_height: u32,
    /// Left edge margin in pixels.
    pub left: u16,
    /// Top edge margin in pixels.
    pub top: u16,
    /// Right edge margin in pixels.
    pub right: u16,
    /// Bottom edge margin in pixels.
    pub bottom: u16,
}

/// Nine-patch definition for a texture.
///
/// The texture is divided into a 3x3 grid. Corners render at fixed size,
/// edges stretch in one dimension, and the center stretches in both.
pub struct NinePatch {
    /// Source texture.
    pub texture: TextureId,
    /// Total texture width in pixels.
    pub tex_width: u32,
    /// Total texture height in pixels.
    pub tex_height: u32,
    /// Left edge margin in pixels.
    pub left: u16,
    /// Right edge margin in pixels.
    pub right: u16,
    /// Top edge margin in pixels.
    pub top: u16,
    /// Bottom edge margin in pixels.
    pub bottom: u16,
}

impl NinePatch {
    /// Build a nine-patch from a texture id plus slicing metadata.
    pub fn from_slices(texture: TextureId, s: NinePatchSlices) -> Self {
        Self {
            texture,
            tex_width: s.tex_width,
            tex_height: s.tex_height,
            left: s.left,
            right: s.right,
            top: s.top,
            bottom: s.bottom,
        }
    }

    /// Draw the nine-patch at the given screen position and size.
    pub fn draw(&self, backend: &mut dyn SdiBackend, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let l = self.left as u32;
        let r = self.right as u32;
        let t = self.top as u32;
        let b = self.bottom as u32;
        let tw = self.tex_width;
        let th = self.tex_height;

        // Skip drawing if margins exceed texture or target dimensions.
        if l + r > tw || t + b > th || l + r > w || t + b > h {
            return Ok(());
        }

        let mid_w = w - (l + r);
        let mid_h = h - (t + b);
        let src_mid_w = tw - (l + r);
        let src_mid_h = th - (t + b);
        let src_r = tw - r;
        let src_b = th - b;
        let dst_r = x + (w - r) as i32;
        let dst_b = y + (h - b) as i32;

        // Corners (fixed size).
        backend.blit_sub(self.texture, 0, 0, l, t, x, y, l, t)?;
        backend.blit_sub(self.texture, src_r, 0, r, t, dst_r, y, r, t)?;
        backend.blit_sub(self.texture, 0, src_b, l, b, x, dst_b, l, b)?;
        backend.blit_sub(self.texture, src_r, src_b, r, b, dst_r, dst_b, r, b)?;

        // Edges (stretched in one dimension).
        backend.blit_sub(self.texture, l, 0, src_mid_w, t, x + l as i32, y, mid_w, t)?;
        backend.blit_sub(
            self.texture,
            l,
            src_b,
            src_mid_w,
            b,
            x + l as i32,
            dst_b,
            mid_w,
            b,
        )?;
        backend.blit_sub(self.texture, 0, t, l, src_mid_h, x, y + t as i32, l, mid_h)?;
        backend.blit_sub(
            self.texture,
            src_r,
            t,
            r,
            src_mid_h,
            dst_r,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_slices_maps_fields() {
        let np = NinePatch::from_slices(
            TextureId(3),
            NinePatchSlices {
                tex_width: 64,
                tex_height: 48,
                left: 4,
                top: 5,
                right: 6,
                bottom: 7,
            },
        );
        assert_eq!(np.texture, TextureId(3));
        assert_eq!(np.tex_width, 64);
        assert_eq!(np.tex_height, 48);
        assert_eq!(np.left, 4);
        assert_eq!(np.top, 5);
        assert_eq!(np.right, 6);
        assert_eq!(np.bottom, 7);
    }
}
