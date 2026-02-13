//! Animation primitives: easing functions and tweens.

use crate::color::lerp_color;
use oasis_types::backend::Color;

/// Standard easing functions.
///
/// Input `t` is clamped to `[0.0, 1.0]`. Output is the eased value.
pub mod easing {
    /// Linear easing (no acceleration).
    pub fn linear(t: f32) -> f32 {
        t.clamp(0.0, 1.0)
    }

    /// Quadratic ease-in (slow start).
    pub fn ease_in_quad(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t
    }

    /// Quadratic ease-out (slow end).
    pub fn ease_out_quad(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * (2.0 - t)
    }

    /// Quadratic ease-in-out (slow start and end).
    pub fn ease_in_out_quad(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        }
    }

    /// Cubic ease-out (slow end, sharper than quad).
    pub fn ease_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let t1 = t - 1.0;
        t1 * t1 * t1 + 1.0
    }

    /// Cubic ease-in-out (smooth start and end).
    pub fn ease_in_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            (t - 1.0) * (2.0 * t - 2.0) * (2.0 * t - 2.0) + 1.0
        }
    }

    /// Elastic ease-out (bounces past target then settles).
    pub fn ease_out_elastic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        let p = 0.3_f32;
        (2.0_f32.powf(-10.0 * t) * ((t - p / 4.0) * (2.0 * core::f32::consts::PI / p)).sin()) + 1.0
    }

    /// Bounce ease-out (bounces multiple times before settling).
    pub fn ease_out_bounce(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 1.0 / 2.75 {
            7.5625 * t * t
        } else if t < 2.0 / 2.75 {
            let t = t - 1.5 / 2.75;
            7.5625 * t * t + 0.75
        } else if t < 2.5 / 2.75 {
            let t = t - 2.25 / 2.75;
            7.5625 * t * t + 0.9375
        } else {
            let t = t - 2.625 / 2.75;
            7.5625 * t * t + 0.984375
        }
    }
}

/// A running animation that interpolates between two values.
pub struct Tween {
    /// Starting value.
    pub start: f32,
    /// Target value.
    pub end: f32,
    /// Total duration in milliseconds.
    pub duration_ms: u32,
    /// Elapsed time in milliseconds.
    pub elapsed_ms: u32,
    /// Easing function to apply.
    pub easing: fn(f32) -> f32,
}

impl Tween {
    /// Create a new tween animation.
    pub fn new(start: f32, end: f32, duration_ms: u32, easing: fn(f32) -> f32) -> Self {
        Self {
            start,
            end,
            duration_ms,
            elapsed_ms: 0,
            easing,
        }
    }

    /// Advance by `dt_ms` and return the current interpolated value.
    pub fn tick(&mut self, dt_ms: u32) -> f32 {
        self.elapsed_ms = (self.elapsed_ms + dt_ms).min(self.duration_ms);
        let t = if self.duration_ms > 0 {
            self.elapsed_ms as f32 / self.duration_ms as f32
        } else {
            1.0
        };
        let eased = (self.easing)(t);
        self.start + (self.end - self.start) * eased
    }

    /// Check if the animation has completed.
    pub fn is_finished(&self) -> bool {
        self.elapsed_ms >= self.duration_ms
    }

    /// Current value without advancing time.
    pub fn value(&self) -> f32 {
        let t = if self.duration_ms > 0 {
            self.elapsed_ms as f32 / self.duration_ms as f32
        } else {
            1.0
        };
        let eased = (self.easing)(t);
        self.start + (self.end - self.start) * eased
    }
}

/// Tween between two colors over time.
pub struct ColorTween {
    /// Starting color.
    pub start: Color,
    /// Target color.
    pub end: Color,
    tween: Tween,
}

impl ColorTween {
    /// Create a new color tween animation.
    pub fn new(start: Color, end: Color, duration_ms: u32, easing: fn(f32) -> f32) -> Self {
        Self {
            start,
            end,
            tween: Tween::new(0.0, 1.0, duration_ms, easing),
        }
    }

    /// Advance by `dt_ms` and return the current interpolated color.
    pub fn tick(&mut self, dt_ms: u32) -> Color {
        let t = self.tween.tick(dt_ms);
        lerp_color(self.start, self.end, t)
    }

    /// Check if the animation has completed.
    pub fn is_finished(&self) -> bool {
        self.tween.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_linear() {
        let mut tw = Tween::new(0.0, 100.0, 100, easing::linear);
        assert_eq!(tw.tick(0), 0.0);
        assert_eq!(tw.tick(50), 50.0);
        assert_eq!(tw.tick(50), 100.0);
        assert!(tw.is_finished());
    }

    #[test]
    fn tween_eased() {
        let mut tw = Tween::new(0.0, 100.0, 100, easing::ease_in_quad);
        let v = tw.tick(50);
        // ease_in_quad at t=0.5 is 0.25, so value should be 25.
        assert!((v - 25.0).abs() < 0.01);
    }

    #[test]
    fn easing_bounds() {
        assert_eq!(easing::linear(0.0), 0.0);
        assert_eq!(easing::linear(1.0), 1.0);
        assert_eq!(easing::ease_out_quad(0.0), 0.0);
        assert_eq!(easing::ease_out_quad(1.0), 1.0);
        assert_eq!(easing::ease_out_cubic(1.0), 1.0);
        assert_eq!(easing::ease_in_out_cubic(0.0), 0.0);
    }

    #[test]
    fn color_tween_works() {
        let mut ct = ColorTween::new(
            Color::rgb(0, 0, 0),
            Color::rgb(200, 100, 50),
            100,
            easing::linear,
        );
        let c = ct.tick(50);
        assert_eq!(c.r, 100);
        assert_eq!(c.g, 50);
        assert_eq!(c.b, 25);
    }
}
