//! ListView widget: scrollable list with virtualized item rendering.

use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::widget::Widget;

/// A scrollable list with virtualized rendering.
///
/// Only items visible within the viewport are drawn. The `render_item`
/// callback draws each visible item.
pub struct ListView<T> {
    pub items: Vec<T>,
    pub scroll_offset: i32,
    pub item_height: u32,
    pub selected: Option<usize>,
    pub render_item: fn(&T, &mut DrawContext<'_>, i32, i32, u32, u32, bool) -> Result<()>,
}

impl<T> ListView<T> {
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
        }
    }

    /// Total content height.
    pub fn content_height(&self) -> u32 {
        self.items.len() as u32 * self.item_height
    }

    /// Scroll to make the given index visible.
    pub fn scroll_to(&mut self, index: usize, viewport_h: u32) {
        let item_y = index as i32 * self.item_height as i32;
        if item_y < self.scroll_offset {
            self.scroll_offset = item_y;
        } else if item_y + self.item_height as i32 > self.scroll_offset + viewport_h as i32 {
            self.scroll_offset = item_y + self.item_height as i32 - viewport_h as i32;
        }
    }

    /// Draw the list view at the given position.
    pub fn draw_at(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        ctx.backend.push_clip_rect(x, y, w, h)?;

        let result = (|| {
            let first = (self.scroll_offset / self.item_height as i32).max(0) as usize;
            let visible = (h / self.item_height + 2) as usize;
            let last = (first + visible).min(self.items.len());

            for i in first..last {
                let item_y = y + (i as i32 * self.item_height as i32) - self.scroll_offset;
                let selected = self.selected == Some(i);

                if selected {
                    ctx.backend.fill_rect(
                        x,
                        item_y,
                        w,
                        self.item_height,
                        ctx.theme.accent_subtle,
                    )?;
                }

                (self.render_item)(
                    &self.items[i],
                    ctx,
                    x,
                    item_y,
                    w,
                    self.item_height,
                    selected,
                )?;
            }
            Ok(())
        })();

        ctx.backend.pop_clip_rect()?;
        result
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
}

impl<T> Widget for ListView<T> {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        (available_w, self.content_height())
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.draw_at(ctx, x, y, w, h)
    }
}
