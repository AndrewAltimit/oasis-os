//! Headless software backend for the e2e harness.
//!
//! [`HeadlessBackend`] wraps the UE5 software RGBA framebuffer
//! ([`Ue5Backend`], the same rasterizer the FFI embeds) and forwards every
//! SDI call to it unchanged, so what the harness renders is what a real
//! software host renders. On top of that it:
//!
//! - records every string passed to `draw_text` during a frame, so tests
//!   can assert on text painted outside the SDI scene graph (window
//!   content, vector overlays) with [`HeadlessBackend::frame_text`];
//! - counts presented frames;
//! - implements [`InputBackend`] from a caller-filled queue (the boot
//!   splash polls input for its skip gesture);
//! - implements [`ShellBackend`]: a window resize re-creates the
//!   framebuffer at the new size and re-uploads every live texture under
//!   its original id (like an SDL window resize, which keeps the
//!   renderer's textures); the host pointer is a no-op.

use std::collections::{BTreeMap, VecDeque};

use oasis_backend_sdl::shader_bridge::ShaderBlitTarget;
use oasis_backend_ue5::Ue5Backend;
use oasis_core::backend::{
    BatchRect, BlendMode, Color, GradientStyle, InputBackend, RenderTargetId, SdiAlpha, SdiBatch,
    SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures,
    SdiVector, TextureId,
};
use oasis_core::error::Result;
use oasis_core::input::InputEvent;

use crate::shell_backend::ShellBackend;

/// One `draw_text` call recorded by [`HeadlessBackend`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawnText {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub font_size: u16,
    pub color: Color,
}

/// Software framebuffer backend with text recording (see module docs).
pub struct HeadlessBackend {
    inner: Ue5Backend,
    /// Text drawn since the last `clear`.
    drawing: Vec<DrawnText>,
    /// Text of the last presented frame.
    presented: Vec<DrawnText>,
    frames_presented: u64,
    input: VecDeque<InputEvent>,
    /// Copy of every live texture (id -> w, h, rgba) so a resize can
    /// rebuild the framebuffer without invalidating texture ids.
    textures: BTreeMap<u64, (u32, u32, Vec<u8>)>,
}

impl HeadlessBackend {
    /// Create a `width` x `height` framebuffer.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            inner: Ue5Backend::new(width, height),
            drawing: Vec::new(),
            presented: Vec::new(),
            frames_presented: 0,
            input: VecDeque::new(),
            textures: BTreeMap::new(),
        }
    }

    /// The RGBA8 framebuffer (row-major, `width * 4` bytes per row).
    pub fn pixels(&self) -> &[u8] {
        self.inner.buffer()
    }

    /// Framebuffer dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        self.inner.dimensions()
    }

    /// Text drawn in the last presented frame.
    pub fn frame_text(&self) -> &[DrawnText] {
        &self.presented
    }

    /// Number of `swap_buffers` calls so far.
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    /// Record a text draw for [`Self::frame_text`].
    fn record_text(&mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color) {
        // Fully transparent text (glyph-cache warm-up) paints nothing.
        if color.a > 0 && !text.is_empty() {
            self.drawing.push(DrawnText {
                text: text.to_string(),
                x,
                y,
                font_size,
                color,
            });
        }
    }

    /// Queue an input event for the next `poll_events`.
    pub fn push_input(&mut self, event: InputEvent) {
        self.input.push_back(event);
    }
}

impl InputBackend for HeadlessBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        self.input.drain(..).collect()
    }
}

impl ShaderBlitTarget for HeadlessBackend {}

impl ShellBackend for HeadlessBackend {
    const NAME: &'static str = "Headless";

    fn set_host_cursor_visible(&mut self, _visible: bool) {}

    fn set_window_size(&mut self, width: u32, height: u32) -> Result<()> {
        // The software framebuffer cannot be resized in place: build a new
        // one and re-upload every live texture. The UE5 backend hands out
        // the lowest free slot as the id, so uploading ids 0..=max in order
        // (with throwaway fillers for the gaps) reproduces every id.
        let mut fresh = Ue5Backend::new(width, height);
        let max = self.textures.keys().next_back().copied();
        let mut fillers = Vec::new();
        if let Some(max) = max {
            for id in 0..=max {
                let got = match self.textures.get(&id) {
                    Some((w, h, data)) => fresh.load_texture(*w, *h, data)?,
                    None => {
                        let filler = fresh.load_texture(1, 1, &[0, 0, 0, 0])?;
                        fillers.push(filler);
                        filler
                    },
                };
                debug_assert_eq!(got.0, id, "texture id preserved across resize");
            }
        }
        for filler in fillers {
            fresh.destroy_texture(filler)?;
        }
        self.inner = fresh;
        Ok(())
    }
}

