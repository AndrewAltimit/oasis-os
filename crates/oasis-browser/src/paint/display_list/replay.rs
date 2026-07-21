//! Display-list replay against an `SdiBackend`.
//!
//! Two replay paths share the same machinery:
//!
//! - [`DisplayList::replay`] walks every item in order, applying scroll
//!   translation, opacity stacks, clip stacks, sticky offset
//!   re-computation, and compositing-layer offscreen render-target
//!   management.
//! - [`DisplayList::replay_dirty`] additionally culls items whose
//!   bounds don't intersect a supplied dirty rectangle.
//!
//! Both batch consecutive `FillRect` items into [`SdiBatch::submit_rect_batch`]
//! and consecutive same-style `DrawText` items into
//! [`SdiBatch::submit_text_batch`] so backends can emit a single GPU
//! draw call per batch.

use oasis_types::backend::{
    BatchRect, BatchText, BlendMode as BackendBlendMode, Color, RenderTargetId, SdiBackend,
};
use oasis_types::error::Result;

#[cfg(feature = "web-fonts")]
use super::WebFontRenderer;
use super::mask::{MaskParams, apply_mask};
use super::{DisplayItem, DisplayList};
use crate::css::values::BorderStyle;
use crate::css::values::types::{BlendMode as CssBlendMode, FilterFunction};
use crate::layout::box_model::Rect;

/// Convert a parsed CSS `BlendMode` into the backend's `BlendMode`
/// (they are the same 16-variant vocabulary — a direct map).
fn css_blend_to_backend(m: CssBlendMode) -> BackendBlendMode {
    match m {
        CssBlendMode::Normal => BackendBlendMode::Normal,
        CssBlendMode::Multiply => BackendBlendMode::Multiply,
        CssBlendMode::Screen => BackendBlendMode::Screen,
        CssBlendMode::Overlay => BackendBlendMode::Overlay,
        CssBlendMode::Darken => BackendBlendMode::Darken,
        CssBlendMode::Lighten => BackendBlendMode::Lighten,
        CssBlendMode::ColorDodge => BackendBlendMode::ColorDodge,
        CssBlendMode::ColorBurn => BackendBlendMode::ColorBurn,
        CssBlendMode::HardLight => BackendBlendMode::HardLight,
        CssBlendMode::SoftLight => BackendBlendMode::SoftLight,
        CssBlendMode::Difference => BackendBlendMode::Difference,
        CssBlendMode::Exclusion => BackendBlendMode::Exclusion,
        CssBlendMode::Hue => BackendBlendMode::Hue,
        CssBlendMode::Saturation => BackendBlendMode::Saturation,
        CssBlendMode::Color => BackendBlendMode::Color,
        CssBlendMode::Luminosity => BackendBlendMode::Luminosity,
    }
}

/// Entry in the compositor layer stack kept during `replay()`.
struct ActiveLayer {
    /// Allocated render target id. `None` when the backend reported
    /// `supports_render_targets() == false` and we fell through to
    /// the opacity-only fast path.
    id: Option<RenderTargetId>,
    /// Destination rect on the parent surface.
    dst_x: i32,
    dst_y: i32,
    dst_w: u32,
    dst_h: u32,
    /// Composite parameters.
    opacity: f32,
    blend: BackendBlendMode,
    /// Filter chain applied to the layer pixels between unbind and
    /// composite. Empty = no filter pass, fast composite_render_target
    /// path. Non-empty triggers a CPU readback + filter + re-upload.
    filters: Vec<FilterFunction>,
    /// Optional `mask-*` parameters. When present, the pop path
    /// rasterizes the mask source over the layer bounds and applies
    /// it to the layer's alpha channel before the final composite.
    mask: Option<MaskParams>,
    /// Saved `(compositor_dx, compositor_dy)` from before this layer
    /// was pushed. Restored on `PopCompositingLayer` so that nested
    /// layers don't accumulate translation incorrectly. (Without
    /// save/restore, an inner layer's screen-space origin would be
    /// subtracted on top of the outer layer's translation, leaving
    /// items off-target after the inner pop.)
    saved_compositor_dx: i32,
    saved_compositor_dy: i32,
}

