//! Box shadow painting: outer shadows with blur, spread, and border-radius;
//! inset shadows painted inside the border box.

use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use super::PaintContext;

/// Paint box shadows behind (outer) and inside (inset) the element.
pub(super) fn paint_box_shadow(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    if layout_box.style.box_shadow.is_empty() {
        return Ok(());
    }

    let border = layout_box.dimensions.border_box();
    let radius = layout_box.style.border_radius;

    // Paint shadows in reverse order (last declared = bottommost).
    for shadow in layout_box.style.box_shadow.iter().rev() {
        if shadow.inset {
            // Inset shadow: painted inside the padding box.
            let padding = layout_box.dimensions.padding_box();
            let px = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
            let py = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
            let pw = padding.width as u32;
            let ph = padding.height as u32;

            // The inset shadow is offset and shrunk by spread.
            let steps = (shadow.blur as i32).max(1);
            for i in (0..steps).rev() {
                let t = i as f32 / steps as f32;
                let alpha = ((shadow.color.a as f32) * (1.0 - t) * 0.4) as u8;
                if alpha == 0 {
                    continue;
                }
                let shrink = i as u32;
                let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
                let sx = px + shadow.offset_x as i32 + shrink as i32;
                let sy = py + shadow.offset_y as i32 + shrink as i32;
                let sw = pw.saturating_sub(shrink * 2);
                let sh = ph.saturating_sub(shrink * 2);
                if sw == 0 || sh == 0 {
                    continue;
                }
                // Paint edge strips (top, bottom, left, right) of thickness 1
                // to create the inset shadow effect without covering content.
                let thickness = (shadow.spread as u32 + 1).min(sh / 2).min(sw / 2);
                // Top strip
                backend.fill_rect(sx, sy, sw, thickness, color)?;
                // Bottom strip
                backend.fill_rect(sx, sy + sh as i32 - thickness as i32, sw, thickness, color)?;
                // Left strip (between top and bottom to avoid corner overlap)
                backend.fill_rect(
                    sx,
                    sy + thickness as i32,
                    thickness,
                    sh.saturating_sub(thickness * 2),
                    color,
                )?;
                // Right strip (between top and bottom to avoid corner overlap)
                backend.fill_rect(
                    sx + sw as i32 - thickness as i32,
                    sy + thickness as i32,
                    thickness,
                    sh.saturating_sub(thickness * 2),
                    color,
                )?;
            }
        } else {
            // Outer shadow — delegate to fill_shadow which GPU backends can
            // override with a Gaussian blur shader.
            let sx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
            let sy = (border.y - ctx.scroll_y + offset_y as f32) as i32;
            let sw = border.width as u32;
            let sh = border.height as u32;
            backend.fill_shadow(
                sx,
                sy,
                sw,
                sh,
                shadow.blur,
                shadow.spread,
                shadow.offset_x,
                shadow.offset_y,
                shadow.color,
                radius,
            )?;
        }
    }
    Ok(())
}
