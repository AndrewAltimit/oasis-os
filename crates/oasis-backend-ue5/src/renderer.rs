//! Software RGBA framebuffer renderer.
//!
//! Implements `SdiBackend` by drawing into a `Vec<u8>` RGBA buffer. UE5 reads
//! the buffer via `oasis_get_buffer()` and copies it to a `UTexture2D`.
//!
//! All extended primitives (rounded rects, lines, circles, triangles,
//! gradients, sub-rect blits, clip/transform stacks) are software-rasterized
//! into the pixel buffer via the shared `oasis-rasterize` crate.

use std::rc::Rc;

use oasis_core::backend::{
    BatchRect, BlendMode, Color, GradientStyle, RenderTargetId, SdiAlpha, SdiBatch,
    SdiClipTransform, SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures, SdiVector,
    TextureId, texture_not_found, validate_rgba_data,
};
use oasis_core::error::{OasisError, Result};
use oasis_rasterize::SoftwareBuffer;
use oasis_types::backend::SdiCore;
use oasis_types::backend::stacks::{ClipPush, ClipStack, TranslateStack};
use oasis_types::geometry::ClipRect;
use oasis_types::rasterize::PixelSink;
use std::collections::HashMap;

use crate::font;

/// A stored texture for later blitting.
struct Texture {
    width: u32,
    height: u32,
    data: Rc<Vec<u8>>,
}

/// Software RGBA framebuffer renderer for UE5 integration.
///
/// All rendering operations write directly to an RGBA pixel buffer.
/// The buffer is exposed to UE5 via the FFI layer. A dirty flag tracks
/// whether the buffer has changed since the last read.
pub struct Ue5Backend {
    fb: SoftwareBuffer,
    dirty: bool,
    textures: Vec<Option<Texture>>,
    clip_stack: ClipStack,
    translate_stack: TranslateStack,
    /// Offscreen render targets. Keyed by `RenderTargetId` inner u64.
    /// When a target is *bound*, its buffer is temporarily swapped into
    /// `self.fb` and the parent surface is stored on the bind stack.
    render_targets: HashMap<u64, SoftwareBuffer>,
    /// Bind stack: each entry holds the render target id that was
    /// active before the bind (`None` = framebuffer) and the
    /// corresponding swapped-out buffer.
    render_target_bind_stack: Vec<(Option<u64>, SoftwareBuffer)>,
    /// Currently bound render target id (`None` = framebuffer).
    current_render_target: Option<u64>,
    /// Monotonic counter for render-target ids.
    next_render_target_id: u64,
}

impl Ue5Backend {
    /// Create a new backend with the given resolution.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            fb: SoftwareBuffer::new(width, height),
            dirty: true,
            textures: Vec::new(),
            clip_stack: ClipStack::new(width, height),
            translate_stack: TranslateStack::new(),
            render_targets: HashMap::new(),
            render_target_bind_stack: Vec::new(),
            current_render_target: None,
            next_render_target_id: 1,
        }
    }

    /// Get a read-only reference to the RGBA pixel buffer.
    pub fn buffer(&self) -> &[u8] {
        self.fb.data()
    }

    /// Whether the buffer has been modified since the last `clear_dirty()`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag (called after UE5 reads the buffer).
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Buffer dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.fb.width(), self.fb.height())
    }

    /// Blit raw RGBA pixels into the framebuffer at the given position.
    pub fn blit_rgba(&mut self, x: u32, y: u32, w: u32, h: u32, pixels: &[u8]) {
        self.fb.blit_rgba(x, y, w, h, pixels);
        self.dirty = true;
    }

    /// Apply cumulative translation to coordinates.
    fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        self.translate_stack.translate(x, y)
    }

    /// Set a single pixel with alpha blending (delegates to
    /// [`SoftwareBuffer`]).
    #[cfg(test)]
    fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.fb.set_pixel(x, y, color);
    }

    /// Get texture data via `Rc::clone` (O(1) refcount bump, no data copy).
    fn get_texture_data(&self, tex: TextureId) -> Result<(u32, u32, Rc<Vec<u8>>)> {
        let idx = tex.0 as usize;
        let texture = self
            .textures
            .get(idx)
            .and_then(|t| t.as_ref())
            .ok_or_else(|| texture_not_found(tex.0))?;
        Ok((texture.width, texture.height, Rc::clone(&texture.data)))
    }
}

