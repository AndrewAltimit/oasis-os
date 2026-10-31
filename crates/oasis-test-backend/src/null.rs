//! `NullBackend` -- a measure-only backend that draws nothing.
//!
//! For tests and benches that need a `&mut dyn SdiBackend` but assert on
//! state elsewhere (window manager geometry, decoder state), and for
//! benches that measure caller-side cost only. Use
//! [`RecordingBackend`](crate::RecordingBackend) to assert on draw calls and
//! [`MockSdiCore`](crate::MockSdiCore) to assert on raw `SdiCore` calls.

use oasis_types::backend::{
    Color, DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH, SdiCore, TextureId,
    bitmap_measure_text, texture_not_found,
};
use oasis_types::error::Result;
use std::collections::HashSet;

/// Type alias for a custom `measure_text` callback.
type MeasureTextFn = Box<dyn Fn(&str, u16) -> u32>;

/// A backend whose draw calls all succeed and do nothing.
///
/// - `measure_text` uses the bitmap font metrics (like every real backend)
///   unless [`set_measure_text_fn`](Self::set_measure_text_fn) overrides it;
/// - `viewport_size` reports the configured size;
/// - `load_texture` hands out distinct ids starting at 1, and
///   `destroy_texture` rejects an id it did not hand out (or already
///   destroyed) with `BackendError::TextureNotFound`, like every backend;
/// - `read_pixels` returns a zeroed buffer of the requested size;
/// - every extension trait keeps its default, so higher-level calls
///   (rounded rects, text wrapping, ...) decompose into no-op primitives.
///
/// ```
/// use oasis_test_backend::NullBackend;
/// use oasis_types::backend::{Color, SdiBackend, SdiShapes};
///
/// let mut null = NullBackend::default();
/// null.fill_rounded_rect(0, 0, 50, 20, 4, Color::WHITE).unwrap();
/// let _: &mut dyn SdiBackend = &mut null;
/// ```
pub struct NullBackend {
    width: u32,
    height: u32,
    next_texture_id: u64,
    live_textures: HashSet<u64>,
    measure_text_fn: Option<MeasureTextFn>,
}

impl NullBackend {
    /// Create a null backend reporting a `width` x `height` viewport.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            next_texture_id: 1,
            live_textures: HashSet::new(),
            measure_text_fn: None,
        }
    }

    /// Replace the bitmap-font `measure_text` with `f`.
    pub fn set_measure_text_fn<F>(&mut self, f: F)
    where
        F: Fn(&str, u16) -> u32 + 'static,
    {
        self.measure_text_fn = Some(Box::new(f));
    }

    /// Builder form of [`set_measure_text_fn`](Self::set_measure_text_fn).
    #[must_use]
    pub fn with_measure_text_fn<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, u16) -> u32 + 'static,
    {
        self.set_measure_text_fn(f);
        self
    }
}

impl Default for NullBackend {
    /// A null backend with the default (PSP-native 480x272) viewport.
    fn default() -> Self {
        Self::new(DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT)
    }
}

impl SdiCore for NullBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn clear(&mut self, _color: Color) -> Result<()> {
        Ok(())
    }

    fn blit(&mut self, _tex: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }

    fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _color: Color) -> Result<()> {
        Ok(())
    }

    fn draw_text(&mut self, _t: &str, _x: i32, _y: i32, _size: u16, _color: Color) -> Result<()> {
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_texture(&mut self, _width: u32, _height: u32, _rgba: &[u8]) -> Result<TextureId> {
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.live_textures.insert(id);
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        if self.live_textures.remove(&tex.0) {
            Ok(())
        } else {
            Err(texture_not_found(tex))
        }
    }

    fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        match self.measure_text_fn {
            Some(ref f) => f(text, font_size),
            None => bitmap_measure_text(text, font_size),
        }
    }

    fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(vec![0u8; (w as usize) * (h as usize) * 4])
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }

    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

oasis_types::impl_sdi_defaults!(NullBackend);
