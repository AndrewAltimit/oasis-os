//! `ClipAuditBackend` -- verifies that drawing stays inside a region.
//!
//! Hosts such as the window manager give an app a content rectangle and
//! push it as a clip before calling the app's draw routine. A real backend
//! then discards any pixel outside the clip, so an app that *draws* past
//! its edges is harmless -- unless it escapes the clip: pops more clips
//! than it pushed, resets the clip with [`SdiCore::reset_clip_rect`], or
//! replaces it with [`SdiCore::set_clip_rect`]. Those bugs paint over
//! neighbouring windows and the desktop.
//!
//! This backend models clipping exactly like the SDL and UE5 backends do:
//! a single *active* clip that [`SdiCore::set_clip_rect`] /
//! [`SdiCore::reset_clip_rect`] overwrite directly, plus the shared
//! [`ClipStack`] / [`TranslateStack`] behind `push_clip_rect` /
//! `pop_clip_rect` (which also overwrite the active clip -- the stack does
//! not know about a clip installed with `set_clip_rect`). For every
//! primitive that would put pixels on screen it computes the *visible* part (primitive bounds intersected with the effective clip).
//! Any visible pixel outside the audited region is recorded as a
//! [`ClipViolation`].
//!
//! Draw commands are recorded in **absolute screen coordinates** (the
//! translate stack already applied), which makes it easy for tests to find
//! where a label was drawn and click it.

use oasis_types::backend::{
    BlendMode, Color, DrawCommand, RenderTargetId, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore,
    SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures, SdiVector, TextureId,
    bitmap_measure_text,
    stacks::{ClipPush, ClipStack, TranslateStack},
};
use oasis_types::error::{OasisError, Result};
use oasis_types::geometry::ClipRect;

/// A draw that put visible pixels outside the audited region, or a clip
/// stack misuse that would let later draws do so.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipViolation {
    /// Which primitive / call caused the violation.
    pub what: String,
    /// Visible bounds of the offending draw (absolute), if it was a draw.
    pub visible: Option<(i32, i32, u32, u32)>,
}

/// A text draw recorded in absolute coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnText {
    /// The string drawn.
    pub text: String,
    /// Absolute left edge.
    pub x: i32,
    /// Absolute top edge.
    pub y: i32,
    /// Measured width in pixels.
    pub w: u32,
    /// Line height in pixels.
    pub h: u32,
    /// Font size requested.
    pub font_size: u16,
    /// Text color.
    pub color: Color,
    /// Whether any of the text was visible (inside the effective clip).
    pub visible: bool,
}

impl DrawnText {
    /// Centre point of the text's bounding box.
    #[must_use]
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w as i32 / 2, self.y + self.h as i32 / 2)
    }
}

/// A `fill_rect` (color `Some`) or `blit` (color `None`) in absolute
/// coordinates, with the part left visible by the active clip.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnRect {
    /// Absolute left edge.
    pub x: i32,
    /// Absolute top edge.
    pub y: i32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
    /// Fill color (`None` for texture blits).
    pub color: Option<Color>,
    /// Visible part `(x, y, w, h)` after clipping, `None` if fully clipped.
    pub visible: Option<(i32, i32, u32, u32)>,
}

/// A backend that audits every primitive against an allowed region.
///
/// See the module documentation for the rules.
pub struct ClipAuditBackend {
    width: u32,
    height: u32,
    region: ClipRect,
    active_clip: Option<ClipRect>,
    clip_stack: ClipStack,
    clip_depth: usize,
    translate_stack: TranslateStack,
    translate_depth: usize,
    commands: Vec<DrawCommand>,
    texts: Vec<DrawnText>,
    rects: Vec<DrawnRect>,
    violations: Vec<ClipViolation>,
    next_texture_id: u64,
    live_textures: std::collections::HashSet<u64>,
    next_render_target_id: u64,
    live_render_targets: std::collections::HashSet<u64>,
    rt_bind_depth: usize,
    auditing: bool,
}