impl PixelSink for Ue5Backend {
    fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        self.fb.hline(x1, x2, y, color);
    }
}

impl SdiCore for Ue5Backend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.fb.resize(width, height);
        self.dirty = true;
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.fb.clear(color);
        self.dirty = true;
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rect(tx, ty, w, h, color);
        self.dirty = true;
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
        let (tx, ty) = self.translate(x, y);
        self.fb.draw_bitmap_text(
            text,
            tx,
            ty,
            font_size,
            color,
            font::glyph,
            font::glyph_metrics,
        );
        self.dirty = true;
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        self.fb.blit_texture(&tex_data, tex_w, tex_h, tx, ty, w, h);
        self.dirty = true;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;

        let texture = Texture {
            width,
            height,
            data: Rc::new(rgba_data.to_vec()),
        };

        for (i, slot) in self.textures.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(texture);
                return Ok(TextureId(i as u64));
            }
        }
        let id = self.textures.len();
        self.textures.push(Some(texture));
        Ok(TextureId(id as u64))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        let idx = tex.0 as usize;
        if idx < self.textures.len() {
            self.textures[idx] = None;
        }
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let clip = ClipRect { x, y, w, h };
        self.fb.set_clip(Some(clip));
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.fb.set_clip(None);
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(self.fb.read_pixels(x, y, w, h))
    }

    fn shutdown(&mut self) -> Result<()> {
        self.fb = SoftwareBuffer::new(0, 0);
        self.textures.clear();
        self.clip_stack.clear();
        self.translate_stack.clear();
        log::info!("UE5 backend shut down");
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiShapes: Shape primitives
// -------------------------------------------------------------------

impl SdiShapes for Ue5Backend {
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rounded_rect(tx, ty, w, h, radius, color);
        self.dirty = true;
        Ok(())
    }

    fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb.stroke_rect(tx, ty, w, h, stroke_width, color);
        self.dirty = true;
        Ok(())
    }

    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.fb.draw_line(tx1, ty1, tx2, ty2, width, color);
        self.dirty = true;
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.fb.fill_circle(tcx, tcy, radius, color);
        self.dirty = true;
        Ok(())
    }

    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.fb.stroke_circle(tcx, tcy, radius, stroke_width, color);
        self.dirty = true;
        Ok(())
    }

    fn fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        let v0 = self.translate(x1, y1);
        let v1 = self.translate(x2, y2);
        let v2 = self.translate(x3, y3);
        self.fb.fill_triangle(v0, v1, v2, color);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiVector: defaults (no overrides needed)
// -------------------------------------------------------------------

impl SdiVector for Ue5Backend {}

// -------------------------------------------------------------------
// SdiGradients: Gradient fills
// -------------------------------------------------------------------

impl SdiGradients for Ue5Backend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rect_gradient(tx, ty, w, h, gradient);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiAlpha: Viewport and alpha utilities
// -------------------------------------------------------------------

impl SdiAlpha for Ue5Backend {
    fn viewport_size(&self) -> (u32, u32) {
        (self.fb.width(), self.fb.height())
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(
            0,
            0,
            self.fb.width(),
            self.fb.height(),
            Color::rgba(0, 0, 0, alpha),
        )
    }
}

// -------------------------------------------------------------------
// SdiText: Text system
// -------------------------------------------------------------------

impl SdiText for Ue5Backend {
    fn measure_text_height(&self, font_size: u16) -> u32 {
        let scale = if font_size >= 8 {
            (font_size / 8) as u32
        } else {
            1
        };
        8 * scale
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        let scale = if font_size >= 8 {
            (font_size / 8) as u32
        } else {
            1
        };
        8 * scale
    }
}

// -------------------------------------------------------------------
// SdiTextures: Texture operations
// -------------------------------------------------------------------

