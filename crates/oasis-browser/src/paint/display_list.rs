//! Display list intermediate representation (light compositor).
//!
//! Instead of issuing draw calls directly during the layout tree walk,
//! the paint layer records [`DisplayItem`]s into a [`DisplayList`].
//! The display list can then be:
//! - Cached between frames when layout hasn't changed
//! - Compacted (horizontal strip merging, zero-size removal)
//! - Optimized (vertical strip merging, occluded rect elimination)
//! - Replayed with batched rect submission ([`SdiBatch::submit_rect_batch`])
//! - Filtered by dirty rectangles (only replay items that intersect)
//! - Scrolled by adjusting offsets without rebuilding
//!
//! Nested clip rectangles are intersected during replay to minimize
//! redundant hardware clip changes. Consecutive `FillRect` items are
//! collected into batches so backends can submit them as a single GPU
//! draw call.

use oasis_types::backend::{
    BatchRect, BatchText, BlendMode as BackendBlendMode, Color, GradientStyle, RenderTargetId,
    SdiBackend, TextureId,
};
use oasis_types::error::Result;

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
    /// Saved `(compositor_dx, compositor_dy)` from before this layer
    /// was pushed. Restored on `PopCompositingLayer` so that nested
    /// layers don't accumulate translation incorrectly. (Without
    /// save/restore, an inner layer's screen-space origin would be
    /// subtracted on top of the outer layer's translation, leaving
    /// items off-target after the inner pop.)
    saved_compositor_dx: i32,
    saved_compositor_dy: i32,
}

// ---------------------------------------------------------------------------
// Display items
// ---------------------------------------------------------------------------

/// A single recorded draw operation.
#[derive(Debug, Clone)]
pub enum DisplayItem {
    /// Solid filled rectangle.
    FillRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        /// Source DOM node for hover color patching.
        node_id: Option<usize>,
    },
    /// Filled rectangle with rounded corners.
    FillRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
        /// Source DOM node for hover color patching.
        node_id: Option<usize>,
    },
    /// Stroked rectangle outline.
    StrokeRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    },
    /// Text rendering.
    DrawText {
        text: String,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
        /// Pre-computed width in pixels (includes letter-spacing).
        /// Used for dirty-rect culling to avoid expensive re-measurement.
        width: u32,
        /// Source DOM node for hover color patching.
        node_id: Option<usize>,
    },
    /// Texture blit.
    Blit {
        texture: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// Sub-rectangle blit from a texture atlas.
    BlitSub {
        texture: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    },
    /// Gradient fill.
    Gradient {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: GradientStyle,
    },
    /// Border edge (solid, dashed, dotted, double).
    BorderEdge {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        style: BorderStyle,
        horizontal: bool,
    },
    /// Box shadow (outer).
    Shadow {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        blur: f32,
        spread: f32,
        offset_x: f32,
        offset_y: f32,
        color: Color,
        radius: f32,
    },
    /// Push a clip rectangle (intersects with parent clip).
    PushClip { x: i32, y: i32, w: u32, h: u32 },
    /// Pop the most recent clip rectangle.
    PopClip,
    /// Begin a compositing layer. All subsequent items until `PopLayer`
    /// should ideally be rendered to an offscreen buffer and composited
    /// with the given opacity. Backends without render target support
    /// fall back to per-item opacity application (current behavior).
    PushLayer { opacity: f32 },
    /// End a compositing layer and composite it back.
    PopLayer,
    /// Begin a *real* compositing layer backed by an offscreen render
    /// target. All items between this and the matching
    /// `PopCompositingLayer` are drawn into an offscreen surface, then
    /// composited back into the parent with the supplied blend mode,
    /// opacity, filter chain, and (when `needs_backdrop`) backdrop
    /// filter chain.
    ///
    /// This is the slow path used for `mix-blend-mode`, `backdrop-filter`,
    /// `isolation: isolate`, box-level `filter`, and (once wired)
    /// `mask-*`. Backends that haven't opted into `SdiRenderTarget` fall
    /// back to a no-op: items are drawn without the effect.
    ///
    /// The simple opacity-only case stays on the `PushLayer` /
    /// `PopLayer` fast path so backends without render-target support
    /// pay zero extra cost.
    PushCompositingLayer {
        /// Pixel rect on the parent surface where the layer will be
        /// composited back. Used both as the render target size and
        /// as the destination rect for the composite.
        bounds: Rect,
        /// Cumulative opacity to multiply the layer by on composite.
        opacity: f32,
        /// Blend mode to use when compositing the layer back.
        blend: CssBlendMode,
        /// Whether the layer needs to sample the parent surface before
        /// drawing (set by `backdrop-filter` and any non-Normal blend
        /// mode that isn't simple alpha).
        needs_backdrop: bool,
        /// Filter chain applied to the layer pixels before composite.
        filters: Vec<FilterFunction>,
        /// Filter chain applied to the sampled backdrop pixels before
        /// the layer contents are drawn on top. Only consulted when
        /// `needs_backdrop` is true.
        backdrop_filters: Vec<FilterFunction>,
    },
    /// End a [`PushCompositingLayer`](Self::PushCompositingLayer).
    PopCompositingLayer,
    /// Hint that subsequent items in this layer should be blurred.
    /// Software fallback applies per-color approximation (desaturation +
    /// dimming) via [`super::filters::apply_filters`].
    /// GPU backends can override with actual Gaussian blur via render targets.
    BlurHint { radius: f32 },
    /// Begin a sticky-positioned element group.
    ///
    /// During scroll-delta replay, the sticky offset is recomputed from
    /// these parameters so the display list can be reused without rebuild.
    PushSticky {
        natural_y: f32,
        box_height: f32,
        top_px: Option<f32>,
        bottom_px: Option<f32>,
        visible_viewport_h: f32,
    },
    /// End a sticky-positioned element group.
    PopSticky,
}

