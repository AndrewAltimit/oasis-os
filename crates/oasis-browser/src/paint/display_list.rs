//! Display list intermediate representation.
//!
//! Instead of issuing draw calls directly during the layout tree walk,
//! the paint layer records [`DisplayItem`]s into a [`DisplayList`].
//! The display list can then be:
//! - Cached between frames when layout hasn't changed
//! - Replayed with batching optimizations (group by draw type)
//! - Filtered by dirty rectangles (only replay items that intersect)
//! - Scrolled by adjusting offsets without rebuilding

// Items and methods are defined for future phases (dirty-rect culling,
// batching, tile caching). Suppress dead_code warnings until they're wired in.

use oasis_types::backend::{Color, GradientStyle, SdiBackend, TextureId};
use oasis_types::error::Result;

use crate::css::values::BorderStyle;
use crate::layout::box_model::Rect;

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
    },
    /// Filled rectangle with rounded corners.
    FillRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
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
    /// Hint that subsequent items in this layer should be blurred.
    /// Software fallback applies per-color approximation (desaturation +
    /// dimming) via [`super::filters::apply_filters`].
    /// GPU backends can override with actual Gaussian blur via render targets.
    BlurHint { radius: f32 },
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
            | DisplayItem::Shadow { x, y, w, h, .. }
            | DisplayItem::PushClip { x, y, w, h } => Some(Rect {
                x: *x as f32,
                y: *y as f32,
                width: *w as f32,
                height: *h as f32,
            }),
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
                text,
                ..
            } => {
                let text_w = oasis_types::backend::bitmap_measure_text(text, *font_size);
                let text_h = (*font_size as f32 * 1.2) as u32;
                Some(Rect {
                    x: *x as f32,
                    y: *y as f32,
                    width: text_w as f32,
                    height: text_h as f32,
                })
            },
            DisplayItem::PopClip
            | DisplayItem::PushLayer { .. }
            | DisplayItem::PopLayer
            | DisplayItem::BlurHint { .. } => None,
        }
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
}

impl DisplayList {
    /// Create a new empty display list.
    pub fn new() -> Self {
        Self {
            items: Vec::with_capacity(256),
            generation: 0,
        }
    }

    /// Clear all items and increment the generation counter.
    pub fn clear(&mut self) {
        self.items.clear();
        self.generation += 1;
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
            | DisplayItem::BorderEdge { w, h, .. }
            => *w > 0 && *h > 0,
            // PushClip must never be removed — its PopClip is always
            // retained, and removing one without the other corrupts the
            // clip stack.  Zero-size clips are harmless (they just clip
            // everything inside to nothing).
            DisplayItem::PushClip { .. } => true,
            DisplayItem::BlitSub { dst_w, dst_h, .. } => *dst_w > 0 && *dst_h > 0,
            DisplayItem::Shadow { w, h, .. } => *w > 0 && *h > 0,
            // DrawText, PopClip, PushLayer, PopLayer — always keep.
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

        for next in drain {
            if let (
                DisplayItem::FillRect {
                    x: cx,
                    y: cy,
                    w: cw,
                    h: ch,
                    color: cc,
                },
                DisplayItem::FillRect {
                    x: nx,
                    y: ny,
                    w: nw,
                    h: nh,
                    color: nc,
                },
            ) = (&current, &next)
            {
                // Same color, same y, same height, and horizontally abutting?
                if cc == nc && cy == ny && ch == nh && cx + *cw as i32 == *nx {
                    current = DisplayItem::FillRect {
                        x: *cx,
                        y: *cy,
                        w: cw + nw,
                        h: *ch,
                        color: *cc,
                    };
                    continue;
                }
            }
            merged.push(current);
            current = next;
        }
        merged.push(current);
        self.items = merged;
    }

