//! Display list recording: walks the layout tree and emits [`DisplayItem`]s.
//!
//! This is the only painter: it never calls backend draw methods. The
//! recorded [`DisplayList`] is cached by the widget and replayed (see
//! `display_list/replay.rs`), or recorded and replayed in one go by
//! [`super::paint`].

use std::collections::HashMap;

use crate::css::values::types::{BackfaceVisibility, TransformStyle};
use crate::css::values::{
    BorderStyle, Dimension, Overflow, Position, TextDecorationStyle, TextOverflow, Visibility,
    WhiteSpace,
};
use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, Rect};
use crate::transform::{AffineTransform2D, Matrix3d};
use oasis_types::backend::Color;

use super::display_list::{DisplayItem, DisplayList, sticky_offset};
use super::intersect_rects;
use super::{LinkRegion, PaintViewport};

mod controls;
mod decorations;
#[cfg(test)]
mod form_tests;
mod replaced;
mod text;

// The component recorders see each other (and this module's private
// context types) through these.
#[allow(unused_imports)]
use {controls::*, decorations::*, replaced::*, text::*};

// -------------------------------------------------------------------
// Internal recording context
// -------------------------------------------------------------------

/// Mutable state threaded through the recursive recording walk.
struct RecordContext<'a> {
    /// Accumulated link regions.
    links: Vec<LinkRegion>,
    /// When recording inside an `<a>` element, holds `(href, node_id)`.
    current_link: Option<(String, NodeId)>,
    /// Vertical scroll offset (includes nested container offsets).
    scroll_y: f32,
    /// Horizontal scroll offset (includes nested container offsets).
    scroll_x: f32,
    /// Viewport height for offscreen culling (includes buffer zone).
    viewport_height: f32,
    /// Viewport width for offscreen culling.
    viewport_width: f32,
    /// True visible viewport height (excludes buffer zone).
    /// Used for sticky positioning so elements stick to the visible
    /// area, not the extended culling boundary.
    visible_viewport_height: f32,
    /// Screen y of the viewport's top edge; sticky offsets are measured
    /// from it, not from the top of the screen.
    viewport_y: i32,
    /// Active clipping rectangle from ancestor `overflow: hidden` boxes.
    clip_rect: Option<Rect>,
    /// When true, text overflowing the clip rect gets "..." appended.
    text_overflow_ellipsis: bool,
    /// Current DOM node being recorded (for hover color patching).
    current_node: Option<NodeId>,
    /// Per-element scroll offsets for nested scroll containers.
    /// Maps DOM node IDs to `(scroll_x, scroll_y)` pixel offsets.
    nested_scroll_offsets: &'a HashMap<NodeId, (f32, f32)>,
    /// DOM node id with keyboard focus, used to draw a caret in
    /// focused `<input>` / `<textarea>` form controls.
    focused_node: Option<NodeId>,
    /// `@counter-style` rules for custom list markers.
    counter_styles: Vec<crate::css::parser::CounterStyleRule>,
    /// Accumulated 2D affine transform from ancestor elements, in
    /// screen space. Descendant backgrounds project through it when it
    /// is not a pure translation.
    transform: AffineTransform2D,
    /// Inherited perspective frame from a `perspective:` ancestor.
    /// Children of an element with `perspective: d` see their 3D
    /// transforms projected through `Persp(d)` centred at the
    /// ancestor's `perspective-origin`.
    perspective_context: Option<PerspectiveContext>,
    /// Accumulated 3D matrix from `transform-style: preserve-3d`
    /// ancestors, in screen space. `None` means "no preserved 3D
    /// context active".
    preserved_3d: Option<Matrix3d>,
    /// Full screen-space 4x4 matrix of the nearest 3D-transformed
    /// ancestor. Descendant backgrounds project all four corners
    /// through it (a true trapezoid under steep perspective) instead
    /// of the 3-corner affine fit in `transform`.
    ambient_screen_matrix: Option<Matrix3d>,
}

/// A perspective frustum inherited from a `perspective:` ancestor.
///
/// `vanishing_x`/`vanishing_y` are the absolute screen-space
/// coordinates of the perspective vanishing point (the ancestor's
/// content box offset by `perspective-origin`).
#[derive(Debug, Clone, Copy)]
struct PerspectiveContext {
    distance: f32,
    vanishing_x: f32,
    vanishing_y: f32,
}

