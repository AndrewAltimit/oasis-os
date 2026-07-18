//! Slider widget for selecting a value within a range.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;
use oasis_types::input::Button;

/// Slider orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Left-to-right slider (default).
    Horizontal,
    /// Bottom-to-top slider.
    Vertical,
}

/// Default track height for horizontal sliders / track width for vertical.
const DEFAULT_TRACK_THICKNESS: u32 = 6;

/// Default total height for horizontal sliders.
const DEFAULT_HEIGHT: u32 = 20;

/// Default total width for vertical sliders.
const DEFAULT_WIDTH: u32 = 20;

/// A slider for selecting a numeric value within a range.
///
/// The internal `value` is always stored as a normalized `f32` in `0.0..=1.0`.
/// Use [`Slider::display_value`] to get the value mapped to the `min..=max`
/// range.
pub struct Slider {
    /// Normalized value (0.0 to 1.0).
    value: f32,
    /// Minimum display value.
    pub min: f32,
    /// Maximum display value.
    pub max: f32,
    /// Optional step increment (in display-value space).
    pub step: Option<f32>,
    /// Slider orientation.
    pub orientation: Orientation,
    /// Whether the slider is disabled (non-interactive).
    pub disabled: bool,
    /// Whether to show the current value as a text label.
    pub show_value_label: bool,
    /// Thumb diameter in pixels.
    pub thumb_size: u16,
}

impl Slider {
    /// Create a new horizontal slider for the given range.
    ///
    /// The initial value is set to `min`. The range must satisfy `min < max`;
    /// if not, `max` is clamped to at least `min + f32::EPSILON`.
    pub fn new(min: f32, max: f32) -> Self {
        let max = if max <= min { min + 1.0 } else { max };
        Self {
            value: 0.0,
            min,
            max,
            step: None,
            orientation: Orientation::Horizontal,
            disabled: false,
            show_value_label: false,
            thumb_size: 12,
        }
    }

    /// Set the current value in display-value space (clamped to `min..=max`).
    ///
    /// The value is normalized internally and snapped to the step grid when
    /// a step is configured.
    pub fn set_value(&mut self, display_val: f32) {
        let clamped = display_val.clamp(self.min, self.max);
        let range = self.max - self.min;
        let mut norm = if range > 0.0 {
            (clamped - self.min) / range
        } else {
            0.0
        };
        if let Some(s) = self.step {
            norm = snap_to_step(norm, s, range);
        }
        self.value = norm.clamp(0.0, 1.0);
    }

    /// Return the normalized value (0.0 to 1.0).
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Return the value mapped to the `min..=max` display range.
    pub fn display_value(&self) -> f32 {
        self.min + self.value * (self.max - self.min)
    }

    /// Builder: set the step increment.
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    /// Builder: set the orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Builder: enable or disable the value label.
    pub fn with_label(mut self, show: bool) -> Self {
        self.show_value_label = show;
        self
    }

    /// Handle a directional button press.
    ///
    /// Returns `true` if the slider consumed the event (value changed or the
    /// button matched the slider axis). Returns `false` for irrelevant
    /// buttons or when disabled.
    pub fn handle_key(&mut self, button: Button) -> bool {
        if self.disabled {
            return false;
        }

        let range = self.max - self.min;
        let delta = if let Some(s) = self.step {
            (s / range).clamp(0.0, 1.0)
        } else {
            // Default: 5% of the range per key press.
            0.05
        };

        match (self.orientation, button) {
            (Orientation::Horizontal, Button::Right) | (Orientation::Vertical, Button::Up) => {
                self.value = (self.value + delta).min(1.0);
                if let Some(s) = self.step {
                    self.value = snap_to_step(self.value, s, range);
                }
                true
            },
            (Orientation::Horizontal, Button::Left) | (Orientation::Vertical, Button::Down) => {
                self.value = (self.value - delta).max(0.0);
                if let Some(s) = self.step {
                    self.value = snap_to_step(self.value, s, range);
                }
                true
            },
            _ => false,
        }
    }
}

/// Snap a normalized value to the nearest step on the grid.
fn snap_to_step(norm: f32, step: f32, range: f32) -> f32 {
    if step <= 0.0 || range <= 0.0 {
        return norm;
    }
    let step_norm = step / range;
    if step_norm <= 0.0 {
        return norm;
    }
    (norm / step_norm).round() * step_norm
}

