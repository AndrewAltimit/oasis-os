//! Display list recording: walks the layout tree and emits [`DisplayItem`]s.
//!
//! This is the display-list counterpart of the immediate-mode paint pass in
//! `mod.rs`. Instead of calling backend draw methods directly, it records
//! operations into a [`DisplayList`] that can be cached and replayed.

use std::collections::HashMap;

use crate::css::values::{
    BorderStyle, Dimension, FilterFunction, Overflow, Position, TextOverflow, Visibility,
    WhiteSpace,
};
use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, Rect};
use oasis_types::backend::Color;

use super::display_list::{DisplayItem, DisplayList};
use super::filters;
use super::{LinkRegion, PaintViewport};

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
}

// -------------------------------------------------------------------
// Public entry point
// -------------------------------------------------------------------

/// Record a layout tree into a display list.
///
/// Returns the display list and link regions, equivalent to [`super::paint`]
/// but without issuing any draw calls.
pub fn record(
    layout: &LayoutBox,
    viewport: PaintViewport,
    link_map: &HashMap<NodeId, String>,
    display_list: &mut DisplayList,
) -> Vec<LinkRegion> {
    let empty = HashMap::new();
    record_with_scroll(layout, viewport, link_map, display_list, &empty)
}

/// Record a layout tree into a display list with nested scroll offsets.
///
/// Like [`record`] but accepts per-element scroll offsets for nested
/// scroll containers (`overflow: auto/scroll`).
pub fn record_with_scroll(
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
        clip_rect: None,
        text_overflow_ellipsis: false,
        current_node: None,
        nested_scroll_offsets,
        focused_node: viewport.focused_node,
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
    // Sticky offset computation (same logic as paint_box).
    let sticky_dy = compute_sticky_dy(layout_box, offset_y, ctx);
    let offset_y = offset_y + sticky_dy;

    // Sticky elements get their own compositing layer so the display list
    // isolates them from surrounding content. With GPU render targets this
    // layer could be translated on the GPU during scroll instead of
    // re-recording the entire sticky subtree.
    let is_sticky = layout_box.style.position == Position::Sticky;
    if is_sticky {
        dl.set_has_sticky();
        dl.push(DisplayItem::PushSticky {
            natural_y: layout_box.dimensions.content.y,
            box_height: layout_box.dimensions.margin_box().height,
            top_px: match layout_box.style.top {
                Dimension::Px(t) => Some(t),
                _ => None,
            },
            bottom_px: match layout_box.style.bottom {
                Dimension::Px(b) => Some(b),
                _ => None,
            },
            visible_viewport_h: ctx.visible_viewport_height,
        });
    }

    // Compute transform translation early so culling accounts for it.
    let (tx_off_x, tx_off_y) = super::compute_transform_offsets(
        &layout_box.style.transforms,
        &layout_box.dimensions.content,
        offset_x,
        offset_y,
    );

    // Screen-space culling (includes CSS transform translation).
    let screen_y = layout_box.dimensions.content.y - ctx.scroll_y
        + sticky_dy as f32
        + (tx_off_y - offset_y) as f32;
    let box_bottom = screen_y + layout_box.dimensions.margin_box().height;
    let screen_x = layout_box.dimensions.content.x - ctx.scroll_x + (tx_off_x - offset_x) as f32;
    let box_right = screen_x + layout_box.dimensions.margin_box().width;

    if box_bottom < 0.0 || screen_y > ctx.viewport_height {
        // Close the sticky group opened above to keep the stack balanced.
        if is_sticky {
            dl.push(DisplayItem::PopSticky);
        }
        return;
    }
    if box_right < 0.0 || screen_x > ctx.viewport_width {
        if is_sticky {
            dl.push(DisplayItem::PopSticky);
        }
        return;
    }

    let is_visible = layout_box.style.visibility == Visibility::Visible;

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

    // Decide which kind of compositing layer (if any) this box needs:
    //
    // * Slow path (`PushCompositingLayer`) — required for
    //   `mix-blend-mode`, `backdrop-filter`, box-level `filter`,
    //   `isolation: isolate`, or `will-change: transform/opacity/filter`.
    //   Carries the full set of parameters the future render-target
    //   compositor needs. Backends without `SdiRenderTarget` support
    //   fall back to plain opacity stacking (see display_list.rs).
    //
    // * Fast path (`PushLayer`) — the opacity-only common case.  No
    //   offscreen surface needed.
    let wants_compositing = super::creates_compositing_layer(layout_box);
    let has_opacity = layout_box.style.opacity < 1.0;
    let layer_kind = if wants_compositing {
        Some(LayerKind::Compositing)
    } else if has_opacity {
        Some(LayerKind::Opacity)
    } else {
        None
    };

    match layer_kind {
        Some(LayerKind::Compositing) => {
            // Use the layout margin box as the layer bounds — covers
            // box shadow, borders, and contents.
            let mb = layout_box.dimensions.margin_box();
            let bounds = Rect {
                x: mb.x + offset_x as f32,
                y: mb.y + offset_y as f32,
                width: mb.width,
                height: mb.height,
            };
            let mix_blend = layout_box.style.mix_blend_mode;
            let has_backdrop = !layout_box.style.backdrop_filters.is_empty();
            let needs_backdrop =
                has_backdrop || !matches!(mix_blend, crate::css::values::types::BlendMode::Normal);
            dl.push(DisplayItem::PushCompositingLayer {
                bounds,
                opacity: layout_box.style.opacity,
                blend: mix_blend,
                needs_backdrop,
                filters: layout_box.style.filters.clone(),
                backdrop_filters: layout_box.style.backdrop_filters.clone(),
            });
        },
        Some(LayerKind::Opacity) => {
            dl.push(DisplayItem::PushLayer {
                opacity: layout_box.style.opacity,
            });
        },
        None => {},
    }
    let needs_layer = layer_kind.is_some();

    // Emit a blur hint for GPU backends that support render-target blur.
    // The software path already applies per-color approximation via
    // `apply_filters` during color recording below.
    let blur_radius = layout_box.style.filters.iter().find_map(|f| match f {
        FilterFunction::Blur(r) => Some(*r),
        _ => None,
    });
    if let Some(radius) = blur_radius {
        if radius > 0.0 {
            dl.push(DisplayItem::BlurHint { radius });
        }
    }

    // When opacity is handled by PushLayer, recording should use 1.0
    // to avoid applying opacity twice (once during recording, once during replay).
    let effective_opacity = if needs_layer {
        1.0
    } else {
        layout_box.style.opacity
    };

    if is_visible {
        // 0. Box shadow.
        record_box_shadow(layout_box, dl, offset_x, offset_y, ctx, effective_opacity);

        // 1. Background.
        record_background(layout_box, dl, offset_x, offset_y, ctx, effective_opacity);

        // 2. Borders.
        record_borders(layout_box, dl, offset_x, offset_y, ctx);

        // 2b. Outline.
        record_outline(layout_box, dl, offset_x, offset_y, ctx);
    }

    // Overflow clipping.
    let prev_clip = ctx.clip_rect;
    let prev_ellipsis = ctx.text_overflow_ellipsis;
    let prev_scroll_x = ctx.scroll_x;
    let prev_scroll_y = ctx.scroll_y;
    let has_overflow_clip = matches!(
        layout_box.style.overflow,
        Overflow::Hidden | Overflow::Scroll | Overflow::Auto
    );
    if has_overflow_clip {
        let new_clip = layout_box.dimensions.content;
        let clipped = match ctx.clip_rect {
            Some(existing) => intersect_rects(existing, new_clip),
            None => new_clip,
        };
        ctx.clip_rect = Some(clipped);
        // text-overflow: ellipsis only activates when white-space
        // prevents wrapping (nowrap / pre), so multi-line content is
        // never ellipsized.
        ctx.text_overflow_ellipsis = layout_box.style.text_overflow == TextOverflow::Ellipsis
            && matches!(
                layout_box.style.white_space,
                WhiteSpace::NoWrap | WhiteSpace::Pre
            );

        dl.push(DisplayItem::PushClip {
            x: (clipped.x - ctx.scroll_x + offset_x as f32) as i32,
            y: (clipped.y - ctx.scroll_y + offset_y as f32) as i32,
            w: clipped.width as u32,
            h: clipped.height as u32,
        });

        // Apply nested scroll offset for this container AFTER pushing
        // the clip so the clip boundary uses the parent's scroll offset,
        // not this container's own scroll.
        if let Some(nid) = layout_box.node {
            if let Some(&(sx, sy)) = ctx.nested_scroll_offsets.get(&nid) {
                ctx.scroll_x += sx;
                ctx.scroll_y += sy;
            }
        }
    }

    // Reuse transform offsets computed earlier (before culling).
    let (tx_offset_x, tx_offset_y) = (tx_off_x, tx_off_y);

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
            // CSS 2.1 painting order (appendix E):
            //   1. Background & borders (already recorded above)
            //   2. Stacking-context children with negative z-index
            //   3. Non-positioned children in tree order (normal flow)
            //   4. Positioned children with z-index: auto (tree order)
            //   5. Stacking-context children with z-index >= 0 (sorted)
            let mut normal_children: Vec<&LayoutBox> = Vec::new();
            let mut positioned_auto: Vec<(usize, &LayoutBox)> = Vec::new();
            let mut stacking_neg: Vec<(i32, usize, &LayoutBox)> = Vec::new();
            let mut stacking_pos: Vec<(i32, usize, &LayoutBox)> = Vec::new();

            for (idx, child) in layout_box.children.iter().enumerate() {
                if super::creates_stacking_context(child) {
                    if child.style.z_index < 0 {
                        stacking_neg.push((child.style.z_index, idx, child));
                    } else {
                        stacking_pos.push((child.style.z_index, idx, child));
                    }
                } else if super::is_positioned(child) {
                    positioned_auto.push((idx, child));
                } else {
                    normal_children.push(child);
                }
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
                record_box(child, dl, tx_offset_x, tx_offset_y, ctx, link_map);
            }

            // Step 3: non-positioned children in DOM order.
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
                record_box(child, dl, tx_offset_x, tx_offset_y, ctx, link_map);
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
                record_box(child, dl, tx_offset_x, tx_offset_y, ctx, link_map);
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
                record_box(child, dl, tx_offset_x, tx_offset_y, ctx, link_map);
            }
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
    if matches!(
        layout_box.style.overflow,
        Overflow::Hidden | Overflow::Scroll | Overflow::Auto
    ) {
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
        if layout_box.node == Some(link_node) || has_text_content(layout_box) {
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
    }

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

// -------------------------------------------------------------------
// Component recorders
// -------------------------------------------------------------------

fn record_background(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
    effective_opacity: f32,
) {
    let padding = layout_box.dimensions.padding_box();
    let x = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
    let w = padding.width as u32;
    let h = padding.height as u32;

    let bg = apply_filters_and_opacity(
        layout_box.style.background_color,
        effective_opacity,
        &layout_box.style.filters,
    );
    if bg.a > 0 {
        if !layout_box.style.border_radius.is_zero() {
            dl.push(DisplayItem::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius: layout_box.style.border_radius.max_radius() as u16,
                color: bg,
                node_id: ctx.current_node,
            });
        } else {
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w,
                h,
                color: bg,
                node_id: ctx.current_node,
            });
        }
    }

    // Linear gradient.
    if let crate::css::values::BackgroundImage::Gradient(ref grad) =
        layout_box.style.background_image
    {
        record_linear_gradient(dl, x, y, w, h, grad, effective_opacity);
    }

    // Radial gradient.
    if let crate::css::values::BackgroundImage::RadialGradient(ref grad) =
        layout_box.style.background_image
    {
        record_radial_gradient(dl, x, y, w, h, grad, effective_opacity);
    }

    // Background image texture: emit one Blit per tile so background-size,
    // background-position, and background-repeat are honored in the display
    // list path (not just immediate-mode).
    if let Some(tex) = layout_box.background_texture {
        let intrinsic = layout_box.background_texture_size.unwrap_or((w, h));
        let tiles = super::background::background_image_tiles(
            x,
            y,
            w,
            h,
            intrinsic,
            &layout_box.style.background_size,
            &layout_box.style.background_position,
            layout_box.style.background_repeat,
        );
        for tile in tiles {
            dl.push(DisplayItem::Blit {
                texture: tex,
                x: tile.x,
                y: tile.y,
                w: tile.w,
                h: tile.h,
            });
        }
    }
}

