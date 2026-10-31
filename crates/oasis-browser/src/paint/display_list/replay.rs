//! Display-list replay against an `SdiBackend`.
//!
//! One walk serves both entry points:
//!
//! - [`DisplayList::replay`] draws every item in order, applying scroll
//!   translation, opacity stacks, clip stacks, sticky offset
//!   re-computation, and compositing-layer offscreen render-target
//!   management.
//! - [`DisplayList::replay_dirty`] additionally culls drawables whose
//!   bounds don't intersect a supplied dirty rectangle, and composites
//!   layers by opacity stacking plus the per-color filter approximation
//!   (no offscreen targets: a partial redraw can't rebuild a whole layer).
//!
//! Both batch consecutive `FillRect` items into [`SdiBatch::submit_rect_batch`]
//! and consecutive same-style `DrawText` items into
//! [`SdiBatch::submit_text_batch`] so backends can emit a single GPU
//! draw call per batch.
//!
//! [`SdiBatch::submit_rect_batch`]: oasis_types::backend::SdiBatch::submit_rect_batch
//! [`SdiBatch::submit_text_batch`]: oasis_types::backend::SdiBatch::submit_text_batch

use oasis_types::backend::{
    BatchRect, BatchText, BlendMode as BackendBlendMode, Color, GradientStyle, RenderTargetId,
    SdiBackend,
};
use oasis_types::error::Result;

#[cfg(feature = "web-fonts")]
use super::WebFontRenderer;
use super::mask::{MaskParams, apply_mask};
use super::{DisplayItem, DisplayList, sticky_offset};
use crate::css::values::BorderStyle;
use crate::css::values::types::{BlendMode as CssBlendMode, FilterFunction};
use crate::layout::box_model::Rect;

/// Span of surface coordinates (from 0) an oversized offscreen layer
/// is clipped to. Covers a 4K framebuffer or a maximal parent layer.
const LAYER_SURFACE_SPAN: i32 = 4096;
/// Extra pixels kept on each side of the clipped span so filters that
/// sample neighbours (`blur()`) still see content just off-surface.
const LAYER_PAD: i32 = 128;
/// Longest layer side allocated as-is; longer sides are clipped.
const MAX_LAYER_SIDE: u32 = (LAYER_SURFACE_SPAN + 2 * LAYER_PAD) as u32;

/// A clip rectangle `(x, y, w, h)`.
type ClipBox = (i32, i32, u32, u32);

/// The glyph renderer handed to a replay (`None` = backend bitmap text).
#[cfg(feature = "web-fonts")]
pub(super) type Fonts<'r> = Option<&'r mut dyn WebFontRenderer>;
/// The glyph renderer handed to a replay; without `web-fonts` there is
/// none, so this is always `None`.
#[cfg(not(feature = "web-fonts"))]
pub(super) type Fonts<'r> = Option<std::marker::PhantomData<&'r ()>>;

/// Bound one axis (`pos`, `len` in the current surface's coordinates)
/// of an offscreen layer.
///
/// An offscreen target costs `w * h * 4` bytes on every backend (and a
/// CPU filter pass reads that much back), so a `33_554_432 px` box with
/// `opacity` / `filter` used to request terabytes. Layers up to
/// [`MAX_LAYER_SIDE`] are untouched; a longer axis is clipped to the
/// part that can reach the surface, which is all that can show.
fn bound_layer_axis(pos: i32, len: u32) -> (i32, u32) {
    if len <= MAX_LAYER_SIDE {
        return (pos, len);
    }
    let lo = pos.max(-LAYER_PAD);
    let hi = (i64::from(pos) + i64::from(len)).min(i64::from(LAYER_SURFACE_SPAN + LAYER_PAD));
    if hi <= i64::from(lo) {
        (lo, 1)
    } else {
        (lo, (hi - i64::from(lo)) as u32)
    }
}

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

