//! List marker painting: disc, circle, square, decimal.

use crate::layout::box_model::{LayoutBox, ListMarker};
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use super::PaintContext;

pub(super) fn paint_list_marker(
    marker: &ListMarker,
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32 - 20.0) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;
    let color = layout_box.style.color;
    let font_size = layout_box.style.font_size as u16;

    match marker {
        ListMarker::Disc => {
            backend.draw_text("\u{2022}", x, y, font_size, color)?;
        },
        ListMarker::Circle => {
            backend.draw_text("\u{25E6}", x, y, font_size, color)?;
        },
        ListMarker::Square => {
            backend.draw_text("\u{25AA}", x, y, font_size, color)?;
        },
        ListMarker::Decimal(n) => {
            let text = format!("{}.", n);
            backend.draw_text(&text, x - 10, y, font_size, color)?;
        },
        ListMarker::None => {},
    }

    Ok(())
}