fn record_linear_gradient(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::LinearGradient,
    opacity: f32,
) {
    use crate::css::values::GradientDirection;
    use oasis_types::backend::GradientStyle;

    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return;
    }

    let angle_deg = match grad.direction {
        GradientDirection::ToBottom => 180.0,
        GradientDirection::ToTop => 0.0,
        GradientDirection::ToRight => 90.0,
        GradientDirection::ToLeft => 270.0,
        GradientDirection::Angle(deg) => deg,
    };
    let norm = ((angle_deg % 360.0) + 360.0) % 360.0;

    let is_vertical =
        (norm - 0.0).abs() < 0.5 || (norm - 180.0).abs() < 0.5 || (norm - 360.0).abs() < 0.5;
    let is_horizontal = (norm - 90.0).abs() < 0.5 || (norm - 270.0).abs() < 0.5;

    if is_vertical || is_horizontal {
        let to_end = is_vertical && (norm - 0.0).abs() < 1.0 || (norm - 360.0).abs() < 1.0;
        let reverse = if is_vertical {
            to_end
        } else {
            (norm - 270.0).abs() < 1.0
        };

        let total_len = if is_vertical { h } else { w } as f32;
        let stops = &grad.stops;

        let last_pos = stops.last().map(|s| s.position).unwrap_or(1.0);
        let first_pos = stops.first().map(|s| s.position).unwrap_or(0.0);
        let pattern_range = last_pos - first_pos;
        let repetitions = if grad.repeating && pattern_range > 0.0 {
            (1.0 / pattern_range).ceil() as u32
        } else {
            1
        };

        for rep in 0..repetitions {
            let rep_offset = if grad.repeating {
                rep as f32 * pattern_range
            } else {
                0.0
            };

            for i in 0..stops.len() - 1 {
                let (s0, s1) = if reverse {
                    let ri = stops.len() - 1 - i;
                    (&stops[ri], &stops[ri - 1])
                } else {
                    (&stops[i], &stops[i + 1])
                };

                let start_frac = if reverse {
                    1.0 - s0.position
                } else {
                    s0.position
                } + rep_offset;
                let end_frac = if reverse {
                    1.0 - s1.position
                } else {
                    s1.position
                } + rep_offset;
                if start_frac >= 1.0 {
                    break;
                }
                let end_frac = end_frac.min(1.0);
                let c0 = apply_opacity(s0.color, opacity);
                let c1 = apply_opacity(s1.color, opacity);

                let start_px = (start_frac * total_len) as i32;
                let end_px = ((end_frac * total_len) as i32).min(total_len as i32);
                let seg_len = (end_px - start_px).max(0) as u32;
                if seg_len == 0 {
                    continue;
                }

                if is_vertical {
                    dl.push(DisplayItem::Gradient {
                        x,
                        y: y + start_px,
                        w,
                        h: seg_len,
                        style: GradientStyle::Vertical {
                            top: c0,
                            bottom: c1,
                        },
                    });
                } else {
                    dl.push(DisplayItem::Gradient {
                        x: x + start_px,
                        y,
                        w: seg_len,
                        h,
                        style: GradientStyle::Horizontal {
                            left: c0,
                            right: c1,
                        },
                    });
                }
            }
        }
    } else {
        // Diagonal gradient: render with horizontal bands matching background.rs.
        let rad = norm.to_radians();
        let dx = rad.sin();
        let dy = -rad.cos();
        let wf = w as f32;
        let hf = h as f32;

        let half_w = wf / 2.0;
        let half_h = hf / 2.0;
        let mut min_proj = f32::MAX;
        let mut max_proj = f32::MIN;
        for &(cx, cy) in &[(0.0, 0.0), (wf, 0.0), (0.0, hf), (wf, hf)] {
            let proj = (cx - half_w) * dx + (cy - half_h) * dy;
            if proj < min_proj {
                min_proj = proj;
            }
            if proj > max_proj {
                max_proj = proj;
            }
        }
        let proj_range = max_proj - min_proj;
        if proj_range < 0.001 {
            return;
        }

        let num_bands = (h as usize).clamp(1, 32);
        let band_h_f = hf / num_bands as f32;

        for band in 0..num_bands {
            let by = band as f32 * band_h_f;
            let band_cy = by + band_h_f / 2.0;

            let t_left = ((0.0 - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;
            let t_right = ((wf - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;

            let c_left = super::background::sample_gradient_pub(&grad.stops, t_left, opacity);
            let c_right = super::background::sample_gradient_pub(&grad.stops, t_right, opacity);

            let start_y = by as i32;
            let end_y = if band == num_bands - 1 {
                h as i32
            } else {
                ((band + 1) as f32 * band_h_f) as i32
            };
            let band_h_px = (end_y - start_y).max(0) as u32;
            if band_h_px == 0 {
                continue;
            }

            dl.push(DisplayItem::Gradient {
                x,
                y: y + start_y,
                w,
                h: band_h_px,
                style: GradientStyle::Horizontal {
                    left: c_left,
                    right: c_right,
                },
            });
        }
    }
}

fn record_radial_gradient(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::RadialGradient,
    opacity: f32,
) {
    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return;
    }

    let hw = w as f32 / 2.0;
    let hh = h as f32 / 2.0;
    let max_radius = if grad.shape_circle {
        hw.max(hh)
    } else {
        (hw * hw + hh * hh).sqrt()
    };

    let bands = (max_radius as u32).clamp(8, 48);

    for i in 0..bands {
        let frac = i as f32 / bands as f32;
        let t = 1.0 - frac;
        let color = super::background::sample_gradient_pub(&grad.stops, t, opacity);
        let color = Color::rgba(color.r, color.g, color.b, 255);

        let bw = (w as f32 * (1.0 - frac)).max(1.0) as u32;
        let bh = (h as f32 * (1.0 - frac)).max(1.0) as u32;
        let bx = x + ((w - bw) / 2) as i32;
        let by = y + ((h - bh) / 2) as i32;
        let r = (bw.min(bh) / 2).max(1) as u16;

        dl.push(DisplayItem::FillRoundedRect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
            radius: r,
            color,
            node_id: None,
        });
    }
}

fn record_borders(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    let d = &layout_box.dimensions;
    let style = &layout_box.style;
    let border = d.border_box();
    let bx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
    let by = (border.y - ctx.scroll_y + offset_y as f32) as i32;
    let bw = border.width as u32;
    let bh = border.height as u32;

    if d.border.top > 0.0 && style.border_top_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by,
            w: bw,
            h: d.border.top as u32,
            color: style.border_top_color,
            style: style.border_top_style,
            horizontal: true,
        });
    }
    if d.border.right > 0.0 && style.border_right_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx + bw as i32 - d.border.right as i32,
            y: by,
            w: d.border.right as u32,
            h: bh,
            color: style.border_right_color,
            style: style.border_right_style,
            horizontal: false,
        });
    }
    if d.border.bottom > 0.0 && style.border_bottom_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by + bh as i32 - d.border.bottom as i32,
            w: bw,
            h: d.border.bottom as u32,
            color: style.border_bottom_color,
            style: style.border_bottom_style,
            horizontal: true,
        });
    }
    if d.border.left > 0.0 && style.border_left_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by,
            w: d.border.left as u32,
            h: bh,
            color: style.border_left_color,
            style: style.border_left_style,
            horizontal: false,
        });
    }
}

