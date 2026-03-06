//! `SdiBackend` implementation using the Canvas 2D API.

use std::collections::{HashMap, VecDeque};

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

use oasis_types::backend::{
    Color, GradientStyle, SdiBackend, SdiCore, TextMetrics, TextureId, texture_not_found,
    validate_rgba_data,
};
use oasis_types::error::{OasisError, Result};

use crate::font;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn js_err<E: std::fmt::Debug>(e: E) -> OasisError {
    OasisError::Backend(format!("{e:?}"))
}

fn get_2d_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d> {
    canvas
        .get_context("2d")
        .map_err(js_err)?
        .ok_or_else(|| OasisError::Backend("no 2d context".into()))?
        .dyn_into::<CanvasRenderingContext2d>()
        .map_err(js_err)
}

/// Return cached CSS color string, allocating only on first use per color.
fn cached_css_color(cache: &mut HashMap<u32, String>, c: Color) -> &str {
    let key = (c.r as u32) << 24 | (c.g as u32) << 16 | (c.b as u32) << 8 | c.a as u32;
    cache.entry(key).or_insert_with(|| {
        if c.a == 255 {
            format!("rgb({},{},{})", c.r, c.g, c.b)
        } else {
            format!("rgba({},{},{},{})", c.r, c.g, c.b, f64::from(c.a) / 255.0)
        }
    })
}

// ---------------------------------------------------------------------------
// Texture storage
// ---------------------------------------------------------------------------

struct TextureData {
    canvas: HtmlCanvasElement,
}

// ---------------------------------------------------------------------------
// Glyph cache key
// ---------------------------------------------------------------------------

/// Packs `(char, font_size, rgba, bold, italic)` into a `u64` for hashing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey(u64);

impl GlyphCacheKey {
    const fn new(ch: char, font_size: u16, color: Color, bold: bool, italic: bool) -> Self {
        // char: 21 bits, font_size: 16 bits, rgba: 25 bits (r5 g5 b5 a8 + 2 flags)
        // Total ≤ 64 bits.
        let c = ch as u64 & 0x1F_FFFF; // 21 bits
        let fs = (font_size as u64) & 0xFFFF; // 16 bits
        let r5 = (color.r as u64 >> 3) & 0x1F; // 5 bits
        let g5 = (color.g as u64 >> 3) & 0x1F; // 5 bits
        let b5 = (color.b as u64 >> 3) & 0x1F; // 5 bits
        let a = color.a as u64; // 8 bits
        let flags = (bold as u64) | ((italic as u64) << 1); // 2 bits
        Self(c | (fs << 21) | (r5 << 37) | (g5 << 42) | (b5 << 47) | (a << 52) | (flags << 60))
    }
}

// ---------------------------------------------------------------------------
// Gradient cache key
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GradientCacheKey {
    kind: u8,
    x: i32,
    y: i32,
    extent: u32,
    color_a: u32,
    color_b: u32,
}

impl GradientCacheKey {
    fn pack_color(c: Color) -> u32 {
        (c.r as u32) << 24 | (c.g as u32) << 16 | (c.b as u32) << 8 | c.a as u32
    }
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

/// Maximum number of cached glyphs before LRU eviction kicks in.
const MAX_GLYPH_CACHE_SIZE: usize = 2048;

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
    color_cache: HashMap<u32, String>,
    glyph_cache: HashMap<GlyphCacheKey, HtmlCanvasElement>,
    /// Insertion-order queue for LRU eviction of glyph cache entries.
    glyph_lru: VecDeque<GlyphCacheKey>,
    gradient_cache: HashMap<GradientCacheKey, web_sys::CanvasGradient>,
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
            color_cache: HashMap::new(),
            glyph_cache: HashMap::new(),
            glyph_lru: VecDeque::new(),
            gradient_cache: HashMap::new(),
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

