//! Replaced element painting: images, `<hr>`, `<input>`, `<select>`, `<button>`.

use crate::css::values::{BorderStyle, ObjectFit};
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
            texture: Some(tex),
            width: img_w,
            height: img_h,
            atlas_region,
            ..
        } => {
            let box_w = content.width as u32;
            let box_h = content.height as u32;
            let (blit_x, blit_y, blit_w, blit_h) = compute_object_fit(
                layout_box.style.object_fit,
                *img_w,
                *img_h,
                box_w,
                box_h,
                x,
                y,
            );
            if let Some(ar) = atlas_region {
                backend.blit_sub(*tex, ar.x, ar.y, ar.w, ar.h, blit_x, blit_y, blit_w, blit_h)?;
            } else {
                backend.blit(*tex, blit_x, blit_y, blit_w, blit_h)?;
            }
        },
        ReplacedContent::Image { alt, .. } => {
            // Broken image placeholder: thin border + alt text or X.
            //
            // Skip drawing entirely when the box has zero content
            // dimensions — this is the common case for sprite elements
            // whose `width`/`height` are only supplied via CSS and the
            // `src` failed to load. Drawing the `×` glyph at (x, y) in
            // that case leaves a stray marker floating around the page.
            if content.width <= 0.0 || content.height <= 0.0 {
                return Ok(());
            }
            // Paint the broken-image frame + alt text, clipped to the
            // box so long `alt=""` values don't bleed into siblings.
            //
            // The frame uses the element's `color`. The label font size
            // follows the element's computed `font-size` (clamped to
            // what fits inside the box vertically) rather than a hard-
            // coded 8px, so tiny sprites don't overflow and large
            // banners get proportionally-sized placeholders.
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
            // Alt text or multiplication sign.
            //
            // Treat `×` and real alt text differently. `×` is an icon —
            // keep it small (8px) and draw it in the corner where real
            // browsers put the broken-image chip. Alt text is intended
            // to be read, so scale it to the element's computed font-
            // size and clip to the box so long values don't bleed into
            // siblings. The interior-height cap prevents a large
            // inherited font-size (e.g. Wikipedia's 3rem logo wrapper)
            // from being drawn at 30px inside a 22x22 sprite.
            let interior_h = h.saturating_sub(4).max(6) as u16;
            backend.set_clip_rect(x, y, w, h)?;
            if alt.is_empty() {
                backend.draw_text("\u{00D7}", x + 2, y + 2, 8u16.min(interior_h), color)?;
            } else {
                let style_fs = layout_box.style.font_size.round().max(6.0) as u16;
                let font_size = style_fs.min(interior_h);
                backend.draw_text(alt, x + 2, y + 2, font_size, color)?;
            }
            backend.reset_clip_rect()?;
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
            value,
            placeholder,
            is_password,
            ..
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
            if !style.border_radius.is_zero() {
                backend.fill_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    style.border_radius.max_radius() as u16,
                    bg,
                )?;
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
                let display_text = if *is_password {
                    "\u{25CF}".repeat(value.chars().count())
                } else {
                    value.clone()
                };
                backend.draw_text(&display_text, x + pad, y + pad_top, font_size, style.color)?;
            } else if !placeholder.is_empty() {
                let gray = Color::rgb(160, 160, 160);
                backend.draw_text(placeholder, x + pad, y + pad_top, font_size, gray)?;
            }
        },
        ReplacedContent::Checkbox { checked } => {
            let w = content.width as u32;
            let h = content.height as u32;
            // White background
            backend.fill_rect(x, y, w, h, Color::rgb(255, 255, 255))?;
            // Border
            let border_color = Color::rgb(118, 118, 118);
            backend.fill_rect(x, y, w, 1, border_color)?;
            backend.fill_rect(x, y + h as i32 - 1, w, 1, border_color)?;
            backend.fill_rect(x, y, 1, h, border_color)?;
            backend.fill_rect(x + w as i32 - 1, y, 1, h, border_color)?;
            // Checkmark when checked
            if *checked && w >= 5 && h >= 5 {
                let ck = Color::rgb(0, 0, 0);
                // Short leg: going down-right
                backend.fill_rect(x + 2, y + h as i32 - 5, 1, 1, ck)?;
                backend.fill_rect(x + 3, y + h as i32 - 4, 1, 1, ck)?;
                backend.fill_rect(x + 4, y + h as i32 - 3, 1, 1, ck)?;
                // Long leg: going up-right
                backend.fill_rect(x + 5, y + h as i32 - 4, 1, 1, ck)?;
                backend.fill_rect(x + 6, y + h as i32 - 5, 1, 1, ck)?;
                backend.fill_rect(x + 7, y + h as i32 - 6, 1, 1, ck)?;
                backend.fill_rect(x + 8, y + h as i32 - 7, 1, 1, ck)?;
                backend.fill_rect(x + 9, y + h as i32 - 8, 1, 1, ck)?;
                backend.fill_rect(x + 10, y + h as i32 - 9, 1, 1, ck)?;
            }
        },
        ReplacedContent::RadioButton { checked } => {
            let w = content.width as u32;
            let h = content.height as u32;
            // Outer circle approximation using rounded rect
            let radius = w.min(h) as u16 / 2;
            backend.fill_rounded_rect(x, y, w, h, radius, Color::rgb(255, 255, 255))?;
            // Border edges for circle approximation
            let bc = Color::rgb(118, 118, 118);
            backend.fill_rect(x + 2, y, w - 4, 1, bc)?;
            backend.fill_rect(x + 2, y + h as i32 - 1, w - 4, 1, bc)?;
            backend.fill_rect(x, y + 2, 1, h - 4, bc)?;
            backend.fill_rect(x + w as i32 - 1, y + 2, 1, h - 4, bc)?;
            backend.fill_rect(x + 1, y + 1, 1, 1, bc)?;
            backend.fill_rect(x + w as i32 - 2, y + 1, 1, 1, bc)?;
            backend.fill_rect(x + 1, y + h as i32 - 2, 1, 1, bc)?;
            backend.fill_rect(x + w as i32 - 2, y + h as i32 - 2, 1, 1, bc)?;
            // Inner filled dot when checked
            if *checked && w >= 7 && h >= 7 {
                let dot = Color::rgb(0, 0, 0);
                let inset = 4_i32;
                let dw = w as i32 - inset * 2;
                let dh = h as i32 - inset * 2;
                if dw > 0 && dh > 0 {
                    backend.fill_rect(x + inset, y + inset, dw as u32, dh as u32, dot)?;
                }
            }
        },
        ReplacedContent::TextArea {
            value, placeholder, ..
        } => {
            let style = &layout_box.style;
            let w = content.width as u32;
            let h = content.height as u32;
            // Background
            let bg = if style.background_color.a > 0 {
                style.background_color
            } else {
                Color::rgb(255, 255, 255)
            };
            backend.fill_rect(x, y, w, h, bg)?;
            // Border: 3D inset
            let dark = Color::rgb(118, 118, 118);
            let light = Color::rgb(200, 200, 200);
            backend.fill_rect(x, y, w, 1, dark)?;
            backend.fill_rect(x, y, 1, h, dark)?;
            backend.fill_rect(x, y + h as i32 - 1, w, 1, light)?;
            backend.fill_rect(x + w as i32 - 1, y, 1, h, light)?;
            // Text content
            let font_size = style.font_size as u16;
            let pad = 3_i32;
            let line_height = font_size as i32 + 2;
            let (text, color) = if !value.is_empty() {
                (value.as_str(), style.color)
            } else if !placeholder.is_empty() {
                (placeholder.as_str(), Color::rgb(160, 160, 160))
            } else {
                ("", style.color)
            };
            if !text.is_empty() {
                for (i, line) in text.lines().enumerate() {
                    let ly = y + pad + i as i32 * line_height;
                    if ly > y + h as i32 {
                        break;
                    }
                    backend.draw_text(line, x + pad, ly, font_size, color)?;
                }
            }
        },
        ReplacedContent::SelectBox {
            label,
            open,
            options,
            selected_index,
        } => {
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
            if *open && !options.is_empty() {
                let line_h = font_size as i32 + 4;
                let dropdown_h = options.len() as u32 * line_h as u32;
                let dy = y + h as i32;
                backend.fill_rect(x, dy, w, dropdown_h, Color::rgb(255, 255, 255))?;
                backend.fill_rect(x, dy, w, 1, border_color)?;
                backend.fill_rect(x, dy + dropdown_h as i32 - 1, w, 1, border_color)?;
                backend.fill_rect(x, dy, 1, dropdown_h, border_color)?;
                backend.fill_rect(x + w as i32 - 1, dy, 1, dropdown_h, border_color)?;
                for (i, opt_label) in options.iter().enumerate() {
                    let oy = dy + i as i32 * line_h;
                    let is_selected = *selected_index == Some(i);
                    if is_selected {
                        backend.fill_rect(
                            x + 1,
                            oy,
                            w.saturating_sub(2),
                            line_h as u32,
                            Color::rgb(51, 122, 183),
                        )?;
                        backend.draw_text(
                            opt_label,
                            x + 3,
                            oy + 2,
                            font_size,
                            Color::rgb(255, 255, 255),
                        )?;
                    } else {
                        backend.draw_text(opt_label, x + 3, oy + 2, font_size, text_color)?;
                    }
                }
            }
        },
        ReplacedContent::Svg { element } => {
            crate::svg::paint_svg(element, backend, x, y, content.width, content.height)?;
        },
        ReplacedContent::Canvas { state } => {
            let s = state.borrow();
            crate::canvas::paint_canvas(
                &s,
                backend,
                x,
                y,
                content.width as u32,
                content.height as u32,
            )?;
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
            if !style.border_radius.is_zero() {
                backend.fill_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    style.border_radius.max_radius() as u16,
                    bg,
                )?;
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
