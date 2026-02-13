//! Shared test utilities for oasis-ui widget tests.
//!
//! Provides a [`MockBackend`] that records all draw calls for assertion.

use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

/// A recorded draw call from the mock backend.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DrawCall {
    FillRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    },
    DrawText {
        text: String,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    },
    Blit {
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
}

/// A mock backend that records all draw calls for test assertions.
pub struct MockBackend {
    pub calls: Vec<DrawCall>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }

    /// Count of `FillRect` calls.
    pub fn fill_rect_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| matches!(c, DrawCall::FillRect { .. }))
            .count()
    }

    /// Count of `DrawText` calls.
    pub fn draw_text_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| matches!(c, DrawCall::DrawText { .. }))
            .count()
    }

    /// Return only the `DrawText` entries.
    #[allow(dead_code)]
    pub fn text_calls(&self) -> Vec<&DrawCall> {
        self.calls
            .iter()
            .filter(|c| matches!(c, DrawCall::DrawText { .. }))
            .collect()
    }

    /// Return text draw calls as `(text, x, y, font_size)` tuples,
    /// sorted by Y then X position for easy geometric analysis.
    pub fn text_positions(&self) -> Vec<(&str, i32, i32, u16)> {
        let mut positions: Vec<_> = self
            .calls
            .iter()
            .filter_map(|c| {
                if let DrawCall::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    ..
                } = c
                {
                    Some((text.as_str(), *x, *y, *font_size))
                } else {
                    None
                }
            })
            .collect();
        positions.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.cmp(&b.1)));
        positions
    }

    /// Check if any `DrawText` call contains the given substring.
    pub fn has_text(&self, needle: &str) -> bool {
        self.calls.iter().any(|c| {
            if let DrawCall::DrawText { text, .. } = c {
                text.contains(needle)
            } else {
                false
            }
        })
    }
}

impl SdiBackend for MockBackend {
    fn init(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self, _color: Color) -> Result<()> {
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.calls.push(DrawCall::Blit { tex, x, y, w, h });
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        self.calls.push(DrawCall::FillRect { x, y, w, h, color });
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
        self.calls.push(DrawCall::DrawText {
            text: text.to_string(),
            x,
            y,
            font_size,
            color,
        });
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_texture(&mut self, _width: u32, _height: u32, _rgba_data: &[u8]) -> Result<TextureId> {
        Ok(TextureId(0))
    }

    fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
        Ok(())
    }

    fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        Ok(())
    }

    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        text.len() as u32 * oasis_types::backend::BITMAP_GLYPH_WIDTH
    }

    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
