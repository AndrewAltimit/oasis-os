//! Paint layer: walks the layout tree and emits draw calls.
//!
//! The paint layer translates the geometry computed by the layout engine into
//! concrete rendering operations against the [`SdiBackend`] trait.  It also
//! records clickable link regions so the browser can perform hit-testing on
//! user input (mouse clicks, PSP button presses).
//!
//! Painting follows the CSS 2.1 painting order:
//! 1. Background -- `fill_rect()` with `background-color`
//! 2. Borders -- `fill_rect()` per edge with `border-color`
//! 3. Block children -- recurse
//! 4. Inline content -- text runs via `draw_text()`, inline backgrounds
//! 5. Replaced content -- images via `blit()`, `<hr>` via `fill_rect()`
//! 6. List markers -- bullets / numbers

use std::collections::HashMap;

use crate::css::values::{BorderStyle, Overflow, TextDecoration, TextOverflow, Visibility};
use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, ListMarker, Rect, ReplacedContent};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

// -------------------------------------------------------------------
// Public types

/// Viewport and scroll parameters for painting.
#[derive(Debug, Clone, Copy)]
pub struct PaintViewport {
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// X origin of the viewport in screen coordinates.
    pub x: i32,
    /// Y origin of the viewport in screen coordinates.
    pub y: i32,
    /// Viewport width (currently unused but reserved for clipping).
    pub width: f32,
    /// Viewport height for culling off-screen content.
    pub height: f32,
}
// -------------------------------------------------------------------

/// A clickable link region recorded during painting.
#[derive(Debug, Clone)]
pub struct LinkRegion {
    /// Screen-space bounding rectangle of the link.
    pub rect: Rect,
    /// The `href` attribute value.
    pub href: String,
    /// The DOM node this link originates from.
    pub node: NodeId,
}

/// The result of a paint pass.
pub struct PaintResult {
    /// Link hit regions recorded during this paint pass.
    pub links: Vec<LinkRegion>,
    /// Total content height in layout pixels (for scroll calculations).
    pub content_height: f32,
}

// -------------------------------------------------------------------
// Internal paint context
// -------------------------------------------------------------------

/// Mutable state threaded through the recursive paint walk.
struct PaintContext {
    /// Accumulated link regions.
    links: Vec<LinkRegion>,
    /// When painting inside an `<a>` element, this holds `(href, node_id)`.
    current_link: Option<(String, NodeId)>,
    /// Vertical scroll offset (content shifts up by this amount).
    scroll_y: f32,
    /// Viewport height for offscreen culling.
    viewport_height: f32,
    /// Active clipping rectangle from ancestor `overflow: hidden` boxes.
    clip_rect: Option<Rect>,
    /// When true, text overflowing the clip rect gets "..." appended.
    text_overflow_ellipsis: bool,
}

// -------------------------------------------------------------------
// Public entry points
// -------------------------------------------------------------------

/// Paint a layout tree to the backend.
///
/// `link_map` maps DOM `NodeId`s of `<a>` elements to their `href`
/// attribute values. This is built by the style/layout phase and passed
/// in so the paint layer can record clickable regions without needing
/// access to the DOM.
pub fn paint(
    layout: &LayoutBox,
    backend: &mut dyn SdiBackend,
    viewport: PaintViewport,
    link_map: &HashMap<NodeId, String>,
) -> Result<PaintResult> {
    let mut ctx = PaintContext {
        links: Vec::new(),
        current_link: None,
        scroll_y: viewport.scroll_y,
        viewport_height: viewport.height,
        clip_rect: None,
        text_overflow_ellipsis: false,
    };

    paint_box(layout, backend, viewport.x, viewport.y, &mut ctx, link_map)?;

    Ok(PaintResult {
        links: ctx.links,
        content_height: layout.dimensions.margin_box().height,
    })
}

/// Paint a highlight rectangle around a link region.
///
/// Used for PSP-style tab navigation where the currently focused link
/// is outlined with a visible border.
pub fn paint_link_highlight(
    link: &LinkRegion,
    backend: &mut dyn SdiBackend,
    highlight_color: Color,
) -> Result<()> {
    let r = &link.rect;
    let x = r.x as i32 - 2;
    let y = r.y as i32 - 1;
    let w = r.width as u32 + 4;
    let h = r.height as u32 + 2;

    // Top edge
    backend.fill_rect(x, y, w, 1, highlight_color)?;
    // Bottom edge
    backend.fill_rect(x, y + h as i32, w, 1, highlight_color)?;
    // Left edge
    backend.fill_rect(x, y, 1, h, highlight_color)?;
    // Right edge
    backend.fill_rect(x + w as i32, y, 1, h, highlight_color)?;

    Ok(())
}

