//! ListView widget: scrollable list with virtualized item rendering.

use crate::context::DrawContext;
use crate::states::WidgetStateColors;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A scrollable list with virtualized rendering.
///
/// Only items visible within the viewport are drawn. The `render_item`
/// callback draws each visible item.
pub struct ListView<T> {
    /// List items.
    pub items: Vec<T>,
    /// Vertical scroll offset in pixels.
    pub scroll_offset: i32,
    /// Height of each item in pixels.
    pub item_height: u32,
    /// Currently selected item index.
    pub selected: Option<usize>,
    /// Callback to render a single item.
    pub render_item: fn(&T, &mut DrawContext<'_>, i32, i32, u32, u32, bool) -> Result<()>,
    /// Index of the item under the pointer, if any.
    pub hovered: Option<usize>,
    /// Whether the list has keyboard focus (draws a focus ring around
    /// the viewport).
    pub focused: bool,
}

impl<T> ListView<T> {
    /// Create a new list view.
    pub fn new(
        items: Vec<T>,
        item_height: u32,
        render_item: fn(&T, &mut DrawContext<'_>, i32, i32, u32, u32, bool) -> Result<()>,
    ) -> Self {
        Self {
            items,
            scroll_offset: 0,
            item_height: item_height.max(1),
            selected: None,
            render_item,
            hovered: None,
            focused: false,
        }
    }

    /// Index of the item at local offset `dy` (pixels from the top of a
    /// `viewport_h`-pixel viewport), accounting for the scroll offset
    /// exactly as [`draw_at`](Self::draw_at) applies it.
    pub fn index_at(&self, dy: i32, viewport_h: u32) -> Option<usize> {
        if dy < 0 {
            return None;
        }
        let content_y = i64::from(dy) + i64::from(self.effective_scroll(viewport_h));
        // `item_height` is a public field: guard a zero set after `new`.
        let i = content_y / i64::from(self.item_height.max(1));
        usize::try_from(i).ok().filter(|&i| i < self.items.len())
    }

    /// `scroll_offset` clamped to `0..=max_scroll(viewport_h)`: the offset
    /// drawing and hit testing use. (Items removed under a scrolled list
    /// used to leave it drawing blank rows past the end.)
    pub fn effective_scroll(&self, viewport_h: u32) -> i32 {
        self.scroll_offset.clamp(0, self.max_scroll(viewport_h))
    }

    /// Total content height.
    pub fn content_height(&self) -> u32 {
        u32::try_from(self.items.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(self.item_height)
    }

    /// Largest valid `scroll_offset` for a `viewport_h`-pixel viewport.
    pub fn max_scroll(&self, viewport_h: u32) -> i32 {
        crate::layout::max_scroll(self.content_height(), viewport_h)
    }

    /// Scroll by `delta` pixels (mouse wheel, page keys), clamped to
    /// `0..=max_scroll(viewport_h)`.
    pub fn scroll_by(&mut self, delta: i32, viewport_h: u32) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(delta)
            .clamp(0, self.max_scroll(viewport_h));
    }

    /// Scroll to make the given index visible.
    ///
    /// The result is clamped to `0..=max_scroll(viewport_h)`, so an index
    /// past the end or a viewport taller than the content cannot scroll
    /// beyond the last item.
    pub fn scroll_to(&mut self, index: usize, viewport_h: u32) {
        let ih = i64::from(self.item_height.max(1));
        let item_y = i64::try_from(index).unwrap_or(i64::MAX).saturating_mul(ih);
        let scroll = i64::from(self.scroll_offset);
        let item_end = item_y.saturating_add(ih);
        let target = if item_y < scroll {
            item_y
        } else if item_end > scroll + i64::from(viewport_h) {
            item_end - i64::from(viewport_h)
        } else {
            scroll
        };
        let max = i64::from(self.max_scroll(viewport_h));
        self.scroll_offset = target.clamp(0, max) as i32;
    }

    /// Draw the list view at the given position.
    pub fn draw_at(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        ctx.backend.push_clip_rect(x, y, w, h)?;

        // `item_height` is a public field: guard a zero set after `new`.
        let ih = self.item_height.max(1);
        let scroll = i64::from(self.effective_scroll(h));
        let result: Result<()> = (|| {
            let first = usize::try_from(scroll / i64::from(ih)).unwrap_or(0);
            // Exact visible count: items that fit plus one for
            // a partially visible trailing item.
            let visible = h / ih + u32::from(!h.is_multiple_of(ih)) + 1;
            let last = first.saturating_add(visible as usize).min(self.items.len());

            for i in first..last {
                // In i64: `i * ih` overflows i32 deep into a long list.
                let item_y = i64::from(y) + i as i64 * i64::from(ih) - scroll;
                let item_y = item_y.clamp(i32::MIN.into(), i32::MAX.into()) as i32;
                let selected = self.selected == Some(i);
                let hovered = self.hovered == Some(i);

                if let Some(fill) = WidgetStateColors::row_bg(ctx.theme, selected, hovered) {
                    ctx.backend.fill_rect(x, item_y, w, ih, fill)?;
                }

                (self.render_item)(&self.items[i], ctx, x, item_y, w, ih, selected)?;
            }
            Ok(())
        })();

        ctx.backend.pop_clip_rect()?;
        result?;

        // Keyboard focus ring around the viewport (outside the clip).
        if self.focused {
            crate::focus::FocusStyle::from_theme(ctx.theme).draw(ctx.backend, x, y, w, h)?;
        }
        Ok(())
    }
}

impl<T> Widget for ListView<T> {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        (available_w, self.content_height())
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.draw_at(ctx, x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_render(
        _item: &String,
        _ctx: &mut DrawContext<'_>,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
        _selected: bool,
    ) -> Result<()> {
        Ok(())
    }

    #[test]
    fn new_defaults() {
        let lv = ListView::new(vec!["a".to_string(), "b".to_string()], 20, dummy_render);
        assert_eq!(lv.items.len(), 2);
        assert_eq!(lv.item_height, 20);
        assert_eq!(lv.scroll_offset, 0);
        assert!(lv.selected.is_none());
    }

    #[test]
    fn item_height_minimum_one() {
        let lv = ListView::new(vec!["x".to_string()], 0, dummy_render);
        assert_eq!(lv.item_height, 1);
    }

    #[test]
    fn content_height_empty() {
        let lv: ListView<String> = ListView::new(vec![], 20, dummy_render);
        assert_eq!(lv.content_height(), 0);
    }

    #[test]
    fn content_height_multiple() {
        let items: Vec<String> = (0..5).map(|i| i.to_string()).collect();
        let lv = ListView::new(items, 30, dummy_render);
        assert_eq!(lv.content_height(), 150);
    }

    #[test]
    fn scroll_to_below_viewport() {
        let items: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        // Viewport is 60px tall, scroll to item 5 (at y=100).
        lv.scroll_to(5, 60);
        // Item 5 bottom = 120, should scroll so it's visible.
        assert!(lv.scroll_offset > 0);
        assert!(lv.scroll_offset <= 100);
    }

    #[test]
    fn scroll_to_above_viewport() {
        let items: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        lv.scroll_offset = 100;
        // Scroll to item 1 (at y=20), which is above current viewport.
        lv.scroll_to(1, 60);
        assert_eq!(lv.scroll_offset, 20);
    }

    #[test]
    fn scroll_to_already_visible() {
        let items: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        // Item 0 is at y=0, viewport starts at 0, height 100 -- already visible.
        lv.scroll_to(0, 100);
        assert_eq!(lv.scroll_offset, 0);
    }

    #[test]
    fn selected_index() {
        let items: Vec<String> = (0..3).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        lv.selected = Some(2);
        assert_eq!(lv.selected, Some(2));
    }

    // -- Draw / measure tests using RecordingBackend --

    use crate::context::DrawContext;
    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    /// A render callback that actually draws the item text so we can
    /// assert on `draw_text` calls recorded by `RecordingBackend`.
    // `&String` is `&T` for `ListView<String>`; the callback type fixes it.
    #[allow(clippy::ptr_arg)]
    fn draw_render(
        item: &String,
        ctx: &mut DrawContext<'_>,
        x: i32,
        y: i32,
        _w: u32,
        _h: u32,
        _selected: bool,
    ) -> Result<()> {
        ctx.label(item, x, y)
    }

    #[test]
    fn measure_returns_item_height_times_count() {
        let items: Vec<String> = (0..5).map(|i| i.to_string()).collect();
        let lv = ListView::new(items, 20, dummy_render);
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, h) = lv.measure(&ctx, 200, 400);
        assert_eq!(w, 200);
        assert_eq!(h, 100); // 5 items * 20px
    }