impl ClipAuditBackend {
    /// Create an audit backend for a `width` x `height` screen that only
    /// allows visible drawing inside `(x, y, w, h)`.
    ///
    /// The region is **not** pushed as a clip automatically -- call
    /// [`Self::push_host_clip`] to emulate a window manager that clips the
    /// app, or leave it unpushed to check that the app keeps to its region
    /// on its own.
    #[must_use]
    pub fn new(width: u32, height: u32, region: (i32, i32, u32, u32)) -> Self {
        Self {
            width,
            height,
            region: ClipRect {
                x: region.0,
                y: region.1,
                w: region.2,
                h: region.3,
            },
            active_clip: None,
            clip_stack: ClipStack::new(width, height),
            clip_depth: 0,
            translate_stack: TranslateStack::new(),
            translate_depth: 0,
            commands: Vec::new(),
            texts: Vec::new(),
            rects: Vec::new(),
            violations: Vec::new(),
            next_texture_id: 1,
            live_textures: std::collections::HashSet::new(),
            next_render_target_id: 1,
            live_render_targets: std::collections::HashSet::new(),
            rt_bind_depth: 0,
            auditing: true,
        }
    }

    /// Change the audited region (e.g. to audit one window at a time).
    pub fn set_region(&mut self, region: (i32, i32, u32, u32)) {
        self.region = ClipRect {
            x: region.0,
            y: region.1,
            w: region.2,
            h: region.3,
        };
    }

    /// Enable or disable violation reporting (drawing is still recorded,
    /// including each draw's visible part). Useful when the caller draws
    /// host chrome outside the region and inspects [`Self::rects`].
    pub fn set_auditing(&mut self, on: bool) {
        self.auditing = on;
    }

    /// Push the audited region as a clip, like a window manager does before
    /// asking an app to draw its content.
    pub fn push_host_clip(&mut self) {
        let r = self.region;
        let _ = self.push_clip_rect(r.x, r.y, r.w, r.h);
    }

    /// Pop the host clip pushed by [`Self::push_host_clip`] and verify the
    /// app left the clip / translate stacks balanced.
    pub fn pop_host_clip(&mut self) {
        if self.clip_depth != 1 {
            self.violations.push(ClipViolation {
                what: format!(
                    "unbalanced clip stack: depth {} after draw",
                    self.clip_depth
                ),
                visible: None,
            });
        }
        if self.translate_depth != 0 {
            self.violations.push(ClipViolation {
                what: format!(
                    "unbalanced translate stack: depth {} after draw",
                    self.translate_depth
                ),
                visible: None,
            });
        }
        if self.rt_bind_depth != 0 {
            self.violations.push(ClipViolation {
                what: format!("render target left bound (depth {})", self.rt_bind_depth),
                visible: None,
            });
        }
        let _ = self.pop_clip_rect();
    }

    /// Recorded commands, in absolute coordinates.
    #[must_use]
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Recorded text draws, in absolute coordinates.
    #[must_use]
    pub fn texts(&self) -> &[DrawnText] {
        &self.texts
    }

    /// Recorded `fill_rect` / `blit` draws with their visible part.
    #[must_use]
    pub fn rects(&self) -> &[DrawnRect] {
        &self.rects
    }

    /// Violations found so far.
    #[must_use]
    pub fn violations(&self) -> &[ClipViolation] {
        &self.violations
    }

    /// Number of textures loaded and not destroyed.
    #[must_use]
    pub fn live_texture_count(&self) -> usize {
        self.live_textures.len()
    }

    /// First visible text draw whose string equals `text` exactly.
    #[must_use]
    pub fn find_text(&self, text: &str) -> Option<&DrawnText> {
        self.texts.iter().find(|t| t.visible && t.text == text)
    }

    /// First visible text draw whose string contains `needle`.
    #[must_use]
    pub fn find_text_containing(&self, needle: &str) -> Option<&DrawnText> {
        self.texts
            .iter()
            .find(|t| t.visible && t.text.contains(needle))
    }

    /// All visible text, joined with newlines (handy for assertions).
    #[must_use]
    pub fn visible_text(&self) -> String {
        let mut out = String::new();
        for t in self.texts.iter().filter(|t| t.visible) {
            out.push_str(&t.text);
            out.push('\n');
        }
        out
    }

    /// Clear recorded commands, texts and violations (keeps stacks).
    pub fn clear_records(&mut self) {
        self.commands.clear();
        self.texts.clear();
        self.rects.clear();
        self.violations.clear();
    }