impl DisplayList {
    pub(super) fn replay_inner(
        &self,
        backend: &mut dyn SdiBackend,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
        #[cfg(feature = "web-fonts")] mut web_font_renderer: Option<&mut dyn WebFontRenderer>,
    ) -> Result<()> {
        backend.begin_batch()?;

        let mut opacity_stack: Vec<f32> = Vec::new();
        // Clip stack stores the *intersected* clip rect for each level.
        let mut clip_stack: Vec<(i32, i32, u32, u32)> = Vec::new();
        // Sticky correction stack: Y offset adjustment for sticky elements
        // when replaying with a scroll delta different from recording time.
        let mut sticky_dy_stack: Vec<i32> = Vec::new();
        // Active compositor layer stack. Each entry carries the
        // render-target id (if the backend supports it) and composite
        // parameters for the matching `PopCompositingLayer`. An
        // additional translation (`-bounds.x`, `-bounds.y`) is applied
        // to every drawable inside the layer so contents land at
        // target-local coordinates.
        let mut layer_stack: Vec<ActiveLayer> = Vec::new();
        let mut compositor_dx: i32 = 0;
        let mut compositor_dy: i32 = 0;
        // Batch of consecutive FillRect items for batched submission.
        let mut rect_batch: Vec<BatchRect> = Vec::new();
        // Batch of consecutive same-style DrawText items.
        let mut text_batch: Vec<BatchText<'_>> = Vec::new();
        let mut text_batch_key: (u16, bool, bool) = (0, false, false);

        let supports_rt = backend.supports_render_targets();

        for item in &self.items {
            match item {
                DisplayItem::PushLayer { opacity } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopLayer => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.pop();
                    continue;
                },
                DisplayItem::PushCompositingLayer {
                    bounds,
                    opacity,
                    blend,
                    filters,
                    mask,
                    ..
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    let dx = bounds.x as i32;
                    let dy = bounds.y as i32;
                    let dw = bounds.width.max(1.0) as u32;
                    let dh = bounds.height.max(1.0) as u32;
                    let blend_backend = css_blend_to_backend(*blend);
                    let layer_id = if supports_rt {
                        match backend.create_render_target(dw, dh) {
                            Ok(id) => {
                                if backend.bind_render_target(id).is_ok() {
                                    Some(id)
                                } else {
                                    let _ = backend.destroy_render_target(id);
                                    None
                                }
                            },
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    // Snapshot the current compositor offset BEFORE
                    // we mutate it for the new layer. The pop path
                    // restores from this snapshot, which is correct
                    // even when layers are nested (the inner pop
                    // restores the outer layer's offset, not zero).
                    let saved_dx = compositor_dx;
                    let saved_dy = compositor_dy;
                    if layer_id.is_some() {
                        // Contents draw at target-local coordinates,
                        // i.e. the screen-space origin of the layer
                        // becomes (0, 0) inside the offscreen target.
                        // For nested layers this absolute reset is
                        // correct because each render target has its
                        // own coordinate space starting at (0, 0).
                        compositor_dx = -dx;
                        compositor_dy = -dy;
                    } else {
                        // Fallback: plain opacity stacking. Blend mode
                        // and filters are dropped — documented as the
                        // accepted degradation in
                        // `docs/compositor-overhaul-plan.md` §3.4 step 4.
                        opacity_stack.push(*opacity);
                    }
                    layer_stack.push(ActiveLayer {
                        id: layer_id,
                        dst_x: dx,
                        dst_y: dy,
                        dst_w: dw,
                        dst_h: dh,
                        opacity: *opacity,
                        blend: blend_backend,
                        filters: filters.clone(),
                        mask: mask.clone(),
                        saved_compositor_dx: saved_dx,
                        saved_compositor_dy: saved_dy,
                    });
                    continue;
                },
                DisplayItem::PopCompositingLayer => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    if let Some(layer) = layer_stack.pop() {
                        if let Some(id) = layer.id {
                            // Restore the parent layer's compositor
                            // offset (or zero if this was the
                            // outermost layer) instead of trying to
                            // back out the inner layer's translation
                            // by addition.
                            compositor_dx = layer.saved_compositor_dx;
                            compositor_dy = layer.saved_compositor_dy;
                            let unbind_res = backend.unbind_render_target();
                            // Filter/mask CPU pass: if the layer has
                            // filters or masks AND the backend supports
                            // pixel readback, read the layer target,
                            // apply the chain on CPU, then composite
                            // back with proper blend mode support.
                            //
                            // Non-Normal blend modes are composited on
                            // CPU by reading the destination pixels and
                            // applying the W3C blend formula before
                            // upload — no longer silently dropped.
                            let has_filter = !layer.filters.is_empty();
                            let has_mask = layer.mask.is_some();
                            let run_cpu_pass = (has_filter || has_mask)
                                && backend.supports_render_target_readback();
                            let composite_res = if unbind_res.is_ok() {
                                if run_cpu_pass {
                                    let byte_count = (layer.dst_w * layer.dst_h * 4) as usize;
                                    let mut buf = vec![0u8; byte_count];
                                    if backend.read_render_target(id, &mut buf).is_ok() {
                                        if has_filter {
                                            crate::paint::filter_chain::apply_filter_chain(
                                                &mut buf,
                                                layer.dst_w,
                                                layer.dst_h,
                                                &layer.filters,
                                            );
                                        }
                                        if let Some(mask) = layer.mask.as_ref() {
                                            apply_mask(&mut buf, layer.dst_w, layer.dst_h, mask);
                                        }
                                        // Pre-multiply opacity into alpha.
                                        if (layer.opacity - 1.0).abs() > f32::EPSILON {
                                            let f = layer.opacity.clamp(0.0, 1.0);
                                            for chunk in buf.as_chunks_mut::<4>().0.iter_mut() {
                                                chunk[3] = ((chunk[3] as f32) * f).round() as u8;
                                            }
                                        }
                                        // Non-Normal blend: read destination
                                        // pixels and composite on CPU so the
                                        // blend mode is preserved through the
                                        // filter/mask path.
                                        if !layer.blend.is_normal() {
                                            if let Ok(dst_pixels) = backend.read_pixels(
                                                layer.dst_x,
                                                layer.dst_y,
                                                layer.dst_w,
                                                layer.dst_h,
                                            ) {
                                                let mut blended = dst_pixels;
                                                crate::paint::filter_chain::cpu_blend_composite(
                                                    &buf,
                                                    &mut blended,
                                                    layer.dst_w,
                                                    layer.dst_h,
                                                    layer.blend,
                                                );
                                                if let Ok(tex) = backend.load_texture(
                                                    layer.dst_w,
                                                    layer.dst_h,
                                                    &blended,
                                                ) {
                                                    let _ = backend.blit(
                                                        tex,
                                                        layer.dst_x,
                                                        layer.dst_y,
                                                        layer.dst_w,
                                                        layer.dst_h,
                                                    );
                                                    let _ = backend.destroy_texture(tex);
                                                }
                                                Ok(())
                                            } else {
                                                // read_pixels failed — fall
                                                // back to Normal blit.
                                                if let Ok(tex) = backend.load_texture(
                                                    layer.dst_w,
                                                    layer.dst_h,
                                                    &buf,
                                                ) {
                                                    let _ = backend.blit(
                                                        tex,
                                                        layer.dst_x,
                                                        layer.dst_y,
                                                        layer.dst_w,
                                                        layer.dst_h,
                                                    );
                                                    let _ = backend.destroy_texture(tex);
                                                }
                                                Ok(())
                                            }
                                        } else {
                                            // Normal blend — plain blit.
                                            if let Ok(tex) =
                                                backend.load_texture(layer.dst_w, layer.dst_h, &buf)
                                            {
                                                let _ = backend.blit(
                                                    tex,
                                                    layer.dst_x,
                                                    layer.dst_y,
                                                    layer.dst_w,
                                                    layer.dst_h,
                                                );
                                                let _ = backend.destroy_texture(tex);
                                            }
                                            Ok(())
                                        }
                                    } else {
                                        backend.composite_render_target(
                                            id,
                                            layer.dst_x,
                                            layer.dst_y,
                                            layer.dst_w,
                                            layer.dst_h,
                                            layer.blend,
                                            layer.opacity,
                                        )
                                    }
                                } else {
                                    backend.composite_render_target(
                                        id,
                                        layer.dst_x,
                                        layer.dst_y,
                                        layer.dst_w,
                                        layer.dst_h,
                                        layer.blend,
                                        layer.opacity,
                                    )
                                }
                            } else {
                                Ok(())
                            };
                            // Always destroy the render target even
                            // if unbind/composite failed.
                            let destroy_res = backend.destroy_render_target(id);
                            unbind_res.and(composite_res).and(destroy_res)?;
                        } else {
                            opacity_stack.pop();
                        }
                    }
                    continue;
                },
                DisplayItem::PushSticky {
                    natural_y,
                    box_height,
                    top_px,
                    bottom_px,
                    visible_viewport_h,
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    // Compute the sticky offset difference between replay
                    // scroll and recording scroll.
                    let eff_scroll = self.recording_scroll_y - scroll_dy as f32;
                    let new_dy = compute_sticky_dy_from_params(
                        *natural_y,
                        *box_height,
                        *top_px,
                        *bottom_px,
                        *visible_viewport_h,
                        eff_scroll,
                    );
                    let rec_dy = compute_sticky_dy_from_params(
                        *natural_y,
                        *box_height,
                        *top_px,
                        *bottom_px,
                        *visible_viewport_h,
                        self.recording_scroll_y,
                    );
                    sticky_dy_stack.push(new_dy - rec_dy);
                    continue;
                },
                DisplayItem::PopSticky => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    sticky_dy_stack.pop();
                    continue;
                },
                _ => {},
            }

            let layer_opacity = layer_opacity_product(&opacity_stack);
            let sticky_correction = sticky_dy_stack.last().copied().unwrap_or(0);
            // When a compositing layer is active, items are drawn at
            // target-local coordinates — `compositor_dx`/`compositor_dy`
            // translate screen-space recordings into that space. When
            // no layer is active both values are zero. Shadow the
            // outer `scroll_dx` so the existing per-item drawing
            // branches below pick up the layer offset automatically.
            #[allow(clippy::shadow_unrelated)]
            let scroll_dx = scroll_dx + compositor_dx;
            let eff_dy = scroll_dy + compositor_dy + sticky_correction;

            match item {
                DisplayItem::FillRect {
                    x, y, w, h, color, ..
                } => {
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    let c = apply_layer_opacity(*color, layer_opacity);
                    rect_batch.push(BatchRect {
                        x: x + scroll_dx,
                        y: y + eff_dy,
                        w: *w,
                        h: *h,
                        color: c,
                    });
                    continue;
                },
                DisplayItem::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    color,
                    bold,
                    italic,
                    #[cfg(feature = "web-fonts")]
                    web_font_id,
                    ..
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;

                    // Web font path: render glyphs via the font
                    // registry when a renderer is attached.
                    #[cfg(feature = "web-fonts")]
                    if let Some(font_id) = web_font_id {
                        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                        let c = apply_layer_opacity(*color, layer_opacity);
                        if let Some(ref mut renderer) = web_font_renderer {
                            renderer.render(
                                backend,
                                text,
                                x + scroll_dx,
                                y + eff_dy,
                                *font_size,
                                c,
                                *font_id,
                            )?;
                        } else {
                            // No renderer attached — bitmap fallback.
                            backend.draw_text_styled(
                                text,
                                x + scroll_dx,
                                y + eff_dy,
                                *font_size,
                                c,
                                *bold,
                                *italic,
                            )?;
                        }
                        continue;
                    }

                    let c = apply_layer_opacity(*color, layer_opacity);
                    let key = (*font_size, *bold, *italic);
                    if !text_batch.is_empty() && text_batch_key != key {
                        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    }
                    text_batch_key = key;
                    text_batch.push(BatchText {
                        text,
                        x: x + scroll_dx,
                        y: y + eff_dy,
                        color: c,
                    });
                    continue;
                },
                _ => {
                    // Non-FillRect/DrawText item breaks both batches.
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                },
            }

