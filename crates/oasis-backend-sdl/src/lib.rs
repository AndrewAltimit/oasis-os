//! SDL2 backend for OASIS_OS.
//!
//! Implements `SdiBackend` and `InputBackend` using SDL2. Used for desktop
//! development and Raspberry Pi deployment (via SDL2's kmsdrm or X11 backend).
//!
//! Extended primitives (rounded rects, lines, circles, triangles, gradients,
//! sub-rect blits, tinted blits, clip/transform stacks) are implemented using
//! SDL2 renderer API calls and software rasterization helpers.

mod font;
mod sdl_audio;

use std::collections::HashMap;

use sdl2::EventPump;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};

use oasis_core::backend::{Color, SdiBackend, TextureId};
use oasis_core::error::{OasisError, Result};
use oasis_core::input::{Button, InputEvent, Trigger};

pub use sdl_audio::SdlAudioBackend;

/// Stored clip rectangle.
#[derive(Clone, Copy)]
struct ClipRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
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
    canvas: Canvas<Window>,
    event_pump: EventPump,
    textures: HashMap<u64, Texture<'static>>,
    texture_creator: TextureCreator<WindowContext>,
    next_texture_id: u64,
    clip_stack: Vec<ClipRect>,
    translate_stack: Vec<(i32, i32)>,
    cumulative_translate: (i32, i32),
    viewport_w: u32,
    viewport_h: u32,
}

