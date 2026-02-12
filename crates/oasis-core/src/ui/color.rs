//! Color utility functions.

use crate::backend::Color;

/// Linearly interpolate between two colors.
///
/// `t` is clamped to `[0.0, 1.0]`. Returns `a` when `t == 0.0` and `b` when
/// `t == 1.0`.
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::rgba(
        (a.r as f32 + (b.r as f32 - a.r as f32) * t) as u8,
        (a.g as f32 + (b.g as f32 - a.g as f32) * t) as u8,
        (a.b as f32 + (b.b as f32 - a.b as f32) * t) as u8,
        (a.a as f32 + (b.a as f32 - a.a as f32) * t) as u8,
    )
}

/// Darken a color by a factor (0.0 = black, 1.0 = unchanged).
pub fn darken(color: Color, factor: f32) -> Color {
    let f = factor.clamp(0.0, 1.0);
    Color::rgba(
        (color.r as f32 * f) as u8,
        (color.g as f32 * f) as u8,
        (color.b as f32 * f) as u8,
        color.a,
    )
}

/// Lighten a color by blending toward white (0.0 = unchanged, 1.0 = white).
pub fn lighten(color: Color, factor: f32) -> Color {
    lerp_color(color, Color::WHITE, factor)
}

/// Set the alpha channel of a color.
pub fn with_alpha(color: Color, alpha: u8) -> Color {
    color.with_alpha(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_endpoints() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(255, 255, 255);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), Color::rgb(255, 255, 255));
    }

    #[test]
    fn lerp_midpoint() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(200, 100, 50);
        let mid = lerp_color(a, b, 0.5);
        assert_eq!(mid.r, 100);
        assert_eq!(mid.g, 50);
        assert_eq!(mid.b, 25);
    }

    #[test]
    fn darken_halves() {
        let c = Color::rgb(200, 100, 50);
        let d = darken(c, 0.5);
        assert_eq!(d.r, 100);
        assert_eq!(d.g, 50);
        assert_eq!(d.b, 25);
    }

    #[test]
    fn lighten_full() {
        let c = Color::rgb(0, 0, 0);
        let l = lighten(c, 1.0);
        assert_eq!(l, Color::rgb(255, 255, 255));
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        fn arb_color() -> impl Strategy<Value = Color> {
            (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
                .prop_map(|(r, g, b, a)| Color::rgba(r, g, b, a))
        }

        proptest! {
            #[test]
            fn rgb_roundtrip(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
                let c = Color::rgb(r, g, b);
                prop_assert_eq!(c.r, r);
                prop_assert_eq!(c.g, g);
                prop_assert_eq!(c.b, b);
                prop_assert_eq!(c.a, 255);
            }

            #[test]
            fn rgba_roundtrip(r in any::<u8>(), g in any::<u8>(), b in any::<u8>(), a in any::<u8>()) {
                let c = Color::rgba(r, g, b, a);
                prop_assert_eq!(c.r, r);
                prop_assert_eq!(c.g, g);
                prop_assert_eq!(c.b, b);
                prop_assert_eq!(c.a, a);
            }

            #[test]
            fn lerp_at_zero_returns_first(a in arb_color(), b in arb_color()) {
                let result = lerp_color(a, b, 0.0);
                prop_assert_eq!(result, a);
            }

            #[test]
            fn lerp_at_one_returns_second(a in arb_color(), b in arb_color()) {
                let result = lerp_color(a, b, 1.0);
                // Allow +-1 due to floating point rounding.
                prop_assert!((result.r as i16 - b.r as i16).abs() <= 1);
                prop_assert!((result.g as i16 - b.g as i16).abs() <= 1);
                prop_assert!((result.b as i16 - b.b as i16).abs() <= 1);
                prop_assert!((result.a as i16 - b.a as i16).abs() <= 1);
            }

            #[test]
            fn lerp_clamps_above_one(a in arb_color(), b in arb_color(), t in 1.0f32..100.0) {
                let at_one = lerp_color(a, b, 1.0);
                let clamped = lerp_color(a, b, t);
                prop_assert_eq!(at_one, clamped, "t > 1.0 should be clamped to 1.0");
            }

            #[test]
            fn lerp_clamps_below_zero(a in arb_color(), b in arb_color(), t in -100.0f32..0.0) {
                let at_zero = lerp_color(a, b, 0.0);
                let clamped = lerp_color(a, b, t);
                prop_assert_eq!(at_zero, clamped, "t < 0.0 should be clamped to 0.0");
            }

            #[test]
            fn darken_preserves_alpha(c in arb_color(), f in 0.0f32..=1.0) {
                let d = darken(c, f);
                prop_assert_eq!(d.a, c.a, "darken must preserve alpha");
            }

            #[test]
            fn darken_zero_is_black(c in arb_color()) {
                let d = darken(c, 0.0);
                prop_assert_eq!(d.r, 0);
                prop_assert_eq!(d.g, 0);
                prop_assert_eq!(d.b, 0);
                prop_assert_eq!(d.a, c.a);
            }

            #[test]
            fn darken_one_is_unchanged(c in arb_color()) {
                let d = darken(c, 1.0);
                // Allow +-1 for float rounding.
                prop_assert!((d.r as i16 - c.r as i16).abs() <= 1);
                prop_assert!((d.g as i16 - c.g as i16).abs() <= 1);
                prop_assert!((d.b as i16 - c.b as i16).abs() <= 1);
            }

            #[test]
            fn lighten_zero_is_unchanged(c in arb_color()) {
                let l = lighten(c, 0.0);
                prop_assert_eq!(l, c);
            }

            #[test]
            fn lighten_one_is_white_rgb(c in arb_color()) {
                let l = lighten(c, 1.0);
                // Allow +-1 for float rounding.
                prop_assert!((l.r as i16 - 255).abs() <= 1);
                prop_assert!((l.g as i16 - 255).abs() <= 1);
                prop_assert!((l.b as i16 - 255).abs() <= 1);
            }

            #[test]
            fn with_alpha_sets_alpha(c in arb_color(), a in any::<u8>()) {
                let result = with_alpha(c, a);
                prop_assert_eq!(result.r, c.r);
                prop_assert_eq!(result.g, c.g);
                prop_assert_eq!(result.b, c.b);
                prop_assert_eq!(result.a, a);
            }
        }
    }
}
