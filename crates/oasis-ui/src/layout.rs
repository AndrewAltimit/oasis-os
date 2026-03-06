//! Layout helpers: centering, alignment, padding, distribution.

/// Gap between a control indicator (checkbox, radio) and its label text.
pub const LABEL_GAP: u32 = 6;

/// Padding specification for all four sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Padding {
    /// Top padding in pixels.
    pub top: u16,
    /// Right padding in pixels.
    pub right: u16,
    /// Bottom padding in pixels.
    pub bottom: u16,
    /// Left padding in pixels.
    pub left: u16,
}

impl Padding {
    /// Zero padding on all sides.
    pub const ZERO: Self = Self::uniform(0);

    /// Create uniform padding on all sides.
    pub const fn uniform(p: u16) -> Self {
        Self {
            top: p,
            right: p,
            bottom: p,
            left: p,
        }
    }

    /// Create symmetric padding (horizontal and vertical).
    pub const fn symmetric(h: u16, v: u16) -> Self {
        Self {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }

    /// Create padding with individual side values.
    pub const fn new(top: u16, right: u16, bottom: u16, left: u16) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Compute the inner rectangle after applying padding.
    pub fn inner_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        (
            x + self.left as i32,
            y + self.top as i32,
            w.saturating_sub(self.left as u32 + self.right as u32),
            h.saturating_sub(self.top as u32 + self.bottom as u32),
        )
    }

    /// Total horizontal padding (left + right).
    pub fn horizontal(&self) -> u32 {
        self.left as u32 + self.right as u32
    }

    /// Total vertical padding (top + bottom).
    pub fn vertical(&self) -> u32 {
        self.top as u32 + self.bottom as u32
    }
}

/// Compute centered position of a child within a parent.
///
/// Uses round-up division so that on odd remainders the child is biased
/// toward the visual center rather than the top-left edge.
pub fn center(parent_size: u32, child_size: u32) -> i32 {
    let diff = parent_size as i32 - child_size as i32;
    if diff <= 0 {
        return 0;
    }
    (diff + 1) / 2
}

/// Compute vertical center for text within a given height.
pub fn center_text_y(height: u32, font_size: u16, ascent: u32) -> i32 {
    let text_h = font_size as i32;
    (height as i32 - text_h) / 2 + ascent as i32
}

/// Distribute `n` items evenly across `total` pixels with `gap` pixels between.
///
/// Returns `(item_size, positions)`.
pub fn distribute(total: u32, n: u32, gap: u32) -> (u32, Vec<i32>) {
    if n == 0 {
        return (0, Vec::new());
    }
    let total_gap = gap * n.saturating_sub(1);
    let item_size = total.saturating_sub(total_gap) / n;
    let positions = (0..n).map(|i| (i * (item_size + gap)) as i32).collect();
    (item_size, positions)
}

/// Horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    /// Align to the left edge.
    Left,
    /// Align to the center.
    Center,
    /// Align to the right edge.
    Right,
}

/// Vertical alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    /// Align to the top edge.
    Top,
    /// Align to the center.
    Center,
    /// Align to the bottom edge.
    Bottom,
}

/// Compute x position for horizontal alignment.
pub fn align_x(container_w: u32, child_w: u32, align: HAlign) -> i32 {
    match align {
        HAlign::Left => 0,
        HAlign::Center => center(container_w, child_w),
        HAlign::Right => (container_w as i32 - child_w as i32).max(0),
    }
}

/// Compute y position for vertical alignment.
pub fn align_y(container_h: u32, child_h: u32, align: VAlign) -> i32 {
    match align {
        VAlign::Top => 0,
        VAlign::Center => center(container_h, child_h),
        VAlign::Bottom => (container_h as i32 - child_h as i32).max(0),
    }
}

// ---------------------------------------------------------------------------
// Measure cache
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use crate::context::DrawContext;
use crate::widget::Widget;

