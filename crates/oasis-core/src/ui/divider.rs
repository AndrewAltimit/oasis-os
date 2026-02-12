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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_defaults() {
        let d = Divider::horizontal();
        assert_eq!(d.orientation, DividerOrientation::Horizontal);
        assert!(d.color.is_none());
        assert_eq!(d.thickness, 1);
    }

    #[test]
    fn vertical_defaults() {
        let d = Divider::vertical();
        assert_eq!(d.orientation, DividerOrientation::Vertical);
        assert!(d.color.is_none());
        assert_eq!(d.thickness, 1);
    }

    #[test]
    fn custom_color() {
        let mut d = Divider::horizontal();
        d.color = Some(Color::rgb(255, 0, 0));
        assert_eq!(d.color.unwrap(), Color::rgb(255, 0, 0));
    }

    #[test]
    fn custom_thickness() {
        let mut d = Divider::horizontal();
        d.thickness = 3;
        assert_eq!(d.thickness, 3);
    }

    #[test]
    fn orientation_equality() {
        assert_eq!(
            DividerOrientation::Horizontal,
            DividerOrientation::Horizontal
        );
        assert_ne!(DividerOrientation::Horizontal, DividerOrientation::Vertical);
    }

    #[test]
    fn orientation_debug() {
        let _ = format!("{:?}", DividerOrientation::Horizontal);
        let _ = format!("{:?}", DividerOrientation::Vertical);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::browser::test_utils::MockBackend;
    use crate::ui::context::DrawContext;
    use crate::ui::theme::Theme;
    use crate::ui::widget::Widget;

    #[test]
    fn measure_horizontal_returns_full_width() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let d = Divider::horizontal();
        let (w, h) = d.measure(&ctx, 300, 100);
        assert_eq!(w, 300);
        assert_eq!(h, d.thickness as u32);
    }

    #[test]
    fn draw_horizontal_emits_fill() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let d = Divider::horizontal();
            d.draw(&mut ctx, 0, 0, 200, 1).unwrap();
        }
        // draw_line for horizontal falls back to fill_rect.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_vertical_emits_fill() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let d = Divider::vertical();
            d.draw(&mut ctx, 0, 0, 1, 100).unwrap();
        }
        // draw_line for vertical falls back to fill_rect.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_custom_thickness() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Divider::horizontal();
            d.thickness = 3;
            d.draw(&mut ctx, 0, 0, 200, 3).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_custom_color() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Divider::horizontal();
            d.color = Some(Color::rgb(255, 0, 0));
            d.draw(&mut ctx, 0, 0, 200, 1).unwrap();
        }
        // Should not panic.
        assert!(backend.fill_rect_count() > 0);
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