impl SdiTextures for Ue5Backend {
    fn blit_sub(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        let (tex_w, _tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        self.fb.blit_texture_sub(
            &tex_data, tex_w, src_x, src_y, src_w, src_h, tx, ty, dst_w, dst_h,
        );
        self.dirty = true;
        Ok(())
    }

    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        self.fb
            .blit_texture_tinted(&tex_data, tex_w, tex_h, tx, ty, w, h, tint);
        self.dirty = true;
        Ok(())
    }

    fn blit_sub_tinted(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tex_w, _tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        self.fb.blit_texture_sub_tinted(
            &tex_data, tex_w, src_x, src_y, src_w, src_h, tx, ty, dst_w, dst_h, tint,
        );
        self.dirty = true;
        Ok(())
    }

    fn blit_flipped(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        self.fb
            .blit_texture_flipped(&tex_data, tex_w, tex_h, tx, ty, w, h, flip_h, flip_v);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiClipTransform: Clip and transform stack
// -------------------------------------------------------------------

impl SdiClipTransform for Ue5Backend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let new_clip = ClipRect { x: tx, y: ty, w, h };
        match self.clip_stack.push(new_clip) {
            ClipPush::Clip(c) => self.fb.set_clip(Some(c)),
            ClipPush::Empty => {
                self.fb.set_clip(Some(ClipRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                }));
            },
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.fb.set_clip(self.clip_stack.pop());
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.fb.clip().map(|c| (c.x, c.y, c.w, c.h))
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(dx, dy);
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        self.translate_stack.pop();
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.translate_stack.current()
    }
}

// -------------------------------------------------------------------
// SdiBatch: tighter inner loop than the default — caches the
// translate offset (the default re-derives it per call), batches the
// `dirty` flag flip into a single store at the end, and skips the
// per-rect SdiCore::fill_rect dispatch. The host UE5 process polls
// `oasis_get_dirty` once per tick so coalescing the dirty flip costs
// nothing visible but saves a few writes per frame.
// -------------------------------------------------------------------