impl Widget for Slider {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        let label_extra = if self.show_value_label {
            let sample = format!("{:.0}", self.max);
            let (tw, _) = ctx.measure_text_sized(&sample, ctx.theme.font_size_xs);
            tw + ctx.theme.spacing_sm as u32
        } else {
            0
        };
        match self.orientation {
            Orientation::Horizontal => {
                let w = available_w.min(available_w);
                let h = DEFAULT_HEIGHT.max(self.thumb_size as u32);
                (w + label_extra, h)
            },
            Orientation::Vertical => {
                let w = DEFAULT_WIDTH.max(self.thumb_size as u32) + label_extra;
                let h = available_h.min(available_h);
                (w, h)
            },
        }
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // Dedicated slider slots; their defaults equal the fields the
        // slider historically borrowed (input_bg / accent / surface),
        // so untouched themes render identically.
        let accent = if self.disabled {
            ctx.theme.text_disabled
        } else {
            ctx.theme.slider_fill
        };
        let track_bg = if self.disabled {
            dim_color(ctx.theme.slider_track)
        } else {
            ctx.theme.slider_track
        };
        let thumb_fill = if self.disabled {
            dim_color(ctx.theme.slider_thumb)
        } else {
            ctx.theme.slider_thumb
        };
        let thumb_border = ctx.theme.interactive_border(self.disabled, true);

        let ts = self.thumb_size as u32;

        match self.orientation {
            Orientation::Horizontal => {
                // Reserve space for label on the right if enabled.
                let label_w = if self.show_value_label {
                    let sample = format!("{:.0}", self.max);
                    let (tw, _) = ctx
                        .backend
                        .measure_text_extents(&sample, ctx.theme.font_size_xs);
                    tw + ctx.theme.spacing_sm as u32
                } else {
                    0
                };
                let track_w = w.saturating_sub(label_w);

                // Track centered vertically.
                let track_y = y + layout::center(h, DEFAULT_TRACK_THICKNESS);
                let track_r = (DEFAULT_TRACK_THICKNESS as u16) / 2;

                // Track background.
                ctx.backend.fill_rounded_rect(
                    x,
                    track_y,
                    track_w,
                    DEFAULT_TRACK_THICKNESS,
                    track_r,
                    track_bg,
                )?;

                // Filled portion.
                let fill_w = (track_w as f32 * self.value) as u32;
                if fill_w > 0 {
                    ctx.backend.fill_rounded_rect(
                        x,
                        track_y,
                        fill_w,
                        DEFAULT_TRACK_THICKNESS,
                        track_r,
                        accent,
                    )?;
                }

                // Thumb.
                let thumb_cx = x + (track_w as f32 * self.value) as i32;
                let thumb_cy = y + h as i32 / 2;
                let thumb_r = ts as u16 / 2;
                // Border circle (slightly larger).
                ctx.backend
                    .fill_circle(thumb_cx, thumb_cy, thumb_r, thumb_border)?;
                // Inner fill (1px smaller for the border effect).
                if thumb_r > 1 {
                    ctx.backend
                        .fill_circle(thumb_cx, thumb_cy, thumb_r - 1, thumb_fill)?;
                }

                // Value label.
                if self.show_value_label {
                    let label = format_display_value(self.display_value());
                    let fs = ctx.theme.font_size_xs;
                    let th = ctx.backend.measure_text_height(fs);
                    let lx = x + track_w as i32 + ctx.theme.spacing_sm as i32;
                    let ly = y + layout::center(h, th);
                    let text_color = ctx.theme.interactive_text(self.disabled);
                    ctx.backend.draw_text(&label, lx, ly, fs, text_color)?;
                }
            },
            Orientation::Vertical => {
                // Track centered horizontally.
                let track_x = x + layout::center(w, DEFAULT_TRACK_THICKNESS);
                let track_r = (DEFAULT_TRACK_THICKNESS as u16) / 2;

                // Track background (full height).
                ctx.backend.fill_rounded_rect(
                    track_x,
                    y,
                    DEFAULT_TRACK_THICKNESS,
                    h,
                    track_r,
                    track_bg,
                )?;

                // Filled portion (from bottom).
                let fill_h = (h as f32 * self.value) as u32;
                if fill_h > 0 {
                    let fill_y = y + (h - fill_h) as i32;
                    ctx.backend.fill_rounded_rect(
                        track_x,
                        fill_y,
                        DEFAULT_TRACK_THICKNESS,
                        fill_h,
                        track_r,
                        accent,
                    )?;
                }

                // Thumb (value=0 at bottom, value=1 at top).
                let thumb_cx = x + w as i32 / 2;
                let thumb_cy = y + h as i32 - (h as f32 * self.value) as i32;
                let thumb_r = ts as u16 / 2;
                ctx.backend
                    .fill_circle(thumb_cx, thumb_cy, thumb_r, thumb_border)?;
                if thumb_r > 1 {
                    ctx.backend
                        .fill_circle(thumb_cx, thumb_cy, thumb_r - 1, thumb_fill)?;
                }

                // Value label (below the slider).
                if self.show_value_label {
                    let label = format_display_value(self.display_value());
                    let fs = ctx.theme.font_size_xs;
                    let tw = ctx.backend.measure_text(&label, fs);
                    let lx = x + layout::center(w, tw);
                    let ly = y + h as i32 + ctx.theme.spacing_xs as i32;
                    let text_color = ctx.theme.interactive_text(self.disabled);
                    ctx.backend.draw_text(&label, lx, ly, fs, text_color)?;
                }
            },
        }
        Ok(())
    }
}

