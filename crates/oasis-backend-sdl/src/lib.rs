//! SDL2 backend for OASIS_OS.
//!
//! Implements `SdiBackend` and `InputBackend` using SDL2. Used for desktop
//! development and Raspberry Pi deployment (via SDL2's kmsdrm or X11 backend).
//!
//! Extended primitives (rounded rects, lines, circles, triangles, gradients,
//! sub-rect blits, tinted blits, clip/transform stacks) are implemented using
//! SDL2 renderer API calls and software rasterization helpers.

mod blitting;
mod font;
mod input;
pub mod network;
mod sdl_audio;
mod shapes;

use std::collections::HashMap;

use sdl2::EventPump;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};

use oasis_core::backend::{
    BackendErrExt, Color, GradientStyle, SdiBackend, SdiCore, TextureId, texture_not_found,
    validate_rgba_data,
};
use oasis_core::error::Result;

pub use network::SdlNetworkBackend;
pub use sdl_audio::SdlAudioBackend;

use shapes::{intersect_clip, isqrt, lerp_color_sdl};

// Re-export input helpers for tests.
#[cfg(test)]
use input::{map_key_down, map_key_up};
#[cfg(test)]
use shapes::edge_x;

/// Stored clip rectangle.
#[derive(Clone, Copy)]
pub(crate) struct ClipRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

/// SDL2 rendering and input backend.
///
/// Supports solid-color rects, 8x8 bitmap text, and RGBA texture loading/blitting.
///
/// # Safety
///
/// `textures` is declared before `texture_creator` so that Rust's drop order
/// (declaration order) destroys all textures before the creator they borrow from.
/// The `Texture<'static>` lifetime is erased via transmute in `load_texture()` --
/// this is sound because the `TextureCreator` always outlives the textures.
pub struct SdlBackend {
    pub(crate) canvas: Canvas<Window>,
    pub(crate) event_pump: EventPump,
    pub(crate) textures: HashMap<u64, Texture<'static>>,
    texture_creator: TextureCreator<WindowContext>,
    next_texture_id: u64,
    pub(crate) clip_stack: Vec<ClipRect>,
    pub(crate) translate_stack: Vec<(i32, i32)>,
    pub(crate) cumulative_translate: (i32, i32),
    pub(crate) viewport_w: u32,
    pub(crate) viewport_h: u32,
}

impl SdlBackend {
    /// Create a new SDL2 backend with a window.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self> {
        let sdl = sdl2::init().backend_err()?;
        let video = sdl.video().backend_err()?;
        let window = video
            .window(title, width, height)
            .position_centered()
            .build()
            .backend_err()?;
        let headless =
            std::env::var("SDL_RENDER_DRIVER").is_ok_and(|v| v.eq_ignore_ascii_case("software"));
        let mut builder = window.into_canvas();
        if !headless {
            builder = builder.accelerated().present_vsync();
        }
        let canvas = builder.build().backend_err()?;
        let texture_creator = canvas.texture_creator();
        let event_pump = sdl.event_pump().backend_err()?;

        log::info!("SDL2 backend initialized: {width}x{height}");

        Ok(Self {
            canvas,
            event_pump,
            textures: HashMap::new(),
            texture_creator,
            next_texture_id: 1,
            clip_stack: Vec::new(),
            translate_stack: Vec::new(),
            cumulative_translate: (0, 0),
            viewport_w: width,
            viewport_h: height,
        })
    }

    /// Apply cumulative translation to coordinates.
    pub(crate) fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x + self.cumulative_translate.0,
            y + self.cumulative_translate.1,
        )
    }

    /// Set the SDL draw color with optional blend mode.
    pub(crate) fn set_color(&mut self, color: Color) {
        if color.a < 255 {
            self.canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        } else {
            self.canvas.set_blend_mode(sdl2::render::BlendMode::None);
        }
        self.canvas.set_draw_color(sdl2::pixels::Color::RGBA(
            color.r, color.g, color.b, color.a,
        ));
    }
}

