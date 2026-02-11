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

        let first = (self.scroll_offset / self.item_height as i32).max(0) as usize;
        let visible = (h / self.item_height + 2) as usize;
        let last = (first + visible).min(self.items.len());

        for i in first..last {
            let item_y = y + (i as i32 * self.item_height as i32) - self.scroll_offset;
            let selected = self.selected == Some(i);

            if selected {
                ctx.backend
                    .fill_rect(x, item_y, w, self.item_height, ctx.theme.accent_subtle)?;
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

        ctx.backend.pop_clip_rect()?;
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
