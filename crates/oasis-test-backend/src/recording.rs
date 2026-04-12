//! `RecordingBackend` -- records [`DrawCommand`] history for verification.
//!
//! Extends the mock approach with structured command recording and proper
//! clip/translate stack management so that `SdiBackend` default methods
//! can be tested end-to-end.

use oasis_types::backend::{
    BlendMode, Color, DrawCommand, RenderTargetId, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore,
    SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures, SdiVector, TextureId,
    bitmap_measure_text,
    stacks::{ClipStack, TranslateStack},
};
use oasis_types::error::{OasisError, Result};
use oasis_types::geometry::ClipRect;
use std::collections::HashSet;

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
/// ```no_run
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
    // Render-target pool state.
    next_render_target_id: u64,
    live_render_targets: HashSet<u64>,
    render_target_bind_stack: Vec<RenderTargetId>,
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
            next_render_target_id: 1,
            live_render_targets: HashSet::new(),
            render_target_bind_stack: Vec::new(),
        }
    }

    /// Return the number of render targets currently allocated (created
    /// but not destroyed) on this backend.
    pub fn live_render_target_count(&self) -> usize {
        self.live_render_targets.len()
    }

    /// Return the current depth of the render-target bind stack.
    /// Zero means subsequent draws land on the framebuffer.
    pub fn render_target_bind_depth(&self) -> usize {
        self.render_target_bind_stack.len()
    }

    /// Return the id at the top of the bind stack, if any.
    pub fn currently_bound_render_target(&self) -> Option<RenderTargetId> {
        self.render_target_bind_stack.last().copied()
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

impl SdiRenderTarget for RecordingBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        let id = RenderTargetId(self.next_render_target_id);
        self.next_render_target_id += 1;
        self.live_render_targets.insert(id.0);
        self.commands
            .push(DrawCommand::CreateRenderTarget { id, w, h });
        Ok(id)
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        if !self.live_render_targets.contains(&id.0) {
            return Err(OasisError::Backend(
                format!("bind_render_target: unknown id {id:?}").into(),
            ));
        }
        self.render_target_bind_stack.push(id);
        self.commands.push(DrawCommand::BindRenderTarget { id });
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        if self.render_target_bind_stack.pop().is_none() {
            return Err(OasisError::Backend(
                "unbind_render_target: bind stack underflow".into(),
            ));
        }
        self.commands.push(DrawCommand::UnbindRenderTarget);
        Ok(())
    }

    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    ) -> Result<()> {
        debug_assert!(
            (0.0..=1.0).contains(&opacity),
            "opacity must be in [0.0, 1.0], got {opacity}"
        );
        if !self.live_render_targets.contains(&id.0) {
            return Err(OasisError::Backend(
                format!("composite_render_target: unknown id {id:?}").into(),
            ));
        }
        self.commands.push(DrawCommand::CompositeRenderTarget {
            id,
            dst_x,
            dst_y,
            dst_w,
            dst_h,
            blend,
            opacity,
        });
        Ok(())
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        if !self.live_render_targets.contains(&id.0) {
            return Err(OasisError::Backend(
                format!("read_render_target: unknown id {id:?}").into(),
            ));
        }
        // Recording backend never stores pixel contents; return zeros
        // so callers that exercise the readback path still get
        // deterministic bytes.
        dst.fill(0);
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        if !self.live_render_targets.remove(&id.0) {
            return Err(OasisError::Backend(
                format!("destroy_render_target: unknown id {id:?}").into(),
            ));
        }
        self.commands.push(DrawCommand::DestroyRenderTarget { id });
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        true
    }

    fn supports_render_target_readback(&self) -> bool {
        true
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
        let (tx, ty) = self.translate_stack.translate(x, y);
        let clip = ClipRect { x: tx, y: ty, w, h };
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
