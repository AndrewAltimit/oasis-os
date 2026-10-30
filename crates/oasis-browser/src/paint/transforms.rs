//! CSS transform / 3D / perspective helpers used by the paint pass.
//!
//! Pure functions for resolving `transform-origin` and
//! `perspective-origin` against an element's box, building the
//! composed 2D affine matrix from a `transform:` list, and asking
//! "does this transform list contain any 3D function?". The
//! `record_box` recursive walker in `record/mod.rs` uses these to decide
//! between the cheap 2D-translation fast path, the full affine path,
//! and the screen-space 3D projection path.

use crate::css::values::TransformFunction;
use crate::css::values::types::{PerspectiveOrigin, TransformOrigin};
use crate::layout::box_model::Rect;

/// Returns `true` if the transform list contains any 3D function.
/// Used to gate the perspective projection path: the orthographic
/// flatten is the right choice for pure 2D transforms even when an
/// ancestor has `perspective`.
pub(crate) fn transforms_have_3d(transforms: &[TransformFunction]) -> bool {
    transforms.iter().any(|t| {
        matches!(
            t,
            TransformFunction::Translate3d(..)
                | TransformFunction::TranslateZ(_)
                | TransformFunction::Scale3d(..)
                | TransformFunction::ScaleZ(_)
                | TransformFunction::RotateX(_)
                | TransformFunction::RotateY(_)
                | TransformFunction::Rotate3d(..)
                | TransformFunction::Matrix3d(_)
                | TransformFunction::Perspective(_)
        )
    })
}

/// Resolve a CSS `transform-origin` against an element's content box.
///
/// Returns absolute pixel offsets `(ox, oy, oz)` from the content
/// box's top-left corner. When `origin` is `None`, defaults to the
/// spec's `50% 50% 0` (box center, Z = 0).
pub(crate) fn resolve_transform_origin(
    origin: Option<&TransformOrigin>,
    content: &Rect,
) -> (f32, f32, f32) {
    match origin {
        None => (content.width / 2.0, content.height / 2.0, 0.0),
        Some(o) => {
            let ox = o.x_pct.map(|p| content.width * p).unwrap_or(o.x);
            let oy = o.y_pct.map(|p| content.height * p).unwrap_or(o.y);
            (ox, oy, o.z)
        },
    }
}

/// Resolve a CSS `perspective-origin` against an element's border box.
///
/// Defaults to the spec's `50% 50%` when `origin` is `None`.
pub(crate) fn resolve_perspective_origin(
    origin: Option<&PerspectiveOrigin>,
    container: &Rect,
) -> (f32, f32) {
    match origin {
        None => (container.width / 2.0, container.height / 2.0),
        Some(o) => {
            let ox = o.x_pct.map(|p| container.width * p).unwrap_or(o.x);
            let oy = o.y_pct.map(|p| container.height * p).unwrap_or(o.y);
            (ox, oy)
        },
    }
}

/// Compute the 2D affine transform from CSS transforms, pivoting on
/// the content-box-local transform origin (`50% 50% 0` when `None`).
///
/// The recorder uses it to detect translate-only transforms (whose
/// `e`/`f` are a plain box offset); rotations and scales are built in
/// screen space instead.
pub(super) fn compute_transform_matrix(
    transforms: &[TransformFunction],
    transform_origin: Option<&TransformOrigin>,
    content: &Rect,
) -> crate::transform::AffineTransform2D {
    let (ox, oy, _oz) = resolve_transform_origin(transform_origin, content);
    crate::transform::AffineTransform2D::from_css_transforms(transforms, ox, oy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::TransformFunction;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn empty_transform_list_has_no_3d() {
        assert!(!transforms_have_3d(&[]));
    }

    #[test]
    fn translate_z_counts_as_3d() {
        assert!(transforms_have_3d(&[TransformFunction::TranslateZ(10.0)]));
    }

    #[test]
    fn rotate_2d_does_not_count_as_3d() {
        assert!(!transforms_have_3d(&[TransformFunction::Rotate(45.0)]));
    }

    #[test]
    fn rotate_x_counts_as_3d() {
        assert!(transforms_have_3d(&[TransformFunction::RotateX(45.0)]));
    }

    #[test]
    fn perspective_counts_as_3d() {
        assert!(transforms_have_3d(&[TransformFunction::Perspective(500.0)]));
    }

    #[test]
    fn transform_origin_defaults_to_box_center() {
        let (ox, oy, oz) = resolve_transform_origin(None, &rect(0.0, 0.0, 100.0, 60.0));
        assert_eq!(ox, 50.0);
        assert_eq!(oy, 30.0);
        assert_eq!(oz, 0.0);
    }

    #[test]
    fn perspective_origin_defaults_to_box_center() {
        let (ox, oy) = resolve_perspective_origin(None, &rect(0.0, 0.0, 200.0, 80.0));
        assert_eq!(ox, 100.0);
        assert_eq!(oy, 40.0);
    }

    #[test]
    fn empty_transforms_are_identity() {
        let m = compute_transform_matrix(&[], None, &rect(0.0, 0.0, 100.0, 100.0));
        assert!(m.is_translation_only());
        assert_eq!((m.e, m.f), (0.0, 0.0));
    }

    #[test]
    fn translate_transform_is_translation_only() {
        let m = compute_transform_matrix(
            &[TransformFunction::Translate(10.0, 20.0)],
            None,
            &rect(0.0, 0.0, 100.0, 100.0),
        );
        assert!(m.is_translation_only());
        assert_eq!((m.e, m.f), (10.0, 20.0));
    }
}