    /// Replay all display items against the backend.
    ///
    /// This is the main rendering path. The `scroll_dx` and `scroll_dy`
    /// offsets are applied to all items, enabling scroll without rebuild.
    ///
    /// `PushLayer`/`PopLayer` items maintain an opacity stack. Colors of
    /// draw items are multiplied by the cumulative layer opacity, providing
    /// correct compositing for the common single-layer case. True offscreen
    /// compositing for overlapping children within a layer requires render
    /// target support in the backend (future GPU override path).
    /// Replay all display items against the backend.
    ///
    /// `base_clip` is the outer clip rectangle (browser window content area)
    /// that should be restored when `PopClip` empties the clip stack. This
    /// prevents content from rendering outside the browser window when the
    /// scroll buffer zone extends the recorded area beyond the viewport.
    pub fn replay(
        &self,
        backend: &mut dyn SdiBackend,
        scroll_dx: i32,
        scroll_dy: i32,
        base_clip: Option<(i32, i32, u32, u32)>,
    ) -> Result<()> {
        backend.begin_batch()?;

        let mut opacity_stack: Vec<f32> = Vec::new();
        let mut clip_stack: Vec<(i32, i32, u32, u32)> = Vec::new();

        for item in &self.items {
            match item {
                DisplayItem::PushLayer { opacity } => {
                    opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopLayer => {
                    opacity_stack.pop();
                    continue;
                },
                _ => {},
            }

            let layer_opacity = layer_opacity_product(&opacity_stack);

            match item {
                DisplayItem::FillRect { x, y, w, h, color } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rect(x + scroll_dx, y + scroll_dy, *w, *h, c)?;
                },
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rounded_rect(x + scroll_dx, y + scroll_dy, *w, *h, *radius, c)?;
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
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        c,
                    )?;
                },
                DisplayItem::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    color,
                    bold,
                    italic,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.draw_text_styled(
                        text,
                        x + scroll_dx,
                        y + scroll_dy,
                        *font_size,
                        c,
                        *bold,
                        *italic,
                    )?;
                },
                DisplayItem::Blit {
                    texture,
                    x,
                    y,
                    w,
                    h,
                } => {
                    backend.blit(*texture, x + scroll_dx, y + scroll_dy, *w, *h)?;
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
                        dst_y + scroll_dy,
                        *dst_w,
                        *dst_h,
                    )?;
                },
                DisplayItem::Gradient { x, y, w, h, style } => {
                    backend.fill_rect_gradient(x + scroll_dx, y + scroll_dy, *w, *h, style)?;
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
                        y + scroll_dy,
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
                        y + scroll_dy,
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
                    clip_stack.push((x + scroll_dx, y + scroll_dy, *w, *h));
                    backend.set_clip_rect(x + scroll_dx, y + scroll_dy, *w, *h)?;
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
                DisplayItem::PushLayer { .. } | DisplayItem::PopLayer => {},
                // BlurHint is metadata for GPU backends; software fallback
                // applies per-color approximation during recording.
                DisplayItem::BlurHint { .. } => {},
            }
        }

        backend.flush_batch()?;
        Ok(())
    }

    /// Replay only items that intersect the given dirty rectangle.
    ///
    /// `dirty` is in screen coordinates. Items fully outside `dirty`
    /// are skipped. Clip push/pop and layer push/pop items are always
    /// replayed to maintain correct compositing and clip state.
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

        for item in &self.items {
            // Clip and layer items must always be replayed.
            match item {
                DisplayItem::PushClip { x, y, w, h } => {
                    clip_stack.push((x + scroll_dx, y + scroll_dy, *w, *h));
                    backend.set_clip_rect(x + scroll_dx, y + scroll_dy, *w, *h)?;
                    continue;
                },
                DisplayItem::PopClip => {
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
                    opacity_stack.push(*opacity);
                    continue;
                },
                DisplayItem::PopLayer => {
                    opacity_stack.pop();
                    continue;
                },
                _ => {},
            }

            // Cull items outside the dirty rect.
            if let Some(bounds) = item.bounds() {
                let shifted = Rect {
                    x: bounds.x + scroll_dx as f32,
                    y: bounds.y + scroll_dy as f32,
                    width: bounds.width,
                    height: bounds.height,
                };
                if !rects_intersect(&shifted, dirty) {
                    continue;
                }
            }

            let layer_opacity = layer_opacity_product(&opacity_stack);

            // Replay the item.
            match item {
                DisplayItem::FillRect { x, y, w, h, color } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rect(x + scroll_dx, y + scroll_dy, *w, *h, c)?;
                },
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.fill_rounded_rect(x + scroll_dx, y + scroll_dy, *w, *h, *radius, c)?;
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
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        c,
                    )?;
                },
                DisplayItem::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    color,
                    bold,
                    italic,
                } => {
                    let c = apply_layer_opacity(*color, layer_opacity);
                    backend.draw_text_styled(
                        text,
                        x + scroll_dx,
                        y + scroll_dy,
                        *font_size,
                        c,
                        *bold,
                        *italic,
                    )?;
                },
                DisplayItem::Blit {
                    texture,
                    x,
                    y,
                    w,
                    h,
                } => {
                    backend.blit(*texture, x + scroll_dx, y + scroll_dy, *w, *h)?;
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
                        dst_y + scroll_dy,
                        *dst_w,
                        *dst_h,
                    )?;
                },
                DisplayItem::Gradient { x, y, w, h, style } => {
                    backend.fill_rect_gradient(x + scroll_dx, y + scroll_dy, *w, *h, style)?;
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
                        y + scroll_dy,
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
                        y + scroll_dy,
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
                DisplayItem::PushClip { .. }
                | DisplayItem::PopClip
                | DisplayItem::PushLayer { .. }
                | DisplayItem::PopLayer => {
                    // Already handled above.
                },
                // BlurHint is metadata for GPU backends; no-op here.
                DisplayItem::BlurHint { .. } => {},
            }
        }

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
        BorderStyle::None => {},
    }
    Ok(())
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
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 0,
            color: Color::rgb(255, 0, 0),
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(0, 255, 0),
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
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color,
        });
        dl.push(DisplayItem::FillRect {
            x: 20,
            y: 0,
            w: 10,
            h: 5,
            color,
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
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color: Color::rgb(0, 255, 0),
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
        });
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 0,
            w: 10,
            h: 5,
            color,
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
        });
        // Item outside dirty rect.
        dl.push(DisplayItem::FillRect {
            x: 500,
            y: 500,
            w: 20,
            h: 20,
            color: Color::rgb(0, 255, 0),
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
}