/// Cache key: `(widget_id, available_width)`.
///
/// `widget_id` is a caller-supplied opaque identifier (e.g. an index or hash)
/// that uniquely identifies a widget instance within the layout pass.
/// `available_width` is included because it is by far the most common axis
/// that changes between measure calls; height rarely constrains widgets.
type MeasureKey = (u64, u32);

/// Opt-in cache for `Widget::measure()` results.
///
/// Widgets are measured every frame even when their inputs have not changed.
/// Wrap the call in [`MeasureCache::get_or_measure`] to skip redundant work.
///
/// # Invalidation
///
/// Call [`MeasureCache::next_generation`] once per frame (or whenever widget
/// content changes). Entries from a previous generation are treated as stale
/// and will be recomputed on next access.
///
/// # Example
///
/// ```ignore
/// let mut cache = MeasureCache::new();
/// // — each frame —
/// cache.next_generation();
/// let (w, h) = cache.get_or_measure(widget_id, avail_w, || {
///     my_widget.measure(&ctx, avail_w, avail_h)
/// });
/// ```
pub struct MeasureCache {
    generation: u64,
    entries: HashMap<MeasureKey, MeasureCacheEntry>,
}

/// A single cached measurement together with the generation it was stored in.
struct MeasureCacheEntry {
    generation: u64,
    size: (u32, u32),
}

impl MeasureCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            generation: 0,
            entries: HashMap::new(),
        }
    }

    /// Advance the generation counter.
    ///
    /// Previous entries are not deleted immediately — they are lazily evicted
    /// on the next lookup — so this call is O(1).
    pub fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Return the current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Return a cached size or compute it via the closure, caching the result.
    ///
    /// `widget_id` must uniquely identify the widget within the current layout.
    /// `available_w` is the width constraint passed to `Widget::measure`.
    pub fn get_or_measure(
        &mut self,
        widget_id: u64,
        available_w: u32,
        measure_fn: impl FnOnce() -> (u32, u32),
    ) -> (u32, u32) {
        let key = (widget_id, available_w);
        if let Some(entry) = self.entries.get(&key)
            && entry.generation == self.generation
        {
            return entry.size;
        }
        let size = measure_fn();
        self.entries.insert(
            key,
            MeasureCacheEntry {
                generation: self.generation,
                size,
            },
        );
        size
    }

    /// Remove all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries currently stored (including stale ones).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache contains zero entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for MeasureCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: measure a widget through the cache.
