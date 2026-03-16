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

mod background;
mod borders;
mod markers;
mod replaced;
mod shadow;
mod text;

use std::collections::HashMap;

use crate::css::values::{
    Dimension, Overflow, Position, TextOverflow, TransformFunction, Visibility,
};
use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, Rect};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use background::paint_background;
use borders::{paint_borders, paint_outline};
use markers::paint_list_marker;
use replaced::paint_replaced;
use shadow::paint_box_shadow;
use text::paint_inline_content;

// -------------------------------------------------------------------
// Public types

/// Viewport and scroll parameters for painting.
#[derive(Debug, Clone, Copy)]
pub struct PaintViewport {
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// X origin of the viewport in screen coordinates.
    pub x: i32,
    /// Y origin of the viewport in screen coordinates.
    pub y: i32,
    /// Viewport width for culling off-screen content.
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
pub(super) struct PaintContext {
    /// Accumulated link regions.
    links: Vec<LinkRegion>,
    /// When painting inside an `<a>` element, this holds `(href, node_id)`.
    current_link: Option<(String, NodeId)>,
    /// Vertical scroll offset (content shifts up by this amount).
    scroll_y: f32,
    /// Horizontal scroll offset (content shifts left by this amount).
    scroll_x: f32,
    /// Viewport height for offscreen culling.
    viewport_height: f32,
    /// Viewport width for offscreen culling.
    viewport_width: f32,
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
        scroll_x: viewport.scroll_x,
        viewport_height: viewport.height,
        viewport_width: viewport.width,
        clip_rect: None,
        text_overflow_ellipsis: false,
    };

