//! Border painting: per-edge rendering with solid, dashed, dotted, and double styles.

use crate::css::values::BorderStyle;
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use super::PaintContext;

pub(super) fn paint_borders(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let d = &layout_box.dimensions;
    let style = &layout_box.style;
    let border = d.border_box();
    let bx = (border.x + offset_x as f32) as i32;
    let by = (border.y - ctx.scroll_y + offset_y as f32) as i32;
    let bw = border.width as u32;
    let bh = border.height as u32;

    // Top
    if d.border.top > 0.0 && style.border_top_style != BorderStyle::None {
        paint_border_edge(
            backend,
            bx,
            by,
            bw,
            d.border.top as u32,
            style.border_top_color,
            style.border_top_style,
            true,
        )?;
    }
    // Right
    if d.border.right > 0.0 && style.border_right_style != BorderStyle::None {
        paint_border_edge(
            backend,
            bx + bw as i32 - d.border.right as i32,
            by,
            d.border.right as u32,
            bh,
            style.border_right_color,
            style.border_right_style,
            false,
        )?;
    }
    // Bottom
    if d.border.bottom > 0.0 && style.border_bottom_style != BorderStyle::None {
        paint_border_edge(
            backend,
            bx,
            by + bh as i32 - d.border.bottom as i32,
            bw,
            d.border.bottom as u32,
            style.border_bottom_color,
            style.border_bottom_style,
            true,
        )?;
    }
    // Left
    if d.border.left > 0.0 && style.border_left_style != BorderStyle::None {
        paint_border_edge(
            backend,
            bx,
            by,
            d.border.left as u32,
            bh,
            style.border_left_color,
            style.border_left_style,
            false,
        )?;
    }

    Ok(())
}

/// Paint a single border edge with the appropriate style.
///
/// For `Solid`, draws a filled rectangle. For `Dashed`, draws
/// alternating filled/empty segments. For `Dotted`, draws small
/// square dots. For `Double`, draws two parallel lines.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_border_edge(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: Color,
    style: BorderStyle,
    horizontal: bool,
) -> Result<()> {
    match style {
        BorderStyle::Solid => {
            backend.fill_rect(x, y, w, h, color)?;
        },
        BorderStyle::Dashed => {
            // Alternating filled/empty segments along the edge.
            let length = if horizontal { w } else { h };
            let thickness = if horizontal { h } else { w };
            let dash_len = (thickness * 3).max(4);
            let mut pos = 0u32;
            let mut draw = true;
            while pos < length {
                let seg = dash_len.min(length - pos);
                if draw {
                    if horizontal {
                        backend.fill_rect(x + pos as i32, y, seg, thickness, color)?;
                    } else {
                        backend.fill_rect(x, y + pos as i32, thickness, seg, color)?;
                    }
                }
                pos += seg;
                draw = !draw;
            }
        },
        BorderStyle::Dotted => {
            // Small square dots along the edge.
            let length = if horizontal { w } else { h };
            let thickness = if horizontal { h } else { w };
            let dot_size = thickness.max(1);
            let mut pos = 0u32;
            while pos < length {
                if horizontal {
                    backend.fill_rect(x + pos as i32, y, dot_size, thickness, color)?;
                } else {
                    backend.fill_rect(x, y + pos as i32, thickness, dot_size, color)?;
                }
                pos += dot_size * 2;
            }
        },
        BorderStyle::Double => {
            // Two parallel lines separated by a gap.
            let thickness = if horizontal { h } else { w };
            let line = (thickness / 3).max(1);
            let gap = thickness.saturating_sub(line * 2);
            if horizontal {
                backend.fill_rect(x, y, w, line, color)?;
                backend.fill_rect(x, y + (line + gap) as i32, w, line, color)?;
            } else {
                backend.fill_rect(x, y, line, h, color)?;
                backend.fill_rect(x + (line + gap) as i32, y, line, h, color)?;
            }
        },
        BorderStyle::None => {},
    }
    Ok(())
}