fn record_outline(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    let style = &layout_box.style;
    if style.outline_width <= 0.0 || style.outline_style == BorderStyle::None {
        return;
    }

    let border = layout_box.dimensions.border_box();
    let ow = style.outline_width;
    let oo = style.outline_offset;
    let ox = (border.x - ctx.scroll_x + offset_x as f32 - oo - ow) as i32;
    let oy = (border.y - ctx.scroll_y + offset_y as f32 - oo - ow) as i32;
    let total_w = (border.width + 2.0 * (oo + ow)).max(0.0) as u32;
    let total_h = (border.height + 2.0 * (oo + ow)).max(0.0) as u32;
    let thickness = ow as u32;
    let color = style.outline_color;
    let outline_style = style.outline_style;

    // Top
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy,
        w: total_w,
        h: thickness,
        color,
        style: outline_style,
        horizontal: true,
    });
    // Bottom
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy + total_h as i32 - thickness as i32,
        w: total_w,
        h: thickness,
        color,
        style: outline_style,
        horizontal: true,
    });
    // Left
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy,
        w: thickness,
        h: total_h,
        color,
        style: outline_style,
        horizontal: false,
    });
    // Right
    dl.push(DisplayItem::BorderEdge {
        x: ox + total_w as i32 - thickness as i32,
        y: oy,
        w: thickness,
        h: total_h,
        color,
        style: outline_style,
        horizontal: false,
    });
}