    /// Create an offscreen `<canvas>` for texture operations.
    fn make_offscreen(&self, w: u32, h: u32) -> Result<HtmlCanvasElement> {
        let doc = web_sys::window()
            .ok_or_else(|| OasisError::Backend("no window".into()))?
            .document()
            .ok_or_else(|| OasisError::Backend("no document".into()))?;
        let el = doc.create_element("canvas").map_err(js_err)?;
        let c = el.dyn_into::<HtmlCanvasElement>().map_err(js_err)?;
        c.set_width(w);
        c.set_height(h);
        Ok(c)
    }

    /// Pre-render RGBA data onto an offscreen canvas (used at load_texture time).
    fn rgba_to_offscreen(&self, width: u32, height: u32, rgba: &[u8]) -> Result<HtmlCanvasElement> {
        let offscreen = self.make_offscreen(width, height)?;
        let off_ctx = get_2d_context(&offscreen)?;
        let image_data =
            ImageData::new_with_u8_clamped_array_and_sh(wasm_bindgen::Clamped(rgba), width, height)
                .map_err(js_err)?;
        off_ctx
            .put_image_data(&image_data, 0.0, 0.0)
            .map_err(js_err)?;
        Ok(offscreen)
    }

    /// Render a single glyph character to an offscreen canvas.
    fn render_glyph_to_canvas(
        &self,
        ch: char,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<HtmlCanvasElement> {
        let fs = font_size.max(1) as i32;
        // Canvas size: character advance width + italic extra + bold extra.
        let advance = font::glyph_advance_scaled(ch, font_size);
        let italic_extra = if italic { fs as u32 / 4 } else { 0 };
        let bold_extra = if bold { 1 } else { 0 };
        let cw = (advance + italic_extra + bold_extra).max(1);
        let ch_height = fs as u32;

        let offscreen = self.make_offscreen(cw, ch_height)?;
        let off_ctx = get_2d_context(&offscreen)?;

        let css = if color.a == 255 {
            format!("rgb({},{},{})", color.r, color.g, color.b)
        } else {
            format!(
                "rgba({},{},{},{})",
                color.r,
                color.g,
                color.b,
                f64::from(color.a) / 255.0
            )
        };
        off_ctx.set_fill_style_str(&css);

        let glyph = font::glyph(ch);
        let (left_pad, _) = font::glyph_metrics(ch);
        let left_pad = left_pad as i32;

        for row in 0..8i32 {
            let bits = glyph[row as usize];
            if bits == 0 {
                continue;
            }
            let oy0 = row * fs / 8;
            let oy1 = (row + 1) * fs / 8;
            let rh = (oy1 - oy0).max(1);
            let italic_offset = if italic { (7 - row) * fs / 32 } else { 0 };
            for col in 0..8i32 {
                if bits & (0x80 >> col) != 0 {
                    let src_col = col - left_pad;
                    let ox0 = src_col * fs / 8;
                    let ox1 = (src_col + 1) * fs / 8;
                    let rw = (ox1 - ox0).max(1);
                    let px = ox0 + italic_offset;
                    let py = oy0;
                    off_ctx.fill_rect(px as f64, py as f64, rw as f64, rh as f64);
                    if bold {
                        off_ctx.fill_rect((px + 1) as f64, py as f64, 1.0, rh as f64);
                    }
                }
            }
        }
        Ok(offscreen)
    }

    /// Shared glyph-rendering implementation for both `draw_text` and
    /// `draw_text_styled`.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_impl(
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
        let mut cx = tx as i32;

        for ch in text.chars() {
            let key = GlyphCacheKey::new(ch, font_size, color, bold, italic);
            if self.glyph_cache.contains_key(&key) {
                // Promote to back of LRU on cache hit.
                if let Some(pos) = self.glyph_lru.iter().position(|k| k == &key) {
                    self.glyph_lru.remove(pos);
                }
                self.glyph_lru.push_back(key);
            } else {
                // Evict least-recently-used entries when cache is full.
                while self.glyph_cache.len() >= MAX_GLYPH_CACHE_SIZE {
                    if let Some(old_key) = self.glyph_lru.pop_front() {
                        self.glyph_cache.remove(&old_key);
                    } else {
                        break;
                    }
                }
                let canvas = self.render_glyph_to_canvas(ch, font_size, color, bold, italic)?;
                self.glyph_cache.insert(key, canvas);
                self.glyph_lru.push_back(key);
            }
            let glyph_canvas = &self.glyph_cache[&key];
            self.ctx
                .draw_image_with_html_canvas_element(glyph_canvas, cx as f64, ty)
                .map_err(js_err)?;
            cx += font::glyph_advance_scaled(ch, font_size) as i32;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SdiCore — 13 required methods
// ---------------------------------------------------------------------------

impl SdiCore for WasmBackend {
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.canvas.set_width(width);
        self.canvas.set_height(height);
        self.ctx.set_image_smoothing_enabled(false);
        self.glyph_cache.clear();
        self.glyph_lru.clear();
        self.color_cache.clear();
        self.gradient_cache.clear();
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
        self.ctx
            .fill_rect(0.0, 0.0, self.width as f64, self.height as f64);
        self.gradient_cache.clear();
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
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
        self.draw_text_impl(text, x, y, font_size, color, false, false)
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let td = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        let (tx, ty) = self.translate(x, y);
        self.ctx
            .draw_image_with_html_canvas_element_and_dw_and_dh(
                &td.canvas, tx, ty, w as f64, h as f64,
            )
            .map_err(js_err)?;
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;
        let canvas = self.rgba_to_offscreen(width, height, rgba_data)?;
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, TextureData { canvas });
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
        self.clip_stack.push(ClipRect {
            x: tx as i32,
            y: ty as i32,
            w,
            h,
        });
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.clip_stack.pop();
        self.ctx.restore();
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_types::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        // getImageData throws SecurityError if the canvas is tainted by a
        // cross-origin video frame.  Return opaque black in that case.
        match self
            .ctx
            .get_image_data(x as f64, y as f64, w as f64, h as f64)
        {
            Ok(data) => Ok(data.data().to_vec()),
            Err(_) => Ok([0, 0, 0, 255].repeat((w * h) as usize)),
        }
    }

    fn shutdown(&mut self) -> Result<()> {
        self.textures.clear();
        self.glyph_cache.clear();
        self.glyph_lru.clear();
        self.color_cache.clear();
        self.gradient_cache.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SdiBackend — override methods
// ---------------------------------------------------------------------------

impl SdiBackend for WasmBackend {
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
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
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
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
                let cache_key = GradientCacheKey {
                    kind: 0,
                    x: tx as i32,
                    y: ty as i32,
                    extent: h,
                    color_a: GradientCacheKey::pack_color(*top),
                    color_b: GradientCacheKey::pack_color(*bottom),
                };
                if !self.gradient_cache.contains_key(&cache_key) {
                    let grad = self.ctx.create_linear_gradient(tx, ty, tx, ty + fh);
                    let css_top = cached_css_color(&mut self.color_cache, *top).to_owned();
                    let css_bot = cached_css_color(&mut self.color_cache, *bottom).to_owned();
                    grad.add_color_stop(0.0, &css_top).map_err(js_err)?;
                    grad.add_color_stop(1.0, &css_bot).map_err(js_err)?;
                    self.gradient_cache.insert(cache_key, grad);
                }
                let grad = &self.gradient_cache[&cache_key];
                self.ctx.set_fill_style_canvas_gradient(grad);
                self.ctx.fill_rect(tx, ty, fw, fh);
            },
            GradientStyle::Horizontal { left, right } => {
                let cache_key = GradientCacheKey {
                    kind: 1,
                    x: tx as i32,
                    y: ty as i32,
                    extent: w,
                    color_a: GradientCacheKey::pack_color(*left),
                    color_b: GradientCacheKey::pack_color(*right),
                };
                if !self.gradient_cache.contains_key(&cache_key) {
                    let grad = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                    let css_left = cached_css_color(&mut self.color_cache, *left).to_owned();
                    let css_right = cached_css_color(&mut self.color_cache, *right).to_owned();
                    grad.add_color_stop(0.0, &css_left).map_err(js_err)?;
                    grad.add_color_stop(1.0, &css_right).map_err(js_err)?;
                    self.gradient_cache.insert(cache_key, grad);
                }
                let grad = &self.gradient_cache[&cache_key];
                self.ctx.set_fill_style_canvas_gradient(grad);
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
                let css_tl = cached_css_color(&mut self.color_cache, *top_left).to_owned();
                let css_bl = cached_css_color(&mut self.color_cache, *bottom_left).to_owned();
                let grad_v = self.ctx.create_linear_gradient(tx, ty, tx, ty + fh);
                grad_v.add_color_stop(0.0, &css_tl).map_err(js_err)?;
                grad_v.add_color_stop(1.0, &css_bl).map_err(js_err)?;
                self.ctx.set_fill_style_canvas_gradient(&grad_v);
                self.ctx.fill_rect(tx, ty, fw, fh);

                // Second pass: horizontal gradient with half alpha for blending.
                self.ctx.set_global_alpha(0.5);
                let css_tl2 = cached_css_color(&mut self.color_cache, *top_left).to_owned();
                let css_tr = cached_css_color(&mut self.color_cache, *top_right).to_owned();
                let grad_h = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                grad_h.add_color_stop(0.0, &css_tl2).map_err(js_err)?;
                grad_h.add_color_stop(1.0, &css_tr).map_err(js_err)?;
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
                let css_top = cached_css_color(&mut self.color_cache, *top).to_owned();
                let css_bot = cached_css_color(&mut self.color_cache, *bottom).to_owned();
                g.add_color_stop(0.0, &css_top).map_err(js_err)?;
                g.add_color_stop(1.0, &css_bot).map_err(js_err)?;
                g
            },
            GradientStyle::Horizontal { left, right } => {
                let g = self.ctx.create_linear_gradient(tx, ty, tx + fw, ty);
                let css_left = cached_css_color(&mut self.color_cache, *left).to_owned();
                let css_right = cached_css_color(&mut self.color_cache, *right).to_owned();
                g.add_color_stop(0.0, &css_left).map_err(js_err)?;
                g.add_color_stop(1.0, &css_right).map_err(js_err)?;
                g
            },
            _ => {
                // Four-corner: fallback to primary color.
                let c = gradient.primary_color();
                let css = cached_css_color(&mut self.color_cache, c);
                self.ctx.set_fill_style_str(css);
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

    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(0, 0, self.width, self.height, Color::rgba(0, 0, 0, alpha))
    }

    // -------------------------------------------------------------------
    // Extended: vector graphics primitives
    // -------------------------------------------------------------------

    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 || color.a == 0 {
            return Ok(());
        }
        self.ctx.begin_path();
        let (tx0, ty0) = self.translate(points[0].0, points[0].1);
        self.ctx.move_to(tx0, ty0);
        for &(x, y) in &points[1..] {
            let (tx, ty) = self.translate(x, y);
            self.ctx.line_to(tx, ty);
        }
        self.ctx.close_path();
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
        self.ctx.fill();
        Ok(())
    }

    fn stroke_polygon(&mut self, points: &[(i32, i32)], width: u16, color: Color) -> Result<()> {
        if points.len() < 2 || color.a == 0 {
            return Ok(());
        }
        self.ctx.begin_path();
        let (tx0, ty0) = self.translate(points[0].0, points[0].1);
        self.ctx.move_to(tx0, ty0);
        for &(x, y) in &points[1..] {
            let (tx, ty) = self.translate(x, y);
            self.ctx.line_to(tx, ty);
        }
        self.ctx.close_path();
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
        self.ctx.set_line_width(f64::from(width));
        self.ctx.stroke();
        Ok(())
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
        if color.a == 0 || radius == 0 {
            return Ok(());
        }
        let (tcx, tcy) = self.translate(cx, cy);
        self.ctx.begin_path();
        self.ctx.move_to(tcx, tcy);
        self.ctx
            .arc(
                tcx,
                tcy,
                f64::from(radius),
                f64::from(start_angle),
                f64::from(end_angle),
            )
            .map_err(js_err)?;
        self.ctx.close_path();
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_fill_style_str(css);
        self.ctx.fill();
        Ok(())
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
        if color.a == 0 || radius == 0 {
            return Ok(());
        }
        let (tcx, tcy) = self.translate(cx, cy);
        self.ctx.begin_path();
        self.ctx
            .arc(
                tcx,
                tcy,
                f64::from(radius),
                f64::from(start_angle),
                f64::from(end_angle),
            )
            .map_err(js_err)?;
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
        self.ctx.set_line_width(f64::from(width));
        self.ctx.stroke();
        Ok(())
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
        if color.a == 0 {
            return Ok(());
        }
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        // Canvas 2D has native dash support via setLineDash.
        let dash_array = js_sys::Array::new();
        dash_array.push(&wasm_bindgen::JsValue::from(f64::from(dash)));
        dash_array.push(&wasm_bindgen::JsValue::from(f64::from(gap)));
        self.ctx.set_line_dash(&dash_array).map_err(js_err)?;
        self.ctx.begin_path();
        self.ctx.move_to(tx1, ty1);
        self.ctx.line_to(tx2, ty2);
        let css = cached_css_color(&mut self.color_cache, color);
        self.ctx.set_stroke_style_str(css);
        self.ctx.set_line_width(f64::from(width));
        self.ctx.stroke();
        // Reset dash pattern.
        self.ctx
            .set_line_dash(&js_sys::Array::new())
            .map_err(|e| OasisError::Backend(format!("{e:?}")))?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Extended: text
    // -------------------------------------------------------------------

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
        self.draw_text_impl(text, x, y, font_size, color, bold, italic)
    }

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
        let td = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        self.ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &td.canvas,
                src_x as f64,
                src_y as f64,
                src_w as f64,
                src_h as f64,
                tx,
                ty,
                dst_w as f64,
                dst_h as f64,
            )
            .map_err(js_err)?;
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
        if !flip_h && !flip_v {
            return self.blit(tex, x, y, w, h);
        }

