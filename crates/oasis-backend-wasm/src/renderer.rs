//! `SdiBackend` implementation using the Canvas 2D API.

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

use oasis_types::backend::{Color, GradientStyle, SdiBackend, TextMetrics, TextureId};
use oasis_types::error::{OasisError, Result};

use crate::font;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn color_to_css(c: Color) -> String {
    if c.a == 255 {
        format!("rgb({},{},{})", c.r, c.g, c.b)
    } else {
        format!("rgba({},{},{},{})", c.r, c.g, c.b, f64::from(c.a) / 255.0)
    }
}

fn js_err<E: std::fmt::Debug>(e: E) -> OasisError {
    OasisError::Backend(format!("{e:?}"))
}

// ---------------------------------------------------------------------------
// Texture storage
// ---------------------------------------------------------------------------

struct TextureData {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// Clip rectangle
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ClipRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

// ---------------------------------------------------------------------------
// WasmBackend
// ---------------------------------------------------------------------------

pub struct WasmBackend {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    width: u32,
    height: u32,
    textures: HashMap<u64, TextureData>,
    next_texture_id: u64,
    clip_stack: Vec<ClipRect>,
    translate_stack: Vec<(i32, i32)>,
    cumulative_translate: (i32, i32),
}

impl WasmBackend {
    /// Create a new WASM backend attached to a `<canvas>` element.
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self> {
        let ctx = canvas
            .get_context("2d")
            .map_err(js_err)?
            .ok_or_else(|| OasisError::Backend("failed to get 2d context".into()))?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(js_err)?;

        let width = canvas.width();
        let height = canvas.height();

        // Disable image smoothing for crisp pixel art scaling.
        ctx.set_image_smoothing_enabled(false);

        Ok(Self {
            canvas,
            ctx,
            width,
            height,
            textures: HashMap::new(),
            next_texture_id: 1,
            clip_stack: Vec::new(),
            translate_stack: Vec::new(),
            cumulative_translate: (0, 0),
        })
    }

