//! SDL3 backend for OASIS_OS.
//!
//! Implements `SdiBackend` and `InputBackend` using SDL3. Used for desktop
//! development and Raspberry Pi deployment (via SDL3's kmsdrm or X11 backend).
//!
//! Extended primitives (rounded rects, lines, circles, triangles, gradients,
//! sub-rect blits, tinted blits, clip/transform stacks) are implemented using
//! SDL3 renderer API calls and software rasterization helpers.

mod blitting;
mod core_impl;
mod font;
mod glyph_cache;
mod gradients;
mod input;
pub mod network;
mod sdl_audio;
pub mod shader_bridge;
mod shapes;

use std::collections::HashMap;

use sdl3::EventPump;
use sdl3::render::{Canvas, FPoint, FRect, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

use oasis_core::backend::{
    ArcParams, BackendErrExt, BatchRect, BatchText, BlendMode, Color, DashStyle, RenderTargetId,
    SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiRenderTarget, SdiShapes, SdiText,
    SdiTextures, SdiVector, StrokeStyle, TextureId,
};
use oasis_core::error::{OasisError, Result};
use oasis_types::backend::stacks::{ClipPush, ClipStack, TranslateStack};
pub use oasis_types::geometry::ClipRect;

pub use network::SdlNetworkBackend;
pub use sdl_audio::SdlAudioBackend;

use crate::glyph_cache::GlyphEntry;
use oasis_rasterize::GlyphCacheKey;

// Re-export input helpers for tests.
#[cfg(test)]
use input::{map_key_down, map_key_up};
#[cfg(test)]
use oasis_types::geometry::isqrt_i32;
#[cfg(test)]
use oasis_types::rasterize::edge_x;

/// Convert integer coordinates to an `FRect` for SDL3 renderer calls.
pub(crate) fn frect(x: i32, y: i32, w: u32, h: u32) -> FRect {
    FRect::new(x as f32, y as f32, w as f32, h as f32)
}

/// Convert integer coordinates to an `FPoint` for SDL3 renderer calls.
pub(crate) fn fpoint(x: i32, y: i32) -> FPoint {
    FPoint::new(x as f32, y as f32)
}

/// SDL3 rendering and input backend.
///
/// Supports solid-color rects, 8x8 bitmap text, and RGBA texture loading/blitting.
///
/// # Safety: field declaration order matters
///
/// `Texture<'static>` lifetimes are erased via transmute in `load_texture()` --
/// the textures actually borrow from `texture_creator` in this struct.
/// An explicit `Drop` impl calls `self.textures.clear()` to destroy all textures
/// while `texture_creator` is still alive, ensuring soundness regardless of
/// field declaration order.
///
/// **Even though the `Drop` impl makes this safe today**, `textures` is declared
/// before `texture_creator` as a defense-in-depth measure: Rust drops fields in
/// declaration order, so if the explicit `Drop` impl were ever removed, the
/// textures would still be dropped before the creator. **Do not reorder these
/// two fields** without verifying the `Drop` impl is intact.
pub struct SdlBackend {
    pub(crate) canvas: Canvas<Window>,
    pub(crate) event_pump: EventPump,
    // SAFETY: Must be declared before `texture_creator`. The textures borrow
    // from texture_creator (lifetime erased via transmute in load_texture).
    // Rust drops fields in declaration order, so this field is dropped first.
    // The explicit Drop impl also clears this map, but field order provides
    // defense-in-depth. Reordering these fields without the Drop impl is UB.
    pub(crate) textures: HashMap<u64, Texture<'static>>,
    /// Offscreen render targets (compositor PR4). Same lifetime story
    /// as `textures` — the `Texture<'static>` handles borrow from
    /// `texture_creator`, must be dropped before the creator.
    pub(crate) render_targets: HashMap<u64, Texture<'static>>,
    /// Bind stack for `SdiRenderTarget`. Each entry is the `RenderTargetId`
    /// that was active before the corresponding `bind_render_target`
    /// call, i.e. popping restores the entry. `None` means "framebuffer".
    pub(crate) render_target_bind_stack: Vec<Option<u64>>,
    /// Currently bound render target (`None` = framebuffer).
    pub(crate) current_render_target: Option<u64>,
    /// Monotonic counter for render-target ids.
    pub(crate) next_render_target_id: u64,
    /// Maps a color-independent glyph key to its cached texture and metrics.
    /// Glyphs are rasterized white and tinted at blit time, so one entry
    /// serves every color the shell draws that character in.
    pub(crate) glyph_cache: HashMap<GlyphCacheKey, GlyphEntry>,
    /// Monotonic counter for LRU access tracking (bumped on every glyph draw).
    pub(crate) glyph_access_counter: u64,
    /// Active skin-provided TrueType font (`SdiText::set_font`). `None`
    /// keeps the built-in bitmap font — the pixel-identical default path.
    pub(crate) ttf_font: Option<oasis_rasterize::ttf::TtfFont>,
    // SAFETY: Must be declared after `textures` -- see comment above.
    pub(crate) texture_creator: TextureCreator<WindowContext>,
    pub(crate) next_texture_id: u64,
    pub(crate) clip_stack: ClipStack,
    pub(crate) translate_stack: TranslateStack,
    pub(crate) viewport_w: u32,
    pub(crate) viewport_h: u32,
    /// Last color passed to `set_color`, to skip redundant SDL draw-color
    /// and blend-mode calls. All canvas draw-color/blend changes must go
    /// through `set_color` (or reset this to `None`) to stay coherent.
    pub(crate) last_draw_color: Option<Color>,
}

impl SdlBackend {
    /// Create a new SDL3 backend with a window.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self> {
        let sdl = sdl3::init().backend_err()?;
        let video = sdl.video().backend_err()?;
        let window = video
            .window(title, width, height)
            .position_centered()
            .build()
            .backend_err()?;
        let canvas: Canvas<Window> = window.into_canvas();
        let texture_creator = canvas.texture_creator();
        let headless =
            std::env::var("SDL_RENDER_DRIVER").is_ok_and(|v| v.eq_ignore_ascii_case("software"));
        if !headless {
            // SAFETY: canvas.raw() returns the valid SDL_Renderer pointer owned by canvas.
            unsafe {
                sdl3::sys::render::SDL_SetRenderVSync(canvas.raw(), 1);
            }
        }

        // Enable SDL3 text input so TextInput events are generated.
        // Without this call, SDL3 only produces key-down/key-up events
        // and regular character typing does not reach the application.
        // SAFETY: canvas.window().raw() returns the valid SDL_Window
        // pointer owned by the canvas.
        unsafe {
            sdl3::sys::keyboard::SDL_StartTextInput(canvas.window().raw());
        }

        let event_pump = sdl.event_pump().backend_err()?;

        log::info!("SDL3 backend initialized: {width}x{height}");

        Ok(Self {
            canvas,
            event_pump,
            textures: HashMap::new(),
            render_targets: HashMap::new(),
            render_target_bind_stack: Vec::new(),
            current_render_target: None,
            next_render_target_id: 1,
            glyph_cache: HashMap::new(),
            glyph_access_counter: 0,
            ttf_font: None,
            texture_creator,
            next_texture_id: 1,
            clip_stack: ClipStack::new(width, height),
            translate_stack: TranslateStack::new(),
            viewport_w: width,
            viewport_h: height,
            last_draw_color: None,
        })
    }

    /// Access the underlying SDL window.
    pub fn window(&self) -> &sdl3::video::Window {
        self.canvas.window()
    }

    /// Access the texture creator (for creating streaming textures).
    pub fn texture_creator(&self) -> &TextureCreator<WindowContext> {
        &self.texture_creator
    }

    /// Access the SDL canvas mutably (for shader blit operations).
    pub fn canvas_mut(&mut self) -> &mut Canvas<Window> {
        &mut self.canvas
    }

    /// Resize the SDL window and internal viewport/clip state.
    ///
    /// The caller is responsible for updating any other subsystems that
    /// were initialized with the old dimensions (shader bridge, window
    /// manager screen size, dashboard layout, etc.).
    pub fn set_window_size(&mut self, width: u32, height: u32) -> Result<()> {
        self.canvas
            .window_mut()
            .set_size(width, height)
            .map_err(|e| {
                OasisError::Backend(oasis_types::error::BackendError::Other(format!(
                    "SDL set_size failed: {e:?}"
                )))
            })?;
        self.viewport_w = width;
        self.viewport_h = height;
        self.clip_stack = ClipStack::new(width, height);
        self.translate_stack = TranslateStack::new();
        Ok(())
    }

    /// Apply cumulative translation to coordinates.
    pub(crate) fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        self.translate_stack.translate(x, y)
    }

    /// Set the SDL draw color with optional blend mode.
    ///
    /// Skips the SDL calls entirely when the color (and therefore the
    /// derived blend mode) matches the last one set — gradients and
    /// batched chrome otherwise re-send identical state per scanline.
    pub(crate) fn set_color(&mut self, color: Color) {
        if self.last_draw_color == Some(color) {
            return;
        }
        if color.a < 255 {
            self.canvas.set_blend_mode(sdl3::render::BlendMode::Blend);
        } else {
            self.canvas.set_blend_mode(sdl3::render::BlendMode::None);
        }
        self.canvas.set_draw_color(sdl3::pixels::Color::RGBA(
            color.r, color.g, color.b, color.a,
        ));
        self.last_draw_color = Some(color);
    }
}

