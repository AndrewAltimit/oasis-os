//! SDL3 backend for OASIS_OS.
//!
//! Implements `SdiBackend` and `InputBackend` using SDL3. Used for desktop
//! development and Raspberry Pi deployment (via SDL3's kmsdrm or X11 backend).
//!
//! Extended primitives (rounded rects, lines, circles, triangles, gradients,
//! sub-rect blits, tinted blits, clip/transform stacks) are implemented using
//! SDL3 renderer API calls and software rasterization helpers.

mod blitting;
mod font;
mod input;
pub mod network;
mod sdl_audio;
pub mod shader_bridge;
mod shapes;

use std::collections::HashMap;

use sdl3::EventPump;
use sdl3::pixels::PixelFormat;
use sdl3::rect::Rect;
use sdl3::render::{Canvas, FPoint, FRect, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

use oasis_core::backend::{
    BackendErrExt, Color, GradientStyle, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore,
    SdiGradients, SdiShapes, SdiText, SdiTextures, SdiVector, TextureId, texture_not_found,
    validate_rgba_data,
};
use oasis_core::error::Result;
use oasis_types::backend::stacks::{ClipPush, ClipStack, TranslateStack};
use oasis_types::color::lerp_color_ratio;
pub use oasis_types::geometry::ClipRect;

pub use network::SdlNetworkBackend;
pub use sdl_audio::SdlAudioBackend;

use oasis_rasterize::GlyphCacheKey;
use oasis_types::geometry::rounded_rect_inset;

/// Maximum number of cached glyph textures before LRU eviction kicks in.
const MAX_GLYPH_CACHE_SIZE: usize = 2048;

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
    /// Maps glyph key to a cached SDL texture ID (lives in `textures`).
    glyph_cache: HashMap<GlyphCacheKey, u64>,
    /// LRU access timestamps for glyph cache eviction.
    glyph_access: HashMap<GlyphCacheKey, u64>,
    /// Monotonic counter for LRU access tracking.
    glyph_access_counter: u64,
    // SAFETY: Must be declared after `textures` -- see comment above.
    texture_creator: TextureCreator<WindowContext>,
    next_texture_id: u64,
    pub(crate) clip_stack: ClipStack,
    pub(crate) translate_stack: TranslateStack,
    pub(crate) viewport_w: u32,
    pub(crate) viewport_h: u32,
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
            glyph_cache: HashMap::new(),
            glyph_access: HashMap::new(),
            glyph_access_counter: 0,
            texture_creator,
            next_texture_id: 1,
            clip_stack: ClipStack::new(width, height),
            translate_stack: TranslateStack::new(),
            viewport_w: width,
            viewport_h: height,
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

    /// Apply cumulative translation to coordinates.
    pub(crate) fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        self.translate_stack.translate(x, y)
    }

    /// Render a single glyph to an RGBA buffer and load it as an SDL
    /// texture. Returns the texture ID stored in `self.textures`.
    fn render_glyph_texture(
        &mut self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<u64> {
        let fs = font_size.max(1) as i32;
        let advance = oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
        let bold_extra = if bold { 1 } else { 0 };
        let italic_extra = if italic { fs / 32 + 1 } else { 0 };
        let gw = (advance + bold_extra + italic_extra).max(1) as u32;
        let gh = font_size.max(1) as u32;
        let mut rgba = vec![0u8; (gw * gh * 4) as usize];

        let glyph_data = font::glyph(ch);
        let (left_pad, _) = font::glyph_metrics(ch);
        let left_pad = left_pad as i32;

        for row in 0..8i32 {
            let bits = glyph_data[row as usize];
            if bits == 0 {
                continue;
            }
            let oy0 = row * fs / 8;
            let oy1 = (row + 1) * fs / 8;
            let italic_off = if italic { (7 - row) * fs / 32 } else { 0 };

            for col in 0..8i32 {
                if bits & (0x80 >> col) == 0 {
                    continue;
                }
                let src_col = col - left_pad;
                let ox0 = src_col * fs / 8;
                let ox1 = (src_col + 1) * fs / 8;
                // Fill the scaled rectangle in the buffer.
                for py in oy0..oy1.max(oy0 + 1) {
                    for px in ox0..ox1.max(ox0 + 1) {
                        let bx = px + italic_off;
                        Self::set_glyph_pixel(&mut rgba, gw, gh, bx, py, color);
                        if bold {
                            Self::set_glyph_pixel(&mut rgba, gw, gh, bx + 1, py, color);
                        }
                    }
                }
            }
        }

        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormat::ABGR8888, gw, gh)
            .backend_err()?;
        texture
            .with_lock(None, |buf: &mut [u8], pitch: usize| {
                let row_bytes = (gw as usize) * 4;
                for y in 0..gh as usize {
                    let src_start = y * row_bytes;
                    let dst_start = y * pitch;
                    buf[dst_start..dst_start + row_bytes]
                        .copy_from_slice(&rgba[src_start..src_start + row_bytes]);
                }
            })
            .backend_err()?;
        texture.set_blend_mode(sdl3::render::BlendMode::Blend);

        // SAFETY: The texture borrows from self.texture_creator
        // which lives in the same struct. The explicit `Drop` impl
        // clears all textures before texture_creator is dropped.
        let texture: Texture<'static> = unsafe { std::mem::transmute(texture) };

        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, texture);
        Ok(id)
    }

    /// Write a single pixel into a glyph RGBA buffer, with bounds
    /// checking.
    fn set_glyph_pixel(buf: &mut [u8], w: u32, h: u32, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return;
        }
        let offset = (y as usize * w as usize + x as usize) * 4;
        if offset + 3 < buf.len() {
            buf[offset] = color.r;
            buf[offset + 1] = color.g;
            buf[offset + 2] = color.b;
            buf[offset + 3] = color.a;
        }
    }

    /// Set the SDL draw color with optional blend mode.
    pub(crate) fn set_color(&mut self, color: Color) {
        if color.a < 255 {
            self.canvas.set_blend_mode(sdl3::render::BlendMode::Blend);
        } else {
            self.canvas.set_blend_mode(sdl3::render::BlendMode::None);
        }
        self.canvas.set_draw_color(sdl3::pixels::Color::RGBA(
            color.r, color.g, color.b, color.a,
        ));
    }
}