    if let Err(e) = paint_box(layout, backend, viewport.x, viewport.y, &mut ctx, link_map) {
        log::warn!(
            "browser paint failed at scroll_y={}, viewport={}x{}: {e}",
            viewport.scroll_y,
            viewport.width,
            viewport.height,
        );
        return Err(e);
    }

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

pub(super) fn paint_box(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut PaintContext,
    link_map: &HashMap<NodeId, String>,
) -> Result<()> {
    // Compute sticky offset: if position:sticky and top is set, clamp the
    // box so it doesn't scroll above `top` offset from the viewport top.
    let sticky_dy = if layout_box.style.position == Position::Sticky {
        let top_px = match layout_box.style.top {
            Dimension::Px(t) => Some(t),
            _ => None,
        };
        if let Some(top) = top_px {
            let natural_screen_y = layout_box.dimensions.content.y - ctx.scroll_y + offset_y as f32;
            if natural_screen_y < top {
                (top - natural_screen_y) as i32
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };
    let offset_y = offset_y + sticky_dy;

    // Screen-space Y of this box (layout Y + viewport offset - scroll).
    let screen_y = layout_box.dimensions.content.y - ctx.scroll_y + sticky_dy as f32;
    let box_bottom = screen_y + layout_box.dimensions.margin_box().height;

    // Screen-space X of this box (layout X + viewport offset - scroll).
    let screen_x = layout_box.dimensions.content.x - ctx.scroll_x;
    let box_right = screen_x + layout_box.dimensions.margin_box().width;

    // Cull boxes that are entirely outside the viewport.
    if box_bottom < 0.0 || screen_y > ctx.viewport_height {
        return Ok(());
    }
    if box_right < 0.0 || screen_x > ctx.viewport_width {
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
        if let Err(e) = paint_box_shadow(layout_box, backend, offset_x, offset_y, ctx) {
            let b = layout_box.dimensions.border_box();
            log::debug!(
                "paint box_shadow failed at ({}, {}) {}x{}: {e}",
                b.x,
                b.y,
                b.width,
                b.height,
            );
            return Err(e);
        }

        // 1. Background
        if let Err(e) = paint_background(layout_box, backend, offset_x, offset_y, ctx) {
            let b = layout_box.dimensions.border_box();
            log::debug!(
                "paint background failed at ({}, {}) {}x{}: {e}",
                b.x,
                b.y,
                b.width,
                b.height,
            );
            return Err(e);
        }

        // 2. Borders
        if let Err(e) = paint_borders(layout_box, backend, offset_x, offset_y, ctx) {
            let b = layout_box.dimensions.border_box();
            log::debug!(
                "paint borders failed at ({}, {}) {}x{}: {e}",
                b.x,
                b.y,
                b.width,
                b.height,
            );
            return Err(e);
        }

        // 2b. Outline (outside border box, after borders).
        if let Err(e) = paint_outline(layout_box, backend, offset_x, offset_y, ctx) {
            let b = layout_box.dimensions.border_box();
            log::debug!(
                "paint outline failed at ({}, {}) {}x{}: {e}",
                b.x,
                b.y,
                b.width,
                b.height,
            );
            return Err(e);
        }
    }

    // Check overflow:hidden clipping -- if this box clips, intersect
    // with any existing clip from an ancestor.
    let prev_clip = ctx.clip_rect;
    let prev_ellipsis = ctx.text_overflow_ellipsis;
    if matches!(
        layout_box.style.overflow,
        Overflow::Hidden | Overflow::Scroll | Overflow::Auto
    ) {
        let new_clip = layout_box.dimensions.content;
        ctx.clip_rect = Some(match ctx.clip_rect {
            Some(existing) => intersect_rects(existing, new_clip),
            None => new_clip,
        });
        ctx.text_overflow_ellipsis = layout_box.style.text_overflow == TextOverflow::Ellipsis;
    }

    // Compute transform offset adjustments for children.
    // Translate: add dx/dy to offset. Scale: shift from center.
    // Rotate: no-op for now (requires backend rotation support).
    let (tx_offset_x, tx_offset_y) = compute_transform_offsets(
        &layout_box.style.transforms,
        &layout_box.dimensions.content,
        offset_x,
        offset_y,
    );

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
            // Stacking context: separate non-positioned (DOM order)
            // from positioned children (sorted by z-index).
            let mut normal_children: Vec<&LayoutBox> = Vec::new();
            let mut positioned_children: Vec<(i32, usize, &LayoutBox)> = Vec::new();

            for (idx, child) in layout_box.children.iter().enumerate() {
                if creates_stacking_context(child) {
                    positioned_children.push((child.style.z_index, idx, child));
                } else {
                    normal_children.push(child);
                }
            }

            // Paint non-positioned children in DOM order first.
            for child in &normal_children {
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
                paint_box(child, backend, tx_offset_x, tx_offset_y, ctx, link_map)?;
            }

            // Sort positioned children by z-index (stable sort
            // preserves DOM order for equal z-index values).
            positioned_children.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

            // Paint positioned children in z-order.
            for (_, _, child) in &positioned_children {
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
                paint_box(child, backend, tx_offset_x, tx_offset_y, ctx, link_map)?;
            }
        },
        BoxType::Inline => {
            if is_visible
                && let Err(e) = paint_inline_content(
                    layout_box,
                    backend,
                    tx_offset_x,
                    tx_offset_y,
                    ctx,
                    link_map,
                )
            {
                let c = &layout_box.dimensions.content;
                let text_preview = layout_box
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>();
                log::debug!(
                    "paint inline failed at ({}, {}) text={:?}: {e}",
                    c.x,
                    c.y,
                    text_preview,
                );
                return Err(e);
            }
        },
        BoxType::ListItem { marker } => {
            if is_visible {
                paint_list_marker(marker, layout_box, backend, tx_offset_x, tx_offset_y, ctx)?;
            }
            for child in &layout_box.children {
                paint_box(child, backend, tx_offset_x, tx_offset_y, ctx, link_map)?;
            }
        },
        BoxType::Replaced(replaced) => {
            if is_visible
                && let Err(e) =
                    paint_replaced(replaced, layout_box, backend, tx_offset_x, tx_offset_y, ctx)
            {
                let c = &layout_box.dimensions.content;
                log::debug!(
                    "paint replaced element failed at ({}, {}) {}x{}: {e}",
                    c.x,
                    c.y,
                    c.width,
                    c.height,
                );
                return Err(e);
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
                x: border.x - ctx.scroll_x + offset_x as f32,
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
// Helpers
// -------------------------------------------------------------------

/// Scale a color's alpha channel by an opacity factor.
pub(super) fn apply_opacity(color: Color, opacity: f32) -> Color {
    color.apply_opacity(opacity)
}

/// Returns `true` if the layout box or any of its descendants is an
/// inline box or contains inline fragments that carry text.
fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

/// Compute offset adjustments from CSS transforms.
///
/// Applies translate and scale transforms in order. Translate adds dx/dy.
/// Scale adjusts the offset from the element's center so children are
/// painted at the scaled position. Rotate is a no-op (requires backend
/// rotation support).
fn compute_transform_offsets(
    transforms: &[TransformFunction],
    content: &Rect,
    base_x: i32,
    base_y: i32,
) -> (i32, i32) {
    if transforms.is_empty() {
        return (base_x, base_y);
    }

    let mut dx: f32 = 0.0;
    let mut dy: f32 = 0.0;

    for tf in transforms {
        match tf {
            TransformFunction::Translate(tx, ty) => {
                dx += tx;
                dy += ty;
            },
            TransformFunction::Scale(sx, sy) => {
                // Scale from center: offset by half the size change.
                let cx = content.width / 2.0;
                let cy = content.height / 2.0;
                dx += cx * (1.0 - sx);
                dy += cy * (1.0 - sy);
            },
            TransformFunction::Rotate(_) => {
                // No-op: rotation requires actual backend rotation support.
            },
        }
    }

    (base_x + dx as i32, base_y + dy as i32)
}

/// Returns `true` if a layout box creates a new stacking context.
///
/// A stacking context is created by:
/// - Positioned elements (non-static) with a non-zero z-index
/// - Elements with opacity < 1.0
/// - Elements with CSS transforms
fn creates_stacking_context(layout_box: &LayoutBox) -> bool {
    let style = &layout_box.style;

    // Positioned + non-zero z-index.
    if style.position != Position::Static && style.z_index != 0 {
        return true;
    }

    // Opacity < 1.0 creates a stacking context.
    if style.opacity < 1.0 {
        return true;
    }

    // Non-empty transforms create a stacking context.
    if !style.transforms.is_empty() {
        return true;
    }

    false
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
    use crate::css::values::{BorderStyle, ComputedStyle, TransformFunction};
    use crate::layout::box_model::{EdgeSizes, ListMarker, Rect, ReplacedContent};
    use crate::test_utils::{DrawCall, MockBackend};
    use oasis_types::backend::Color;

    /// Default test viewport (480x272 at origin, no scroll).
    const TEST_VP: PaintViewport = PaintViewport {
        scroll_y: 0.0,
        scroll_x: 0.0,
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
        assert!(
            matches!(&&backend.calls[0], DrawCall::FillRect { .. }),
            "expected FillRect for background"
        );
        let DrawCall::FillRect { color, .. } = &&backend.calls[0] else {
            unreachable!()
        };
        assert_eq!(*color, Color::rgb(255, 0, 0));
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
        assert!(
            matches!(&&backend.calls[0], DrawCall::FillRect { .. }),
            "expected border FillRect"
        );
        let DrawCall::FillRect { h, color, .. } = &&backend.calls[0] else {
            unreachable!()
        };
        assert_eq!(*h, 2);
        assert_eq!(*color, Color::BLACK);
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
        assert!(
            matches!(&&backend.calls[0], DrawCall::DrawText { .. }),
            "expected DrawText for disc marker"
        );
        let DrawCall::DrawText { text, .. } = &&backend.calls[0] else {
            unreachable!()
        };
        assert_eq!(text, "\u{2022}");
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
        assert!(
            matches!(&&backend.calls[0], DrawCall::DrawText { .. }),
            "expected DrawText for decimal marker"
        );
        let DrawCall::DrawText { text, .. } = &&backend.calls[0] else {
            unreachable!()
        };
        assert_eq!(text, "3.");
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

    // ---------------------------------------------------------------
    // Test: stacking context helper
    // ---------------------------------------------------------------

    #[test]
    fn static_position_no_stacking_context() {
        let style = ComputedStyle::default();
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(!creates_stacking_context(&lb));
    }

    #[test]
    fn positioned_with_z_index_creates_stacking_context() {
        let mut style = ComputedStyle::default();
        style.position = Position::Relative;
        style.z_index = 1;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
    }

    #[test]
    fn opacity_creates_stacking_context() {
        let mut style = ComputedStyle::default();
        style.opacity = 0.5;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
    }

    #[test]
    fn transform_creates_stacking_context() {
        let mut style = ComputedStyle::default();
        style.transforms = vec![TransformFunction::Translate(10.0, 0.0)];
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
    }

    #[test]
    fn positioned_z_index_zero_no_stacking_context() {
        let mut style = ComputedStyle::default();
        style.position = Position::Relative;
        style.z_index = 0;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(!creates_stacking_context(&lb));
    }

    // ---------------------------------------------------------------
    // Test: stacking context paint order
    // ---------------------------------------------------------------

    #[test]
    fn stacking_context_z_order() {
        let mut backend = MockBackend::new();
        let link_map = HashMap::new();

        // Parent with two children:
        // Child A: z-index=2 (red, painted second)
        // Child B: z-index=1 (green, painted first)
        let mut root_style = ComputedStyle::default();
        root_style.background_color = Color::rgba(0, 0, 0, 0);
        let mut root = make_block(0.0, 0.0, 480.0, 272.0, root_style);

        // Child A: z-index=2
        let mut style_a = ComputedStyle::default();
        style_a.background_color = Color::rgb(255, 0, 0);
        style_a.position = Position::Relative;
        style_a.z_index = 2;
        let child_a = make_block(10.0, 10.0, 50.0, 50.0, style_a);

        // Child B: z-index=1
        let mut style_b = ComputedStyle::default();
        style_b.background_color = Color::rgb(0, 255, 0);
        style_b.position = Position::Relative;
        style_b.z_index = 1;
        let child_b = make_block(10.0, 70.0, 50.0, 50.0, style_b);

        // Add A first, then B in DOM order.
        root.children.push(child_a);
        root.children.push(child_b);

        paint(&root, &mut backend, TEST_VP, &link_map).unwrap();

        // Both children create stacking contexts.
        // z-index=1 (green) should be painted before z-index=2 (red).
        let fill_calls: Vec<_> = backend
            .calls
            .iter()
            .filter_map(|c| {
                if let DrawCall::FillRect { color, .. } = c {
                    Some(*color)
                } else {
                    None
                }
            })
            .collect();

        assert!(fill_calls.len() >= 2, "should have at least 2 fill rects");
        // Green (z=1) before red (z=2).
        let green_idx = fill_calls.iter().position(|c| *c == Color::rgb(0, 255, 0));
        let red_idx = fill_calls.iter().position(|c| *c == Color::rgb(255, 0, 0));
        assert!(
            green_idx.is_some() && red_idx.is_some(),
            "both colors should be painted",
        );
        assert!(
            green_idx.expect("green") < red_idx.expect("red"),
            "z-index=1 (green) should be painted before z-index=2 (red)",
        );
    }
}
