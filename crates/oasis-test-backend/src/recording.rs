//! `RecordingBackend` -- records [`DrawCommand`] history for verification.
//!
//! Extends the mock approach with structured command recording and proper
//! clip/translate stack management so that `SdiBackend` default methods
//! can be tested end-to-end.

use oasis_types::backend::{
    Color, DrawCommand, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiShapes,
    SdiText, SdiTextures, SdiVector, TextureId, bitmap_measure_text,
    stacks::{ClipStack, TranslateStack},
};
use oasis_types::error::Result;
use oasis_types::geometry::ClipRect;

/// Type alias for a custom `measure_text` callback.
type MeasureTextFn = Box<dyn Fn(&str, u16) -> u32>;

/// A recording backend that captures every draw call as a [`DrawCommand`]
/// and maintains clip/translate stacks.
///
/// Useful for testing that `SdiBackend` default method implementations
/// decompose correctly into primitive calls.
///
/// # Example
///
/// ```
/// use oasis_test_backend::{RecordingBackend, DrawCommand, Color};
/// use oasis_types::backend::{SdiBackend, SdiShapes};
///
/// let mut rec = RecordingBackend::new(480, 272);
/// rec.fill_rounded_rect(10, 20, 100, 50, 8, Color::WHITE).ok();
///
/// let cmds = rec.commands();
/// assert_eq!(cmds.len(), 1);
/// assert!(matches!(cmds[0], DrawCommand::FillRect { .. }));
/// ```
pub struct RecordingBackend {
    width: u32,
    height: u32,
    commands: Vec<DrawCommand>,
    clip_stack: ClipStack,
    translate_stack: TranslateStack,
    next_texture_id: u64,
    measure_text_fn: Option<MeasureTextFn>,
}

impl RecordingBackend {
    /// Create a new recording backend with the given viewport dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            commands: Vec::new(),
            clip_stack: ClipStack::new(width, height),
            translate_stack: TranslateStack::new(),
            next_texture_id: 1,
            measure_text_fn: None,
        }
    }

    /// Return all recorded draw commands.
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Clear the recorded command history.
    ///
    /// Does not reset the clip/translate stacks.
    pub fn clear_commands(&mut self) {
        self.commands.clear();
    }

    /// Set a custom function for `measure_text`.
    pub fn set_measure_text_fn<F>(&mut self, f: F)
    where
        F: Fn(&str, u16) -> u32 + 'static,
    {
        self.measure_text_fn = Some(Box::new(f));
    }
}

impl SdiCore for RecordingBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.clip_stack = ClipStack::new(width, height);
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.commands.push(DrawCommand::FillRect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
            color,
        });
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.commands.push(DrawCommand::Blit { tex, x, y, w, h });
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        self.commands
            .push(DrawCommand::FillRect { x, y, w, h, color });
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
        self.commands.push(DrawCommand::DrawText {
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
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.commands.push(DrawCommand::PushClip { x, y, w, h });
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.commands.push(DrawCommand::PopClip);
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        if let Some(ref f) = self.measure_text_fn {
            return f(text, font_size);
        }
        bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(vec![0u8; (w as usize) * (h as usize) * 4])
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

impl SdiShapes for RecordingBackend {}
impl SdiGradients for RecordingBackend {}
impl SdiAlpha for RecordingBackend {
    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
impl SdiText for RecordingBackend {}
impl SdiTextures for RecordingBackend {}
impl SdiVector for RecordingBackend {}
impl SdiBatch for RecordingBackend {}

impl SdiClipTransform for RecordingBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let clip = ClipRect { x, y, w, h };
        self.clip_stack.push(clip);
        self.commands.push(DrawCommand::PushClip { x, y, w, h });
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.clip_stack.pop();
        self.commands.push(DrawCommand::PopClip);
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.clip_stack.current_tuple()
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(dx, dy);
        self.commands.push(DrawCommand::PushTranslate { dx, dy });
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        self.translate_stack.pop();
        self.commands.push(DrawCommand::PopTranslate);
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.translate_stack.current()
    }
}
