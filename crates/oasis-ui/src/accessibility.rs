//! Accessibility utilities for widgets.
//!
//! Provides types and helpers for building accessible UI experiences:
//! - `AccessibilityLabel` for screen reader text and alt text
//! - `contrast_ratio()` for WCAG color contrast validation
//! - `meets_wcag_aa()` / `meets_wcag_aaa()` for compliance checks

use oasis_types::backend::Color;

/// Accessibility label for a widget or visual element.
///
/// Stores descriptive text for screen readers and assistive technology.
/// Not displayed visually; purely for semantic annotation.
#[derive(Debug, Clone, Default)]
pub struct AccessibilityLabel {
    /// Short label (e.g. "Save button", "Volume slider").
    pub label: Option<String>,
    /// Extended description for complex widgets.
    pub description: Option<String>,
    /// Role hint (e.g. "button", "checkbox", "slider").
    pub role: Option<&'static str>,
}

impl AccessibilityLabel {
    /// Create a new label with the given text.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            description: None,
            role: None,
        }
    }

    /// Set the ARIA-like role.
    pub fn with_role(mut self, role: &'static str) -> Self {
        self.role = Some(role);
        self
    }

    /// Set the extended description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// Calculate the relative luminance of a color per WCAG 2.0.
///
/// Returns a value between 0.0 (black) and 1.0 (white).
pub fn relative_luminance(color: Color) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let s = f64::from(channel) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }

    let r = linearize(color.r);
    let g = linearize(color.g);
    let b = linearize(color.b);

    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Calculate the contrast ratio between two colors per WCAG 2.0.
///
/// Returns a value between 1.0 (identical) and 21.0 (black/white).
pub fn contrast_ratio(fg: Color, bg: Color) -> f64 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);

    let lighter = l1.max(l2);
    let darker = l1.min(l2);

    (lighter + 0.05) / (darker + 0.05)
}

/// Check if a foreground/background pair meets WCAG AA for normal text (4.5:1).
pub fn meets_wcag_aa(fg: Color, bg: Color) -> bool {
    contrast_ratio(fg, bg) >= 4.5
}

/// Check if a foreground/background pair meets WCAG AAA for normal text (7:1).
pub fn meets_wcag_aaa(fg: Color, bg: Color) -> bool {
    contrast_ratio(fg, bg) >= 7.0
}

/// Check if a foreground/background pair meets WCAG AA for large text (3:1).
pub fn meets_wcag_aa_large(fg: Color, bg: Color) -> bool {
    contrast_ratio(fg, bg) >= 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_white_max_contrast() {
        let black = Color::rgba(0, 0, 0, 255);
        let white = Color::rgba(255, 255, 255, 255);
        let ratio = contrast_ratio(black, white);
        assert!(ratio > 20.0, "expected ~21:1, got {ratio}");
    }

    #[test]
    fn same_color_min_contrast() {
        let c = Color::rgba(128, 128, 128, 255);
        let ratio = contrast_ratio(c, c);
        assert!((ratio - 1.0).abs() < 0.01);
    }

    #[test]
    fn wcag_aa_black_on_white() {
        let black = Color::rgba(0, 0, 0, 255);
        let white = Color::rgba(255, 255, 255, 255);
        assert!(meets_wcag_aa(black, white));
        assert!(meets_wcag_aaa(black, white));
    }

    #[test]
    fn wcag_aa_fails_low_contrast() {
        let light1 = Color::rgba(200, 200, 200, 255);
        let light2 = Color::rgba(220, 220, 220, 255);
        assert!(!meets_wcag_aa(light1, light2));
    }

    #[test]
    fn relative_luminance_black() {
        let black = Color::rgba(0, 0, 0, 255);
        assert!(relative_luminance(black) < 0.001);
    }

    #[test]
    fn relative_luminance_white() {
        let white = Color::rgba(255, 255, 255, 255);
        assert!((relative_luminance(white) - 1.0).abs() < 0.01);
    }

    #[test]
    fn accessibility_label_builder() {
        let label = AccessibilityLabel::new("Save")
            .with_role("button")
            .with_description("Save the current document");
        assert_eq!(label.label.as_deref(), Some("Save"));
        assert_eq!(label.role, Some("button"));
        assert!(label.description.is_some());
    }

    #[test]
    fn accessibility_label_default() {
        let label = AccessibilityLabel::default();
        assert!(label.label.is_none());
        assert!(label.role.is_none());
        assert!(label.description.is_none());
    }

    #[test]
    fn contrast_ratio_symmetric() {
        let a = Color::rgba(100, 50, 200, 255);
        let b = Color::rgba(240, 240, 240, 255);
        let ratio1 = contrast_ratio(a, b);
        let ratio2 = contrast_ratio(b, a);
        assert!((ratio1 - ratio2).abs() < 0.01);
    }

    #[test]
    fn highcontrast_skin_text_passes_aaa() {
        // High contrast skin: white text on black background.
        let white = Color::rgba(255, 255, 255, 255);
        let black = Color::rgba(0, 0, 0, 255);
        assert!(meets_wcag_aaa(white, black));
    }

    #[test]
    fn highcontrast_skin_accent_passes_aa() {
        // High contrast skin: yellow accent on black background.
        let yellow = Color::rgba(255, 255, 0, 255);
        let black = Color::rgba(0, 0, 0, 255);
        assert!(meets_wcag_aa(yellow, black));
    }
}
