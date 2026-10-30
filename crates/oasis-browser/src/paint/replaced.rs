//! `object-fit` geometry for replaced images (used by `record.rs`).

use crate::css::values::ObjectFit;

/// Compute blit position and size for a given `object-fit` mode.
///
/// Exposed as `pub(crate)` for the display list recorder.
pub(crate) fn compute_object_fit_pub(
    fit: ObjectFit,
    img_w: u32,
    img_h: u32,
    box_w: u32,
    box_h: u32,
    box_x: i32,
    box_y: i32,
) -> (i32, i32, u32, u32) {
    compute_object_fit(fit, img_w, img_h, box_w, box_h, box_x, box_y)
}

/// Compute blit position and size for a given `object-fit` mode.
///
/// Returns `(x, y, width, height)` in screen pixels.
fn compute_object_fit(
    fit: ObjectFit,
    img_w: u32,
    img_h: u32,
    box_w: u32,
    box_h: u32,
    box_x: i32,
    box_y: i32,
) -> (i32, i32, u32, u32) {
    if img_w == 0 || img_h == 0 || box_w == 0 || box_h == 0 {
        return (box_x, box_y, box_w, box_h);
    }

    match fit {
        ObjectFit::Fill => (box_x, box_y, box_w, box_h),
        ObjectFit::Contain | ObjectFit::ScaleDown => {
            let scale_x = box_w as f32 / img_w as f32;
            let scale_y = box_h as f32 / img_h as f32;
            let mut scale = scale_x.min(scale_y);
            if fit == ObjectFit::ScaleDown {
                scale = scale.min(1.0);
            }
            let w = (img_w as f32 * scale) as u32;
            let h = (img_h as f32 * scale) as u32;
            let x = box_x + (box_w as i32 - w as i32) / 2;
            let y = box_y + (box_h as i32 - h as i32) / 2;
            (x, y, w, h)
        },
        ObjectFit::Cover => {
            let scale_x = box_w as f32 / img_w as f32;
            let scale_y = box_h as f32 / img_h as f32;
            let scale = scale_x.max(scale_y);
            let w = (img_w as f32 * scale) as u32;
            let h = (img_h as f32 * scale) as u32;
            let x = box_x + (box_w as i32 - w as i32) / 2;
            let y = box_y + (box_h as i32 - h as i32) / 2;
            (x, y, w, h)
        },
        ObjectFit::None => {
            let x = box_x + (box_w as i32 - img_w as i32) / 2;
            let y = box_y + (box_h as i32 - img_h as i32) / 2;
            (x, y, img_w, img_h)
        },
    }
}