/// Entry in the compositor layer stack kept during a replay.
struct ActiveLayer<'a> {
    /// Allocated render target id. `None` when the backend has no
    /// render targets (or this is a dirty replay) and the layer fell
    /// through to the opacity-only fast path.
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
    filters: &'a [FilterFunction],
    /// Optional `mask-*` parameters. When present, the pop path
    /// rasterizes the mask source over the layer bounds and applies
    /// it to the layer's alpha channel before the final composite.
    mask: Option<&'a MaskParams>,
    /// `(compositor_dx, compositor_dy)` from before this layer was
    /// pushed, restored on `PopCompositingLayer` so nested layers don't
    /// accumulate translation (an inner layer's origin subtracted on top
    /// of the outer layer's would leave items off-target after the pop).
    saved_compositor: (i32, i32),
}

/// Mutable state of one replay walk.
struct ReplayState<'a> {
    /// Outer clip (browser window content area, screen space) restored
    /// when the clip stack empties.
    base_clip: Option<ClipBox>,
    opacity_stack: Vec<f32>,
    /// One entry per active compositing layer: the box-level filter
    /// chain to approximate per color on the items inside it. Empty
    /// when the layer's pixels get the real per-pixel chain on readback
    /// (render target + readback support), so a filter is never applied
    /// twice.
    filter_stack: Vec<&'a [FilterFunction]>,
    /// The *intersected* clip for each level, in screen space (scroll
    /// and sticky applied, compositor offset not): clips pushed inside
    /// and outside an offscreen layer intersect in one coordinate space,
    /// and are translated into the bound surface only when applied.
    clip_stack: Vec<ClipBox>,
    /// Sticky correction stack: Y offset adjustment for sticky elements
    /// when replaying with a scroll delta different from recording time.
    sticky_dy_stack: Vec<i32>,
    layer_stack: Vec<ActiveLayer<'a>>,
    /// Translation from screen space into the bound offscreen layer
    /// (`-origin` of the innermost layer with a render target; zero on
    /// the framebuffer).
    compositor_dx: i32,
    compositor_dy: i32,
    /// Consecutive `FillRect` items awaiting batched submission.
    rect_batch: Vec<BatchRect>,
    /// Consecutive same-style `DrawText` items, keyed by
    /// `(font_size, bold, italic)`.
    text_batch: Vec<BatchText<'a>>,
    text_batch_key: (u16, bool, bool),
}

