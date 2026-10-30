//! Form controls: text inputs, checkboxes, radios, textareas, selects
//! and buttons.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn record_text_input(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    value: &str,
    placeholder: &str,
    is_password: bool,
    focused: bool,
) {
    let style = &layout_box.style;
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    let bg = if style.background_color.a > 0 {
        style.background_color
    } else {
        Color::rgb(255, 255, 255)
    };
    if !style.border_radius.is_zero() {
        dl.push(DisplayItem::FillRoundedRect {
            x,
            y,
            w,
            h,
            radius: style.border_radius.max_radius() as u16,
            color: bg,
            node_id: None,
        });
    } else {
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h,
            color: bg,
            node_id: None,
        });
    }
    let has_css_border = style.border_top_style != BorderStyle::None;
    if has_css_border {
        let bw = style.border_top_width.max(1.0) as u32;
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h: bw,
            color: style.border_top_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: y + h as i32 - bw as i32,
            w,
            h: bw,
            color: style.border_bottom_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w: bw,
            h,
            color: style.border_left_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: x + w as i32 - bw as i32,
            y,
            w: bw,
            h,
            color: style.border_right_color,
            node_id: None,
        });
    } else {
        let dark = Color::rgb(118, 118, 118);
        let light = Color::rgb(200, 200, 200);
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h: 1,
            color: dark,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w: 1,
            h,
            color: dark,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: y + h as i32 - 1,
            w,
            h: 1,
            color: light,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: x + w as i32 - 1,
            y,
            w: 1,
            h,
            color: light,
            node_id: None,
        });
    }
    let font_size = style.font_size as u16;
    let pad = style.padding_left.max(3.0) as i32;
    let pad_top = ((h as i32 - font_size as i32) / 2).max(1);
    // Width of the rendered value text, used both to draw the text and
    // to position the caret immediately after it.
    let mut value_text_width: u32 = 0;
    if !value.is_empty() {
        let display_text = if is_password {
            "\u{25CF}".repeat(value.chars().count())
        } else {
            value.to_string()
        };
        let value_w = oasis_types::backend::bitmap_measure_text(&display_text, font_size);
        value_text_width = value_w;
        dl.push(DisplayItem::DrawText {
            text: display_text,
            x: x + pad,
            y: y + pad_top,
            font_size,
            color: style.color,
            bold: false,
            italic: false,
            width: value_w,
            node_id: None,
            #[cfg(feature = "web-fonts")]
            web_font_id: None,
        });
    } else if !placeholder.is_empty() {
        let ph_w = oasis_types::backend::bitmap_measure_text(placeholder, font_size);
        dl.push(DisplayItem::DrawText {
            text: placeholder.to_string(),
            x: x + pad,
            y: y + pad_top,
            font_size,
            color: Color::rgb(160, 160, 160),
            bold: false,
            italic: false,
            width: ph_w,
            node_id: None,
            #[cfg(feature = "web-fonts")]
            web_font_id: None,
        });
    }
    // Draw a one-pixel caret after the value when the field has focus.
    // `caret-color` overrides the text color; default to it.
    if focused {
        let caret_color = style.caret_color.unwrap_or(style.color);
        let caret_x = x + pad + value_text_width as i32;
        let caret_h = font_size as u32 + 2;
        let caret_y = y + pad_top - 1;
        // Clip the caret inside the visible content box.
        if caret_x < x + w as i32 - 1 {
            dl.push(DisplayItem::FillRect {
                x: caret_x,
                y: caret_y,
                w: 1,
                h: caret_h,
                color: caret_color,
                node_id: None,
            });
        }
    }
}