// -------------------------------------------------------------------
// Public entry point
// -------------------------------------------------------------------

/// Record a layout tree into `display_list` (cleared first) and return
/// the link hit regions.
///
/// `nested_scroll_offsets` maps scroll-container node ids
/// (`overflow: auto/scroll`) to their `(scroll_x, scroll_y)`.
pub fn record(
    layout: &LayoutBox,
    viewport: PaintViewport,
    link_map: &HashMap<NodeId, String>,
    display_list: &mut DisplayList,
    nested_scroll_offsets: &HashMap<NodeId, (f32, f32)>,
) -> Vec<LinkRegion> {
    display_list.clear();
    display_list.set_recording_scroll_y(viewport.scroll_y);

    let mut ctx = RecordContext {
        links: Vec::new(),
        current_link: None,
        scroll_y: viewport.scroll_y,
        scroll_x: viewport.scroll_x,
        viewport_height: viewport.height,
        viewport_width: viewport.width,
        visible_viewport_height: viewport.visible_height,
        viewport_y: viewport.y,
        clip_rect: None,
        text_overflow_ellipsis: false,
        current_node: None,
        nested_scroll_offsets,
        focused_node: viewport.focused_node,
        counter_styles: viewport.counter_styles.clone(),
        transform: AffineTransform2D::identity(),
        perspective_context: None,
        preserved_3d: None,
        ambient_screen_matrix: None,
    };

    record_box(
        layout,
        display_list,
        viewport.x,
        viewport.y,
        &mut ctx,
        link_map,
    );

    ctx.links
}

// -------------------------------------------------------------------
// Recursive box recorder
// -------------------------------------------------------------------

/// Which kind of compositing layer a `record_box` call opened for
/// the current box, so the matching pop can close it.
#[derive(Copy, Clone, Debug)]
enum LayerKind {
    /// Fast path: `PushLayer { opacity }`. Opacity only.
    Opacity,
    /// Slow path: `PushCompositingLayer { .. }`. Needs a real
    /// offscreen render target to paint correctly; backends without
    /// that capability fall back to the opacity fast path during
    /// replay.
    Compositing,
}