    /// Get the underlying canvas element.
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    fn translate(&self, x: i32, y: i32) -> (f64, f64) {
        let (tx, ty) = self.cumulative_translate;
        ((x + tx) as f64, (y + ty) as f64)
    }
}

// ---------------------------------------------------------------------------
// SdiBackend — core methods (13 required)
// ---------------------------------------------------------------------------

impl SdiBackend for WasmBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.canvas.set_width(width);
        self.canvas.set_height(height);
        self.ctx.set_image_smoothing_enabled(false);
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.ctx.set_fill_style_str(&color_to_css(color));
        self.ctx
            .fill_rect(0.0, 0.0, self.width as f64, self.height as f64);
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        // Canvas 2D is immediate-mode; nothing to swap.
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        self.ctx.set_fill_style_str(&color_to_css(color));
        self.ctx.fill_rect(tx, ty, w as f64, h as f64);
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
        if text.is_empty() || color.a == 0 || font_size == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);

        // Use bitmap font rasteriser for pixel-perfect consistency with other
        // backends. Each glyph is an 8x8 bitmap scaled by font_size/8.
        let scale = f64::from(font_size) / 8.0;
        self.ctx.set_fill_style_str(&color_to_css(color));

        let mut cx = tx;
        for ch in text.chars() {
            let glyph = font::glyph(ch);
            let (left, _advance) = font::glyph_metrics(ch);
            cx += f64::from(left) * scale;

            for row in 0..8u8 {
                let bits = glyph[row as usize];
                for col in 0..8u8 {
                    if bits & (1 << (7 - col)) != 0 {
                        self.ctx.fill_rect(
                            cx + f64::from(col) * scale,
                            ty + f64::from(row) * scale,
                            scale.ceil(),
                            scale.ceil(),
                        );
                    }
                }
            }
            cx += font::glyph_advance_scaled(ch, font_size) as f64;
        }
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let td = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| OasisError::Backend(format!("texture {} not found", tex.0)))?;

        let image_data = ImageData::new_with_u8_clamped_array_and_sh(
            wasm_bindgen::Clamped(&td.data),
            td.width,
            td.height,
        )
        .map_err(js_err)?;

        let (tx, ty) = self.translate(x, y);

        // If destination size matches source, use putImageData for speed.
        if w == td.width && h == td.height {
            self.ctx
                .put_image_data(&image_data, tx, ty)
                .map_err(js_err)?;
        } else {
            // Create an offscreen canvas for scaling.
            let doc = web_sys::window()
                .ok_or_else(|| OasisError::Backend("no window".into()))?
                .document()
                .ok_or_else(|| OasisError::Backend("no document".into()))?;
            let offscreen = doc
                .create_element("canvas")
                .map_err(js_err)?
                .dyn_into::<HtmlCanvasElement>()
                .map_err(js_err)?;
            offscreen.set_width(td.width);
            offscreen.set_height(td.height);
            let off_ctx = offscreen
                .get_context("2d")
                .map_err(js_err)?
                .ok_or_else(|| OasisError::Backend("offscreen ctx".into()))?
                .dyn_into::<CanvasRenderingContext2d>()
                .map_err(js_err)?;
            off_ctx
                .put_image_data(&image_data, 0.0, 0.0)
                .map_err(js_err)?;
            self.ctx
                .draw_image_with_html_canvas_element_and_dw_and_dh(
                    &offscreen, tx, ty, w as f64, h as f64,
                )
                .map_err(js_err)?;
        }
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let expected = (width * height * 4) as usize;
        if rgba_data.len() != expected {
            return Err(OasisError::Backend(format!(
                "texture data length mismatch: expected {expected}, got {}",
                rgba_data.len()
            )));
        }
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(
            id,
            TextureData {
                data: rgba_data.to_vec(),
                width,
                height,
            },
        );
        Ok(TextureId(id))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        self.textures.remove(&tex.0);
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(tx, ty, w as f64, h as f64);
        self.ctx.clip();
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.ctx.restore();
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_types::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        let data = self
            .ctx
            .get_image_data(x as f64, y as f64, w as f64, h as f64)
            .map_err(js_err)?;
        Ok(data.data().to_vec())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.textures.clear();
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: shape primitives
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
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let fw = w as f64;
        let fh = h as f64;
        let r = f64::from(radius).min(fw / 2.0).min(fh / 2.0);

        self.ctx.begin_path();
        self.ctx.move_to(tx + r, ty);
        self.ctx.line_to(tx + fw - r, ty);
        self.ctx
            .arc_to(tx + fw, ty, tx + fw, ty + r, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx + fw, ty + fh - r);
        self.ctx
            .arc_to(tx + fw, ty + fh, tx + fw - r, ty + fh, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx + r, ty + fh);
        self.ctx
            .arc_to(tx, ty + fh, tx, ty + fh - r, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx, ty + r);
        self.ctx.arc_to(tx, ty, tx + r, ty, r).map_err(js_err)?;
        self.ctx.close_path();
        self.ctx.set_fill_style_str(&color_to_css(color));
        self.ctx.fill();
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
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        self.ctx.set_stroke_style_str(&color_to_css(color));
        self.ctx.set_line_width(f64::from(stroke_width));
        let offset = f64::from(stroke_width) / 2.0;
        self.ctx.stroke_rect(
            tx + offset,
            ty + offset,
            w as f64 - f64::from(stroke_width),
            h as f64 - f64::from(stroke_width),
        );
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
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let sw = f64::from(stroke_width);
        let offset = sw / 2.0;
        let fw = w as f64 - sw;
        let fh = h as f64 - sw;
        let r = f64::from(radius).min(fw / 2.0).min(fh / 2.0);

        self.ctx.begin_path();
        let bx = tx + offset;
        let by = ty + offset;
        self.ctx.move_to(bx + r, by);
        self.ctx.line_to(bx + fw - r, by);
        self.ctx
            .arc_to(bx + fw, by, bx + fw, by + r, r)
            .map_err(js_err)?;
        self.ctx.line_to(bx + fw, by + fh - r);
        self.ctx
            .arc_to(bx + fw, by + fh, bx + fw - r, by + fh, r)
            .map_err(js_err)?;
        self.ctx.line_to(bx + r, by + fh);
        self.ctx
            .arc_to(bx, by + fh, bx, by + fh - r, r)
            .map_err(js_err)?;
        self.ctx.line_to(bx, by + r);
        self.ctx.arc_to(bx, by, bx + r, by, r).map_err(js_err)?;
        self.ctx.close_path();
        self.ctx.set_stroke_style_str(&color_to_css(color));
        self.ctx.set_line_width(sw);
        self.ctx.stroke();
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
        if color.a == 0 {
            return Ok(());
        }
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.ctx.begin_path();
        self.ctx.move_to(tx1, ty1);
        self.ctx.line_to(tx2, ty2);
        self.ctx.set_stroke_style_str(&color_to_css(color));
        self.ctx.set_line_width(f64::from(width));
        self.ctx.stroke();
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        if radius == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(cx, cy);
        self.ctx.begin_path();
        self.ctx
            .arc(tx, ty, f64::from(radius), 0.0, std::f64::consts::TAU)
            .map_err(js_err)?;
        self.ctx.set_fill_style_str(&color_to_css(color));
        self.ctx.fill();
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
        if radius == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(cx, cy);
        self.ctx.begin_path();
        self.ctx
            .arc(tx, ty, f64::from(radius), 0.0, std::f64::consts::TAU)
            .map_err(js_err)?;
        self.ctx.set_stroke_style_str(&color_to_css(color));
        self.ctx.set_line_width(f64::from(stroke_width));
        self.ctx.stroke();
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
        if color.a == 0 {
            return Ok(());
        }
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        let (tx3, ty3) = self.translate(x3, y3);
        self.ctx.begin_path();
        self.ctx.move_to(tx1, ty1);
        self.ctx.line_to(tx2, ty2);
        self.ctx.line_to(tx3, ty3);
        self.ctx.close_path();
        self.ctx.set_fill_style_str(&color_to_css(color));
        self.ctx.fill();
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: gradients
    // -------------------------------------------------------------------

    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let fw = w as f64;
        let fh = h as f64;

        match gradient {
            GradientStyle::Vertical { top, bottom } => {
                let grad = self.ctx.create_linear_gradient(tx, ty, tx, ty + fh);
                grad.add_color_stop(0.0, &color_to_css(*top))
                    .map_err(js_err)?;
                grad.add_color_stop(1.0, &color_to_css(*bottom))
                    .map_err(js_err)?;
                self.ctx.set_fill_style_canvas_gradient(&grad);
                self.ctx.fill_rect(tx, ty, fw, fh);
            },
            GradientStyle::Horizontal { left, right } => {
                let grad = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                grad.add_color_stop(0.0, &color_to_css(*left))
                    .map_err(js_err)?;
                grad.add_color_stop(1.0, &color_to_css(*right))
                    .map_err(js_err)?;
                self.ctx.set_fill_style_canvas_gradient(&grad);
                self.ctx.fill_rect(tx, ty, fw, fh);
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                // Approximate 4-corner gradient with two overlapping linear
                // gradients using globalAlpha blending.
                let prev_alpha = self.ctx.global_alpha();

                // First pass: vertical gradient (top_left → bottom_left).
                let grad_v = self.ctx.create_linear_gradient(tx, ty, tx, ty + fh);
                grad_v
                    .add_color_stop(0.0, &color_to_css(*top_left))
                    .map_err(js_err)?;
                grad_v
                    .add_color_stop(1.0, &color_to_css(*bottom_left))
                    .map_err(js_err)?;
                self.ctx.set_fill_style_canvas_gradient(&grad_v);
                self.ctx.fill_rect(tx, ty, fw, fh);

                // Second pass: horizontal gradient with half alpha for blending.
                self.ctx.set_global_alpha(0.5);
                let grad_h = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                grad_h
                    .add_color_stop(0.0, &color_to_css(*top_left))
                    .map_err(js_err)?;
                grad_h
                    .add_color_stop(1.0, &color_to_css(*top_right))
                    .map_err(js_err)?;
                self.ctx.set_fill_style_canvas_gradient(&grad_h);
                self.ctx.fill_rect(tx, ty, fw, fh);

                self.ctx.set_global_alpha(prev_alpha);

                // Note: This is an approximation. True bilinear interpolation
                // would require per-pixel blending, but this is visually
                // close enough for the 480x272 resolution.
                let _ = bottom_right;
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
        if w == 0 || h == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        let fw = w as f64;
        let fh = h as f64;
        let r = f64::from(radius).min(fw / 2.0).min(fh / 2.0);

        // Build rounded rect path.
        self.ctx.begin_path();
        self.ctx.move_to(tx + r, ty);
        self.ctx.line_to(tx + fw - r, ty);
        self.ctx
            .arc_to(tx + fw, ty, tx + fw, ty + r, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx + fw, ty + fh - r);
        self.ctx
            .arc_to(tx + fw, ty + fh, tx + fw - r, ty + fh, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx + r, ty + fh);
        self.ctx
            .arc_to(tx, ty + fh, tx, ty + fh - r, r)
            .map_err(js_err)?;
        self.ctx.line_to(tx, ty + r);
        self.ctx.arc_to(tx, ty, tx + r, ty, r).map_err(js_err)?;
        self.ctx.close_path();

        // Create gradient fill.
        let grad = match gradient {
            GradientStyle::Vertical { top, bottom } => {
                let g = self.ctx.create_linear_gradient(tx, ty, tx, ty + fh);
                g.add_color_stop(0.0, &color_to_css(*top)).map_err(js_err)?;
                g.add_color_stop(1.0, &color_to_css(*bottom))
                    .map_err(js_err)?;
                g
            },
            GradientStyle::Horizontal { left, right } => {
                let g = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                g.add_color_stop(0.0, &color_to_css(*left))
                    .map_err(js_err)?;
                g.add_color_stop(1.0, &color_to_css(*right))
                    .map_err(js_err)?;
                g
            },
            _ => {
                // Four-corner: fallback to primary color.
                let c = gradient.primary_color();
                self.ctx.set_fill_style_str(&color_to_css(c));
                self.ctx.fill();
                return Ok(());
            },
        };
        self.ctx.set_fill_style_canvas_gradient(&grad);
        self.ctx.fill();
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: alpha
    // -------------------------------------------------------------------

    fn fill_rect_alpha(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        alpha: u8,
    ) -> Result<()> {
        let c = Color::rgba(color.r, color.g, color.b, alpha);
        self.fill_rect(x, y, w, h, c)
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(0, 0, self.width, self.height, Color::rgba(0, 0, 0, alpha))
    }

    // -------------------------------------------------------------------
    // Extended: text
    // -------------------------------------------------------------------

    fn measure_text_height(&self, font_size: u16) -> u32 {
        (f64::from(font_size) * 1.2).ceil() as u32
    }

    fn font_ascent(&self, font_size: u16) -> u32 {
        (f64::from(font_size) * 0.85).ceil() as u32
    }

    fn text_metrics(&self, text: &str, font_size: u16) -> TextMetrics {
        TextMetrics {
            width: self.measure_text(text, font_size),
            height: self.measure_text_height(font_size),
            ascent: self.font_ascent(font_size),
        }
    }

    // -------------------------------------------------------------------
    // Extended: texture operations
    // -------------------------------------------------------------------

    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        // Draw the base texture.
        self.blit(tex, x, y, w, h)?;

        // Apply tint by drawing a colored rectangle with multiply composite.
        let (tx, ty) = self.translate(x, y);
        let prev_op = self
            .ctx
            .global_composite_operation()
            .unwrap_or_else(|_| "source-over".to_string());
        let _ = self.ctx.set_global_composite_operation("multiply");
        self.ctx.set_fill_style_str(&color_to_css(tint));
        self.ctx.fill_rect(tx, ty, w as f64, h as f64);
        let _ = self.ctx.set_global_composite_operation(&prev_op);
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: clip & translate stacks
    // -------------------------------------------------------------------

    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tx, ty) = self.translate(x, y);

        // Intersect with current clip if any.
        let clipped = if let Some(cur) = self.clip_stack.last() {
            let cur_tx = cur.x as f64;
            let cur_ty = cur.y as f64;
            let cur_r = cur_tx + cur.w as f64;
            let cur_b = cur_ty + cur.h as f64;
            let new_r = tx + w as f64;
            let new_b = ty + h as f64;
            let ix = tx.max(cur_tx);
            let iy = ty.max(cur_ty);
            let iw = (new_r.min(cur_r) - ix).max(0.0);
            let ih = (new_b.min(cur_b) - iy).max(0.0);
            ClipRect {
                x: ix as i32,
                y: iy as i32,
                w: iw as u32,
                h: ih as u32,
            }
        } else {
            ClipRect {
                x: tx as i32,
                y: ty as i32,
                w,
                h,
            }
        };

        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(
            clipped.x as f64,
            clipped.y as f64,
            clipped.w as f64,
            clipped.h as f64,
        );
        self.ctx.clip();
        self.clip_stack.push(clipped);
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.clip_stack.pop();
        self.ctx.restore();
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.clip_stack.last().map(|c| (c.x, c.y, c.w, c.h))
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

    fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.push_translate(x, y)?;
        self.push_clip_rect(0, 0, w, h)?;
        Ok(())
    }

    fn pop_region(&mut self) -> Result<()> {
        self.pop_clip_rect()?;
        self.pop_translate()?;
        Ok(())
    }
}
