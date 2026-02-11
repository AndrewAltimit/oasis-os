//! Button widget.

use crate::backend::Color;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::icon::Icon;
use crate::ui::layout::{self, Padding};
use crate::ui::widget::Widget;

/// Button visual state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hover,
    Pressed,
    Disabled,
}

/// Button visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Outline,
    Ghost,
}

/// A clickable button with label and optional icon.
pub struct Button {
    pub label: String,
    pub icon: Option<Icon>,
    pub state: ButtonState,
    pub style: ButtonStyle,
    pub padding: Padding,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            state: ButtonState::Normal,
            style: ButtonStyle::Secondary,
            padding: Padding::symmetric(8, 4),
        }
    }

    pub fn primary(label: impl Into<String>) -> Self {
        Self {
            style: ButtonStyle::Primary,
            ..Self::new(label)
        }
    }

    fn bg_color(&self, theme: &crate::ui::theme::Theme) -> Option<Color> {
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

    fn text_color(&self, theme: &crate::ui::theme::Theme) -> Color {
        if self.state == ButtonState::Disabled {
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