pub(super) fn record_checkbox(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    checked: bool,
) {
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    let accent = layout_box.style.accent_color;
    // Background: when checked and the author supplied `accent-color`,
    // tint the box with the accent like Blink does. Otherwise keep the
    // plain white box.
    let bg = match (checked, accent) {
        (true, Some(c)) => c,
        _ => Color::rgb(255, 255, 255),
    };
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h,
        color: bg,
        node_id: None,
    });
    // Border
    let bc = Color::rgb(118, 118, 118);
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h: 1,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y: y + h as i32 - 1,
        w,
        h: 1,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w: 1,
        h,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x: x + w as i32 - 1,
        y,
        w: 1,
        h,
        color: bc,
        node_id: None,
    });
    // Checkmark when checked. If the box is filled with `accent-color`
    // we draw the check in white for contrast; otherwise the check
    // takes the accent color (or black as the spec default).
    if checked && w >= 5 && h >= 5 {
        let ck = if accent.is_some() {
            Color::rgb(255, 255, 255)
        } else {
            Color::rgb(0, 0, 0)
        };
        // Short leg of checkmark
        for &(dx, dy) in &[(2, -5), (3, -4), (4, -3)] {
            dl.push(DisplayItem::FillRect {
                x: x + dx,
                y: y + h as i32 + dy,
                w: 1,
                h: 1,
                color: ck,
                node_id: None,
            });
        }
        // Long leg of checkmark
        for &(dx, dy) in &[(5, -4), (6, -5), (7, -6), (8, -7), (9, -8), (10, -9)] {
            dl.push(DisplayItem::FillRect {
                x: x + dx,
                y: y + h as i32 + dy,
                w: 1,
                h: 1,
                color: ck,
                node_id: None,
            });
        }
    }
}

pub(super) fn record_radio_button(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    checked: bool,
) {
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    // White background (using rounded rect for circle)
    let radius = w.min(h) as u16 / 2;
    dl.push(DisplayItem::FillRoundedRect {
        x,
        y,
        w,
        h,
        radius,
        color: Color::rgb(255, 255, 255),
        node_id: None,
    });
    // Border edges for circle approximation
    let bc = Color::rgb(118, 118, 118);
    dl.push(DisplayItem::FillRect {
        x: x + 2,
        y,
        w: w - 4,
        h: 1,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x: x + 2,
        y: y + h as i32 - 1,
        w: w - 4,
        h: 1,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y: y + 2,
        w: 1,
        h: h - 4,
        color: bc,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x: x + w as i32 - 1,
        y: y + 2,
        w: 1,
        h: h - 4,
        color: bc,
        node_id: None,
    });
    // Corner pixels
    for &(dx, dy) in &[
        (1, 1),
        (w as i32 - 2, 1),
        (1, h as i32 - 2),
        (w as i32 - 2, h as i32 - 2),
    ] {
        dl.push(DisplayItem::FillRect {
            x: x + dx,
            y: y + dy,
            w: 1,
            h: 1,
            color: bc,
            node_id: None,
        });
    }
    // Inner dot when checked. Take `accent-color` if the author set it,
    // otherwise fall back to plain black.
    if checked && w > 8 && h > 8 {
        let inset = 4_u32;
        let dw = w - inset * 2;
        let dh = h - inset * 2;
        let dot = layout_box.style.accent_color.unwrap_or(Color::rgb(0, 0, 0));
        if dw > 0 && dh > 0 {
            dl.push(DisplayItem::FillRect {
                x: x + inset as i32,
                y: y + inset as i32,
                w: dw,
                h: dh,
                color: dot,
                node_id: None,
            });
        }
    }
}