impl SdiCore for HeadlessBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.inner.init(width, height)
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.drawing.clear();
        self.inner.clear(color)
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        self.inner.fill_rect(x, y, w, h, color)
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> Result<()> {
        self.record_text(text, x, y, font_size, color);
        self.inner.draw_text(text, x, y, font_size, color)
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.inner.blit(tex, x, y, w, h)
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.presented = std::mem::take(&mut self.drawing);
        self.frames_presented += 1;
        self.inner.swap_buffers()
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let id = self.inner.load_texture(width, height, rgba_data)?;
        self.textures
            .insert(id.0, (width, height, rgba_data.to_vec()));
        Ok(id)
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.textures.remove(&tex.0);
        self.inner.destroy_texture(tex)
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.inner.set_clip_rect(x, y, w, h)
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.inner.reset_clip_rect()
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        self.inner.measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        self.inner.read_pixels(x, y, w, h)
    }

    fn shutdown(&mut self) -> Result<()> {
        self.inner.shutdown()
    }
}

impl SdiShapes for HeadlessBackend {
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        self.inner.fill_rounded_rect(x, y, w, h, radius, color)
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
        self.inner.stroke_rect(x, y, w, h, stroke_width, color)
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
        self.inner.draw_line(x1, y1, x2, y2, width, color)
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        self.inner.fill_circle(cx, cy, radius, color)
    }

    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        self.inner
            .stroke_circle(cx, cy, radius, stroke_width, color)
    }

    #[allow(clippy::too_many_arguments)]
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
        self.inner.fill_triangle(x1, y1, x2, y2, x3, y3, color)
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        self.inner
            .stroke_rounded_rect(x, y, w, h, radius, stroke_width, color)
    }
}

impl SdiVector for HeadlessBackend {
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        self.inner.fill_polygon(points, color)
    }
}

impl SdiGradients for HeadlessBackend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.inner.fill_rect_gradient(x, y, w, h, gradient)
    }

    fn fill_rounded_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.inner
            .fill_rounded_rect_gradient(x, y, w, h, radius, gradient)
    }
}

impl SdiAlpha for HeadlessBackend {
    fn viewport_size(&self) -> (u32, u32) {
        self.inner.viewport_size()
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.inner.dim_screen(alpha)
    }
}

impl SdiText for HeadlessBackend {
    #[allow(clippy::too_many_arguments)]
    fn draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        self.record_text(text, x, y, font_size, color);
        self.inner
            .draw_text_styled(text, x, y, font_size, color, bold, italic)
    }

    fn measure_text_height(&self, font_size: u16) -> u32 {
        self.inner.measure_text_height(font_size)
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        self.inner.font_ascent(font_size)
    }
}

impl SdiTextures for HeadlessBackend {
    #[allow(clippy::too_many_arguments)]
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
        self.inner
            .blit_sub(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
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
        self.inner.blit_tinted(tex, x, y, w, h, tint)
    }

    #[allow(clippy::too_many_arguments)]
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
        self.inner.blit_sub_tinted(
            tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h, tint,
        )
    }

    #[allow(clippy::too_many_arguments)]
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
        self.inner.blit_flipped(tex, x, y, w, h, flip_h, flip_v)
    }
}

impl SdiClipTransform for HeadlessBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.inner.push_clip_rect(x, y, w, h)
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.inner.pop_clip_rect()
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.inner.current_clip_rect()
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.inner.push_translate(dx, dy)
    }

    fn pop_translate(&mut self) -> Result<()> {
        self.inner.pop_translate()
    }

    fn current_translate(&self) -> (i32, i32) {
        self.inner.current_translate()
    }
}

impl SdiBatch for HeadlessBackend {
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        self.inner.submit_rect_batch(rects)
    }
}

impl SdiRenderTarget for HeadlessBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        self.inner.create_render_target(w, h)
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        self.inner.bind_render_target(id)
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        self.inner.unbind_render_target()
    }

    #[allow(clippy::too_many_arguments)]
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
        self.inner
            .composite_render_target(id, dst_x, dst_y, dst_w, dst_h, blend, opacity)
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        self.inner.read_render_target(id, dst)
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        self.inner.destroy_render_target(id)
    }

    fn supports_render_targets(&self) -> bool {
        self.inner.supports_render_targets()
    }

    fn supports_render_target_readback(&self) -> bool {
        self.inner.supports_render_target_readback()
    }
}
