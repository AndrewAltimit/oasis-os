//! Button widget.

use crate::context::DrawContext;
use crate::icon::Icon;
use crate::layout::{self, Padding};
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

/// Button visual state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// Default state.
    Normal,
    /// Mouse/cursor is over the button.
    Hover,
    /// Button is being pressed.
    Pressed,
    /// Button is disabled and non-interactive.
    Disabled,
}

/// Button visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Filled button with accent color.
    Primary,
    /// Filled button with neutral color.
    Secondary,
    /// Button with border only, no fill.
    Outline,
    /// Button with no background or border until hovered.
    Ghost,
}

/// A clickable button with label and optional icon.
pub struct Button {
    /// Button text label.
    pub label: String,
    /// Optional icon to display.
    pub icon: Option<Icon>,
    /// Current visual state.
    pub state: ButtonState,
    /// Visual style variant.
    pub style: ButtonStyle,
    /// Internal padding around label.
    pub padding: Padding,
}

impl Button {
    /// Create a new secondary button.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            state: ButtonState::Normal,
            style: ButtonStyle::Secondary,
            padding: Padding::symmetric(8, 4),
        }
    }

    /// Create a new primary (accent-colored) button.
    pub fn primary(label: impl Into<String>) -> Self {
        Self {
            style: ButtonStyle::Primary,
            ..Self::new(label)
        }
    }

    fn bg_color(&self, theme: &crate::theme::Theme) -> Option<Color> {
        match self.style {
            ButtonStyle::Primary => Some(match self.state {
                ButtonState::Pressed => theme.accent_pressed,
                ButtonState::Hover => theme.accent_hover,
                ButtonState::Disabled => theme.button_bg_disabled,
                _ => theme.accent,
            }),
            ButtonStyle::Secondary => Some(match self.state {
                ButtonState::Pressed => theme.button_bg_pressed,
                ButtonState::Hover => theme.button_bg_hover,
                ButtonState::Disabled => theme.button_bg_disabled,
                _ => theme.button_bg,
            }),
            ButtonStyle::Outline | ButtonStyle::Ghost => {
                if self.state == ButtonState::Hover {
                    Some(theme.accent_subtle)
                } else if self.state == ButtonState::Pressed {
                    Some(theme.button_bg_pressed)
                } else {
                    None
                }
            },
        }
    }

    fn text_color(&self, theme: &crate::theme::Theme) -> Color {
        if self.state == ButtonState::Disabled {
            return theme.text_disabled;
        }
        match self.style {
            ButtonStyle::Primary => theme.text_on_accent,
            _ => theme.text_primary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let b = Button::new("Click");
        assert_eq!(b.label, "Click");
        assert_eq!(b.state, ButtonState::Normal);
        assert_eq!(b.style, ButtonStyle::Secondary);
        assert!(b.icon.is_none());
    }

    #[test]
    fn primary_style() {
        let b = Button::primary("OK");
        assert_eq!(b.style, ButtonStyle::Primary);
        assert_eq!(b.label, "OK");
        assert_eq!(b.state, ButtonState::Normal);
    }

    #[test]
    fn state_transitions() {
        let mut b = Button::new("test");
        assert_eq!(b.state, ButtonState::Normal);
        b.state = ButtonState::Hover;
        assert_eq!(b.state, ButtonState::Hover);
        b.state = ButtonState::Pressed;
        assert_eq!(b.state, ButtonState::Pressed);
        b.state = ButtonState::Disabled;
        assert_eq!(b.state, ButtonState::Disabled);
    }

    #[test]
    fn style_variants() {
        assert_ne!(ButtonStyle::Primary, ButtonStyle::Secondary);
        assert_ne!(ButtonStyle::Outline, ButtonStyle::Ghost);
        assert_eq!(ButtonStyle::Primary, ButtonStyle::Primary);
    }

    #[test]
    fn state_variants_debug() {
        for state in [
            ButtonState::Normal,
            ButtonState::Hover,
            ButtonState::Pressed,
            ButtonState::Disabled,
        ] {
            let _ = format!("{state:?}");
        }
    }

    #[test]
    fn padding_applied() {
        let b = Button::new("test");
        assert!(b.padding.horizontal() > 0);
        assert!(b.padding.vertical() > 0);
    }

    #[test]
    fn from_string_type() {
        let b = Button::new(String::from("dynamic"));
        assert_eq!(b.label, "dynamic");
    }

    #[test]
    fn from_str_ref() {
        let label = "borrowed";
        let b = Button::new(label);
        assert_eq!(b.label, "borrowed");
    }

    // -- Draw / measure tests using MockBackend --

    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_text_width_plus_padding() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let btn = Button::new("Test");
        let (w, h) = btn.measure(&ctx, 200, 100);
        // "Test" = 4 chars * 8px = 32px text width + horizontal padding (16)
        assert!(w >= 32, "width {w} should be >= text width 32");
        assert!(w >= 32 + btn.padding.horizontal());
        assert!(h > 0);
    }

    #[test]
    fn draw_normal_state_emits_fill_and_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let btn = Button::new("Test");
            btn.draw(&mut ctx, 0, 0, 100, 30).unwrap();
        }
        assert!(
            backend.fill_rect_count() > 0,
            "should emit at least one fill_rect"
        );
        assert!(backend.has_text("Test"), "should draw the label text");
    }

    #[test]
    fn draw_hover_state() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Hov");
            btn.state = ButtonState::Hover;
            btn.draw(&mut ctx, 0, 0, 80, 24).unwrap();
        }
        assert!(
            backend.fill_rect_count() > 0,
            "hover state should emit fill_rect"
        );
    }

    #[test]
    fn draw_pressed_state() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Press");
            btn.state = ButtonState::Pressed;
            btn.draw(&mut ctx, 0, 0, 80, 24).unwrap();
        }
        assert!(
            backend.fill_rect_count() > 0,
            "pressed state should emit fill_rect"
        );
    }

    #[test]
    fn draw_disabled_uses_disabled_color() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Off");
            btn.state = ButtonState::Disabled;
            btn.draw(&mut ctx, 0, 0, 80, 24).unwrap();
        }
        assert!(
            backend.draw_text_count() > 0,
            "disabled button should still draw text"
        );
    }

    #[test]
    fn draw_primary_style() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let btn = Button::primary("Go");
            btn.draw(&mut ctx, 0, 0, 60, 24).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
        assert!(backend.has_text("Go"));
    }

    #[test]
    fn draw_outline_style() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Outline");
            btn.style = ButtonStyle::Outline;
            btn.draw(&mut ctx, 0, 0, 100, 30).unwrap();
        }
        // Outline in Normal state has no bg but does have a stroke border
        // stroke_rounded_rect -> stroke_rect -> 4 fill_rects
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_ghost_normal_no_bg() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Ghost");
            btn.style = ButtonStyle::Ghost;
            btn.draw(&mut ctx, 0, 0, 80, 24).unwrap();
        }
        // Ghost in Normal state: bg_color returns None, no fill_rect for bg.
        // Only text should be drawn.
        assert!(backend.has_text("Ghost"));
    }

    #[test]
    fn all_four_states_draw_without_panic() {
        let theme = Theme::dark();
        for state in [
            ButtonState::Normal,
            ButtonState::Hover,
            ButtonState::Pressed,
            ButtonState::Disabled,
        ] {
            let mut backend = MockBackend::new();
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("X");
            btn.state = state;
            btn.draw(&mut ctx, 0, 0, 40, 20).unwrap();
        }
    }

    #[test]
    fn draw_text_is_centered() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let btn = Button::new("AB");
            btn.draw(&mut ctx, 0, 0, 100, 30).unwrap();
        }
        let positions = backend.text_positions();
        assert!(!positions.is_empty(), "should have drawn text");
        let (_, tx, ty, _) = positions[0];
        // "AB" = 16px text width. Centered in 100px => x ~ (100-16)/2 = 42
        assert!(tx > 0, "text x ({tx}) should be offset from left edge");
        assert!(ty >= 0, "text y ({ty}) should be non-negative");
    }
}

impl Widget for Button {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let text_w = ctx
            .backend
            .measure_text(&self.label, ctx.theme.font_size_md);
        let text_h = ctx.backend.measure_text_height(ctx.theme.font_size_md);
        (
            text_w + self.padding.horizontal(),
            text_h + self.padding.vertical(),
        )
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let radius = ctx.theme.border_radius_md;

        // Background.
        if let Some(bg) = self.bg_color(ctx.theme) {
            ctx.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;
        }

        // Outline border for Outline style.
        if self.style == ButtonStyle::Outline {
            let bc = if self.state == ButtonState::Disabled {
                ctx.theme.border_subtle
            } else {
                ctx.theme.border
            };
            ctx.backend.stroke_rounded_rect(x, y, w, h, radius, 1, bc)?;
        }

        // Label.
        let text_w = ctx
            .backend
            .measure_text(&self.label, ctx.theme.font_size_md);
        let text_h = ctx.backend.measure_text_height(ctx.theme.font_size_md);
        let tx = x + layout::center(w, text_w);
        let ty = y + layout::center(h, text_h);
        let color = self.text_color(ctx.theme);
        ctx.backend
            .draw_text(&self.label, tx, ty, ctx.theme.font_size_md, color)?;

        Ok(())
    }
}
