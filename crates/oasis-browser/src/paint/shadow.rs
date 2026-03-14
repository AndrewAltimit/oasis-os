//! Box shadow painting.

use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use super::PaintContext;

/// Paint a box shadow behind the element.
pub(super) fn paint_box_shadow(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let shadow = match layout_box.style.box_shadow {
        Some(ref s) => s,
        None => return Ok(()),
    };

    let border = layout_box.dimensions.border_box();
    let bx = (border.x + offset_x as f32 + shadow.offset_x) as i32;
    let by = (border.y - ctx.scroll_y + offset_y as f32 + shadow.offset_y) as i32;
    let bw = (border.width + shadow.spread * 2.0) as u32;
    let bh = (border.height + shadow.spread * 2.0) as u32;

    // Approximate blur with concentric rectangles at decreasing opacity.
    let steps = (shadow.blur as i32).max(1);
    for i in (0..steps).rev() {
        let t = i as f32 / steps as f32;
        let alpha = ((shadow.color.a as f32) * (1.0 - t) * 0.4) as u8;
        if alpha == 0 {
            continue;
        }
        let expand = i;
        let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
        backend.fill_rect(
            bx - expand,
            by - expand,
            bw + expand as u32 * 2,
            bh + expand as u32 * 2,
            color,
        )?;
    }
    Ok(())
}
