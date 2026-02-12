//! Toggle switch widget.

use crate::error::Result;
use crate::ui::color::lerp_color;
use crate::ui::context::DrawContext;
use crate::ui::widget::Widget;

/// An on/off toggle switch.
pub struct Toggle {
    pub on: bool,
    /// Animation progress (0.0 = off, 1.0 = on).
    pub progress: f32,
}

impl Toggle {
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

    // -- Draw / measure tests using MockBackend --

    use crate::browser::test_utils::MockBackend;
    use crate::ui::context::DrawContext;
    use crate::ui::theme::Theme;
    use crate::ui::widget::Widget;

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
        let bg = lerp_color(ctx.theme.scrollbar_track, ctx.theme.accent, self.progress);
        ctx.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;

        // Thumb circle.
        let thumb_r = (h as i32 / 2) - 2;
        let travel = w as i32 - h as i32;
        let thumb_x = x + h as i32 / 2 + (travel as f32 * self.progress) as i32;
        let thumb_y = y + h as i32 / 2;
        ctx.backend
            .fill_circle(thumb_x, thumb_y, thumb_r as u16, ctx.theme.text_on_accent)?;
        Ok(())
    }
}
