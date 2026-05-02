//! CSS transform / 3D / perspective helpers used by the paint pass.
//!
//! Pure functions for resolving `transform-origin` and
//! `perspective-origin` against an element's box, building the
//! composed 2D affine matrix from a `transform:` list, and asking
//! "does this transform list contain any 3D function?". The
//! `paint_box` recursive walker in [`super`] uses these to decide
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

/// Compute the full 2D affine transform from CSS transforms.
///
/// Returns the composed matrix which callers use either as a simple
/// translation offset (fast path) or for full geometry transformation.
/// `transform_origin` defaults to `50% 50% 0` when `None`.
pub(crate) fn compute_transform_matrix(
    transforms: &[TransformFunction],
    transform_origin: Option<&TransformOrigin>,
    content: &Rect,
) -> crate::transform::AffineTransform2D {
    let (ox, oy, _oz) = resolve_transform_origin(transform_origin, content);
    crate::transform::AffineTransform2D::from_css_transforms(transforms, ox, oy)
}

/// Compute offset adjustments from CSS transforms.
///
/// Returns the translation component of the composed transform matrix
/// added to the base offsets. For translation-only transforms this is
/// exact; for rotation/scale/skew the full matrix is available via
/// [`compute_transform_matrix`].
pub(crate) fn compute_transform_offsets(
    transforms: &[TransformFunction],
    transform_origin: Option<&TransformOrigin>,
    content: &Rect,
    base_x: i32,
    base_y: i32,
) -> (i32, i32) {
    if transforms.is_empty() {
        return (base_x, base_y);
    }
    let m = compute_transform_matrix(transforms, transform_origin, content);
    (base_x + m.e as i32, base_y + m.f as i32)
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
    fn empty_transforms_return_base_offsets_unchanged() {
        let (x, y) = compute_transform_offsets(&[], None, &rect(0.0, 0.0, 100.0, 100.0), 12, 34);
        assert_eq!((x, y), (12, 34));
    }

    #[test]
    fn translate_transform_shifts_offsets() {
        let (x, y) = compute_transform_offsets(
            &[TransformFunction::Translate(10.0, 20.0)],
            None,
            &rect(0.0, 0.0, 100.0, 100.0),
            5,
            7,
        );
        assert_eq!((x, y), (15, 27));
    }
}