impl SdlBackend {
    /// Create a new SDL2 backend with a window.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self> {
        let sdl = sdl2::init().map_err(|e| OasisError::Backend(e.to_string()))?;
        let video = sdl
            .video()
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        let window = video
            .window(title, width, height)
            .position_centered()
            .build()
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        let canvas = window
            .into_canvas()
            .accelerated()
            .present_vsync()
            .build()
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        let texture_creator = canvas.texture_creator();
        let event_pump = sdl
            .event_pump()
            .map_err(|e| OasisError::Backend(e.to_string()))?;

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
    fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x + self.cumulative_translate.0,
            y + self.cumulative_translate.1,
        )
    }

    /// Set the SDL draw color with optional blend mode.
    fn set_color(&mut self, color: Color) {
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

impl SdiBackend for SdlBackend {
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
            .ok_or_else(|| OasisError::Backend(format!("texture not found: {}", tex.0)))?;
        self.canvas
            .copy(texture, None, Rect::new(tx, ty, w, h))
            .map_err(|e| OasisError::Backend(e.to_string()))?;
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
        let scale = if font_size >= 8 {
            (font_size / 8) as i32
        } else {
            1
        };
        let glyph_w = (font::GLYPH_WIDTH as i32) * scale;
        let sdl_color = sdl2::pixels::Color::RGBA(color.r, color.g, color.b, color.a);
        self.canvas.set_draw_color(sdl_color);

        let mut cx = tx;
        for ch in text.chars() {
            let glyph_data = font::glyph(ch);
            for row in 0..8i32 {
                let bits = glyph_data[row as usize];
                for col in 0..8i32 {
                    if bits & (0x80 >> col) != 0 {
                        let px = cx + col * scale;
                        let py = ty + row * scale;
                        if scale == 1 {
                            let _ = self.canvas.draw_point(sdl2::rect::Point::new(px, py));
                        } else {
                            let _ = self.canvas.fill_rect(Rect::new(
                                px,
                                py,
                                scale as u32,
                                scale as u32,
                            ));
                        }
                    }
                }
            }
            cx += glyph_w;
        }
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.set_color(color);
        self.canvas
            .fill_rect(Rect::new(tx, ty, w, h))
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.canvas.present();
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let expected = (width * height * 4) as usize;
        if rgba_data.len() != expected {
            return Err(OasisError::Backend(format!(
                "texture data size mismatch: expected {expected}, got {}",
                rgba_data.len()
            )));
        }

        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormatEnum::ABGR8888, width, height)
            .map_err(|e| OasisError::Backend(e.to_string()))?;

        texture
            .with_lock(None, |buffer: &mut [u8], _pitch: usize| {
                buffer[..expected].copy_from_slice(rgba_data);
            })
            .map_err(|e| OasisError::Backend(e.to_string()))?;

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
        let scale = if font_size >= 8 {
            (font_size / 8) as u32
        } else {
            1
        };
        text.len() as u32 * 8 * scale
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let rect = Rect::new(x, y, w, h);
        self.canvas
            .read_pixels(rect, PixelFormatEnum::ABGR8888)
            .map_err(|e| OasisError::Backend(e.to_string()))
    }

    fn shutdown(&mut self) -> Result<()> {
        log::info!("SDL2 backend shut down");
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Shape Primitives
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
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let (tx, ty) = self.translate(x, y);
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;
        self.set_color(color);

        // Center body rect.
        let _ = self
            .canvas
            .fill_rect(Rect::new(tx, ty + r, w, h - r as u32 * 2));
        // Top strip.
        let _ = self
            .canvas
            .fill_rect(Rect::new(tx + r, ty, w - r as u32 * 2, r as u32));
        // Bottom strip.
        let _ = self.canvas.fill_rect(Rect::new(
            tx + r,
            ty + h as i32 - r,
            w - r as u32 * 2,
            r as u32,
        ));

        // Corner fills using midpoint circle horizontal spans.
        let mut cx = 0i32;
        let mut cy = r;
        let mut d = 1 - r;
        while cx <= cy {
            // Top-left + top-right.
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + r - cy, ty + r - cx),
                sdl2::rect::Point::new(tx + w as i32 - 1 - r + cy, ty + r - cx),
            );
            if cx != cy {
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tx + r - cx, ty + r - cy),
                    sdl2::rect::Point::new(tx + w as i32 - 1 - r + cx, ty + r - cy),
                );
            }
            // Bottom-left + bottom-right.
            if cx != 0 {
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tx + r - cy, ty + h as i32 - 1 - r + cx),
                    sdl2::rect::Point::new(tx + w as i32 - 1 - r + cy, ty + h as i32 - 1 - r + cx),
                );
            }
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + r - cx, ty + h as i32 - 1 - r + cy),
                sdl2::rect::Point::new(tx + w as i32 - 1 - r + cx, ty + h as i32 - 1 - r + cy),
            );

            cx += 1;
            if d < 0 {
                d += 2 * cx + 1;
            } else {
                cy -= 1;
                d += 2 * (cx - cy) + 1;
            }
        }
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
        self.set_color(color);
        if stroke_width == 1 {
            let _ = self.canvas.draw_rect(Rect::new(tx, ty, w, h));
        } else {
            let sw = stroke_width as u32;
            let _ = self.canvas.fill_rect(Rect::new(tx, ty, w, sw));
            let _ = self
                .canvas
                .fill_rect(Rect::new(tx, ty + h as i32 - sw as i32, w, sw));
            let _ =
                self.canvas
                    .fill_rect(Rect::new(tx, ty + sw as i32, sw, h.saturating_sub(sw * 2)));
            let _ = self.canvas.fill_rect(Rect::new(
                tx + w as i32 - sw as i32,
                ty + sw as i32,
                sw,
                h.saturating_sub(sw * 2),
            ));
        }
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
        self.set_color(color);
        if width <= 1 {
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx1, ty1),
                sdl2::rect::Point::new(tx2, ty2),
            );
        } else {
            // Draw multiple parallel lines for thickness.
            let half = width as i32 / 2;
            let dx = (tx2 - tx1) as f32;
            let dy = (ty2 - ty1) as f32;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let nx = (-dy / len) as i32;
            let ny = (dx / len) as i32;
            for i in -half..=(width as i32 - half - 1) {
                let ox = nx * i;
                let oy = ny * i;
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tx1 + ox, ty1 + oy),
                    sdl2::rect::Point::new(tx2 + ox, ty2 + oy),
                );
            }
        }
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        let r = radius as i32;
        self.set_color(color);

        let mut x = 0i32;
        let mut y = r;
        let mut d = 1 - r;
        while x <= y {
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tcx - y, tcy + x),
                sdl2::rect::Point::new(tcx + y, tcy + x),
            );
            if x != 0 {
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tcx - y, tcy - x),
                    sdl2::rect::Point::new(tcx + y, tcy - x),
                );
            }
            if x != y {
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tcx - x, tcy + y),
                    sdl2::rect::Point::new(tcx + x, tcy + y),
                );
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(tcx - x, tcy - y),
                    sdl2::rect::Point::new(tcx + x, tcy - y),
                );
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
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
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        let (tx3, ty3) = self.translate(x3, y3);

        // Sort by y.
        let mut verts = [(tx1, ty1), (tx2, ty2), (tx3, ty3)];
        verts.sort_by_key(|v| v.1);
        let (vx0, vy0) = verts[0];
        let (vx1, vy1) = verts[1];
        let (vx2, vy2) = verts[2];

        self.set_color(color);

        for y in vy0..=vy2 {
            let mut x_min = i32::MAX;
            let mut x_max = i32::MIN;

            let x_02 = edge_x(vx0, vy0, vx2, vy2, y);
            x_min = x_min.min(x_02);
            x_max = x_max.max(x_02);

            if y <= vy1 && vy0 != vy1 {
                let x_01 = edge_x(vx0, vy0, vx1, vy1, y);
                x_min = x_min.min(x_01);
                x_max = x_max.max(x_01);
            }
            if y >= vy1 && vy1 != vy2 {
                let x_12 = edge_x(vx1, vy1, vx2, vy2, y);
                x_min = x_min.min(x_12);
                x_max = x_max.max(x_12);
            }
            if y == vy1 {
                x_min = x_min.min(vx1);
                x_max = x_max.max(vx1);
            }

            if x_min <= x_max {
                let _ = self.canvas.draw_line(
                    sdl2::rect::Point::new(x_min, y),
                    sdl2::rect::Point::new(x_max, y),
                );
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: Gradient Fills
    // -------------------------------------------------------------------

    fn fill_rect_gradient_v(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top_color: Color,
        bottom_color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let h_max = h.saturating_sub(1).max(1);
        for dy in 0..h as i32 {
            let color = lerp_color_sdl(top_color, bottom_color, dy as u32, h_max);
            self.set_color(color);
            let _ = self.canvas.fill_rect(Rect::new(tx, ty + dy, w, 1));
        }
        Ok(())
    }

    fn fill_rect_gradient_h(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        left_color: Color,
        right_color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        let w_max = w.saturating_sub(1).max(1);
        for dx in 0..w as i32 {
            let color = lerp_color_sdl(left_color, right_color, dx as u32, w_max);
            self.set_color(color);
            let _ = self.canvas.fill_rect(Rect::new(tx + dx, ty, 1, h));
        }
        Ok(())
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
        if radius == 0 || w == 0 || h == 0 {
            return self.stroke_rect(x, y, w, h, stroke_width, color);
        }
        let (tx, ty) = self.translate(x, y);
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        self.set_color(color);

        let sw = (stroke_width as i32).max(1);
        for t in 0..sw {
            // Top edge.
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + r, ty + t),
                sdl2::rect::Point::new(tx + w as i32 - 1 - r, ty + t),
            );
            // Bottom edge.
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + r, ty + h as i32 - 1 - t),
                sdl2::rect::Point::new(tx + w as i32 - 1 - r, ty + h as i32 - 1 - t),
            );
            // Left edge.
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + t, ty + r),
                sdl2::rect::Point::new(tx + t, ty + h as i32 - 1 - r),
            );
            // Right edge.
            let _ = self.canvas.draw_line(
                sdl2::rect::Point::new(tx + w as i32 - 1 - t, ty + r),
                sdl2::rect::Point::new(tx + w as i32 - 1 - t, ty + h as i32 - 1 - r),
            );

            // Rounded corners via midpoint circle arc.
            let cr = r - t;
            if cr <= 0 {
                continue;
            }
            let mut cx = 0i32;
            let mut cy = cr;
            let mut d = 1 - cr;
            while cx <= cy {
                // Top-left corner.
                let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                    tx + r - cy,
                    ty + r - cx,
                ));
                if cx != cy {
                    let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                        tx + r - cx,
                        ty + r - cy,
                    ));
                }
                // Top-right corner.
                let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                    tx + w as i32 - 1 - r + cy,
                    ty + r - cx,
                ));
                if cx != cy {
                    let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                        tx + w as i32 - 1 - r + cx,
                        ty + r - cy,
                    ));
                }
                // Bottom-left corner.
                if cx != 0 {
                    let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                        tx + r - cy,
                        ty + h as i32 - 1 - r + cx,
                    ));
                }
                let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                    tx + r - cx,
                    ty + h as i32 - 1 - r + cy,
                ));
                // Bottom-right corner.
                if cx != 0 {
                    let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                        tx + w as i32 - 1 - r + cy,
                        ty + h as i32 - 1 - r + cx,
                    ));
                }
                let _ = self.canvas.draw_point(sdl2::rect::Point::new(
                    tx + w as i32 - 1 - r + cx,
                    ty + h as i32 - 1 - r + cy,
                ));

                cx += 1;
                if d < 0 {
                    d += 2 * cx + 1;
                } else {
                    cy -= 1;
                    d += 2 * (cx - cy) + 1;
                }
            }
        }
        Ok(())
    }

    fn fill_rounded_rect_gradient_v(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        top_color: Color,
        bottom_color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect_gradient_v(x, y, w, h, top_color, bottom_color);
        }
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
                let _ = self.canvas.fill_rect(Rect::new(
                    lx,
                    ty + dy,
                    (rx - lx + 1) as u32,
                    1,
                ));
            }
        }
        Ok(())
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

    // -------------------------------------------------------------------
    // Extended: Texture Operations
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
        let (tx, ty) = self.translate(dst_x, dst_y);
        let texture = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| OasisError::Backend(format!("texture not found: {}", tex.0)))?;
        let src_rect = Rect::new(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = Rect::new(tx, ty, dst_w, dst_h);
        self.canvas
            .copy(texture, src_rect, dst_rect)
            .map_err(|e| OasisError::Backend(e.to_string()))?;
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
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| OasisError::Backend(format!("texture not found: {}", tex.0)))?;
        texture.set_color_mod(tint.r, tint.g, tint.b);
        texture.set_alpha_mod(tint.a);
        let dst_rect = Rect::new(tx, ty, w, h);
        self.canvas
            .copy(texture, None, dst_rect)
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        // Reset modulation.
        let texture = self.textures.get_mut(&tex.0).unwrap();
        texture.set_color_mod(255, 255, 255);
        texture.set_alpha_mod(255);
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
        let (tx, ty) = self.translate(dst_x, dst_y);
        let texture = self
            .textures
            .get_mut(&tex.0)
            .ok_or_else(|| OasisError::Backend(format!("texture not found: {}", tex.0)))?;
        texture.set_color_mod(tint.r, tint.g, tint.b);
        texture.set_alpha_mod(tint.a);
        let src_rect = Rect::new(src_x as i32, src_y as i32, src_w, src_h);
        let dst_rect = Rect::new(tx, ty, dst_w, dst_h);
        self.canvas
            .copy(texture, src_rect, dst_rect)
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        let texture = self.textures.get_mut(&tex.0).unwrap();
        texture.set_color_mod(255, 255, 255);
        texture.set_alpha_mod(255);
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
        let (tx, ty) = self.translate(x, y);
        let texture = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| OasisError::Backend(format!("texture not found: {}", tex.0)))?;
        let dst_rect = Rect::new(tx, ty, w, h);
        self.canvas
            .copy_ex(texture, None, dst_rect, 0.0, None, flip_h, flip_v)
            .map_err(|e| OasisError::Backend(e.to_string()))?;
        Ok(())
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