    fn effective_clip(&self) -> ClipRect {
        self.active_clip.unwrap_or(ClipRect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        })
    }

    /// Visible part of an absolute rect, or `None` when fully clipped.
    fn visible_part(&self, x: i32, y: i32, w: u32, h: u32) -> Option<ClipRect> {
        if w == 0 || h == 0 || self.rt_bind_depth > 0 {
            // Offscreen targets are audited when composited.
            return None;
        }
        self.effective_clip().intersect(&ClipRect { x, y, w, h })
    }

    fn audit(&mut self, what: &str, x: i32, y: i32, w: u32, h: u32) -> bool {
        let Some(vis) = self.visible_part(x, y, w, h) else {
            return false;
        };
        if !self.auditing {
            return true;
        }
        let inside = vis.x >= self.region.x
            && vis.y >= self.region.y
            && vis.x + vis.w as i32 <= self.region.x + self.region.w as i32
            && vis.y + vis.h as i32 <= self.region.y + self.region.h as i32;
        if !inside {
            self.violations.push(ClipViolation {
                what: what.to_string(),
                visible: Some((vis.x, vis.y, vis.w, vis.h)),
            });
        }
        true
    }
}

impl SdiCore for ClipAuditBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.clip_stack = ClipStack::new(width, height);
        self.active_clip = None;
        self.clip_depth = 0;
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        // Clearing the whole screen from inside an app is always a leak.
        self.violations.push(ClipViolation {
            what: "clear() called while drawing app content".to_string(),
            visible: Some((0, 0, self.width, self.height)),
        });
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
        let (x, y) = self.translate_stack.translate(x, y);
        self.audit("blit", x, y, w, h);
        self.rects.push(DrawnRect {
            x,
            y,
            w,
            h,
            color: None,
            visible: self.visible_part(x, y, w, h).map(|c| (c.x, c.y, c.w, c.h)),
        });
        self.commands.push(DrawCommand::Blit { tex, x, y, w, h });
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        let (x, y) = self.translate_stack.translate(x, y);
        if color.a > 0 {
            self.audit("fill_rect", x, y, w, h);
        }
        self.rects.push(DrawnRect {
            x,
            y,
            w,
            h,
            color: Some(color),
            visible: self.visible_part(x, y, w, h).map(|c| (c.x, c.y, c.w, c.h)),
        });
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
        let (x, y) = self.translate_stack.translate(x, y);
        let w = self.measure_text(text, font_size);
        let h = self.measure_text_height(font_size);
        let visible = if color.a > 0 && !text.trim().is_empty() {
            self.audit(&format!("draw_text {text:?}"), x, y, w, h)
        } else {
            false
        };
        self.texts.push(DrawnText {
            text: text.to_string(),
            x,
            y,
            w,
            h,
            font_size,
            color,
            visible,
        });
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

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba_data.len()) {
            return Err(OasisError::Backend(
                format!(
                    "load_texture: {width}x{height} needs {expected:?} bytes, got {}",
                    rgba_data.len()
                )
                .into(),
            ));
        }
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.live_textures.insert(id);
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.live_textures.remove(&tex.0);
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // Like SDL / UE5: installs the clip directly, bypassing the stack.
        if self.clip_depth > 0 {
            self.violations.push(ClipViolation {
                what: format!("set_clip_rect({x}, {y}, {w}, {h}) replaces a pushed clip"),
                visible: None,
            });
        }
        self.active_clip = Some(ClipRect { x, y, w, h });
        self.commands.push(DrawCommand::PushClip { x, y, w, h });
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        if self.clip_depth > 0 {
            self.violations.push(ClipViolation {
                what: "reset_clip_rect() discards a pushed clip".to_string(),
                visible: None,
            });
        }
        self.active_clip = None;
        self.commands.push(DrawCommand::PopClip);
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(vec![0u8; (w as usize) * (h as usize) * 4])
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

impl SdiClipTransform for ClipAuditBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        let pushed = self.clip_stack.push(ClipRect { x: tx, y: ty, w, h });
        self.active_clip = Some(match pushed {
            ClipPush::Clip(c) => c,
            ClipPush::Empty => ClipRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
        });
        self.clip_depth += 1;
        self.commands
            .push(DrawCommand::PushClip { x: tx, y: ty, w, h });
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        if self.clip_depth == 0 {
            self.violations.push(ClipViolation {
                what: "pop_clip_rect() underflow (popped the host clip)".to_string(),
                visible: None,
            });
        } else {
            self.clip_depth -= 1;
        }
        self.active_clip = self.clip_stack.pop();
        self.commands.push(DrawCommand::PopClip);
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.active_clip.map(|c| (c.x, c.y, c.w, c.h))
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(dx, dy);
        self.translate_depth += 1;
        self.commands.push(DrawCommand::PushTranslate { dx, dy });
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        if self.translate_depth == 0 {
            self.violations.push(ClipViolation {
                what: "pop_translate() underflow".to_string(),
                visible: None,
            });
        } else {
            self.translate_depth -= 1;
        }
        self.translate_stack.pop();
        self.commands.push(DrawCommand::PopTranslate);
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.translate_stack.current()
    }
}

