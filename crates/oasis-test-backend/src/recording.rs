//! `RecordingBackend` -- records [`DrawCommand`] history for verification.
//!
//! Extends the mock approach with structured command recording and proper
//! clip/translate stack management so that `SdiBackend` default methods
//! can be tested end-to-end. Render-target binds follow the drawing-state
//! contract (ADR-003): a bound target starts with no clip and no
//! translation and reports its own size, and the unbind restores the
//! parent's stacks.

use crate::DrawCommand;
use oasis_types::backend::{
    BlendMode, Color, DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH, RenderTargetId,
    SdiClipTransform, SdiCore, SdiRenderTarget, TextureId, bitmap_measure_text,
    stacks::{ClipStack, DrawState, TranslateStack},
    texture_not_found,
};
use oasis_types::error::{OasisError, Result};
use oasis_types::geometry::ClipRect;
use std::collections::{HashMap, HashSet};

/// Type alias for a custom `measure_text` callback.
type MeasureTextFn = Box<dyn Fn(&str, u16) -> u32>;

/// What a render-target bind saved from the surface below it.
struct BoundTarget {
    id: RenderTargetId,
    /// The parent's clip and translate stacks.
    parent_state: DrawState,
    /// The parent's viewport size.
    parent_size: (u32, u32),
}

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
    live_textures: HashSet<u64>,
    measure_text_fn: Option<MeasureTextFn>,
    // Render-target pool state: live ids -> size.
    next_render_target_id: u64,
    live_render_targets: HashMap<u64, (u32, u32)>,
    render_target_bind_stack: Vec<BoundTarget>,
    render_targets_supported: bool,
}