            match item {
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                    ..
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rounded_rect(x + scroll_dx, y + eff_dy, *w, *h, *radius, c)?;
                },
                DisplayItem::FillPolygon { points, color, .. } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    let shifted = [
                        (points[0].0 + scroll_dx, points[0].1 + eff_dy),
                        (points[1].0 + scroll_dx, points[1].1 + eff_dy),
                        (points[2].0 + scroll_dx, points[2].1 + eff_dy),
                        (points[3].0 + scroll_dx, points[3].1 + eff_dy),
                    ];
                    backend.fill_polygon(&shifted, c)?;
                },
                DisplayItem::StrokeRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    stroke_width,
                    color,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.stroke_rounded_rect(
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        c,
                    )?;
                },
                DisplayItem::Blit {
                    texture,
                    x,
                    y,
                    w,
                    h,
                } => {
                    backend.blit(*texture, x + scroll_dx, y + eff_dy, *w, *h)?;
                },
                DisplayItem::BlitSub {
                    texture,
                    src_x,
                    src_y,
                    src_w,
                    src_h,
                    dst_x,
                    dst_y,
                    dst_w,
                    dst_h,
                } => {
                    backend.blit_sub(
                        *texture,
                        *src_x,
                        *src_y,
                        *src_w,
                        *src_h,
                        dst_x + scroll_dx,
                        dst_y + eff_dy,
                        *dst_w,
                        *dst_h,
                    )?;
                },
                DisplayItem::Gradient { x, y, w, h, style } => {
                    backend.fill_rect_gradient(x + scroll_dx, y + eff_dy, *w, *h, style)?;
                },
                DisplayItem::BorderEdge {
                    x,
                    y,
                    w,
                    h,
                    color,
                    style,
                    horizontal,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    replay_border_edge(
                        backend,
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        c,
                        *style,
                        *horizontal,
                    )?;
                },
                DisplayItem::Shadow {
                    x,
                    y,
                    w,
                    h,
                    blur,
                    spread,
                    offset_x,
                    offset_y,
                    color,
                    radius,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_shadow(
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        *blur,
                        *spread,
                        *offset_x,
                        *offset_y,
                        c,
                        *radius,
                    )?;
                },
                DisplayItem::PushClip { x, y, w, h } => {
                    let raw = (x + scroll_dx, y + eff_dy, *w, *h);
                    // Intersect with parent clip for tighter bounds.
                    let effective = if let Some(&parent) = clip_stack.last() {
                        intersect_clip(parent, raw)
                    } else if let Some(base) = base_clip {
                        intersect_clip(base, raw)
                    } else {
                        raw
                    };
                    clip_stack.push(effective);
                    backend.set_clip_rect(effective.0, effective.1, effective.2, effective.3)?;
                },
                DisplayItem::PopClip => {
                    clip_stack.pop();
                    // Restore to parent clip or base clip (browser window).
                    if let Some(&(cx, cy, cw, ch)) = clip_stack.last() {
                        backend.set_clip_rect(cx, cy, cw, ch)?;
                    } else if let Some((bx, by, bw, bh)) = base_clip {
                        backend.set_clip_rect(bx, by, bw, bh)?;
                    } else {
                        backend.reset_clip_rect()?;
                    }
                },
                // Already handled above the match.
                DisplayItem::FillRect { .. }
                | DisplayItem::DrawText { .. }
                | DisplayItem::PushLayer { .. }
                | DisplayItem::PopLayer
                | DisplayItem::PushCompositingLayer { .. }
                | DisplayItem::PopCompositingLayer
                | DisplayItem::PushSticky { .. }
                | DisplayItem::PopSticky => {},
                // BlurHint is metadata for GPU backends; software fallback
                // applies per-color approximation during recording.
                DisplayItem::BlurHint { .. } => {},
            }
        }

        flush_rect_batch(backend, &mut rect_batch)?;
        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
        backend.flush_batch()?;
        Ok(())
    }

    pub(super) fn replay_dirty_inner(
        &self,
        backend: &mut dyn SdiBackend,
        dirty: &Rect,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
        #[cfg(feature = "web-fonts")] mut web_font_renderer: Option<&mut dyn WebFontRenderer>,
    ) -> Result<()> {
        backend.begin_batch()?;

        let mut opacity_stack: Vec<f32> = Vec::new();
        let mut clip_stack: Vec<(i32, i32, u32, u32)> = Vec::new();
        let mut sticky_dy_stack: Vec<i32> = Vec::new();
        let mut rect_batch: Vec<BatchRect> = Vec::new();
        let mut text_batch: Vec<BatchText<'_>> = Vec::new();
        let mut text_batch_key: (u16, bool, bool) = (0, false, false);

        for item in &self.items {
            // Clip, layer, and sticky items must always be replayed.
            match item {
                DisplayItem::PushClip { x, y, w, h } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    let sticky_corr = sticky_dy_stack.last().copied().unwrap_or(0);
                    let raw = (x + scroll_dx, y + scroll_dy + sticky_corr, *w, *h);
                    let effective = if let Some(&parent) = clip_stack.last() {
                        intersect_clip(parent, raw)
                    } else if let Some(base) = base_clip {
                        intersect_clip(base, raw)
                    } else {
                        raw
                    };
                    clip_stack.push(effective);
                    backend.set_clip_rect(effective.0, effective.1, effective.2, effective.3)?;
                    continue;
                },
                DisplayItem::PopClip => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    clip_stack.pop();
                    if let Some(&(cx, cy, cw, ch)) = clip_stack.last() {
                        backend.set_clip_rect(cx, cy, cw, ch)?;
                    } else if let Some((bx, by, bw, bh)) = base_clip {
                        backend.set_clip_rect(bx, by, bw, bh)?;
                    } else {
                        backend.reset_clip_rect()?;
                    }
                    continue;
                },
                DisplayItem::PushLayer { opacity } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopLayer => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.pop();
                    continue;
                },
                DisplayItem::PushCompositingLayer { opacity, .. } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopCompositingLayer => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    opacity_stack.pop();
                    continue;
                },
                DisplayItem::PushSticky {
                    natural_y,
                    box_height,
                    top_px,
                    bottom_px,
                    visible_viewport_h,
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    let eff_scroll = self.recording_scroll_y - scroll_dy as f32;
                    let new_dy = compute_sticky_dy_from_params(
                        *natural_y,
                        *box_height,
                        *top_px,
                        *bottom_px,
                        *visible_viewport_h,
                        eff_scroll,
                    );
                    let rec_dy = compute_sticky_dy_from_params(
                        *natural_y,
                        *box_height,
                        *top_px,
                        *bottom_px,
                        *visible_viewport_h,
                        self.recording_scroll_y,
                    );
                    sticky_dy_stack.push(new_dy - rec_dy);
                    continue;
                },
                DisplayItem::PopSticky => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    sticky_dy_stack.pop();
                    continue;
                },
                _ => {},
            }

            // Cull items outside the dirty rect.
            let sticky_correction = sticky_dy_stack.last().copied().unwrap_or(0);
            let eff_dy = scroll_dy + sticky_correction;
            if let Some(bounds) = item.bounds() {
                let shifted = Rect {
                    x: bounds.x + scroll_dx as f32,
                    y: bounds.y + eff_dy as f32,
                    width: bounds.width,
                    height: bounds.height,
                };
                if !rects_intersect(&shifted, dirty) {
                    continue;
                }
            }

            let layer_opacity = layer_opacity_product(&opacity_stack);

            // Replay the item — batch consecutive FillRects and DrawTexts.
            match item {
                DisplayItem::FillRect {
                    x, y, w, h, color, ..
                } => {
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    let c = apply_layer_opacity(*color, layer_opacity);
                    rect_batch.push(BatchRect {
                        x: x + scroll_dx,
                        y: y + eff_dy,
                        w: *w,
                        h: *h,
                        color: c,
                    });
                    continue;
                },
                DisplayItem::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    color,
                    bold,
                    italic,
                    #[cfg(feature = "web-fonts")]
                    web_font_id,
                    ..
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;

                    #[cfg(feature = "web-fonts")]
                    if let Some(font_id) = web_font_id {
                        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                        let c = apply_layer_opacity(*color, layer_opacity);
                        if let Some(ref mut renderer) = web_font_renderer {
                            renderer.render(
                                backend,
                                text,
                                x + scroll_dx,
                                y + eff_dy,
                                *font_size,
                                c,
                                *font_id,
                            )?;
                        } else {
                            backend.draw_text_styled(
                                text,
                                x + scroll_dx,
                                y + eff_dy,
                                *font_size,
                                c,
                                *bold,
                                *italic,
                            )?;
                        }
                        continue;
                    }

                    let c = apply_layer_opacity(*color, layer_opacity);
                    let key = (*font_size, *bold, *italic);
                    if !text_batch.is_empty() && text_batch_key != key {
                        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                    }
                    text_batch_key = key;
                    text_batch.push(BatchText {
                        text,
                        x: x + scroll_dx,
                        y: y + eff_dy,
                        color: c,
                    });
                    continue;
                },
                _ => {
                    flush_rect_batch(backend, &mut rect_batch)?;
                    flush_text_batch(backend, &mut text_batch, text_batch_key)?;
                },
            }

            match item {
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                    ..
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rounded_rect(x + scroll_dx, y + eff_dy, *w, *h, *radius, c)?;
                },
                DisplayItem::FillPolygon { points, color, .. } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    let shifted = [
                        (points[0].0 + scroll_dx, points[0].1 + eff_dy),
                        (points[1].0 + scroll_dx, points[1].1 + eff_dy),
                        (points[2].0 + scroll_dx, points[2].1 + eff_dy),
                        (points[3].0 + scroll_dx, points[3].1 + eff_dy),
                    ];
                    backend.fill_polygon(&shifted, c)?;
                },
                DisplayItem::StrokeRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    stroke_width,
                    color,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.stroke_rounded_rect(
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        c,
                    )?;
                },
                DisplayItem::Blit {
                    texture,
                    x,
                    y,
                    w,
                    h,
                } => {
                    backend.blit(*texture, x + scroll_dx, y + eff_dy, *w, *h)?;
                },
                DisplayItem::BlitSub {
                    texture,
                    src_x,
                    src_y,
                    src_w,
                    src_h,
                    dst_x,
                    dst_y,
                    dst_w,
                    dst_h,
                } => {
                    backend.blit_sub(
                        *texture,
                        *src_x,
                        *src_y,
                        *src_w,
                        *src_h,
                        dst_x + scroll_dx,
                        dst_y + eff_dy,
                        *dst_w,
                        *dst_h,
                    )?;
                },
                DisplayItem::Gradient { x, y, w, h, style } => {
                    backend.fill_rect_gradient(x + scroll_dx, y + eff_dy, *w, *h, style)?;
                },
                DisplayItem::BorderEdge {
                    x,
                    y,
                    w,
                    h,
                    color,
                    style,
                    horizontal,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    replay_border_edge(
                        backend,
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        c,
                        *style,
                        *horizontal,
                    )?;
                },
                DisplayItem::Shadow {
                    x,
                    y,
                    w,
                    h,
                    blur,
                    spread,
                    offset_x,
                    offset_y,
                    color,
                    radius,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_shadow(
                        x + scroll_dx,
                        y + eff_dy,
                        *w,
                        *h,
                        *blur,
                        *spread,
                        *offset_x,
                        *offset_y,
                        c,
                        *radius,
                    )?;
                },
                DisplayItem::FillRect { .. }
                | DisplayItem::DrawText { .. }
                | DisplayItem::PushClip { .. }
                | DisplayItem::PopClip
                | DisplayItem::PushLayer { .. }
                | DisplayItem::PopLayer
                | DisplayItem::PushCompositingLayer { .. }
                | DisplayItem::PopCompositingLayer
                | DisplayItem::PushSticky { .. }
                | DisplayItem::PopSticky => {},
                DisplayItem::BlurHint { .. } => {},
            }
        }

        flush_rect_batch(backend, &mut rect_batch)?;
        flush_text_batch(backend, &mut text_batch, text_batch_key)?;
        backend.flush_batch()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Replay helpers