// -------------------------------------------------------------------
// SdiShapes: Shape primitives (delegated to shapes.rs)
// -------------------------------------------------------------------

impl SdiShapes for SdlBackend {
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        self.shape_fill_rounded_rect(x, y, w, h, radius, color)
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
        self.shape_stroke_rect(
            x,
            y,
            w,
            h,
            StrokeStyle {
                width: stroke_width,
                color,
            },
        )
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
        self.shape_draw_line(x1, y1, x2, y2, width, color)
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        self.shape_fill_circle(cx, cy, radius, color)
    }

    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        self.shape_stroke_circle(
            cx,
            cy,
            radius,
            StrokeStyle {
                width: stroke_width,
                color,
            },
        )
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
        self.shape_fill_triangle(x1, y1, x2, y2, x3, y3, color)
    }

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
        self.shape_stroke_rounded_rect(
            x,
            y,
            w,
            h,
            radius,
            StrokeStyle {
                width: stroke_width,
                color,
            },
        )
    }
}

// -------------------------------------------------------------------
// SdiVector: Polygon, arc, dashed line (delegated to shapes.rs)
// -------------------------------------------------------------------

impl SdiVector for SdlBackend {
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        self.shape_fill_polygon(points, color)
    }

    fn fill_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) -> Result<()> {
        self.shape_fill_arc(
            ArcParams {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
            },
            color,
        )
    }

    fn stroke_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        self.shape_stroke_arc(
            ArcParams {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
            },
            StrokeStyle { width, color },
        )
    }

    fn stroke_line_dashed(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
        dash: u16,
        gap: u16,
    ) -> Result<()> {
        self.shape_stroke_line_dashed(
            x1,
            y1,
            x2,
            y2,
            StrokeStyle { width, color },
            DashStyle { dash, gap },
        )
    }
}

// -------------------------------------------------------------------
// SdiTextures: Texture operations (delegated to blitting.rs)
// -------------------------------------------------------------------

impl SdiTextures for SdlBackend {
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
        self.blit_sub_impl(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
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
        self.blit_tinted_impl(tex, x, y, w, h, tint)
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
        self.blit_sub_tinted_impl(
            tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h, tint,
        )
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
        self.blit_flipped_impl(tex, x, y, w, h, flip_h, flip_v)
    }
}

// -------------------------------------------------------------------
// SdiAlpha: Viewport and alpha utilities
// -------------------------------------------------------------------