    #[test]
    fn draw_renders_visible_items() {
        let items: Vec<String> = (0..5).map(|i| format!("item{i}")).collect();
        let lv = ListView::new(items, 20, draw_render);
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.draw_text_count() > 0);
        assert!(backend.has_text("item0"));
    }

    #[test]
    fn draw_selected_item_highlight() {
        let items: Vec<String> = (0..3).map(|i| format!("item{i}")).collect();
        let mut lv = ListView::new(items, 20, draw_render);
        lv.selected = Some(1);
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // The selected item triggers a fill_rect for the highlight background.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn scroll_offset_skips_items() {
        let items: Vec<String> = (0..10).map(|i| format!("item{i}")).collect();
        let mut lv = ListView::new(items, 20, draw_render);
        // Scroll past the first two items (offset = 40 means items 0,1 are above).
        lv.scroll_offset = 40;
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.draw(&mut ctx, 0, 0, 200, 60).unwrap();
        }
        // Items at index 2+ should be drawn; item0 and item1 should be skipped.
        assert!(!backend.has_text("item0"));
        assert!(!backend.has_text("item1"));
        assert!(backend.has_text("item2"));
    }

    #[test]
    fn content_height_calculation() {
        let items: Vec<String> = (0..7).map(|i| i.to_string()).collect();
        let lv = ListView::new(items, 15, dummy_render);
        assert_eq!(lv.content_height(), 7 * 15);
    }