// ---------------------------------------------------------------------------

/// Flush accumulated `FillRect` items as a single batched call.
///
/// When the batch contains multiple rects, they are submitted via
/// `SdiBatch::submit_rect_batch` so backends can optimize them into
/// a single draw call. Single-rect batches fall through to `fill_rect`
/// to avoid the overhead of a batch submission for a trivial case.
fn flush_rect_batch(backend: &mut dyn SdiBackend, batch: &mut Vec<BatchRect>) -> Result<()> {
    match batch.len() {
        0 => {},
        1 => {
            let r = batch[0];
            backend.fill_rect(r.x, r.y, r.w, r.h, r.color)?;
        },
        _ => {
            backend.submit_rect_batch(batch)?;
        },
    }
    batch.clear();
    Ok(())
}

/// Flush accumulated `DrawText` items as a single batched call.
///
/// When the batch contains multiple texts with the same font style, they
/// are submitted via `SdiBatch::submit_text_batch` so backends can
/// coalesce glyph atlas lookups. Single-text batches fall through to
/// `draw_text_styled` to avoid overhead.
fn flush_text_batch(
    backend: &mut dyn SdiBackend,
    batch: &mut Vec<BatchText<'_>>,
    key: (u16, bool, bool),
) -> Result<()> {
    match batch.len() {
        0 => {},
        1 => {
            let t = &batch[0];
            backend.draw_text_styled(t.text, t.x, t.y, key.0, t.color, key.1, key.2)?;
        },
        _ => {
            backend.submit_text_batch(batch, key.0, key.1, key.2)?;
        },
    }
    batch.clear();
    Ok(())
}