impl SdiCore for SdlBackend {
    fn init(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.canvas.set_draw_color(sdl3::pixels::Color::RGBA(
            color.r, color.g, color.b, color.a,
        ));
        self.canvas.clear();
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        self.canvas
            .copy(texture, None, frect(tx, ty, w, h))
            .backend_err()?;
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
        self.draw_text_styled(text, x, y, font_size, color, false, false)
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.set_color(color);
        self.canvas.fill_rect(frect(tx, ty, w, h)).backend_err()?;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.canvas.present();
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;

        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormat::ABGR8888, width, height)
            .backend_err()?;

        texture
            .with_lock(None, |buffer: &mut [u8], _pitch: usize| {
                buffer[..rgba_data.len()].copy_from_slice(rgba_data);
            })
            .backend_err()?;

        texture.set_blend_mode(sdl3::render::BlendMode::Blend);

        // SAFETY: The texture borrows from self.texture_creator which lives in the
        // same struct. The explicit `Drop` impl clears all textures before
        // texture_creator is dropped. The erased lifetime is therefore always valid.
        let texture: Texture<'static> = unsafe { std::mem::transmute(texture) };

        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, texture);
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.textures.remove(&tex.0);
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.canvas.set_clip_rect(Rect::new(x, y, w, h));
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.canvas.set_clip_rect(None);
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let rect = Rect::new(x, y, w, h);
        let surface = self.canvas.read_pixels(rect).backend_err()?;
        let pitch = surface.pitch() as usize;
        let height = surface.height() as usize;
        let width = surface.width() as usize;
        let bpp = 4usize; // RGBA
        // SAFETY: The surface was just created by read_pixels and is not
        // shared; we only read the pixel data before it goes out of scope.
        let data = unsafe { surface.without_lock() }.ok_or_else(|| {
            oasis_core::error::OasisError::Backend("cannot lock surface pixels".into())
        })?;
        // Copy pixel data row by row (pitch may differ from width * bpp).
        let mut pixels = Vec::with_capacity(width * height * bpp);
        for row in 0..height {
            let start = row * pitch;
            let end = start + width * bpp;
            if end <= data.len() {
                pixels.extend_from_slice(&data[start..end]);
            }
        }
        Ok(pixels)
    }

    fn shutdown(&mut self) -> Result<()> {
        log::info!("SDL3 backend shut down");
        Ok(())
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
        self.shape_stroke_rect(x, y, w, h, stroke_width, color)
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
        self.shape_stroke_circle(cx, cy, radius, stroke_width, color)
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
        self.shape_stroke_rounded_rect(x, y, w, h, radius, stroke_width, color)
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
        self.shape_fill_arc(cx, cy, radius, start_angle, end_angle, color)
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
        self.shape_stroke_arc(cx, cy, radius, start_angle, end_angle, width, color)
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
        self.shape_stroke_line_dashed(x1, y1, x2, y2, width, color, dash, gap)
    }
}

// -------------------------------------------------------------------
// SdiGradients: Gradient fills
// -------------------------------------------------------------------