impl SdiCore for SdlBackend {
    fn init(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.canvas.set_draw_color(sdl2::pixels::Color::RGBA(
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
            .copy(texture, None, Rect::new(tx, ty, w, h))
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
        self.canvas
            .fill_rect(Rect::new(tx, ty, w, h))
            .backend_err()?;
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
            .create_texture_streaming(PixelFormatEnum::ABGR8888, width, height)
            .backend_err()?;

        texture
            .with_lock(None, |buffer: &mut [u8], _pitch: usize| {
                buffer[..rgba_data.len()].copy_from_slice(rgba_data);
            })
            .backend_err()?;

        texture.set_blend_mode(sdl2::render::BlendMode::Blend);

        // SAFETY: The texture borrows from self.texture_creator which lives in the
        // same struct. `textures` is declared before `texture_creator`, so Rust drops
        // textures first. The erased lifetime is therefore always valid.
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
        self.canvas
            .read_pixels(rect, PixelFormatEnum::ABGR8888)
            .backend_err()
    }

    fn shutdown(&mut self) -> Result<()> {
        log::info!("SDL2 backend shut down");
        Ok(())
    }
}

impl SdiBackend for SdlBackend {
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
        let (tx, ty) = self.translate(x, y);
        let fs = font_size.max(1) as i32;
        let sdl_color = sdl2::pixels::Color::RGBA(color.r, color.g, color.b, color.a);
        self.canvas.set_draw_color(sdl_color);

        let mut cx = tx;
        for ch in text.chars() {
            let glyph_data = font::glyph(ch);
            let (left_pad, _advance) = font::glyph_metrics(ch);
            let left_pad = left_pad as i32;
            for row in 0..8i32 {
                let bits = glyph_data[row as usize];
                if bits == 0 {
                    continue;
                }
                let oy0 = row * fs / 8;
                let oy1 = (row + 1) * fs / 8;
                let rh = (oy1 - oy0).max(1);
                // Faux-italic: shift top rows rightward (~12-degree).
                let italic_offset = if italic { (7 - row) * fs / 32 } else { 0 };
                for col in 0..8i32 {
                    if bits & (0x80 >> col) != 0 {
                        let src_col = col - left_pad;
                        let ox0 = src_col * fs / 8;
                        let ox1 = (src_col + 1) * fs / 8;
                        let rw = (ox1 - ox0).max(1);
                        let px = cx + ox0 + italic_offset;
                        let py = ty + oy0;
                        if rw == 1 && rh == 1 {
                            let _ = self.canvas.draw_point(sdl2::rect::Point::new(px, py));
                        } else {
                            let _ = self
                                .canvas
                                .fill_rect(Rect::new(px, py, rw as u32, rh as u32));
                        }
                        if bold {
                            let _ = self.canvas.draw_point(sdl2::rect::Point::new(px + 1, py));
                            if rh > 1 {
                                let _ = self.canvas.fill_rect(Rect::new(px + 1, py, 1, rh as u32));
                            }
                        }
                    }
                }
            }
            cx += oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size) as i32;
        }
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Shape Primitives (delegated to shapes.rs)
    // -------------------------------------------------------------------

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

    // -------------------------------------------------------------------
    // Extended: Gradient Fills
    // -------------------------------------------------------------------

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
                    let color = lerp_color_sdl(top, bottom, dy as u32, h_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(Rect::new(tx, ty + dy, w, 1));
                }
            },
            GradientStyle::Horizontal { left, right } => {
                let w_max = w.saturating_sub(1).max(1);
                for dx in 0..w as i32 {
                    let color = lerp_color_sdl(left, right, dx as u32, w_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(Rect::new(tx + dx, ty, 1, h));
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
                    let left = lerp_color_sdl(top_left, bottom_left, dy as u32, h_max);
                    let right = lerp_color_sdl(top_right, bottom_right, dy as u32, h_max);
                    for dx in 0..w as i32 {
                        let color = lerp_color_sdl(left, right, dx as u32, w_max);
                        self.set_color(color);
                        let _ = self.canvas.fill_rect(Rect::new(tx + dx, ty + dy, 1, 1));
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
            let color = lerp_color_sdl(top_color, bottom_color, dy as u32, h_max as u32);
            self.set_color(color);

            // Compute horizontal inset for rounded corners.
            let inset = if dy < r {
                // Top corners.
                let ry = r - dy;
                r - isqrt((r * r - ry * ry).max(0))
            } else if dy >= h as i32 - r {
                // Bottom corners.
                let ry = dy - (h as i32 - 1 - r);
                r - isqrt((r * r - ry * ry).max(0))
            } else {
                0
            };

            let lx = tx + inset;
            let rx = tx + w as i32 - 1 - inset;
            if lx <= rx {
                let _ = self
                    .canvas
                    .fill_rect(Rect::new(lx, ty + dy, (rx - lx + 1) as u32, 1));
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Texture Operations (delegated to blitting.rs)
    // -------------------------------------------------------------------

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

    // -------------------------------------------------------------------
    // Extended: Text System
    // -------------------------------------------------------------------

    fn measure_text_height(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 1.2 (the actual rendered row height).
        (f64::from(font_size.max(8)) * 1.2).ceil() as u32
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        // Match WASM: font_size * 0.85 (baseline offset from top).
        (f64::from(font_size.max(8)) * 0.85).ceil() as u32
    }

    // -------------------------------------------------------------------
    // Extended: Clip and Transform Stack
    // -------------------------------------------------------------------

    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let new_clip = ClipRect { x: tx, y: ty, w, h };
        if let Some(current_sdl) = self.canvas.clip_rect() {
            let current = ClipRect {
                x: current_sdl.x(),
                y: current_sdl.y(),
                w: current_sdl.width(),
                h: current_sdl.height(),
            };
            self.clip_stack.push(current);
            let isect = intersect_clip(&current, &new_clip);
            if let Some(c) = isect {
                self.canvas.set_clip_rect(Rect::new(c.x, c.y, c.w, c.h));
            } else {
                self.canvas.set_clip_rect(Rect::new(0, 0, 0, 0));
            }
        } else {
            self.clip_stack.push(ClipRect {
                x: 0,
                y: 0,
                w: self.viewport_w,
                h: self.viewport_h,
            });
            self.canvas
                .set_clip_rect(Rect::new(new_clip.x, new_clip.y, new_clip.w, new_clip.h));
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        if let Some(prev) = self.clip_stack.pop() {
            if prev.x == 0 && prev.y == 0 && prev.w == self.viewport_w && prev.h == self.viewport_h
            {
                self.canvas.set_clip_rect(None);
            } else {
                self.canvas
                    .set_clip_rect(Rect::new(prev.x, prev.y, prev.w, prev.h));
            }
        } else {
            self.canvas.set_clip_rect(None);
        }
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.canvas
            .clip_rect()
            .map(|r| (r.x(), r.y(), r.width(), r.height()))
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(self.cumulative_translate);
        self.cumulative_translate.0 += dx;
        self.cumulative_translate.1 += dy;
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        if let Some(prev) = self.translate_stack.pop() {
            self.cumulative_translate = prev;
        }
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.cumulative_translate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sdl2::keyboard::Keycode;

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
        assert_eq!(map_key_down(Keycode::Num0), None);
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
        // Every mapped key_down should have a corresponding key_up (except Backspace).
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
                    assert_eq!(b1, b2, "press/release button mismatch for {key:?}");
                },
                (InputEvent::TriggerPress(t1), InputEvent::TriggerRelease(t2)) => {
                    assert_eq!(t1, t2, "press/release trigger mismatch for {key:?}");
                },
                (d, u) => panic!("unexpected pair for {key:?}: {d:?} / {u:?}"),
            }
        }
    }

    // ---------------------------------------------------------------
    // Helper function tests
    // ---------------------------------------------------------------

    #[test]
    fn intersect_clip_overlapping() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        let b = ClipRect {
            x: 50,
            y: 50,
            w: 100,
            h: 100,
        };
        let r = intersect_clip(&a, &b).unwrap();
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 50);
        assert_eq!(r.w, 50);
        assert_eq!(r.h, 50);
    }