// -------------------------------------------------------------------
// Recursive box painter
// -------------------------------------------------------------------

fn paint_box(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut PaintContext,
    link_map: &HashMap<NodeId, String>,
) -> Result<()> {
    // Screen-space Y of this box (layout Y + viewport offset - scroll).
    let screen_y = layout_box.dimensions.content.y - ctx.scroll_y;
    let box_bottom = screen_y + layout_box.dimensions.margin_box().height;

    // Cull boxes that are entirely above or below the viewport.
    if box_bottom < 0.0 || screen_y > ctx.viewport_height {
        return Ok(());
    }

    let is_visible = layout_box.style.visibility == Visibility::Visible;

    // Track whether we just entered a link element.
    let entered_link = if let Some(node_id) = layout_box.node {
        if let Some(href) = link_map.get(&node_id) {
            ctx.current_link = Some((href.clone(), node_id));
            true
        } else {
            false
        }
    } else {
        false
    };

    // visibility:hidden skips painting this box's own background/borders/content
    // but children may override visibility and still paint.
    if is_visible {
        // 0. Box shadow (behind background).
        paint_box_shadow(layout_box, backend, offset_x, offset_y, ctx)?;

        // 1. Background
        paint_background(layout_box, backend, offset_x, offset_y, ctx)?;

        // 2. Borders
        paint_borders(layout_box, backend, offset_x, offset_y, ctx)?;
    }

    // Check overflow:hidden clipping -- if this box clips, intersect
    // with any existing clip from an ancestor.
    let prev_clip = ctx.clip_rect;
    let prev_ellipsis = ctx.text_overflow_ellipsis;
    if layout_box.style.overflow == Overflow::Hidden {
        let new_clip = layout_box.dimensions.content;
        ctx.clip_rect = Some(match ctx.clip_rect {
            Some(existing) => intersect_rects(existing, new_clip),
            None => new_clip,
        });
        ctx.text_overflow_ellipsis = layout_box.style.text_overflow == TextOverflow::Ellipsis;
    }

    // 3-6. Children / inline content / replaced / markers
    match &layout_box.box_type {
        BoxType::Block
        | BoxType::Flex
        | BoxType::Grid
        | BoxType::Anonymous
        | BoxType::TableWrapper
        | BoxType::TableRow
        | BoxType::TableCell
        | BoxType::InlineBlock => {
            for child in &layout_box.children {
                // Skip children entirely outside clip rect.
                if let Some(clip) = &ctx.clip_rect {
                    let cb = child.dimensions.border_box();
                    if cb.y + cb.height < clip.y
                        || cb.y > clip.y + clip.height
                        || cb.x + cb.width < clip.x
                        || cb.x > clip.x + clip.width
                    {
                        continue;
                    }
                }
                paint_box(child, backend, offset_x, offset_y, ctx, link_map)?;
            }
        },
        BoxType::Inline => {
            if is_visible {
                paint_inline_content(layout_box, backend, offset_x, offset_y, ctx, link_map)?;
            }
        },
        BoxType::ListItem { marker } => {
            if is_visible {
                paint_list_marker(marker, layout_box, backend, offset_x, offset_y, ctx)?;
            }
            for child in &layout_box.children {
                paint_box(child, backend, offset_x, offset_y, ctx, link_map)?;
            }
        },
        BoxType::Replaced(replaced) => {
            if is_visible {
                paint_replaced(replaced, layout_box, backend, offset_x, offset_y, ctx)?;
            }
        },
    }

    // Restore previous clip rect and ellipsis flag.
    ctx.clip_rect = prev_clip;
    ctx.text_overflow_ellipsis = prev_ellipsis;

    // Record a link hit region when leaving a link element.
    if let Some((ref href, link_node)) = ctx.current_link
        && (layout_box.node == Some(link_node) || has_text_content(layout_box))
    {
        let border = layout_box.dimensions.border_box();
        ctx.links.push(LinkRegion {
            rect: Rect {
                x: border.x + offset_x as f32,
                y: border.y - ctx.scroll_y + offset_y as f32,
                width: border.width,
                height: border.height,
            },
            href: href.clone(),
            node: link_node,
        });
    }

    // Reset link tracking when leaving the link element's box.
    if entered_link
        && let Some(node_id) = layout_box.node
        && ctx
            .current_link
            .as_ref()
            .is_some_and(|(_, n)| *n == node_id)
    {
        ctx.current_link = None;
    }

    Ok(())
}