fn record_box_shadow(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
    _effective_opacity: f32,
) {
    if layout_box.style.box_shadow.is_empty() {
        return;
    }

    let border = layout_box.dimensions.border_box();
    let radius = layout_box.style.border_radius.max_radius();

    for shadow in layout_box.style.box_shadow.iter().rev() {
        if shadow.inset {
            let padding = layout_box.dimensions.padding_box();
            let px = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
            let py = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
            let pw = padding.width as u32;
            let ph = padding.height as u32;

            let steps = (shadow.blur as i32).max(1);
            for i in (0..steps).rev() {
                let t = i as f32 / steps as f32;
                let alpha = ((shadow.color.a as f32) * (1.0 - t) * 0.4) as u8;
                if alpha == 0 {
                    continue;
                }
                let shrink = i as u32;
                let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
                let sx = px + shadow.offset_x as i32 + shrink as i32;
                let sy = py + shadow.offset_y as i32 + shrink as i32;
                let sw = pw.saturating_sub(shrink * 2);
                let sh = ph.saturating_sub(shrink * 2);
                if sw == 0 || sh == 0 {
                    continue;
                }
                let thickness = (shadow.spread as u32 + 1).min(sh / 2).min(sw / 2);
                // Top strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy,
                    w: sw,
                    h: thickness,
                    color,
                    node_id: ctx.current_node,
                });
                // Bottom strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy + sh as i32 - thickness as i32,
                    w: sw,
                    h: thickness,
                    color,
                    node_id: ctx.current_node,
                });
                // Left strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy + thickness as i32,
                    w: thickness,
                    h: sh.saturating_sub(thickness * 2),
                    color,
                    node_id: ctx.current_node,
                });
                // Right strip
                dl.push(DisplayItem::FillRect {
                    x: sx + sw as i32 - thickness as i32,
                    y: sy + thickness as i32,
                    w: thickness,
                    h: sh.saturating_sub(thickness * 2),
                    color,
                    node_id: ctx.current_node,
                });
            }
        } else {
            // Emit a single Shadow display item — GPU backends can render
            // this with a Gaussian blur shader during replay.
            let sx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
            let sy = (border.y - ctx.scroll_y + offset_y as f32) as i32;
            let sw = border.width as u32;
            let sh = border.height as u32;
            dl.push(DisplayItem::Shadow {
                x: sx,
                y: sy,
                w: sw,
                h: sh,
                blur: shadow.blur,
                spread: shadow.spread,
                offset_x: shadow.offset_x,
                offset_y: shadow.offset_y,
                color: shadow.color,
                radius,
            });
        }
    }
}

