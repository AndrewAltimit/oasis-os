//! Standard easing curves shared by the UI tweens and WM window animations.
//!
//! Every function takes a progress value `t`, clamps it to `[0.0, 1.0]`, and
//! returns the eased progress. All curves map `0.0 -> 0.0` and `1.0 -> 1.0`;
//! `ease_out_elastic` overshoots past `1.0` in between.
//!
//! `oasis_ui::animation::easing` and `oasis_wm::animation::easing` re-export
//! this module.

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

/// Elastic ease-out (overshoots the target, then settles).
pub fn ease_out_elastic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let p = 0.3_f32;
    let two_pi_over_p = 2.0 * core::f32::consts::PI / p;
    2.0_f32.powf(-10.0 * t) * ((t - p / 4.0) * two_pi_over_p).sin() + 1.0
}

/// Bounce ease-out (bounces several times before settling).
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

#[cfg(test)]
mod tests {
    use super::*;

    type Curve = fn(f32) -> f32;

    const ALL: [(&str, Curve); 8] = [
        ("linear", linear),
        ("ease_in_quad", ease_in_quad),
        ("ease_out_quad", ease_out_quad),
        ("ease_in_out_quad", ease_in_out_quad),
        ("ease_out_cubic", ease_out_cubic),
        ("ease_in_out_cubic", ease_in_out_cubic),
        ("ease_out_elastic", ease_out_elastic),
        ("ease_out_bounce", ease_out_bounce),
    ];

    #[test]
    fn endpoints_are_fixed() {
        for (name, f) in ALL {
            assert!(f(0.0).abs() < 1e-5, "{name}(0) = {}", f(0.0));
            assert!((f(1.0) - 1.0).abs() < 1e-5, "{name}(1) = {}", f(1.0));
        }
    }

    #[test]
    fn input_is_clamped() {
        for (name, f) in ALL {
            assert_eq!(f(-1.0), f(0.0), "{name} below range");
            assert_eq!(f(2.0), f(1.0), "{name} above range");
        }
    }

    #[test]
    fn midpoints() {
        assert!((ease_in_quad(0.5) - 0.25).abs() < 1e-6);
        assert!((ease_out_quad(0.5) - 0.75).abs() < 1e-6);
        assert!((ease_in_out_quad(0.5) - 0.5).abs() < 1e-6);
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-6);
        assert!((ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
    }

    #[test]
    fn elastic_overshoots() {
        let peak = (1..100)
            .map(|i| ease_out_elastic(i as f32 / 100.0))
            .fold(f32::MIN, f32::max);
        assert!(peak > 1.0);
    }
}