    #[test]
    fn intersect_clip_contained() {
        let outer = ClipRect {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        };
        let inner = ClipRect {
            x: 10,
            y: 20,
            w: 50,
            h: 30,
        };
        let r = intersect_clip(&outer, &inner).unwrap();
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.w, 50);
        assert_eq!(r.h, 30);
    }

    #[test]
    fn intersect_clip_no_overlap() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        };
        let b = ClipRect {
            x: 100,
            y: 100,
            w: 50,
            h: 50,
        };
        assert!(intersect_clip(&a, &b).is_none());
    }

    #[test]
    fn intersect_clip_touching_edge() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        };
        let b = ClipRect {
            x: 50,
            y: 0,
            w: 50,
            h: 50,
        };
        // Touching at edge but not overlapping.
        assert!(intersect_clip(&a, &b).is_none());
    }

    #[test]
    fn intersect_clip_same_rect() {
        let a = ClipRect {
            x: 10,
            y: 20,
            w: 100,
            h: 80,
        };
        let r = intersect_clip(&a, &a).unwrap();
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.w, 100);
        assert_eq!(r.h, 80);
    }

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(25), 5);
        assert_eq!(isqrt(100), 10);
    }

    #[test]
    fn isqrt_non_perfect_squares() {
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(5), 2);
        assert_eq!(isqrt(8), 2);
        assert_eq!(isqrt(10), 3);
        assert_eq!(isqrt(99), 9);
    }

    #[test]
    fn isqrt_negative() {
        assert_eq!(isqrt(-1), 0);
        assert_eq!(isqrt(-100), 0);
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

    #[test]
    fn lerp_color_sdl_endpoints() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(255, 255, 255);
        let at_start = lerp_color_sdl(a, b, 0, 10);
        assert_eq!(at_start, a);
        let at_end = lerp_color_sdl(a, b, 10, 10);
        assert_eq!(at_end, b);
    }

    #[test]
    fn lerp_color_sdl_midpoint() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(200, 100, 50);
        let mid = lerp_color_sdl(a, b, 5, 10);
        assert_eq!(mid.r, 100);
        assert_eq!(mid.g, 50);
        assert_eq!(mid.b, 25);
    }

    #[test]
    fn lerp_color_sdl_zero_denominator() {
        let a = Color::rgb(42, 42, 42);
        let b = Color::rgb(200, 200, 200);
        let result = lerp_color_sdl(a, b, 5, 0);
        assert_eq!(result, a);
    }

    // ---------------------------------------------------------------
    // SDL rendering correctness tests (require display)
    //
    // These tests require a working SDL2 display. In CI, set
    // SDL_VIDEODRIVER=dummy (or x11/wayland) and run:
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
}
