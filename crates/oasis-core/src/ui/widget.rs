//! Widget trait definition.

use crate::error::Result;
use crate::ui::context::DrawContext;

/// Minimum interface for a renderable UI element.
pub trait Widget {
    /// Compute the desired size given available space.
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32);

    /// Draw the widget at the given position and size.
    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
}
