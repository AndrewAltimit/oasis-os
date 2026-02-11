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
}