impl SdiAlpha for SdlBackend {
    fn viewport_size(&self) -> (u32, u32) {
        (self.viewport_w, self.viewport_h)
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(
            0,
            0,
            self.viewport_w,
            self.viewport_h,
            Color::rgba(0, 0, 0, alpha),
        )
    }
}

// -------------------------------------------------------------------
// SdiClipTransform: Clip and transform stack
// -------------------------------------------------------------------

impl SdiClipTransform for SdlBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let new_clip = ClipRect { x: tx, y: ty, w, h };
        match self.clip_stack.push(new_clip) {
            ClipPush::Clip(c) => {
                if c.w == 0 || c.h == 0 {
                    // `Rect::new` would clamp 0 to 1, letting a 1-pixel
                    // sliver through. Use `Zero` to actually reject all
                    // subsequent draws.
                    self.canvas.set_clip_rect(sdl3::render::ClippingRect::Zero);
                } else {
                    self.canvas
                        .set_clip_rect(sdl3::rect::Rect::new(c.x, c.y, c.w, c.h));
                }
            },
            ClipPush::Empty => {
                self.canvas.set_clip_rect(sdl3::render::ClippingRect::Zero);
            },
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        match self.clip_stack.pop() {
            Some(prev) => {
                if prev.w == 0 || prev.h == 0 {
                    self.canvas.set_clip_rect(sdl3::render::ClippingRect::Zero);
                } else {
                    self.canvas
                        .set_clip_rect(sdl3::rect::Rect::new(prev.x, prev.y, prev.w, prev.h));
                }
            },
            None => {
                self.canvas.set_clip_rect(None);
            },
        }
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.clip_stack.current_tuple()
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
// SdiBatch: collapse consecutive same-color rect runs into a single
// `SDL_RenderFillRects` call. The win is one (set_color + draw)
// syscall pair per color *group* instead of per rect; for typical
// frames (file-manager grid, browser borders, skin chrome) most
// neighbouring rects share a color so this is a meaningful drop in
// SDL syscalls. Order is preserved — no sorting — so callers passing
// overlapping rects keep correct paint order.
// -------------------------------------------------------------------

impl SdiBatch for SdlBackend {
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        if rects.is_empty() {
            return Ok(());
        }
        let mut group: Vec<sdl3::render::FRect> = Vec::with_capacity(rects.len());
        let mut group_color: Option<Color> = None;
        for r in rects {
            if r.w == 0 || r.h == 0 || r.color.a == 0 {
                continue;
            }
            if group_color != Some(r.color) {
                if let Some(prev) = group_color.take() {
                    self.set_color(prev);
                    self.canvas.fill_rects(&group).backend_err()?;
                    group.clear();
                }
                group_color = Some(r.color);
            }
            let (tx, ty) = self.translate(r.x, r.y);
            group.push(frect(tx, ty, r.w, r.h));
        }
        if let Some(color) = group_color {
            self.set_color(color);
            self.canvas.fill_rects(&group).backend_err()?;
        }
        Ok(())
    }

    fn submit_text_batch(
        &mut self,
        texts: &[BatchText<'_>],
        font_size: u16,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        // One styled pass per item: bold comes from the glyph cache's
        // baked double-strike entries instead of the default impl's
        // second shifted draw_text (which blits every glyph twice and
        // double-blends antialiased TTF coverage).
        for t in texts {
            self.draw_text_styled(t.text, t.x, t.y, font_size, t.color, bold, italic)?;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiRenderTarget: Offscreen compositing layers (compositor PR4)
// -------------------------------------------------------------------

/// Map a CSS blend mode onto SDL3's built-in blend-mode enum.
///
/// SDL3 only ships a handful of blend modes natively: `NONE`, `BLEND`,
/// `ADD`, `MOD`, `MUL`. Anything CSS-specific (`Overlay`, `ColorDodge`,
/// the non-separable HSL modes, …) falls back to plain alpha blending
/// for the moment — documented as accepted degradation in
/// `docs/compositor-overhaul-plan.md` §3.4 step 4. A CPU compositor
/// extension is queued for a follow-up PR.
fn sdl_blend_for(mode: BlendMode) -> sdl3::render::BlendMode {
    use sdl3::render::BlendMode as Sdl;
    match mode {
        BlendMode::Normal => Sdl::Blend,
        // SDL3 `MOD` is `dst * src`, matching CSS `multiply` closely enough
        // for alpha-1 src. Non-unit alpha will drift but is still closer
        // than plain alpha over.
        BlendMode::Multiply => Sdl::Mod,
        // Everything else: software path is TODO, degrade to standard
        // alpha blending so the page still renders.
        _ => Sdl::Blend,
    }
}

impl SdiRenderTarget for SdlBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        if w == 0 || h == 0 {
            return Err(OasisError::Backend(
                format!("create_render_target: zero dimension ({w}x{h})").into(),
            ));
        }
        // Create an ABGR8888 texture with TARGET access so the
        // renderer can draw into it.
        let raw_renderer = self.canvas.raw();
        // SAFETY: raw_renderer is a valid SDL_Renderer pointer owned
        // by `canvas`. SDL_CreateTexture does not retain the pointer;
        // it returns a fresh SDL_Texture on success or null on failure.
        let raw_tex = unsafe {
            sdl3::sys::render::SDL_CreateTexture(
                raw_renderer,
                sdl3::sys::pixels::SDL_PIXELFORMAT_ABGR8888,
                sdl3::sys::render::SDL_TEXTUREACCESS_TARGET,
                w as i32,
                h as i32,
            )
        };
        if raw_tex.is_null() {
            return Err(OasisError::Backend(
                format!("SDL_CreateTexture (target {w}x{h}) failed").into(),
            ));
        }
        // SAFETY: raw_tex is a valid SDL_Texture; wrap in Rust handle.
        let tex: Texture<'_> = unsafe { self.texture_creator.raw_create_texture(raw_tex) };
        // SAFETY: Erase the lifetime to `'static` using the same pattern
        // as regular textures on this backend. Correctness relies on
        // `render_targets.clear()` in `Drop` ordering before the
        // texture_creator is dropped.
        let tex: Texture<'static> = unsafe { std::mem::transmute(tex) };
        let id = self.next_render_target_id;
        self.next_render_target_id += 1;
        self.render_targets.insert(id, tex);
        Ok(RenderTargetId(id))
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        let raw_tex = match self.render_targets.get(&id.0) {
            Some(tex) => self::sdl_texture_raw(tex),
            None => {
                return Err(OasisError::Backend(
                    format!("bind_render_target: unknown id {id:?}").into(),
                ));
            },
        };
        // Remember the previous binding so unbind can restore it.
        self.render_target_bind_stack
            .push(self.current_render_target);
        // SAFETY: raw_tex is a valid SDL_Texture owned by this backend.
        // SDL_SetRenderTarget accepts any texture created with
        // `TEXTUREACCESS_TARGET` from the same renderer.
        let ok = unsafe { sdl3::sys::render::SDL_SetRenderTarget(self.canvas.raw(), raw_tex) };
        if !ok {
            self.render_target_bind_stack.pop();
            return Err(OasisError::Backend(
                "SDL_SetRenderTarget (bind) failed".into(),
            ));
        }
        self.current_render_target = Some(id.0);
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let prev = self.render_target_bind_stack.pop().ok_or_else(|| {
            OasisError::Backend("unbind_render_target: bind stack underflow".into())
        })?;
        let raw_tex: *mut sdl3::sys::render::SDL_Texture = match prev {
            Some(id) => match self.render_targets.get(&id) {
                Some(tex) => self::sdl_texture_raw(tex),
                None => std::ptr::null_mut(),
            },
            None => std::ptr::null_mut(),
        };
        // SAFETY: raw_tex is either null (framebuffer) or a valid
        // texture owned by this backend that has outlived the current
        // bind scope.
        let ok = unsafe { sdl3::sys::render::SDL_SetRenderTarget(self.canvas.raw(), raw_tex) };
        if !ok {
            return Err(OasisError::Backend(
                "SDL_SetRenderTarget (unbind) failed".into(),
            ));
        }
        self.current_render_target = prev;
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
        let Self {
            canvas,
            render_targets,
            ..
        } = self;
        let tex = render_targets.get_mut(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("composite_render_target: unknown id {id:?}").into())
        })?;
        tex.set_blend_mode(sdl_blend_for(blend));
        let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        tex.set_alpha_mod(alpha);
        tex.set_color_mod(255, 255, 255);
        let r = canvas
            .copy(tex, None, Some(frect(dst_x, dst_y, dst_w, dst_h)))
            .backend_err();
        tex.set_alpha_mod(255);
        tex.set_blend_mode(sdl3::render::BlendMode::Blend);
        r
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        // If the caller destroys a still-bound target, refuse — same
        // contract the RecordingBackend tests enforce.
        if self.current_render_target == Some(id.0)
            || self.render_target_bind_stack.contains(&Some(id.0))
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
        // SDL3 supports SDL_RenderReadPixels but read_render_target
        // is not yet wired — report false until implemented.
        false
    }
}

