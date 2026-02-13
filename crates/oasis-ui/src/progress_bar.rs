//! ProgressBar widget.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Progress bar visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStyle {
    /// Horizontal bar.
    Bar,
    /// Circular/radial indicator.
    Circular,
    /// Animated indeterminate progress.
    Indeterminate,
}

/// A progress indicator.
pub struct ProgressBar {
    /// Progress value (0.0 to 1.0).
    pub value: f32,
    /// Visual style variant.
    pub style: ProgressStyle,
    /// Whether to show percentage label.
    pub show_label: bool,
}

impl ProgressBar {
    /// Create a new progress bar (value clamped to 0.0-1.0).
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            style: ProgressStyle::Bar,
            show_label: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_value() {
        let p = ProgressBar::new(0.5);
        assert!((p.value - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn new_clamps_above_one() {
        let p = ProgressBar::new(1.5);
        assert!((p.value - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn new_clamps_below_zero() {
        let p = ProgressBar::new(-0.5);
        assert!((p.value - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn new_defaults() {
        let p = ProgressBar::new(0.75);
        assert_eq!(p.style, ProgressStyle::Bar);
        assert!(!p.show_label);
    }

    #[test]
    fn style_variants() {
        assert_ne!(ProgressStyle::Bar, ProgressStyle::Circular);
        assert_ne!(ProgressStyle::Circular, ProgressStyle::Indeterminate);
    }

    #[test]
    fn zero_progress() {
        let p = ProgressBar::new(0.0);
        assert!((p.value - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn full_progress() {
        let p = ProgressBar::new(1.0);
        assert!((p.value - 1.0).abs() < f32::EPSILON);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_fixed_dimensions() {
        let p = ProgressBar::new(0.5);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, h) = p.measure(&ctx, 200, 100);
        // Bar style: returns (available_w, 8).
        assert_eq!(w, 200);
        assert_eq!(h, 8);
    }

    #[test]
    fn draw_bar_fill_proportional() {
        let p = ProgressBar::new(0.5);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            p.draw(&mut ctx, 0, 0, 100, 8).unwrap();
        }
        // At least 2 fill_rect calls: one for track, one for fill.
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_zero_progress() {
        let p = ProgressBar::new(0.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            p.draw(&mut ctx, 0, 0, 100, 8).unwrap();
        }
        // fill_w = 0, so only the track is drawn (1 fill_rect).
        assert!(backend.fill_rect_count() >= 1);
    }

    #[test]
    fn draw_full_progress() {
        let p = ProgressBar::new(1.0);
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            p.draw(&mut ctx, 0, 0, 100, 8).unwrap();
        }
        // Track + full fill bar = at least 2 fill_rect calls.
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn value_clamped_below_zero() {
        let p = ProgressBar::new(-0.5);
        assert!((p.value - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn value_clamped_above_one() {
        let p = ProgressBar::new(1.5);
        assert!((p.value - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn label_shows_percentage() {
        let mut p = ProgressBar::new(0.75);
        p.show_label = true;
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            p.draw(&mut ctx, 0, 0, 200, 16).unwrap();
        }
        // The label should contain "75%".
        assert!(backend.has_text("75%"));
    }

    #[test]
    fn draw_circular_style() {
        let mut p = ProgressBar::new(0.5);
        p.style = ProgressStyle::Circular;
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            // Should not panic for circular style.
            p.draw(&mut ctx, 0, 0, 24, 24).unwrap();
        }
        // stroke_circle falls back to fill_circle then fill_rect in mock.
        assert!(backend.fill_rect_count() >= 1);
    }
}

impl Widget for ProgressBar {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        match self.style {
            ProgressStyle::Bar | ProgressStyle::Indeterminate => (available_w, 8),
            ProgressStyle::Circular => (24, 24),
        }
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        match self.style {
            ProgressStyle::Bar | ProgressStyle::Indeterminate => {
                let radius = h as u16 / 2;
                // Track.
                ctx.backend
                    .fill_rounded_rect(x, y, w, h, radius, ctx.theme.scrollbar_track)?;
                // Fill.
                let fill_w = (w as f32 * self.value) as u32;
                if fill_w > 0 {
                    ctx.backend
                        .fill_rounded_rect(x, y, fill_w, h, radius, ctx.theme.accent)?;
                }
                // Label.
                if self.show_label {
                    let pct = format!("{}%", (self.value * 100.0) as u32);
                    let fs = ctx.theme.font_size_xs;
                    let tw = ctx.backend.measure_text(&pct, fs);
                    let th = ctx.backend.measure_text_height(fs);
                    let tx = x + layout::center(w, tw);
                    let ty = y + layout::center(h, th);
                    ctx.backend
                        .draw_text(&pct, tx, ty, fs, ctx.theme.text_primary)?;
                }
            },
            ProgressStyle::Circular => {
                let r = (h.min(w) / 2) as u16;
                let cx = x + r as i32;
                let cy = y + r as i32;
                // Background circle.
                ctx.backend
                    .stroke_circle(cx, cy, r, 2, ctx.theme.scrollbar_track)?;
                // Progress arc approximated as a partial circle overlay.
                // Full circle at 100%.
                if self.value >= 0.99 {
                    ctx.backend.stroke_circle(cx, cy, r, 2, ctx.theme.accent)?;
                }
            },
        }
        Ok(())
    }
}