fn record_inline_content(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &mut RecordContext,
    link_map: &HashMap<NodeId, String>,
) {
    // Inline background.
    let bg = layout_box.style.background_color;
    if bg.a > 0 {
        let pb = layout_box.dimensions.padding_box();
        let x = (pb.x - ctx.scroll_x + offset_x as f32) as i32;
        let y = (pb.y - ctx.scroll_y + offset_y as f32) as i32;
        let w = pb.width as u32;
        let h = pb.height as u32;
        if !layout_box.style.border_radius.is_zero() {
            dl.push(DisplayItem::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius: layout_box.style.border_radius.max_radius() as u16,
                color: bg,
                node_id: ctx.current_node,
            });
        } else {
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w,
                h,
                color: bg,
                node_id: ctx.current_node,
            });
        }
    }

    // Text content.
    if let Some(ref text) = layout_box.text {
        let content = &layout_box.dimensions.content;
        record_text(
            text,
            content.x,
            content.y,
            &layout_box.style,
            dl,
            offset_x,
            offset_y,
            ctx,
        );
    }

    for child in &layout_box.children {
        record_box(child, dl, offset_x, offset_y, ctx, link_map);
    }
}

fn record_text(
    text: &str,
    x: f32,
    y: f32,
    style: &crate::css::values::ComputedStyle,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    let sx = (x - ctx.scroll_x + offset_x as f32) as i32;
    let sy = (y - ctx.scroll_y + offset_y as f32) as i32;

    // Opacity is handled by PushLayer/PopLayer during replay — use 1.0 here
    // to avoid doubling opacity (once in recording, once during replay).
    let color = apply_filters_and_opacity(style.color, 1.0, &style.filters);
    let bold = style.font_weight.is_bold();
    let italic = style.font_style == crate::css::values::FontStyle::Italic;
    let font_size = style.font_size as u16;

    // text-overflow: ellipsis handling.
    let display_text: std::borrow::Cow<'_, str>;
    if ctx.text_overflow_ellipsis {
        if let Some(clip) = &ctx.clip_rect {
            let avail = (clip.x + clip.width - x).max(0.0) as u32;
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

    // Pre-compute text width including letter-spacing for bounds and decorations.
    let mut text_w = oasis_types::backend::bitmap_measure_text(&display_text, font_size) as f32;
    if style.letter_spacing != 0.0 {
        let chars = display_text.chars().count();
        if chars > 1 {
            text_w += style.letter_spacing * (chars - 1) as f32;
        }
    }
    let text_width = text_w.max(0.0) as u32;

    // Text shadow.
    if let Some(ref shadow) = style.text_shadow {
        let shadow_color = apply_opacity(shadow.color, 1.0);
        dl.push(DisplayItem::DrawText {
            text: display_text.to_string(),
            x: sx + shadow.offset_x as i32,
            y: sy + shadow.offset_y as i32,
            font_size,
            color: shadow_color,
            bold,
            italic,
            width: text_width,
            node_id: ctx.current_node,
        });
    }

    dl.push(DisplayItem::DrawText {
        text: display_text.to_string(),
        x: sx,
        y: sy,
        font_size,
        color,
        bold,
        italic,
        width: text_width,
        node_id: ctx.current_node,
    });

    if !style.text_decoration.line.is_none() {
        let deco_color = style.text_decoration.color.map_or(color, |c| {
            super::apply_filters_and_opacity(c, 1.0, &style.filters)
        });
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
            dl.push(DisplayItem::FillRect {
                x: sx,
                y: deco_y,
                w: text_width,
                h: 1,
                color: deco_color,
                node_id: ctx.current_node,
            });
        }
    }
}

fn record_list_marker(
    marker: &crate::layout::box_model::ListMarker,
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    use crate::layout::box_model::ListMarker;

    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;
    let font_size = layout_box.style.font_size as u16;
    let color = layout_box.style.color;

    let text = match marker {
        ListMarker::Disc => "\u{2022}".to_string(),
        ListMarker::Circle => "\u{25CB}".to_string(),
        ListMarker::Square => "\u{25A0}".to_string(),
        ListMarker::Decimal(n) => format!("{n}."),
        ListMarker::None => return,
    };

    let marker_w = oasis_types::backend::bitmap_measure_text(&text, font_size);
    dl.push(DisplayItem::DrawText {
        text,
        x: x - marker_w as i32 - 4,
        y,
        font_size,
        color,
        bold: false,
        italic: false,
        width: marker_w,
        node_id: ctx.current_node,
    });
}

fn record_replaced(
    replaced: &crate::layout::box_model::ReplacedContent,
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    use crate::layout::box_model::ReplacedContent;

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
            let (blit_x, blit_y, blit_w, blit_h) = super::replaced::compute_object_fit_pub(
                layout_box.style.object_fit,
                *img_w,
                *img_h,
                box_w,
                box_h,
                x,
                y,
            );
            if let Some(ar) = atlas_region {
                dl.push(DisplayItem::BlitSub {
                    texture: *tex,
                    src_x: ar.x,
                    src_y: ar.y,
                    src_w: ar.w,
                    src_h: ar.h,
                    dst_x: blit_x,
                    dst_y: blit_y,
                    dst_w: blit_w,
                    dst_h: blit_h,
                });
            } else {
                dl.push(DisplayItem::Blit {
                    texture: *tex,
                    x: blit_x,
                    y: blit_y,
                    w: blit_w,
                    h: blit_h,
                });
            }
        },
        ReplacedContent::Image { alt, .. } => {
            let w = content.width.max(16.0) as u32;
            let h = content.height.max(16.0) as u32;
            let color = layout_box.style.color;
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w,
                h: 1,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x,
                y: y + h as i32 - 1,
                w,
                h: 1,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w: 1,
                h,
                color,
                node_id: ctx.current_node,
            });
            dl.push(DisplayItem::FillRect {
                x: x + w as i32 - 1,
                y,
                w: 1,
                h,
                color,
                node_id: ctx.current_node,
            });
            let label = if alt.is_empty() { "\u{00D7}" } else { alt };
            let label_w = oasis_types::backend::bitmap_measure_text(label, 8);
            dl.push(DisplayItem::DrawText {
                text: label.to_string(),
                x: x + 2,
                y: y + 2,
                font_size: 8,
                color,
                bold: false,
                italic: false,
                width: label_w,
                node_id: ctx.current_node,
            });
        },
        ReplacedContent::HorizontalRule => {
            let style = &layout_box.style;
            let w = content.width as u32;
            if style.border_top_style != BorderStyle::None && style.border_top_width > 0.0 {
                dl.push(DisplayItem::BorderEdge {
                    x,
                    y,
                    w,
                    h: style.border_top_width as u32,
                    color: style.border_top_color,
                    style: style.border_top_style,
                    horizontal: true,
                });
            } else {
                dl.push(DisplayItem::FillRect {
                    x,
                    y,
                    w,
                    h: 1,
                    color: Color::rgb(128, 128, 128),
                    node_id: ctx.current_node,
                });
            }
        },
        ReplacedContent::LineBreak => {},
        ReplacedContent::TextInput {
            value,
            placeholder,
            is_password,
            ..
        } => {
            let focused = layout_box.node.is_some() && layout_box.node == ctx.focused_node;
            record_text_input(
                layout_box,
                dl,
                x,
                y,
                value,
                placeholder,
                *is_password,
                focused,
            );
        },
        ReplacedContent::Checkbox { checked } => {
            record_checkbox(layout_box, dl, x, y, *checked);
        },
        ReplacedContent::RadioButton { checked } => {
            record_radio_button(layout_box, dl, x, y, *checked);
        },
        ReplacedContent::TextArea {
            value, placeholder, ..
        } => {
            let focused = layout_box.node.is_some() && layout_box.node == ctx.focused_node;
            record_textarea(layout_box, dl, x, y, value, placeholder, focused);
        },
        ReplacedContent::SelectBox {
            label,
            open,
            options,
            selected_index,
        } => {
            record_select_box(layout_box, dl, x, y, label, *open, options, *selected_index);
        },
        ReplacedContent::SubmitButton { label } => {
            record_submit_button(layout_box, dl, x, y, label);
        },
        // SVG and Canvas elements still use immediate-mode rendering
        // because they have their own complex drawing pipelines.
        // They'll be recorded as a placeholder and painted in a
        // second pass during replay.
        ReplacedContent::Svg { .. } | ReplacedContent::Canvas { .. } => {
            // Fallback: these elements are painted via the immediate-mode
            // path in widget_paint.rs. The display list records nothing
            // for them; the BrowserWidget paint method handles them separately.
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn record_text_input(
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

fn record_checkbox(layout_box: &LayoutBox, dl: &mut DisplayList, x: i32, y: i32, checked: bool) {
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

fn record_radio_button(
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

fn record_textarea(
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

fn record_select_box(
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
            });
        }
    }
}

fn record_submit_button(layout_box: &LayoutBox, dl: &mut DisplayList, x: i32, y: i32, label: &str) {
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
    });
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

