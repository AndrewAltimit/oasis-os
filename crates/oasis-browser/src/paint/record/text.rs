//! Inline content: text runs, text decorations and list markers.

use super::*;

pub(super) fn record_inline_content(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut RecordContext,
    link_map: &HashMap<NodeId, String>,
) {
    // The inline box's background was already recorded by
    // `record_box` (`record_background`); recording it again here
    // double-blended translucent inline backgrounds.

    // Text content.
    if let Some(ref text) = layout_box.text {
        let content = &layout_box.dimensions.content;
        record_text(
            text,
            content.x,
            content.y,
            &layout_box.style,
            dl,
            offset_x,
            offset_y,
            ctx,
        );
    }

    for child in &layout_box.children {
        record_box(child, dl, offset_x, offset_y, ctx, link_map);
    }
}

pub(super) fn record_text(
    text: &str,
    x: f32,
    y: f32,
    style: &crate::css::values::ComputedStyle,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    // Skip empty runs (left when `trim_line_boundary_spaces` strips an
    // inter-element space at a line edge) and bare newline runs, which
    // whitespace-only DOM text between inline siblings can carry into
    // paint: they have no glyph, and backends draw "\n" as tofu.
    if text.is_empty() || text.chars().all(|c| c == '\n' || c == '\r') {
        return;
    }
    let sx = (x - ctx.scroll_x + offset_x as f32) as i32;
    let sy = (y - ctx.scroll_y + offset_y as f32) as i32;

    // Opacity is handled by PushLayer/PopLayer during replay — use 1.0 here
    // to avoid doubling opacity (once in recording, once during replay).
    // `filter` is applied by the compositing layer at replay.
    let color = style.color;
    let bold = style.font_weight.is_bold();
    let italic = style.font_style == crate::css::values::FontStyle::Italic;
    let font_size = style.font_size as u16;

    // text-overflow: ellipsis handling.
    let display_text: std::borrow::Cow<'_, str>;
    if ctx.text_overflow_ellipsis {
        if let Some(clip) = &ctx.clip_rect {
            let avail = (clip.x + clip.width - x).max(0.0) as u32;
            let text_w = oasis_types::backend::bitmap_measure_text(text, font_size);
            if text_w > avail {
                let ellipsis = "\u{2026}";
                let ew = oasis_types::backend::bitmap_measure_text(ellipsis, font_size);
                let target = avail.saturating_sub(ew);
                let mut accum = 0u32;
                let mut cut = 0;
                for (i, ch) in text.char_indices() {
                    let cw = oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size);
                    if accum + cw > target {
                        cut = i;
                        break;
                    }
                    accum += cw;
                    cut = i + ch.len_utf8();
                }
                let mut truncated = text[..cut].to_string();
                truncated.push_str(ellipsis);
                display_text = std::borrow::Cow::Owned(truncated);
            } else {
                display_text = std::borrow::Cow::Borrowed(text);
            }
        } else {
            display_text = std::borrow::Cow::Borrowed(text);
        }
    } else {
        display_text = std::borrow::Cow::Borrowed(text);
    }

    // Pre-compute text width including letter-spacing for bounds and decorations.
    let mut text_w = oasis_types::backend::bitmap_measure_text(&display_text, font_size) as f32;
    if style.letter_spacing != 0.0 {
        let chars = display_text.chars().count();
        if chars > 1 {
            text_w += style.letter_spacing * (chars - 1) as f32;
        }
    }
    let text_width = text_w.max(0.0) as u32;

    // Resolve web font ID from the computed style.
    #[cfg(feature = "web-fonts")]
    let wf_id = style.web_font_id;

    // Text shadow.
    if let Some(ref shadow) = style.text_shadow {
        let shadow_color = apply_opacity(shadow.color, 1.0);
        dl.push(DisplayItem::DrawText {
            text: display_text.to_string(),
            x: sx + shadow.offset_x as i32,
            y: sy + shadow.offset_y as i32,
            font_size,
            color: shadow_color,
            bold,
            italic,
            width: text_width,
            node_id: ctx.current_node,
            #[cfg(feature = "web-fonts")]
            web_font_id: wf_id,
        });
    }

    dl.push(DisplayItem::DrawText {
        text: display_text.to_string(),
        x: sx,
        y: sy,
        font_size,
        color,
        bold,
        italic,
        width: text_width,
        node_id: ctx.current_node,
        #[cfg(feature = "web-fonts")]
        web_font_id: wf_id,
    });

    if !style.text_decoration.line.is_none() {
        let deco_color = style.text_decoration.color.unwrap_or(color);
        let positions: &[(bool, i32)] = &[
            (
                style.text_decoration.line.has_underline(),
                sy + (style.font_size * 0.85) as i32 + style.text_underline_offset as i32,
            ),
            (
                style.text_decoration.line.has_line_through(),
                sy + (style.font_size * 0.4) as i32,
            ),
            (style.text_decoration.line.has_overline(), sy),
        ];
        for &(active, deco_y) in positions {
            if active {
                let style = style.text_decoration.style;
                record_decoration_line(dl, sx, deco_y, text_width, deco_color, style, ctx);
            }
        }
    }
}

/// Record one text-decoration line (`text-decoration-style`) of
/// `width` pixels starting at `(x, y)`.
pub(super) fn record_decoration_line(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    width: u32,
    color: Color,
    style: TextDecorationStyle,
    ctx: &RecordContext,
) {
    let mut seg = |dx: u32, dy: i32, w: u32| {
        dl.push(DisplayItem::FillRect {
            x: x + dx as i32,
            y: y + dy,
            w,
            h: 1,
            color,
            node_id: ctx.current_node,
        });
    };
    match style {
        TextDecorationStyle::Solid => seg(0, 0, width),
        TextDecorationStyle::Double => {
            seg(0, 0, width);
            seg(0, 2, width);
        },
        TextDecorationStyle::Dashed => {
            // 4 px dash, 4 px gap.
            let mut pos = 0;
            while pos < width {
                seg(pos, 0, 4.min(width - pos));
                pos += 8;
            }
        },
        TextDecorationStyle::Dotted => {
            let mut pos = 0;
            while pos < width {
                seg(pos, 0, 1);
                pos += 2;
            }
        },
        TextDecorationStyle::Wavy => {
            // 2 px runs alternating between the line and one pixel below.
            let mut pos = 0;
            while pos < width {
                let dy = i32::from(!(pos / 2).is_multiple_of(2));
                seg(pos, dy, 2.min(width - pos));
                pos += 2;
            }
        },
    }
}

pub(super) fn record_list_marker(
    marker: &crate::layout::box_model::ListMarker,
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    use crate::layout::box_model::ListMarker;

    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;
    let font_size = layout_box.style.font_size as u16;
    let color = layout_box.style.color;

    let text = match marker {
        ListMarker::Disc => "\u{2022}".to_string(),
        ListMarker::Circle => "\u{25CB}".to_string(),
        ListMarker::Square => "\u{25A0}".to_string(),
        ListMarker::Ordered(style, n) => {
            crate::paint::markers::format_ordered_marker(style, *n, &ctx.counter_styles)
        },
        ListMarker::None => return,
    };

    let marker_w = oasis_types::backend::bitmap_measure_text(&text, font_size);
    dl.push(DisplayItem::DrawText {
        text,
        x: x - marker_w as i32 - 4,
        y,
        font_size,
        color,
        bold: false,
        italic: false,
        width: marker_w,
        node_id: ctx.current_node,
        #[cfg(feature = "web-fonts")]
        web_font_id: None,
    });
}