pub(super) fn record_textarea(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    value: &str,
    placeholder: &str,
    focused: bool,
) {
    let style = &layout_box.style;
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    // Background
    let bg = if style.background_color.a > 0 {
        style.background_color
    } else {
        Color::rgb(255, 255, 255)
    };
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h,
        color: bg,
        node_id: None,
    });
    // Border: 3D inset
    let dark = Color::rgb(118, 118, 118);
    let light = Color::rgb(200, 200, 200);
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h: 1,
        color: dark,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w: 1,
        h,
        color: dark,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y: y + h as i32 - 1,
        w,
        h: 1,
        color: light,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x: x + w as i32 - 1,
        y,
        w: 1,
        h,
        color: light,
        node_id: None,
    });
    // Text content
    let font_size = style.font_size as u16;
    let pad = 3_i32;
    let line_height = font_size as i32 + 2;
    let (text, color) = if !value.is_empty() {
        (value, style.color)
    } else if !placeholder.is_empty() {
        (placeholder, Color::rgb(160, 160, 160))
    } else {
        ("", style.color)
    };
    let mut last_line_index: i32 = 0;
    let mut last_line_width: u32 = 0;
    if !text.is_empty() {
        // Use `split('\n')` rather than `lines()` so a trailing
        // newline produces a final empty entry. That matters for the
        // caret: typing "abc\n" should put the caret on the empty
        // next line, not at the end of "abc".
        for (i, line) in text.split('\n').enumerate() {
            let ly = y + pad + i as i32 * line_height;
            if ly > y + h as i32 {
                break;
            }
            let lw = oasis_types::backend::bitmap_measure_text(line, font_size);
            // Skip the empty trailing draw call (no text to render),
            // but still record the index/width so the caret lands on
            // the empty line.
            if !line.is_empty() {
                dl.push(DisplayItem::DrawText {
                    text: line.to_string(),
                    x: x + pad,
                    y: ly,
                    font_size,
                    color,
                    bold: false,
                    italic: false,
                    width: lw,
                    node_id: None,
                    #[cfg(feature = "web-fonts")]
                    web_font_id: None,
                });
            }
            last_line_index = i as i32;
            last_line_width = lw;
        }
    }
    // Caret on the last visible line, after the last character.
    if focused {
        let caret_color = style.caret_color.unwrap_or(style.color);
        // If the value is empty (we drew the placeholder), put the
        // caret at the top-left of the field instead of after the
        // placeholder text.
        let (caret_x, caret_y) = if value.is_empty() {
            (x + pad, y + pad)
        } else {
            (
                x + pad + last_line_width as i32,
                y + pad + last_line_index * line_height,
            )
        };
        let caret_h = font_size as u32 + 2;
        if caret_x < x + w as i32 - 1 && caret_y + caret_h as i32 <= y + h as i32 {
            dl.push(DisplayItem::FillRect {
                x: caret_x,
                y: caret_y,
                w: 1,
                h: caret_h,
                color: caret_color,
                node_id: None,
            });
        }
    }
}

