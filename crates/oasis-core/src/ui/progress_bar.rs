//! ProgressBar widget.

use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout;
use crate::ui::widget::Widget;

/// Progress bar visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStyle {
    Bar,
    Circular,
    Indeterminate,
}

/// A progress indicator.
pub struct ProgressBar {
    pub value: f32,
    pub style: ProgressStyle,
    pub show_label: bool,
}

impl ProgressBar {
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