/// Compute the intersection of two clip rectangles.
///
/// Returns the overlapping region. If the rectangles don't overlap,
/// returns a zero-size rect at the intersection point (the GPU will
/// clip everything, which is correct — nothing should be visible).
fn intersect_clip(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> (i32, i32, u32, u32) {
    let x1 = a.0.max(b.0);
    let y1 = a.1.max(b.1);
    let x2 = (a.0 + a.2 as i32).min(b.0 + b.2 as i32);
    let y2 = (a.1 + a.3 as i32).min(b.1 + b.3 as i32);
    let w = (x2 - x1).max(0) as u32;
    let h = (y2 - y1).max(0) as u32;
    (x1, y1, w, h)
}

/// Compute the product of all opacities in the layer stack.
///
/// Returns `1.0` when the stack is empty (no layers active), meaning
/// colors are passed through unmodified.
fn layer_opacity_product(stack: &[f32]) -> f32 {
    if stack.is_empty() {
        return 1.0;
    }
    stack.iter().copied().fold(1.0f32, |acc, o| acc * o)
}

/// Apply the cumulative layer opacity to a color's alpha channel.
///
/// When `opacity` is `1.0` this is a no-op (the compiler can often
/// elide the multiplication entirely).
fn apply_layer_opacity(color: Color, opacity: f32) -> Color {
    if (opacity - 1.0).abs() < f32::EPSILON {
        return color;
    }
    let a = (color.a as f32 * opacity).round().clamp(0.0, 255.0) as u8;
    Color::rgba(color.r, color.g, color.b, a)
}

/// Compute the sticky Y offset from the element's parameters and a scroll value.
///
/// This mirrors `compute_sticky_dy` in `record.rs` but works from stored
/// parameters rather than a live layout box, enabling offset recomputation
/// during scroll-delta replay.
fn compute_sticky_dy_from_params(
    natural_y: f32,
    box_height: f32,
    top_px: Option<f32>,
    bottom_px: Option<f32>,
    visible_viewport_h: f32,
    scroll_y: f32,
) -> i32 {
    if let Some(top) = top_px {
        let natural = natural_y - scroll_y;
        if natural < top {
            (top - natural) as i32
        } else {
            0
        }
    } else if let Some(bottom) = bottom_px {
        let natural = natural_y - scroll_y;
        let threshold = visible_viewport_h - bottom - box_height;
        if natural > threshold {
            (threshold - natural) as i32
        } else {
            0
        }
    } else {
        0
    }
}

/// Test whether two rectangles overlap.
fn rects_intersect(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

/// Replay a border edge using the correct style (solid, dashed, dotted, double).
#[allow(clippy::too_many_arguments)]
fn replay_border_edge(
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
        BorderStyle::Groove | BorderStyle::Ridge => {
            // 3D effect with two halves.
            let (outer, inner) = if style == BorderStyle::Groove {
                (darken_color(color), lighten_color(color))
            } else {
                (lighten_color(color), darken_color(color))
            };
            let thickness = if horizontal { h } else { w };
            let half = thickness / 2;
            let other = thickness - half;
            if horizontal {
                backend.fill_rect(x, y, w, half.max(1), outer)?;
                backend.fill_rect(x, y + half as i32, w, other.max(1), inner)?;
            } else {
                backend.fill_rect(x, y, half.max(1), h, outer)?;
                backend.fill_rect(x + half as i32, y, other.max(1), h, inner)?;
            }
        },
        BorderStyle::Inset | BorderStyle::Outset => {
            let shade = match (style, horizontal) {
                (BorderStyle::Inset, true) | (BorderStyle::Outset, false) => darken_color(color),
                _ => lighten_color(color),
            };
            backend.fill_rect(x, y, w, h, shade)?;
        },
        BorderStyle::None => {},
    }
    Ok(())
}

fn darken_color(c: Color) -> Color {
    Color::rgba(
        (c.r as f32 * 0.6) as u8,
        (c.g as f32 * 0.6) as u8,
        (c.b as f32 * 0.6) as u8,
        c.a,
    )
}

fn lighten_color(c: Color) -> Color {
    Color::rgba(
        (c.r as f32 + (255.0 - c.r as f32) * 0.4) as u8,
        (c.g as f32 + (255.0 - c.g as f32) * 0.4) as u8,
        (c.b as f32 + (255.0 - c.b as f32) * 0.4) as u8,
        c.a,
    )
}

#[cfg(test)]
mod tests {
    use oasis_types::backend::Color;

    use super::super::{DisplayItem, DisplayList};
    use super::{intersect_clip, rects_intersect};
    use crate::css::values::types::BlendMode as CssBlendMode;
    use crate::layout::box_model::Rect;
    use crate::test_utils::{DrawCall, MockBackend};

    fn layer_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn push_cl(bounds: Rect, opacity: f32) -> DisplayItem {
        DisplayItem::PushCompositingLayer {
            bounds,
            opacity,
            blend: CssBlendMode::Multiply,
            needs_backdrop: false,
            filters: Vec::new(),
            backdrop_filters: Vec::new(),
            mask: None,
        }
    }

    #[test]
    fn rects_intersect_overlapping() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let b = Rect {
            x: 50.0,
            y: 50.0,
            width: 100.0,
            height: 100.0,
        };
        assert!(rects_intersect(&a, &b));
    }

    #[test]
    fn rects_intersect_non_overlapping() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let b = Rect {
            x: 200.0,
            y: 200.0,
            width: 100.0,
            height: 100.0,
        };
        assert!(!rects_intersect(&a, &b));
    }

    #[test]
    fn intersect_clip_overlapping() {
        let result = intersect_clip((10, 10, 100, 100), (50, 50, 100, 100));
        assert_eq!(result, (50, 50, 60, 60));
    }

    #[test]
    fn intersect_clip_non_overlapping() {
        let result = intersect_clip((0, 0, 10, 10), (20, 20, 10, 10));
        assert_eq!(result, (20, 20, 0, 0));
    }

    #[test]
    fn intersect_clip_contained() {
        let result = intersect_clip((0, 0, 100, 100), (10, 10, 20, 20));
        assert_eq!(result, (10, 10, 20, 20));
    }

    #[test]
    fn replay_dirty_culls_offscreen_items() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 50,
            y: 50,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 500,
            y: 500,
            w: 20,
            h: 20,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });

        let dirty = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };

        let mut backend = MockBackend::new();
        dl.replay_dirty(&mut backend, &dirty, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
    }

    #[test]
    fn replay_applies_layer_opacity_to_fill_rect() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgba(255, 0, 0, 200),
            node_id: None,
        });
        dl.push(DisplayItem::PopLayer);

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { color, .. } = &backend.calls[0] {
            assert_eq!(color.a, 100);
            assert_eq!(color.r, 255);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn replay_applies_nested_layer_opacity() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgba(255, 0, 0, 200),
            node_id: None,
        });
        dl.push(DisplayItem::PopLayer);
        dl.push(DisplayItem::PopLayer);

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { color, .. } = &backend.calls[0] {
            // 200 * 0.5 * 0.5 = 50
            assert_eq!(color.a, 50);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn replay_no_layer_passes_color_through() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgba(255, 0, 0, 200),
            node_id: None,
        });

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        if let DrawCall::FillRect { color, .. } = &backend.calls[0] {
            assert_eq!(color.a, 200);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn replay_layer_opacity_applies_to_text() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::DrawText {
            text: "hello".into(),
            x: 0,
            y: 0,
            font_size: 12,
            color: Color::rgba(0, 0, 0, 254),
            bold: false,
            italic: false,
            width: 0,
            node_id: None,
            #[cfg(feature = "web-fonts")]
            web_font_id: None,
        });
        dl.push(DisplayItem::PopLayer);

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.draw_text_count(), 1);
        if let DrawCall::DrawText { color, .. } = &backend.calls[0] {
            assert_eq!(color.a, 127);
        } else {
            panic!("expected DrawText");
        }
    }

    #[test]
    fn replay_dirty_applies_layer_opacity() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgba(0, 255, 0, 100),
            node_id: None,
        });
        dl.push(DisplayItem::PopLayer);

        let dirty = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };

        let mut backend = MockBackend::new();
        dl.replay_dirty(&mut backend, &dirty, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        if let DrawCall::FillRect { color, .. } = &backend.calls[0] {
            assert_eq!(color.a, 50);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn replay_batches_consecutive_fill_rects() {
        let mut dl = DisplayList::new();
        for i in 0..3 {
            dl.push(DisplayItem::FillRect {
                x: i * 10,
                y: 0,
                w: 10,
                h: 10,
                color: Color::rgb(255, 0, 0),
                node_id: None,
            });
        }

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 3);
    }

    #[test]
    fn replay_flushes_batch_on_non_rect_item() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::DrawText {
            text: "hi".into(),
            x: 0,
            y: 0,
            font_size: 12,
            color: Color::BLACK,
            bold: false,
            italic: false,
            width: 1,
            node_id: None,
            #[cfg(feature = "web-fonts")]
            web_font_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 2);
        assert_eq!(backend.draw_text_count(), 1);
        assert!(matches!(&backend.calls[0], DrawCall::FillRect { .. }));
        assert!(matches!(&backend.calls[1], DrawCall::DrawText { .. }));
        assert!(matches!(&backend.calls[2], DrawCall::FillRect { .. }));
    }

    #[test]
    fn push_compositing_layer_replays_as_opacity_fallback() {
        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(0.0, 0.0, 100.0, 100.0), 0.5));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgba(100, 100, 100, 200),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();
        assert_eq!(backend.fill_rect_count(), 1);
        match &backend.calls[0] {
            DrawCall::FillRect { color, .. } => {
                assert_eq!(color.a, 100);
            },
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn compositing_layer_drives_render_target_commands() {
        // Against a backend that reports `supports_render_targets()`,
        // replay() must create+bind+draw+unbind+composite+destroy.
        use oasis_test_backend::RecordingBackend;
        use oasis_types::backend::DrawCommand;

        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(10.0, 20.0, 80.0, 60.0), 0.75));
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 20,
            w: 40,
            h: 30,
            color: Color::rgba(100, 150, 200, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = RecordingBackend::new(480, 272);
        dl.replay(&mut backend, 0, 0, None).unwrap();

        let cmds = backend.commands();
        let create_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::CreateRenderTarget { .. }))
            .expect("create fired");
        let bind_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::BindRenderTarget { .. }))
            .expect("bind fired");
        let unbind_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::UnbindRenderTarget))
            .expect("unbind fired");
        let composite_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::CompositeRenderTarget { .. }))
            .expect("composite fired");
        let destroy_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::DestroyRenderTarget { .. }))
            .expect("destroy fired");
        assert!(create_idx < bind_idx);
        assert!(bind_idx < unbind_idx);
        assert!(unbind_idx < composite_idx);
        assert!(composite_idx < destroy_idx);

        let composite = &cmds[composite_idx];
        match composite {
            DrawCommand::CompositeRenderTarget {
                dst_x,
                dst_y,
                dst_w,
                dst_h,
                blend,
                opacity,
                ..
            } => {
                assert_eq!((*dst_x, *dst_y, *dst_w, *dst_h), (10, 20, 80, 60));
                assert_eq!(*blend, oasis_types::backend::BlendMode::Multiply);
                assert!((*opacity - 0.75).abs() < 1e-5);
            },
            _ => panic!("expected CompositeRenderTarget"),
        }

        let inner_fill = cmds[bind_idx + 1..unbind_idx]
            .iter()
            .any(|c| matches!(c, DrawCommand::FillRect { .. }));
        assert!(inner_fill, "inner fill recorded outside the layer");
    }

    #[test]
    fn compositing_layer_translates_contents_to_local_space() {
        use oasis_test_backend::RecordingBackend;
        use oasis_types::backend::DrawCommand;

        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(50.0, 100.0, 30.0, 20.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 60,
            y: 110,
            w: 10,
            h: 10,
            color: Color::rgba(0, 0, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = RecordingBackend::new(480, 272);
        dl.replay(&mut backend, 0, 0, None).unwrap();

        let cmds = backend.commands();
        let bind_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::BindRenderTarget { .. }))
            .expect("bind");
        let unbind_idx = cmds
            .iter()
            .position(|c| matches!(c, DrawCommand::UnbindRenderTarget))
            .expect("unbind");
        let inner = cmds[bind_idx + 1..unbind_idx]
            .iter()
            .find_map(|c| match c {
                DrawCommand::FillRect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
                _ => None,
            })
            .expect("inner fill");
        assert_eq!(inner, (10, 10, 10, 10), "local-space translation wrong");
    }

    /// Regression test for the nested compositing layer coordinate
    /// bug found in PR #114 review. Outer layer at (10, 20), inner
    /// layer at (50, 60), inner content at screen (55, 65). The
    /// inner content must land at inner-local (5, 5), and after the
    /// inner pop the outer content at screen (15, 25) must land at
    /// outer-local (5, 5) — not at compositor_dx +/- accumulated
    /// junk.
    #[test]
    fn nested_compositing_layers_use_per_layer_local_space() {
        use oasis_test_backend::RecordingBackend;
        use oasis_types::backend::DrawCommand;

        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(10.0, 20.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 25,
            w: 1,
            h: 1,
            color: Color::rgba(255, 0, 0, 255),
            node_id: None,
        });
        dl.push(push_cl(layer_rect(50.0, 60.0, 100.0, 100.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 55,
            y: 65,
            w: 2,
            h: 2,
            color: Color::rgba(0, 255, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        dl.push(DisplayItem::FillRect {
            x: 16,
            y: 26,
            w: 3,
            h: 3,
            color: Color::rgba(0, 0, 255, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = RecordingBackend::new(480, 272);
        dl.replay(&mut backend, 0, 0, None).unwrap();
        let cmds = backend.commands();

        let mut fills = cmds.iter().filter_map(|c| match c {
            DrawCommand::FillRect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        });
        assert_eq!(fills.next(), Some((5, 5, 1, 1)), "outer-before");
        assert_eq!(fills.next(), Some((5, 5, 2, 2)), "inner");
        assert_eq!(fills.next(), Some((6, 6, 3, 3)), "outer-after");
    }
}
