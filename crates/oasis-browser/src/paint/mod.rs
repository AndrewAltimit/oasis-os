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
mod clip_path;
#[allow(dead_code)]
pub(crate) mod display_list;
#[allow(dead_code)]
pub(crate) mod filter_chain;
pub(crate) mod filters;
mod markers;
#[allow(clippy::too_many_arguments, clippy::collapsible_if)]
pub(crate) mod record;
#[allow(dead_code)]
pub(crate) mod render_target_pool;
mod replaced;
mod shadow;
mod stacking;
#[cfg(test)]
mod tests;
mod text;
#[allow(dead_code)]
pub(crate) mod tiling;
mod transforms;

pub(crate) use clip_path::{intersect_rects, resolve_clip_path_rect};
pub(crate) use stacking::{creates_compositing_layer, creates_stacking_context, is_positioned};
pub(crate) use transforms::{
    compute_transform_offsets, resolve_perspective_origin, resolve_transform_origin,
    transforms_have_3d,
};

use std::collections::HashMap;

use crate::css::values::types::{BackfaceVisibility, TransformStyle};
use crate::css::values::{
    BackgroundImage, Dimension, Overflow, Position, TextOverflow, Visibility, WhiteSpace,
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
#[derive(Debug, Clone)]
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
    /// Viewport height for culling off-screen content (may include buffer zone).
    pub height: f32,
    /// True visible viewport height (excludes buffer zone).
    /// Used for sticky positioning. Defaults to `height` if not set.
    pub visible_height: f32,
    /// DOM node id that currently has keyboard focus, if any. Used by
    /// the recorder to draw a caret in focused text inputs and pick
    /// the `caret-color` from that element's computed style.
    pub focused_node: Option<NodeId>,
    /// `@counter-style` rules collected from stylesheets, used for
    /// custom list-marker formatting. Empty by default.
    pub counter_styles: Vec<crate::css::parser::CounterStyleRule>,
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
    /// Accumulated CSS transform from ancestor elements.
    transform: crate::transform::AffineTransform2D,
    /// Inherited perspective frame from a `perspective:` ancestor.
    /// Children of an element with `perspective: d` see their 3D
    /// transforms projected through `Persp(d)` centred at the
    /// ancestor's `perspective-origin`.
    perspective_context: Option<PerspectiveContext>,
    /// Accumulated 3D matrix from `transform-style: preserve-3d`
    /// ancestors, in screen-space (post-translated to the ancestor's
    /// transform origin). `None` means "no preserved 3D context active".
    preserved_3d: Option<crate::transform::Matrix3d>,
    /// Full screen-space 4×4 matrix of the nearest 3D-transformed
    /// ancestor that went through the screen path. Descendants use
    /// this to project their background quads through all 4 corners
    /// individually (rather than via the 3-corner affine fit stored
    /// in `ctx.transform`), producing a true trapezoidal shape under
    /// steep perspective rotations like `rotateY(75deg)
    /// perspective(200px)`. `None` means "no 3D ancestor matrix
    /// available — fall back to the 2D affine".
    ambient_screen_matrix: Option<crate::transform::Matrix3d>,
    /// `@counter-style` rules from the stylesheets, used to format
    /// custom list markers.
    counter_styles: Vec<crate::css::parser::CounterStyleRule>,
}

