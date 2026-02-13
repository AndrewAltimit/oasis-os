//! Panel widget: container with background, border, shadow, rounded corners.

use crate::context::DrawContext;
use crate::layout::Padding;
use crate::shadow::Shadow;
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

/// A container with background, optional border, shadow, and rounded corners.
pub struct Panel {
    /// Optional background color.
    pub background: Option<Color>,
    /// Optional border (thickness, color).
    pub border: Option<(u16, Color)>,
    /// Corner radius in pixels.
    pub radius: u16,
    /// Shadow elevation level (0 = no shadow).
    pub elevation: u8,
    /// Internal padding.
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
    /// Create a panel with theme defaults.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_available_size() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let panel = Panel::default();
        let (w, h) = panel.measure(&ctx, 200, 100);
        assert_eq!(w, 200);
        assert_eq!(h, 100);
    }

    #[test]
    fn draw_background() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut panel = Panel::default();
            panel.background = Some(Color::rgb(30, 30, 30));
            panel.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_border() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut panel = Panel::default();
            panel.background = Some(Color::rgb(30, 30, 30));
            panel.border = Some((1, Color::WHITE));
            panel.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Background fill + border (stroke_rect emits 4 fill_rects) → > 1.
        assert!(backend.fill_rect_count() > 1);
    }

    #[test]
    fn draw_zero_elevation() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut panel = Panel::default();
            panel.elevation = 0;
            panel.background = Some(Color::rgb(30, 30, 30));
            panel.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Should not panic.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_with_radius() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut panel = Panel::default();
            panel.radius = 8;
            panel.background = Some(Color::rgb(30, 30, 30));
            panel.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Should not panic; fill_rounded_rect falls back to fill_rect.
        assert!(backend.fill_rect_count() > 0);
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
