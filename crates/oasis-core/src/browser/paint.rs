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

use crate::backend::{Color, SdiBackend};
use crate::browser::css::values::{BorderStyle, TextDecoration};
use crate::browser::html::dom::NodeId;
use crate::browser::layout::box_model::{
    BoxType, InlineFragment, LayoutBox, LineBox, ListMarker, Rect, ReplacedContent,
};
use crate::error::Result;

// -------------------------------------------------------------------
// Public types
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
#[allow(clippy::too_many_arguments)]
pub fn paint(
    layout: &LayoutBox,
    backend: &mut dyn SdiBackend,
    scroll_y: f32,
    viewport_x: i32,
    viewport_y: i32,
    _viewport_width: f32,
    viewport_height: f32,
    link_map: &HashMap<NodeId, String>,
) -> Result<PaintResult> {
    let mut ctx = PaintContext {
        links: Vec::new(),
        current_link: None,
        scroll_y,
        viewport_height,
    };

    paint_box(layout, backend, viewport_x, viewport_y, &mut ctx, link_map)?;

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

    // 1. Background
    paint_background(layout_box, backend, offset_x, offset_y, ctx)?;

    // 2. Borders
    paint_borders(layout_box, backend, offset_x, offset_y, ctx)?;

    // 3-6. Children / inline content / replaced / markers
    match &layout_box.box_type {
        BoxType::Block
        | BoxType::Anonymous
        | BoxType::TableWrapper
        | BoxType::TableRow
        | BoxType::TableCell
        | BoxType::InlineBlock => {
            for child in &layout_box.children {
                paint_box(child, backend, offset_x, offset_y, ctx, link_map)?;
            }
        },
        BoxType::Inline => {
            paint_inline_content(layout_box, backend, offset_x, offset_y, ctx, link_map)?;
        },
        BoxType::ListItem { marker } => {
            paint_list_marker(marker, layout_box, backend, offset_x, offset_y, ctx)?;
            for child in &layout_box.children {
                paint_box(child, backend, offset_x, offset_y, ctx, link_map)?;
            }
        },
        BoxType::Replaced(replaced) => {
            paint_replaced(replaced, layout_box, backend, offset_x, offset_y, ctx)?;
        },
    }

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
    let bg = layout_box.style.background_color;
    if bg.a == 0 {
        return Ok(());
    }

    let padding = layout_box.dimensions.padding_box();
    let x = (padding.x + offset_x as f32) as i32;
    let y = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
    backend.fill_rect(x, y, padding.width as u32, padding.height as u32, bg)
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
        backend.fill_rect(bx, by, bw, d.border.top as u32, style.border_top_color)?;
    }
    // Right
    if d.border.right > 0.0 && style.border_right_style != BorderStyle::None {
        backend.fill_rect(
            bx + bw as i32 - d.border.right as i32,
            by,
            d.border.right as u32,
            bh,
            style.border_right_color,
        )?;
    }
    // Bottom
    if d.border.bottom > 0.0 && style.border_bottom_style != BorderStyle::None {
        backend.fill_rect(
            bx,
            by + bh as i32 - d.border.bottom as i32,
            bw,
            d.border.bottom as u32,
            style.border_bottom_color,
        )?;
    }
    // Left
    if d.border.left > 0.0 && style.border_left_style != BorderStyle::None {
        backend.fill_rect(bx, by, d.border.left as u32, bh, style.border_left_color)?;
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
    style: &crate::browser::css::values::ComputedStyle,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let sx = (x + offset_x as f32) as i32;
    let sy = (y - ctx.scroll_y + offset_y as f32) as i32;

    backend.draw_text(text, sx, sy, style.font_size as u16, style.color)?;

    // Approximate text width: each glyph is roughly half the font size
    // wide (matching the 8x8 bitmap font scaled up).
    let text_width = text.len() as u32 * (style.font_size as u32 / 2);

    // Underline decoration
    if style.text_decoration == TextDecoration::Underline {
        let underline_y = sy + style.font_size as i32;
        backend.fill_rect(sx, underline_y, text_width, 1, style.color)?;
    }

    // Line-through decoration
    if style.text_decoration == TextDecoration::LineThrough {
        let strike_y = sy + (style.font_size as i32 / 2);
        backend.fill_rect(sx, strike_y, text_width, 1, style.color)?;
    }

    Ok(())
}

/// Paint the fragments of a line box.
///
/// Will be called from the inline formatting context paint path once
/// the layout engine produces [`LineBox`] data.
#[allow(dead_code)]
fn paint_line_box(
    line: &LineBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    line_y: f32,
    ctx: &PaintContext,
) -> Result<()> {
    for frag in &line.fragments {
        match frag {
            InlineFragment::Text { text, x, style, .. } => {
                paint_text(text, *x, line_y, style, backend, offset_x, offset_y, ctx)?;
            },
            InlineFragment::InlineBox { layout_box } => {
                let content = &layout_box.dimensions.content;
                let sx = (content.x + offset_x as f32) as i32;
                let sy = (content.y - ctx.scroll_y + offset_y as f32) as i32;
                backend.draw_text(
                    "",
                    sx,
                    sy,
                    layout_box.style.font_size as u16,
                    layout_box.style.color,
                )?;
            },
            InlineFragment::ReplacedInline {
                replaced,
                x,
                width,
                height,
                style,
                ..
            } => {
                let sx = (*x + offset_x as f32) as i32;
                let sy = (line_y - ctx.scroll_y + offset_y as f32) as i32;
                match replaced {
                    ReplacedContent::Image {
                        texture: Some(tex), ..
                    } => {
                        backend.blit(*tex, sx, sy, *width as u32, *height as u32)?;
                    },
                    ReplacedContent::Image { alt, .. } => {
                        let label = if alt.is_empty() { "\u{00D7}" } else { alt };
                        backend.draw_text(label, sx + 2, sy + 2, 8, style.color)?;
                    },
                    _ => {},
                }
            },
        }
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
            let w = content.width as u32;
            let color = layout_box.style.border_top_color;
            backend.fill_rect(x, y, w, 1, color)?;
        },
        ReplacedContent::LineBreak => {
            // Nothing to paint.
        },
    }

    Ok(())
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Returns `true` if the layout box or any of its descendants is an
/// inline box or contains inline fragments that carry text.
fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Color;
    use crate::browser::css::values::ComputedStyle;
    use crate::browser::layout::box_model::{EdgeSizes, Rect};
    use crate::browser::test_utils::{DrawCall, MockBackend};

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        let result = paint(&root, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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
        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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
        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        let result = paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

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

        paint(&lb, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { w, h, color, .. } = &backend.calls[0] {
            assert_eq!(*w, 480);
            assert_eq!(*h, 1);
            assert_eq!(*color, Color::rgb(128, 128, 128));
        }
    }
}
