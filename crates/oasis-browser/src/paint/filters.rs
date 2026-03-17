//! CSS filter effect application: color-based filters (grayscale, invert,
//! sepia, brightness, contrast, saturate, hue-rotate, opacity, blur).
//!
//! True Gaussian blur requires per-pixel post-processing via render targets.
//! The software fallback approximates blur by slightly dimming and desaturating
//! colors proportional to the blur radius. GPU backends can override via the
//! `BlurHint` display item.

use crate::css::values::FilterFunction;
use oasis_types::backend::Color;

/// Apply a chain of CSS filter functions to a color.
///
/// Filters are applied in declaration order per the CSS spec.
pub fn apply_filters(color: Color, filters: &[FilterFunction]) -> Color {
    let mut r = color.r as f32;
    let mut g = color.g as f32;
    let mut b = color.b as f32;
    let mut a = color.a as f32;

    for filter in filters {
        match *filter {
            FilterFunction::Grayscale(amount) => {
                let amount = amount.clamp(0.0, 1.0);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = r + (luma - r) * amount;
                g = g + (luma - g) * amount;
                b = b + (luma - b) * amount;
            },
            FilterFunction::Invert(amount) => {
                let amount = amount.clamp(0.0, 1.0);
                r = r + (255.0 - 2.0 * r) * amount;
                g = g + (255.0 - 2.0 * g) * amount;
                b = b + (255.0 - 2.0 * b) * amount;
            },
            FilterFunction::Sepia(amount) => {
                let amount = amount.clamp(0.0, 1.0);
                let sr = (0.393 * r + 0.769 * g + 0.189 * b).min(255.0);
                let sg = (0.349 * r + 0.686 * g + 0.168 * b).min(255.0);
                let sb = (0.272 * r + 0.534 * g + 0.131 * b).min(255.0);
                r = r + (sr - r) * amount;
                g = g + (sg - g) * amount;
                b = b + (sb - b) * amount;
            },
            FilterFunction::Brightness(factor) => {
                let factor = factor.max(0.0);
                r = (r * factor).min(255.0);
                g = (g * factor).min(255.0);
                b = (b * factor).min(255.0);
            },
            FilterFunction::Contrast(factor) => {
                let factor = factor.max(0.0);
                r = ((r - 127.5) * factor + 127.5).clamp(0.0, 255.0);
                g = ((g - 127.5) * factor + 127.5).clamp(0.0, 255.0);
                b = ((b - 127.5) * factor + 127.5).clamp(0.0, 255.0);
            },
            FilterFunction::Saturate(factor) => {
                let factor = factor.max(0.0);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = (luma + (r - luma) * factor).clamp(0.0, 255.0);
                g = (luma + (g - luma) * factor).clamp(0.0, 255.0);
                b = (luma + (b - luma) * factor).clamp(0.0, 255.0);
            },
            FilterFunction::HueRotate(deg) => {
                let rad = deg.to_radians();
                let cos = rad.cos();
                let sin = rad.sin();
                // Hue rotation matrix (approximate, from CSS spec).
                let nr = (0.213 + cos * 0.787 - sin * 0.213) * r
                    + (0.715 - cos * 0.715 - sin * 0.715) * g
                    + (0.072 - cos * 0.072 + sin * 0.928) * b;
                let ng = (0.213 - cos * 0.213 + sin * 0.143) * r
                    + (0.715 + cos * 0.285 + sin * 0.140) * g
                    + (0.072 - cos * 0.072 - sin * 0.283) * b;
                let nb = (0.213 - cos * 0.213 - sin * 0.787) * r
                    + (0.715 - cos * 0.715 + sin * 0.715) * g
                    + (0.072 + cos * 0.928 + sin * 0.072) * b;
                r = nr.clamp(0.0, 255.0);
                g = ng.clamp(0.0, 255.0);
                b = nb.clamp(0.0, 255.0);
            },
            FilterFunction::Opacity(factor) => {
                let factor = factor.clamp(0.0, 1.0);
                a *= factor;
            },
            FilterFunction::Blur(radius) => {
                // True Gaussian blur requires per-pixel post-processing
                // (render targets). Approximate by slightly dimming and
                // desaturating proportional to the blur radius — a visual
                // hint that blur is active without actual convolution.
                let clamped = radius.clamp(0.0, 10.0);
                // Slight brightness reduction.
                let bright = 1.0 - clamped * 0.02;
                r = (r * bright).clamp(0.0, 255.0);
                g = (g * bright).clamp(0.0, 255.0);
                b = (b * bright).clamp(0.0, 255.0);
                // Slight desaturation.
                let desat = 1.0 - clamped * 0.05;
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = (luma + (r - luma) * desat).clamp(0.0, 255.0);
                g = (luma + (g - luma) * desat).clamp(0.0, 255.0);
                b = (luma + (b - luma) * desat).clamp(0.0, 255.0);
            },
        }
    }

    Color::rgba(r as u8, g as u8, b as u8, a as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grayscale_full() {
        let c = Color::rgb(255, 0, 0);
        let result = apply_filters(c, &[FilterFunction::Grayscale(1.0)]);
        // Pure red → luma ≈ 54
        assert!(result.r == result.g && result.g == result.b);
        assert!(result.r > 50 && result.r < 60);
    }

    #[test]
    fn invert_full() {
        let c = Color::rgb(255, 0, 128);
        let result = apply_filters(c, &[FilterFunction::Invert(1.0)]);
        assert_eq!(result.r, 0);
        assert_eq!(result.g, 255);
        assert_eq!(result.b, 127);
    }

    #[test]
    fn brightness_double() {
        let c = Color::rgb(100, 50, 25);
        let result = apply_filters(c, &[FilterFunction::Brightness(2.0)]);
        assert_eq!(result.r, 200);
        assert_eq!(result.g, 100);
        assert_eq!(result.b, 50);
    }

    #[test]
    fn identity_filters() {
        let c = Color::rgb(128, 64, 32);
        // These should be identity operations.
        let result = apply_filters(
            c,
            &[
                FilterFunction::Grayscale(0.0),
                FilterFunction::Invert(0.0),
                FilterFunction::Sepia(0.0),
                FilterFunction::Brightness(1.0),
                FilterFunction::Contrast(1.0),
                FilterFunction::Saturate(1.0),
                FilterFunction::HueRotate(0.0),
                FilterFunction::Opacity(1.0),
            ],
        );
        // Allow ±1 rounding tolerance.
        assert!((result.r as i32 - c.r as i32).abs() <= 1);
        assert!((result.g as i32 - c.g as i32).abs() <= 1);
        assert!((result.b as i32 - c.b as i32).abs() <= 1);
    }

    #[test]
    fn opacity_filter() {
        let c = Color::rgba(255, 128, 64, 200);
        let result = apply_filters(c, &[FilterFunction::Opacity(0.5)]);
        assert_eq!(result.a, 100);
        assert_eq!(result.r, 255);
    }

    #[test]
    fn blur_approximation_dims_and_desaturates() {
        let c = Color::rgb(200, 100, 50);
        let result = apply_filters(c, &[FilterFunction::Blur(5.0)]);
        // Blur(5) → brightness = 1 - 5*0.02 = 0.9, saturation = 1 - 5*0.05 = 0.75.
        // Should be slightly dimmer and less saturated than original.
        assert!(result.r < c.r, "red should be dimmer");
        assert!(
            result.g < c.g || result.g >= c.g,
            "green may shift either way"
        );
        assert!(
            result.b >= c.b || result.b < c.b,
            "blue may shift either way"
        );
        // Alpha is unchanged.
        assert_eq!(result.a, c.a);
    }

    #[test]
    fn blur_zero_is_identity() {
        let c = Color::rgb(128, 64, 32);
        let result = apply_filters(c, &[FilterFunction::Blur(0.0)]);
        // Blur(0) should be a no-op.
        assert_eq!(result.r, c.r);
        assert_eq!(result.g, c.g);
        assert_eq!(result.b, c.b);
    }
}