impl DisplayList {
    /// The one replay walk behind [`replay`](Self::replay),
    /// [`replay_dirty`](Self::replay_dirty) and their `_with_fonts`
    /// variants. `dirty = Some(rect)` culls drawables outside `rect`
    /// and never allocates offscreen layers.
    pub(super) fn replay_inner(
        &self,
        backend: &mut dyn SdiBackend,
        dirty: Option<&Rect>,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<ClipBox>,
        mut fonts: Fonts<'_>,
    ) -> Result<()> {
        let supports_rt = dirty.is_none() && backend.supports_render_targets();
        let mut st = ReplayState::new(base_clip);

        for item in &self.items {
            // State items: always replayed (dirty culling included), and
            // each one ends the current batches.
            match item {
                DisplayItem::PushLayer { opacity } => {
                    st.flush(backend)?;
                    st.opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopLayer => {
                    st.flush(backend)?;
                    st.opacity_stack.pop();
                    continue;
                },
                DisplayItem::PushCompositingLayer {
                    bounds,
                    opacity,
                    blend,
                    filters,
                    mask,
                } => {
                    st.flush(backend)?;
                    // `bounds` is screen space at recording time; where
                    // the layer lands now is shifted by the replay
                    // scroll delta and any enclosing sticky correction.
                    let x = bounds.x as i32 + scroll_dx;
                    let y = bounds.y as i32 + scroll_dy + st.sticky();
                    let layer = PendingLayer {
                        x,
                        y,
                        w: bounds.width.max(1.0) as u32,
                        h: bounds.height.max(1.0) as u32,
                        opacity: *opacity,
                        blend: css_blend_to_backend(*blend),
                        filters,
                        mask: mask.as_ref(),
                    };
                    st.push_compositing_layer(backend, supports_rt, layer);
                    continue;
                },
                DisplayItem::PopCompositingLayer => {
                    st.flush(backend)?;
                    st.pop_compositing_layer(backend)?;
                    continue;
                },
                DisplayItem::PushSticky {
                    natural_y,
                    box_height,
                    top_px,
                    bottom_px,
                    visible_viewport_h,
                } => {
                    st.flush(backend)?;
                    // Sticky offset at the replay scroll minus the one
                    // baked in at recording time.
                    let at = |scroll: f32| {
                        sticky_offset(
                            *natural_y,
                            *box_height,
                            *top_px,
                            *bottom_px,
                            *visible_viewport_h,
                            scroll,
                        )
                    };
                    let eff_scroll = self.recording_scroll_y - scroll_dy as f32;
                    st.sticky_dy_stack
                        .push(at(eff_scroll) - at(self.recording_scroll_y));
                    continue;
                },
                DisplayItem::PopSticky => {
                    st.flush(backend)?;
                    st.sticky_dy_stack.pop();
                    continue;
                },
                DisplayItem::PushClip { x, y, w, h } => {
                    st.flush(backend)?;
                    let raw = (x + scroll_dx, y + scroll_dy + st.sticky(), *w, *h);
                    st.push_clip(backend, raw)?;
                    continue;
                },
                DisplayItem::PopClip => {
                    st.flush(backend)?;
                    st.pop_clip(backend)?;
                    continue;
                },
                _ => {},
            }

            // Inside an offscreen layer items draw at target-local
            // coordinates: the compositor offset maps screen space there.
            let dx = scroll_dx + st.compositor_dx;
            let dy = scroll_dy + st.compositor_dy + st.sticky();
            if let Some(dirty) = dirty
                && let Some(bounds) = item.bounds()
            {
                let shifted = Rect {
                    x: bounds.x + dx as f32,
                    y: bounds.y + dy as f32,
                    width: bounds.width,
                    height: bounds.height,
                };
                if !rects_intersect(&shifted, dirty) {
                    continue;
                }
            }
            st.draw(backend, item, dx, dy, &mut fonts)?;
        }

        st.flush(backend)
    }
}

/// A `PushCompositingLayer` resolved to the replay's screen space.
struct PendingLayer<'a> {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    opacity: f32,
    blend: BackendBlendMode,
    filters: &'a [FilterFunction],
    mask: Option<&'a MaskParams>,
}

impl<'a> ReplayState<'a> {
    fn new(base_clip: Option<ClipBox>) -> Self {
        Self {
            base_clip,
            opacity_stack: Vec::new(),
            filter_stack: Vec::new(),
            clip_stack: Vec::new(),
            sticky_dy_stack: Vec::new(),
            layer_stack: Vec::new(),
            compositor_dx: 0,
            compositor_dy: 0,
            rect_batch: Vec::new(),
            text_batch: Vec::new(),
            text_batch_key: (0, false, false),
        }
    }

    /// The innermost sticky correction (0 outside sticky boxes).
    fn sticky(&self) -> i32 {
        self.sticky_dy_stack.last().copied().unwrap_or(0)
    }

    /// Submit both pending batches.
    fn flush(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        flush_rect_batch(backend, &mut self.rect_batch)?;
        flush_text_batch(backend, &mut self.text_batch, self.text_batch_key)
    }

    /// Apply a screen-space clip to the bound surface.
    fn apply_clip(&self, backend: &mut dyn SdiBackend, clip: ClipBox) -> Result<()> {
        backend.set_clip_rect(
            clip.0 + self.compositor_dx,
            clip.1 + self.compositor_dy,
            clip.2,
            clip.3,
        )
    }

    /// Push a screen-space clip, intersected with the parent clip (or
    /// the base clip) for tighter bounds.
    fn push_clip(&mut self, backend: &mut dyn SdiBackend, raw: ClipBox) -> Result<()> {
        let effective = match self.clip_stack.last().copied().or(self.base_clip) {
            Some(parent) => intersect_clip(parent, raw),
            None => raw,
        };
        self.clip_stack.push(effective);
        self.apply_clip(backend, effective)
    }