impl oasis_core::backend::InputBackend for SdlBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for event in self.event_pump.poll_iter() {
            if let Some(e) = map_sdl_event(event) {
                events.push(e);
            }
        }
        events
    }
}

/// Map an SDL2 event to an OASIS_OS input event.
fn map_sdl_event(event: Event) -> Option<InputEvent> {
    match event {
        Event::Quit { .. } => Some(InputEvent::Quit),
        Event::KeyDown {
            keycode: Some(key), ..
        } => map_key_down(key),
        Event::KeyUp {
            keycode: Some(key), ..
        } => map_key_up(key),
        Event::MouseMotion { x, y, .. } => Some(InputEvent::CursorMove { x, y }),
        Event::MouseButtonDown { x, y, .. } => Some(InputEvent::PointerClick { x, y }),
        Event::MouseButtonUp { x, y, .. } => Some(InputEvent::PointerRelease { x, y }),
        Event::Window {
            win_event: sdl2::event::WindowEvent::FocusGained,
            ..
        } => Some(InputEvent::FocusGained),
        Event::Window {
            win_event: sdl2::event::WindowEvent::FocusLost,
            ..
        } => Some(InputEvent::FocusLost),
        Event::TextInput { text, .. } => text.chars().next().map(InputEvent::TextInput),
        _ => None,
    }
}

