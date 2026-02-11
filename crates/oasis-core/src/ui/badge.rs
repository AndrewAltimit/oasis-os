//! Badge widget: small colored tag/indicator.

use crate::backend::Color;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout;
use crate::ui::widget::Widget;

/// A small colored tag or counter indicator.
pub struct Badge {
    pub text: String,
    pub bg_color: Option<Color>,
    pub text_color: Option<Color>,
}

impl Badge {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bg_color: None,
            text_color: None,
        }
    }

    pub fn count(n: u32) -> Self {
        Self::new(n.to_string())
    }
}

impl Widget for Badge {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_xs;
        let text_w = ctx.backend.measure_text(&self.text, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        let w = text_w + 8;
        let h = text_h + 4;
        (w.max(h), h) // Ensure minimum width equals height for single chars.
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let bg = self.bg_color.unwrap_or(ctx.theme.accent);
        let tc = self.text_color.unwrap_or(ctx.theme.text_on_accent);
        let radius = h as u16 / 2;

        ctx.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;

        let fs = ctx.theme.font_size_xs;
        let text_w = ctx.backend.measure_text(&self.text, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        let tx = x + layout::center(w, text_w);
        let ty = y + layout::center(h, text_h);
        ctx.backend.draw_text(&self.text, tx, ty, fs, tc)?;
        Ok(())
    }
}
