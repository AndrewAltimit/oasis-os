//! TabBar widget.

use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout;
use crate::ui::widget::Widget;

/// Tab visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStyle {
    Underline,
    Filled,
    Pill,
}

/// A horizontal tab strip.
pub struct TabBar {
    pub tabs: Vec<String>,
    pub active: usize,
    pub style: TabStyle,
}

impl TabBar {
    pub fn new(tabs: Vec<String>) -> Self {
        Self {
            tabs,
            active: 0,
            style: TabStyle::Underline,
        }
    }
}

impl Widget for TabBar {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let h = ctx.backend.measure_text_height(ctx.theme.font_size_md) + 8;
        (available_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        if self.tabs.is_empty() {
            return Ok(());
        }
        let tab_w = w / self.tabs.len() as u32;
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);

        for (i, tab) in self.tabs.iter().enumerate() {
            let tx = x + (i as u32 * tab_w) as i32;
            let active = i == self.active;

            match self.style {
                TabStyle::Underline => {
                    if active {
                        ctx.backend
                            .fill_rect(tx, y + h as i32 - 2, tab_w, 2, ctx.theme.accent)?;
                    }
                },
                TabStyle::Filled => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            tab_w - 4,
                            h - 4,
                            ctx.theme.border_radius_sm,
                            ctx.theme.accent,
                        )?;
                    }
                },
                TabStyle::Pill => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            tab_w - 4,
                            h - 4,
                            (h - 4) as u16 / 2,
                            ctx.theme.accent,
                        )?;
                    }
                },
            }

            let text_w = ctx.backend.measure_text(tab, fs);
            let label_x = tx + layout::center(tab_w, text_w);
            let label_y = y + layout::center(h, text_h);
            let color = if active {
                match self.style {
                    TabStyle::Underline => ctx.theme.accent,
                    TabStyle::Filled | TabStyle::Pill => ctx.theme.text_on_accent,
                }
            } else {
                ctx.theme.text_secondary
            };
            ctx.backend.draw_text(tab, label_x, label_y, fs, color)?;
        }

        // Bottom border for underline style.
        if self.style == TabStyle::Underline {
            ctx.backend.draw_line(
                x,
                y + h as i32 - 1,
                x + w as i32,
                y + h as i32 - 1,
                1,
                ctx.theme.border_subtle,
            )?;
        }
        Ok(())
    }
}
