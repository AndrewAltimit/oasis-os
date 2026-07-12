//! Nine-patch (9-slice) rendering for scalable themed borders.
//!
//! The implementation lives in `oasis_types::nine_patch` so the SDI
//! registry can draw nine-patch objects without depending on this crate;
//! this module re-exports it for widget consumers.

pub use oasis_types::nine_patch::{NinePatch, NinePatchSlices};

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::TextureId;

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