impl SdiGradients for SdlBackend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                let h_max = h.saturating_sub(1).max(1);
                for dy in 0..h as i32 {
                    let color = lerp_color_ratio(top, bottom, dy as u32, h_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(frect(tx, ty + dy, w, 1));
                }
            },
            GradientStyle::Horizontal { left, right } => {
                let w_max = w.saturating_sub(1).max(1);
                for dx in 0..w as i32 {
                    let color = lerp_color_ratio(left, right, dx as u32, w_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(frect(tx + dx, ty, 1, h));
                }
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                let h_max = h.saturating_sub(1).max(1);
                let w_max = w.saturating_sub(1).max(1);
                for dy in 0..h as i32 {
                    let left = lerp_color_ratio(top_left, bottom_left, dy as u32, h_max);
                    let right = lerp_color_ratio(top_right, bottom_right, dy as u32, h_max);
                    for dx in 0..w as i32 {
                        let color = lerp_color_ratio(left, right, dx as u32, w_max);
                        self.set_color(color);
                        let _ = self.canvas.fill_rect(frect(tx + dx, ty + dy, 1, 1));
                    }
                }
            },
        }
        Ok(())
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
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect_gradient(x, y, w, h, gradient);
        }
        // Currently only Vertical gradients get rounded-rect acceleration;
        // other styles fall back to a flat rounded rect to preserve shape.
        let (top_color, bottom_color) = match *gradient {
            GradientStyle::Vertical { top, bottom } => (top, bottom),
            _ => return self.fill_rounded_rect(x, y, w, h, radius, gradient.primary_color()),
        };
        let (tx, ty) = self.translate(x, y);
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        let h_max = (h as i32 - 1).max(1);

        // Draw scanline by scanline, clipping to the rounded rect shape.
        for dy in 0..h as i32 {
            let color = lerp_color_ratio(top_color, bottom_color, dy as u32, h_max as u32);
            self.set_color(color);

            // Compute horizontal inset for rounded corners.
            let inset = rounded_rect_inset(dy, h as i32, r);

            let lx = tx + inset;
            let rx = tx + w as i32 - 1 - inset;
            if lx <= rx {
                let _ = self
                    .canvas
                    .fill_rect(frect(lx, ty + dy, (rx - lx + 1) as u32, 1));
            }
        }
        Ok(())
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
// SdiText: Text system
// -------------------------------------------------------------------

impl SdiText for SdlBackend {
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
        if text.is_empty() || color.a == 0 || font_size == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let mut cx = tx;

        for ch in text.chars() {
            let key = GlyphCacheKey::new(ch, font_size, color, bold, italic);
            if self.glyph_cache.contains_key(&key) {
                // Cache hit: update LRU access counter.
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
            } else {
                // Evict LRU entry when cache is full.
                while self.glyph_cache.len() >= MAX_GLYPH_CACHE_SIZE {
                    if let Some((&oldest_key, _)) =
                        self.glyph_access.iter().min_by_key(|&(_, &ts)| ts)
                    {
                        if let Some(tex_id) = self.glyph_cache.remove(&oldest_key) {
                            self.textures.remove(&tex_id);
                        }
                        self.glyph_access.remove(&oldest_key);
                    } else {
                        break;
                    }
                }
                // Render the glyph to a small RGBA buffer.
                let tex_id = self.render_glyph_texture(ch, font_size, color, bold, italic)?;
                self.glyph_cache.insert(key, tex_id);
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
            }
            // Blit the cached glyph texture.
            if let Some(&tex_id) = self.glyph_cache.get(&key)
                && let Some(texture) = self.textures.get(&tex_id)
            {
                let query = texture.query();
                let _ = self
                    .canvas
                    .copy(texture, None, frect(cx, ty, query.width, query.height));
            }
            cx += oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
        }
        Ok(())
    }

    fn measure_text_height(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 1.2 (the actual rendered row height).
        (f64::from(font_size.max(8)) * 1.2).ceil() as u32
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 0.85 (baseline offset from top).
        (f64::from(font_size.max(8)) * 0.85).ceil() as u32
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
                self.canvas.set_clip_rect(Rect::new(c.x, c.y, c.w, c.h));
            },
            ClipPush::Empty => {
                self.canvas.set_clip_rect(Rect::new(0, 0, 0, 0));
            },
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        match self.clip_stack.pop() {
            Some(prev) => {
                self.canvas
                    .set_clip_rect(Rect::new(prev.x, prev.y, prev.w, prev.h));
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
// SdiBatch: No-op (use default impl)
// -------------------------------------------------------------------

impl SdiBatch for SdlBackend {}

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
        self.glyph_access.clear();
        self.textures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sdl3::keyboard::Keycode;

    use oasis_core::input::{Button, InputEvent, Trigger};

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
        assert_eq!(
            map_key_down(Keycode::Tab),
            Some(InputEvent::ButtonPress(Button::Square))
        );
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
        assert_eq!(
            map_key_up(Keycode::Tab),
            Some(InputEvent::ButtonRelease(Button::Square))
        );
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
        let keys = [
            Keycode::Up,
            Keycode::Down,
            Keycode::Left,
            Keycode::Right,
            Keycode::Return,
            Keycode::Escape,
            Keycode::Space,
            Keycode::Tab,
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
        assert!(backend.glyph_access.is_empty());
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
        assert_eq!(backend.glyph_access.len(), 2);

        // Drawing the same text again should not increase cache
        // size (cache hits).
        backend
            .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
            .unwrap();
        assert_eq!(backend.glyph_cache.len(), 2);
    }
}
