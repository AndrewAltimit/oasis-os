//! Headless software backend for the e2e harness.
//!
//! [`HeadlessBackend`] wraps the UE5 software RGBA framebuffer
//! ([`Ue5Backend`], the same rasterizer the FFI embeds) and forwards
//! *every* SDI method to it, defaults included, so what the harness
//! renders is exactly what a real software host renders: an override
//! added to `Ue5Backend` reaches the harness without touching this file.
//! On top of that it:
//!
//! - turns on the UE5 text log, so tests can assert on text painted
//!   outside the SDI scene graph (window content, vector overlays) with
//!   [`HeadlessBackend::frame_text`];
//! - implements [`InputBackend`] from a caller-filled queue (the boot
//!   splash polls input for its skip gesture);
//! - implements [`ShellBackend`]: a window resize resizes the framebuffer
//!   in place and keeps every texture id (like an SDL window resize,
//!   which keeps the renderer's textures); the host pointer is a no-op.

use std::collections::VecDeque;

use oasis_backend_sdl::shader_bridge::ShaderBlitTarget;
pub use oasis_backend_ue5::DrawnText;
use oasis_backend_ue5::Ue5Backend;
use oasis_core::backend::{
    BatchRect, BatchText, BlendMode, Color, GradientStyle, InputBackend, RenderTargetId, SdiAlpha,
    SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes, SdiText,
    SdiTextures, SdiVector, TextMetrics, TextureId,
};
use oasis_core::error::Result;
use oasis_core::input::InputEvent;

use crate::shell_backend::ShellBackend;

/// Software framebuffer backend with text recording (see module docs).
pub struct HeadlessBackend {
    inner: Ue5Backend,
    input: VecDeque<InputEvent>,
}

impl HeadlessBackend {
    /// Create a `width` x `height` framebuffer.
    pub fn new(width: u32, height: u32) -> Self {
        let mut inner = Ue5Backend::new(width, height);
        inner.enable_text_log();
        Self {
            inner,
            input: VecDeque::new(),
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
        self.inner.presented_text()
    }

    /// Number of `swap_buffers` calls so far.
    pub fn frames_presented(&self) -> u64 {
        self.inner.frames_presented()
    }

    /// Number of live textures (loaded and not yet destroyed). Leak
    /// checks compare this against a baseline.
    pub fn live_textures(&self) -> usize {
        self.inner.live_textures()
    }

    /// Number of live offscreen render targets.
    pub fn live_render_targets(&self) -> usize {
        self.inner.live_render_targets()
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
        // Resizes in place: texture and render-target ids stay valid.
        self.inner.init(width, height)
    }
}

// ---------------------------------------------------------------------------
// SDI: forward every method (defaults included) to the UE5 backend
// ---------------------------------------------------------------------------

/// Forward `&mut self` methods to `self.inner`.
macro_rules! forward_mut {
    ($(fn $name:ident(&mut self $(, $arg:ident: $ty:ty)*) $(-> $ret:ty)?;)*) => {
        $(
            fn $name(&mut self $(, $arg: $ty)*) $(-> $ret)? {
                self.inner.$name($($arg),*)
            }
        )*
    };
}

/// Forward `&self` methods to `self.inner`.
macro_rules! forward_ref {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $ret:ty;)*) => {
        $(
            fn $name(&self $(, $arg: $ty)*) -> $ret {
                self.inner.$name($($arg),*)
            }
        )*
    };
}

impl SdiCore for HeadlessBackend {
    forward_mut! {
        fn init(&mut self, width: u32, height: u32) -> Result<()>;
        fn clear(&mut self, color: Color) -> Result<()>;
        fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
        fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()>;
        fn draw_text(
            &mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color
        ) -> Result<()>;
        fn swap_buffers(&mut self) -> Result<()>;
        fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId>;
        fn destroy_texture(&mut self, tex: TextureId) -> Result<()>;
        fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
        fn reset_clip_rect(&mut self) -> Result<()>;
        fn shutdown(&mut self) -> Result<()>;
    }
    forward_ref! {
        fn measure_text(&self, text: &str, font_size: u16) -> u32;
        fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>>;
    }
}

#[allow(clippy::too_many_arguments)]
impl SdiShapes for HeadlessBackend {
    forward_mut! {
        fn fill_rounded_rect(
            &mut self, x: i32, y: i32, w: u32, h: u32, radius: u16, color: Color
        ) -> Result<()>;
        fn stroke_rect(
            &mut self, x: i32, y: i32, w: u32, h: u32, stroke_width: u16, color: Color
        ) -> Result<()>;
        fn stroke_rounded_rect(
            &mut self, x: i32, y: i32, w: u32, h: u32, radius: u16, stroke_width: u16,
            color: Color
        ) -> Result<()>;
        fn draw_line(
            &mut self, x1: i32, y1: i32, x2: i32, y2: i32, width: u16, color: Color
        ) -> Result<()>;
        fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()>;
        fn fill_triangle(
            &mut self, x1: i32, y1: i32, x2: i32, y2: i32, x3: i32, y3: i32, color: Color
        ) -> Result<()>;
        fn fill_shadow(
            &mut self, x: i32, y: i32, w: u32, h: u32, blur: f32, spread: f32,
            offset_x: f32, offset_y: f32, color: Color, radius: f32
        ) -> Result<()>;
        fn stroke_circle(
            &mut self, cx: i32, cy: i32, radius: u16, stroke_width: u16, color: Color
        ) -> Result<()>;
    }
}

