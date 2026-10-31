//! Font-aware text measurement that delegates to the [`FontRegistry`]
//! for web fonts and falls back to the bitmap font for everything else.

use std::cell::RefCell;

use super::registry::{FontId, FontRegistry};
use crate::layout::block::TextMeasurer;

/// A text measurer that measures runs in a loaded `@font-face` font
/// with the [`FontRegistry`] and everything else with `inner`.
///
/// Layout passes each run's font (`ComputedStyle::web_font_id`) to
/// [`TextMeasurer::measure_text_font`]; the browser widget's layout
/// measurer is one of these once faces have loaded, so web-font text is
/// broken into lines with the advances it is painted with.
///
/// The registry is behind a `RefCell` because rasterization (painting)
/// borrows it mutably; layout only reads it, single-threaded.
pub struct FontAwareTextMeasurer<'a> {
    inner: &'a dyn TextMeasurer,
    registry: &'a RefCell<FontRegistry>,
}

impl<'a> FontAwareTextMeasurer<'a> {
    /// Create a new font-aware measurer.
    pub fn new(inner: &'a dyn TextMeasurer, registry: &'a RefCell<FontRegistry>) -> Self {
        FontAwareTextMeasurer { inner, registry }
    }
}

impl TextMeasurer for FontAwareTextMeasurer<'_> {
    fn measure_text_font(&self, text: &str, font_size: u16, web_font: Option<u32>) -> u32 {
        if let Some(raw) = web_font {
            let reg = self.registry.borrow();
            if reg.font_count() > raw as usize {
                return reg.measure_text(text, font_size as f32, FontId::from_raw(raw));
            }
        }
        self.inner.measure_text_font(text, font_size, None)
    }

    /// Text with no web font: the bitmap (`inner`) measurement.
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        self.inner.measure_text(text, font_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BitmapMeasurer;
    impl TextMeasurer for BitmapMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    #[test]
    fn fallback_to_bitmap_when_no_font() {
        let reg = RefCell::new(FontRegistry::new());
        let bitmap = BitmapMeasurer;
        let measurer = FontAwareTextMeasurer::new(&bitmap, &reg);
        let expected = oasis_types::backend::bitmap_measure_text("hello", 16);
        assert_eq!(measurer.measure_text("hello", 16), expected);
        // An id with no loaded face falls back too.
        assert_eq!(measurer.measure_text_font("hello", 16, Some(3)), expected);
    }

    #[test]
    fn web_font_measurement_when_loaded() {
        let mut registry = FontRegistry::new();
        let data = include_bytes!("../../test_data/minimal.ttf");
        let ok = registry.load_font_data(
            "TestFont",
            (400, 400),
            crate::css::parser::FontFaceStyle::Normal,
            data,
        );
        assert!(ok);

        let reg = RefCell::new(registry);
        let bitmap = BitmapMeasurer;
        let measurer = FontAwareTextMeasurer::new(&bitmap, &reg);

        let web_w = measurer.measure_text_font(" ", 16, Some(0));
        let expected = reg.borrow().measure_text(" ", 16.0, FontId::from_raw(0));
        assert_eq!(web_w, expected, "the run's face measures it");
        assert!(web_w > 0, "web font should produce width");
        let bitmap_w = measurer.measure_text_font(" ", 16, None);
        assert_eq!(bitmap_w, oasis_types::backend::bitmap_measure_text(" ", 16));
    }
}