        let td = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex.0))?;

        let (tx, ty) = self.translate(x, y);
        let fw = w as f64;
        let fh = h as f64;

        // Apply flip via scale transform.
        self.ctx.save();
        let sx = if flip_h { -1.0 } else { 1.0 };
        let sy = if flip_v { -1.0 } else { 1.0 };
        let dx = if flip_h { -(tx + fw) } else { tx };
        let dy = if flip_v { -(ty + fh) } else { ty };
        self.ctx.scale(sx, sy).map_err(js_err)?;
        self.ctx
            .draw_image_with_html_canvas_element_and_dw_and_dh(&td.canvas, dx, dy, fw, fh)
            .map_err(js_err)?;
        self.ctx.restore();
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
        // Draw the base texture.
        self.blit(tex, x, y, w, h)?;

        // Apply tint by drawing a colored rectangle with multiply composite.
        let (tx, ty) = self.translate(x, y);
        let prev_op = self
            .ctx
            .global_composite_operation()
            .unwrap_or_else(|_| "source-over".to_string());
        let _ = self.ctx.set_global_composite_operation("multiply");
        let css = cached_css_color(&mut self.color_cache, tint);
        self.ctx.set_fill_style_str(css);
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

// ---------------------------------------------------------------------------
// Extra public helpers (not part of SdiBackend)
// ---------------------------------------------------------------------------

impl WasmBackend {
    /// Register an existing offscreen `<canvas>` as a texture.
    ///
    /// The caller keeps a clone of the `HtmlCanvasElement` reference so it can
    /// draw into it (e.g. `ctx.drawImage(video, …)`).  When `blit()` runs it
    /// will paint the latest content.
    pub fn register_canvas_as_texture(&mut self, canvas: HtmlCanvasElement) -> TextureId {
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        self.textures.insert(id, TextureData { canvas });
        TextureId(id)
    }
}
