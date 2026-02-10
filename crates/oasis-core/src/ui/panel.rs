//! Panel widget: container with background, border, shadow, rounded corners.

use crate::backend::Color;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout::Padding;
use crate::ui::shadow::Shadow;
use crate::ui::widget::Widget;

/// A container with background, optional border, shadow, and rounded corners.
pub struct Panel {
    pub background: Option<Color>,
    pub border: Option<(u16, Color)>,
    pub radius: u16,
    pub elevation: u8,
    pub padding: Padding,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            background: None,
            border: None,
            radius: 0,
            elevation: 0,
            padding: Padding::ZERO,
        }
    }
}

impl Panel {
    pub fn themed(ctx: &DrawContext<'_>) -> Self {
        Self {
            background: Some(ctx.theme.surface),
            border: Some((1, ctx.theme.border)),
            radius: ctx.theme.border_radius_lg,
            elevation: 1,
            padding: Padding::uniform(ctx.theme.spacing_md),
        }
    }

    /// Draw the panel at the given position and size.
    pub fn draw_at(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // Shadow.
        if self.elevation > 0 {
            let shadow = Shadow::elevation(self.elevation);
            shadow.draw(ctx.backend, x, y, w, h, self.radius)?;
        }
        // Background.
        if let Some(bg) = self.background {
            ctx.backend.fill_rounded_rect(x, y, w, h, self.radius, bg)?;
        }
        // Border.
        if let Some((bw, bc)) = self.border {
            ctx.backend
                .stroke_rounded_rect(x, y, w, h, self.radius, bw, bc)?;
        }
        Ok(())
    }
}

impl Widget for Panel {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        (available_w, available_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.draw_at(ctx, x, y, w, h)
    }
}
