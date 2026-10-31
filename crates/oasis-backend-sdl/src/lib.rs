//! SDL3 backend for OASIS_OS.
//!
//! Implements `SdiBackend` and `InputBackend` using SDL3. Used for desktop
//! development and Raspberry Pi deployment (via SDL3's kmsdrm or X11 backend).
//!
//! Extended primitives (rounded rects, lines, circles, triangles, gradients,
//! sub-rect blits, tinted blits, clip/transform stacks) are implemented using
//! SDL3 renderer API calls and software rasterization helpers. One module
//! per trait group: `core_impl` (`SdiCore`), `shapes`, `gradients`,
//! `glyph_cache` (`SdiText`), `blitting` (textures), `render_target`; the
//! clip, vector and batch impls live here.

mod blitting;
mod core_impl;
mod glyph_cache;
mod gradients;
mod input;
pub mod mp3;
pub mod network;
mod render_target;
mod sdl_audio;
pub mod shader_bridge;
mod shapes;

use std::collections::HashMap;

use sdl3::EventPump;
use sdl3::render::{Canvas, FRect, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

use oasis_types::backend::stacks::{ClipStack, DrawState, TranslateStack};
use oasis_types::backend::{
    BackendErrExt, BatchRect, Color, SdiBatch, SdiClipTransform, SdiShapes, SdiTextures, SdiVector,
    StrokeStyle, TextureId,
};
use oasis_types::error::{OasisError, Result};
pub use oasis_types::geometry::ClipRect;

pub use network::SdlNetworkBackend;
pub use sdl_audio::SdlAudioBackend;

use crate::glyph_cache::GlyphEntry;
use crate::render_target::BoundFrom;
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
    /// Bind stack for `SdiRenderTarget`: one entry per active bind,
    /// innermost last, holding what the unbind restores.
    pub(crate) render_target_bind_stack: Vec<BoundFrom>,
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
    /// Reusable scratch buffer for batched `fill_rects` submissions: the
    /// scanline spans of filled and stroked shapes (rounded rects,
    /// circles, rings, thick lines, triangles, polygons, arcs) go to SDL
    /// in one call instead of one per row.
    pub(crate) rect_batch: Vec<FRect>,
    /// Reusable scratch buffer for polygon fills: translated vertices.
    /// Same reuse-instead-of-allocate pattern as `rect_batch` --
    /// `fill_polygon` previously allocated a fresh `Vec` per call.
    pub(crate) poly_points: Vec<(i32, i32)>,
    /// Reusable scratch buffer for polygon fills: per-scanline edge
    /// intersection x coordinates.
    pub(crate) poly_xs: Vec<i32>,
    /// Tracked color/alpha modulation per texture in `textures`, keyed by
    /// texture id. Lets blit paths skip redundant `set_color_mod` /
    /// `set_alpha_mod` calls (tinted blits previously set + reset four
    /// mods on every call). All mod changes to `textures` entries must go
    /// through `ensure_texture_mod` to stay coherent.
    pub(crate) texture_mods: HashMap<u64, (u8, u8, u8, u8)>,
    /// Input state carried across SDL events (wheel remainder).
    pub(crate) input_state: input::InputState,
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
        Ok(Self::from_canvas(canvas, event_pump, width, height))
    }

    /// Create a backend that renders without a visible window, for tests
    /// and offscreen rendering.
    ///
    /// With `render_driver: None` the process-wide SDL hints are forced
    /// (in-process, overriding any environment variables) to the
    /// `offscreen` video driver and the `software` renderer, so this works
    /// on a CI box with no display. `Some(driver)` keeps the platform
    /// video driver and asks for that renderer (e.g. `"direct3d11"`,
    /// `"opengl"`, `"vulkan"`) on a hidden window, to exercise a hardware
    /// path such as padded texture pitches locally.
    ///
    /// SDL may only be initialized from one thread at a time: callers
    /// creating several backends from test threads must serialize them
    /// (and drop each backend before creating the next).
    pub fn new_headless(width: u32, height: u32, render_driver: Option<&str>) -> Result<Self> {
        use sdl3::hint::{self, Hint};
        let (video_driver, renderer) = match render_driver {
            None => (Some("offscreen"), "software"),
            Some(driver) => (None, driver),
        };
        if let Some(v) = video_driver {
            hint::set_with_priority("SDL_VIDEO_DRIVER", v, &Hint::Override);
        }
        hint::set_with_priority("SDL_RENDER_DRIVER", renderer, &Hint::Override);
        let sdl = sdl3::init().backend_err()?;
        let video = sdl.video().backend_err()?;
        let window = video
            .window("oasis-headless", width, height)
            .hidden()
            .build()
            .backend_err()?;
        let canvas: Canvas<Window> = window.into_canvas();
        let event_pump = sdl.event_pump().backend_err()?;
        log::info!(
            "SDL3 headless backend initialized: {width}x{height} ({} renderer)",
            canvas.renderer_name
        );
        Ok(Self::from_canvas(canvas, event_pump, width, height))
    }

    /// Name of the active SDL renderer (e.g. `"software"`,
    /// `"direct3d11"`).
    pub fn renderer_name(&self) -> &str {
        &self.canvas.renderer_name
    }

    fn from_canvas(canvas: Canvas<Window>, event_pump: EventPump, width: u32, height: u32) -> Self {
        let texture_creator = canvas.texture_creator();
        Self {
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
            clip_stack: ClipStack::default(),
            translate_stack: TranslateStack::new(),
            viewport_w: width,
            viewport_h: height,
            last_draw_color: None,
            rect_batch: Vec::new(),
            poly_points: Vec::new(),
            poly_xs: Vec::new(),
            texture_mods: HashMap::new(),
            input_state: input::InputState::default(),
        }
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
        if let Some(window) = self.render_target_bind_stack.first_mut() {
            // A render target is bound: it keeps its own size and stacks.
            // The window's new size, and its reset stacks and clip, take
            // effect when the bind chain unwinds back to it.
            window.viewport = (width, height);
            window.state = DrawState::default();
            window.drop_parent_clip = true;
        } else {
            self.viewport_w = width;
            self.viewport_h = height;
            self.clip_stack = ClipStack::default();
            self.translate_stack = TranslateStack::new();
            // The SDL-side clip goes with the stacks; leaving it applied
            // would keep clipping while `current_clip_rect` says `None`.
            self.apply_clip(None);
        }
        Ok(())
    }

    /// Apply cumulative translation to coordinates.
    pub(crate) fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        self.translate_stack.translate(x, y)
    }

    /// Body of `submit_rect_batch`: one `fill_rects` per run of
    /// same-colored rects, collected in `group`.
    fn submit_rect_groups(&mut self, rects: &[BatchRect], group: &mut Vec<FRect>) -> Result<()> {
        let mut group_color: Option<Color> = None;
        for r in rects {
            if r.w == 0 || r.h == 0 || r.color.a == 0 {
                continue;
            }
            if group_color != Some(r.color) {
                if let Some(prev) = group_color.take() {
                    self.set_color(prev);
                    self.canvas.fill_rects(group).backend_err()?;
                    group.clear();
                }
                group_color = Some(r.color);
            }
            let (tx, ty) = self.translate(r.x, r.y);
            group.push(frect(tx, ty, r.w, r.h));
        }
        if let Some(color) = group_color {
            self.set_color(color);
            self.canvas.fill_rects(group).backend_err()?;
        }
        Ok(())
    }

    /// Apply `clip` (surface pixels) to the SDL renderer, `None` = no clip.
    pub(crate) fn apply_clip(&mut self, clip: Option<ClipRect>) {
        match clip {
            // A degenerate clip (w == 0 or h == 0) means "intersection
            // collapsed to nothing". `sdl3::rect::Rect::new` would silently
            // clamp the zero to one, leaving a 1-pixel slot that any later
            // `fill_rect` / glyph blit can sneak a sliver of pixels through
            // (the dotted-underline leak below an old.reddit.com browser
            // window). `ClippingRect::Zero` makes SDL3 reject every draw.
            Some(c) if c.w == 0 || c.h == 0 => {
                self.canvas.set_clip_rect(sdl3::render::ClippingRect::Zero);
            },
            Some(c) => self
                .canvas
                .set_clip_rect(sdl3::rect::Rect::new(c.x, c.y, c.w, c.h)),
            None => self.canvas.set_clip_rect(None),
        }
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

    // `stroke_arc` and `stroke_line_dashed` use the trait defaults, which
    // decompose into `draw_line` -- the same pixels the software
    // rasterizer produces (the old overrides thickened with parallel
    // lines along a truncated integer normal, i.e. not at all on
    // diagonals).
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
        let src = (src_x, src_y, src_w, src_h);
        self.copy_texture(tex, Some(src), (dst_x, dst_y, dst_w, dst_h), Color::WHITE)
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
        self.copy_texture(tex, None, (x, y, w, h), tint)
    }
}

// -------------------------------------------------------------------
// SdiClipTransform: Clip and transform stack
// -------------------------------------------------------------------

impl SdiClipTransform for SdlBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let applied = self
            .clip_stack
            .push_translated(&self.translate_stack, x, y, w, h);
        self.apply_clip(Some(applied));
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        let prev = self.clip_stack.pop();
        self.apply_clip(prev);
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<ClipRect> {
        self.clip_stack.current()
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
        // Reuse the shape scratch buffer instead of allocating per batch
        // (nothing below draws a shape, so it is free for the duration).
        let mut group = std::mem::take(&mut self.rect_batch);
        group.clear();
        let result = self.submit_rect_groups(rects, &mut group);
        self.rect_batch = group;
        result
    }

    // `submit_text_batch`: the default, one `draw_text_styled` per item,
    // whose bold comes from the glyph cache's baked double-strike entries.
}

impl oasis_types::backend::ClipboardBackend for SdlBackend {
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

    fn clear_clipboard(&mut self) {
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
mod tests;