    /// Pop a clip, restoring the parent clip, the base clip (browser
    /// window), or no clip.
    fn pop_clip(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.clip_stack.pop();
        match self.clip_stack.last().copied().or(self.base_clip) {
            Some(clip) => self.apply_clip(backend, clip),
            None => backend.reset_clip_rect(),
        }
    }

    /// Open a compositing layer: an offscreen render target when
    /// `supports_rt` and the backend can allocate one, otherwise plain
    /// opacity stacking (blend mode dropped, filters degraded to the
    /// per-color approximation — the accepted degradation in
    /// `docs/archive/compositor-overhaul-plan.md` §3.4 step 4).
    fn push_compositing_layer(
        &mut self,
        backend: &mut dyn SdiBackend,
        supports_rt: bool,
        layer: PendingLayer<'a>,
    ) {
        // Express the layer in the currently bound surface (the
        // framebuffer, or an enclosing layer's target), clipped to the
        // part that can reach it.
        let (dst_x, dst_w) = bound_layer_axis(layer.x + self.compositor_dx, layer.w);
        let (dst_y, dst_h) = bound_layer_axis(layer.y + self.compositor_dy, layer.h);
        let id = if supports_rt {
            match backend.create_render_target(dst_w, dst_h) {
                Ok(id) if backend.bind_render_target(id).is_ok() => Some(id),
                Ok(id) => {
                    let _ = backend.destroy_render_target(id);
                    None
                },
                Err(_) => None,
            }
        } else {
            None
        };
        let saved_compositor = (self.compositor_dx, self.compositor_dy);
        if id.is_some() {
            // Contents draw at target-local coordinates: the layer's
            // current screen origin becomes (0, 0). Each render target
            // has its own space, so this absolute reset is right for
            // nested layers too.
            self.compositor_dx = -(dst_x - saved_compositor.0);
            self.compositor_dy = -(dst_y - saved_compositor.1);
        } else {
            self.opacity_stack.push(layer.opacity);
        }
        let per_pixel_filter = id.is_some() && backend.supports_render_target_readback();
        self.filter_stack
            .push(if per_pixel_filter { &[] } else { layer.filters });
        self.layer_stack.push(ActiveLayer {
            id,
            dst_x,
            dst_y,
            dst_w,
            dst_h,
            opacity: layer.opacity,
            blend: layer.blend,
            filters: layer.filters,
            mask: layer.mask,
            saved_compositor,
        });
    }

    /// Close the innermost compositing layer, compositing its target
    /// onto the parent surface. The target is destroyed even when
    /// unbinding or compositing fails.
    fn pop_compositing_layer(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        let Some(layer) = self.layer_stack.pop() else {
            return Ok(());
        };
        self.filter_stack.pop();
        let Some(id) = layer.id else {
            self.opacity_stack.pop();
            return Ok(());
        };
        (self.compositor_dx, self.compositor_dy) = layer.saved_compositor;
        let unbound = backend.unbind_render_target();
        let composited = if unbound.is_ok() {
            composite_layer(backend, id, &layer)
        } else {
            Ok(())
        };
        let destroyed = backend.destroy_render_target(id);
        unbound.and(composited).and(destroyed)
    }

