//! Divider widget: horizontal or vertical separator line.

use crate::backend::Color;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::widget::Widget;

/// Orientation of the divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DividerOrientation {
    Horizontal,
    Vertical,
}

/// A separator line.
pub struct Divider {
    pub orientation: DividerOrientation,
    pub color: Option<Color>,
    pub thickness: u16,
}

impl Divider {
    pub fn horizontal() -> Self {
        Self {
            orientation: DividerOrientation::Horizontal,
            color: None,
            thickness: 1,
        }
    }

    pub fn vertical() -> Self {
        Self {
            orientation: DividerOrientation::Vertical,
            color: None,
            thickness: 1,
        }
    }
}

impl Widget for Divider {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        match self.orientation {
            DividerOrientation::Horizontal => (available_w, self.thickness as u32),
            DividerOrientation::Vertical => (self.thickness as u32, available_h),
        }
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let color = self.color.unwrap_or(ctx.theme.border_subtle);
        match self.orientation {
            DividerOrientation::Horizontal => {
                ctx.backend
                    .draw_line(x, y, x + w as i32, y, self.thickness, color)
            },
            DividerOrientation::Vertical => {
                ctx.backend
                    .draw_line(x, y, x, y + h as i32, self.thickness, color)
            },
        }
    }
}