/// Extract the raw `SDL_Texture` pointer from a safe `Texture`
/// handle.
///
/// The sdl3 crate hides its `raw` field behind `pub(crate)`, but we
/// need the pointer to call `SDL_SetRenderTarget`.  `Texture<'r>` in
/// sdl3 0.17 is `{ raw: *mut SDL_Texture, _marker: PhantomData }` —
/// layout-compatible with a single pointer. We cast through a
/// repr-identical shadow struct.
#[repr(C)]
struct TextureLayout {
    raw: *mut sdl3::sys::render::SDL_Texture,
    _marker: std::marker::PhantomData<&'static ()>,
}

fn sdl_texture_raw(tex: &Texture<'static>) -> *mut sdl3::sys::render::SDL_Texture {
    // SAFETY: This relies on sdl3 0.17 `Texture<'r>` having the exact
    // layout `{ raw: *mut SDL_Texture, _marker: PhantomData }`, which
    // it does (verified in `render.rs:2213-2216` of the sdl3 crate).
    // `TextureLayout` mirrors that layout with `#[repr(C)]` and a
    // single pointer-sized field + ZST.  A bump in the sdl3 crate
    // that changes this layout is the reason we gate sdl3 at exact
    // minor version in Cargo.toml.
    unsafe {
        let layout: &TextureLayout = &*(tex as *const Texture<'static>).cast::<TextureLayout>();
        layout.raw
    }
}

impl oasis_core::backend::ClipboardBackend for SdlBackend {
    fn copy(&mut self, text: &str) {
        let clipboard = self.canvas.window().subsystem().clipboard();
        if let Err(e) = clipboard.set_clipboard_text(text) {
            log::warn!("SDL clipboard copy failed: {e}");
        }
    }

    fn paste(&self) -> Option<String> {
        let clipboard = self.canvas.window().subsystem().clipboard();
        clipboard.clipboard_text().ok().filter(|s| !s.is_empty())
    }

    fn has_content(&self) -> bool {
        self.canvas
            .window()
            .subsystem()
            .clipboard()
            .has_clipboard_text()
    }

    fn clear(&mut self) {
        let clipboard = self.canvas.window().subsystem().clipboard();
        if let Err(e) = clipboard.set_clipboard_text("") {
            log::warn!("SDL clipboard clear failed: {e}");
        }
    }
}

impl Drop for SdlBackend {
    fn drop(&mut self) {
        // SAFETY: Textures hold transmuted `'static` references that actually borrow
        // from `self.texture_creator`. We must drop all textures before the
        // texture_creator is dropped. Without this explicit Drop impl, correctness
        // would depend on struct field declaration order (Rust drops fields in
        // declaration order), which is a fragile invariant. Clearing here makes
        // the safety guarantee explicit and immune to field reordering.
        self.glyph_cache.clear();
        self.textures.clear();
        // Render targets share the same lifetime-erasure pattern as
        // `textures` and must be dropped before `texture_creator`.
        self.render_targets.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sdl3::keyboard::Keycode;

    use oasis_core::input::{Button, InputEvent, Trigger};
    use oasis_types::backend::SdiText;

    // ---------------------------------------------------------------
    // Input mapping tests
    // ---------------------------------------------------------------

    #[test]
    fn key_down_arrow_keys() {
        assert_eq!(
            map_key_down(Keycode::Up),
            Some(InputEvent::ButtonPress(Button::Up))
        );
        assert_eq!(
            map_key_down(Keycode::Down),
            Some(InputEvent::ButtonPress(Button::Down))
        );
        assert_eq!(
            map_key_down(Keycode::Left),
            Some(InputEvent::ButtonPress(Button::Left))
        );
        assert_eq!(
            map_key_down(Keycode::Right),
            Some(InputEvent::ButtonPress(Button::Right))
        );
    }

    #[test]
    fn key_down_action_keys() {
        assert_eq!(
            map_key_down(Keycode::Return),
            Some(InputEvent::ButtonPress(Button::Confirm))
        );
        assert_eq!(
            map_key_down(Keycode::Escape),
            Some(InputEvent::ButtonPress(Button::Cancel))
        );
        assert_eq!(
            map_key_down(Keycode::Space),
            Some(InputEvent::ButtonPress(Button::Triangle))
        );
        // Tab is handled in map_sdl_event (with Shift detection).
        assert_eq!(map_key_down(Keycode::Tab), None);
    }

    #[test]
    fn key_down_function_keys() {
        assert_eq!(
            map_key_down(Keycode::F1),
            Some(InputEvent::ButtonPress(Button::Start))
        );
        assert_eq!(
            map_key_down(Keycode::F2),
            Some(InputEvent::ButtonPress(Button::Select))
        );
    }

    #[test]
    fn key_down_triggers() {
        assert_eq!(
            map_key_down(Keycode::Q),
            Some(InputEvent::TriggerPress(Trigger::Left))
        );
        assert_eq!(
            map_key_down(Keycode::E),
            Some(InputEvent::TriggerPress(Trigger::Right))
        );
    }

    #[test]
    fn key_down_backspace() {
        assert_eq!(
            map_key_down(Keycode::Backspace),
            Some(InputEvent::Backspace)
        );
    }

    #[test]
    fn key_down_f11_toggle_fullscreen() {
        assert_eq!(
            map_key_down(Keycode::F11),
            Some(InputEvent::ToggleFullscreen)
        );
    }

    #[test]
    fn key_down_unmapped_returns_none() {
        assert_eq!(map_key_down(Keycode::A), None);
        assert_eq!(map_key_down(Keycode::Z), None);
        assert_eq!(map_key_down(Keycode::_0), None);
        assert_eq!(map_key_down(Keycode::F5), None);
    }

    #[test]
    fn key_up_arrow_keys() {
        assert_eq!(
            map_key_up(Keycode::Up),
            Some(InputEvent::ButtonRelease(Button::Up))
        );
        assert_eq!(
            map_key_up(Keycode::Down),
            Some(InputEvent::ButtonRelease(Button::Down))
        );
        assert_eq!(
            map_key_up(Keycode::Left),
            Some(InputEvent::ButtonRelease(Button::Left))
        );
        assert_eq!(
            map_key_up(Keycode::Right),
            Some(InputEvent::ButtonRelease(Button::Right))
        );
    }

    #[test]
    fn key_up_action_keys() {
        assert_eq!(
            map_key_up(Keycode::Return),
            Some(InputEvent::ButtonRelease(Button::Confirm))
        );
        assert_eq!(
            map_key_up(Keycode::Escape),
            Some(InputEvent::ButtonRelease(Button::Cancel))
        );
        assert_eq!(
            map_key_up(Keycode::Space),
            Some(InputEvent::ButtonRelease(Button::Triangle))
        );
        // Tab is handled in map_sdl_event (with Shift detection).
        assert_eq!(map_key_up(Keycode::Tab), None);
    }

    #[test]
    fn key_up_triggers() {
        assert_eq!(
            map_key_up(Keycode::Q),
            Some(InputEvent::TriggerRelease(Trigger::Left))
        );
        assert_eq!(
            map_key_up(Keycode::E),
            Some(InputEvent::TriggerRelease(Trigger::Right))
        );
    }

    #[test]
    fn key_up_unmapped_returns_none() {
        assert_eq!(map_key_up(Keycode::A), None);
        assert_eq!(map_key_up(Keycode::Backspace), None);
    }

    #[test]
    fn key_down_up_symmetry() {
        // Every mapped key_down should have a corresponding key_up
        // (except Backspace and F11).
        // Tab is handled in map_sdl_event (with Shift detection).
        let keys = [
            Keycode::Up,
            Keycode::Down,
            Keycode::Left,
            Keycode::Right,
            Keycode::Return,
            Keycode::Escape,
            Keycode::Space,
            Keycode::F1,
            Keycode::F2,
            Keycode::Q,
            Keycode::E,
        ];
        for key in keys {
            let down = map_key_down(key);
            let up = map_key_up(key);
            assert!(down.is_some(), "key_down({key:?}) should be mapped");
            assert!(up.is_some(), "key_up({key:?}) should be mapped");
            // Verify press/release correspondence.
            match (down.unwrap(), up.unwrap()) {
                (InputEvent::ButtonPress(b1), InputEvent::ButtonRelease(b2)) => {
                    assert_eq!(b1, b2, "press/release mismatch {key:?}");
                },
                (InputEvent::TriggerPress(t1), InputEvent::TriggerRelease(t2)) => {
                    assert_eq!(t1, t2, "press/release mismatch {key:?}");
                },
                (d, u) => panic!("unexpected pair {key:?}: {d:?}/{u:?}"),
            }
        }
    }

    #[test]
    fn key_up_backspace_not_mapped() {
        // Backspace has no key_up mapping (fire-and-forget).
        assert_eq!(map_key_up(Keycode::Backspace), None);
    }

    #[test]
    fn key_up_f11_not_mapped() {
        // F11 (ToggleFullscreen) has no key_up mapping.
        assert_eq!(map_key_up(Keycode::F11), None);
    }

    #[test]
    fn key_down_f1_f2_mapped_but_other_fn_keys_not() {
        assert!(map_key_down(Keycode::F1).is_some());
        assert!(map_key_down(Keycode::F2).is_some());
        assert!(map_key_down(Keycode::F3).is_none());
        assert!(map_key_down(Keycode::F4).is_none());
        assert!(map_key_down(Keycode::F5).is_none());
        assert!(map_key_down(Keycode::F6).is_none());
        assert!(map_key_down(Keycode::F7).is_none());
        assert!(map_key_down(Keycode::F8).is_none());
        assert!(map_key_down(Keycode::F9).is_none());
        assert!(map_key_down(Keycode::F10).is_none());
        assert!(map_key_down(Keycode::F12).is_none());
    }

    #[test]
    fn key_down_letter_keys_only_q_e_mapped() {
        // Q and E are trigger keys; no other letters are mapped.
        assert!(map_key_down(Keycode::Q).is_some());
        assert!(map_key_down(Keycode::E).is_some());
        assert!(map_key_down(Keycode::A).is_none());
        assert!(map_key_down(Keycode::W).is_none());
        assert!(map_key_down(Keycode::S).is_none());
        assert!(map_key_down(Keycode::D).is_none());
    }

    #[test]
    fn mouse_wheel_delta_sign_inversion() {
        // SDL3 wheel y > 0 = scroll up, but OASIS expects positive
        // delta = scroll down. The mapping negates the value.
        // Simulating: Event::MouseWheel { y: 1.0 } -> delta: -1
        // and Event::MouseWheel { y: -1.0 } -> delta: 1
        //
        // We can't construct SDL events directly but we can verify
        // the formula: delta = -(y as i32)
        let sdl_y_up: f32 = 3.0;
        let sdl_y_down: f32 = -2.0;
        assert_eq!(-(sdl_y_up as i32), -3);
        assert_eq!(-(sdl_y_down as i32), 2);
    }

    #[test]
    fn mouse_coordinate_truncation() {
        // SDL3 mouse coords are f32; truncation to i32 floors.
        let fx: f32 = 42.7;
        let fy: f32 = 99.9;
        assert_eq!(fx as i32, 42);
        assert_eq!(fy as i32, 99);

        // Negative coords truncate toward zero.
        let neg_x: f32 = -1.5;
        assert_eq!(neg_x as i32, -1);
    }

    #[test]
    fn frect_conversion() {
        let r = frect(10, 20, 100, 50);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.w, 100.0);
        assert_eq!(r.h, 50.0);
    }

    #[test]
    fn frect_zero_size() {
        let r = frect(0, 0, 0, 0);
        assert_eq!(r.x, 0.0);
        assert_eq!(r.w, 0.0);
    }

    #[test]
    fn fpoint_conversion() {
        let p = fpoint(42, -7);
        assert_eq!(p.x, 42.0);
        assert_eq!(p.y, -7.0);
    }

    // ---------------------------------------------------------------
    // Helper function tests
    // ---------------------------------------------------------------

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt_i32(0), 0);
        assert_eq!(isqrt_i32(1), 1);
        assert_eq!(isqrt_i32(4), 2);
        assert_eq!(isqrt_i32(9), 3);
        assert_eq!(isqrt_i32(16), 4);
        assert_eq!(isqrt_i32(25), 5);
        assert_eq!(isqrt_i32(100), 10);
    }

    #[test]
    fn isqrt_known_non_perfect_squares() {
        assert_eq!(isqrt_i32(2), 1);
        assert_eq!(isqrt_i32(3), 1);
        assert_eq!(isqrt_i32(5), 2);
        assert_eq!(isqrt_i32(8), 2);
        assert_eq!(isqrt_i32(10), 3);
        assert_eq!(isqrt_i32(99), 9);
    }

    #[test]
    fn isqrt_negative() {
        assert_eq!(isqrt_i32(-1), 0);
        assert_eq!(isqrt_i32(-100), 0);
    }

    #[test]
    fn edge_x_horizontal_line() {
        // Horizontal line: y0 == y1, should return x0.
        assert_eq!(edge_x(10, 5, 20, 5, 5), 10);
    }

    #[test]
    fn edge_x_vertical_line() {
        // Vertical line from (10,0) to (10,100).
        assert_eq!(edge_x(10, 0, 10, 100, 50), 10);
    }

    #[test]
    fn edge_x_diagonal() {
        // Line from (0,0) to (100,100): at y=50, x should be 50.
        assert_eq!(edge_x(0, 0, 100, 100, 50), 50);
    }

    #[test]
    fn edge_x_endpoints() {
        // At y=y0, should return x0.
        assert_eq!(edge_x(10, 20, 50, 80, 20), 10);
        // At y=y1, should return x1.
        assert_eq!(edge_x(10, 20, 50, 80, 80), 50);
    }

    // ---------------------------------------------------------------
    // SDL rendering correctness tests (require display)
    //
    // These tests require a working SDL3 display. In CI, set
    // SDL_VIDEO_DRIVER=dummy (or x11/wayland) and run:
    //   cargo test -p oasis-backend-sdl -- --ignored
    //
    // Locally with a display, they can be run directly.
    // ---------------------------------------------------------------

    fn try_create_backend() -> Option<SdlBackend> {
        SdlBackend::new("test", 64, 64).ok()
    }

    #[test]
    #[ignore]
    fn render_clear_red() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        let red = Color::rgb(255, 0, 0);
        backend.clear(red).unwrap();
        backend.swap_buffers().unwrap();

        let pixels = backend.read_pixels(0, 0, 64, 64).unwrap();
        // read_pixels returns ABGR8888 format, 4 bytes per pixel.
        // Check a sample pixel at (0,0).
        assert_eq!(pixels.len(), 64 * 64 * 4);
        // First pixel should be red (exact format depends on SDL).
        // At minimum, the red channel should be 255 and blue should be 0.
        let r = pixels[0];
        let g = pixels[1];
        let b = pixels[2];
        assert!(
            r > 200 || b > 200,
            "red channel should be dominant: r={r} g={g} b={b}"
        );
    }

    #[test]
    #[ignore]
    fn render_fill_rect_at_position() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        // Clear to black, then fill a small rect with white.
        backend.clear(Color::BLACK).unwrap();
        backend.fill_rect(10, 10, 4, 4, Color::WHITE).unwrap();
        backend.swap_buffers().unwrap();

        // Read the filled area.
        let pixels = backend.read_pixels(10, 10, 4, 4).unwrap();
        assert_eq!(pixels.len(), 4 * 4 * 4);
        // All pixels in the rect should be non-zero (white).
        let all_nonzero = pixels.chunks(4).all(|px| px[0] > 200 || px[2] > 200);
        assert!(all_nonzero, "filled rect should have white pixels");

        // Read an area outside the rect (should be black).
        let outside = backend.read_pixels(0, 0, 4, 4).unwrap();
        let all_dark = outside
            .chunks(4)
            .all(|px| px[0] < 50 && px[1] < 50 && px[2] < 50);
        assert!(all_dark, "area outside rect should be black");
    }

    #[test]
    #[ignore]
    fn render_texture_load_and_blit() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        // Create a 2x2 red texture.
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let tex = backend.load_texture(2, 2, &rgba).unwrap();
        backend.clear(Color::BLACK).unwrap();
        backend.blit(tex, 0, 0, 2, 2).unwrap();
        backend.swap_buffers().unwrap();

        let pixels = backend.read_pixels(0, 0, 2, 2).unwrap();
        // Should have non-black pixels where the texture was blitted.
        let has_color = pixels.iter().any(|&b| b > 50);
        assert!(has_color, "blitted texture should produce colored pixels");

        backend.destroy_texture(tex).unwrap();
    }

    #[test]
    #[ignore]
    fn render_draw_text_produces_pixels() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::BLACK).unwrap();
        backend.draw_text("A", 0, 0, 8, Color::WHITE).unwrap();
        backend.swap_buffers().unwrap();

        let pixels = backend.read_pixels(0, 0, 8, 8).unwrap();
        // The letter 'A' should have some white pixels.
        let white_count = pixels
            .chunks(4)
            .filter(|px| px[0] > 200 || px[2] > 200)
            .count();
        assert!(
            white_count > 5,
            "letter 'A' should have visible pixels, got {white_count}"
        );
    }

    #[test]
    #[ignore]
    fn render_measure_text_width() {
        let backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        // At font_size=8, each char is 8px wide.
        assert_eq!(backend.measure_text("A", 8), 8);
        assert_eq!(backend.measure_text("AB", 8), 16);
        assert_eq!(backend.measure_text("Hello", 8), 40);
        assert_eq!(backend.measure_text("", 8), 0);

        // At font_size=16, each char is 16px wide (scale=2).
        assert_eq!(backend.measure_text("A", 16), 16);
    }

    #[test]
    #[ignore]
    fn render_clip_rect_restricts_drawing() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::BLACK).unwrap();
        // Set clip to top-left 16x16.
        backend.set_clip_rect(0, 0, 16, 16).unwrap();
        // Fill entire 64x64 with white -- only 16x16 should be affected.
        backend.fill_rect(0, 0, 64, 64, Color::WHITE).unwrap();
        backend.reset_clip_rect().unwrap();
        backend.swap_buffers().unwrap();

        // Inside clip: should be white.
        let inside = backend.read_pixels(0, 0, 16, 16).unwrap();
        let inside_white = inside
            .chunks(4)
            .filter(|px| px[0] > 200 || px[2] > 200)
            .count();
        assert!(inside_white > 200, "inside clip should be mostly white");

        // Outside clip: should be black.
        let outside = backend.read_pixels(32, 32, 16, 16).unwrap();
        let outside_dark = outside
            .chunks(4)
            .all(|px| px[0] < 50 && px[1] < 50 && px[2] < 50);
        assert!(outside_dark, "outside clip should remain black");
    }

    // ---------------------------------------------------------------
    // Glyph cache tests
    // ---------------------------------------------------------------

    #[test]
    fn glyph_cache_key_roundtrip() {
        // Verify that GlyphCacheKey packing produces distinct keys
        // for different parameters and equal keys for the same
        // parameters.
        let c = Color::rgba(255, 128, 64, 200);
        let k1 = GlyphCacheKey::new('A', 16, c, false, false);
        let k2 = GlyphCacheKey::new('A', 16, c, false, false);
        assert_eq!(k1, k2);
        assert_eq!(k1.raw(), k2.raw());

        // Different character produces a different key.
        let k3 = GlyphCacheKey::new('B', 16, c, false, false);
        assert_ne!(k1, k3);

        // Different font size produces a different key.
        let k4 = GlyphCacheKey::new('A', 24, c, false, false);
        assert_ne!(k1, k4);

        // Bold flag produces a different key.
        let k5 = GlyphCacheKey::new('A', 16, c, true, false);
        assert_ne!(k1, k5);

        // Italic flag produces a different key.
        let k6 = GlyphCacheKey::new('A', 16, c, false, true);
        assert_ne!(k1, k6);
    }

    #[test]
    fn glyph_cache_fields_initialized() {
        // Verify that glyph cache fields exist and are properly
        // initialized (empty maps and zero counter). This test
        // can only run when an SDL display is available.
        let backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        assert!(backend.glyph_cache.is_empty());
        assert_eq!(backend.glyph_access_counter, 0);
    }

    #[test]
    #[ignore]
    fn glyph_cache_populates_on_draw() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::BLACK).unwrap();
        assert!(backend.glyph_cache.is_empty());
        backend
            .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        // Two distinct characters should create two cache entries.
        assert_eq!(backend.glyph_cache.len(), 2);

        // Drawing the same text again should not increase cache
        // size (cache hits).
        backend
            .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        assert_eq!(backend.glyph_cache.len(), 2);
    }

    #[test]
    #[ignore]
    fn glyph_cache_is_color_independent() {
        // Glyphs are rasterized white and tinted at blit time, so drawing the
        // same character in a dozen colors (themes, hovers, fades) must reuse
        // one cache entry — the whole point of D2.
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::BLACK).unwrap();
        for i in 0..12u8 {
            let color = Color::rgb(i * 20, 255 - i * 20, 128);
            backend
                .draw_text_styled("A", 0, 0, 16, color, false, false)
                .unwrap();
        }
        assert_eq!(backend.glyph_cache.len(), 1);

        // Style variants still get their own entries.
        backend
            .draw_text_styled("A", 0, 0, 16, Color::WHITE, true, false)
            .unwrap();
        backend
            .draw_text_styled("A", 0, 0, 24, Color::WHITE, false, false)
            .unwrap();
        assert_eq!(backend.glyph_cache.len(), 3);
    }

    #[test]
    #[ignore]
    fn glyph_cache_entry_carries_dimensions() {
        // The draw path reads dimensions from the entry instead of calling
        // Texture::query() per glyph per frame (D6).
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend
            .draw_text_styled("A", 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        let entry = *backend
            .glyph_cache
            .values()
            .next()
            .expect("one glyph cached");
        assert_eq!(entry.h, 16);
        assert!(entry.w > 0);
        let texture = backend.textures.get(&entry.texture).expect("glyph texture");
        let query = texture.query();
        assert_eq!((entry.w, entry.h), (query.width, query.height));
    }

    /// Regression: a degenerate (zero-width or zero-height) clip rect
    /// must reject every subsequent draw. Before the fix, the SDL3 Rust
    /// wrapper's `Rect::new` silently clamped a 0-dimension to 1, so a
    /// `set_clip_rect(.., 0)` produced a 1-pixel slot that let the top
    /// row of glyphs/fills through — the source of the dotted-underline
    /// leak below the OASIS browser window on old.reddit.com. The
    /// browser intersects nested clip rects during display-list replay;
    /// when a child rect's bottom is at or above the parent's top the
    /// intersection collapses to height 0, and the OASIS clip needs to
    /// reject every subsequent draw rather than admit a 1-pixel sliver.
    #[test]
    #[ignore]
    fn degenerate_clip_rejects_all_draws() {
        use oasis_types::backend::SdiCore;
        let backend_opt = SdlBackend::new("clip-degen", 256, 256).ok();
        let mut backend = match backend_opt {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        // Zero-height clip — must clip everything.
        backend.set_clip_rect(0, 100, 256, 0).unwrap();
        backend
            .fill_rect(0, 100, 256, 5, Color::rgb(0, 255, 0))
            .unwrap();
        let pixels = backend.read_pixels(50, 100, 1, 1).unwrap();
        assert_eq!(
            pixels[0], 255,
            "zero-height clip should reject all fills, got {pixels:?}",
        );
    }

    // ---------------------------------------------------------------
    // Skin TTF font tests
    // ---------------------------------------------------------------

    /// The oasis-browser web-font test fixture: a tiny but valid TTF.
    const TEST_TTF: &[u8] = include_bytes!("../../oasis-browser/test_data/minimal.ttf");

    #[test]
    fn set_font_installs_and_clears_ttf_state() {
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        assert!(backend.ttf_font.is_none());

        // Garbage bytes are rejected; the bitmap font stays active.
        backend.set_font(Some(b"not a font"));
        assert!(backend.ttf_font.is_none());

        backend.set_font(Some(TEST_TTF));
        assert!(backend.ttf_font.is_some());

        backend.set_font(None);
        assert!(backend.ttf_font.is_none());
    }

    #[test]
    #[ignore]
    fn set_font_evicts_glyph_cache_textures() {
        use oasis_types::backend::SdiCore;
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        assert_eq!(backend.glyph_cache.len(), 2);
        let glyph_textures: Vec<u64> = backend.glyph_cache.values().map(|e| e.texture).collect();

        backend.set_font(Some(TEST_TTF));
        assert!(backend.glyph_cache.is_empty());
        for id in glyph_textures {
            assert!(
                !backend.textures.contains_key(&id),
                "stale glyph texture {id} survived the font swap"
            );
        }
    }

    #[test]
    fn ttf_measure_matches_per_char_advances() {
        use oasis_types::backend::SdiCore;
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        // Bitmap path is untouched without a font.
        assert_eq!(
            backend.measure_text("Hello", 16),
            oasis_core::backend::bitmap_measure_text("Hello", 16)
        );

        backend.set_font(Some(TEST_TTF));
        let ttf = backend.ttf_font.as_ref().expect("font installed");
        // Whatever glyphs the fixture has, measurement must equal the same
        // per-character integer advances the draw loop accumulates — with
        // bitmap fallback for characters the font lacks.
        let text = "Hello ▲▼ world";
        let expected: u32 = text
            .chars()
            .map(|ch| {
                if ttf.has_glyph(ch) {
                    ttf.advance(ch, 16.0).max(0) as u32
                } else {
                    oasis_types::bitmap_font::glyph_advance_scaled(ch, 16) as u32
                }
            })
            .sum();
        assert_eq!(backend.measure_text(text, 16), expected);
    }

    #[test]
    #[ignore]
    fn ttf_draw_advances_agree_with_measure() {
        use oasis_types::backend::SdiCore;
        let mut backend = match try_create_backend() {
            Some(b) => b,
            None => return,
        };
        backend.set_font(Some(TEST_TTF));
        backend.clear(Color::BLACK).unwrap();
        let text = "AV.";
        backend
            .draw_text_styled(text, 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        // Sum the advances of the entries the draw created, in draw order.
        let drawn: i32 = text
            .chars()
            .map(|ch| backend.glyph_cache[&GlyphCacheKey::colorless(ch, 16, false, false)].advance)
            .sum();
        assert_eq!(drawn.max(0) as u32, backend.measure_text(text, 16));
    }
}