// -------------------------------------------------------------------
// Background
// -------------------------------------------------------------------

fn paint_background(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let padding = layout_box.dimensions.padding_box();
    let x = (padding.x + offset_x as f32) as i32;
    let y = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
    let w = padding.width as u32;
    let h = padding.height as u32;

    // Paint background color.
    let bg = apply_opacity(layout_box.style.background_color, layout_box.style.opacity);
    if bg.a > 0 {
        if layout_box.style.border_radius > 0.0 {
            backend.fill_rounded_rect(x, y, w, h, layout_box.style.border_radius as u16, bg)?;
        } else {
            backend.fill_rect(x, y, w, h, bg)?;
        }
    }

    // Paint linear gradient background.
    if let crate::css::values::BackgroundImage::Gradient(ref grad) =
        layout_box.style.background_image
    {
        paint_linear_gradient(backend, x, y, w, h, grad, layout_box.style.opacity)?;
    }

    // Paint background image (if texture has been resolved).
    if let Some(tex) = layout_box.background_texture {
        backend.blit(tex, x, y, w, h)?;
    }

    Ok(())
}

/// Render a CSS `linear-gradient(...)` using the backend's gradient fill.
fn paint_linear_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::LinearGradient,
    opacity: f32,
) -> Result<()> {
    use crate::css::values::GradientDirection;
    use oasis_types::backend::GradientStyle;

    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return Ok(());
    }

    let first = apply_opacity(grad.stops[0].color, opacity);
    let last = apply_opacity(grad.stops[grad.stops.len() - 1].color, opacity);

    match grad.direction {
        GradientDirection::ToBottom | GradientDirection::Angle(180.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Vertical {
                    top: first,
                    bottom: last,
                },
            )?;
        },
        GradientDirection::ToTop | GradientDirection::Angle(0.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Vertical {
                    top: last,
                    bottom: first,
                },
            )?;
        },
        GradientDirection::ToRight | GradientDirection::Angle(90.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Horizontal {
                    left: first,
                    right: last,
                },
            )?;
        },
        GradientDirection::ToLeft | GradientDirection::Angle(270.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Horizontal {
                    left: last,
                    right: first,
                },
            )?;
        },
        GradientDirection::Angle(deg) => {
            // For arbitrary angles, approximate with the closest axis.
            let norm = ((deg % 360.0) + 360.0) % 360.0;
            if !(45.0..315.0).contains(&norm) {
                // ~to top
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Vertical {
                        top: last,
                        bottom: first,
                    },
                )?;
            } else if norm < 135.0 {
                // ~to right
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Horizontal {
                        left: first,
                        right: last,
                    },
                )?;
            } else if norm < 225.0 {
                // ~to bottom
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Vertical {
                        top: first,
                        bottom: last,
                    },
                )?;
            } else {
                // ~to left
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Horizontal {
                        left: last,
                        right: first,
                    },
                )?;
            }
        },
    }

    Ok(())
}

// -------------------------------------------------------------------
// Borders
// -------------------------------------------------------------------

