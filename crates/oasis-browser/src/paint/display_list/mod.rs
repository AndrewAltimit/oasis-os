//! Display list intermediate representation (light compositor).
//!
//! Instead of issuing draw calls directly during the layout tree walk,
//! the paint layer records [`DisplayItem`]s into a [`DisplayList`].
//! The display list can then be:
//! - Cached between frames when layout hasn't changed
//! - Compacted (horizontal strip merging, zero-size removal)
//! - Optimized (vertical strip merging, occluded rect elimination)
//! - Replayed with batched rect submission (`SdiBatch::submit_rect_batch`)
//! - Filtered by dirty rectangles (only replay items that intersect)
//! - Scrolled by adjusting offsets without rebuilding
//!
//! Nested clip rectangles are intersected during replay to minimize
//! redundant hardware clip changes. Consecutive `FillRect` items are
//! collected into batches so backends can submit them as a single GPU
//! draw call.
//!
//! Internal layout:
//!
//! - [`mask`] — `MaskParams` and the mask rasterizers / `apply_mask`
//!   pass invoked by the compositor pop.
//! - [`optimize`] — `compact()` / `optimize()` passes (run after
//!   recording, before replay).
//! - [`replay`] — `replay()` / `replay_dirty()` walkers and their
//!   per-item helpers (clip intersection, opacity stack, sticky offset
//!   recompute, border-edge styling).

mod mask;
mod optimize;
mod replay;

use oasis_types::backend::{Color, GradientStyle, SdiBackend, TextureId};
use oasis_types::error::Result;

use crate::css::values::BorderStyle;
use crate::css::values::types::{BlendMode as CssBlendMode, FilterFunction};
use crate::layout::box_model::Rect;

pub use mask::MaskParams;

