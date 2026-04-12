//! `MockSdiCore` -- a configurable mock backend for testing.
//!
//! Records all method calls with their parameters and allows configuring
//! return values (e.g., what `measure_text` returns). Implements both
//! [`SdiCore`](oasis_types::backend::SdiCore) and [`SdiBackend`](oasis_types::backend::SdiBackend) (with default implementations).

use oasis_types::backend::{
    Color, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes,
    SdiText, SdiTextures, SdiVector, TextureId, bitmap_measure_text,
};
use oasis_types::error::Result;

/// A recorded method call: `(method_name, formatted_parameters)`.
pub type CallRecord = (String, String);

/// Type alias for a custom `measure_text` callback.
type MeasureTextFn = Box<dyn Fn(&str, u16) -> u32>;

/// A mock rendering backend that records all calls made to it.
///
/// # Usage
///
/// ```no_run
/// use oasis_test_backend::MockSdiCore;
/// use oasis_types::backend::{Color, SdiCore};
///
/// let mut mock = MockSdiCore::new(480, 272);
/// mock.fill_rect(10, 20, 100, 50, Color::WHITE).ok();
///
/// let calls = mock.calls();
/// assert_eq!(calls.len(), 1);
/// assert_eq!(calls[0].0, "fill_rect");
/// ```
pub struct MockSdiCore {
    width: u32,
    height: u32,
    calls: Vec<CallRecord>,
    next_texture_id: u64,
    measure_text_fn: Option<MeasureTextFn>,
}

impl MockSdiCore {
    /// Create a new mock backend with the given viewport dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            calls: Vec::new(),
            next_texture_id: 1,
            measure_text_fn: None,
        }
    }

    /// Return all recorded calls.
    pub fn calls(&self) -> &[CallRecord] {
        &self.calls
    }

    /// Clear the recorded call history.
    pub fn clear_calls(&mut self) {
        self.calls.clear();
    }

    /// Set a custom function for `measure_text`.
    ///
    /// By default, `measure_text` uses the bitmap font metrics from
    /// `oasis-types`. Use this to return specific widths in tests.
    pub fn set_measure_text_fn<F>(&mut self, f: F)
    where
        F: Fn(&str, u16) -> u32 + 'static,
    {
        self.measure_text_fn = Some(Box::new(f));
    }

    /// Record a call.
    fn record(&mut self, method: &str, params: String) {
        self.calls.push((method.to_string(), params));
    }
}

impl SdiCore for MockSdiCore {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.record("init", format!("{width}, {height}"));
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.record("clear", format!("{color:?}"));
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.record("blit", format!("{tex:?}, {x}, {y}, {w}, {h}"));
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        self.record("fill_rect", format!("{x}, {y}, {w}, {h}, {color:?}"));
        Ok(())
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> Result<()> {
        self.record(
            "draw_text",
            format!("{text:?}, {x}, {y}, {font_size}, {color:?}"),
        );
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.record("swap_buffers", String::new());
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.record(
            "load_texture",
            format!("{width}, {height}, {} bytes", rgba_data.len()),
        );
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.record("destroy_texture", format!("{tex:?}"));
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.record("set_clip_rect", format!("{x}, {y}, {w}, {h}"));
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.record("reset_clip_rect", String::new());
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        if let Some(ref f) = self.measure_text_fn {
            return f(text, font_size);
        }
        bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let _ = (x, y);
        Ok(vec![0u8; (w as usize) * (h as usize) * 4])
    }

    fn shutdown(&mut self) -> Result<()> {
        self.record("shutdown", String::new());
        Ok(())
    }
}

// Pick up all extension trait default implementations (fill_rounded_rect falls
// back to fill_rect, draw_line uses Bresenham's, etc.).
// The blanket impl on SdiBackend gives us SdiBackend for free.
impl SdiShapes for MockSdiCore {}
impl SdiGradients for MockSdiCore {}
impl SdiAlpha for MockSdiCore {
    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
impl SdiText for MockSdiCore {}
impl SdiTextures for MockSdiCore {}
impl SdiClipTransform for MockSdiCore {}
impl SdiVector for MockSdiCore {}
impl SdiBatch for MockSdiCore {}
impl SdiRenderTarget for MockSdiCore {}
