//! Font-aware text measurement that delegates to the [`FontRegistry`]
//! for web fonts and falls back to the bitmap font for everything else.

use std::cell::RefCell;

use super::registry::{FontId, FontRegistry};
use crate::layout::block::TextMeasurer;

/// A text measurer that checks the [`FontRegistry`] for web fonts
/// before falling back to the bitmap font measurer.
///
/// Because `measure_text` takes `&self` but rasterization (which
/// populates the glyph cache on demand) needs `&mut`, the registry
/// is wrapped in `RefCell`. This is safe because layout is
/// single-threaded.
///
/// Usage: call `set_font_id` before each text run's measurement to
/// select the active web font (from `ComputedStyle::web_font_id`).
pub struct FontAwareTextMeasurer<'a> {
    inner: &'a dyn TextMeasurer,
    registry: &'a RefCell<FontRegistry>,
    /// Currently active web font (set per-run).
    active_font: RefCell<Option<FontId>>,
}

impl<'a> FontAwareTextMeasurer<'a> {
    /// Create a new font-aware measurer.
    pub fn new(inner: &'a dyn TextMeasurer, registry: &'a RefCell<FontRegistry>) -> Self {
        FontAwareTextMeasurer {
            inner,
            registry,
            active_font: RefCell::new(None),
        }
    }

    /// Set the active web font ID for subsequent measurements.
    ///
    /// Pass `None` to fall back to bitmap font measurement.
    pub fn set_font_id(&self, font_id: Option<u32>) {
        *self.active_font.borrow_mut() = font_id.map(FontId::from_raw);
    }
}

impl TextMeasurer for FontAwareTextMeasurer<'_> {
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        if let Some(font_id) = *self.active_font.borrow() {
            let reg = self.registry.borrow();
            if reg.font_count() > font_id.as_raw() as usize {
                return reg.measure_text(text, font_size as f32, font_id);
            }
        }
        // Fall back to bitmap font.
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

        // No web font set — should use bitmap.
        let w = measurer.measure_text("hello", 16);
        assert_eq!(w, oasis_types::backend::bitmap_measure_text("hello", 16),);
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

        // Set web font — should use fontdue measurement.
        measurer.set_font_id(Some(0));
        let web_w = measurer.measure_text(" ", 16);

        // Clear web font — should use bitmap.
        measurer.set_font_id(None);
        let bitmap_w = measurer.measure_text(" ", 16);

        // The widths will differ because fontdue and bitmap use
        // different metrics. Just verify both produce non-zero results.
        assert!(web_w > 0, "web font should produce width");
        assert!(bitmap_w > 0, "bitmap should produce width");
    }
}