/// Trait for rendering web font text during display list replay.
///
/// Implementors rasterize individual glyphs from a font registry and
/// blit them to the backend. The display list calls this instead of
/// `draw_text_styled` when a `DrawText` item carries a `web_font_id`.
#[cfg(feature = "web-fonts")]
#[allow(clippy::too_many_arguments)]
pub trait WebFontRenderer {
    /// Render a text string using the web font identified by `font_id`.
    fn render(
        &mut self,
        backend: &mut dyn SdiBackend,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        font_id: u32,
    ) -> Result<()>;
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
    /// Filled quadrilateral defined by four corners (screen-space).
    ///
    /// Used for 3D-transformed backgrounds where perspective projection
    /// produces a true trapezoid instead of an axis-aligned rectangle.
    /// The four points are in clockwise winding order: top-left,
    /// top-right, bottom-right, bottom-left.
    FillPolygon {
        points: [(i32, i32); 4],
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
        /// Web font ID from the [`FontRegistry`]. When `Some`, the text
        /// is rendered using the web font's rasterized glyphs instead of
        /// the backend's bitmap font.
        #[cfg(feature = "web-fonts")]
        web_font_id: Option<u32>,
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
        /// Optional `mask-*` parameters. When present, the replay path
        /// reads the layer pixels after filters are applied,
        /// rasterizes the mask source into an alpha buffer, and
        /// combines it with the layer alpha (per `mask-mode`:
        /// alpha / luminance / match-source) before the layer is
        /// composited back to the parent.
        ///
        /// `mask-composite` selects the per-pixel operation. Note
        /// that the single-layer pop path collapses `Add` and
        /// `Intersect` to destination-in — see the inline comment
        /// in `apply_mask` for the spec rationale. Multi-layer
        /// composition (where `Add` = source-over between mask
        /// layers) is a follow-up.
        mask: Option<MaskParams>,
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
            DisplayItem::FillPolygon { points, .. } => {
                let min_x = points.iter().map(|p| p.0).min().unwrap_or(0);
                let min_y = points.iter().map(|p| p.1).min().unwrap_or(0);
                let max_x = points.iter().map(|p| p.0).max().unwrap_or(0);
                let max_y = points.iter().map(|p| p.1).max().unwrap_or(0);
                Some(Rect {
                    x: min_x as f32,
                    y: min_y as f32,
                    width: (max_x - min_x) as f32,
                    height: (max_y - min_y) as f32,
                })
            },
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
                | DisplayItem::FillRoundedRect { color, node_id, .. }
                | DisplayItem::FillPolygon { color, node_id, .. }
                    if *node_id == Some(target_node_id) =>
                {
                    *color = bg;
                    count += 1;
                },
                DisplayItem::DrawText { color, node_id, .. }
                    if *node_id == Some(target_node_id) =>
                {
                    *color = fg;
                    count += 1;
                },
                _ => {},
            }
        }
        count
    }

    /// Replay all display items against the backend.
    ///
    /// `base_clip` is the outer clip rectangle (browser window content area)
    /// that should be restored when `PopClip` empties the clip stack. This
    /// prevents content from rendering outside the browser window when the
    /// scroll buffer zone extends the recorded area beyond the viewport.
    ///
    /// Consecutive `FillRect` items at the same layer opacity are collected
    /// into a batch and submitted via `SdiBatch::submit_rect_batch`,
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
        #[cfg(feature = "web-fonts")]
        return self.replay_inner(backend, scroll_dx, scroll_dy, base_clip, None);
        #[cfg(not(feature = "web-fonts"))]
        return self.replay_inner(backend, scroll_dx, scroll_dy, base_clip);
    }

    /// Replay with a web font renderer for glyph-level rendering.
    ///
    /// When `renderer` is provided, `DrawText` items carrying a
    /// `web_font_id` are routed through the renderer instead of the
    /// backend's bitmap `draw_text_styled`.
    #[cfg(feature = "web-fonts")]
    pub fn replay_with_fonts(
        &self,
        backend: &mut dyn SdiBackend,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
        renderer: &mut dyn WebFontRenderer,
    ) -> Result<()> {
        self.replay_inner(backend, scroll_dx, scroll_dy, base_clip, Some(renderer))
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
        #[cfg(feature = "web-fonts")]
        return self.replay_dirty_inner(backend, dirty, scroll_dx, scroll_dy, base_clip, None);
        #[cfg(not(feature = "web-fonts"))]
        return self.replay_dirty_inner(backend, dirty, scroll_dx, scroll_dy, base_clip);
    }

    /// Replay dirty items with a web font renderer for glyph-level rendering.
    #[cfg(feature = "web-fonts")]
    pub fn replay_dirty_with_fonts(
        &self,
        backend: &mut dyn SdiBackend,
        dirty: &Rect,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
        renderer: &mut dyn WebFontRenderer,
    ) -> Result<()> {
        self.replay_dirty_inner(
            backend,
            dirty,
            scroll_dx,
            scroll_dy,
            base_clip,
            Some(renderer),
        )
    }
}

impl Default for DisplayList {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use oasis_types::backend::Color;

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
        let call = &backend.calls[0];
        if let DrawCall::FillRect { x, y, .. } = call {
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
    fn push_pop_layer_bounds_is_none() {
        assert!(DisplayItem::PushLayer { opacity: 0.5 }.bounds().is_none());
        assert!(DisplayItem::PopLayer.bounds().is_none());
    }

    #[test]
    fn is_layer_boundary_flags_new_variants() {
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let push_cl = DisplayItem::PushCompositingLayer {
            bounds,
            opacity: 1.0,
            blend: CssBlendMode::Multiply,
            needs_backdrop: false,
            filters: Vec::new(),
            backdrop_filters: Vec::new(),
            mask: None,
        };
        assert!(push_cl.is_layer_boundary());
        assert!(DisplayItem::PopCompositingLayer.is_layer_boundary());
        assert!(!DisplayItem::PushLayer { opacity: 0.5 }.is_layer_boundary());
        assert!(!DisplayItem::PopLayer.is_layer_boundary());
    }
}
