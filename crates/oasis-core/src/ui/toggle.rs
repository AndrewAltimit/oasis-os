//! Toggle switch widget.

use crate::error::Result;
use crate::ui::color::lerp_color;
use crate::ui::context::DrawContext;
use crate::ui::widget::Widget;

/// An on/off toggle switch.
pub struct Toggle {
    pub on: bool,
    /// Animation progress (0.0 = off, 1.0 = on).
    pub progress: f32,
}

impl Toggle {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            progress: if on { 1.0 } else { 0.0 },
        }
    }

    /// Animate toward the current `on` state.
    pub fn animate(&mut self, dt_ms: u32) {
        let target = if self.on { 1.0 } else { 0.0 };
        let speed = dt_ms as f32 / 150.0;
        if self.progress < target {
            self.progress = (self.progress + speed).min(1.0);
        } else if self.progress > target {
            self.progress = (self.progress - speed).max(0.0);
        }
    }
}

impl Widget for Toggle {
    fn measure(&self, _ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        (28, 16)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let radius = h as u16 / 2;
        let bg = lerp_color(ctx.theme.scrollbar_track, ctx.theme.accent, self.progress);
        ctx.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;

        // Thumb circle.
        let thumb_r = (h as i32 / 2) - 2;
        let travel = w as i32 - h as i32;
        let thumb_x = x + h as i32 / 2 + (travel as f32 * self.progress) as i32;
        let thumb_y = y + h as i32 / 2;
        ctx.backend
            .fill_circle(thumb_x, thumb_y, thumb_r as u16, ctx.theme.text_on_accent)?;
        Ok(())
    }
}
