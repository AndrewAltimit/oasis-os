//! SpinBox widget: numeric input with increment/decrement buttons.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Height of the spin box widget.
const DEFAULT_HEIGHT: u32 = 22;

/// Width of each +/- button.
const BUTTON_WIDTH: u32 = 18;

/// A numeric input with +/- buttons for incrementing/decrementing a value.
pub struct SpinBox {
    /// Current value.
    pub value: f64,
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
    /// Step size for each increment/decrement.
    pub step: f64,
    /// Number of decimal places to display.
    pub decimals: u8,
    /// Whether the spin box is disabled.
    pub disabled: bool,
}

impl SpinBox {
    /// Create a new spin box with the given range.
    pub fn new(min: f64, max: f64) -> Self {
        let max = if max <= min { min + 1.0 } else { max };
        Self {
            value: min,
            min,
            max,
            step: 1.0,
            decimals: 0,
            disabled: false,
        }
    }

    /// Set the current value (clamped to range).
    pub fn set_value(&mut self, val: f64) {
        self.value = val.clamp(self.min, self.max);
    }

    /// Increment the value by one step.
    pub fn increment(&mut self) {
        if !self.disabled {
            self.value = (self.value + self.step).min(self.max);
        }
    }

    /// Decrement the value by one step.
    pub fn decrement(&mut self) {
        if !self.disabled {
            self.value = (self.value - self.step).max(self.min);
        }
    }

    /// Format the current value as a display string.
    pub fn display_value(&self) -> String {
        if self.decimals == 0 {
            format!("{}", self.value as i64)
        } else {
            format!("{:.prec$}", self.value, prec = self.decimals as usize)
        }
    }
}

impl Widget for SpinBox {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let h = DEFAULT_HEIGHT.max(text_h);
        let display = self.display_value();
        let text_w = ctx.backend.measure_text(&display, fs);
        let w = BUTTON_WIDTH + text_w + BUTTON_WIDTH + 8; // 4px padding each side
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let border = ctx.theme.interactive_border(self.disabled, false);
        let bg = ctx.theme.surface;

        // Background.
        ctx.backend.fill_rect(x, y, w, h, bg)?;
        ctx.backend.stroke_rect(x, y, w, h, 1, border)?;

        // Minus button.
        let btn_h = h;
        let minus_fg = ctx
            .theme
            .interactive_text(self.disabled || self.value <= self.min);
        ctx.backend
            .stroke_rect(x, y, BUTTON_WIDTH, btn_h, 1, border)?;
        let minus_x = x + layout::center(BUTTON_WIDTH, ctx.backend.measure_text("-", fs));
        let minus_y = y + layout::center(btn_h, text_h);
        ctx.backend.draw_text("-", minus_x, minus_y, fs, minus_fg)?;

        // Plus button.
        let plus_x = x + (w - BUTTON_WIDTH) as i32;
        let plus_fg = ctx
            .theme
            .interactive_text(self.disabled || self.value >= self.max);
        ctx.backend
            .stroke_rect(plus_x, y, BUTTON_WIDTH, btn_h, 1, border)?;
        let plus_tx = plus_x + layout::center(BUTTON_WIDTH, ctx.backend.measure_text("+", fs));
        let plus_ty = y + layout::center(btn_h, text_h);
        ctx.backend.draw_text("+", plus_tx, plus_ty, fs, plus_fg)?;

        // Value text (centered in remaining area).
        let display = self.display_value();
        let text_area_w = w.saturating_sub(BUTTON_WIDTH * 2);
        let text_x = x
            + BUTTON_WIDTH as i32
            + layout::center(text_area_w, ctx.backend.measure_text(&display, fs));
        let text_y = y + layout::center(h, text_h);
        ctx.backend.draw_text(
            &display,
            text_x,
            text_y,
            fs,
            ctx.theme.interactive_text(self.disabled),
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let s = SpinBox::new(0.0, 10.0);
        assert_eq!(s.value, 0.0);
        assert_eq!(s.min, 0.0);
        assert_eq!(s.max, 10.0);
        assert_eq!(s.step, 1.0);
        assert!(!s.disabled);
    }

    #[test]
    fn increment_clamps_to_max() {
        let mut s = SpinBox::new(0.0, 3.0);
        s.increment();
        assert_eq!(s.value, 1.0);
        s.increment();
        s.increment();
        s.increment();
        assert_eq!(s.value, 3.0);
    }

    #[test]
    fn decrement_clamps_to_min() {
        let mut s = SpinBox::new(0.0, 10.0);
        s.decrement();
        assert_eq!(s.value, 0.0);
    }

    #[test]
    fn disabled_ignores_input() {
        let mut s = SpinBox::new(0.0, 10.0);
        s.disabled = true;
        s.increment();
        assert_eq!(s.value, 0.0);
        s.decrement();
        assert_eq!(s.value, 0.0);
    }

    #[test]
    fn set_value_clamps() {
        let mut s = SpinBox::new(0.0, 10.0);
        s.set_value(20.0);
        assert_eq!(s.value, 10.0);
        s.set_value(-5.0);
        assert_eq!(s.value, 0.0);
    }

    #[test]
    fn display_value_decimals() {
        let mut s = SpinBox::new(0.0, 10.0);
        s.set_value(3.0);
        assert_eq!(s.display_value(), "3");
        s.decimals = 2;
        assert_eq!(s.display_value(), "3.00");
    }

    #[test]
    fn min_max_inverted_corrected() {
        let s = SpinBox::new(10.0, 5.0);
        assert!(s.max > s.min);
    }

    #[test]
    fn step_works() {
        let mut s = SpinBox::new(0.0, 10.0);
        s.step = 2.5;
        s.increment();
        assert_eq!(s.value, 2.5);
        s.increment();
        assert_eq!(s.value, 5.0);
    }

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn draw_shows_value() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = SpinBox::new(0.0, 10.0);
            s.set_value(5.0);
            s.draw(&mut ctx, 0, 0, 120, 22).unwrap();
        }
        assert!(backend.has_text("5"));
        assert!(backend.has_text("-"));
        assert!(backend.has_text("+"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let s = SpinBox::new(0.0, 100.0);
            s.draw(ctx, 0, 0, 120, 22).unwrap();
        });
    }
}