///
/// This is a free function so callers do not need to manually construct the
/// closure. `widget_id` must uniquely identify `widget` in the current layout.
pub fn cached_measure(
    cache: &mut MeasureCache,
    widget_id: u64,
    widget: &dyn Widget,
    ctx: &DrawContext<'_>,
    available_w: u32,
    available_h: u32,
) -> (u32, u32) {
    cache.get_or_measure(widget_id, available_w, || {
        widget.measure(ctx, available_w, available_h)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_inner_rect() {
        let p = Padding::uniform(4);
        let (x, y, w, h) = p.inner_rect(10, 10, 100, 50);
        assert_eq!((x, y, w, h), (14, 14, 92, 42));
    }

    #[test]
    fn center_calculation() {
        assert_eq!(center(100, 20), 40);
        assert_eq!(center(10, 20), 0); // Child larger than parent.
        // Odd remainder rounds up for better visual centering.
        assert_eq!(center(101, 20), 41); // (101-20+1)/2 = 41
        assert_eq!(center(31, 10), 11); // (31-10+1)/2 = 11
    }

    #[test]
    fn distribute_items() {
        let (size, pos) = distribute(100, 4, 4);
        assert_eq!(size, 22);
        assert_eq!(pos, vec![0, 26, 52, 78]);
    }

    // -- Additional layout tests --

    #[test]
    fn padding_zero_is_identity() {
        let p = Padding::ZERO;
        let (x, y, w, h) = p.inner_rect(10, 20, 100, 50);
        assert_eq!((x, y, w, h), (10, 20, 100, 50));
    }

    #[test]
    fn padding_symmetric() {
        let p = Padding::symmetric(10, 5);
        assert_eq!(p.left, 10);
        assert_eq!(p.right, 10);
        assert_eq!(p.top, 5);
        assert_eq!(p.bottom, 5);
        assert_eq!(p.horizontal(), 20);
        assert_eq!(p.vertical(), 10);
    }

    #[test]
    fn padding_individual_sides() {
        let p = Padding::new(1, 2, 3, 4);
        assert_eq!(p.top, 1);
        assert_eq!(p.right, 2);
        assert_eq!(p.bottom, 3);
        assert_eq!(p.left, 4);
        assert_eq!(p.horizontal(), 6);
        assert_eq!(p.vertical(), 4);
    }

    #[test]
    fn padding_inner_rect_larger_than_container() {
        let p = Padding::uniform(100);
        let (x, y, w, h) = p.inner_rect(0, 0, 50, 50);
        assert_eq!(x, 100);
        assert_eq!(y, 100);
        assert_eq!(w, 0); // saturating_sub
        assert_eq!(h, 0);
    }

    #[test]
    fn center_zero_parent() {
        assert_eq!(center(0, 10), 0);
    }

    #[test]
    fn center_zero_child() {
        assert_eq!(center(100, 0), 50);
    }

    #[test]
    fn center_equal_sizes() {
        assert_eq!(center(50, 50), 0);
    }

    #[test]
    fn center_text_y_basic() {
        let y = center_text_y(24, 12, 10);
        assert_eq!(y, 16); // (24-12)/2 + 10
    }

    #[test]
    fn distribute_zero_items() {
        let (size, pos) = distribute(100, 0, 4);
        assert_eq!(size, 0);
        assert!(pos.is_empty());
    }

    #[test]
    fn distribute_one_item() {
        let (size, pos) = distribute(100, 1, 0);
        assert_eq!(size, 100);
        assert_eq!(pos, vec![0]);
    }

    #[test]
    fn distribute_no_gap() {
        let (size, pos) = distribute(100, 5, 0);
        assert_eq!(size, 20);
        assert_eq!(pos, vec![0, 20, 40, 60, 80]);
    }

    #[test]
    fn distribute_large_gap_saturates() {
        let (size, pos) = distribute(10, 4, 100);
        // total_gap = 300, but total = 10 so saturating_sub = 0
        assert_eq!(size, 0);
        assert_eq!(pos.len(), 4);
    }

    #[test]
    fn align_x_left() {
        assert_eq!(align_x(200, 50, HAlign::Left), 0);
    }

    #[test]
    fn align_x_center() {
        let x = align_x(200, 50, HAlign::Center);
        assert_eq!(x, center(200, 50));
    }

    #[test]
    fn align_x_right() {
        assert_eq!(align_x(200, 50, HAlign::Right), 150);
    }

    #[test]
    fn align_x_right_child_larger() {
        assert_eq!(align_x(50, 200, HAlign::Right), 0);
    }

    #[test]
    fn align_y_top() {
        assert_eq!(align_y(100, 20, VAlign::Top), 0);
    }

    #[test]
    fn align_y_center() {
        let y = align_y(100, 20, VAlign::Center);
        assert_eq!(y, center(100, 20));
    }

    #[test]
    fn align_y_bottom() {
        assert_eq!(align_y(100, 20, VAlign::Bottom), 80);
    }

    #[test]
    fn align_y_bottom_child_larger() {
        assert_eq!(align_y(20, 100, VAlign::Bottom), 0);
    }

    #[test]
    fn halign_debug() {
        assert_eq!(format!("{:?}", HAlign::Left), "Left");
        assert_eq!(format!("{:?}", HAlign::Center), "Center");
        assert_eq!(format!("{:?}", HAlign::Right), "Right");
    }

    #[test]
    fn valign_debug() {
        assert_eq!(format!("{:?}", VAlign::Top), "Top");
        assert_eq!(format!("{:?}", VAlign::Center), "Center");
        assert_eq!(format!("{:?}", VAlign::Bottom), "Bottom");
    }

    #[test]
    fn padding_clone_and_eq() {
        let a = Padding::uniform(8);
        let b = a;
        assert_eq!(a, b);
    }

    // -- MeasureCache tests -------------------------------------------------

    use super::MeasureCache;
    use std::cell::Cell;

    #[test]
    fn cache_new_is_empty() {
        let cache = MeasureCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.generation(), 0);
    }

    #[test]
    fn cache_default_is_new() {
        let cache = MeasureCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.generation(), 0);
    }

    #[test]
    fn cache_stores_and_returns_result() {
        let mut cache = MeasureCache::new();
        let call_count = Cell::new(0u32);
        let size = cache.get_or_measure(1, 200, || {
            call_count.set(call_count.get() + 1);
            (80, 24)
        });
        assert_eq!(size, (80, 24));
        assert_eq!(call_count.get(), 1);

        // Second call with same key should return cached value.
        let size2 = cache.get_or_measure(1, 200, || {
            call_count.set(call_count.get() + 1);
            (999, 999)
        });
        assert_eq!(size2, (80, 24));
        assert_eq!(call_count.get(), 1, "measure_fn should not be called again");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_different_widget_id() {
        let mut cache = MeasureCache::new();
        cache.get_or_measure(1, 200, || (10, 20));
        cache.get_or_measure(2, 200, || (30, 40));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get_or_measure(1, 200, || unreachable!()), (10, 20));
        assert_eq!(cache.get_or_measure(2, 200, || unreachable!()), (30, 40));
    }

    #[test]
    fn cache_different_width() {
        let mut cache = MeasureCache::new();
        cache.get_or_measure(1, 200, || (80, 24));
        cache.get_or_measure(1, 300, || (120, 24));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get_or_measure(1, 200, || unreachable!()), (80, 24));
        assert_eq!(cache.get_or_measure(1, 300, || unreachable!()), (120, 24));
    }

    #[test]
    fn cache_invalidates_on_next_generation() {
        let mut cache = MeasureCache::new();
        cache.get_or_measure(1, 200, || (80, 24));

        cache.next_generation();
        assert_eq!(cache.generation(), 1);

        // Same key but stale generation — should recompute.
        let call_count = Cell::new(0u32);
        let size = cache.get_or_measure(1, 200, || {
            call_count.set(call_count.get() + 1);
            (90, 30)
        });
        assert_eq!(size, (90, 30));
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn cache_clear_removes_all() {
        let mut cache = MeasureCache::new();
        cache.get_or_measure(1, 100, || (10, 10));
        cache.get_or_measure(2, 100, || (20, 20));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_generation_wraps() {
        let mut cache = MeasureCache::new();
        cache.generation = u64::MAX;
        cache.next_generation();
        assert_eq!(cache.generation(), 0);
    }

    #[test]
    fn cached_measure_helper() {
        use super::cached_measure;
        use crate::button::Button;
        use crate::context::DrawContext;
        use crate::test_utils::MockBackend;
        use crate::theme::Theme;

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let btn = Button::new("Hello");

        let mut cache = MeasureCache::new();
        let (w1, h1) = cached_measure(&mut cache, 42, &btn, &ctx, 200, 100);
        assert!(w1 > 0);
        assert!(h1 > 0);

        // Second call returns same result from cache.
        let (w2, h2) = cached_measure(&mut cache, 42, &btn, &ctx, 200, 100);
        assert_eq!((w1, h1), (w2, h2));
        assert_eq!(cache.len(), 1);
    }
}
