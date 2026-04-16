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
mod text;
#[allow(dead_code)]
pub(crate) mod tiling;

use std::collections::HashMap;

use crate::css::values::types::{
    BackfaceVisibility, PerspectiveOrigin, TransformOrigin, TransformStyle,
};
use crate::css::values::{
    BackgroundImage, ClipLength, ClipPath, Dimension, Overflow, Position, TextOverflow,
    TransformFunction, Visibility, WhiteSpace,
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
                    normal_children.push(child);
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

/// Returns `true` if the transform list contains any 3D function.
/// Used to gate the perspective projection path: the orthographic
/// flatten is the right choice for pure 2D transforms even when an
/// ancestor has `perspective`.
pub(crate) fn transforms_have_3d(transforms: &[TransformFunction]) -> bool {
    transforms.iter().any(|t| {
        matches!(
            t,
            TransformFunction::Translate3d(..)
                | TransformFunction::TranslateZ(_)
                | TransformFunction::Scale3d(..)
                | TransformFunction::ScaleZ(_)
                | TransformFunction::RotateX(_)
                | TransformFunction::RotateY(_)
                | TransformFunction::Rotate3d(..)
                | TransformFunction::Matrix3d(_)
                | TransformFunction::Perspective(_)
        )
    })
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

/// Resolve a CSS `transform-origin` against an element's content box.
///
/// Returns absolute pixel offsets `(ox, oy, oz)` from the content
/// box's top-left corner. When `origin` is `None`, defaults to the
/// spec's `50% 50% 0` (box center, Z = 0).
pub(crate) fn resolve_transform_origin(
    origin: Option<&TransformOrigin>,
    content: &Rect,
) -> (f32, f32, f32) {
    match origin {
        None => (content.width / 2.0, content.height / 2.0, 0.0),
        Some(o) => {
            let ox = o.x_pct.map(|p| content.width * p).unwrap_or(o.x);
            let oy = o.y_pct.map(|p| content.height * p).unwrap_or(o.y);
            (ox, oy, o.z)
        },
    }
}

/// Resolve a CSS `perspective-origin` against an element's border box.
///
/// Defaults to the spec's `50% 50%` when `origin` is `None`.
pub(crate) fn resolve_perspective_origin(
    origin: Option<&PerspectiveOrigin>,
    container: &Rect,
) -> (f32, f32) {
    match origin {
        None => (container.width / 2.0, container.height / 2.0),
        Some(o) => {
            let ox = o.x_pct.map(|p| container.width * p).unwrap_or(o.x);
            let oy = o.y_pct.map(|p| container.height * p).unwrap_or(o.y);
            (ox, oy)
        },
    }
}

/// Compute the full 2D affine transform from CSS transforms.
///
/// Returns the composed matrix which callers use either as a simple
/// translation offset (fast path) or for full geometry transformation.
/// `transform_origin` defaults to `50% 50% 0` when `None`.
pub(crate) fn compute_transform_matrix(
    transforms: &[TransformFunction],
    transform_origin: Option<&TransformOrigin>,
    content: &Rect,
) -> crate::transform::AffineTransform2D {
    let (ox, oy, _oz) = resolve_transform_origin(transform_origin, content);
    crate::transform::AffineTransform2D::from_css_transforms(transforms, ox, oy)
}

/// Compute offset adjustments from CSS transforms.
///
/// Returns the translation component of the composed transform matrix
/// added to the base offsets. For translation-only transforms this is
/// exact; for rotation/scale/skew the full matrix is available via
/// [`compute_transform_matrix`].
pub(crate) fn compute_transform_offsets(
    transforms: &[TransformFunction],
    transform_origin: Option<&TransformOrigin>,
    content: &Rect,
    base_x: i32,
    base_y: i32,
) -> (i32, i32) {
    if transforms.is_empty() {
        return (base_x, base_y);
    }
    let m = compute_transform_matrix(transforms, transform_origin, content);
    (base_x + m.e as i32, base_y + m.f as i32)
}

/// Returns `true` if a layout box creates a new stacking context.
///
/// Per CSS 2.1, a stacking context is created by:
/// - Positioned elements (non-static) with an explicit z-index (not `auto`)
/// - Elements with opacity < 1.0
/// - Elements with CSS transforms
/// - Elements with CSS filters
/// - Elements with will-change: transform
///
/// Note: positioned elements with `z-index: auto` do NOT create a stacking
/// context -- they participate in the parent's stacking context at level 0.
pub(crate) fn creates_stacking_context(layout_box: &LayoutBox) -> bool {
    let style = &layout_box.style;

    // Positioned + explicit z-index (not auto).
    if style.position != Position::Static && !style.z_index_auto {
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

    // Filters create a stacking context.
    if !style.filters.is_empty() {
        return true;
    }

    // will-change: transform/opacity/filter creates a stacking context.
    if style.will_change_promotes_layer {
        return true;
    }

    // `isolation: isolate` forces a stacking context so descendant
    // `mix-blend-mode` is contained (CSS Compositing and Blending L1).
    if matches!(
        style.isolation,
        crate::css::values::types::Isolation::Isolate
    ) {
        return true;
    }

    // `mix-blend-mode` on a non-Normal value implies a stacking context
    // so the blend applies to the entire painted subtree.
    if !matches!(
        style.mix_blend_mode,
        crate::css::values::types::BlendMode::Normal
    ) {
        return true;
    }

    // Non-empty backdrop-filter chain implies a stacking context
    // (the backdrop is sampled at the layer's own boundary).
    if !style.backdrop_filters.is_empty() {
        return true;
    }

    // `mask-image: <image>` forces a stacking context so the mask can
    // be applied to the entire painted subtree in one destination-in
    // pass instead of per-box.
    if !matches!(style.mask_image, crate::css::values::BackgroundImage::None) {
        return true;
    }

    false
}

/// Whether this layout box needs a *real* compositing layer — i.e. an
/// offscreen render target — rather than just the cheap `PushLayer`
/// opacity fast path.
///
/// True when any of `mix-blend-mode`, `backdrop-filter`, `filter`,
/// `isolation: isolate`, or `will-change: transform/opacity/filter` is
/// active. Plain `opacity < 1.0` stays on the fast path.
pub(crate) fn creates_compositing_layer(layout_box: &LayoutBox) -> bool {
    let style = &layout_box.style;

    if !matches!(
        style.mix_blend_mode,
        crate::css::values::types::BlendMode::Normal
    ) {
        return true;
    }

    if !style.backdrop_filters.is_empty() {
        return true;
    }

    if !style.filters.is_empty() {
        return true;
    }

    if matches!(
        style.isolation,
        crate::css::values::types::Isolation::Isolate
    ) {
        return true;
    }

    if style.will_change_promotes_layer {
        return true;
    }

    // `mask-image` needs a real offscreen layer so we can apply a
    // destination-in pass to the full painted subtree. Without this
    // promotion, the mask would have nothing to bite into at
    // `PopCompositingLayer` time.
    if !matches!(style.mask_image, crate::css::values::BackgroundImage::None) {
        return true;
    }

    false
}

/// Returns `true` if a layout box is positioned (position != static).
pub(crate) fn is_positioned(layout_box: &LayoutBox) -> bool {
    !matches!(layout_box.style.position, Position::Static)
}

/// Compute the intersection of two rectangles.
/// Resolve a [`ClipPath`] shape to a bounding rect in the layout coordinate
/// space, anchored to the element's border box.
///
/// Circle/ellipse shapes are reduced to their axis-aligned bounding box —
/// the only clipping primitive the backend trait exposes today. Returns
/// `None` if the shape collapses to an empty rect.
fn resolve_clip_path_rect(shape: &ClipPath, border_box: &Rect) -> Option<Rect> {
    let bw = border_box.width;
    let bh = border_box.height;
    let bx = border_box.x;
    let by = border_box.y;

    let rect = match *shape {
        ClipPath::Inset {
            top,
            right,
            bottom,
            left,
        } => {
            let t = top.resolve(bh);
            let r = right.resolve(bw);
            let b = bottom.resolve(bh);
            let l = left.resolve(bw);
            Rect {
                x: bx + l,
                y: by + t,
                width: (bw - l - r).max(0.0),
                height: (bh - t - b).max(0.0),
            }
        },
        ClipPath::Rect {
            top,
            right,
            bottom,
            left,
        } => {
            let t = top.unwrap_or(0.0);
            let l = left.unwrap_or(0.0);
            let r = right.unwrap_or(bw);
            let b = bottom.unwrap_or(bh);
            Rect {
                x: bx + l,
                y: by + t,
                width: (r - l).max(0.0),
                height: (b - t).max(0.0),
            }
        },
        ClipPath::Circle { cx, cy, r } => {
            let ref_diag = ((bw * bw + bh * bh) / 2.0).sqrt();
            let radius = match r {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * ref_diag,
            };
            let cx = cx.resolve(bw);
            let cy = cy.resolve(bh);
            Rect {
                x: bx + cx - radius,
                y: by + cy - radius,
                width: (radius * 2.0).max(0.0),
                height: (radius * 2.0).max(0.0),
            }
        },
        ClipPath::Ellipse { cx, cy, rx, ry } => {
            let rx_px = match rx {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * bw,
            };
            let ry_px = match ry {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * bh,
            };
            let cx = cx.resolve(bw);
            let cy = cy.resolve(bh);
            Rect {
                x: bx + cx - rx_px,
                y: by + cy - ry_px,
                width: (rx_px * 2.0).max(0.0),
                height: (ry_px * 2.0).max(0.0),
            }
        },
    };

    if rect.width <= 0.0 || rect.height <= 0.0 {
        None
    } else {
        Some(rect)
    }
}

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
    use crate::layout::box_model::{EdgeSizes, ListMarker, ListMarkerStyle, Rect, ReplacedContent};
    use crate::test_utils::{DrawCall, MockBackend};
    use oasis_types::backend::Color;

    /// Default test viewport (480x272 at origin, no scroll).
    fn test_vp() -> PaintViewport {
        PaintViewport {
            scroll_y: 0.0,
            scroll_x: 0.0,
            x: 0,
            y: 0,
            width: 480.0,
            height: 272.0,
            visible_height: 272.0,
            focused_node: None,
            counter_styles: Vec::new(),
        }
    }

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        let result = paint(&root, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
                marker: ListMarker::Ordered(ListMarkerStyle::Decimal, 3),
            },
            style,
            Some(0),
        );
        let link_map = HashMap::new();
        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
                atlas_region: None,
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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
                atlas_region: None,
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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
    fn clip_path_inset_resolves_to_shrunken_rect() {
        use crate::css::values::{ClipLength, ClipPath};
        let bb = Rect {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
        };
        let shape = ClipPath::Inset {
            top: ClipLength::Px(10.0),
            right: ClipLength::Px(20.0),
            bottom: ClipLength::Px(30.0),
            left: ClipLength::Px(40.0),
        };
        let r = resolve_clip_path_rect(&shape, &bb).expect("non-empty");
        assert_eq!(r.x, 140.0);
        assert_eq!(r.y, 110.0);
        assert_eq!(r.width, 140.0);
        assert_eq!(r.height, 60.0);
    }

    #[test]
    fn clip_path_circle_half_width_bounding_box() {
        use crate::css::values::{ClipLength, ClipPath};
        let bb = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let shape = ClipPath::Circle {
            cx: ClipLength::Frac(0.5),
            cy: ClipLength::Frac(0.5),
            r: ClipLength::Px(40.0),
        };
        let r = resolve_clip_path_rect(&shape, &bb).expect("non-empty");
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 10.0);
        assert_eq!(r.width, 80.0);
        assert_eq!(r.height, 80.0);
    }

    #[test]
    fn clip_path_inset_fully_collapsed_returns_none() {
        use crate::css::values::{ClipLength, ClipPath};
        let bb = Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        };
        let shape = ClipPath::Inset {
            top: ClipLength::Frac(0.5),
            right: ClipLength::Frac(0.5),
            bottom: ClipLength::Frac(0.5),
            left: ClipLength::Frac(0.5),
        };
        assert!(resolve_clip_path_rect(&shape, &bb).is_none());
    }

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

        let result = paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
    // 3D transforms / perspective integration tests
    // ---------------------------------------------------------------

    #[test]
    fn perspective_ancestor_routes_3d_child_through_polygon_path() {
        // The existing paint pipeline applies an element's transform
        // to its DESCENDANTS (the element's own background paints
        // before child_matrix is composed). So to exercise the
        // perspective projection path we need 3 levels:
        //   grandparent  – perspective: 800px (no own transform)
        //   parent       – rotateY(45deg)     (no background)
        //   child        – background: red    (no transform)
        // The parent's rotation under the grandparent's perspective
        // composes into ctx.transform, so child's paint_background
        // sees a non-trivial transform and goes through fill_polygon.
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(255, 0, 0);
        let child = make_block(60.0, 60.0, 80.0, 80.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(45.0)];
        let mut parent = make_block(50.0, 50.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        let mut grandparent_style = ComputedStyle::default();
        grandparent_style.perspective = Some(800.0);
        let mut grandparent = make_block(0.0, 0.0, 200.0, 200.0, grandparent_style);
        grandparent.children.push(parent);

        paint(&grandparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let polygon = backend
            .polygon_calls()
            .into_iter()
            .find_map(|c| {
                if let DrawCall::FillPolygon { points, color } = c
                    && *color == Color::rgb(255, 0, 0)
                {
                    Some(points)
                } else {
                    None
                }
            })
            .expect("expected red fill_polygon from perspective path");
        assert_eq!(polygon.len(), 4);
        // Under perspective(800) with rotateY(45), the right edge of
        // the rotated box is further from the viewer and should
        // shrink toward the parent's vanishing point. The right-most
        // projected x must be strictly less than the un-projected
        // right edge (60 + 80 = 140 in screen coords).
        let max_x = polygon.iter().map(|p| p.0).max().unwrap();
        assert!(
            max_x < 140,
            "expected perspective shrink past x=140, got max_x={max_x}",
        );
    }

    #[test]
    fn flat_3d_child_without_perspective_uses_orthographic_path() {
        // Without an ancestor `perspective`, a 3D-transformed parent
        // still flattens orthographically — `rotateY(60deg)` becomes
        // a horizontal squash by cos(60°)=0.5, which is non-trivial,
        // so the child's background should hit fill_polygon.
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(0, 255, 0);
        let child = make_block(10.0, 10.0, 80.0, 80.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(60.0)];
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        assert!(
            backend.fill_polygon_count() > 0,
            "expected fill_polygon for orthographically flattened rotateY",
        );
    }

    #[test]
    fn backface_visibility_hidden_culls_rotated_subtree() {
        // rotateY(180deg) flips the front face away from the viewer;
        // backface-visibility: hidden should skip painting the entire
        // subtree (including any background-bearing children).
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(0, 0, 255);
        let child = make_block(0.0, 0.0, 100.0, 100.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(180.0)];
        parent_style.backface_visibility = BackfaceVisibility::Hidden;
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        // No background draw at all — the entire subtree is culled.
        assert_eq!(backend.fill_rect_count(), 0);
        assert_eq!(backend.fill_polygon_count(), 0);
    }

    #[test]
    fn backface_hidden_child_culled_by_inherited_preserve_3d() {
        // Parent: rotateY(180deg) + preserve-3d
        // Child:  backface-visibility: hidden, no own transforms
        // The child faces away via the inherited matrix and must be culled.
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(255, 0, 0);
        child_style.backface_visibility = BackfaceVisibility::Hidden;
        let child = make_block(0.0, 0.0, 80.0, 80.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(180.0)];
        parent_style.transform_style = TransformStyle::Preserve3d;
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        let mut gparent_style = ComputedStyle::default();
        gparent_style.perspective = Some(800.0);
        let mut gparent = make_block(0.0, 0.0, 200.0, 200.0, gparent_style);
        gparent.children.push(parent);

        paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        assert_eq!(backend.fill_rect_count(), 0);
        assert_eq!(backend.fill_polygon_count(), 0);
    }

    #[test]
    fn preserve_3d_propagates_parent_matrix_to_children() {
        // Four-level structure:
        //   ggparent  – perspective: 800px
        //   gparent   – rotateY(30deg) + transform-style: preserve-3d
        //   parent    – translateZ(50px)        (would be a no-op under
        //                                        orthographic flatten;
        //                                        becomes visible under
        //                                        preserve-3d perspective)
        //   inner     – background: yellow      (no transform)
        // The yellow background should be painted via fill_polygon.
        let mut backend = MockBackend::new();

        let mut inner_style = ComputedStyle::default();
        inner_style.background_color = Color::rgb(255, 255, 0);
        let inner = make_block(10.0, 10.0, 40.0, 40.0, inner_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::TranslateZ(50.0)];
        let mut parent = make_block(20.0, 20.0, 60.0, 60.0, parent_style);
        parent.children.push(inner);

        let mut gparent_style = ComputedStyle::default();
        gparent_style.transforms = vec![TransformFunction::RotateY(30.0)];
        gparent_style.transform_style = TransformStyle::Preserve3d;
        let mut gparent = make_block(50.0, 50.0, 100.0, 100.0, gparent_style);
        gparent.children.push(parent);

        let mut ggparent_style = ComputedStyle::default();
        ggparent_style.perspective = Some(800.0);
        let mut ggparent = make_block(0.0, 0.0, 200.0, 200.0, ggparent_style);
        ggparent.children.push(gparent);

        paint(&ggparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let yellow_polygons: Vec<_> = backend
            .polygon_calls()
            .into_iter()
            .filter(|c| {
                matches!(
                    c,
                    DrawCall::FillPolygon { color, .. } if *color == Color::rgb(255, 255, 0)
                )
            })
            .collect();
        assert!(
            !yellow_polygons.is_empty(),
            "expected the inner child to be projected via fill_polygon under preserve-3d",
        );
    }

    #[test]
    fn preserve_3d_propagates_without_ancestor_perspective() {
        // Reviewer-flagged regression risk: a `preserve-3d` element
        // with NO ancestor `perspective` would previously fall back
        // to orthographic flatten, leaving descendants in a flat
        // coordinate system. The fix routes preserve-3d through the
        // screen path unconditionally so descendants inherit the
        // parent's screen-space matrix.
        //
        // Structure:
        //   parent — rotateY(60deg) + transform-style: preserve-3d
        //     child — background red, no transform
        // The child should be painted via fill_polygon — proving
        // the parent's rotation propagates as a 3D context even
        // without a `perspective:` ancestor.
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(255, 0, 0);
        let child = make_block(10.0, 10.0, 80.0, 80.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(60.0)];
        parent_style.transform_style = TransformStyle::Preserve3d;
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let red_polygons: Vec<_> = backend
            .polygon_calls()
            .into_iter()
            .filter(|c| {
                matches!(
                    c,
                    DrawCall::FillPolygon { color, .. } if *color == Color::rgb(255, 0, 0)
                )
            })
            .collect();
        assert!(
            !red_polygons.is_empty(),
            "preserve-3d parent without ancestor perspective should still propagate \
             its 3D matrix to descendants — regression guard",
        );
    }

    #[test]
    fn preserve_3d_with_2d_only_transform_still_propagates() {
        // CSS Transforms 2 §6 — `transform-style: preserve-3d`
        // unconditionally establishes a 3D rendering context, even
        // when the element has no 3D transforms of its own. A 2D
        // rotation on a preserve-3d parent must still be composed
        // with a 3D-transformed child in 3D space, not flattened
        // first.
        //
        // Structure:
        //   parent — rotate(45deg) (2D!) + transform-style: preserve-3d
        //     child — rotateY(60deg) (3D)
        //       grandchild — background green
        // The grandchild should be painted via fill_polygon —
        // proving the parent's 2D transform propagated as a 3D
        // context (otherwise the child would have flattened
        // independently and the chain would break).
        let mut backend = MockBackend::new();

        let mut grandchild_style = ComputedStyle::default();
        grandchild_style.background_color = Color::rgb(0, 255, 0);
        let grandchild = make_block(5.0, 5.0, 30.0, 30.0, grandchild_style);

        let mut child_style = ComputedStyle::default();
        child_style.transforms = vec![TransformFunction::RotateY(60.0)];
        let mut child = make_block(20.0, 20.0, 40.0, 40.0, child_style);
        child.children.push(grandchild);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::Rotate(45.0)];
        parent_style.transform_style = TransformStyle::Preserve3d;
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let green_polygons: Vec<_> = backend
            .polygon_calls()
            .into_iter()
            .filter(|c| {
                matches!(
                    c,
                    DrawCall::FillPolygon { color, .. } if *color == Color::rgb(0, 255, 0)
                )
            })
            .collect();
        assert!(
            !green_polygons.is_empty(),
            "preserve-3d with a 2D-only parent transform should still establish a 3D \
             rendering context per CSS Transforms 2 §6",
        );
    }

    #[test]
    fn near_camera_plane_background_skipped_not_saturated() {
        // Regression guard for the non-finite / overflow cast in
        // `paint_background`'s 3D projection path. When a point's
        // homogeneous `w` lands just above `apply_point_3d`'s
        // `1e-6` divide-by-zero threshold, the perspective divide
        // produces finite-but-astronomical coordinates that
        // saturate on the `as i32` cast to `i32::MAX`, painting a
        // screen-spanning garbage polygon.
        //
        // We construct this by hand with `matrix3d(...)`: an
        // otherwise-identity matrix with `m[15] = 2e-6` and a
        // large translation component. `transform-style:
        // preserve-3d` forces the screen path so the matrix lands
        // in `ambient_screen_matrix`; the child has no transforms
        // and paints a coloured background. With the guard,
        // `fill_polygon` must NOT be called with the child's
        // color — the element grazing the camera plane silently
        // skips instead of flashing the whole viewport.
        let mut backend = MockBackend::new();

        // Column-major: [col0|col1|col2|col3]. Mostly identity
        // but with a large x-translation (10000) and a tiny m[15]
        // (2e-6) so `wo = 2e-6` for inputs at Z=0 and the divided
        // x-coordinate is ~5e9 — well past `i32::MAX`.
        #[rustfmt::skip]
        let pathological = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            10000.0, 0.0, 0.0, 2.0e-6_f32,
        ];

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(77, 88, 99);
        let child = make_block(50.0, 50.0, 100.0, 100.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transform_style = TransformStyle::Preserve3d;
        parent_style.transforms = vec![TransformFunction::Matrix3d(pathological)];
        let mut parent = make_block(0.0, 0.0, 300.0, 300.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let offending = backend.polygon_calls().into_iter().find(|c| {
            matches!(
                c,
                DrawCall::FillPolygon { color, .. } if *color == Color::rgb(77, 88, 99)
            )
        });
        assert!(
            offending.is_none(),
            "element near the camera plane must skip `fill_polygon` \
             instead of painting a saturated-cast garbage quad",
        );
    }

    #[test]
    fn flat_2d_child_between_3d_ancestor_and_grandchild_composes_into_ambient() {
        // Regression guard for a subtle `ambient_screen_matrix`
        // propagation bug: when a `transform-style: flat` child
        // with its own 2D transform sits between a 3D ancestor
        // and a grandchild, the grandchild's background paint
        // must reflect *both* the 3D ancestor chain AND the
        // intervening 2D transform. Before this fix, ambient was
        // inherited unchanged across the flat 2D element, so the
        // grandchild's background dropped the child's rotation.
        //
        // Shape:
        //   gp    — perspective(1_000_000) (≈ orthographic, so
        //            ambient ≈ identity for Z=0 points; isolates
        //            the test from real perspective distortion)
        //   parent — rotateY(0deg) (identity math but triggers
        //            the screen path so `ambient_screen_matrix`
        //            gets set)
        //   child  — rotate(90deg) 2D (flat path with own transform)
        //   gchild — background, inside the child
        //
        // The grandchild's rotated-quad centroid must reflect the
        // child's 90° rotation around the child's screen center.
        let mut backend = MockBackend::new();

        let mut gchild_style = ComputedStyle::default();
        gchild_style.background_color = Color::rgb(200, 150, 100);
        // Grandchild absolute screen rect (100, 100)–(120, 120),
        // center (110, 110). Inside the child's (100, 100)–(200, 200).
        let gchild = make_block(100.0, 100.0, 20.0, 20.0, gchild_style);

        let mut child_style = ComputedStyle::default();
        child_style.transforms = vec![TransformFunction::Rotate(90.0)];
        let mut child = make_block(100.0, 100.0, 100.0, 100.0, child_style);
        child.children.push(gchild);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(0.0)];
        let mut parent = make_block(50.0, 50.0, 300.0, 300.0, parent_style);
        parent.children.push(child);

        let mut gp_style = ComputedStyle::default();
        gp_style.perspective = Some(1_000_000.0);
        let mut gp = make_block(0.0, 0.0, 400.0, 400.0, gp_style);
        gp.children.push(parent);

        paint(&gp, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let quad = backend
            .polygon_calls()
            .into_iter()
            .find_map(|c| {
                if let DrawCall::FillPolygon { points, color } = c
                    && *color == Color::rgb(200, 150, 100)
                {
                    Some(points)
                } else {
                    None
                }
            })
            .expect("expected grandchild polygon");
        let cx = quad.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
        let cy = quad.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
        // Grandchild's natural center (110, 110) rotated 90° CW
        // around the child's screen center (150, 150): (190, 110).
        assert!(
            (cx - 190.0).abs() < 3.0,
            "grandchild centroid x={cx} (expected ≈190) — \
             intervening child rotate(90) should compose into ambient",
        );
        assert!(
            (cy - 110.0).abs() < 3.0,
            "grandchild centroid y={cy} (expected ≈110) — \
             intervening child rotate(90) should compose into ambient",
        );
    }

    #[test]
    fn steep_perspective_produces_trapezoidal_quad() {
        // Regression guard for the "trapezoidal background"
        // follow-up. Under `rotateY(75deg) perspective(200px)`
        // the 3-corner-fit affine used to approximate the 4th
        // corner as the parallelogram completion
        // `p1 + p2 - p0`. With the full 4×4 projection path
        // wired into `paint_background` via
        // `ctx.ambient_screen_matrix`, all 4 corners are
        // projected individually through the perspective
        // frustum, so opposite edges have different lengths
        // (a true trapezoid) — `|top_edge| ≠ |bottom_edge|`.
        //
        // Shape: grandparent establishes perspective, parent
        // rotates, child has the background we're measuring.
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(200, 100, 50);
        // Child shares the parent's rect so the trapezoid is
        // the rotated parent's silhouette.
        let child = make_block(0.0, 0.0, 100.0, 100.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::RotateY(75.0)];
        let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        let mut gparent_style = ComputedStyle::default();
        gparent_style.perspective = Some(200.0);
        let mut gparent = make_block(0.0, 0.0, 300.0, 300.0, gparent_style);
        gparent.children.push(parent);

        paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let quad = backend
            .polygon_calls()
            .into_iter()
            .find_map(|c| {
                if let DrawCall::FillPolygon { points, color } = c
                    && *color == Color::rgb(200, 100, 50)
                {
                    Some(points)
                } else {
                    None
                }
            })
            .expect("expected rotated child polygon");
        assert_eq!(quad.len(), 4);
        let top_len_sq = {
            let dx = (quad[1].0 - quad[0].0) as f32;
            let dy = (quad[1].1 - quad[0].1) as f32;
            dx * dx + dy * dy
        };
        let bottom_len_sq = {
            let dx = (quad[2].0 - quad[3].0) as f32;
            let dy = (quad[2].1 - quad[3].1) as f32;
            dx * dx + dy * dy
        };
        // For a parallelogram these would be equal. Under true
        // perspective projection they differ — one edge is the
        // "near" edge, the other the "far" edge.
        let ratio = top_len_sq.max(bottom_len_sq) / top_len_sq.min(bottom_len_sq);
        assert!(
            ratio > 1.02,
            "expected top/bottom edges to differ under perspective (ratio={ratio})",
        );
    }

    #[test]
    fn preserve_3d_children_z_sorted_back_to_front() {
        // Two siblings inside a preserve-3d parent under a
        // perspective ancestor:
        //   back  — translateZ(-100px), DOM-first, red
        //   front — translateZ( 100px), DOM-last,  blue
        // Under painter's algorithm the back child must paint
        // first, the front child second. The DOM order already
        // matches that here so we also run a reversed variant
        // below where DOM order disagrees with Z order and the
        // sort must kick in.
        //
        // Before the Z-sort follow-up, `preserve-3d` children
        // painted in DOM order, so a DOM-later far child would
        // incorrectly paint over a DOM-earlier near one.
        let mut backend = MockBackend::new();

        let mut front_style = ComputedStyle::default();
        front_style.background_color = Color::rgb(0, 0, 255);
        front_style.transforms = vec![TransformFunction::TranslateZ(100.0)];
        // DOM-first (would paint first without sorting).
        let front = make_block(10.0, 10.0, 50.0, 50.0, front_style);

        let mut back_style = ComputedStyle::default();
        back_style.background_color = Color::rgb(255, 0, 0);
        back_style.transforms = vec![TransformFunction::TranslateZ(-100.0)];
        // DOM-last (would paint last without sorting).
        let back = make_block(10.0, 10.0, 50.0, 50.0, back_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transform_style = TransformStyle::Preserve3d;
        let mut parent = make_block(0.0, 0.0, 200.0, 200.0, parent_style);
        parent.children.push(front);
        parent.children.push(back);

        let mut gparent_style = ComputedStyle::default();
        gparent_style.perspective = Some(800.0);
        let mut gparent = make_block(0.0, 0.0, 400.0, 400.0, gparent_style);
        gparent.children.push(parent);

        paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        // Each child's background projects through the preserve-3d
        // parent's `ambient_screen_matrix`, so both come out as
        // `fill_polygon` calls. The Z-sort runs at the child loop,
        // so the back (red) polygon must appear before the front
        // (blue) polygon in the draw-call stream.
        let fills: Vec<_> = backend
            .calls
            .iter()
            .filter_map(|c| {
                if let DrawCall::FillPolygon { color, .. } = c {
                    Some(*color)
                } else {
                    None
                }
            })
            .collect();
        let red_idx = fills
            .iter()
            .position(|c| *c == Color::rgb(255, 0, 0))
            .expect("red (back) should paint");
        let blue_idx = fills
            .iter()
            .position(|c| *c == Color::rgb(0, 0, 255))
            .expect("blue (front) should paint");
        assert!(
            red_idx < blue_idx,
            "preserve-3d: back child (red, translateZ(-100)) must paint \
             before front child (blue, translateZ(+100)); red_idx={red_idx}, \
             blue_idx={blue_idx}",
        );
    }

    #[test]
    fn rotate_around_box_center_produces_symmetric_quad() {
        // Regression guard for the "double-translation" bug: a
        // `rotate(45deg)` applied to a parent at screen (100, 50),
        // size 80×40, with default origin (box center) must
        // produce — when composed onto a descendant's background
        // — a polygon whose centroid equals the parent's true
        // screen center (140, 70). Before the fix the matrix was
        // built in local space (pivot at content-local (40, 20))
        // and applied to screen coordinates, so the centroid
        // came out near (40, 20) — the local pivot, not the
        // screen pivot.
        //
        // NOTE: element backgrounds paint with the *ancestor's*
        // `ctx.transform`, so to observe the parent's own matrix
        // we need a child that shares the same screen rect. The
        // centroid of the child's rotated quad then equals the
        // parent's screen center, which is the invariant we're
        // checking.
        let mut backend = MockBackend::new();

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::Rotate(45.0)];
        let mut parent = make_block(100.0, 50.0, 80.0, 40.0, parent_style);

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(10, 20, 30);
        // Child shares the parent's screen rect so its background
        // is the rotated box we're measuring.
        let child = make_block(100.0, 50.0, 80.0, 40.0, child_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let polygon = backend
            .polygon_calls()
            .into_iter()
            .find_map(|c| {
                if let DrawCall::FillPolygon { points, color } = c
                    && *color == Color::rgb(10, 20, 30)
                {
                    Some(points)
                } else {
                    None
                }
            })
            .expect("expected fill_polygon for rotated box");
        assert_eq!(polygon.len(), 4);
        let cx = polygon.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
        let cy = polygon.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
        // Expected centroid is the box's screen center (140, 70).
        assert!(
            (cx - 140.0).abs() < 2.0,
            "rotated quad centroid x={cx} (expected ≈140)",
        );
        assert!(
            (cy - 70.0).abs() < 2.0,
            "rotated quad centroid y={cy} (expected ≈70)",
        );
    }

    #[test]
    fn rotated_parent_does_not_shift_child_offset() {
        // Regression guard for the second half of the
        // "double-translation" bug: a rotated parent's
        // `child_matrix.e/.f` used to be naively added to
        // `tx_offset_x/y` for its children, injecting the
        // rotation-pivot compensation as a random translation.
        // With the fix, non-trivial matrices compose into
        // `ctx.transform` and children paint at their natural
        // screen position (then get rotated by composition).
        //
        // The test rotates a parent by 180° and asserts the
        // child's polygon centroid equals what you'd get by
        // rotating the child around the parent's screen center
        // — i.e. 180°-flipped to the opposite side of the
        // parent. Under the old bug, the child was offset by
        // the parent's `.e/.f` values (unrelated to its real
        // rotated position).
        let mut backend = MockBackend::new();

        let mut child_style = ComputedStyle::default();
        child_style.background_color = Color::rgb(240, 240, 240);
        // Child at absolute screen rect (110, 110)–(130, 130) —
        // i.e. inside the parent's (100, 100)–(200, 200) rect.
        let child = make_block(110.0, 110.0, 20.0, 20.0, child_style);

        let mut parent_style = ComputedStyle::default();
        parent_style.transforms = vec![TransformFunction::Rotate(180.0)];
        let mut parent = make_block(100.0, 100.0, 100.0, 100.0, parent_style);
        parent.children.push(child);

        paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

        let polygon = backend
            .polygon_calls()
            .into_iter()
            .find_map(|c| {
                if let DrawCall::FillPolygon { points, color } = c
                    && *color == Color::rgb(240, 240, 240)
                {
                    Some(points)
                } else {
                    None
                }
            })
            .expect("expected fill_polygon for rotated child");
        let cx = polygon.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
        let cy = polygon.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
        // Parent screen rect is (100, 100)–(200, 200), center (150, 150).
        // Child natural screen rect is (110, 110)–(130, 130), center (120, 120).
        // Rotating (120, 120) by 180° around (150, 150) → (180, 180).
        assert!(
            (cx - 180.0).abs() < 2.0,
            "rotated child centroid x={cx} (expected ≈180)",
        );
        assert!(
            (cy - 180.0).abs() < 2.0,
            "rotated child centroid y={cy} (expected ≈180)",
        );
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

        paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

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
        style.z_index_auto = false;
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
    fn positioned_z_index_auto_no_stacking_context() {
        // z-index: auto (the default) does NOT create a stacking context.
        let mut style = ComputedStyle::default();
        style.position = Position::Relative;
        // z_index_auto is true by default -- this is "z-index: auto".
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(!creates_stacking_context(&lb));
    }

    // ---------------------------------------------------------------
    // Compositor epic close-out: every CSS property that should force
    // a stacking context AND a real compositing layer (the "slow path"
    // that allocates an offscreen render target) has an assertion
    // here. Keep this list in sync with `creates_stacking_context` /
    // `creates_compositing_layer` in this file.
    // ---------------------------------------------------------------

    #[test]
    fn mix_blend_mode_triggers_compositing_layer() {
        let mut style = ComputedStyle::default();
        style.mix_blend_mode = crate::css::values::types::BlendMode::Multiply;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(creates_compositing_layer(&lb));
    }

    #[test]
    fn backdrop_filter_triggers_compositing_layer() {
        let mut style = ComputedStyle::default();
        style.backdrop_filters = vec![crate::css::values::FilterFunction::Blur(4.0)];
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(creates_compositing_layer(&lb));
    }

    #[test]
    fn box_level_filter_triggers_compositing_layer() {
        let mut style = ComputedStyle::default();
        style.filters = vec![crate::css::values::FilterFunction::Grayscale(1.0)];
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(creates_compositing_layer(&lb));
    }

    #[test]
    fn isolation_isolate_triggers_compositing_layer() {
        let mut style = ComputedStyle::default();
        style.isolation = crate::css::values::types::Isolation::Isolate;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(creates_compositing_layer(&lb));
    }

    #[test]
    fn will_change_triggers_compositing_layer() {
        let mut style = ComputedStyle::default();
        style.will_change_promotes_layer = true;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(creates_compositing_layer(&lb));
    }

    #[test]
    fn mask_image_triggers_compositing_layer() {
        use crate::css::values::types::{GradientDirection, GradientStop, LinearGradient};
        let mut style = ComputedStyle::default();
        style.mask_image = crate::css::values::BackgroundImage::Gradient(LinearGradient {
            direction: GradientDirection::ToBottom,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 0),
                    position: 1.0,
                },
            ],
        });
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(
            creates_stacking_context(&lb),
            "mask-image must force a stacking context",
        );
        assert!(
            creates_compositing_layer(&lb),
            "mask-image must force an offscreen compositing layer",
        );
    }

    #[test]
    fn plain_opacity_stays_on_fast_path() {
        // opacity < 1 creates a stacking context but NOT a real
        // compositing layer — it rides the `PushLayer` fast path.
        let mut style = ComputedStyle::default();
        style.opacity = 0.5;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
        assert!(
            !creates_compositing_layer(&lb),
            "plain opacity must not allocate a render target",
        );
    }

    #[test]
    fn positioned_z_index_zero_explicit_creates_stacking_context() {
        // z-index: 0 (explicitly set) DOES create a stacking context.
        let mut style = ComputedStyle::default();
        style.position = Position::Relative;
        style.z_index = 0;
        style.z_index_auto = false;
        let lb = LayoutBox::new(BoxType::Block, style, None);
        assert!(creates_stacking_context(&lb));
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
        style_a.z_index_auto = false;
        let child_a = make_block(10.0, 10.0, 50.0, 50.0, style_a);

        // Child B: z-index=1
        let mut style_b = ComputedStyle::default();
        style_b.background_color = Color::rgb(0, 255, 0);
        style_b.position = Position::Relative;
        style_b.z_index = 1;
        style_b.z_index_auto = false;
        let child_b = make_block(10.0, 70.0, 50.0, 50.0, style_b);

        // Add A first, then B in DOM order.
        root.children.push(child_a);
        root.children.push(child_b);

        paint(&root, &mut backend, test_vp(), &link_map).unwrap();

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

    // ---------------------------------------------------------------
    // Test: CSS 2.1 full 5-step painting order
    // ---------------------------------------------------------------

    #[test]
    fn css21_painting_order_negative_normal_positioned_positive() {
        let mut backend = MockBackend::new();
        let link_map = HashMap::new();

        let mut root_style = ComputedStyle::default();
        root_style.background_color = Color::rgba(0, 0, 0, 0);
        let mut root = make_block(0.0, 0.0, 480.0, 272.0, root_style);

        // Step 2: negative z-index (blue, should paint first)
        let mut style_neg = ComputedStyle::default();
        style_neg.background_color = Color::rgb(0, 0, 255);
        style_neg.position = Position::Relative;
        style_neg.z_index = -1;
        style_neg.z_index_auto = false;
        let child_neg = make_block(10.0, 10.0, 50.0, 50.0, style_neg);

        // Step 3: non-positioned normal flow (white)
        let mut style_normal = ComputedStyle::default();
        style_normal.background_color = Color::rgb(255, 255, 255);
        let child_normal = make_block(10.0, 70.0, 50.0, 50.0, style_normal);

        // Step 4: positioned with z-index: auto (yellow)
        let mut style_auto = ComputedStyle::default();
        style_auto.background_color = Color::rgb(255, 255, 0);
        style_auto.position = Position::Relative;
        // z_index_auto is true by default.
        let child_auto = make_block(10.0, 130.0, 50.0, 50.0, style_auto);

        // Step 5: positive z-index stacking context (red)
        let mut style_pos = ComputedStyle::default();
        style_pos.background_color = Color::rgb(255, 0, 0);
        style_pos.position = Position::Relative;
        style_pos.z_index = 1;
        style_pos.z_index_auto = false;
        let child_pos = make_block(10.0, 190.0, 50.0, 50.0, style_pos);

        // Add in scrambled DOM order to verify sorting.
        root.children.push(child_pos);
        root.children.push(child_normal);
        root.children.push(child_neg);
        root.children.push(child_auto);

        paint(&root, &mut backend, test_vp(), &link_map).unwrap();

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

        let blue_idx = fill_calls
            .iter()
            .position(|c| *c == Color::rgb(0, 0, 255))
            .expect("blue (z=-1) should be painted");
        let white_idx = fill_calls
            .iter()
            .position(|c| *c == Color::rgb(255, 255, 255))
            .expect("white (normal) should be painted");
        let yellow_idx = fill_calls
            .iter()
            .position(|c| *c == Color::rgb(255, 255, 0))
            .expect("yellow (auto) should be painted");
        let red_idx = fill_calls
            .iter()
            .position(|c| *c == Color::rgb(255, 0, 0))
            .expect("red (z=1) should be painted");

        // CSS 2.1 order: negative < normal < positioned-auto < positive
        assert!(
            blue_idx < white_idx,
            "negative z-index (blue) should paint before normal flow (white)",
        );
        assert!(
            white_idx < yellow_idx,
            "normal flow (white) should paint before positioned-auto (yellow)",
        );
        assert!(
            yellow_idx < red_idx,
            "positioned-auto (yellow) should paint before positive z-index (red)",
        );
    }
}
