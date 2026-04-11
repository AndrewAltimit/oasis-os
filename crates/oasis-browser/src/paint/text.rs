//! Inline content and text run painting.

use std::collections::HashMap;

use crate::css::values::TextDecorationStyle;
use crate::html::dom::NodeId;
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use super::{PaintContext, apply_filters_and_opacity, apply_opacity, paint_box};

pub(super) fn paint_inline_content(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut PaintContext,
    link_map: &HashMap<NodeId, String>,
) -> Result<()> {
    // Paint inline background if non-transparent.
    let bg = layout_box.style.background_color;
    if bg.a > 0 {
        let pb = layout_box.dimensions.padding_box();
        let x = (pb.x - ctx.scroll_x + offset_x as f32) as i32;
        let y = (pb.y - ctx.scroll_y + offset_y as f32) as i32;
        let w = pb.width as u32;
        let h = pb.height as u32;
        if !layout_box.style.border_radius.is_zero() {
            let r = layout_box.style.border_radius.max_radius() as u16;
            backend.fill_rounded_rect(x, y, w, h, r, bg)?;
        } else {
            backend.fill_rect(x, y, w, h, bg)?;
        }
    }

    // If this inline box carries text content, render it directly.
    if let Some(ref text) = layout_box.text {
        let content = &layout_box.dimensions.content;
        paint_text(
            text,
            content.x,
            content.y,
            &layout_box.style,
            backend,
            offset_x,
            offset_y,
            ctx,
        )?;
    }

    for child in &layout_box.children {
        paint_box(child, backend, offset_x, offset_y, ctx, link_map)?;
    }
    Ok(())
}

/// Paint a single text run with optional decoration (underline,
/// line-through).
///
/// Called by [`paint_inline_content`] when rendering inline fragment text runs.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_text(
    text: &str,
    x: f32,
    y: f32,
    style: &crate::css::values::ComputedStyle,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let sx = (x - ctx.scroll_x + offset_x as f32) as i32;
    let sy = (y - ctx.scroll_y + offset_y as f32) as i32;

    let color = apply_filters_and_opacity(style.color, style.opacity, &style.filters);
    let bold = style.font_weight.is_bold();
    let italic = style.font_style == crate::css::values::FontStyle::Italic;
    let font_size = style.font_size as u16;

    // text-overflow: ellipsis — truncate text that overflows the clip
    // rect's right edge and append "...".
    let display_text: std::borrow::Cow<'_, str>;
    if ctx.text_overflow_ellipsis {
        if let Some(clip) = &ctx.clip_rect {
            let max_x = (clip.x + clip.width) as i32 - offset_x;
            let avail = (max_x - sx).max(0) as u32;
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

    // Draw text shadow first (behind the main text).
    if let Some(ref shadow) = style.text_shadow {
        let shadow_color = apply_opacity(shadow.color, style.opacity);
        let shx = sx + shadow.offset_x as i32;
        let shy = sy + shadow.offset_y as i32;
        backend.draw_text_styled(
            &display_text,
            shx,
            shy,
            font_size,
            shadow_color,
            bold,
            italic,
        )?;
    }

    backend.draw_text_styled(&display_text, sx, sy, font_size, color, bold, italic)?;

    // Measure actual text width including letter-spacing.
    let mut text_w = oasis_types::backend::bitmap_measure_text(&display_text, font_size) as f32;
    if style.letter_spacing != 0.0 {
        let chars = display_text.chars().count();
        if chars > 1 {
            text_w += style.letter_spacing * (chars - 1) as f32;
        }
    }
    let text_width = text_w.max(0.0) as u32;

    // Text decorations: underline, line-through, overline (bitflags — multiple can be active).
    if !style.text_decoration.line.is_none() {
        let deco_color = style
            .text_decoration
            .color
            .map(|c| apply_filters_and_opacity(c, style.opacity, &style.filters))
            .unwrap_or(color);

        // Draw each active decoration line.
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
            if !active {
                continue;
            }
            match style.text_decoration.style {
                TextDecorationStyle::Solid => {
                    backend.fill_rect(sx, deco_y, text_width, 1, deco_color)?;
                },
                TextDecorationStyle::Double => {
                    backend.fill_rect(sx, deco_y, text_width, 1, deco_color)?;
                    backend.fill_rect(sx, deco_y + 2, text_width, 1, deco_color)?;
                },
                TextDecorationStyle::Dashed => {
                    let dash_len = 4u32;
                    let mut pos = 0u32;
                    let mut draw = true;
                    while pos < text_width {
                        let seg = dash_len.min(text_width - pos);
                        if draw {
                            backend.fill_rect(sx + pos as i32, deco_y, seg, 1, deco_color)?;
                        }
                        pos += seg;
                        draw = !draw;
                    }
                },
                TextDecorationStyle::Dotted => {
                    let mut pos = 0u32;
                    while pos < text_width {
                        backend.fill_rect(sx + pos as i32, deco_y, 1, 1, deco_color)?;
                        pos += 2;
                    }
                },
                TextDecorationStyle::Wavy => {
                    let mut pos = 0u32;
                    while pos < text_width {
                        let offset = if (pos / 2).is_multiple_of(2) { 0 } else { 1 };
                        backend.fill_rect(
                            sx + pos as i32,
                            deco_y + offset,
                            1.min(text_width - pos),
                            1,
                            deco_color,
                        )?;
                        pos += 1;
                    }
                },
            }
        }
    }

    Ok(())
}