/// Format a display value for the label. Shows integers without decimals and
/// fractional values with one decimal place.
fn format_display_value(val: f32) -> String {
    if (val - val.round()).abs() < 0.01 {
        format!("{:.0}", val)
    } else {
        format!("{:.1}", val)
    }
}

/// Dim a color for the disabled state by reducing its alpha.
fn dim_color(c: Color) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Construction --

    #[test]
    fn new_defaults() {
        let s = Slider::new(0.0, 100.0);
        assert!((s.value() - 0.0).abs() < f32::EPSILON);
        assert!((s.min - 0.0).abs() < f32::EPSILON);
        assert!((s.max - 100.0).abs() < f32::EPSILON);
        assert!(s.step.is_none());
        assert_eq!(s.orientation, Orientation::Horizontal);
        assert!(!s.disabled);
        assert!(!s.show_value_label);
        assert_eq!(s.thumb_size, 12);
    }

    #[test]
    fn new_invalid_range_clamped() {
        // max <= min should be corrected.
        let s = Slider::new(10.0, 5.0);
        assert!(s.max > s.min);
    }

    #[test]
    fn new_equal_range_clamped() {
        let s = Slider::new(5.0, 5.0);
        assert!(s.max > s.min);
    }

    // -- Value setting and clamping --

    #[test]
    fn set_value_within_range() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(50.0);
        assert!((s.display_value() - 50.0).abs() < 0.5);
        assert!((s.value() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn set_value_below_min_clamped() {
        let mut s = Slider::new(10.0, 20.0);
        s.set_value(5.0);
        assert!((s.value() - 0.0).abs() < f32::EPSILON);
        assert!((s.display_value() - 10.0).abs() < 0.5);
    }

    #[test]
    fn set_value_above_max_clamped() {
        let mut s = Slider::new(0.0, 50.0);
        s.set_value(999.0);
        assert!((s.value() - 1.0).abs() < f32::EPSILON);
        assert!((s.display_value() - 50.0).abs() < 0.5);
    }

    #[test]
    fn set_value_at_min() {
        let mut s = Slider::new(10.0, 20.0);
        s.set_value(10.0);
        assert!((s.value() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn set_value_at_max() {
        let mut s = Slider::new(10.0, 20.0);
        s.set_value(20.0);
        assert!((s.value() - 1.0).abs() < f32::EPSILON);
    }

    // -- Step snapping --

    #[test]
    fn step_snapping() {
        let mut s = Slider::new(0.0, 100.0).with_step(25.0);
        s.set_value(30.0);
        // Should snap to 25 (nearest step).
        assert!((s.display_value() - 25.0).abs() < 1.0);
    }

    #[test]
    fn step_snapping_to_max() {
        let mut s = Slider::new(0.0, 100.0).with_step(25.0);
        s.set_value(90.0);
        // Should snap to 100 (nearest step).
        assert!((s.display_value() - 100.0).abs() < 1.0);
    }

    #[test]
    fn step_snapping_exact() {
        let mut s = Slider::new(0.0, 10.0).with_step(5.0);
        s.set_value(5.0);
        assert!((s.display_value() - 5.0).abs() < 0.5);
    }

    // -- Orientation --

    #[test]
    fn orientation_horizontal_default() {
        let s = Slider::new(0.0, 1.0);
        assert_eq!(s.orientation, Orientation::Horizontal);
    }

    #[test]
    fn orientation_vertical_builder() {
        let s = Slider::new(0.0, 1.0).with_orientation(Orientation::Vertical);
        assert_eq!(s.orientation, Orientation::Vertical);
    }

    #[test]
    fn orientation_enum_equality() {
        assert_eq!(Orientation::Horizontal, Orientation::Horizontal);
        assert_ne!(Orientation::Horizontal, Orientation::Vertical);
    }

    // -- Builder methods --

    #[test]
    fn with_label_enables_label() {
        let s = Slider::new(0.0, 1.0).with_label(true);
        assert!(s.show_value_label);
    }

    #[test]
    fn with_step_sets_step() {
        let s = Slider::new(0.0, 100.0).with_step(10.0);
        assert!((s.step.unwrap() - 10.0).abs() < f32::EPSILON);
    }

    // -- Keyboard input --

    #[test]
    fn handle_key_right_increases_horizontal() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(50.0);
        let consumed = s.handle_key(Button::Right);
        assert!(consumed);
        assert!(s.value() > 0.5);
    }

    #[test]
    fn handle_key_left_decreases_horizontal() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(50.0);
        let consumed = s.handle_key(Button::Left);
        assert!(consumed);
        assert!(s.value() < 0.5);
    }

    #[test]
    fn handle_key_up_increases_vertical() {
        let mut s = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
        s.set_value(50.0);
        let consumed = s.handle_key(Button::Up);
        assert!(consumed);
        assert!(s.value() > 0.5);
    }

    #[test]
    fn handle_key_down_decreases_vertical() {
        let mut s = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
        s.set_value(50.0);
        let consumed = s.handle_key(Button::Down);
        assert!(consumed);
        assert!(s.value() < 0.5);
    }

    #[test]
    fn handle_key_irrelevant_not_consumed() {
        let mut s = Slider::new(0.0, 100.0);
        // Up/Down are irrelevant for a horizontal slider.
        assert!(!s.handle_key(Button::Up));
        assert!(!s.handle_key(Button::Down));
        assert!(!s.handle_key(Button::Confirm));
    }

    #[test]
    fn handle_key_disabled_not_consumed() {
        let mut s = Slider::new(0.0, 100.0);
        s.disabled = true;
        assert!(!s.handle_key(Button::Right));
    }

    #[test]
    fn handle_key_does_not_exceed_bounds() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(100.0);
        s.handle_key(Button::Right);
        assert!(s.value() <= 1.0);

        s.set_value(0.0);
        s.handle_key(Button::Left);
        assert!(s.value() >= 0.0);
    }

    #[test]
    fn handle_key_with_step() {
        let mut s = Slider::new(0.0, 100.0).with_step(10.0);
        s.set_value(50.0);
        s.handle_key(Button::Right);
        // Step is 10/100 = 0.1 in normalized space.
        assert!((s.display_value() - 60.0).abs() < 1.0);
    }

    // -- Display value formatting --

    #[test]
    fn display_value_at_min() {
        let s = Slider::new(10.0, 20.0);
        assert!((s.display_value() - 10.0).abs() < 0.1);
    }

    #[test]
    fn display_value_at_max() {
        let mut s = Slider::new(10.0, 20.0);
        s.set_value(20.0);
        assert!((s.display_value() - 20.0).abs() < 0.1);
    }

    #[test]
    fn format_display_value_integer() {
        assert_eq!(format_display_value(42.0), "42");
    }

    #[test]
    fn format_display_value_fractional() {
        assert_eq!(format_display_value(3.7), "3.7");
    }

    // -- Disabled state --

    #[test]
    fn disabled_state() {
        let mut s = Slider::new(0.0, 100.0);
        s.disabled = true;
        assert!(s.disabled);
        // Keyboard input should be rejected.
        assert!(!s.handle_key(Button::Right));
    }

    // -- dim_color helper --

    #[test]
    fn dim_color_halves_alpha() {
        let c = Color::rgba(100, 150, 200, 200);
        let d = dim_color(c);
        assert_eq!(d.r, 100);
        assert_eq!(d.g, 150);
        assert_eq!(d.b, 200);
        assert_eq!(d.a, 100);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::{self, MockBackend};
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_horizontal() {
        let s = Slider::new(0.0, 100.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, h) = s.measure(&ctx, 200, 100);
        assert_eq!(w, 200);
        assert!(h >= 12); // At least thumb_size.
    }

    #[test]
    fn measure_vertical() {
        let s = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, h) = s.measure(&ctx, 200, 150);
        assert!(w >= 12);
        assert_eq!(h, 150);
    }

    #[test]
    fn measure_with_label_wider() {
        let s_no_label = Slider::new(0.0, 100.0);
        let s_label = Slider::new(0.0, 100.0).with_label(true);
        let theme = Theme::dark();

        let mut b1 = MockBackend::new();
        let ctx1 = DrawContext::new(&mut b1, &theme);
        let (w1, _) = s_no_label.measure(&ctx1, 200, 100);

        let mut b2 = MockBackend::new();
        let ctx2 = DrawContext::new(&mut b2, &theme);
        let (w2, _) = s_label.measure(&ctx2, 200, 100);

        assert!(w2 > w1, "label should add width");
    }

    #[test]
    fn draw_horizontal_no_panic() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(50.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        // Track + fill + thumb border + thumb fill = at least 4 draw calls.
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_vertical_no_panic() {
        let mut s = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
        s.set_value(75.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 20, 150).unwrap();
        }
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_zero_value() {
        let s = Slider::new(0.0, 100.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        // fill_w = 0, so only track + thumb. At least 1 fill_rect for track.
        assert!(backend.fill_rect_count() >= 1);
    }

    #[test]
    fn draw_full_value() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(100.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_with_label_shows_text() {
        let mut s = Slider::new(0.0, 100.0).with_label(true);
        s.set_value(75.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.has_text("75"));
    }

    #[test]
    fn draw_disabled_no_panic() {
        let mut s = Slider::new(0.0, 100.0);
        s.set_value(50.0);
        s.disabled = true;
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            s.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.fill_rect_count() >= 1);
    }

    #[test]
    fn draw_all_themes_horizontal() {
        test_utils::test_draw_all_themes(|ctx| {
            let mut s = Slider::new(0.0, 100.0);
            s.set_value(50.0);
            s.draw(ctx, 0, 0, 200, 20).unwrap();
        });
    }

    #[test]
    fn draw_all_themes_vertical() {
        test_utils::test_draw_all_themes(|ctx| {
            let mut s = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
            s.set_value(50.0);
            s.draw(ctx, 0, 0, 20, 150).unwrap();
        });
    }

    #[test]
    fn draw_all_themes_with_label() {
        test_utils::test_draw_all_themes(|ctx| {
            let mut s = Slider::new(0.0, 100.0).with_label(true);
            s.set_value(33.0);
            s.draw(ctx, 0, 0, 200, 20).unwrap();
        });
    }

    // -- snap_to_step unit tests --

    #[test]
    fn snap_to_step_basic() {
        // Range 0-100, step 25 -> step_norm = 0.25
        let snapped = snap_to_step(0.3, 25.0, 100.0);
        assert!((snapped - 0.25).abs() < 0.01);
    }

    #[test]
    fn snap_to_step_exact() {
        let snapped = snap_to_step(0.5, 25.0, 100.0);
        assert!((snapped - 0.5).abs() < 0.01);
    }

    #[test]
    fn snap_to_step_zero_step() {
        let snapped = snap_to_step(0.33, 0.0, 100.0);
        assert!((snapped - 0.33).abs() < f32::EPSILON);
    }

    #[test]
    fn snap_to_step_zero_range() {
        let snapped = snap_to_step(0.5, 10.0, 0.0);
        assert!((snapped - 0.5).abs() < f32::EPSILON);
    }
}
