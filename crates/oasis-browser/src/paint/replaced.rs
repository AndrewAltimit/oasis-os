//! Replaced element painting: images, `<hr>`, `<input>`, `<select>`, `<button>`.

use crate::css::values::BorderStyle;
use crate::layout::box_model::{LayoutBox, ReplacedContent};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use super::PaintContext;
use super::borders::paint_border_edge;

pub(super) fn paint_replaced(
    replaced: &ReplacedContent,
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;

    match replaced {
        ReplacedContent::Image {
            texture: Some(tex), ..
        } => {
            backend.blit(*tex, x, y, content.width as u32, content.height as u32)?;
        },
        ReplacedContent::Image { alt, .. } => {
            // Broken image placeholder: thin border + alt text or X.
            let w = content.width.max(16.0) as u32;
            let h = content.height.max(16.0) as u32;
            let color = layout_box.style.color;
            // Top edge
            backend.fill_rect(x, y, w, 1, color)?;
            // Bottom edge
            backend.fill_rect(x, y + h as i32 - 1, w, 1, color)?;
            // Left edge
            backend.fill_rect(x, y, 1, h, color)?;
            // Right edge
            backend.fill_rect(x + w as i32 - 1, y, 1, h, color)?;
            // Alt text or multiplication sign
            let label = if alt.is_empty() { "\u{00D7}" } else { alt };
            backend.draw_text(label, x + 2, y + 2, 8, color)?;
        },
        ReplacedContent::HorizontalRule => {
            let style = &layout_box.style;
            let w = content.width as u32;
            if style.border_top_style != BorderStyle::None && style.border_top_width > 0.0 {
                // Use CSS border-top properties.
                paint_border_edge(
                    backend,
                    x,
                    y,
                    w,
                    style.border_top_width as u32,
                    style.border_top_color,
                    style.border_top_style,
                    true,
                )?;
            } else {
                // Fallback: 1px solid gray.
                backend.fill_rect(x, y, w, 1, Color::rgb(128, 128, 128))?;
            }
        },
        ReplacedContent::LineBreak => {
            // Nothing to paint.
        },
        ReplacedContent::TextInput {
            value, placeholder, ..
        } => {
            let style = &layout_box.style;
            let w = content.width as u32;
            let h = content.height as u32;
            // Background: use CSS background-color, fallback to white.
            let bg = if style.background_color.a > 0 {
                style.background_color
            } else {
                Color::rgb(255, 255, 255)
            };
            if style.border_radius > 0.0 {
                backend.fill_rounded_rect(x, y, w, h, style.border_radius as u16, bg)?;
            } else {
                backend.fill_rect(x, y, w, h, bg)?;
            }
            // Border: use CSS border properties, or default 3D inset.
            let has_css_border = style.border_top_style != BorderStyle::None;
            if has_css_border {
                let bw = style.border_top_width.max(1.0) as u32;
                let bc = style.border_top_color;
                backend.fill_rect(x, y, w, bw, bc)?;
                let bc_b = style.border_bottom_color;
                backend.fill_rect(x, y + h as i32 - bw as i32, w, bw, bc_b)?;
                let bc_l = style.border_left_color;
                backend.fill_rect(x, y, bw, h, bc_l)?;
                let bc_r = style.border_right_color;
                backend.fill_rect(x + w as i32 - bw as i32, y, bw, h, bc_r)?;
            } else {
                // 3D inset appearance: dark top/left, light bottom/right.
                let dark = Color::rgb(118, 118, 118);
                let light = Color::rgb(200, 200, 200);
                backend.fill_rect(x, y, w, 1, dark)?;
                backend.fill_rect(x, y, 1, h, dark)?;
                backend.fill_rect(x, y + h as i32 - 1, w, 1, light)?;
                backend.fill_rect(x + w as i32 - 1, y, 1, h, light)?;
            }
            let font_size = style.font_size as u16;
            let pad = style.padding_left.max(3.0) as i32;
            let pad_top = ((h as i32 - font_size as i32) / 2).max(1);
            // Show value text, or placeholder if empty.
            if !value.is_empty() {
                backend.draw_text(value, x + pad, y + pad_top, font_size, style.color)?;
            } else if !placeholder.is_empty() {
                let gray = Color::rgb(160, 160, 160);
                backend.draw_text(placeholder, x + pad, y + pad_top, font_size, gray)?;
            }
        },
        ReplacedContent::SelectBox { label } => {
            let style = &layout_box.style;
            let w = content.width as u32;
            let h = content.height as u32;
            // White background with border.
            let bg = if style.background_color.a > 0 {
                style.background_color
            } else {
                Color::rgb(255, 255, 255)
            };
            backend.fill_rect(x, y, w, h, bg)?;
            // Border
            let border_color = Color::rgb(118, 118, 118);
            backend.fill_rect(x, y, w, 1, border_color)?;
            backend.fill_rect(x, y + h as i32 - 1, w, 1, border_color)?;
            backend.fill_rect(x, y, 1, h, border_color)?;
            backend.fill_rect(x + w as i32 - 1, y, 1, h, border_color)?;
            // Label text
            let font_size = style.font_size as u16;
            let text_color = style.color;
            let pad_top = ((h as i32 - font_size as i32) / 2).max(1);
            backend.draw_text(label, x + 3, y + pad_top, font_size, text_color)?;
            // Dropdown arrow "v" on the right
            let arrow_x = x + w as i32 - 10;
            backend.draw_text("v", arrow_x, y + pad_top, font_size, text_color)?;
        },
        ReplacedContent::SubmitButton { label } => {
            let style = &layout_box.style;
            let w = content.width as u32;
            let h = content.height as u32;
            // Button background: use CSS background-color, fallback light gray.
            let bg = if style.background_color.a > 0 {
                style.background_color
            } else {
                Color::rgb(239, 239, 239)
            };
            if style.border_radius > 0.0 {
                backend.fill_rounded_rect(x, y, w, h, style.border_radius as u16, bg)?;
            } else {
                backend.fill_rect(x, y, w, h, bg)?;
            }
            // Border: use CSS border properties, or default 3D raised.
            let has_css_border = style.border_top_style != BorderStyle::None;
            if has_css_border {
                let bw = style.border_top_width.max(1.0) as u32;
                let bc = style.border_top_color;
                backend.fill_rect(x, y, w, bw, bc)?;
                let bc_b = style.border_bottom_color;
                backend.fill_rect(x, y + h as i32 - bw as i32, w, bw, bc_b)?;
                let bc_l = style.border_left_color;
                backend.fill_rect(x, y, bw, h, bc_l)?;
                let bc_r = style.border_right_color;
                backend.fill_rect(x + w as i32 - bw as i32, y, bw, h, bc_r)?;
            } else {
                // 3D raised appearance: light top/left, dark bottom/right.
                let light = Color::rgb(255, 255, 255);
                let dark = Color::rgb(160, 160, 160);
                backend.fill_rect(x, y, w, 1, light)?;
                backend.fill_rect(x, y, 1, h, light)?;
                backend.fill_rect(x, y + h as i32 - 1, w, 1, dark)?;
                backend.fill_rect(x + w as i32 - 1, y, 1, h, dark)?;
            }
            // Label text centered using bitmap measurement.
            let font_size = style.font_size as u16;
            let text_color = style.color;
            let text_w = oasis_types::backend::bitmap_measure_text(label, font_size);
            let text_x = x + (w as i32 - text_w as i32) / 2;
            let text_y = y + (h as i32 - font_size as i32) / 2;
            backend.draw_text(label, text_x, text_y, font_size, text_color)?;
        },
    }

    Ok(())
}