impl RecordingBackend {
    /// Create a new recording backend with the given viewport dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            commands: Vec::new(),
            clip_stack: ClipStack::default(),
            translate_stack: TranslateStack::new(),
            next_texture_id: 1,
            live_textures: HashSet::new(),
            measure_text_fn: None,
            next_render_target_id: 1,
            live_render_targets: HashMap::new(),
            render_target_bind_stack: Vec::new(),
            render_targets_supported: true,
        }
    }

    /// Make [`SdiRenderTarget::supports_render_targets`] report `false`.
    ///
    /// Lets tests exercise the immediate-mode fallback of callers that
    /// probe the capability before using offscreen targets. The
    /// explicit render-target methods keep working so unrelated tests
    /// are unaffected.
    pub fn disable_render_targets(&mut self) {
        self.render_targets_supported = false;
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
        self.render_target_bind_stack.last().map(|b| b.id)
    }

    /// Return all recorded draw commands.
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Number of recorded [`DrawCommand::FillRect`]s (default methods such
    /// as `fill_rounded_rect` decompose into several).
    pub fn fill_rect_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::FillRect { .. }))
            .count()
    }

    /// Colors of every recorded [`DrawCommand::FillRect`], in draw order.
    pub fn fill_colors(&self) -> Vec<Color> {
        self.commands
            .iter()
            .filter_map(|c| match c {
                DrawCommand::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect()
    }

    /// Number of recorded [`DrawCommand::DrawText`]s.
    pub fn draw_text_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::DrawText { .. }))
            .count()
    }

    /// Text of every recorded [`DrawCommand::DrawText`], in draw order.
    pub fn texts(&self) -> Vec<&str> {
        self.text_positions().into_iter().map(|t| t.0).collect()
    }

    /// `(text, x, y, font_size)` of every recorded
    /// [`DrawCommand::DrawText`], in draw order.
    pub fn text_positions(&self) -> Vec<(&str, i32, i32, u16)> {
        self.commands
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText {
                    text,
                    x,
                    y,
                    font_size,
                    ..
                } => Some((text.as_str(), *x, *y, *font_size)),
                _ => None,
            })
            .collect()
    }

    /// Colors of every recorded [`DrawCommand::DrawText`], in draw order.
    pub fn text_colors(&self) -> Vec<Color> {
        self.commands
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText { color, .. } => Some(*color),
                _ => None,
            })
            .collect()
    }

    /// Whether any recorded [`DrawCommand::DrawText`] contains `needle`.
    pub fn has_text(&self, needle: &str) -> bool {
        self.text_positions().iter().any(|t| t.0.contains(needle))
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

impl Default for RecordingBackend {
    /// A recording backend with the default (PSP-native 480x272) viewport.
    fn default() -> Self {
        Self::new(DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT)
    }
}

impl SdiCore for RecordingBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.clip_stack = ClipStack::default();
        self.translate_stack = TranslateStack::new();
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

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.commands.push(DrawCommand::SetClip { x, y, w, h });
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.commands.push(DrawCommand::ResetClip);
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

    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl RecordingBackend {
    /// Reject `op` on a target that is on the bind chain, as the real
    /// backends do (UE5 and WASM take a bound target out of storage).
    fn reject_bound(&self, op: &str, id: RenderTargetId) -> Result<()> {
        if self.render_target_bind_stack.iter().any(|b| b.id == id) {
            return Err(OasisError::Backend(
                format!("{op}: render target {} is bound", id.0).into(),
            ));
        }
        Ok(())
    }
}

impl SdiRenderTarget for RecordingBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        let id = RenderTargetId(self.next_render_target_id);
        self.next_render_target_id += 1;
        self.live_render_targets.insert(id.0, (w, h));
        self.commands
            .push(DrawCommand::CreateRenderTarget { id, w, h });
        Ok(id)
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        let Some(&size) = self.live_render_targets.get(&id.0) else {
            return Err(OasisError::Backend(
                format!("bind_render_target: unknown id {id:?}").into(),
            ));
        };
        self.reject_bound("bind_render_target", id)?;
        // The target starts untranslated and unclipped in its own
        // coordinates; the parent's state comes back on unbind.
        self.render_target_bind_stack.push(BoundTarget {
            id,
            parent_state: DrawState::take(&mut self.clip_stack, &mut self.translate_stack),
            parent_size: (self.width, self.height),
        });
        (self.width, self.height) = size;
        self.commands.push(DrawCommand::BindRenderTarget { id });
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let Some(bound) = self.render_target_bind_stack.pop() else {
            return Err(OasisError::Backend(
                "unbind_render_target: bind stack underflow".into(),
            ));
        };
        bound
            .parent_state
            .restore(&mut self.clip_stack, &mut self.translate_stack);
        (self.width, self.height) = bound.parent_size;
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
        if !self.live_render_targets.contains_key(&id.0) {
            return Err(OasisError::Backend(
                format!("composite_render_target: unknown id {id:?}").into(),
            ));
        }
        self.reject_bound("composite_render_target", id)?;
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

    fn composite_render_target_premultiplied(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        if !self.live_render_targets.contains_key(&id.0) {
            return Err(OasisError::Backend(
                format!("composite_render_target_premultiplied: unknown id {id:?}").into(),
            ));
        }
        self.reject_bound("composite_render_target_premultiplied", id)?;
        self.commands
            .push(DrawCommand::CompositeRenderTargetPremultiplied {
                id,
                dst_x,
                dst_y,
                dst_w,
                dst_h,
            });
        Ok(())
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        let Some(&(w, h)) = self.live_render_targets.get(&id.0) else {
            return Err(OasisError::Backend(
                format!("read_render_target: unknown id {id:?}").into(),
            ));
        };
        self.reject_bound("read_render_target", id)?;
        if dst.len() as u64 != w as u64 * h as u64 * 4 {
            return Err(OasisError::Backend(
                format!(
                    "read_render_target: dst holds {} bytes, a {w}x{h} target needs {}",
                    dst.len(),
                    w as u64 * h as u64 * 4
                )
                .into(),
            ));
        }
        // Recording backend never stores pixel contents; return zeros
        // so callers that exercise the readback path still get
        // deterministic bytes.
        dst.fill(0);
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        self.reject_bound("destroy_render_target", id)?;
        if self.live_render_targets.remove(&id.0).is_none() {
            return Err(OasisError::Backend(
                format!("destroy_render_target: unknown id {id:?}").into(),
            ));
        }
        self.commands.push(DrawCommand::DestroyRenderTarget { id });
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        self.render_targets_supported
    }

    fn supports_render_target_readback(&self) -> bool {
        true
    }
}

oasis_types::impl_sdi_defaults!(RecordingBackend; except SdiClipTransform, SdiRenderTarget);

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

    fn current_clip_rect(&self) -> Option<ClipRect> {
        self.clip_stack.current()
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