impl SdiGradients for HeadlessBackend {
    forward_mut! {
        fn fill_rect_gradient(
            &mut self, x: i32, y: i32, w: u32, h: u32, gradient: &GradientStyle
        ) -> Result<()>;
        fn fill_rounded_rect_gradient(
            &mut self, x: i32, y: i32, w: u32, h: u32, radius: u16, gradient: &GradientStyle
        ) -> Result<()>;
    }
}

impl SdiAlpha for HeadlessBackend {
    forward_mut! {
        fn fill_rect_alpha(
            &mut self, x: i32, y: i32, w: u32, h: u32, color: Color, alpha: u8
        ) -> Result<()>;
        fn dim_screen(&mut self, alpha: u8) -> Result<()>;
    }
    forward_ref! {
        fn viewport_size(&self) -> (u32, u32);
    }
}

#[allow(clippy::too_many_arguments)]
impl SdiText for HeadlessBackend {
    forward_mut! {
        fn set_font(&mut self, font: Option<&[u8]>);
        fn draw_text_ellipsis(
            &mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color, max_width: u32
        ) -> Result<u32>;
        fn draw_text_styled(
            &mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color, bold: bool,
            italic: bool
        ) -> Result<()>;
        fn draw_text_wrapped(
            &mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color,
            max_width: u32, line_height: u32
        ) -> Result<u32>;
    }
    forward_ref! {
        fn measure_text_height(&self, font_size: u16) -> u32;
        fn font_ascent(&self, font_size: u16) -> u32;
        fn text_metrics(&self, text: &str, font_size: u16) -> TextMetrics;
        fn measure_text_extents(&self, text: &str, font_size: u16) -> (u32, u32);
    }
}

#[allow(clippy::too_many_arguments)]
impl SdiTextures for HeadlessBackend {
    forward_mut! {
        fn blit_sub(
            &mut self, tex: TextureId, src_x: u32, src_y: u32, src_w: u32, src_h: u32,
            dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32
        ) -> Result<()>;
        fn blit_tinted(
            &mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32, tint: Color
        ) -> Result<()>;
        fn blit_sub_tinted(
            &mut self, tex: TextureId, src_x: u32, src_y: u32, src_w: u32, src_h: u32,
            dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32, tint: Color
        ) -> Result<()>;
        fn blit_flipped(
            &mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32, flip_h: bool,
            flip_v: bool
        ) -> Result<()>;
    }
}

impl SdiClipTransform for HeadlessBackend {
    forward_mut! {
        fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
        fn pop_clip_rect(&mut self) -> Result<()>;
        fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()>;
        fn pop_translate(&mut self) -> Result<()>;
        fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
        fn pop_region(&mut self) -> Result<()>;
    }
    forward_ref! {
        fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)>;
        fn current_translate(&self) -> (i32, i32);
    }
}

#[allow(clippy::too_many_arguments)]
impl SdiVector for HeadlessBackend {
    forward_mut! {
        fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()>;
        fn stroke_polygon(&mut self, points: &[(i32, i32)], width: u16, color: Color) -> Result<()>;
        fn stroke_polyline(
            &mut self, points: &[(i32, i32)], width: u16, color: Color
        ) -> Result<()>;
        fn fill_arc(
            &mut self, cx: i32, cy: i32, radius: u16, start_angle: f32, end_angle: f32,
            color: Color
        ) -> Result<()>;
        fn stroke_arc(
            &mut self, cx: i32, cy: i32, radius: u16, start_angle: f32, end_angle: f32,
            width: u16, color: Color
        ) -> Result<()>;
        fn stroke_line_dashed(
            &mut self, x1: i32, y1: i32, x2: i32, y2: i32, width: u16, color: Color, dash: u16,
            gap: u16
        ) -> Result<()>;
        fn fill_polygon_gradient(
            &mut self, points: &[(i32, i32)], color_start: Color, color_end: Color
        ) -> Result<()>;
    }
}

impl SdiBatch for HeadlessBackend {
    forward_mut! {
        fn begin_batch(&mut self) -> Result<()>;
        fn flush_batch(&mut self) -> Result<()>;
        fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()>;
        fn submit_text_batch(
            &mut self, texts: &[BatchText<'_>], font_size: u16, bold: bool, italic: bool
        ) -> Result<()>;
    }
}

#[allow(clippy::too_many_arguments)]
impl SdiRenderTarget for HeadlessBackend {
    forward_mut! {
        fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId>;
        fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()>;
        fn unbind_render_target(&mut self) -> Result<()>;
        fn composite_render_target(
            &mut self, id: RenderTargetId, dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32,
            blend: BlendMode, opacity: f32
        ) -> Result<()>;
        fn composite_render_target_premultiplied(
            &mut self, id: RenderTargetId, dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32
        ) -> Result<()>;
        fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()>;
        fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()>;
    }
    forward_ref! {
        fn supports_render_targets(&self) -> bool;
        fn supports_render_target_readback(&self) -> bool;
    }
}