impl SdiRenderTarget for ClipAuditBackend {
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
        self.rt_bind_depth += 1;
        self.commands.push(DrawCommand::BindRenderTarget { id });
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        if self.rt_bind_depth == 0 {
            return Err(OasisError::Backend(
                "unbind_render_target: bind stack underflow".into(),
            ));
        }
        self.rt_bind_depth -= 1;
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
        let (x, y) = self.translate_stack.translate(dst_x, dst_y);
        self.audit("composite_render_target", x, y, dst_w, dst_h);
        self.commands.push(DrawCommand::CompositeRenderTarget {
            id,
            dst_x: x,
            dst_y: y,
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
        let (x, y) = self.translate_stack.translate(dst_x, dst_y);
        self.audit("composite_render_target_premultiplied", x, y, dst_w, dst_h);
        self.commands
            .push(DrawCommand::CompositeRenderTargetPremultiplied {
                id,
                dst_x: x,
                dst_y: y,
                dst_w,
                dst_h,
            });
        Ok(())
    }

    fn read_render_target(&mut self, _id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        dst.fill(0);
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        self.live_render_targets.remove(&id.0);
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

impl SdiShapes for ClipAuditBackend {}
impl SdiGradients for ClipAuditBackend {}
impl SdiAlpha for ClipAuditBackend {
    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
impl SdiText for ClipAuditBackend {}
impl SdiTextures for ClipAuditBackend {}
impl SdiVector for ClipAuditBackend {}
impl SdiBatch for ClipAuditBackend {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_inside_host_clip_are_fine_even_if_bounds_overflow() {
        let mut b = ClipAuditBackend::new(200, 200, (10, 10, 50, 50));
        b.push_host_clip();
        b.fill_rect(0, 0, 200, 200, Color::WHITE).ok();
        b.draw_text("hi", 20, 20, 8, Color::WHITE).ok();
        b.pop_host_clip();
        assert!(b.violations().is_empty(), "{:?}", b.violations());
        assert!(b.find_text("hi").is_some());
    }

    #[test]
    fn unclipped_overflow_is_reported() {
        let mut b = ClipAuditBackend::new(200, 200, (10, 10, 50, 50));
        b.fill_rect(0, 0, 20, 20, Color::WHITE).ok();
        assert_eq!(b.violations().len(), 1);
    }

    #[test]
    fn popping_the_host_clip_is_reported() {
        let mut b = ClipAuditBackend::new(200, 200, (10, 10, 50, 50));
        b.push_host_clip();
        b.pop_clip_rect().ok();
        b.fill_rect(0, 0, 20, 20, Color::WHITE).ok();
        b.pop_host_clip();
        assert!(b.violations().len() >= 2, "{:?}", b.violations());
    }

    #[test]
    fn reset_clip_is_reported() {
        let mut b = ClipAuditBackend::new(200, 200, (10, 10, 50, 50));
        b.push_host_clip();
        b.reset_clip_rect().ok();
        assert!(!b.violations().is_empty());
    }

    #[test]
    fn push_after_raw_set_clip_does_not_intersect_like_sdl() {
        // Mirrors SDL/UE5: a clip installed with set_clip_rect is unknown
        // to the stack, so a push/pop pair ends with clipping disabled.
        let mut b = ClipAuditBackend::new(200, 200, (10, 10, 50, 50));
        b.set_auditing(false);
        b.set_clip_rect(10, 10, 50, 50).ok();
        b.push_clip_rect(20, 20, 10, 10).ok();
        b.pop_clip_rect().ok();
        b.fill_rect(0, 0, 100, 100, Color::WHITE).ok();
        assert_eq!(b.rects()[0].visible, Some((0, 0, 100, 100)));
    }

    #[test]
    fn translate_is_applied_to_recorded_text() {
        let mut b = ClipAuditBackend::new(200, 200, (0, 0, 200, 200));
        b.push_host_clip();
        b.push_translate(30, 40).ok();
        b.draw_text("x", 1, 2, 8, Color::WHITE).ok();
        b.pop_translate().ok();
        b.pop_host_clip();
        let t = b.find_text("x").map(|t| (t.x, t.y));
        assert_eq!(t, Some((31, 42)));
        assert!(b.violations().is_empty());
    }
}