    /// Draw one drawable item at offset `(dx, dy)`, batching
    /// consecutive `FillRect`s and same-style `DrawText`s.
    fn draw(
        &mut self,
        backend: &mut dyn SdiBackend,
        item: &'a DisplayItem,
        dx: i32,
        dy: i32,
        fonts: &mut Fonts<'_>,
    ) -> Result<()> {
        #[cfg(not(feature = "web-fonts"))]
        let _ = fonts;
        let opacity = layer_opacity_product(&self.opacity_stack);
        match item {
            DisplayItem::FillRect {
                x, y, w, h, color, ..
            } => {
                flush_text_batch(backend, &mut self.text_batch, self.text_batch_key)?;
                self.rect_batch.push(BatchRect {
                    x: x + dx,
                    y: y + dy,
                    w: *w,
                    h: *h,
                    color: layer_color(*color, opacity, &self.filter_stack),
                });
                return Ok(());
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
                flush_rect_batch(backend, &mut self.rect_batch)?;
                let c = layer_color(*color, opacity, &self.filter_stack);

                // Web font path: render glyphs via the font registry
                // when a renderer is attached, else bitmap fallback.
                #[cfg(feature = "web-fonts")]
                if let Some(font_id) = web_font_id {
                    flush_text_batch(backend, &mut self.text_batch, self.text_batch_key)?;
                    let (tx, ty, size) = (x + dx, y + dy, *font_size);
                    return match fonts {
                        Some(renderer) => renderer.render(backend, text, tx, ty, size, c, *font_id),
                        None => backend.draw_text_styled(text, tx, ty, size, c, *bold, *italic),
                    };
                }

                let key = (*font_size, *bold, *italic);
                if !self.text_batch.is_empty() && self.text_batch_key != key {
                    flush_text_batch(backend, &mut self.text_batch, self.text_batch_key)?;
                }
                self.text_batch_key = key;
                self.text_batch.push(BatchText {
                    text,
                    x: x + dx,
                    y: y + dy,
                    color: c,
                });
                return Ok(());
            },
            // Any other item breaks both batches.
            _ => self.flush(backend)?,
        }

        let color = |c: Color| layer_color(c, opacity, &self.filter_stack);
        match item {
            DisplayItem::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color: c,
                ..
            } => backend.fill_rounded_rect(x + dx, y + dy, *w, *h, *radius, color(*c)),
            DisplayItem::FillPolygon {
                points, color: c, ..
            } => {
                let shifted = points.map(|(px, py)| (px + dx, py + dy));
                backend.fill_polygon(&shifted, color(*c))
            },
            DisplayItem::Blit {
                texture,
                x,
                y,
                w,
                h,
            } => backend.blit(*texture, x + dx, y + dy, *w, *h),
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
            } => backend.blit_sub(
                *texture,
                *src_x,
                *src_y,
                *src_w,
                *src_h,
                dst_x + dx,
                dst_y + dy,
                *dst_w,
                *dst_h,
            ),
            DisplayItem::Gradient { x, y, w, h, style } => {
                let g = layer_gradient(style, opacity, &self.filter_stack);
                backend.fill_rect_gradient(x + dx, y + dy, *w, *h, &g)
            },
            DisplayItem::BorderEdge {
                x,
                y,
                w,
                h,
                color: c,
                style,
                horizontal,
            } => replay_border_edge(
                backend,
                x + dx,
                y + dy,
                *w,
                *h,
                color(*c),
                *style,
                *horizontal,
            ),
            DisplayItem::Shadow {
                x,
                y,
                w,
                h,
                blur,
                spread,
                offset_x,
                offset_y,
                color: c,
                radius,
            } => backend.fill_shadow(
                x + dx,
                y + dy,
                *w,
                *h,
                *blur,
                *spread,
                *offset_x,
                *offset_y,
                color(*c),
                *radius,
            ),
            DisplayItem::Svg { .. } | DisplayItem::Canvas { .. } => {
                replay_embedded(backend, item, dx, dy, opacity)
            },
            // Batched above, or state items handled by the walk.
            DisplayItem::FillRect { .. }
            | DisplayItem::DrawText { .. }
            | DisplayItem::PushClip { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushLayer { .. }
            | DisplayItem::PopLayer
            | DisplayItem::PushCompositingLayer { .. }
            | DisplayItem::PopCompositingLayer
            | DisplayItem::PushSticky { .. }
            | DisplayItem::PopSticky => Ok(()),
        }
    }
}

