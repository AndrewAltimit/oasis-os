//! Color utility functions.

use crate::backend::Color;

/// Linearly interpolate between two colors.
///
/// `t` is clamped to `[0.0, 1.0]`. Returns `a` when `t == 0.0` and `b` when
/// `t == 1.0`.
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    a.lerp(b, t)
}

/// Linearly interpolate between two colors using integer ratio `num / den`.
///
/// This avoids floating-point math and is preferred for gradient scanline loops
/// where `num` steps from `0` to `den`. Returns `a` when `den == 0`.
pub fn lerp_color_ratio(a: Color, b: Color, num: u32, den: u32) -> Color {
    if den == 0 {
        return a;
    }
    let num = num.min(den);
    let inv = den - num;
    Color::rgba(
        ((a.r as u32 * inv + b.r as u32 * num + den / 2) / den) as u8,
        ((a.g as u32 * inv + b.g as u32 * num + den / 2) / den) as u8,
        ((a.b as u32 * inv + b.b as u32 * num + den / 2) / den) as u8,
        ((a.a as u32 * inv + b.a as u32 * num + den / 2) / den) as u8,
    )
}

/// Darken a color by a factor (0.0 = black, 1.0 = unchanged).
pub fn darken(color: Color, factor: f32) -> Color {
    color.darken(factor)
}

/// Lighten a color by blending toward white (0.0 = unchanged, 1.0 = white).
pub fn lighten(color: Color, factor: f32) -> Color {
    color.lighten(factor)
}

/// Set the alpha channel of a color.
pub fn with_alpha(color: Color, alpha: u8) -> Color {
    color.with_alpha(alpha)
}

/// Convert an RGB color to HSL (hue, saturation, lightness).
///
/// Returns `(h, s, l)` where `h` is in `[0.0, 360.0)`, and `s`, `l` are
/// in `[0.0, 1.0]`. Alpha is discarded.
pub fn rgb_to_hsl(color: Color) -> (f32, f32, f32) {
    let r = color.r as f32 / 255.0;
    let g = color.g as f32 / 255.0;
    let b = color.b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        // Achromatic.
        return (0.0, 0.0, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < f32::EPSILON {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };

    (h * 60.0, s, l)
}

/// Convert HSL to an RGB color (alpha = 255).
///
/// `h` is in degrees `[0.0, 360.0)`, `s` and `l` are in `[0.0, 1.0]`.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);

    if s.abs() < f32::EPSILON {
        let v = (l * 255.0).round() as u8;
        return Color::rgb(v, v, v);
    }

    let h = ((h % 360.0) + 360.0) % 360.0; // normalize to [0, 360)

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h_norm = h / 360.0;

    let hue_to_rgb = |p: f32, q: f32, mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };

    let r = (hue_to_rgb(p, q, h_norm + 1.0 / 3.0) * 255.0).round() as u8;
    let g = (hue_to_rgb(p, q, h_norm) * 255.0).round() as u8;
    let b = (hue_to_rgb(p, q, h_norm - 1.0 / 3.0) * 255.0).round() as u8;

    Color::rgb(r, g, b)
}

/// Rotate the hue of a color by the given number of degrees.
///
/// Wraps around at 360. Preserves saturation, lightness, and alpha.
pub fn rotate_hue(color: Color, degrees: f32) -> Color {
    let (h, s, l) = rgb_to_hsl(color);
    let mut result = hsl_to_rgb(h + degrees, s, l);
    result.a = color.a;
    result
}

// -----------------------------------------------------------------------
// WCAG 2.1 contrast utilities
// -----------------------------------------------------------------------

/// Compute the relative luminance of a color per WCAG 2.1.
///
/// Uses the sRGB linearization formula. Returns a value in `[0.0, 1.0]`
/// where 0 is darkest black and 1 is lightest white.
pub fn relative_luminance(c: Color) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let s = channel as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b)
}

/// Compute the WCAG 2.1 contrast ratio between two colors.
///
/// Returns a value in `[1.0, 21.0]`. The order of arguments does not matter.
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Check whether `fg` on `bg` meets WCAG AA for normal text (>= 4.5:1).
pub fn meets_wcag_aa(fg: Color, bg: Color) -> bool {
    contrast_ratio(fg, bg) >= 4.5
}

/// Check whether `fg` on `bg` meets WCAG AA for large text (>= 3.0:1).
pub fn meets_wcag_aa_large(fg: Color, bg: Color) -> bool {
    contrast_ratio(fg, bg) >= 3.0
}

/// Parse a hex color string like `"#RRGGBB"` or `"#RRGGBBAA"`.
///
/// Requires a leading `#`. Returns `None` for invalid input.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::rgb(r, g, b))
    } else if s.len() == 8 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        let a = u8::from_str_radix(&s[6..8], 16).ok()?;
        Some(Color::rgba(r, g, b, a))
    } else {
        None
    }
}