fn map_key_down(key: Keycode) -> Option<InputEvent> {
    match key {
        Keycode::Up => Some(InputEvent::ButtonPress(Button::Up)),
        Keycode::Down => Some(InputEvent::ButtonPress(Button::Down)),
        Keycode::Left => Some(InputEvent::ButtonPress(Button::Left)),
        Keycode::Right => Some(InputEvent::ButtonPress(Button::Right)),
        Keycode::Return => Some(InputEvent::ButtonPress(Button::Confirm)),
        Keycode::Escape => Some(InputEvent::ButtonPress(Button::Cancel)),
        Keycode::Space => Some(InputEvent::ButtonPress(Button::Triangle)),
        Keycode::Tab => Some(InputEvent::ButtonPress(Button::Square)),
        Keycode::F1 => Some(InputEvent::ButtonPress(Button::Start)),
        Keycode::F2 => Some(InputEvent::ButtonPress(Button::Select)),
        Keycode::Backspace => Some(InputEvent::Backspace),
        Keycode::Q => Some(InputEvent::TriggerPress(Trigger::Left)),
        Keycode::E => Some(InputEvent::TriggerPress(Trigger::Right)),
        _ => None,
    }
}

fn map_key_up(key: Keycode) -> Option<InputEvent> {
    match key {
        Keycode::Up => Some(InputEvent::ButtonRelease(Button::Up)),
        Keycode::Down => Some(InputEvent::ButtonRelease(Button::Down)),
        Keycode::Left => Some(InputEvent::ButtonRelease(Button::Left)),
        Keycode::Right => Some(InputEvent::ButtonRelease(Button::Right)),
        Keycode::Return => Some(InputEvent::ButtonRelease(Button::Confirm)),
        Keycode::Escape => Some(InputEvent::ButtonRelease(Button::Cancel)),
        Keycode::Space => Some(InputEvent::ButtonRelease(Button::Triangle)),
        Keycode::Tab => Some(InputEvent::ButtonRelease(Button::Square)),
        Keycode::F1 => Some(InputEvent::ButtonRelease(Button::Start)),
        Keycode::F2 => Some(InputEvent::ButtonRelease(Button::Select)),
        Keycode::Q => Some(InputEvent::TriggerRelease(Trigger::Left)),
        Keycode::E => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}

/// Compute the intersection of two clip rectangles.
fn intersect_clip(a: &ClipRect, b: &ClipRect) -> Option<ClipRect> {
    let ax2 = a.x.saturating_add(a.w as i32);
    let ay2 = a.y.saturating_add(a.h as i32);
    let bx2 = b.x.saturating_add(b.w as i32);
    let by2 = b.y.saturating_add(b.h as i32);
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let x2 = ax2.min(bx2);
    let y2 = ay2.min(by2);
    if x2 > x && y2 > y {
        Some(ClipRect {
            x,
            y,
            w: (x2 - x) as u32,
            h: (y2 - y) as u32,
        })
    } else {
        None
    }
}

/// Compute the x coordinate along an edge at a given y.
fn edge_x(x0: i32, y0: i32, x1: i32, y1: i32, y: i32) -> i32 {
    if y1 == y0 {
        return x0;
    }
    x0 + (x1 - x0) * (y - y0) / (y1 - y0)
}

/// Integer square root (floor).
fn isqrt(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut x = (n as f32).sqrt() as i32;
    // Newton correction.
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// Linear interpolation between two colors.
fn lerp_color_sdl(a: Color, b: Color, num: u32, den: u32) -> Color {
    if den == 0 {
        return a;
    }
    let inv = den - num;
    Color::rgba(
        ((a.r as u32 * inv + b.r as u32 * num + den / 2) / den) as u8,
        ((a.g as u32 * inv + b.g as u32 * num + den / 2) / den) as u8,
        ((a.b as u32 * inv + b.b as u32 * num + den / 2) / den) as u8,
        ((a.a as u32 * inv + b.a as u32 * num + den / 2) / den) as u8,
    )
}
