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
            DisplayItem::PopClip => None,
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

    /// Replay all display items against the backend.
    ///
    /// This is the main rendering path. The `scroll_dx` and `scroll_dy`
    /// offsets are applied to all items, enabling scroll without rebuild.
    pub fn replay(
        &self,
        backend: &mut dyn SdiBackend,
        scroll_dx: i32,
        scroll_dy: i32,
    ) -> Result<()> {
        backend.begin_batch()?;

        for item in &self.items {
            match item {
                DisplayItem::FillRect { x, y, w, h, color } => {
                    backend.fill_rect(x + scroll_dx, y + scroll_dy, *w, *h, *color)?;
                },
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    backend.fill_rounded_rect(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *color,
                    )?;
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
                    backend.stroke_rounded_rect(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        *color,
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
                    backend.draw_text_styled(
                        text,
                        x + scroll_dx,
                        y + scroll_dy,
                        *font_size,
                        *color,
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
                    replay_border_edge(
                        backend,
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *color,
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
                    backend.fill_shadow(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *blur,
                        *spread,
                        *offset_x,
                        *offset_y,
                        *color,
                        *radius,
                    )?;
                },
                DisplayItem::PushClip { x, y, w, h } => {
                    backend.set_clip_rect(x + scroll_dx, y + scroll_dy, *w, *h)?;
                },
                DisplayItem::PopClip => {
                    backend.reset_clip_rect()?;
                },
            }
        }

        backend.flush_batch()?;
        Ok(())
    }

    /// Replay only items that intersect the given dirty rectangle.
    ///
    /// `dirty` is in screen coordinates. Items fully outside `dirty`
    /// are skipped. Clip push/pop items are always replayed to maintain
    /// correct GPU clip state.
    pub fn replay_dirty(
        &self,
        backend: &mut dyn SdiBackend,
        dirty: &Rect,
        scroll_dx: i32,
        scroll_dy: i32,
    ) -> Result<()> {
        backend.begin_batch()?;

        for item in &self.items {
            // Clip items must always be replayed.
            match item {
                DisplayItem::PushClip { x, y, w, h } => {
                    backend.set_clip_rect(x + scroll_dx, y + scroll_dy, *w, *h)?;
                    continue;
                },
                DisplayItem::PopClip => {
                    backend.reset_clip_rect()?;
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

            // Replay the item.
            match item {
                DisplayItem::FillRect { x, y, w, h, color } => {
                    backend.fill_rect(x + scroll_dx, y + scroll_dy, *w, *h, *color)?;
                },
                DisplayItem::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    backend.fill_rounded_rect(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *color,
                    )?;
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
                    backend.stroke_rounded_rect(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *radius,
                        *stroke_width,
                        *color,
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
                    backend.draw_text_styled(
                        text,
                        x + scroll_dx,
                        y + scroll_dy,
                        *font_size,
                        *color,
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
                    replay_border_edge(
                        backend,
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *color,
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
                    backend.fill_shadow(
                        x + scroll_dx,
                        y + scroll_dy,
                        *w,
                        *h,
                        *blur,
                        *spread,
                        *offset_x,
                        *offset_y,
                        *color,
                        *radius,
                    )?;
                },
                DisplayItem::PushClip { .. } | DisplayItem::PopClip => {
                    // Already handled above.
                },
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
    use crate::test_utils::MockBackend;

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
        dl.replay(&mut backend, 5, -10).unwrap();

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
        dl.replay_dirty(&mut backend, &dirty, 0, 0).unwrap();

        // Only the first item should have been drawn.
        assert_eq!(backend.fill_rect_count(), 1);
    }
}
