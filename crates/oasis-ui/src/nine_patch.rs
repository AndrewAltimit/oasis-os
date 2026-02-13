//! Nine-patch (9-slice) rendering for scalable themed borders.

use oasis_types::backend::{SdiBackend, TextureId};
use oasis_types::error::Result;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NinePatch {
        NinePatch {
            texture: TextureId(1),
            tex_width: 64,
            tex_height: 64,
            left: 8,
            right: 8,
            top: 8,
            bottom: 8,
        }
    }

    #[test]
    fn fields_accessible() {
        let np = sample();
        assert_eq!(np.texture, TextureId(1));
        assert_eq!(np.tex_width, 64);
        assert_eq!(np.tex_height, 64);
        assert_eq!(np.left, 8);
        assert_eq!(np.right, 8);
        assert_eq!(np.top, 8);
        assert_eq!(np.bottom, 8);
    }

    #[test]
    fn asymmetric_margins() {
        let np = NinePatch {
            texture: TextureId(2),
            tex_width: 100,
            tex_height: 80,
            left: 10,
            right: 20,
            top: 5,
            bottom: 15,
        };
        assert_eq!(np.left, 10);
        assert_eq!(np.right, 20);
        assert_eq!(np.top, 5);
        assert_eq!(np.bottom, 15);
    }

    #[test]
    fn zero_margins() {
        let np = NinePatch {
            texture: TextureId(3),
            tex_width: 32,
            tex_height: 32,
            left: 0,
            right: 0,
            top: 0,
            bottom: 0,
        };
        assert_eq!(np.left + np.right, 0);
        assert_eq!(np.top + np.bottom, 0);
    }
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