pub(super) fn record_select_box(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    label: &str,
    open: bool,
    options: &[String],
    selected_index: Option<usize>,
) {
    let style = &layout_box.style;
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    let bg = if style.background_color.a > 0 {
        style.background_color
    } else {
        Color::rgb(255, 255, 255)
    };
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h,
        color: bg,
        node_id: None,
    });
    let border_color = Color::rgb(118, 118, 118);
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w,
        h: 1,
        color: border_color,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y: y + h as i32 - 1,
        w,
        h: 1,
        color: border_color,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x,
        y,
        w: 1,
        h,
        color: border_color,
        node_id: None,
    });
    dl.push(DisplayItem::FillRect {
        x: x + w as i32 - 1,
        y,
        w: 1,
        h,
        color: border_color,
        node_id: None,
    });
    let font_size = style.font_size as u16;
    let pad_top = ((h as i32 - font_size as i32) / 2).max(1);
    let label_w = oasis_types::backend::bitmap_measure_text(label, font_size);
    dl.push(DisplayItem::DrawText {
        text: label.to_string(),
        x: x + 3,
        y: y + pad_top,
        font_size,
        color: style.color,
        bold: false,
        italic: false,
        width: label_w,
        node_id: None,
        #[cfg(feature = "web-fonts")]
        web_font_id: None,
    });
    let arrow_w = oasis_types::backend::bitmap_measure_text("v", font_size);
    dl.push(DisplayItem::DrawText {
        text: "v".to_string(),
        x: x + w as i32 - 10,
        y: y + pad_top,
        font_size,
        color: style.color,
        bold: false,
        italic: false,
        width: arrow_w,
        node_id: None,
        #[cfg(feature = "web-fonts")]
        web_font_id: None,
    });
    if open && !options.is_empty() {
        let line_h = font_size as u32 + 4;
        let dropdown_h = options.len() as u32 * line_h;
        let dy = y + h as i32;
        dl.push(DisplayItem::FillRect {
            x,
            y: dy,
            w,
            h: dropdown_h,
            color: Color::rgb(255, 255, 255),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: dy,
            w,
            h: 1,
            color: border_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: dy + dropdown_h as i32 - 1,
            w,
            h: 1,
            color: border_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: dy,
            w: 1,
            h: dropdown_h,
            color: border_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: x + w as i32 - 1,
            y: dy,
            w: 1,
            h: dropdown_h,
            color: border_color,
            node_id: None,
        });
        for (i, opt_label) in options.iter().enumerate() {
            let oy = dy + i as i32 * line_h as i32;
            let is_selected = selected_index == Some(i);
            if is_selected {
                dl.push(DisplayItem::FillRect {
                    x: x + 1,
                    y: oy,
                    w: w.saturating_sub(2),
                    h: line_h,
                    color: Color::rgb(51, 122, 183),
                    node_id: None,
                });
            }
            let text_color = if is_selected {
                Color::rgb(255, 255, 255)
            } else {
                style.color
            };
            let opt_w = oasis_types::backend::bitmap_measure_text(opt_label, font_size);
            dl.push(DisplayItem::DrawText {
                text: opt_label.clone(),
                x: x + 3,
                y: oy + 2,
                font_size,
                color: text_color,
                bold: false,
                italic: false,
                width: opt_w,
                node_id: None,
                #[cfg(feature = "web-fonts")]
                web_font_id: None,
            });
        }
    }
}

pub(super) fn record_submit_button(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    label: &str,
) {
    let style = &layout_box.style;
    let w = layout_box.dimensions.content.width as u32;
    let h = layout_box.dimensions.content.height as u32;
    let bg = if style.background_color.a > 0 {
        style.background_color
    } else {
        Color::rgb(239, 239, 239)
    };
    if !style.border_radius.is_zero() {
        dl.push(DisplayItem::FillRoundedRect {
            x,
            y,
            w,
            h,
            radius: style.border_radius.max_radius() as u16,
            color: bg,
            node_id: None,
        });
    } else {
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h,
            color: bg,
            node_id: None,
        });
    }
    let has_css_border = style.border_top_style != BorderStyle::None;
    if has_css_border {
        let bw = style.border_top_width.max(1.0) as u32;
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h: bw,
            color: style.border_top_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: y + h as i32 - bw as i32,
            w,
            h: bw,
            color: style.border_bottom_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w: bw,
            h,
            color: style.border_left_color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: x + w as i32 - bw as i32,
            y,
            w: bw,
            h,
            color: style.border_right_color,
            node_id: None,
        });
    } else {
        let light = Color::rgb(255, 255, 255);
        let dark = Color::rgb(160, 160, 160);
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w,
            h: 1,
            color: light,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y,
            w: 1,
            h,
            color: light,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x,
            y: y + h as i32 - 1,
            w,
            h: 1,
            color: dark,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: x + w as i32 - 1,
            y,
            w: 1,
            h,
            color: dark,
            node_id: None,
        });
    }
    let font_size = style.font_size as u16;
    let text_w = oasis_types::backend::bitmap_measure_text(label, font_size);
    let text_x = x + (w as i32 - text_w as i32) / 2;
    let text_y = y + (h as i32 - font_size as i32) / 2;
    dl.push(DisplayItem::DrawText {
        text: label.to_string(),
        x: text_x,
        y: text_y,
        font_size,
        color: style.color,
        bold: false,
        italic: false,
        width: text_w,
        node_id: None,
        #[cfg(feature = "web-fonts")]
        web_font_id: None,
    });
}