impl DisplayItem {
    /// Bounding rectangle of this display item in screen coordinates.
    ///
    /// Used for dirty-rect culling during replay.
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            DisplayItem::FillRect { x, y, w, h, .. }
            | DisplayItem::FillRoundedRect { x, y, w, h, .. }
            | DisplayItem::Blit { x, y, w, h, .. }
            | DisplayItem::Gradient { x, y, w, h, .. }
            | DisplayItem::BorderEdge { x, y, w, h, .. }
            | DisplayItem::PushClip { x, y, w, h } => Some(Rect {
                x: *x as f32,
                y: *y as f32,
                width: *w as f32,
                height: *h as f32,
            }),
            DisplayItem::Shadow {
                x,
                y,
                w,
                h,
                blur,
                spread,
                offset_x,
                offset_y,
                ..
            } => {
                let expand = *spread + *blur;
                Some(Rect {
                    x: *x as f32 + *offset_x - expand,
                    y: *y as f32 + *offset_y - expand,
                    width: *w as f32 + expand * 2.0,
                    height: *h as f32 + expand * 2.0,
                })
            },
            DisplayItem::BlitSub {
                dst_x: x,
                dst_y: y,
                dst_w: w,
                dst_h: h,
                ..
            } => Some(Rect {
                x: *x as f32,
                y: *y as f32,
                width: *w as f32,
                height: *h as f32,
            }),
            DisplayItem::StrokeRoundedRect { x, y, w, h, .. } => Some(Rect {
                x: *x as f32,
                y: *y as f32,
                width: *w as f32,
                height: *h as f32,
            }),
            DisplayItem::DrawText {
                x,
                y,
                font_size,
                width,
                ..
            } => {
                let text_w = *width;
                let text_h = (*font_size as f32 * 1.2) as u32;
                Some(Rect {
                    x: *x as f32,
                    y: *y as f32,
                    width: text_w as f32,
                    height: text_h as f32,
                })
            },
            DisplayItem::PushCompositingLayer { bounds, .. } => Some(*bounds),
            DisplayItem::PopClip
            | DisplayItem::PushLayer { .. }
            | DisplayItem::PopLayer
            | DisplayItem::PopCompositingLayer
            | DisplayItem::BlurHint { .. }
            | DisplayItem::PushSticky { .. }
            | DisplayItem::PopSticky => None,
        }
    }

    /// Whether this item opens a compositing layer boundary that
    /// [`DisplayList::compact`] / [`DisplayList::optimize`] must NOT
    /// merge or eliminate rects across.
    pub fn is_layer_boundary(&self) -> bool {
        matches!(
            self,
            DisplayItem::PushCompositingLayer { .. } | DisplayItem::PopCompositingLayer
        )
    }
}