fn compute_sticky_dy(layout_box: &LayoutBox, offset_y: i32, ctx: &RecordContext) -> i32 {
    if layout_box.style.position != Position::Sticky {
        return 0;
    }
    let top_px = match layout_box.style.top {
        Dimension::Px(t) => Some(t),
        _ => None,
    };
    let bottom_px = match layout_box.style.bottom {
        Dimension::Px(b) => Some(b),
        _ => None,
    };
    if let Some(top) = top_px {
        let natural = layout_box.dimensions.content.y - ctx.scroll_y + offset_y as f32;
        if natural < top {
            (top - natural) as i32
        } else {
            0
        }
    } else if let Some(bottom) = bottom_px {
        let natural = layout_box.dimensions.content.y - ctx.scroll_y + offset_y as f32;
        let box_h = layout_box.dimensions.margin_box().height;
        let threshold = ctx.visible_viewport_height - bottom - box_h;
        if natural > threshold {
            (threshold - natural) as i32
        } else {
            0
        }
    } else {
        0
    }
}

fn has_text_content(layout_box: &LayoutBox) -> bool {
    match &layout_box.box_type {
        BoxType::Inline => true,
        _ => layout_box.children.iter().any(has_text_content),
    }
}

fn apply_opacity(color: Color, opacity: f32) -> Color {
    color.apply_opacity(opacity)
}

