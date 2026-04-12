//! Border painting: per-edge rendering with solid, dashed, dotted, and double styles.

use crate::css::values::BorderStyle;
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use super::PaintContext;

/// Darken a color by 40% for 3D border effects.
fn darken(c: Color) -> Color {
    Color::rgba(
        (c.r as f32 * 0.6) as u8,
        (c.g as f32 * 0.6) as u8,
        (c.b as f32 * 0.6) as u8,
        c.a,
    )
}

/// Lighten a color by 40% for 3D border effects.
fn lighten(c: Color) -> Color {
    Color::rgba(
        (c.r as f32 + (255.0 - c.r as f32) * 0.4) as u8,
        (c.g as f32 + (255.0 - c.g as f32) * 0.4) as u8,
        (c.b as f32 + (255.0 - c.b as f32) * 0.4) as u8,
        c.a,
    )
}

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
    let bx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
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
        BorderStyle::Groove | BorderStyle::Ridge => {
            // 3D effect: two halves with light/dark colors.
            // Groove: outer half darker, inner half lighter.
            // Ridge: outer half lighter, inner half darker.
            let (outer, inner) = if style == BorderStyle::Groove {
                (darken(color), lighten(color))
            } else {
                (lighten(color), darken(color))
            };
            let thickness = if horizontal { h } else { w };
            let half = thickness / 2;
            let other = thickness - half;
            if horizontal {
                backend.fill_rect(x, y, w, half.max(1), outer)?;
                backend.fill_rect(x, y + half as i32, w, other.max(1), inner)?;
            } else {
                backend.fill_rect(x, y, half.max(1), h, outer)?;
                backend.fill_rect(x + half as i32, y, other.max(1), h, inner)?;
            }
        },
        BorderStyle::Inset | BorderStyle::Outset => {
            // 3D effect: top/left vs bottom/right get different shading.
            // For a single edge we use simpler logic: inset makes top/left
            // darker, outset makes top/left lighter.
            let is_top_or_left = horizontal; // called with horizontal=true for top edge
            let shade = match (style, is_top_or_left) {
                (BorderStyle::Inset, true) | (BorderStyle::Outset, false) => darken(color),
                _ => lighten(color),
            };
            backend.fill_rect(x, y, w, h, shade)?;
        },
        BorderStyle::None => {},
    }
    Ok(())
}

/// Paint the CSS `outline` around the border box.
///
/// The outline is drawn outside the border box, offset by `outline_offset`.
/// Unlike borders, outlines do not take up space in the layout.
pub(super) fn paint_outline(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let style = &layout_box.style;

    if style.outline_width <= 0.0 || style.outline_style == BorderStyle::None {
        return Ok(());
    }

    let border = layout_box.dimensions.border_box();
    let ow = style.outline_width;
    let oo = style.outline_offset;

    // The outline box is the border box expanded by (outline_offset + outline_width).
    let ox = (border.x - ctx.scroll_x + offset_x as f32 - oo - ow) as i32;
    let oy = (border.y - ctx.scroll_y + offset_y as f32 - oo - ow) as i32;
    let total_w = (border.width + 2.0 * (oo + ow)).max(0.0) as u32;
    let total_h = (border.height + 2.0 * (oo + ow)).max(0.0) as u32;
    let thickness = ow as u32;
    let color = style.outline_color;
    let outline_style = style.outline_style;

    // Top
    paint_border_edge(
        backend,
        ox,
        oy,
        total_w,
        thickness,
        color,
        outline_style,
        true,
    )?;
    // Bottom
    paint_border_edge(
        backend,
        ox,
        oy + total_h as i32 - thickness as i32,
        total_w,
        thickness,
        color,
        outline_style,
        true,
    )?;
    // Left
    paint_border_edge(
        backend,
        ox,
        oy,
        thickness,
        total_h,
        color,
        outline_style,
        false,
    )?;
    // Right
    paint_border_edge(
        backend,
        ox + total_w as i32 - thickness as i32,
        oy,
        thickness,
        total_h,
        color,
        outline_style,
        false,
    )?;

    Ok(())
}