// ---------------------------------------------------------------------------
// Display list
// ---------------------------------------------------------------------------

/// A recorded sequence of draw operations.
///
/// Built by the paint pass, then replayed against an [`SdiBackend`].
/// Can be cached between frames when layout hasn't changed, and
/// adjusted for scroll offset during replay without rebuilding.
#[derive(Debug, Clone)]
pub struct DisplayList {
    /// The recorded items in paint order.
    items: Vec<DisplayItem>,
    /// Generation counter — incremented on each rebuild so caches
    /// can detect staleness.
    generation: u64,
    /// Whether the display list contains items from sticky-positioned
    /// elements. When true, scroll-only replay with delta offsets is
    /// not safe because sticky elements change their visual position
    /// relative to scroll independently of normal content flow.
    has_sticky: bool,
    /// The scroll Y value at the time this display list was recorded.
    /// Used by `PushSticky` to recompute sticky offsets during replay.
    recording_scroll_y: f32,
}

impl DisplayList {
    /// Create a new empty display list.
    pub fn new() -> Self {
        Self {
            items: Vec::with_capacity(256),
            generation: 0,
            has_sticky: false,
            recording_scroll_y: 0.0,
        }
    }

    /// Clear all items and increment the generation counter.
    pub fn clear(&mut self) {
        self.items.clear();
        self.generation += 1;
        self.has_sticky = false;
        self.recording_scroll_y = 0.0;
    }

    /// Set the scroll Y value at recording time.
    pub fn set_recording_scroll_y(&mut self, scroll_y: f32) {
        self.recording_scroll_y = scroll_y;
    }

    /// Whether the display list contains sticky-positioned elements.
    pub fn has_sticky(&self) -> bool {
        self.has_sticky
    }

    /// Mark the display list as containing sticky-positioned elements.
    pub fn set_has_sticky(&mut self) {
        self.has_sticky = true;
    }

    /// Push a display item onto the list.
    #[inline]
    pub fn push(&mut self, item: DisplayItem) {
        self.items.push(item);
    }

    /// Number of recorded items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the display list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Access the items slice.
    pub fn items(&self) -> &[DisplayItem] {
        &self.items
    }

    /// Patch colors for all display items tagged with `target_node_id`.
    ///
    /// `bg` replaces the color of `FillRect`/`FillRoundedRect` items
    /// tagged with the node. `fg` replaces the color of `DrawText`
    /// items tagged with the node. Returns the number of items patched.
    pub fn patch_node_colors(&mut self, target_node_id: usize, bg: Color, fg: Color) -> usize {
        let mut count = 0;
        for item in &mut self.items {
            match item {
                DisplayItem::FillRect { color, node_id, .. }
                | DisplayItem::FillRoundedRect { color, node_id, .. } => {
                    if *node_id == Some(target_node_id) {
                        *color = bg;
                        count += 1;
                    }
                },
                DisplayItem::DrawText { color, node_id, .. } => {
                    if *node_id == Some(target_node_id) {
                        *color = fg;
                        count += 1;
                    }
                },
                _ => {},
            }
        }
        count
    }