/// A perspective frustum inherited from a `perspective:` ancestor.
///
/// `vanishing_x`/`vanishing_y` are the absolute screen-space
/// coordinates of the perspective vanishing point (typically the
/// centre of the ancestor's content box, offset by
/// `perspective-origin`).
#[derive(Debug, Clone, Copy)]
pub(super) struct PerspectiveContext {
    pub distance: f32,
    pub vanishing_x: f32,
    pub vanishing_y: f32,
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
        transform: crate::transform::AffineTransform2D::identity(),
        perspective_context: None,
        preserved_3d: None,
        ambient_screen_matrix: None,
        counter_styles: viewport.counter_styles.clone(),
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
        let bottom_px = match layout_box.style.bottom {
            Dimension::Px(b) => Some(b),
            _ => None,
        };
        if let Some(top) = top_px {
            let natural_screen_y = layout_box.dimensions.content.y - ctx.scroll_y + offset_y as f32;
            if natural_screen_y < top {
                (top - natural_screen_y) as i32
            } else {
                0
            }
        } else if let Some(bottom) = bottom_px {
            // Sticky bottom: clamp from viewport bottom.
            let natural_screen_y = layout_box.dimensions.content.y - ctx.scroll_y + offset_y as f32;
            let box_h = layout_box.dimensions.margin_box().height;
            let threshold = ctx.viewport_height - bottom - box_h;
            if natural_screen_y > threshold {
                (threshold - natural_screen_y) as i32
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

    // backface-visibility: hidden — when the element's effective 3D
    // transform flips its front face away from the viewer, skip
    // painting the entire subtree. The effective matrix includes
    // both the element's own transforms and any inherited preserve-3d
    // ancestor matrix.
    if layout_box.style.backface_visibility == BackfaceVisibility::Hidden
        && (!layout_box.style.transforms.is_empty() || ctx.preserved_3d.is_some())
    {
        let content = &layout_box.dimensions.content;
        let (ox, oy, oz) =
            resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        let local_m3d = if layout_box.style.transforms.is_empty() {
            crate::transform::Matrix3d::identity()
        } else {
            crate::transform::Matrix3d::from_css_transforms_3d(
                &layout_box.style.transforms,
                ox,
                oy,
                oz,
            )
        };
        let effective = match ctx.preserved_3d {
            Some(p) => p.multiply(&local_m3d),
            None => local_m3d,
        };
        if effective.front_face_normal_z(content.width, content.height) < 0.0 {
            return Ok(());
        }
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

        // 1. Background — skip if fully transparent with no image/texture.
        let has_bg = layout_box.style.background_color.a != 0
            || !matches!(layout_box.style.background_image, BackgroundImage::None)
            || layout_box.background_texture.is_some();
        if has_bg && let Err(e) = paint_background(layout_box, backend, offset_x, offset_y, ctx) {
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

        // 2. Borders — skip if all four border widths are zero.
        let bd = &layout_box.dimensions.border;
        let has_borders = bd.top != 0.0 || bd.right != 0.0 || bd.bottom != 0.0 || bd.left != 0.0;
        if has_borders && let Err(e) = paint_borders(layout_box, backend, offset_x, offset_y, ctx) {
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

        // 2b. Outline (outside border box, after borders) — skip if zero width.
        if layout_box.style.outline_width > 0.0
            && let Err(e) = paint_outline(layout_box, backend, offset_x, offset_y, ctx)
        {
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
        // text-overflow: ellipsis only activates when white-space
        // prevents wrapping (nowrap / pre), so multi-line content is
        // never ellipsized.
        ctx.text_overflow_ellipsis = layout_box.style.text_overflow == TextOverflow::Ellipsis
            && matches!(
                layout_box.style.white_space,
                WhiteSpace::NoWrap | WhiteSpace::Pre
            );
    }

    // CSS `clip-path`: intersect with the resolved shape's bounding rect.
    // True shape clipping (circle/ellipse edges) needs stencil support the
    // backend trait does not expose yet; we approximate to the bounding box
    // so at minimum the element can't paint outside its declared clip.
    if let Some(shape) = &layout_box.style.clip_path
        && let Some(shape_rect) = resolve_clip_path_rect(shape, &layout_box.dimensions.border_box())
    {
        ctx.clip_rect = Some(match ctx.clip_rect {
            Some(existing) => intersect_rects(existing, shape_rect),
            None => shape_rect,
        });
    }

    // Push hardware clip rect to GPU when an overflow clip is active.
    let did_push_hw_clip = if ctx.clip_rect != prev_clip {
        if let Some(cr) = ctx.clip_rect {
            let cx = (cr.x - ctx.scroll_x) as i32 + offset_x;
            let cy = (cr.y - ctx.scroll_y) as i32 + offset_y;
            let cw = cr.width.max(0.0) as u32;
            let ch = cr.height.max(0.0) as u32;
            backend.set_clip_rect(cx, cy, cw, ch)?;
            true
        } else {
            false
        }
    } else {
        false
    };

    // Compute transform matrix for this element.
    //
    // Two paths:
    //   • Screen-space 3D path — when this element is participating
    //     in a 3D rendering context (it has 3D transforms under a
    //     perspective ancestor, OR an ancestor has
    //     `transform-style: preserve-3d` and propagated a 4×4 matrix),
    //     build the full screen-space chain and project the box's
    //     screen rect through it.
    //   • Flat path — the existing 2D affine flatten via
    //     `compute_transform_matrix`. Used for plain 2D rotates,
    //     2D scales, or 3D transforms with no perspective ancestor
    //     and no inherited preserve-3d context.
    let content = &layout_box.dimensions.content;
    let has_3d_transforms = transforms_have_3d(&layout_box.style.transforms);
    // Per CSS Transforms 2 §6, `transform-style: preserve-3d`
    // unconditionally establishes a 3D rendering context — independent
    // of whether the element itself has 3D transforms. So a parent
    // with `transform: rotate(45deg); transform-style: preserve-3d`
    // and a child with `rotateY(60deg)` should compose the 2D
    // rotation with the 3D rotation in 3D space, not flatten the
    // parent first. We enter the screen path whenever the element
    // (or any ancestor) is participating in 3D rendering.
    let needs_screen_path = ctx.preserved_3d.is_some()
        || (ctx.perspective_context.is_some() && has_3d_transforms)
        || layout_box.style.transform_style == TransformStyle::Preserve3d;
    // Box top-left in screen coordinates (matching the background.rs
    // convention: layout coord - scroll + offset). Used by the screen
    // path and below by the children walk.
    let sx = content.x - ctx.scroll_x + offset_x as f32;
    let sy = content.y - ctx.scroll_y + offset_y as f32;
    // The full 4×4 screen-space matrix for this element, retained for
    // `transform-style: preserve-3d` propagation to descendants.
    let mut this_element_screen_matrix: Option<crate::transform::Matrix3d> = None;
    let child_matrix = if needs_screen_path {
        let (ox_l, oy_l, oz_l) =
            resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        // Screen-space transform-origin pivot for this element.
        let cox = sx + ox_l;
        let coy = sy + oy_l;
        // Local 3D transforms expressed without origin pre/post
        // translate — we apply origin in screen space below so that
        // composition with preserved/perspective matrices stays in
        // a single coordinate system.
        let local_no_origin = crate::transform::Matrix3d::from_css_transforms_3d(
            &layout_box.style.transforms,
            0.0,
            0.0,
            0.0,
        );
        // Element-local screen-space matrix: pivot at (cox, coy, oz_l).
        let m_local_screen = crate::transform::Matrix3d::translate(cox, coy, oz_l)
            .multiply(&local_no_origin)
            .multiply(&crate::transform::Matrix3d::translate(-cox, -coy, -oz_l));
        // Compose with the inherited `preserve-3d` ambient matrix.
        let m_preserved = match ctx.preserved_3d {
            Some(p) => p.multiply(&m_local_screen),
            None => m_local_screen,
        };
        // Apply the perspective frustum (if any) on the outside — but
        // skip when inheriting a preserve-3d matrix that already
        // contains the frustum.
        let m_screen = if ctx.preserved_3d.is_none()
            && let Some(persp) = ctx.perspective_context
        {
            crate::transform::Matrix3d::translate(persp.vanishing_x, persp.vanishing_y, 0.0)
                .multiply(&crate::transform::Matrix3d::perspective(persp.distance))
                .multiply(&crate::transform::Matrix3d::translate(
                    -persp.vanishing_x,
                    -persp.vanishing_y,
                    0.0,
                ))
                .multiply(&m_preserved)
        } else {
            m_preserved
        };
        this_element_screen_matrix = Some(m_screen);
        m_screen.project_screen_rect_affine(sx, sy, content.width, content.height)
    } else if layout_box.style.transforms.is_empty() {
        crate::transform::AffineTransform2D::identity()
    } else {
        // Flat 2D path. Build the matrix in SCREEN space: the pivot
        // is `(sx + ox_local, sy + oy_local)` rather than the
        // content-box-local origin. Previously we built the matrix
        // in local space (origin at `(ox_local, oy_local)`) and then
        // applied it to screen coordinates in `background.rs`,
        // which rotated the box around screen `(ox_local, oy_local)`
        // — only correct when the box was pinned to screen (0,0) —
        // and injected rotation-pivot compensation into `.e/.f` that
        // then got double-counted when naively added to child
        // offsets below.
        let (ox_l, oy_l, _oz) =
            resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        crate::transform::AffineTransform2D::from_css_transforms(
            &layout_box.style.transforms,
            sx + ox_l,
            sy + oy_l,
        )
    };
    // Compose with parent transform.
    let prev_transform = ctx.transform;
    let composed = ctx.transform.multiply(&child_matrix);
    if !child_matrix.is_translation_only() {
        ctx.transform = composed;
    }
    // Child offset handling:
    //   • Screen path — the projected affine already maps
    //     un-transformed screen coordinates to their projected
    //     positions, so adding its `.e/.f` would double-count.
    //   • Flat path, translate-only — `ctx.transform` is NOT
    //     updated (fast path), so children won't see the
    //     translation through the matrix. Push it through the
    //     offset instead.
    //   • Flat path, non-trivial — the matrix now lives in screen
    //     space, so `ctx.transform` handles all translation when
    //     children's backgrounds project through it. Adding `.e/.f`
    //     to the offset here was the "double-translation" bug from
    //     the backlog: it injected the rotation pivot compensation
    //     as an unrelated offset on children.
    let (tx_offset_x, tx_offset_y) = if needs_screen_path {
        (offset_x, offset_y)
    } else if child_matrix.is_translation_only() {
        (
            offset_x + child_matrix.e as i32,
            offset_y + child_matrix.f as i32,
        )
    } else {
        (offset_x, offset_y)
    };

    // If this element establishes a perspective frame, push it for
    // the duration of the children walk. The vanishing point is
    // resolved against this element's content box and offset to
    // absolute screen coordinates.
    let prev_perspective_context = ctx.perspective_context;
    if let Some(d) = layout_box.style.perspective
        && d > 0.0
    {
        let (po_x, po_y) =
            resolve_perspective_origin(layout_box.style.perspective_origin.as_ref(), content);
        let parent_screen_x = content.x - ctx.scroll_x + tx_offset_x as f32;
        let parent_screen_y = content.y - ctx.scroll_y + tx_offset_y as f32;
        ctx.perspective_context = Some(PerspectiveContext {
            distance: d,
            vanishing_x: parent_screen_x + po_x,
            vanishing_y: parent_screen_y + po_y,
        });
    }

    // `transform-style: preserve-3d` propagates this element's full
    // 4×4 screen-space matrix to descendants so they render in the
    // same 3D space. The default `flat` flushes the preserved
    // context: descendants render onto this element's flattened
    // 2D image, not in the ancestor's 3D frame.
    let prev_preserved_3d = ctx.preserved_3d;
    if layout_box.style.transform_style == TransformStyle::Preserve3d
        && let Some(m) = this_element_screen_matrix
    {
        ctx.preserved_3d = Some(m);
    } else {
        ctx.preserved_3d = None;
    }

    // Propagate this element's full 4×4 screen-space matrix to
    // descendants so their backgrounds can project all 4 corners
    // directly — see the trapezoidal-background follow-up in
    // `docs/browser-backlog.md`.
    //
    // Three cases:
    //   • Screen-path element — use `this_element_screen_matrix`
    //     directly; it already encodes the full parent chain plus
    //     this element's own local 3D matrix.
    //   • Flat-path element with its own non-trivial 2D transform
    //     AND an inherited ambient matrix — lift the 2D affine to
    //     4D via `Matrix3d::from_2d_affine` and compose onto the
    //     inherited ambient so descendants see the intervening 2D
    //     transform. Without this compose, an `<div>` with
    //     `transform: rotate(45deg)` sitting between a 3D
    //     ancestor and a grandchild would silently drop its
    //     rotation from the grandchild's background projection.
    //   • Otherwise — leave the inherited ambient matrix in place
    //     so deeper descendants still see their nearest 3D
    //     ancestor's matrix.
    let prev_ambient_screen_matrix = ctx.ambient_screen_matrix;
    if let Some(m) = this_element_screen_matrix {
        ctx.ambient_screen_matrix = Some(m);
    } else if let Some(parent_ambient) = ctx.ambient_screen_matrix
        && !child_matrix.is_translation_only()
    {
        let child_4d = crate::transform::Matrix3d::from_2d_affine(child_matrix);
        ctx.ambient_screen_matrix = Some(parent_ambient.multiply(&child_4d));
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
            // CSS 2.1 painting order (appendix E):
            //   1. Background & borders (already painted above)
            //   2. Stacking-context children with negative z-index
            //   3. Non-positioned children in tree order (normal flow)
            //   4. Positioned children with z-index: auto (tree order)
            //   5. Stacking-context children with z-index >= 0 (sorted)
            let child_count = layout_box.children.len();
            let mut normal_children: Vec<&LayoutBox> = Vec::with_capacity(child_count);
            let mut positioned_auto: Vec<(usize, &LayoutBox)> = Vec::new();
            let mut stacking_neg: Vec<(i32, usize, &LayoutBox)> = Vec::new();
            let mut stacking_pos: Vec<(i32, usize, &LayoutBox)> = Vec::new();

            // Inside a `transform-style: preserve-3d` subtree,
            // paint order is entirely determined by the children's
            // projected Z — CSS 2.1 stacking contexts inside the
            // 3D frame are effectively flattened into a single
            // back-to-front pass. Siblings with `translateZ(-100)`
            // and `translateZ(+100)` get their own stacking
            // contexts (any `transform` creates one), but that
            // must not freeze their order: the Z-sort below is
            // what actually decides who paints first.
            let preserve3d_flatten = layout_box.style.transform_style == TransformStyle::Preserve3d
                && this_element_screen_matrix.is_some();

            for (idx, child) in layout_box.children.iter().enumerate() {
                if preserve3d_flatten {
                    // Per CSS Transforms L2 §6.1: a child with an
                    // explicit `z-index` (not auto) inside a
                    // `preserve-3d` parent opts out of the 3D
                    // rendering context — its `transform-style` is
                    // treated as `flat` and it participates in the
                    // regular CSS 2.1 stacking tiers instead of the
                    // projected-Z sort. Only children that stay in
                    // the 3D context (z-index: auto, which is the
                    // default for transformed elements) join the
                    // Z-sorted `normal_children` list.
                    if !child.style.z_index_auto && creates_stacking_context(child) {
                        if child.style.z_index < 0 {
                            stacking_neg.push((child.style.z_index, idx, child));
                        } else {
                            stacking_pos.push((child.style.z_index, idx, child));
                        }
                    } else {
                        normal_children.push(child);
                    }
                } else if creates_stacking_context(child) {
                    if child.style.z_index < 0 {
                        stacking_neg.push((child.style.z_index, idx, child));
                    } else {
                        stacking_pos.push((child.style.z_index, idx, child));
                    }
                } else if is_positioned(child) {
                    positioned_auto.push((idx, child));
                } else {
                    normal_children.push(child);
                }
            }

            // Z-sort normal-flow children inside a preserve-3d
            // subtree. Siblings in a 3D rendering context occlude
            // each other by their projected Z, not by DOM order —
            // so e.g. a `translateZ(-50px)` card should paint before
            // a `translateZ(50px)` card even if the first appears
            // later in the DOM. Without this, DOM-later far children
            // would incorrectly paint over near ones.
            //
            // Sort key: the child's screen-center point passed
            // through `(parent_screen_matrix * child_local_matrix)`,
            // taking Z after the perspective divide. Sorted
            // ascending — in our frame, smaller Z = farther from
            // the viewer (the perspective matrix at column 2 row 3
            // is `-1/d`, so input `translateZ(+k)` lands with
            // smaller `w` and larger `z/w`), so ascending order
            // puts far first, matching the painter's-algorithm
            // back-to-front draw order. Inside a preserve-3d
            // parent, `preserve3d_flatten` above collapses CSS
            // 2.1's stacking-context tiers into the single
            // `normal_children` list so ALL siblings participate
            // in the sort, not just the normal-flow ones.
            let y_sorted = matches!(
                layout_box.box_type,
                BoxType::Block | BoxType::Anonymous | BoxType::TableWrapper
            ) && !(layout_box.style.transform_style == TransformStyle::Preserve3d
                && this_element_screen_matrix.is_some());

            if layout_box.style.transform_style == TransformStyle::Preserve3d
                && let Some(m_parent) = this_element_screen_matrix
                && normal_children.len() > 1
            {
                let mut z_keys: Vec<(f32, usize)> = Vec::with_capacity(normal_children.len());
                for (i, c) in normal_children.iter().enumerate() {
                    z_keys.push((
                        preserve3d_child_z(c, &m_parent, ctx, tx_offset_x, tx_offset_y),
                        i,
                    ));
                }
                // stable: ties preserve DOM order
                z_keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let sorted: Vec<&LayoutBox> =
                    z_keys.iter().map(|&(_, i)| normal_children[i]).collect();
                normal_children = sorted;
            }

            // Step 2: negative z-index stacking contexts (ascending).
            stacking_neg.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, _, child) in &stacking_neg {
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

            // Step 3: non-positioned children in DOM order.
            for child in &normal_children {
                if let Some(clip) = &ctx.clip_rect {
                    let cb = child.dimensions.border_box();
                    if cb.y > clip.y + clip.height {
                        if y_sorted {
                            break;
                        }
                        continue;
                    }
                    if cb.y + cb.height < clip.y
                        || cb.x + cb.width < clip.x
                        || cb.x > clip.x + clip.width
                    {
                        continue;
                    }
                }
                paint_box(child, backend, tx_offset_x, tx_offset_y, ctx, link_map)?;
            }

            // Step 4: positioned with z-index: auto in tree order.
            for (_, child) in &positioned_auto {
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

            // Step 5: non-negative z-index stacking contexts (sorted).
            stacking_pos.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, _, child) in &stacking_pos {
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

    // Restore hardware clip rect before restoring software clip.
    if did_push_hw_clip {
        backend.reset_clip_rect()?;
        if let Some(cr) = prev_clip {
            let cx = (cr.x - ctx.scroll_x) as i32 + offset_x;
            let cy = (cr.y - ctx.scroll_y) as i32 + offset_y;
            let cw = cr.width.max(0.0) as u32;
            let ch = cr.height.max(0.0) as u32;
            backend.set_clip_rect(cx, cy, cw, ch)?;
        }
    }

    // Restore previous clip rect, ellipsis flag, transform,
    // perspective context, and preserved-3d ambient matrix.
    ctx.clip_rect = prev_clip;
    ctx.text_overflow_ellipsis = prev_ellipsis;
    ctx.transform = prev_transform;
    ctx.perspective_context = prev_perspective_context;
    ctx.preserved_3d = prev_preserved_3d;
    ctx.ambient_screen_matrix = prev_ambient_screen_matrix;

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

/// Apply CSS filter effects and opacity to a color.
pub(super) fn apply_filters_and_opacity(
    color: Color,
    opacity: f32,
    filter_list: &[crate::css::values::FilterFunction],
) -> Color {
    let c = if filter_list.is_empty() {
        color
    } else {
        filters::apply_filters(color, filter_list)
    };
    c.apply_opacity(opacity)
}

/// Returns `true` if the layout box or any of its descendants is an
/// inline box or contains inline fragments that carry text.
fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

/// Sort key for `transform-style: preserve-3d` child ordering.
///
/// Returns the Z coordinate (after any perspective divide carried by
/// the parent's screen-space matrix) of the child's layout-center
/// point, after the child's own local 3D transform has been applied.
/// Used by `paint_box` to reorder normal-flow children into
/// back-to-front painter's-algorithm order inside a preserve-3d
/// subtree.
fn preserve3d_child_z(
    child: &LayoutBox,
    parent_screen_matrix: &crate::transform::Matrix3d,
    ctx: &PaintContext,
    offset_x: i32,
    offset_y: i32,
) -> f32 {
    let c = &child.dimensions.content;
    let ccx = c.x - ctx.scroll_x + offset_x as f32 + c.width / 2.0;
    let ccy = c.y - ctx.scroll_y + offset_y as f32 + c.height / 2.0;

    let child_local = if child.style.transforms.is_empty() {
        crate::transform::Matrix3d::identity()
    } else {
        let (ox_l, oy_l, oz_l) = resolve_transform_origin(child.style.transform_origin.as_ref(), c);
        let cox = c.x - ctx.scroll_x + offset_x as f32 + ox_l;
        let coy = c.y - ctx.scroll_y + offset_y as f32 + oy_l;
        let local = crate::transform::Matrix3d::from_css_transforms_3d(
            &child.style.transforms,
            0.0,
            0.0,
            0.0,
        );
        crate::transform::Matrix3d::translate(cox, coy, oz_l)
            .multiply(&local)
            .multiply(&crate::transform::Matrix3d::translate(-cox, -coy, -oz_l))
    };

    let composed = parent_screen_matrix.multiply(&child_local);
    let (_, _, z) = composed.apply_point_3d(ccx, ccy, 0.0);
    z
}