/// Adjust the saturation of a color by a multiplier.
///
/// `factor = 0.0` produces grayscale, `1.0` is unchanged, `2.0` doubles
/// saturation (clamped to 1.0). Preserves hue, lightness, and alpha.
pub fn adjust_saturation(color: Color, factor: f32) -> Color {
    let (h, s, l) = rgb_to_hsl(color);
    let new_s = (s * factor).clamp(0.0, 1.0);
    let mut result = hsl_to_rgb(h, new_s, l);
    result.a = color.a;
    result
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
    fn lerp_ratio_endpoints() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(255, 255, 255);
        let at_start = lerp_color_ratio(a, b, 0, 10);
        assert_eq!(at_start, a);
        let at_end = lerp_color_ratio(a, b, 10, 10);
        assert_eq!(at_end, b);
    }

    #[test]
    fn lerp_ratio_midpoint() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(200, 100, 50);
        let mid = lerp_color_ratio(a, b, 5, 10);
        assert_eq!(mid.r, 100);
        assert_eq!(mid.g, 50);
        assert_eq!(mid.b, 25);
    }

    #[test]
    fn lerp_ratio_zero_denominator() {
        let a = Color::rgb(42, 42, 42);
        let b = Color::rgb(200, 200, 200);
        let result = lerp_color_ratio(a, b, 5, 0);
        assert_eq!(result, a);
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

    #[test]
    fn hsl_red() {
        let (h, s, l) = rgb_to_hsl(Color::rgb(255, 0, 0));
        assert!((h - 0.0).abs() < 1.0);
        assert!((s - 1.0).abs() < 0.01);
        assert!((l - 0.5).abs() < 0.01);
    }

    #[test]
    fn hsl_green() {
        let (h, s, l) = rgb_to_hsl(Color::rgb(0, 255, 0));
        assert!((h - 120.0).abs() < 1.0);
        assert!((s - 1.0).abs() < 0.01);
        assert!((l - 0.5).abs() < 0.01);
    }

    #[test]
    fn hsl_blue() {
        let (h, s, l) = rgb_to_hsl(Color::rgb(0, 0, 255));
        assert!((h - 240.0).abs() < 1.0);
        assert!((s - 1.0).abs() < 0.01);
        assert!((l - 0.5).abs() < 0.01);
    }

    #[test]
    fn hsl_white() {
        let (h, s, l) = rgb_to_hsl(Color::rgb(255, 255, 255));
        assert!((s - 0.0).abs() < 0.01);
        assert!((l - 1.0).abs() < 0.01);
        let _ = h; // hue is undefined for achromatic
    }

    #[test]
    fn hsl_black() {
        let (h, s, l) = rgb_to_hsl(Color::rgb(0, 0, 0));
        assert!((s - 0.0).abs() < 0.01);
        assert!((l - 0.0).abs() < 0.01);
        let _ = h;
    }

    #[test]
    fn hsl_to_rgb_known_values() {
        // Pure red.
        let c = hsl_to_rgb(0.0, 1.0, 0.5);
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
        // Pure green.
        let c = hsl_to_rgb(120.0, 1.0, 0.5);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
        // Pure blue.
        let c = hsl_to_rgb(240.0, 1.0, 0.5);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
    }

    #[test]
    fn hsl_grayscale() {
        let c = hsl_to_rgb(0.0, 0.0, 0.5);
        assert_eq!(c.r, 128);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 128);
    }

    #[test]
    fn rotate_hue_red_to_green() {
        let result = rotate_hue(Color::rgb(255, 0, 0), 120.0);
        assert!(result.g > 200);
        assert!(result.r < 50);
        assert!(result.b < 50);
    }

    #[test]
    fn rotate_hue_preserves_alpha() {
        let result = rotate_hue(Color::rgba(255, 0, 0, 128), 120.0);
        assert_eq!(result.a, 128);
    }

    #[test]
    fn adjust_saturation_zero_is_gray() {
        let result = adjust_saturation(Color::rgb(255, 0, 0), 0.0);
        // Should be grayscale: r == g == b.
        assert!((result.r as i16 - result.g as i16).abs() <= 1);
        assert!((result.g as i16 - result.b as i16).abs() <= 1);
    }

    #[test]
    fn adjust_saturation_preserves_alpha() {
        let result = adjust_saturation(Color::rgba(255, 0, 0, 100), 0.5);
        assert_eq!(result.a, 100);
    }

    // -- WCAG contrast utility tests --

    #[test]
    fn luminance_black_is_zero() {
        let l = relative_luminance(Color::rgb(0, 0, 0));
        assert!(l.abs() < 1e-6);
    }

    #[test]
    fn luminance_white_is_one() {
        let l = relative_luminance(Color::rgb(255, 255, 255));
        assert!((l - 1.0).abs() < 1e-4);
    }

    #[test]
    fn luminance_mid_gray() {
        let l = relative_luminance(Color::rgb(128, 128, 128));
        // sRGB 128 -> ~0.2158 relative luminance
        assert!(l > 0.2 && l < 0.25);
    }

    #[test]
    fn contrast_black_white_is_21() {
        let ratio = contrast_ratio(Color::rgb(0, 0, 0), Color::rgb(255, 255, 255));
        assert!((ratio - 21.0).abs() < 0.1);
    }

    #[test]
    fn contrast_same_color_is_one() {
        let c = Color::rgb(100, 150, 200);
        let ratio = contrast_ratio(c, c);
        assert!((ratio - 1.0).abs() < 1e-6);
    }

    #[test]
    fn contrast_is_symmetric() {
        let a = Color::rgb(50, 100, 200);
        let b = Color::rgb(200, 200, 200);
        let r1 = contrast_ratio(a, b);
        let r2 = contrast_ratio(b, a);
        assert!((r1 - r2).abs() < 1e-10);
    }

    #[test]
    fn wcag_aa_white_on_black() {
        assert!(meets_wcag_aa(
            Color::rgb(255, 255, 255),
            Color::rgb(0, 0, 0)
        ));
    }

    #[test]
    fn wcag_aa_fails_light_on_light() {
        // Light gray on white should fail AA.
        assert!(!meets_wcag_aa(
            Color::rgb(200, 200, 200),
            Color::rgb(255, 255, 255)
        ));
    }

    #[test]
    fn wcag_aa_large_is_more_lenient() {
        // A pair that fails AA (4.5:1) but passes AA-large (3.0:1).
        // Gray 135 on white gives ~3.59:1 contrast.
        let fg = Color::rgb(135, 135, 135);
        let bg = Color::rgb(255, 255, 255);
        let ratio = contrast_ratio(fg, bg);
        assert!(ratio >= 3.0 && ratio < 4.5);
        assert!(!meets_wcag_aa(fg, bg));
        assert!(meets_wcag_aa_large(fg, bg));
    }

    #[test]
    fn wcag_known_ratio_value() {
        // WCAG example: pure blue (#0000FF) on white
        // Luminance of blue: 0.0722, white: 1.0
        // ratio = (1.0 + 0.05) / (0.0722 + 0.05) = 1.05/0.1222 ≈ 8.59
        let ratio = contrast_ratio(Color::rgb(0, 0, 255), Color::rgb(255, 255, 255));
        assert!(ratio > 8.0 && ratio < 9.0);
    }

    #[test]
    fn luminance_ignores_alpha() {
        let a = relative_luminance(Color::rgba(100, 150, 200, 0));
        let b = relative_luminance(Color::rgba(100, 150, 200, 255));
        assert!((a - b).abs() < 1e-10);
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

            #[test]
            fn hsl_roundtrip(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
                let orig = Color::rgb(r, g, b);
                let (h, s, l) = rgb_to_hsl(orig);
                let back = hsl_to_rgb(h, s, l);
                // Allow +-2 for float rounding through the conversion.
                prop_assert!((back.r as i16 - orig.r as i16).abs() <= 2,
                    "r: {} vs {} (h={h}, s={s}, l={l})", back.r, orig.r);
                prop_assert!((back.g as i16 - orig.g as i16).abs() <= 2,
                    "g: {} vs {} (h={h}, s={s}, l={l})", back.g, orig.g);
                prop_assert!((back.b as i16 - orig.b as i16).abs() <= 2,
                    "b: {} vs {} (h={h}, s={s}, l={l})", back.b, orig.b);
            }

            #[test]
            fn rotate_hue_360_is_identity(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
                let orig = Color::rgb(r, g, b);
                let rotated = rotate_hue(orig, 360.0);
                prop_assert!((rotated.r as i16 - orig.r as i16).abs() <= 2);
                prop_assert!((rotated.g as i16 - orig.g as i16).abs() <= 2);
                prop_assert!((rotated.b as i16 - orig.b as i16).abs() <= 2);
            }

            #[test]
            fn adjust_saturation_one_is_identity(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
                let orig = Color::rgb(r, g, b);
                let result = adjust_saturation(orig, 1.0);
                prop_assert!((result.r as i16 - orig.r as i16).abs() <= 2);
                prop_assert!((result.g as i16 - orig.g as i16).abs() <= 2);
                prop_assert!((result.b as i16 - orig.b as i16).abs() <= 2);
            }
        }
    }
}