    /// Compact the display list by removing degenerate items and merging
    /// consecutive `FillRect` items that share the same color and form a
    /// horizontal strip (same y, same height, abutting edges).
    ///
    /// This reduces draw call count without changing visual output or
    /// violating paint order — only truly consecutive same-type items
    /// are merged.
    pub fn compact(&mut self) {
        // Pass 1: remove zero-size items (no visual contribution).
        self.items.retain(|item| match item {
            DisplayItem::FillRect { w, h, .. }
            | DisplayItem::FillRoundedRect { w, h, .. }
            | DisplayItem::StrokeRoundedRect { w, h, .. }
            | DisplayItem::Blit { w, h, .. }
            | DisplayItem::Gradient { w, h, .. }
            | DisplayItem::BorderEdge { w, h, .. } => *w > 0 && *h > 0,
            // PushClip must never be removed — its PopClip is always
            // retained, and removing one without the other corrupts the
            // clip stack.  Zero-size clips are harmless (they just clip
            // everything inside to nothing).
            DisplayItem::PushClip { .. } => true,
            DisplayItem::BlitSub { dst_w, dst_h, .. } => *dst_w > 0 && *dst_h > 0,
            // Shadows are always retained: a 0x0 source with large spread/blur
            // still produces visible pixels.
            DisplayItem::Shadow { .. } => true,
            DisplayItem::DrawText { text, width, .. } => !text.is_empty() && *width > 0,
            // PopClip, PushLayer, PopLayer, BlurHint — always keep.
            _ => true,
        });

        // Pass 2: merge consecutive FillRect items with the same color and
        // height that form a horizontal strip (same y, abutting x + w == next x).
        if self.items.len() < 2 {
            return;
        }
        let mut merged: Vec<DisplayItem> = Vec::with_capacity(self.items.len());
        let mut drain = self.items.drain(..);
        let mut current = drain.next().expect("len >= 2");

        // Track compositing-layer depth: rects inside a layer target a
        // different surface from rects outside, so they must never
        // merge with each other. `current` sits at `current_layer`
        // depth; `next` sits at `next_layer` — if they differ the rects
        // are on different surfaces and merging would be incorrect.
        let mut current_layer: usize =
            matches!(&current, DisplayItem::PushCompositingLayer { .. }) as usize;
        for next in drain {
            let next_layer = match &next {
                DisplayItem::PushCompositingLayer { .. } => current_layer + 1,
                DisplayItem::PopCompositingLayer => current_layer.saturating_sub(1),
                _ => current_layer,
            };

            if current_layer == next_layer
                && let (
                    DisplayItem::FillRect {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                        color: cc,
                        ..
                    },
                    DisplayItem::FillRect {
                        x: nx,
                        y: ny,
                        w: nw,
                        h: nh,
                        color: nc,
                        ..
                    },
                ) = (&current, &next)
            {
                // Same color, same y, same height, horizontally abutting?
                // Note: node_id is intentionally NOT compared here so that
                // adjacent rects from different DOM nodes still merge.
                // This keeps the display list compact, which is critical on
                // PSP where the GU command buffer is limited.
                if cc == nc && cy == ny && ch == nh && cx + *cw as i32 == *nx {
                    current = DisplayItem::FillRect {
                        x: *cx,
                        y: *cy,
                        w: cw + nw,
                        h: *ch,
                        color: *cc,
                        // Merged rects lose their node association since they
                        // span multiple nodes. patch_node_colors will skip them.
                        node_id: None,
                    };
                    continue;
                }
            }
            merged.push(current);
            current = next;
            current_layer = next_layer;
        }
        merged.push(current);
        self.items = merged;
    }

    /// Optimize the display list by merging and culling items.
    ///
    /// Call after [`compact()`](Self::compact) for additional optimizations:
    /// - Merge consecutive vertically abutting `FillRect` items (same x,
    ///   width, and color)
    /// - Eliminate opaque `FillRect` items fully occluded by a later opaque
    ///   `FillRect` within the same clip context
    ///
    /// These reduce draw call count and command buffer usage on all backends,
    /// which is critical on PSP where the GU command buffer is 1 MB.
    pub fn optimize(&mut self) {
        self.merge_vertical_strips();
        self.eliminate_occluded();
    }

    /// Merge consecutive `FillRect` items that form a vertical strip
    /// (same x, same width, same color, abutting y + h == next y).
    fn merge_vertical_strips(&mut self) {
        if self.items.len() < 2 {
            return;
        }
        let mut merged: Vec<DisplayItem> = Vec::with_capacity(self.items.len());
        let mut drain = self.items.drain(..);
        let mut current = drain.next().expect("len >= 2");

        let mut current_layer: usize =
            matches!(&current, DisplayItem::PushCompositingLayer { .. }) as usize;
        for next in drain {
            let next_layer = match &next {
                DisplayItem::PushCompositingLayer { .. } => current_layer + 1,
                DisplayItem::PopCompositingLayer => current_layer.saturating_sub(1),
                _ => current_layer,
            };
            if current_layer == next_layer
                && let (
                    DisplayItem::FillRect {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                        color: cc,
                        ..
                    },
                    DisplayItem::FillRect {
                        x: nx,
                        y: ny,
                        w: nw,
                        h: nh,
                        color: nc,
                        ..
                    },
                ) = (&current, &next)
            {
                // Same color, same x, same width, vertically abutting?
                if cc == nc && cx == nx && cw == nw && cy + *ch as i32 == *ny {
                    current = DisplayItem::FillRect {
                        x: *cx,
                        y: *cy,
                        w: *cw,
                        h: ch + nh,
                        color: *cc,
                        node_id: None,
                    };
                    continue;
                }
            }
            merged.push(current);
            current = next;
            current_layer = next_layer;
        }
        merged.push(current);
        self.items = merged;
    }

