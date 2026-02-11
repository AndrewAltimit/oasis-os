//! ScrollView widget: scrollable content region with scrollbar.

use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::widget::Widget;

/// Scrollbar visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarStyle {
    Thin,
    Wide,
    Hidden,
}

/// A scrollable content region with a scrollbar.
pub struct ScrollView {
    pub content_height: u32,
    pub scroll_y: i32,
    pub viewport_height: u32,
    pub scrollbar_style: ScrollbarStyle,
}

impl ScrollView {
    pub fn new(content_height: u32, viewport_height: u32) -> Self {
        Self {
            content_height,
            scroll_y: 0,
            viewport_height,
            scrollbar_style: ScrollbarStyle::Thin,
        }
    }

    /// Clamp scroll position to valid range.
    pub fn clamp_scroll(&mut self) {
        let max_scroll = (self.content_height as i32 - self.viewport_height as i32).max(0);
        self.scroll_y = self.scroll_y.clamp(0, max_scroll);
    }

    /// Scroll by a delta amount.
    pub fn scroll_by(&mut self, delta: i32) {
        self.scroll_y += delta;
        self.clamp_scroll();
    }

    /// Whether the scrollbar should be visible.
    pub fn needs_scrollbar(&self) -> bool {
        self.content_height > self.viewport_height && self.scrollbar_style != ScrollbarStyle::Hidden
    }

    /// Draw the scrollbar.
    pub fn draw_scrollbar(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, h: u32) -> Result<()> {
        if !self.needs_scrollbar() {
            return Ok(());
        }
        let bar_w = match self.scrollbar_style {
            ScrollbarStyle::Thin => 3u32,
            ScrollbarStyle::Wide => 6,
            ScrollbarStyle::Hidden => return Ok(()),
        };

        // Track.
        ctx.backend.fill_rounded_rect(
            x,
            y,
            bar_w,
            h,
            bar_w as u16 / 2,
            ctx.theme.scrollbar_track,
        )?;

        // Thumb.
        let ratio = self.viewport_height as f32 / self.content_height as f32;
        let thumb_h = ((h as f32 * ratio).max(bar_w as f32)) as u32;
        let scroll_range = self.content_height - self.viewport_height;
        let thumb_y = if scroll_range > 0 {
            ((h - thumb_h) as f32 * self.scroll_y as f32 / scroll_range as f32) as i32
        } else {
            0
        };
        ctx.backend.fill_rounded_rect(
            x,
            y + thumb_y,
            bar_w,
            thumb_h,
            bar_w as u16 / 2,
            ctx.theme.scrollbar_thumb,
        )?;
        Ok(())
    }
}

impl Widget for ScrollView {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        (available_w, available_h.min(self.viewport_height))
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let bar_w = if self.needs_scrollbar() {
            match self.scrollbar_style {
                ScrollbarStyle::Thin => 3u32,
                ScrollbarStyle::Wide => 6,
                ScrollbarStyle::Hidden => 0,
            }
        } else {
            0
        };
        let content_w = w.saturating_sub(bar_w + 2);

        // Draw scrollbar.
        if bar_w > 0 {
            self.draw_scrollbar(ctx, x + content_w as i32 + 2, y, h)?;
        }
        Ok(())
    }
}