/// Composite an unbound layer target onto the parent surface.
///
/// With a filter or mask and a backend that can read the target back,
/// the pixels run through the CPU chain (filters, mask, opacity, then a
/// CPU blend against the destination for non-`Normal` blend modes) and
/// are re-uploaded; otherwise the backend composites the target itself.
fn composite_layer(
    backend: &mut dyn SdiBackend,
    id: RenderTargetId,
    layer: &ActiveLayer<'_>,
) -> Result<()> {
    let (x, y, w, h) = (layer.dst_x, layer.dst_y, layer.dst_w, layer.dst_h);
    let cpu_pass = (!layer.filters.is_empty() || layer.mask.is_some())
        && backend.supports_render_target_readback();
    if cpu_pass {
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        if backend.read_render_target(id, &mut buf).is_ok() {
            if !layer.filters.is_empty() {
                crate::paint::filter_chain::apply_filter_chain(&mut buf, w, h, layer.filters);
            }
            if let Some(mask) = layer.mask {
                apply_mask(&mut buf, w, h, mask);
            }
            // Pre-multiply opacity into alpha.
            if (layer.opacity - 1.0).abs() > f32::EPSILON {
                let f = layer.opacity.clamp(0.0, 1.0);
                for px in buf.as_chunks_mut::<4>().0.iter_mut() {
                    px[3] = ((px[3] as f32) * f).round() as u8;
                }
            }
            // Non-Normal blend: composite against the destination on
            // the CPU so the blend mode survives the filter/mask path
            // (falling back to a Normal blit if it can't be read).
            if !layer.blend.is_normal()
                && let Ok(mut dst) = backend.read_pixels(x, y, w, h)
            {
                crate::paint::filter_chain::cpu_blend_composite(&buf, &mut dst, w, h, layer.blend);
                buf = dst;
            }
            if let Ok(tex) = backend.load_texture(w, h, &buf) {
                let _ = backend.blit(tex, x, y, w, h);
                let _ = backend.destroy_texture(tex);
            }
            return Ok(());
        }
    }
    backend.composite_render_target(id, x, y, w, h, layer.blend, layer.opacity)
}

// ---------------------------------------------------------------------------
// Replay helpers
// ---------------------------------------------------------------------------

/// Replay an [`DisplayItem::Svg`] / [`DisplayItem::Canvas`] item, whose
/// content issues its own backend calls, with its top-left at
/// `(x + dx, y + dy)`.
///
/// The active clip already applies (it is backend state). Under a
/// translucent opacity layer the content is drawn into an offscreen
/// render target and composited back at `opacity` when the backend
/// supports render targets; otherwise it is drawn opaque (the same
/// degradation `PushCompositingLayer` accepts on PSP).
fn replay_embedded(
    backend: &mut dyn SdiBackend,
    item: &DisplayItem,
    dx: i32,
    dy: i32,
    opacity: f32,
) -> Result<()> {
    let Some(bounds) = item.bounds() else {
        return Ok(());
    };
    let x = bounds.x as i32 + dx;
    let y = bounds.y as i32 + dy;
    let w = bounds.width.max(0.0).ceil() as u32;
    let h = bounds.height.max(0.0).ceil() as u32;
    if w == 0 || h == 0 {
        return Ok(());
    }
    // The offscreen target covers only the part that can reach the
    // surface; the content draws shifted by the clipped-off amount.
    let (tx, tw) = bound_layer_axis(x, w);
    let (ty, th) = bound_layer_axis(y, h);
    let draw = |backend: &mut dyn SdiBackend, ox: i32, oy: i32| -> Result<()> {
        match item {
            DisplayItem::Svg {
                element,
                width,
                height,
                ..
            } => crate::svg::paint_svg(element, backend, ox, oy, *width, *height),
            DisplayItem::Canvas { state, w, h, .. } => match state.try_borrow() {
                Ok(state) => crate::canvas::paint_canvas(&state, backend, ox, oy, *w, *h),
                // A JS callback holding the canvas mid-replay can't
                // happen on the UI thread; skip rather than panic.
                Err(_) => Ok(()),
            },
            _ => Ok(()),
        }
    };
    if opacity < 1.0
        && backend.supports_render_targets()
        && let Ok(id) = backend.create_render_target(tw, th)
    {
        if backend.bind_render_target(id).is_ok() {
            let drawn = draw(backend, x - tx, y - ty);
            let unbound = backend.unbind_render_target();
            let composited = if drawn.is_ok() && unbound.is_ok() {
                backend.composite_render_target(
                    id,
                    tx,
                    ty,
                    tw,
                    th,
                    BackendBlendMode::Normal,
                    opacity,
                )
            } else {
                Ok(())
            };
            let destroyed = backend.destroy_render_target(id);
            return drawn.and(unbound).and(composited).and(destroyed);
        }
        backend.destroy_render_target(id)?;
    }
    draw(backend, x, y)
}

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