    /// Remove opaque `FillRect` items fully covered by a later opaque
    /// `FillRect` within the same clip level.
    ///
    /// Uses a backward scan with a small window (32 items) to keep the
    /// algorithm O(n × k) rather than O(n²). Only eliminates items in the
    /// same clip depth to preserve correctness.
    fn eliminate_occluded(&mut self) {
        if self.items.len() < 2 {
            return;
        }

        const SCAN_WINDOW: usize = 32;
        let mut clip_depth: usize = 0;
        let mut sticky_depth: usize = 0;
        let mut compositing_depth: usize = 0;
        let mut clip_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut sticky_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut compositing_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut in_translucent: Vec<bool> = Vec::with_capacity(self.items.len());

        // First pass: compute clip/sticky depth and translucent-layer flag.
        let mut translucent_layer_depth: usize = 0;
        for item in &self.items {
            match item {
                DisplayItem::PushClip { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                    clip_depth += 1;
                },
                DisplayItem::PopClip => {
                    clip_depth = clip_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushLayer { opacity } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    if *opacity < 1.0 {
                        translucent_layer_depth += 1;
                    }
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PopLayer => {
                    translucent_layer_depth = translucent_layer_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushCompositingLayer { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    // Contents of a compositing layer draw into an
                    // offscreen surface and may be re-blended with a
                    // non-Normal blend mode or filter — treat them as
                    // translucent so they can neither be eliminated
                    // nor act as occluders.
                    compositing_depth += 1;
                    translucent_layer_depth += 1;
                    in_translucent.push(true);
                },
                DisplayItem::PopCompositingLayer => {
                    compositing_depth = compositing_depth.saturating_sub(1);
                    translucent_layer_depth = translucent_layer_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushSticky { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                    sticky_depth += 1;
                },
                DisplayItem::PopSticky => {
                    sticky_depth = sticky_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                _ => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
            }
        }

        // Second pass: mark items fully occluded by later opaque rects.
        let mut remove = vec![false; self.items.len()];

        for i in 1..self.items.len() {
            // Only opaque FillRect items outside translucent layers can occlude.
            if in_translucent[i] {
                continue;
            }
            let (cx, cy, cw, ch) = match &self.items[i] {
                DisplayItem::FillRect {
                    x, y, w, h, color, ..
                } if color.a == 255 => (*x, *y, *w, *h),
                _ => continue,
            };

            let my_clip = clip_depths[i];
            let my_sticky = sticky_depths[i];
            let my_compositing = compositing_depths[i];
            let start = i.saturating_sub(SCAN_WINDOW);

            for j in start..i {
                if remove[j]
                    || clip_depths[j] != my_clip
                    || sticky_depths[j] != my_sticky
                    || compositing_depths[j] != my_compositing
                {
                    continue;
                }

                let (ox, oy, ow, oh) = match &self.items[j] {
                    DisplayItem::FillRect { x, y, w, h, .. } => (*x, *y, *w, *h),
                    _ => continue,
                };

                // Is the earlier rect fully contained within the covering rect?
                if ox >= cx
                    && oy >= cy
                    && ox + ow as i32 <= cx + cw as i32
                    && oy + oh as i32 <= cy + ch as i32
                {
                    remove[j] = true;
                }
            }
        }

        // Third pass: remove marked items.
        let mut idx = 0;
        self.items.retain(|_| {
            let keep = !remove[idx];
            idx += 1;
            keep
        });
    }

    /// Replay all display items against the backend.
    ///
    /// `base_clip` is the outer clip rectangle (browser window content area)
    /// that should be restored when `PopClip` empties the clip stack. This
    /// prevents content from rendering outside the browser window when the
    /// scroll buffer zone extends the recorded area beyond the viewport.
    ///
    /// Consecutive `FillRect` items at the same layer opacity are collected
    /// into a batch and submitted via [`SdiBatch::submit_rect_batch`],
    /// allowing backends to optimize them into a single draw call.
    /// Nested clip rectangles are intersected to avoid redundant hardware
    /// clip changes.
    pub fn replay(
        &self,
        backend: &mut dyn SdiBackend,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
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
                            // Filter pass: if the layer has any
                            // filters AND the backend supports pixel
                            // readback, read the target, apply the
                            // filter chain on CPU, upload as a
                            // temporary texture, and blit it at the
                            // destination rect. The blit path drops
                            // the CSS blend mode (becomes Normal) but
                            // keeps opacity — documented degradation
                            // until a read-modify-write path lands on
                            // the render target itself.
                            let composite_res = if unbind_res.is_ok() {
                                let run_filter = !layer.filters.is_empty()
                                    && backend.supports_render_target_readback();
                                if run_filter {
                                    let byte_count = (layer.dst_w * layer.dst_h * 4) as usize;
                                    let mut buf = vec![0u8; byte_count];
                                    if backend.read_render_target(id, &mut buf).is_ok() {
                                        crate::paint::filter_chain::apply_filter_chain(
                                            &mut buf,
                                            layer.dst_w,
                                            layer.dst_h,
                                            &layer.filters,
                                        );
                                        // Pre-multiply opacity into
                                        // alpha so a plain alpha-over
                                        // blit gives the correct result.
                                        if (layer.opacity - 1.0).abs() > f32::EPSILON {
                                            let f = layer.opacity.clamp(0.0, 1.0);
                                            for chunk in buf.chunks_exact_mut(4) {
                                                chunk[3] = ((chunk[3] as f32) * f).round() as u8;
                                            }
                                        }
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
                    ..
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
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

    /// Replay only items that intersect the given dirty rectangle.
    ///
    /// `dirty` is in screen coordinates. Items fully outside `dirty`
    /// are skipped. Clip push/pop and layer push/pop items are always
    /// replayed to maintain correct compositing and clip state.
    /// Consecutive `FillRect` items are batched like in [`replay`](Self::replay).
    pub fn replay_dirty(
        &self,
        backend: &mut dyn SdiBackend,
        dirty: &Rect,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
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
                    ..
                } => {
                    flush_rect_batch(backend, &mut rect_batch)?;
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

impl Default for DisplayList {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flush accumulated `FillRect` items as a single batched call.
///
/// When the batch contains multiple rects, they are submitted via
/// [`SdiBatch::submit_rect_batch`] so backends can optimize them into
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
/// are submitted via [`SdiBatch::submit_text_batch`] so backends can
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{DrawCall, MockBackend};

    #[test]
    fn display_list_new_is_empty() {
        let dl = DisplayList::new();
        assert!(dl.is_empty());
        assert_eq!(dl.len(), 0);
        assert_eq!(dl.generation(), 0);
    }

    #[test]
    fn display_list_push_and_clear() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        assert_eq!(dl.len(), 1);
        assert_eq!(dl.generation(), 0);

        dl.clear();
        assert!(dl.is_empty());
        assert_eq!(dl.generation(), 1);
    }

    #[test]
    fn display_list_replay_applies_scroll_offset() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 20,
            w: 100,
            h: 50,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 5, -10, None).unwrap();

        assert_eq!(backend.fill_rect_count(), 1);
        // The fill_rect should be at (10+5, 20-10) = (15, 10).
        let call = &backend.calls[0];
        if let crate::test_utils::DrawCall::FillRect { x, y, .. } = call {
            assert_eq!(*x, 15);
            assert_eq!(*y, 10);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn bounds_returns_none_for_pop_clip() {
        let item = DisplayItem::PopClip;
        assert!(item.bounds().is_none());
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
    fn compact_removes_zero_size_items() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 0,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 0,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        assert_eq!(dl.len(), 3);
        dl.compact();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { color, .. } = &dl.items()[0] {
            assert_eq!(*color, Color::rgb(0, 255, 0));
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn compact_merges_horizontal_fill_rects() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        // Three abutting rects: (0,0,10,5), (10,0,10,5), (20,0,10,5)
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 20,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { x, y, w, h, .. } = &dl.items()[0] {
            assert_eq!(*x, 0);
            assert_eq!(*y, 0);
            assert_eq!(*w, 30);
            assert_eq!(*h, 5);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn compact_does_not_merge_different_colors() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn compact_does_not_merge_non_abutting() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn compact_preserves_non_fill_rect_items() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushClip {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        });
        dl.push(DisplayItem::DrawText {
            text: "hello".into(),
            x: 5,
            y: 5,
            font_size: 12,
            color: Color::rgb(0, 0, 0),
            bold: false,
            italic: false,
            width: 1,
            node_id: None,
        });
        dl.push(DisplayItem::PopClip);
        dl.compact();
        assert_eq!(dl.len(), 3);
    }

    #[test]
    fn replay_dirty_culls_offscreen_items() {
        let mut dl = DisplayList::new();
        // Item inside dirty rect.
        dl.push(DisplayItem::FillRect {
            x: 50,
            y: 50,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        // Item outside dirty rect.
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

        // Only the first item should have been drawn.
        assert_eq!(backend.fill_rect_count(), 1);
    }

    #[test]
    fn push_pop_layer_bounds_is_none() {
        assert!(DisplayItem::PushLayer { opacity: 0.5 }.bounds().is_none());
        assert!(DisplayItem::PopLayer.bounds().is_none());
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
            // 200 * 0.5 = 100
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
        });
        dl.push(DisplayItem::PopLayer);

        let mut backend = MockBackend::new();
        dl.replay(&mut backend, 0, 0, None).unwrap();

        assert_eq!(backend.draw_text_count(), 1);
        if let DrawCall::DrawText { color, .. } = &backend.calls[0] {
            // 254 * 0.5 = 127
            assert_eq!(color.a, 127);
        } else {
            panic!("expected DrawText");
        }
    }

    #[test]
    fn compact_preserves_push_pop_layer() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::PopLayer);
        dl.compact();
        assert_eq!(dl.len(), 3);
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
            // 100 * 0.5 = 50
            assert_eq!(color.a, 50);
        } else {
            panic!("expected FillRect");
        }
    }

    // -----------------------------------------------------------------------
    // Optimizer tests
    // -----------------------------------------------------------------------

    #[test]
    fn optimize_merges_vertical_strips() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        // Three vertically abutting rects at x=5, w=10.
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 5,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 10,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { x, y, w, h, .. } = &dl.items()[0] {
            assert_eq!(*x, 5);
            assert_eq!(*y, 0);
            assert_eq!(*w, 10);
            assert_eq!(*h, 15);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn optimize_no_vertical_merge_different_width() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 5,
            w: 20,
            h: 5,
            color,
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn optimize_eliminates_occluded_rect() {
        let mut dl = DisplayList::new();
        // Small rect fully inside the big one.
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        // Opaque rect covering the small one.
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { color, .. } = &dl.items()[0] {
            // Only the covering green rect should remain.
            assert_eq!(*color, Color::rgb(0, 255, 0));
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn optimize_does_not_eliminate_semi_transparent() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        // Semi-transparent covering rect — should NOT eliminate the first.
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgba(0, 255, 0, 128),
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn optimize_respects_clip_depth() {
        let mut dl = DisplayList::new();
        // Rect outside clip.
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        // Enter clip context.
        dl.push(DisplayItem::PushClip {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        });
        // Opaque rect inside clip — should NOT eliminate the one outside clip.
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.push(DisplayItem::PopClip);
        dl.optimize();
        // All items should survive because they're at different clip depths.
        assert_eq!(dl.len(), 4);
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
    fn replay_batches_consecutive_fill_rects() {
        let mut dl = DisplayList::new();
        // Three consecutive FillRects.
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

        // All three rects should be drawn (via batch submission which
        // falls back to individual fill_rect calls in MockBackend).
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
        // Verify ordering: first rect, then text, then second rect.
        assert!(matches!(&backend.calls[0], DrawCall::FillRect { .. }));
        assert!(matches!(&backend.calls[1], DrawCall::DrawText { .. }));
        assert!(matches!(&backend.calls[2], DrawCall::FillRect { .. }));
    }

    // -----------------------------------------------------------------
    // Compositing layer boundary tests (PR3 of compositor overhaul)
    // -----------------------------------------------------------------

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
        }
    }

    #[test]
    fn compact_does_not_merge_fillrects_across_compositing_layer() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(10, 20, 30);
        // Rect inside a layer.
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        // Abutting rect outside the layer (same color, same y, same h,
        // would merge if not for the layer boundary).
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });

        dl.compact();
        // If compact() ignored the boundary we'd see a single merged
        // 20-wide FillRect (len 3). The layer boundary must keep them
        // separate.
        let fill_count = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(
            fill_count,
            2,
            "rects must not merge across layers: {:#?}",
            dl.items()
        );
    }

    #[test]
    fn merge_vertical_strips_does_not_cross_compositing_layer() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(10, 20, 30);
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        // Vertically abutting outside the layer.
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 10,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.optimize();
        let fill_count = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(fill_count, 2);
    }

    #[test]
    fn eliminate_occluded_does_not_cross_compositing_layer() {
        let mut dl = DisplayList::new();
        // Opaque rect inside a layer.
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
            color: Color::rgba(255, 0, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        // A later opaque rect outside the layer that fully covers the
        // inner one would normally cause elimination. It must not cross
        // the layer boundary, because the inner rect draws to a
        // different surface.
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgba(0, 255, 0, 255),
            node_id: None,
        });
        let before = dl.len();
        dl.optimize();
        // Both inner and outer rect survive.
        let after_fills = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(after_fills, 2, "both fills must survive: {:#?}", dl.items());
        assert!(dl.len() >= before.saturating_sub(1));
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
        // One fill emitted with opacity halved.
        assert_eq!(backend.fill_rect_count(), 1);
        match &backend.calls[0] {
            DrawCall::FillRect { color, .. } => {
                // 200 * 0.5 = 100
                assert_eq!(color.a, 100);
            },
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn is_layer_boundary_flags_new_variants() {
        assert!(push_cl(layer_rect(0.0, 0.0, 10.0, 10.0), 1.0).is_layer_boundary());
        assert!(DisplayItem::PopCompositingLayer.is_layer_boundary());
        assert!(!DisplayItem::PushLayer { opacity: 0.5 }.is_layer_boundary());
        assert!(!DisplayItem::PopLayer.is_layer_boundary());
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
        // Find the compositor commands in the recorded stream.
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

        // The composite command references the layer bounds + blend +
        // opacity we supplied.
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

        // An inner FillRect must appear between bind and unbind — it
        // draws into the target, not the framebuffer.
        let inner_fill = cmds[bind_idx + 1..unbind_idx]
            .iter()
            .any(|c| matches!(c, DrawCommand::FillRect { .. }));
        assert!(inner_fill, "inner fill recorded outside the layer");
    }

    #[test]
    fn compositing_layer_translates_contents_to_local_space() {
        use oasis_test_backend::RecordingBackend;
        use oasis_types::backend::DrawCommand;

        // Layer bounds at (50, 100) with size 30x20; inner rect at
        // screen-space (60, 110, 10, 10). After translation it should
        // land at target-local (10, 10, 10, 10).
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
        // Outer layer.
        dl.push(push_cl(layer_rect(10.0, 20.0, 200.0, 200.0), 1.0));
        // Outer-only content (between outer push and inner push).
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 25,
            w: 1,
            h: 1,
            color: Color::rgba(255, 0, 0, 255),
            node_id: None,
        });
        // Inner layer.
        dl.push(push_cl(layer_rect(50.0, 60.0, 100.0, 100.0), 1.0));
        // Inner-only content.
        dl.push(DisplayItem::FillRect {
            x: 55,
            y: 65,
            w: 2,
            h: 2,
            color: Color::rgba(0, 255, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        // Outer-only content (after inner pop).
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

        // Walk the trace and find the three FillRects in order.
        let mut fills = cmds.iter().filter_map(|c| match c {
            DrawCommand::FillRect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        });
        // Outer-content-before-inner: screen (15, 25) → outer-local
        // (5, 5).
        assert_eq!(fills.next(), Some((5, 5, 1, 1)), "outer-before");
        // Inner content: screen (55, 65) → inner-local (5, 5).
        assert_eq!(fills.next(), Some((5, 5, 2, 2)), "inner");
        // Outer-content-after-inner: screen (16, 26) must again
        // resolve to outer-local (6, 6) — would be wrong if the pop
        // path naively added inner.dst_x back to compositor_dx.
        assert_eq!(fills.next(), Some((6, 6, 3, 3)), "outer-after");
    }
}
