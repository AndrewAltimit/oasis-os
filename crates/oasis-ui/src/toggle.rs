//! Toggle switch widget.

use crate::color::lerp_color;
use crate::context::DrawContext;
use crate::widget::Widget;
use oasis_types::error::Result;

/// An on/off toggle switch with smooth animation.
///
/// Call [`animate`](Self::animate) each frame with elapsed milliseconds
/// to interpolate between states. The animation takes ~150ms.
///
/// # Example
///
/// ```ignore
/// let mut toggle = Toggle::new(false);
/// toggle.on = true;
/// // Animate over several frames (16ms each):
/// for _ in 0..10 {
///     toggle.animate(16);
/// }
/// assert!((toggle.progress - 1.0).abs() < 0.1);
/// ```
pub struct Toggle {
    /// Whether the toggle is on.
    pub on: bool,
    /// Animation progress (0.0 = off, 1.0 = on).
    pub progress: f32,
}

impl Toggle {
    /// Create a new toggle in the given state.
    pub fn new(on: bool) -> Self {
        Self {
            on,
            progress: if on { 1.0 } else { 0.0 },
        }
    }

    /// Animate toward the current `on` state.
    pub fn animate(&mut self, dt_ms: u32) {
        let target = if self.on { 1.0 } else { 0.0 };
        let speed = dt_ms as f32 / 150.0;
        if self.progress < target {
            self.progress = (self.progress + speed).min(1.0);
        } else if self.progress > target {
            self.progress = (self.progress - speed).max(0.0);
        }
    }

    /// Animate toward the current `on` state, respecting reduced-motion.
    ///
    /// When `reduced_motion` is `true`, the toggle snaps instantly to
    /// its target state instead of interpolating.
    pub fn animate_reduced(&mut self, dt_ms: u32, reduced_motion: bool) {
        if reduced_motion {
            self.progress = if self.on { 1.0 } else { 0.0 };
        } else {
            self.animate(dt_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_on() {
        let t = Toggle::new(true);
        assert!(t.on);
        assert!((t.progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn new_off() {
        let t = Toggle::new(false);
        assert!(!t.on);
        assert!((t.progress - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn animate_toward_on() {
        let mut t = Toggle::new(false);
        t.on = true;
        // Progress starts at 0.0, should move toward 1.0.
        t.animate(75); // 75/150 = 0.5
        assert!(t.progress > 0.0);
        assert!(t.progress <= 0.5 + f32::EPSILON);
    }

    #[test]
    fn animate_toward_off() {
        let mut t = Toggle::new(true);
        t.on = false;
        // Progress starts at 1.0, should move toward 0.0.
        t.animate(75);
        assert!(t.progress < 1.0);
        assert!(t.progress >= 0.5 - f32::EPSILON);
    }

    #[test]
    fn animate_completes() {
        let mut t = Toggle::new(false);
        t.on = true;
        // After enough time, progress should reach 1.0.
        for _ in 0..20 {
            t.animate(16);
        }
        assert!((t.progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn animate_no_overshoot() {
        let mut t = Toggle::new(false);
        t.on = true;
        t.animate(10000); // Huge dt
        assert!((t.progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn animate_zero_dt_no_change() {
        let mut t = Toggle::new(false);
        t.on = true;
        t.animate(0);
        assert!((t.progress - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn already_at_target() {
        let mut t = Toggle::new(true);
        let before = t.progress;
        t.animate(16);
        assert!((t.progress - before).abs() < f32::EPSILON);
    }

    // -- Reduced-motion tests --

    #[test]
    fn animate_reduced_snaps_to_on() {
        let mut t = Toggle::new(false);
        t.on = true;
        t.animate_reduced(1, true);
        assert!((t.progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn animate_reduced_snaps_to_off() {
        let mut t = Toggle::new(true);
        t.on = false;
        t.animate_reduced(1, true);
        assert!((t.progress - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn animate_reduced_false_is_normal() {
        let mut t = Toggle::new(false);
        t.on = true;
        t.animate_reduced(75, false);
        // Should behave like normal animate: partial progress.
        assert!(t.progress > 0.0);
        assert!(t.progress < 1.0);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_fixed_size() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Toggle::new(false);
        let (w, h) = t.measure(&ctx, 200, 100);
        assert_eq!(w, 28);
        assert_eq!(h, 16);
    }

    #[test]
    fn draw_off_state_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Toggle::new(false);
            t.draw(&mut ctx, 0, 0, 28, 16).unwrap();
        }
        // Should complete without panic.
        assert!(backend.calls.len() > 0);
    }

    #[test]
    fn draw_on_state_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Toggle::new(true);
            t.draw(&mut ctx, 0, 0, 28, 16).unwrap();
        }
        assert!(backend.calls.len() > 0);
    }

    #[test]
    fn draw_emits_fill_rects() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Toggle::new(true);
            t.draw(&mut ctx, 10, 20, 28, 16).unwrap();
        }
        // fill_rounded_rect -> fill_rect (track) + fill_circle -> fill_rect (thumb)
        assert!(
            backend.fill_rect_count() > 0,
            "draw should emit fill_rect calls for track and thumb"
        );
    }

    #[test]
    fn draw_animate_toward_on() {
        let mut t = Toggle::new(false);
        t.on = true;
        let before = t.progress;
        t.animate(50);
        assert!(
            t.progress > before,
            "progress should increase toward on state"
        );
    }

    #[test]
    fn draw_animate_toward_off() {
        let mut t = Toggle::new(true);
        t.on = false;
        let before = t.progress;
        t.animate(50);
        assert!(
            t.progress < before,
            "progress should decrease toward off state"
        );
    }

    #[test]
    fn progress_clamped() {
        let mut t = Toggle::new(false);
        t.on = true;
        for _ in 0..1000 {
            t.animate(100);
        }
        assert!(t.progress >= 0.0, "progress should not go below 0.0");
        assert!(t.progress <= 1.0, "progress should not exceed 1.0");

        t.on = false;
        for _ in 0..1000 {
            t.animate(100);
        }
        assert!(t.progress >= 0.0, "progress should not go below 0.0");
        assert!(t.progress <= 1.0, "progress should not exceed 1.0");
    }

    #[test]
    fn draw_partial_progress() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = Toggle::new(false);
            t.progress = 0.5;
            t.draw(&mut ctx, 0, 0, 28, 16).unwrap();
        }
        // Should draw without panic at midpoint progress.
        assert!(backend.fill_rect_count() > 0);
    }
}

impl Widget for Toggle {
    fn measure(&self, _ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        (28, 16)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let radius = h as u16 / 2;
        let bg = lerp_color(
            ctx.theme.toggle_track_off,
            ctx.theme.toggle_track_on,
            self.progress,
        );
        ctx.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;

        // Thumb circle.
        let thumb_r = (h as i32 / 2) - 2;
        let travel = w as i32 - h as i32;
        let thumb_x = x + h as i32 / 2 + (travel as f32 * self.progress) as i32;
        let thumb_y = y + h as i32 / 2;
        ctx.backend
            .fill_circle(thumb_x, thumb_y, thumb_r as u16, ctx.theme.toggle_thumb)?;
        Ok(())
    }
}
