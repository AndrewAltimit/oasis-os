//! Replaced elements: images, SVG, canvas, video, iframes, rules.

use super::*;

pub(super) fn record_replaced(
    replaced: &crate::layout::box_model::ReplacedContent,
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    use crate::layout::box_model::ReplacedContent;

    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;

    match replaced {
        ReplacedContent::Image {
            texture: Some(tex),
            width: img_w,
            height: img_h,
            atlas_region,
            ..
        } => {
            let box_w = content.width as u32;
            let box_h = content.height as u32;
            let (blit_x, blit_y, blit_w, blit_h) = crate::paint::replaced::compute_object_fit(
                layout_box.style.object_fit,
                *img_w,
                *img_h,
                box_w,
                box_h,
                x,
                y,
            );
            if let Some(ar) = atlas_region {
                dl.push(DisplayItem::BlitSub {
                    texture: *tex,
                    src_x: ar.x,
                    src_y: ar.y,
                    src_w: ar.w,
                    src_h: ar.h,
                    dst_x: blit_x,
                    dst_y: blit_y,
                    dst_w: blit_w,
                    dst_h: blit_h,
                });
            } else {
                dl.push(DisplayItem::Blit {
                    texture: *tex,
                    x: blit_x,
                    y: blit_y,
                    w: blit_w,
                    h: blit_h,
                });
            }
        },
        ReplacedContent::Image { alt, .. } => {
            let w = content.width.max(16.0) as u32;
            let h = content.height.max(16.0) as u32;
            let color = layout_box.style.color;
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w,
                h: 1,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x,
                y: y + h as i32 - 1,
                w,
                h: 1,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w: 1,
                h,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x: x + w as i32 - 1,
                y,
                w: 1,
                h,
                color,
                node_id: ctx.current_node,
            });
            // `×` is an icon: small, in the corner like real browsers'
            // broken-image chip. Alt text is meant to be read, so it
            // follows the computed font-size, capped to the interior
            // height (a 3rem logo wrapper must not draw 30 px text in a
            // 22 px sprite), and is clipped to the box so long values
            // don't bleed into siblings.
            let interior_h = h.saturating_sub(4).max(6) as u16;
            let (label, font_size) = if alt.is_empty() {
                ("\u{00D7}", 8u16.min(interior_h))
            } else {
                let style_fs = layout_box.style.font_size.round().max(6.0) as u16;
                (alt.as_str(), style_fs.min(interior_h))
            };
            let label_w = oasis_types::backend::bitmap_measure_text(label, font_size);
            dl.push(DisplayItem::PushClip { x, y, w, h });
            dl.push(DisplayItem::DrawText {
                text: label.to_string(),
                x: x + 2,
                y: y + 2,
                font_size,
                color,
                bold: false,
                italic: false,
                width: label_w,
                node_id: ctx.current_node,
                #[cfg(feature = "web-fonts")]
                web_font_id: None,
            });
            dl.push(DisplayItem::PopClip);
        },
        ReplacedContent::HorizontalRule => {
            let style = &layout_box.style;
            let w = content.width as u32;
            if style.border_top_style != BorderStyle::None && style.border_top_width > 0.0 {
                dl.push(DisplayItem::BorderEdge {
                    x,
                    y,
                    w,
                    h: style.border_top_width as u32,
                    color: style.border_top_color,
                    style: style.border_top_style,
                    horizontal: true,
                });
            } else {
                dl.push(DisplayItem::FillRect {
                    x,
                    y,
                    w,
                    h: 1,
                    color: Color::rgb(128, 128, 128),
                    node_id: ctx.current_node,
                });
            }
        },
        ReplacedContent::LineBreak => {},
        ReplacedContent::TextInput {
            value,
            placeholder,
            is_password,
            ..
        } => {
            let focused = layout_box.node.is_some() && layout_box.node == ctx.focused_node;
            record_text_input(
                layout_box,
                dl,
                x,
                y,
                value,
                placeholder,
                *is_password,
                focused,
            );
        },
        ReplacedContent::Checkbox { checked } => {
            record_checkbox(layout_box, dl, x, y, *checked);
        },
        ReplacedContent::RadioButton { checked } => {
            record_radio_button(layout_box, dl, x, y, *checked);
        },
        ReplacedContent::TextArea {
            value, placeholder, ..
        } => {
            let focused = layout_box.node.is_some() && layout_box.node == ctx.focused_node;
            record_textarea(layout_box, dl, x, y, value, placeholder, focused);
        },
        ReplacedContent::SelectBox {
            label,
            open,
            options,
            selected_index,
        } => {
            record_select_box(layout_box, dl, x, y, label, *open, options, *selected_index);
        },
        ReplacedContent::SubmitButton { label } => {
            record_submit_button(layout_box, dl, x, y, label);
        },
        // SVG and canvas run their own drawing code at replay time, but
        // are recorded here in paint order so they stack, clip, scroll
        // and fade like everything else.
        ReplacedContent::Svg { element } => {
            dl.push(DisplayItem::Svg {
                element: std::rc::Rc::clone(element),
                x,
                y,
                width: content.width,
                height: content.height,
            });
        },
        ReplacedContent::Canvas { state } => {
            dl.push(DisplayItem::Canvas {
                state: std::rc::Rc::clone(state),
                x,
                y,
                w: content.width as u32,
                h: content.height as u32,
            });
        },
    }
}