impl SdiBatch for Ue5Backend {
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        if rects.is_empty() {
            return Ok(());
        }
        let (dx, dy) = self.translate_stack.current();
        let mut wrote_any = false;
        for r in rects {
            if r.w == 0 || r.h == 0 || r.color.a == 0 {
                continue;
            }
            self.fb.fill_rect(r.x + dx, r.y + dy, r.w, r.h, r.color);
            wrote_any = true;
        }
        if wrote_any {
            self.dirty = true;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiRenderTarget: Offscreen compositing layers (compositor PR4)
// -------------------------------------------------------------------

impl SdiRenderTarget for Ue5Backend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        if w == 0 || h == 0 {
            return Err(OasisError::Backend(
                format!("create_render_target: zero dimension ({w}x{h})").into(),
            ));
        }
        let id = self.next_render_target_id;
        self.next_render_target_id += 1;
        self.render_targets.insert(id, SoftwareBuffer::new(w, h));
        Ok(RenderTargetId(id))
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        // Take the target's buffer out of storage and swap it with
        // `self.fb`. The previous fb ends up held by the bind stack
        // entry and is swapped back on `unbind_render_target`.
        let mut target_fb = self.render_targets.remove(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("bind_render_target: unknown id {id:?}").into())
        })?;
        std::mem::swap(&mut self.fb, &mut target_fb);
        self.render_target_bind_stack
            .push((self.current_render_target, target_fb));
        self.current_render_target = Some(id.0);
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let (prev_id, mut saved_parent) = self.render_target_bind_stack.pop().ok_or_else(|| {
            OasisError::Backend("unbind_render_target: bind stack underflow".into())
        })?;
        // Swap the parent surface back in; the target's buffer ends up
        // in `saved_parent` which we return to the pool.
        std::mem::swap(&mut self.fb, &mut saved_parent);
        debug_assert!(
            self.current_render_target.is_some(),
            "unbind_render_target called without active target"
        );
        if let Some(active_id) = self.current_render_target {
            self.render_targets.insert(active_id, saved_parent);
        }
        self.current_render_target = prev_id;
        Ok(())
    }

    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        _blend: BlendMode,
        opacity: f32,
    ) -> Result<()> {
        let src = self.render_targets.get(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("composite_render_target: unknown id {id:?}").into())
        })?;
        let sw = src.width();
        let sh = src.height();
        debug_assert_eq!(sw, dst_w, "composite_render_target: dst_w != src width");
        debug_assert_eq!(sh, dst_h, "composite_render_target: dst_h != src height");
        let src_pixels = src.data().to_vec();
        self.fb
            .composite_rgba(dst_x, dst_y, sw, sh, &src_pixels, opacity);
        self.dirty = true;
        Ok(())
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        let src = self.render_targets.get(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("read_render_target: unknown id {id:?}").into())
        })?;
        let expected = (src.width() * src.height() * 4) as usize;
        if dst.len() < expected {
            return Err(OasisError::Backend(
                format!(
                    "read_render_target: buffer too small ({} < {expected})",
                    dst.len()
                )
                .into(),
            ));
        }
        dst[..expected].copy_from_slice(src.data());
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        if self.current_render_target == Some(id.0)
            || self
                .render_target_bind_stack
                .iter()
                .any(|(stacked, _)| *stacked == Some(id.0))
        {
            return Err(OasisError::Backend(
                format!("destroy_render_target: id {id:?} is still bound").into(),
            ));
        }
        if self.render_targets.remove(&id.0).is_none() {
            return Err(OasisError::Backend(
                format!("destroy_render_target: unknown id {id:?}").into(),
            ));
        }
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        true
    }

    fn supports_render_target_readback(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_buffer() {
        let backend = Ue5Backend::new(480, 272);
        assert_eq!(backend.buffer().len(), 480 * 272 * 4);
        assert_eq!(backend.dimensions(), (480, 272));
    }

    #[test]
    fn clear_fills_buffer() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[1], 0);
        assert_eq!(backend.buffer()[2], 0);
        assert_eq!(backend.buffer()[3], 255);
        let last = backend.buffer().len() - 4;
        assert_eq!(backend.buffer()[last], 255);
    }

    #[test]
    fn fill_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(2, 2, 3, 3, Color::rgb(0, 255, 0))
            .unwrap();
        let offset = (2 * 10 + 2) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        assert_eq!(backend.buffer()[offset + 1], 255);
        assert_eq!(backend.buffer()[0], 0);
        assert_eq!(backend.buffer()[1], 0);
    }

    #[test]
    fn fill_rect_clips_negative() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(-2, -2, 5, 5, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn draw_text_renders_characters() {
        let mut backend = Ue5Backend::new(100, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_text("A", 0, 0, 8, Color::rgb(255, 255, 255))
            .unwrap();
        let has_white = backend
            .buffer()
            .chunks_exact(4)
            .any(|px| px[0] == 255 && px[1] == 255 && px[2] == 255);
        assert!(has_white);
    }

    #[test]
    fn draw_text_scaled() {
        let mut backend = Ue5Backend::new(100, 40);
        backend.clear(Color::BLACK).unwrap();
        backend.draw_text("X", 0, 0, 16, Color::WHITE).unwrap();
        let white_count = backend
            .buffer()
            .chunks_exact(4)
            .filter(|px| px[0] == 255)
            .count();
        assert!(white_count > 20);
    }

    #[test]
    fn load_and_blit_texture() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        let tex_data = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.blit(tex_id, 1, 1, 2, 2).unwrap();
        let offset = (10 + 1) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        assert_eq!(backend.buffer()[offset + 1], 0);
    }

    #[test]
    fn destroy_texture_invalidates() {
        let mut backend = Ue5Backend::new(10, 10);
        let tex_data = vec![0u8; 2 * 2 * 4];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.destroy_texture(tex_id).unwrap();
        assert!(backend.blit(tex_id, 0, 0, 2, 2).is_err());
    }

    #[test]
    fn texture_data_size_mismatch() {
        let mut backend = Ue5Backend::new(10, 10);
        assert!(backend.load_texture(2, 2, &[0; 8]).is_err());
    }

    #[test]
    fn dirty_flag_tracking() {
        let mut backend = Ue5Backend::new(4, 4);
        assert!(backend.is_dirty());
        backend.clear_dirty();
        assert!(!backend.is_dirty());
        backend.clear(Color::BLACK).unwrap();
        assert!(backend.is_dirty());
    }

    #[test]
    fn clip_rect_restricts_drawing() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend.set_clip_rect(2, 2, 3, 3).unwrap();
        backend
            .fill_rect(0, 0, 10, 10, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 0);
        let offset = (3 * 10 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        backend.reset_clip_rect().unwrap();
        backend.fill_rect(0, 0, 1, 1, Color::WHITE).unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn shutdown_clears_state() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.shutdown().unwrap();
        assert!(backend.buffer().is_empty());
    }

    #[test]
    fn texture_slot_reuse() {
        let mut backend = Ue5Backend::new(4, 4);
        let data = vec![0u8; 4];
        let id0 = backend.load_texture(1, 1, &data).unwrap();
        let id1 = backend.load_texture(1, 1, &data).unwrap();
        backend.destroy_texture(id0).unwrap();
        let id2 = backend.load_texture(1, 1, &data).unwrap();
        assert_eq!(id2.0, id0.0);
        assert_ne!(id1.0, id2.0);
    }

    // -------------------------------------------------------------------
    // Extended primitive tests
    // -------------------------------------------------------------------

    #[test]
    fn fill_rounded_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rounded_rect(2, 2, 16, 16, 4, Color::rgb(0, 255, 0))
            .unwrap();
        // Center pixel should be green.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
        // Corner pixel (2,2) should NOT be filled (inside the radius).
        let corner = (2 * 20 + 2) * 4;
        assert_eq!(backend.buffer()[corner], 0);
    }

    #[test]
    fn draw_line_horizontal() {
        let mut backend = Ue5Backend::new(20, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(2, 5, 18, 5, 1, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (10, 5) should be red.
        let offset = (5 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn draw_line_diagonal() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(0, 0, 19, 19, 1, Color::rgb(0, 0, 255))
            .unwrap();
        // Pixel at (10, 10) should be blue.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 2], 255);
    }

    #[test]
    fn fill_circle_draws() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_circle(15, 15, 10, Color::rgb(255, 0, 0))
            .unwrap();
        // Center should be red.
        let offset = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn fill_triangle_draws() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_triangle(10, 2, 2, 18, 18, 18, Color::rgb(0, 255, 0))
            .unwrap();
        // A point inside the triangle should be green.
        let offset = (14 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
    }

    #[test]
    fn gradient_v_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Vertical {
                    top: Color::WHITE,
                    bottom: Color::BLACK,
                },
            )
            .unwrap();
        // Top pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Bottom pixel should be black.
        let last_row = (9 * 10) * 4;
        assert_eq!(backend.buffer()[last_row], 0);
    }

    #[test]
    fn gradient_h_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Horizontal {
                    left: Color::WHITE,
                    right: Color::BLACK,
                },
            )
            .unwrap();
        // Left pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Right pixel should be black.
        let right = 9 * 4;
        assert_eq!(backend.buffer()[right], 0);
    }

    #[test]
    fn clip_stack_nesting() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        // Push outer clip.
        backend.push_clip_rect(2, 2, 16, 16).unwrap();
        // Push inner clip.
        backend.push_clip_rect(5, 5, 10, 10).unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (0,0) should be black (outside both clips).
        assert_eq!(backend.buffer()[0], 0);
        // Pixel at (3,3) should be black (outside inner clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        // Pixel at (7,7) should be red (inside both clips).
        let offset = (7 * 20 + 7) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        // Pop inner clip.
        backend.pop_clip_rect().unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(0, 255, 0))
            .unwrap();
        // Pixel at (3,3) should now be green (inside outer clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);

        // Pop outer clip.
        backend.pop_clip_rect().unwrap();
    }

    #[test]
    fn translate_stack_offsets() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend.push_translate(5, 5).unwrap();
        // fill_rect at (0,0) should actually draw at (5,5).
        backend
            .fill_rect(0, 0, 2, 2, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (5,5) should be red.
        let offset = (5 * 20 + 5) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        // Pixel at (0,0) should be black.
        assert_eq!(backend.buffer()[0], 0);

        backend.push_translate(3, 3).unwrap();
        assert_eq!(backend.current_translate(), (8, 8));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (5, 5));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (0, 0));
    }

    #[test]
    fn blit_sub_draws_subregion() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 4x4 texture: top-left 2x2 is red, rest is blue.
        let mut tex_data = vec![0u8; 4 * 4 * 4];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let off = ((y * 4 + x) * 4) as usize;
                if x < 2 && y < 2 {
                    tex_data[off] = 255; // R
                    tex_data[off + 3] = 255; // A
                } else {
                    tex_data[off + 2] = 255; // B
                    tex_data[off + 3] = 255; // A
                }
            }
        }
        let tex_id = backend.load_texture(4, 4, &tex_data).unwrap();
        // Blit only the top-left 2x2 subregion.
        backend.blit_sub(tex_id, 0, 0, 2, 2, 0, 0, 2, 2).unwrap();
        // Pixel (0,0) should be red.
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[2], 0);
    }

    #[test]
    fn blit_tinted_applies_color() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 1x1 white texture.
        let tex_data = vec![255u8; 4];
        let tex_id = backend.load_texture(1, 1, &tex_data).unwrap();
        // Tint with red.
        backend
            .blit_tinted(tex_id, 0, 0, 1, 1, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255); // R
        assert_eq!(backend.buffer()[1], 0); // G
        assert_eq!(backend.buffer()[2], 0); // B
    }

    #[test]
    fn blit_flipped_horizontal() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 2x1 texture: left=red, right=blue.
        let tex_data = vec![255, 0, 0, 255, 0, 0, 255, 255];
        let tex_id = backend.load_texture(2, 1, &tex_data).unwrap();
        backend
            .blit_flipped(tex_id, 0, 0, 2, 1, true, false)
            .unwrap();
        // With horizontal flip: left should be blue, right should be red.
        assert_eq!(backend.buffer()[0], 0); // B became left
        assert_eq!(backend.buffer()[2], 255);
        assert_eq!(backend.buffer()[4], 255); // R became right
        assert_eq!(backend.buffer()[6], 0);
    }

    #[test]
    fn stroke_circle_draws_ring() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .stroke_circle(15, 15, 10, 2, Color::rgb(0, 255, 0))
            .unwrap();
        // Center should be black (hollow).
        let center = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[center], 0);
        // Edge pixel should be green.
        let edge = (15 * 30 + 25) * 4;
        assert_eq!(backend.buffer()[edge + 1], 255);
    }

    #[test]
    fn dim_screen_covers_viewport() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::WHITE).unwrap();
        backend.dim_screen(128).unwrap();
        // All pixels should be dimmed (not fully white anymore).
        assert!(backend.buffer()[0] < 255);
        assert!(backend.buffer()[0] > 0);
    }

    #[test]
    fn text_measurement() {
        let backend = Ue5Backend::new(10, 10);
        assert_eq!(backend.measure_text_height(8), 8);
        assert_eq!(backend.measure_text_height(16), 16);
        assert_eq!(backend.font_ascent(8), 8);
        let (w, h) = backend.measure_text_extents("AB", 8);
        assert_eq!(w, 14); // proportional: A(7)+B(7) = 14
        assert_eq!(h, 8);
    }

    // ---------------------------------------------------------------
    // RGBA pixel format / set_pixel color tests
    // ---------------------------------------------------------------

    #[test]
    fn rgba_buffer_layout_red() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255); // R
        assert_eq!(buf[1], 0); // G
        assert_eq!(buf[2], 0); // B
        assert_eq!(buf[3], 255); // A
    }

    #[test]
    fn rgba_buffer_layout_green() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 255, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_blue() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 0, 255)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_white() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::WHITE).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_black() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::BLACK).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_transparent_clear() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgba(0, 0, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn set_pixel_alpha_blend() {
        let mut backend = Ue5Backend::new(1, 1);
        // Start with white background.
        backend.clear(Color::WHITE).unwrap();
        // Draw 50% transparent red over it.
        backend.set_pixel(0, 0, Color::rgba(255, 0, 0, 128));
        let buf = backend.buffer();
        assert!(buf[0] > 200); // R stays high
        assert!(buf[1] > 100 && buf[1] < 140); // G blended ~127
        assert!(buf[2] > 100 && buf[2] < 140); // B blended ~127
        assert_eq!(buf[3], 255); // A always 255 after blend
    }

    #[test]
    fn set_pixel_fully_transparent_no_change() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(42, 84, 126)).unwrap();
        backend.set_pixel(0, 0, Color::rgba(255, 255, 255, 0));
        let buf = backend.buffer();
        assert_eq!(buf[0], 42);
        assert_eq!(buf[1], 84);
        assert_eq!(buf[2], 126);
    }

    #[test]
    fn set_pixel_out_of_bounds_no_crash() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::BLACK).unwrap();
        let before: Vec<u8> = backend.buffer().to_vec();
        backend.set_pixel(-1, 0, Color::WHITE);
        backend.set_pixel(0, -1, Color::WHITE);
        backend.set_pixel(4, 0, Color::WHITE);
        backend.set_pixel(0, 4, Color::WHITE);
        assert_eq!(backend.buffer(), before.as_slice());
    }

    #[test]
    fn rgba_round_trip_encode_decode() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(123, 45, 67, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        let decoded = Color::rgba(buf[0], buf[1], buf[2], buf[3]);
        assert_eq!(decoded, c);
    }

    #[test]
    fn rgba_round_trip_all_channels_max() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(255, 255, 255, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }

    #[test]
    fn rgba_round_trip_all_channels_min() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(0, 0, 0, 0);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }

    fn synthetic_rects(n: usize) -> Vec<oasis_types::backend::BatchRect> {
        (0..n)
            .map(|i| oasis_types::backend::BatchRect {
                x: ((i * 13) % 470) as i32,
                y: ((i * 7) % 262) as i32,
                w: 8,
                h: 8,
                color: Color::rgba(
                    (i * 11) as u8,
                    (i * 17) as u8,
                    (i * 23) as u8,
                    if i % 5 == 0 { 200 } else { 255 },
                ),
            })
            .collect()
    }

    /// Visual parity guarantee: the override must produce a pixel buffer
    /// that's bit-identical to the default `fill_rect` loop over the same
    /// inputs. Without this, a screenshot regression somewhere downstream
    /// (browser display list, file-manager grid) could pass on native and
    /// fail on WASM, or vice versa.
    #[test]
    fn submit_rect_batch_matches_default_loop() {
        use oasis_types::backend::SdiBatch;
        let rects = synthetic_rects(200);

        let mut a = Ue5Backend::new(480, 272);
        a.clear(Color::BLACK).unwrap();
        for r in &rects {
            a.fill_rect(r.x, r.y, r.w, r.h, r.color).unwrap();
        }

        let mut b = Ue5Backend::new(480, 272);
        b.clear(Color::BLACK).unwrap();
        b.submit_rect_batch(&rects).unwrap();

        assert_eq!(
            a.buffer(),
            b.buffer(),
            "submit_rect_batch override produced a different framebuffer \
             than the default fill_rect loop"
        );
    }

    /// Translate offsets must be applied per item by both paths or the
    /// override will silently shift everything by the wrong amount.
    #[test]
    fn submit_rect_batch_honors_translate_stack() {
        use oasis_types::backend::{SdiBatch, SdiClipTransform};
        let rects = synthetic_rects(50);

        let mut a = Ue5Backend::new(480, 272);
        a.clear(Color::BLACK).unwrap();
        a.push_translate(7, 11).unwrap();
        for r in &rects {
            a.fill_rect(r.x, r.y, r.w, r.h, r.color).unwrap();
        }
        a.pop_translate().unwrap();

        let mut b = Ue5Backend::new(480, 272);
        b.clear(Color::BLACK).unwrap();
        b.push_translate(7, 11).unwrap();
        b.submit_rect_batch(&rects).unwrap();
        b.pop_translate().unwrap();

        assert_eq!(a.buffer(), b.buffer(), "translate stack diverged");
    }
}