    #[test]
    fn scroll_to_clamps_range() {
        let items: Vec<String> = (0..5).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        // Scroll to an index way beyond the list.
        lv.scroll_to(100, 60);
        // scroll_offset should not go negative and should be bounded
        // by the item_y calculation (100 * 20 = 2000, minus viewport).
        assert!(lv.scroll_offset >= 0);
    }

    #[test]
    fn empty_list_draws_nothing() {
        let lv: ListView<String> = ListView::new(vec![], 20, draw_render);
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn draw_calls_push_clip() {
        let items: Vec<String> = (0..3).map(|i| format!("item{i}")).collect();
        let lv = ListView::new(items, 20, draw_render);
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // draw_at calls push_clip_rect which delegates to set_clip_rect.
        // RecordingBackend does not record set_clip_rect in `calls`, but the call
        // must not fail. We verify the draw completed and items were rendered.
        assert!(backend.draw_text_count() > 0);
    }

    use oasis_types::backend::Color;

    fn fills_for(hovered: Option<usize>, selected: Option<usize>, focused: bool) -> Vec<Color> {
        crate::test_utils::fill_colors_of(|ctx| {
            let items: Vec<String> = (0..3).map(|i| format!("item{i}")).collect();
            let mut lv = ListView::new(items, 20, dummy_render);
            lv.hovered = hovered;
            lv.selected = selected;
            lv.focused = focused;
            lv.draw(ctx, 0, 0, 200, 60).unwrap();
        })
    }

    #[test]
    fn hovered_row_highlight_is_distinct() {
        let theme = Theme::dark();
        let rest = fills_for(None, None, false);
        assert!(rest.is_empty(), "no fills at rest");
        let sel = fills_for(None, Some(1), false);
        assert_eq!(sel, vec![theme.accent_subtle]);
        let hov = fills_for(Some(1), None, false);
        let both = fills_for(Some(1), Some(1), false);
        assert_eq!(hov.len(), 1);
        assert_ne!(hov, sel);
        assert_ne!(both, sel);
        assert_ne!(both, hov);
    }

    #[test]
    fn focused_list_draws_ring() {
        let theme = Theme::dark();
        let ring = crate::focus::FocusStyle::from_theme(&theme).color;
        assert!(!fills_for(None, None, false).contains(&ring));
        assert!(fills_for(None, None, true).contains(&ring));
    }

    #[test]
    fn index_at_accounts_for_scroll() {
        let items: Vec<String> = (0..5).map(|i| format!("item{i}")).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        assert_eq!(lv.index_at(0, 40), Some(0));
        assert_eq!(lv.index_at(45, 40), Some(2));
        assert_eq!(lv.index_at(-1, 40), None);
        assert_eq!(lv.index_at(100, 40), None);
        lv.scroll_offset = 40;
        assert_eq!(lv.index_at(0, 40), Some(2));
    }

    /// Items removed under a scrolled list left `scroll_offset` past the
    /// end: the list drew blank and clicks missed every row.
    #[test]
    fn stale_scroll_offset_is_clamped_for_drawing_and_hits() {
        let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        lv.scroll_by(10_000, 60);
        lv.items.truncate(3);
        assert_eq!(lv.effective_scroll(60), 0);
        assert_eq!(lv.index_at(0, 60), Some(0));
        let theme = crate::theme::Theme::dark();
        let mut backend = crate::test_utils::RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            lv.selected = Some(0);
            lv.draw(&mut ctx, 0, 0, 100, 60).unwrap();
        }
        assert!(!backend.fill_colors().is_empty(), "selected row drawn");
    }

    #[test]
    fn scroll_to_never_passes_the_last_item() {
        let items: Vec<String> = (0..5).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        lv.scroll_to(99, 40);
        assert_eq!(lv.scroll_offset, lv.max_scroll(40));
        lv.scroll_to(2, 400);
        assert_eq!(lv.scroll_offset, 0, "everything fits");
        lv.scroll_to(usize::MAX, 40);
        assert_eq!(lv.scroll_offset, 60);
    }

    #[test]
    fn scroll_by_clamps_to_content() {
        let items: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let mut lv = ListView::new(items, 20, dummy_render);
        lv.scroll_by(1_000, 60);
        assert_eq!(lv.scroll_offset, 140);
        lv.scroll_by(-1_000, 60);
        assert_eq!(lv.scroll_offset, 0);
        // Everything fits: no scrolling at all.
        lv.scroll_by(50, 400);
        assert_eq!(lv.scroll_offset, 0);
    }

    #[test]
    fn zero_item_height_set_after_new_does_not_divide_by_zero() {
        let mut lv = ListView::new(vec!["a".to_string()], 20, dummy_render);
        lv.item_height = 0;
        let theme = crate::theme::Theme::dark();
        let mut backend = crate::test_utils::RecordingBackend::default();
        let mut ctx = DrawContext::new(&mut backend, &theme);
        lv.draw(&mut ctx, 0, 0, 50, 50).unwrap();
    }
}
