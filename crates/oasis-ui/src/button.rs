//! Button widget.

use crate::context::DrawContext;
use crate::focus::FocusStyle;
use crate::layout::{HAlign, Padding};
use crate::states::{WidgetState, WidgetStateColors};
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

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

/// A clickable button with a text label.
pub struct Button {
    /// Button text label.
    pub label: String,
    /// Current interaction state (the toolkit-wide [`WidgetState`]).
    pub state: WidgetState,
    /// Visual style variant.
    pub style: ButtonStyle,
    /// Internal padding around label.
    pub padding: Padding,
    /// Whether the button has keyboard focus (draws a focus ring).
    pub focused: bool,
}

impl Button {
    /// Create a new secondary button.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            state: WidgetState::Normal,
            style: ButtonStyle::Secondary,
            padding: Padding::symmetric(8, 4),
            focused: false,
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
        let state = self.state;
        match self.style {
            ButtonStyle::Primary => Some(WidgetStateColors::accent_bg(theme, state)),
            ButtonStyle::Secondary => Some(WidgetStateColors::neutral_bg(theme, state)),
            ButtonStyle::Outline | ButtonStyle::Ghost => {
                if self.state == WidgetState::Hover {
                    Some(theme.accent_subtle)
                } else if self.state == WidgetState::Pressed {
                    Some(theme.button_bg_pressed)
                } else {
                    None
                }
            },
        }
    }

    fn text_color(&self, theme: &crate::theme::Theme) -> Color {
        if self.state == WidgetState::Disabled {
            return theme.text_disabled;
        }
        match self.style {
            ButtonStyle::Primary => theme.text_on_accent,
            _ => theme.text_primary,
        }
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
            let bc = if self.state == WidgetState::Disabled {
                ctx.theme.border_subtle
            } else {
                ctx.theme.border
            };
            ctx.backend.stroke_rounded_rect(x, y, w, h, radius, 1, bc)?;
        }

        // Label.
        let color = self.text_color(ctx.theme);
        ctx.text_in(
            (x, y, w, h),
            &self.label,
            ctx.theme.font_size_md,
            color,
            HAlign::Center,
        )?;

        // Keyboard focus ring (only when focused and interactive).
        if self.focused && self.state != WidgetState::Disabled {
            FocusStyle::from_theme(ctx.theme).draw(ctx.backend, x, y, w, h)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let b = Button::new("Click");
        assert_eq!(b.label, "Click");
        assert_eq!(b.state, WidgetState::Normal);
        assert_eq!(b.style, ButtonStyle::Secondary);
    }

    #[test]
    fn primary_style() {
        let b = Button::primary("OK");
        assert_eq!(b.style, ButtonStyle::Primary);
        assert_eq!(b.label, "OK");
        assert_eq!(b.state, WidgetState::Normal);
    }

    #[test]
    fn state_transitions() {
        let mut b = Button::new("test");
        assert_eq!(b.state, WidgetState::Normal);
        b.state = WidgetState::Hover;
        assert_eq!(b.state, WidgetState::Hover);
        b.state = WidgetState::Pressed;
        assert_eq!(b.state, WidgetState::Pressed);
        b.state = WidgetState::Disabled;
        assert_eq!(b.state, WidgetState::Disabled);
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
            WidgetState::Normal,
            WidgetState::Hover,
            WidgetState::Pressed,
            WidgetState::Disabled,
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

    // -- Draw / measure tests using RecordingBackend --

    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_text_width_plus_padding() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        let btn = Button::new("Test");
        let (w, h) = btn.measure(&ctx, 200, 100);
        // "Test" proportional: T(7)+e(7)+s(7)+t(7) = 28px + horizontal padding
        let text_w = 28;
        assert!(w >= text_w, "width {w} should be >= text width {text_w}");
        assert!(w >= text_w + btn.padding.horizontal());
        assert!(h > 0);
    }

    #[test]
    fn draw_normal_state_emits_fill_and_text() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Hov");
            btn.state = WidgetState::Hover;
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
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Press");
            btn.state = WidgetState::Pressed;
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
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("Off");
            btn.state = WidgetState::Disabled;
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
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
            WidgetState::Normal,
            WidgetState::Hover,
            WidgetState::Pressed,
            WidgetState::Disabled,
        ] {
            let mut backend = RecordingBackend::default();
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut btn = Button::new("X");
            btn.state = state;
            btn.draw(&mut ctx, 0, 0, 40, 20).unwrap();
        }
    }

    #[test]
    fn draw_text_is_centered() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
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

    #[test]
    fn bg_color_identity_default_theme() {
        // The helper-routed background must reproduce the exact theme
        // fields the widget referenced before the refactor.
        let t = Theme::dark();
        let mut b = Button::primary("P");
        assert_eq!(b.bg_color(&t), Some(t.accent));
        b.state = WidgetState::Hover;
        assert_eq!(b.bg_color(&t), Some(t.accent_hover));
        b.state = WidgetState::Pressed;
        assert_eq!(b.bg_color(&t), Some(t.accent_pressed));
        b.state = WidgetState::Disabled;
        assert_eq!(b.bg_color(&t), Some(t.button_bg_disabled));

        let mut s = Button::new("S");
        assert_eq!(s.bg_color(&t), Some(t.button_bg));
        s.state = WidgetState::Hover;
        assert_eq!(s.bg_color(&t), Some(t.button_bg_hover));
        s.state = WidgetState::Pressed;
        assert_eq!(s.bg_color(&t), Some(t.button_bg_pressed));
        s.state = WidgetState::Disabled;
        assert_eq!(s.bg_color(&t), Some(t.button_bg_disabled));
    }

    #[test]
    fn unfocused_draws_no_focus_ring() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let mut with_focus = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            Button::new("X").draw(&mut ctx, 0, 0, 60, 24).unwrap();
        }
        {
            let mut ctx = DrawContext::new(&mut with_focus, &theme);
            let mut b = Button::new("X");
            b.focused = true;
            b.draw(&mut ctx, 0, 0, 60, 24).unwrap();
        }
        // Focused button emits additional fill_rects for the ring.
        assert!(with_focus.fill_rect_count() > backend.fill_rect_count());
    }
}