fn record_box(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut RecordContext,
    link_map: &HashMap<NodeId, String>,
) {
    let is_sticky = layout_box.style.position == Position::Sticky;
    let sticky_dy = if is_sticky {
        push_sticky_group(layout_box, dl, offset_y, ctx)
    } else {
        0
    };
    let offset_y = offset_y + sticky_dy;

    // Translate-only transforms move the box itself, so culling must
    // account for them. Rotations/scales are not folded in: their
    // matrix translation is pivot compensation, not a box offset.
    let (tx_off_x, tx_off_y) = if layout_box.style.transforms.is_empty() {
        (offset_x, offset_y)
    } else {
        let m = super::transforms::compute_transform_matrix(
            &layout_box.style.transforms,
            layout_box.style.transform_origin.as_ref(),
            &layout_box.dimensions.content,
        );
        if m.is_translation_only() {
            (offset_x + m.e as i32, offset_y + m.f as i32)
        } else {
            (offset_x, offset_y)
        }
    };

    // Screen-space culling (includes CSS transform translation). Runs
    // BEFORE the 3D ambient context is mutated below — otherwise an early
    // return here would leave `ctx.ambient_screen_matrix` set for siblings.
    let screen_y = layout_box.dimensions.content.y - ctx.scroll_y
        + sticky_dy as f32
        + (tx_off_y - offset_y) as f32;
    let box_bottom = screen_y + layout_box.dimensions.margin_box().height;
    let screen_x = layout_box.dimensions.content.x - ctx.scroll_x + (tx_off_x - offset_x) as f32;
    let box_right = screen_x + layout_box.dimensions.margin_box().width;

    // A box that doesn't clip may have descendants overflowing into the
    // viewport even when its own box is off-screen (e.g. a `height: 100%`
    // body holding the whole page), so only cull it when it can't.
    let offscreen = box_bottom < 0.0
        || screen_y > ctx.viewport_height
        || box_right < 0.0
        || screen_x > ctx.viewport_width;
    if offscreen && (layout_box.clips_overflow() || layout_box.children.is_empty()) {
        // Close the sticky group opened above to keep the stack balanced.
        if is_sticky {
            dl.push(DisplayItem::PopSticky);
        }
        return;
    }

    // backface-visibility: hidden — when the element's effective 3D
    // transform (its own transforms composed onto any inherited
    // preserve-3d matrix) turns its front face away from the viewer,
    // skip the entire subtree.
    if layout_box.style.backface_visibility == BackfaceVisibility::Hidden
        && (!layout_box.style.transforms.is_empty() || ctx.preserved_3d.is_some())
    {
        let content = &layout_box.dimensions.content;
        let (ox, oy, oz) =
            super::resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        let local_m3d = if layout_box.style.transforms.is_empty() {
            Matrix3d::identity()
        } else {
            Matrix3d::from_css_transforms_3d(&layout_box.style.transforms, ox, oy, oz)
        };
        let effective = match ctx.preserved_3d {
            Some(p) => p.multiply(&local_m3d),
            None => local_m3d,
        };
        if effective.front_face_normal_z(content.width, content.height) < 0.0 {
            if is_sticky {
                dl.push(DisplayItem::PopSticky);
            }
            return;
        }
    }

    // An off-screen box that survived culling is only visited for its
    // overflowing descendants; it paints nothing of its own.
    let is_visible = layout_box.style.visibility == Visibility::Visible && !offscreen;

    // Track the current DOM node for hover color patching.
    let prev_node = ctx.current_node;
    if layout_box.node.is_some() {
        ctx.current_node = layout_box.node;
    }

    // Track link entry.
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

    // The element's transform. It applies to the element's own box
    // (decorations, clip, compositing bounds, link region) and to its
    // descendants. Two paths:
    //   * Screen-space 3D path: the element participates in a 3D
    //     rendering context (3D transforms under a perspective
    //     ancestor, an inherited preserve-3d matrix, or its own
    //     `transform-style: preserve-3d`, which per CSS Transforms 2 §6
    //     always establishes one). The full 4x4 screen matrix is built
    //     and the box rect projected through it.
    //   * Flat path: the 2D affine flatten, pivoting on the screen-
    //     space transform origin. Used for plain 2D rotates/scales and
    //     for 3D transforms with no perspective ancestor.
    let content = &layout_box.dimensions.content;
    let has_3d_transforms = super::transforms_have_3d(&layout_box.style.transforms);
    let needs_screen_path = ctx.preserved_3d.is_some()
        || (ctx.perspective_context.is_some() && has_3d_transforms)
        || layout_box.style.transform_style == TransformStyle::Preserve3d;
    let sx = content.x - ctx.scroll_x + offset_x as f32;
    let sy = content.y - ctx.scroll_y + offset_y as f32;
    let mut this_element_screen_matrix: Option<Matrix3d> = None;
    let child_matrix = if needs_screen_path {
        let (ox_l, oy_l, oz_l) =
            super::resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        let cox = sx + ox_l;
        let coy = sy + oy_l;
        let local_no_origin =
            Matrix3d::from_css_transforms_3d(&layout_box.style.transforms, 0.0, 0.0, 0.0);
        let m_local_screen = Matrix3d::translate(cox, coy, oz_l)
            .multiply(&local_no_origin)
            .multiply(&Matrix3d::translate(-cox, -coy, -oz_l));
        let m_preserved = match ctx.preserved_3d {
            Some(p) => p.multiply(&m_local_screen),
            None => m_local_screen,
        };
        // Apply the perspective frustum on the outside, unless an
        // inherited preserve-3d matrix already contains it.
        let m_screen = if ctx.preserved_3d.is_none()
            && let Some(persp) = ctx.perspective_context
        {
            Matrix3d::translate(persp.vanishing_x, persp.vanishing_y, 0.0)
                .multiply(&Matrix3d::perspective(persp.distance))
                .multiply(&Matrix3d::translate(
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
        AffineTransform2D::identity()
    } else {
        // Flat 2D path, built in SCREEN space: the pivot is the
        // screen-space transform origin, so descendants project through
        // `ctx.transform` without any extra offset.
        let (ox_l, oy_l, _oz) =
            super::resolve_transform_origin(layout_box.style.transform_origin.as_ref(), content);
        AffineTransform2D::from_css_transforms(&layout_box.style.transforms, sx + ox_l, sy + oy_l)
    };
    let prev_transform = ctx.transform;
    if !child_matrix.is_translation_only() {
        ctx.transform = ctx.transform.multiply(&child_matrix);
    }
    // Child offsets: the screen path and the non-trivial flat path
    // carry all geometry in the matrices; only a translate-only flat
    // transform moves descendants through the integer offset.
    let (tx_offset_x, tx_offset_y) = if !needs_screen_path && child_matrix.is_translation_only() {
        (
            offset_x + child_matrix.e as i32,
            offset_y + child_matrix.f as i32,
        )
    } else {
        (offset_x, offset_y)
    };

    // Ambient 4x4 matrix for this box's and its descendants'
    // backgrounds: this element's
    // screen matrix if it took the screen path; otherwise a non-trivial
    // 2D transform is lifted to 4D and composed onto the inherited
    // ambient so it isn't dropped between a 3D ancestor and a
    // grandchild.
    let prev_ambient = ctx.ambient_screen_matrix;
    if let Some(m) = this_element_screen_matrix {
        ctx.ambient_screen_matrix = Some(m);
    } else if let Some(parent_ambient) = ctx.ambient_screen_matrix
        && !child_matrix.is_translation_only()
    {
        ctx.ambient_screen_matrix =
            Some(parent_ambient.multiply(&Matrix3d::from_2d_affine(child_matrix)));
    }

    let layer_kind = push_layer(layout_box, dl, ctx, tx_offset_x, tx_offset_y);
    let needs_layer = layer_kind.is_some();

    // When opacity is handled by PushLayer, recording should use 1.0
    // to avoid applying opacity twice (once during recording, once during replay).
    let effective_opacity = if needs_layer {
        1.0
    } else {
        layout_box.style.opacity
    };

    if is_visible {
        // 0. Box shadow.
        record_box_shadow(
            layout_box,
            dl,
            tx_offset_x,
            tx_offset_y,
            ctx,
            effective_opacity,
        );

        // 1. Background.
        record_background(
            layout_box,
            dl,
            tx_offset_x,
            tx_offset_y,
            ctx,
            effective_opacity,
        );

        // 2. Borders.
        record_borders(layout_box, dl, tx_offset_x, tx_offset_y, ctx);

        // 2b. Outline.
        record_outline(layout_box, dl, tx_offset_x, tx_offset_y, ctx);
    }

    // Overflow clipping and `clip-path`.
    let prev_clip = ctx.clip_rect;
    let prev_ellipsis = ctx.text_overflow_ellipsis;
    let prev_scroll_x = ctx.scroll_x;
    let prev_scroll_y = ctx.scroll_y;
    let has_overflow_clip = matches!(
        layout_box.style.overflow,
        Overflow::Hidden | Overflow::Scroll | Overflow::Auto
    );
    // `ctx.clip_rect` is in screen space (layout minus scroll plus the
    // box offset) so clips from different nested scroll containers
    // intersect, and cull children, in one coordinate space.
    let to_screen = |r: Rect| Rect {
        x: r.x - prev_scroll_x + tx_offset_x as f32,
        y: r.y - prev_scroll_y + tx_offset_y as f32,
        ..r
    };
    if has_overflow_clip {
        let new_clip = to_screen(layout_box.dimensions.content);
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
    // CSS `clip-path`: intersect with the resolved shape's bounding
    // rect. True shape clipping (circle/ellipse edges) needs stencil
    // support the backend trait does not expose, so the element is
    // clipped to the shape's bounding box.
    if let Some(shape) = &layout_box.style.clip_path
        && let Some(shape_rect) =
            super::resolve_clip_path_rect(shape, &layout_box.dimensions.border_box())
    {
        let shape_rect = to_screen(shape_rect);
        ctx.clip_rect = Some(match ctx.clip_rect {
            Some(existing) => intersect_rects(existing, shape_rect),
            None => shape_rect,
        });
    }
    let pushed_clip = ctx.clip_rect != prev_clip;
    if pushed_clip && let Some(clipped) = ctx.clip_rect {
        dl.push(DisplayItem::PushClip {
            x: clipped.x as i32,
            y: clipped.y as i32,
            w: clipped.width.max(0.0) as u32,
            h: clipped.height.max(0.0) as u32,
        });
    }
    // Apply the nested scroll offset for this container AFTER pushing
    // the clip so the clip boundary uses the parent's scroll offset,
    // not this container's own scroll.
    if has_overflow_clip
        && let Some(nid) = layout_box.node
        && let Some(&(sx, sy)) = ctx.nested_scroll_offsets.get(&nid)
    {
        ctx.scroll_x += sx;
        ctx.scroll_y += sy;
    }

    // A `perspective:` element establishes a frustum for its children,
    // centred at its `perspective-origin` (resolved against the border
    // box, the transform reference box) in screen space.
    let prev_perspective_context = ctx.perspective_context;
    if let Some(d) = layout_box.style.perspective
        && d > 0.0
    {
        let border = layout_box.dimensions.border_box();
        let (po_x, po_y) = super::resolve_perspective_origin(
            layout_box.style.perspective_origin.as_ref(),
            &border,
        );
        ctx.perspective_context = Some(PerspectiveContext {
            distance: d,
            vanishing_x: border.x - ctx.scroll_x + tx_offset_x as f32 + po_x,
            vanishing_y: border.y - ctx.scroll_y + tx_offset_y as f32 + po_y,
        });
    }

    // `transform-style: preserve-3d` hands this element's 4x4 matrix to
    // descendants; the default `flat` flushes the preserved context.
    let preserve3d_parent = if layout_box.style.transform_style == TransformStyle::Preserve3d {
        this_element_screen_matrix
    } else {
        None
    };
    let prev_preserved_3d = ctx.preserved_3d;
    ctx.preserved_3d = preserve3d_parent;

    // Children / inline / replaced / markers.
    match &layout_box.box_type {
        BoxType::Block
        | BoxType::Flex
        | BoxType::Grid
        | BoxType::Anonymous
        | BoxType::TableWrapper
        | BoxType::TableRow
        | BoxType::TableCell
        | BoxType::InlineBlock => {
            record_block_children(
                layout_box,
                preserve3d_parent,
                dl,
                tx_offset_x,
                tx_offset_y,
                ctx,
                link_map,
            );
        },
        BoxType::Inline => {
            if is_visible {
                record_inline_content(layout_box, dl, tx_offset_x, tx_offset_y, ctx, link_map);
            }
        },
        BoxType::ListItem { marker } => {
            if is_visible {
                record_list_marker(marker, layout_box, dl, tx_offset_x, tx_offset_y, ctx);
            }
            for child in &layout_box.children {
                record_box(child, dl, tx_offset_x, tx_offset_y, ctx, link_map);
            }
        },
        BoxType::Replaced(replaced) => {
            if is_visible {
                record_replaced(replaced, layout_box, dl, tx_offset_x, tx_offset_y, ctx);
            }
        },
    }

    // Close the compositing layer (must be before clip restore so clip
    // state is still correct for any future render-target compositing).
    match layer_kind {
        Some(LayerKind::Compositing) => dl.push(DisplayItem::PopCompositingLayer),
        Some(LayerKind::Opacity) => dl.push(DisplayItem::PopLayer),
        None => {},
    }

    // Restore clip.
    if pushed_clip {
        dl.push(DisplayItem::PopClip);
    }
    ctx.clip_rect = prev_clip;
    ctx.text_overflow_ellipsis = prev_ellipsis;
    ctx.scroll_x = prev_scroll_x;
    ctx.scroll_y = prev_scroll_y;

    // Close the sticky group (outermost, pushed before opacity).
    if is_sticky {
        dl.push(DisplayItem::PopSticky);
    }

    // Restore the previous DOM node context.
    ctx.current_node = prev_node;

    // Record link hit region.
    if let Some((ref href, link_node)) = ctx.current_link {
        if !offscreen && (layout_box.node == Some(link_node) || has_text_content(layout_box)) {
            let border = layout_box.dimensions.border_box();
            ctx.links.push(LinkRegion {
                rect: Rect {
                    x: border.x - ctx.scroll_x + tx_offset_x as f32,
                    y: border.y - ctx.scroll_y + tx_offset_y as f32,
                    width: border.width,
                    height: border.height,
                },
                href: href.clone(),
                node: link_node,
            });
        }
    }

    // Restore the transform context.
    ctx.transform = prev_transform;
    ctx.perspective_context = prev_perspective_context;
    ctx.preserved_3d = prev_preserved_3d;
    ctx.ambient_screen_matrix = prev_ambient;

    // Reset link tracking.
    if entered_link {
        if let Some(node_id) = layout_box.node {
            if ctx
                .current_link
                .as_ref()
                .is_some_and(|(_, n)| *n == node_id)
            {
                ctx.current_link = None;
            }
        }
    }
}

/// Open a `position: sticky` group for `layout_box` so replay can
/// re-evaluate the pin offset for a scroll delta without re-recording,
/// and return the offset at the recording scroll.
fn push_sticky_group(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_y: i32,
    ctx: &RecordContext,
) -> i32 {
    let px = |d: &Dimension| match d {
        Dimension::Px(v) => Some(*v),
        _ => None,
    };
    let natural_y = layout_box.dimensions.content.y + (offset_y - ctx.viewport_y) as f32;
    let box_height = layout_box.dimensions.margin_box().height;
    let top_px = px(&layout_box.style.top);
    let bottom_px = px(&layout_box.style.bottom);
    let visible_viewport_h = ctx.visible_viewport_height;
    dl.push(DisplayItem::PushSticky {
        natural_y,
        box_height,
        top_px,
        bottom_px,
        visible_viewport_h,
    });
    sticky_offset(
        natural_y,
        box_height,
        top_px,
        bottom_px,
        visible_viewport_h,
        ctx.scroll_y,
    )
}

/// Open the compositing layer (if any) `layout_box` needs and return
/// its kind, so the caller can close it after the subtree:
///
/// * Slow path (`PushCompositingLayer`) — required for
///   `mix-blend-mode`, `backdrop-filter`, box-level `filter`, masks,
///   `isolation: isolate`, or `will-change: transform/opacity/filter`.
///   Backends without `SdiRenderTarget` support fall back to plain
///   opacity stacking at replay.
/// * Fast path (`PushLayer`) — the opacity-only common case. No
///   offscreen surface needed.
fn push_layer(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    ctx: &RecordContext,
    offset_x: i32,
    offset_y: i32,
) -> Option<LayerKind> {
    let style = &layout_box.style;
    if !super::creates_compositing_layer(layout_box) {
        if style.opacity < 1.0 {
            dl.push(DisplayItem::PushLayer {
                opacity: style.opacity,
            });
            return Some(LayerKind::Opacity);
        }
        return None;
    }
    // The margin box covers box shadow, borders and contents; it is
    // in screen space like every other item.
    let mb = layout_box.dimensions.margin_box();
    let bounds = Rect {
        x: mb.x - ctx.scroll_x + offset_x as f32,
        y: mb.y - ctx.scroll_y + offset_y as f32,
        width: mb.width,
        height: mb.height,
    };
    let mask = match &style.mask_image {
        crate::css::values::BackgroundImage::None => None,
        image => Some(crate::paint::display_list::MaskParams {
            image: image.clone(),
            mode: style.mask_mode,
            composite: style.mask_composite,
            // URL masks carry the decoded image bytes so replay can
            // sample without a GPU round-trip; gradient masks have none.
            texture: if matches!(image, crate::css::values::BackgroundImage::Url(_)) {
                layout_box.mask_image_data.clone()
            } else {
                None
            },
            size: style.mask_size,
            position: style.mask_position,
            repeat: style.mask_repeat,
        }),
    };
    dl.push(DisplayItem::PushCompositingLayer {
        bounds,
        opacity: style.opacity,
        blend: style.mix_blend_mode,
        filters: style.filters.clone(),
        mask,
    });
    Some(LayerKind::Compositing)
}

/// Record the children of a block-level container in CSS 2.1 painting
/// order (appendix E), after the box's own background and borders:
///
/// 1. stacking-context children with negative z-index (ascending),
/// 2. non-positioned children in tree order,
/// 3. positioned children with `z-index: auto` in tree order,
/// 4. stacking-context children with z-index >= 0 (ascending).
///
/// Inside a `preserve-3d` context (`preserve3d_parent`) the children
/// without an explicit z-index are instead Z-sorted back to front.
/// Children entirely outside the active clip are skipped.
fn record_block_children(
    layout_box: &LayoutBox,
    preserve3d_parent: Option<Matrix3d>,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut RecordContext,
    link_map: &HashMap<NodeId, String>,
) {
    let mut normal_children: Vec<&LayoutBox> = Vec::new();
    let mut positioned_auto: Vec<&LayoutBox> = Vec::new();
    let mut stacking_neg: Vec<(i32, usize, &LayoutBox)> = Vec::new();
    let mut stacking_pos: Vec<(i32, usize, &LayoutBox)> = Vec::new();

    for (idx, child) in layout_box.children.iter().enumerate() {
        // Children with an explicit z-index opt out of the 3D context
        // (CSS Transforms 2 §6.1) and keep the CSS 2.1 stacking tiers.
        let stacking = super::creates_stacking_context(child);
        let in_3d_sort = preserve3d_parent.is_some() && (child.style.z_index_auto || !stacking);
        if in_3d_sort {
            normal_children.push(child);
        } else if stacking {
            if child.style.z_index < 0 {
                stacking_neg.push((child.style.z_index, idx, child));
            } else {
                stacking_pos.push((child.style.z_index, idx, child));
            }
        } else if super::is_positioned(child) {
            positioned_auto.push(child);
        } else {
            normal_children.push(child);
        }
    }

    // Back-to-front Z sort of the 3D context's children (stable, so
    // ties keep DOM order).
    if let Some(m_parent) = preserve3d_parent
        && normal_children.len() > 1
    {
        let mut z_keys: Vec<(f32, &LayoutBox)> = normal_children
            .iter()
            .map(|c| {
                (
                    preserve3d_child_z(c, &m_parent, ctx, offset_x, offset_y),
                    *c,
                )
            })
            .collect();
        z_keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        normal_children = z_keys.into_iter().map(|(_, c)| c).collect();
    }
    stacking_neg.sort_by_key(|&(z, idx, _)| (z, idx));
    stacking_pos.sort_by_key(|&(z, idx, _)| (z, idx));

    let ordered = stacking_neg
        .iter()
        .map(|&(_, _, c)| c)
        .chain(normal_children)
        .chain(positioned_auto)
        .chain(stacking_pos.iter().map(|&(_, _, c)| c));
    for child in ordered {
        if let Some(clip) = &ctx.clip_rect {
            let cb = child.dimensions.border_box();
            let x = cb.x - ctx.scroll_x + offset_x as f32;
            let y = cb.y - ctx.scroll_y + offset_y as f32;
            if y + cb.height < clip.y
                || y > clip.y + clip.height
                || x + cb.width < clip.x
                || x > clip.x + clip.width
            {
                continue;
            }
        }
        record_box(child, dl, offset_x, offset_y, ctx, link_map);
    }
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Sort key for `transform-style: preserve-3d` child ordering: the Z
/// (after the perspective divide) of the child's centre once its own
/// local 3D transform and the parent's screen matrix are applied.
/// Smaller Z is farther from the viewer, so ascending order is
/// back-to-front.
fn preserve3d_child_z(
    child: &LayoutBox,
    parent_screen_matrix: &Matrix3d,
    ctx: &RecordContext,
    offset_x: i32,
    offset_y: i32,
) -> f32 {
    let c = &child.dimensions.content;
    let ccx = c.x - ctx.scroll_x + offset_x as f32 + c.width / 2.0;
    let ccy = c.y - ctx.scroll_y + offset_y as f32 + c.height / 2.0;
    let child_local = if child.style.transforms.is_empty() {
        Matrix3d::identity()
    } else {
        let (ox_l, oy_l, oz_l) =
            super::resolve_transform_origin(child.style.transform_origin.as_ref(), c);
        let cox = c.x - ctx.scroll_x + offset_x as f32 + ox_l;
        let coy = c.y - ctx.scroll_y + offset_y as f32 + oy_l;
        let local = Matrix3d::from_css_transforms_3d(&child.style.transforms, 0.0, 0.0, 0.0);
        Matrix3d::translate(cox, coy, oz_l)
            .multiply(&local)
            .multiply(&Matrix3d::translate(-cox, -coy, -oz_l))
    };
    let (_, _, z) = parent_screen_matrix
        .multiply(&child_local)
        .apply_point_3d(ccx, ccy, 0.0);
    z
}

/// Returns `true` if the layout box or any descendant is an inline
/// box (i.e. can carry text), used to decide which boxes contribute
/// to a link's hit region.
pub(super) fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

fn apply_opacity(color: Color, opacity: f32) -> Color {
    color.apply_opacity(opacity)
}