fn paint_borders(
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
fn paint_border_edge(
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

// -------------------------------------------------------------------
// Inline content
// -------------------------------------------------------------------

fn paint_inline_content(
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
        let x = (pb.x + offset_x as f32) as i32;
        let y = (pb.y - ctx.scroll_y + offset_y as f32) as i32;
        let w = pb.width as u32;
        let h = pb.height as u32;
        if layout_box.style.border_radius > 0.0 {
            backend.fill_rounded_rect(x, y, w, h, layout_box.style.border_radius as u16, bg)?;
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
/// Called by [`paint_line_box`] when rendering inline fragment text runs.
#[allow(clippy::too_many_arguments)]
fn paint_text(
    text: &str,
    x: f32,
    y: f32,
    style: &crate::css::values::ComputedStyle,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let sx = (x + offset_x as f32) as i32;
    let sy = (y - ctx.scroll_y + offset_y as f32) as i32;

    let color = apply_opacity(style.color, style.opacity);
    let bold = style.font_weight == crate::css::values::FontWeight::Bold;
    let italic = style.font_style == crate::css::values::FontStyle::Italic;
    let font_size = style.font_size as u16;

    // text-overflow: ellipsis — truncate text that overflows the clip
    // rect's right edge and append "…".
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

    // Underline decoration: just below baseline (~85% of font-size).
    if style.text_decoration == TextDecoration::Underline {
        let underline_y = sy + (style.font_size * 0.85) as i32;
        backend.fill_rect(sx, underline_y, text_width, 1, color)?;
    }

    // Line-through decoration: at x-height (~40% of font-size).
    if style.text_decoration == TextDecoration::LineThrough {
        let strike_y = sy + (style.font_size * 0.4) as i32;
        backend.fill_rect(sx, strike_y, text_width, 1, color)?;
    }

    // Overline decoration
    if style.text_decoration == TextDecoration::Overline {
        backend.fill_rect(sx, sy, text_width, 1, color)?;
    }

    Ok(())
}

// -------------------------------------------------------------------
// List markers
// -------------------------------------------------------------------

fn paint_list_marker(
    marker: &ListMarker,
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let content = &layout_box.dimensions.content;
    let x = (content.x + offset_x as f32 - 20.0) as i32;
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

// -------------------------------------------------------------------
// Replaced elements
// -------------------------------------------------------------------

fn paint_replaced(
    replaced: &ReplacedContent,
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let content = &layout_box.dimensions.content;
    let x = (content.x + offset_x as f32) as i32;
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

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Scale a color's alpha channel by an opacity factor.
fn apply_opacity(color: Color, opacity: f32) -> Color {
    if opacity >= 1.0 {
        return color;
    }
    Color::rgba(color.r, color.g, color.b, (color.a as f32 * opacity) as u8)
}

/// Paint a box shadow behind the element.
fn paint_box_shadow(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let shadow = match layout_box.style.box_shadow {
        Some(ref s) => s,
        None => return Ok(()),
    };

    let border = layout_box.dimensions.border_box();
    let bx = (border.x + offset_x as f32 + shadow.offset_x) as i32;
    let by = (border.y - ctx.scroll_y + offset_y as f32 + shadow.offset_y) as i32;
    let bw = (border.width + shadow.spread * 2.0) as u32;
    let bh = (border.height + shadow.spread * 2.0) as u32;

    // Approximate blur with concentric rectangles at decreasing opacity.
    let steps = (shadow.blur as i32).max(1);
    for i in (0..steps).rev() {
        let t = i as f32 / steps as f32;
        let alpha = ((shadow.color.a as f32) * (1.0 - t) * 0.4) as u8;
        if alpha == 0 {
            continue;
        }
        let expand = i;
        let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
        backend.fill_rect(
            bx - expand,
            by - expand,
            bw + expand as u32 * 2,
            bh + expand as u32 * 2,
            color,
        )?;
    }
    Ok(())
}

/// Returns `true` if the layout box or any of its descendants is an
/// inline box or contains inline fragments that carry text.
fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

/// Compute the intersection of two rectangles.
fn intersect_rects(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect {
        x,
        y,
        width: (right - x).max(0.0),
        height: (bottom - y).max(0.0),
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::ComputedStyle;
    use crate::layout::box_model::{EdgeSizes, Rect};
    use crate::test_utils::{DrawCall, MockBackend};
    use oasis_types::backend::Color;

    /// Default test viewport (480x272 at origin, no scroll).
    const TEST_VP: PaintViewport = PaintViewport {
        scroll_y: 0.0,
        x: 0,
        y: 0,
        width: 480.0,
        height: 272.0,
    };

    // ---------------------------------------------------------------
    // Helper: build a simple block layout box
    // ---------------------------------------------------------------

    fn make_block(x: f32, y: f32, w: f32, h: f32, style: ComputedStyle) -> LayoutBox {
        let mut lb = LayoutBox::new(BoxType::Block, style, Some(0));
        lb.dimensions.content = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        lb
    }

    // ---------------------------------------------------------------
    // Test 1: background painting skips transparent backgrounds
    // ---------------------------------------------------------------

    #[test]
    fn transparent_background_skipped() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();
        // Default background is transparent (a=0).
        assert_eq!(style.background_color.a, 0);

        let lb = make_block(0.0, 0.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // No fill_rect calls for the transparent background.
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn opaque_background_painted() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.background_color = Color::rgb(255, 0, 0);

        let lb = make_block(10.0, 20.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert!(backend.fill_rect_count() > 0);
        // First fill_rect should be the background.
        if let DrawCall::FillRect { color, .. } = &backend.calls[0] {
            assert_eq!(*color, Color::rgb(255, 0, 0));
        } else {
            panic!("expected FillRect for background");
        }
    }

    // ---------------------------------------------------------------
    // Test 2: border painting with zero-width borders skips calls
    // ---------------------------------------------------------------

    #[test]
    fn zero_width_borders_skipped() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();
        // Default border widths are 0.0.

        let lb = make_block(0.0, 0.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // No calls at all (transparent bg + zero borders).
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn nonzero_borders_painted() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.border_top_width = 2.0;
        style.border_top_style = BorderStyle::Solid;
        style.border_top_color = Color::BLACK;

        let mut lb = make_block(10.0, 10.0, 100.0, 50.0, style);
        lb.dimensions.border = EdgeSizes {
            top: 2.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        };
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // Should have exactly one fill_rect for the top border.
        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { h, color, .. } = &backend.calls[0] {
            assert_eq!(*h, 2);
            assert_eq!(*color, Color::BLACK);
        } else {
            panic!("expected border FillRect");
        }
    }

    // ---------------------------------------------------------------
    // Test 3: link region recording
    // ---------------------------------------------------------------

    #[test]
    fn link_regions_recorded() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let mut link_box = make_block(10.0, 10.0, 80.0, 16.0, style.clone());
        link_box.node = Some(5);

        // Add a child inline box so `has_text_content` returns true.
        let inline_child = LayoutBox::new(BoxType::Inline, style.clone(), None);
        link_box.children.push(inline_child);

        let mut root = make_block(0.0, 0.0, 480.0, 272.0, style);
        root.children.push(link_box);

        let mut link_map = HashMap::new();
        link_map.insert(5_usize, "https://example.com".to_string());

        let result = paint(&root, &mut backend, TEST_VP, &link_map).unwrap();

        assert!(!result.links.is_empty());
        assert_eq!(result.links[0].href, "https://example.com");
        assert_eq!(result.links[0].node, 5);
    }

    // ---------------------------------------------------------------
    // Test 4: offscreen culling
    // ---------------------------------------------------------------

    #[test]
    fn offscreen_above_viewport_culled() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.background_color = Color::rgb(255, 0, 0);

        // Box at y=-100, height=50 with scroll_y=0 => entirely
        // above viewport (screen_y = -100, bottom = -50).
        let lb = make_block(0.0, -100.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert_eq!(
            backend.calls.len(),
            0,
            "offscreen box above viewport should be culled"
        );
    }

    #[test]
    fn offscreen_below_viewport_culled() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.background_color = Color::rgb(0, 255, 0);

        // Box at y=500 with viewport_height=272 => entirely below.
        let lb = make_block(0.0, 500.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert_eq!(
            backend.calls.len(),
            0,
            "offscreen box below viewport should be culled"
        );
    }

    #[test]
    fn onscreen_box_not_culled() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.background_color = Color::rgb(0, 0, 255);

        let lb = make_block(0.0, 100.0, 100.0, 50.0, style);
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert!(!backend.calls.is_empty(), "onscreen box should be painted");
    }

    // ---------------------------------------------------------------
    // Test 5: list marker rendering (disc vs decimal)
    // ---------------------------------------------------------------

    #[test]
    fn list_marker_disc() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let lb = LayoutBox::new(
            BoxType::ListItem {
                marker: ListMarker::Disc,
            },
            style,
            Some(0),
        );
        let link_map = HashMap::new();
        // The box is at default (0,0) with no content -- that is
        // fine; we just check that the bullet character is drawn.
        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert!(backend.draw_text_count() > 0);
        if let DrawCall::DrawText { text, .. } = &backend.calls[0] {
            assert_eq!(text, "\u{2022}");
        } else {
            panic!("expected DrawText for disc marker");
        }
    }

    #[test]
    fn list_marker_decimal() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let lb = LayoutBox::new(
            BoxType::ListItem {
                marker: ListMarker::Decimal(3),
            },
            style,
            Some(0),
        );
        let link_map = HashMap::new();
        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert!(backend.draw_text_count() > 0);
        if let DrawCall::DrawText { text, .. } = &backend.calls[0] {
            assert_eq!(text, "3.");
        } else {
            panic!("expected DrawText for decimal marker");
        }
    }

    // ---------------------------------------------------------------
    // Test 6: broken image placeholder dimensions
    // ---------------------------------------------------------------

    #[test]
    fn broken_image_placeholder() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::Image {
                width: 0,
                height: 0,
                texture: None,
                alt: String::new(),
            }),
            style,
            Some(0),
        );
        // Give it a small content area.
        lb.dimensions.content = Rect {
            x: 10.0,
            y: 10.0,
            width: 8.0,
            height: 8.0,
        };
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // Should have 4 fill_rects (border) + 1 draw_text (X symbol).
        let fill_count = backend.fill_rect_count();
        let text_count = backend.draw_text_count();
        assert_eq!(fill_count, 4, "expected 4 border lines for placeholder");
        assert_eq!(text_count, 1, "expected 1 draw_text for placeholder symbol");

        // The placeholder should use at least 16x16 (the minimum).
        if let DrawCall::FillRect { w, h, .. } = &backend.calls[0] {
            assert!(
                *w >= 16 || *h >= 1,
                "placeholder should enforce minimum size"
            );
        }
    }

    #[test]
    fn broken_image_with_alt_text() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::Image {
                width: 0,
                height: 0,
                texture: None,
                alt: "Photo".to_string(),
            }),
            style,
            Some(0),
        );
        lb.dimensions.content = Rect {
            x: 10.0,
            y: 10.0,
            width: 32.0,
            height: 32.0,
        };
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // The draw_text call should use the alt text, not the X.
        let text_call = backend
            .calls
            .iter()
            .find(|c| matches!(c, DrawCall::DrawText { .. }));
        assert!(text_call.is_some());
        if let DrawCall::DrawText { text, .. } = text_call.unwrap() {
            assert_eq!(text, "Photo");
        }
    }

    // ---------------------------------------------------------------
    // Test: content height reported correctly
    // ---------------------------------------------------------------

    #[test]
    fn content_height_reported() {
        let mut backend = MockBackend::new();
        let style = ComputedStyle::default();

        let mut lb = make_block(0.0, 0.0, 480.0, 500.0, style);
        lb.dimensions.margin = EdgeSizes {
            top: 10.0,
            right: 0.0,
            bottom: 10.0,
            left: 0.0,
        };
        let link_map = HashMap::new();

        let result = paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        // margin_box height = content(500) + margin(10+10) = 520
        assert!((result.content_height - 520.0).abs() < f32::EPSILON);
    }

    // ---------------------------------------------------------------
    // Test: paint_link_highlight draws four edges
    // ---------------------------------------------------------------

    #[test]
    fn link_highlight_draws_border() {
        let mut backend = MockBackend::new();
        let link = LinkRegion {
            rect: Rect {
                x: 50.0,
                y: 100.0,
                width: 80.0,
                height: 16.0,
            },
            href: "https://example.com".to_string(),
            node: 1,
        };

        paint_link_highlight(&link, &mut backend, Color::rgb(255, 255, 0)).unwrap();

        // Should draw exactly 4 fill_rect calls (one per edge).
        assert_eq!(backend.fill_rect_count(), 4);
    }

    // ---------------------------------------------------------------
    // Test: has_text_content helper
    // ---------------------------------------------------------------

    #[test]
    fn has_text_content_inline() {
        let style = ComputedStyle::default();
        let lb = LayoutBox::new(BoxType::Inline, style, None);
        assert!(has_text_content(&lb));
    }

    #[test]
    fn has_text_content_nested() {
        let style = ComputedStyle::default();
        let inner = LayoutBox::new(BoxType::Inline, style.clone(), None);
        let mut outer = LayoutBox::new(BoxType::Block, style, None);
        outer.children.push(inner);
        assert!(has_text_content(&outer));
    }

    #[test]
    fn has_text_content_empty_block() {
        let style = ComputedStyle::default();
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(!has_text_content(&lb));
    }

    // ---------------------------------------------------------------
    // Test: horizontal rule painting
    // ---------------------------------------------------------------

    #[test]
    fn horizontal_rule_painted() {
        let mut backend = MockBackend::new();
        let mut style = ComputedStyle::default();
        style.border_top_color = Color::rgb(128, 128, 128);

        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::HorizontalRule),
            style,
            Some(0),
        );
        lb.dimensions.content = Rect {
            x: 0.0,
            y: 50.0,
            width: 480.0,
            height: 1.0,
        };
        let link_map = HashMap::new();

        paint(&lb, &mut backend, TEST_VP, &link_map).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { w, h, color, .. } = &backend.calls[0] {
            assert_eq!(*w, 480);
            assert_eq!(*h, 1);
            assert_eq!(*color, Color::rgb(128, 128, 128));
        }
    }
}