/// A recorded color as drawn inside the active layers: each layer's
/// per-color filter approximation (innermost first, as filters apply to
/// a layer's content before its ancestors'), then the layer opacity.
fn layer_color(color: Color, opacity: f32, filters: &[&[FilterFunction]]) -> Color {
    let mut c = color;
    for chain in filters.iter().rev() {
        if !chain.is_empty() {
            c = crate::paint::filters::apply_filters(c, chain);
        }
    }
    apply_layer_opacity(c, opacity)
}

/// [`layer_color`] applied to every stop of a gradient (the recorder
/// leaves layer opacity and filters to replay, like for flat fills).
fn layer_gradient(
    style: &GradientStyle,
    opacity: f32,
    filters: &[&[FilterFunction]],
) -> GradientStyle {
    let c = |color: Color| layer_color(color, opacity, filters);
    match *style {
        GradientStyle::Vertical { top, bottom } => GradientStyle::Vertical {
            top: c(top),
            bottom: c(bottom),
        },
        GradientStyle::Horizontal { left, right } => GradientStyle::Horizontal {
            left: c(left),
            right: c(right),
        },
        GradientStyle::FourCorner {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => GradientStyle::FourCorner {
            top_left: c(top_left),
            top_right: c(top_right),
            bottom_left: c(bottom_left),
            bottom_right: c(bottom_right),
        },
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
            filters: Vec::new(),
            mask: None,
        }
    }

    /// Regression: a layer's offscreen target was sized from its full
    /// bounds, so a 33-million-pixel box with `opacity` asked every
    /// backend for terabytes. Normal layers keep their exact rect.
    #[test]
    fn oversized_layer_axes_are_clipped_to_the_surface() {
        use super::{LAYER_PAD, LAYER_SURFACE_SPAN, MAX_LAYER_SIDE, bound_layer_axis};
        assert_eq!(bound_layer_axis(-500, 800), (-500, 800));
        assert_eq!(bound_layer_axis(10, MAX_LAYER_SIDE), (10, MAX_LAYER_SIDE));
        let (x, w) = bound_layer_axis(-33_554_432, 67_108_864);
        assert_eq!(x, -LAYER_PAD);
        assert_eq!(w, MAX_LAYER_SIDE);
        // Entirely past the surface: a 1-pixel placeholder.
        assert_eq!(
            bound_layer_axis(LAYER_SURFACE_SPAN * 4, 10_000),
            (LAYER_SURFACE_SPAN * 4, 1)
        );
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
        use oasis_test_backend::DrawCommand;
        use oasis_test_backend::RecordingBackend;

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
        use oasis_test_backend::DrawCommand;
        use oasis_test_backend::RecordingBackend;

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
        use oasis_test_backend::DrawCommand;
        use oasis_test_backend::RecordingBackend;

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

        // The inner layer composites into the outer target, so its
        // destination is in outer-local space: (50, 60) - (10, 20).
        let dsts: Vec<(i32, i32)> = cmds
            .iter()
            .filter_map(|c| match c {
                DrawCommand::CompositeRenderTarget { dst_x, dst_y, .. } => Some((*dst_x, *dst_y)),
                _ => None,
            })
            .collect();
        assert_eq!(dsts, vec![(40, 40), (10, 20)], "inner then outer composite");
    }

    /// Replaying a cached list with a scroll delta must move the layer
    /// with its contents: before the fix the target stayed at the
    /// recorded position while the contents shifted inside it, so a
    /// scrolled filter / blend / isolation layer drew clipped or empty.
    #[test]
    fn compositing_layer_follows_replay_scroll_delta() {
        use oasis_test_backend::DrawCommand;
        use oasis_test_backend::RecordingBackend;

        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(10.0, 120.0, 40.0, 40.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 125,
            w: 4,
            h: 4,
            color: Color::rgba(0, 0, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = RecordingBackend::new(480, 272);
        dl.replay(&mut backend, 0, -50, None).unwrap();
        let cmds = backend.commands();
        let inner = cmds.iter().find_map(|c| match c {
            DrawCommand::FillRect { x, y, .. } => Some((*x, *y)),
            _ => None,
        });
        assert_eq!(inner, Some((5, 5)), "contents stay at layer-local (5, 5)");
        let dst = cmds.iter().find_map(|c| match c {
            DrawCommand::CompositeRenderTarget { dst_x, dst_y, .. } => Some((*dst_x, *dst_y)),
            _ => None,
        });
        assert_eq!(
            dst,
            Some((10, 70)),
            "layer composited at its scrolled position"
        );
    }

    /// Regression: a clip pushed inside an offscreen layer was
    /// intersected, in layer-local coordinates, with the screen-space
    /// base clip, so an `overflow: hidden` child of a translucent box
    /// lost its top rows (here: everything above screen y = 30 mapped
    /// to layer-local y = 30). Clips now intersect in screen space and
    /// are translated into the bound target when applied.
    #[test]
    fn clip_inside_offscreen_layer_intersects_in_screen_space() {
        use oasis_test_backend::DrawCommand;
        use oasis_test_backend::RecordingBackend;

        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(100.0, 100.0, 100.0, 100.0), 0.5));
        dl.push(DisplayItem::PushClip {
            x: 110,
            y: 110,
            w: 50,
            h: 50,
        });
        dl.push(DisplayItem::FillRect {
            x: 110,
            y: 110,
            w: 50,
            h: 50,
            color: Color::rgba(0, 0, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopClip);
        dl.push(DisplayItem::PopCompositingLayer);

        let mut backend = RecordingBackend::new(480, 272);
        dl.replay(&mut backend, 0, 0, Some((0, 30, 480, 242))).unwrap();

        let cmds = backend.commands();
        let clip = cmds.iter().find_map(|c| match c {
            DrawCommand::SetClip { x, y, w, h } => Some((*x, *y, *w, *h)),
            _ => None,
        });
        assert_eq!(clip, Some((10, 10, 50, 50)), "clip in layer-local space");
    }

    /// The dirty path shares the full walk: clips, sticky offsets and
    /// culling agree with `replay` for items inside the dirty rect.
    #[test]
    fn replay_dirty_draws_what_replay_draws_inside_the_rect() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushClip {
            x: 0,
            y: 0,
            w: 40,
            h: 40,
        });
        for i in 0..4 {
            dl.push(DisplayItem::FillRect {
                x: i * 30,
                y: 0,
                w: 10,
                h: 10,
                color: Color::rgb(255, 0, 0),
                node_id: None,
            });
        }
        dl.push(DisplayItem::PopClip);
        let dirty = Rect {
            x: 0.0,
            y: 0.0,
            width: 45.0,
            height: 45.0,
        };
        let mut full = MockBackend::new();
        dl.replay(&mut full, 0, 0, None).unwrap();
        let mut part = MockBackend::new();
        dl.replay_dirty(&mut part, &dirty, 0, 0, None).unwrap();
        assert_eq!(full.fill_rect_count(), 4);
        assert_eq!(part.fill_rect_count(), 2, "items at x=60/90 culled");
    }
}