fn apply_filters_and_opacity(
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

#[cfg(test)]
mod form_paint_tests {
    use super::*;
    use crate::css::values::ComputedStyle;
    use crate::layout::box_model::{BoxType, Dimensions, LayoutBox, ReplacedContent};

    fn make_text_input_box(value: &str, focused: bool, caret: Option<Color>) -> (LayoutBox, bool) {
        let mut style = ComputedStyle::default();
        style.font_size = 12.0;
        style.color = Color::rgb(0, 0, 0);
        style.caret_color = caret;
        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::TextInput {
                value: value.to_string(),
                placeholder: String::new(),
                size: 20,
                is_password: false,
            }),
            style,
            Some(7),
        );
        lb.dimensions = Dimensions {
            content: crate::layout::box_model::Rect::new(0.0, 0.0, 200.0, 20.0),
            ..Dimensions::default()
        };
        (lb, focused)
    }

    fn record_input_items(value: &str, focused: bool, caret: Option<Color>) -> Vec<DisplayItem> {
        let (lb, focused) = make_text_input_box(value, focused, caret);
        let mut dl = DisplayList::new();
        record_text_input(&lb, &mut dl, 0, 0, value, "", false, focused);
        dl.items().to_vec()
    }

    fn count_caret_with_color(items: &[DisplayItem], color: Color) -> usize {
        items
            .iter()
            .filter(|it| {
                matches!(
                    it,
                    DisplayItem::FillRect { w: 1, color: c, .. } if *c == color
                )
            })
            .count()
    }

    #[test]
    fn caret_drawn_when_focused() {
        let items = record_input_items("abc", true, None);
        // Default caret-color falls back to text color (black).
        assert!(count_caret_with_color(&items, Color::rgb(0, 0, 0)) >= 1);
    }

    #[test]
    fn caret_not_drawn_when_unfocused() {
        let items = record_input_items("abc", false, None);
        assert_eq!(count_caret_with_color(&items, Color::rgb(0, 0, 0)), 0);
    }

    #[test]
    fn caret_color_overrides_text_color() {
        let blue = Color::rgb(0, 0, 255);
        let items = record_input_items("abc", true, Some(blue));
        assert!(count_caret_with_color(&items, blue) >= 1);
        // No black caret in this case (text color is still black, but
        // the 1-pixel caret rect should be blue).
        assert_eq!(count_caret_with_color(&items, Color::rgb(0, 0, 0)), 0);
    }

    #[test]
    fn checkbox_uses_accent_color_when_checked() {
        let mut style = ComputedStyle::default();
        let pink = Color::rgb(255, 0, 128);
        style.accent_color = Some(pink);
        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::Checkbox { checked: true }),
            style,
            Some(11),
        );
        lb.dimensions = Dimensions {
            content: crate::layout::box_model::Rect::new(0.0, 0.0, 13.0, 13.0),
            ..Dimensions::default()
        };
        let mut dl = DisplayList::new();
        record_checkbox(&lb, &mut dl, 0, 0, true);
        // The first FillRect is the background and should now be pink.
        let first_bg = dl.items().iter().find_map(|it| match it {
            DisplayItem::FillRect { color, w, h, .. } if *w == 13 && *h == 13 => Some(*color),
            _ => None,
        });
        assert_eq!(first_bg, Some(pink));
    }

    #[test]
    fn radio_uses_accent_color_for_dot() {
        let mut style = ComputedStyle::default();
        let teal = Color::rgb(0, 200, 200);
        style.accent_color = Some(teal);
        let mut lb = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::RadioButton { checked: true }),
            style,
            Some(12),
        );
        lb.dimensions = Dimensions {
            content: crate::layout::box_model::Rect::new(0.0, 0.0, 13.0, 13.0),
            ..Dimensions::default()
        };
        let mut dl = DisplayList::new();
        record_radio_button(&lb, &mut dl, 0, 0, true);
        // The dot is the rect at the inset; ensure at least one rect
        // uses the teal accent.
        let any_teal = dl
            .items()
            .iter()
            .any(|it| matches!(it, DisplayItem::FillRect { color, .. } if *color == teal));
        assert!(any_teal, "radio dot should use accent-color");
    }
}
