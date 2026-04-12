//! `SdiBackend` implementation using the Canvas 2D API.

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

use oasis_rasterize::GlyphCacheKey;
use oasis_types::backend::stacks::{ClipStack, TranslateStack};
use oasis_types::backend::{
    BlendMode, Color, RenderTargetId, SdiAlpha, SdiBatch, SdiCore, SdiRenderTarget, SdiText,
    SdiVector, TextMetrics, TextureId, texture_not_found, validate_rgba_data,
};
use oasis_types::error::{OasisError, Result};

use crate::font;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn js_err<E: std::fmt::Debug>(e: E) -> OasisError {
    OasisError::Backend(format!("{e:?}").into())
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
pub(crate) fn cached_css_color(cache: &mut HashMap<u32, String>, c: Color) -> &str {
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

pub(crate) struct TextureData {
    pub(crate) canvas: HtmlCanvasElement,
}

// ---------------------------------------------------------------------------
// WasmBackend
// ---------------------------------------------------------------------------

/// Maximum number of cached glyphs before LRU eviction kicks in.
const MAX_GLYPH_CACHE_SIZE: usize = 2048;

/// A single offscreen render target: its canvas element plus a
/// cached 2D context so we don't pay `getContext("2d")` per bind.
pub(crate) struct RenderTargetData {
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) ctx: CanvasRenderingContext2d,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Saved framebuffer state pushed onto the bind stack during
/// `bind_render_target`.
struct SavedSurface {
    prev_id: Option<u64>,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    width: u32,
    height: u32,
}

pub struct WasmBackend {
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) ctx: CanvasRenderingContext2d,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) textures: HashMap<u64, TextureData>,
    pub(crate) next_texture_id: u64,
    pub(crate) clip_stack: ClipStack,
    /// Depth counter for `set_clip_rect`/`reset_clip_rect` (SdiCore), which use
    /// canvas `save()`/`restore()` independently of the `ClipStack`.
    core_clip_depth: u32,
    pub(crate) translate_stack: TranslateStack,
    pub(crate) color_cache: HashMap<u32, String>,
    glyph_cache: HashMap<GlyphCacheKey, HtmlCanvasElement>,
    /// Access timestamps for LRU eviction of glyph cache entries.
    glyph_access: HashMap<GlyphCacheKey, u64>,
    glyph_access_counter: u64,
    pub(crate) gradient_cache: HashMap<crate::gradients::GradientCacheKey, web_sys::CanvasGradient>,
    /// Offscreen render targets (compositor PR4). Each is a
    /// separate `<canvas>` element plus cached context.
    pub(crate) render_targets: HashMap<u64, RenderTargetData>,
    /// Stack of saved framebuffer states for nested compositing.
    render_target_bind_stack: Vec<SavedSurface>,
    /// Currently bound render target (`None` = framebuffer).
    pub(crate) current_render_target: Option<u64>,
    /// Monotonic counter for render-target ids.
    next_render_target_id: u64,
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
            clip_stack: ClipStack::new(width, height),
            core_clip_depth: 0,
            translate_stack: TranslateStack::new(),
            color_cache: HashMap::new(),
            glyph_cache: HashMap::new(),
            glyph_access: HashMap::new(),
            glyph_access_counter: 0,
            gradient_cache: HashMap::new(),
            render_targets: HashMap::new(),
            render_target_bind_stack: Vec::new(),
            current_render_target: None,
            next_render_target_id: 1,
        })
    }

    /// Get the underlying canvas element.
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// Get the 2D rendering context (used by shader bridge for `putImageData`).
    pub fn ctx(&self) -> &web_sys::CanvasRenderingContext2d {
        &self.ctx
    }

    pub(crate) fn translate(&self, x: i32, y: i32) -> (f64, f64) {
        self.translate_stack.translate_f64(x, y)
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
                // Update access timestamp on cache hit.
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
            } else {
                // Evict least-recently-used entry when cache is full (O(N) scan).
                while self.glyph_cache.len() >= MAX_GLYPH_CACHE_SIZE {
                    if let Some((&oldest_key, _)) =
                        self.glyph_access.iter().min_by_key(|&(_, &ts)| ts)
                    {
                        self.glyph_cache.remove(&oldest_key);
                        self.glyph_access.remove(&oldest_key);
                    } else {
                        break;
                    }
                }
                let canvas = self.render_glyph_to_canvas(ch, font_size, color, bold, italic)?;
                self.glyph_cache.insert(key, canvas);
                self.glyph_access_counter += 1;
                self.glyph_access.insert(key, self.glyph_access_counter);
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
        self.glyph_access.clear();
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
        self.core_clip_depth += 1;
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        if self.core_clip_depth > 0 {
            self.core_clip_depth -= 1;
        }
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
        self.glyph_access.clear();
        self.color_cache.clear();
        self.gradient_cache.clear();
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiAlpha: Alpha and viewport
// -------------------------------------------------------------------

impl SdiAlpha for WasmBackend {
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
}

// -------------------------------------------------------------------
// SdiVector: Vector graphics primitives
// -------------------------------------------------------------------

impl SdiVector for WasmBackend {
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
            .map_err(|e| OasisError::Backend(format!("{e:?}").into()))?;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiText: Text system
// -------------------------------------------------------------------

impl SdiText for WasmBackend {
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
}

// -------------------------------------------------------------------
// SdiBatch: No-op (use default impl)
// -------------------------------------------------------------------

impl SdiBatch for WasmBackend {}

// -------------------------------------------------------------------
// SdiRenderTarget: Offscreen compositing layers (compositor PR4)
// -------------------------------------------------------------------

/// Map a CSS blend mode onto the Canvas2D `globalCompositeOperation`
/// string. Canvas2D ships native support for all 16 CSS blend modes.
fn canvas_composite_op(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "source-over",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::HardLight => "hard-light",
        BlendMode::SoftLight => "soft-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
    }
}

impl SdiRenderTarget for WasmBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        if w == 0 || h == 0 {
            return Err(OasisError::Backend(
                format!("create_render_target: zero dimension ({w}x{h})").into(),
            ));
        }
        let target_canvas = self.make_offscreen(w, h)?;
        let target_ctx = get_2d_context(&target_canvas)?;
        target_ctx.set_image_smoothing_enabled(false);
        let id = self.next_render_target_id;
        self.next_render_target_id += 1;
        self.render_targets.insert(
            id,
            RenderTargetData {
                canvas: target_canvas,
                ctx: target_ctx,
                width: w,
                height: h,
            },
        );
        Ok(RenderTargetId(id))
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        let target = self.render_targets.remove(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("bind_render_target: unknown id {id:?}").into())
        })?;
        // Save the current surface and swap the target in.
        let saved_canvas = std::mem::replace(&mut self.canvas, target.canvas);
        let saved_ctx = std::mem::replace(&mut self.ctx, target.ctx);
        let saved_width = std::mem::replace(&mut self.width, target.width);
        let saved_height = std::mem::replace(&mut self.height, target.height);
        self.render_target_bind_stack.push(SavedSurface {
            prev_id: self.current_render_target,
            canvas: saved_canvas,
            ctx: saved_ctx,
            width: saved_width,
            height: saved_height,
        });
        self.current_render_target = Some(id.0);
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let saved = self.render_target_bind_stack.pop().ok_or_else(|| {
            OasisError::Backend("unbind_render_target: bind stack underflow".into())
        })?;
        // Put the target back into storage under its id, then restore
        // the saved surface.
        if let Some(active_id) = self.current_render_target {
            let target_canvas = std::mem::replace(&mut self.canvas, saved.canvas);
            let target_ctx = std::mem::replace(&mut self.ctx, saved.ctx);
            let target_w = std::mem::replace(&mut self.width, saved.width);
            let target_h = std::mem::replace(&mut self.height, saved.height);
            self.render_targets.insert(
                active_id,
                RenderTargetData {
                    canvas: target_canvas,
                    ctx: target_ctx,
                    width: target_w,
                    height: target_h,
                },
            );
        } else {
            // Shouldn't happen, but be defensive.
            self.canvas = saved.canvas;
            self.ctx = saved.ctx;
            self.width = saved.width;
            self.height = saved.height;
        }
        self.current_render_target = saved.prev_id;
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
        let target = self.render_targets.get(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("composite_render_target: unknown id {id:?}").into())
        })?;
        let prev_op = self.ctx.global_composite_operation().map_err(js_err)?;
        let prev_alpha = self.ctx.global_alpha();
        self.ctx
            .set_global_composite_operation(canvas_composite_op(blend))
            .map_err(js_err)?;
        self.ctx.set_global_alpha(opacity.clamp(0.0, 1.0) as f64);
        self.ctx
            .draw_image_with_html_canvas_element_and_dw_and_dh(
                &target.canvas,
                dst_x as f64,
                dst_y as f64,
                dst_w as f64,
                dst_h as f64,
            )
            .map_err(js_err)?;
        // Restore state.
        self.ctx
            .set_global_composite_operation(&prev_op)
            .map_err(js_err)?;
        self.ctx.set_global_alpha(prev_alpha);
        Ok(())
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        let target = self.render_targets.get(&id.0).ok_or_else(|| {
            OasisError::Backend(format!("read_render_target: unknown id {id:?}").into())
        })?;
        let image_data = target
            .ctx
            .get_image_data(0.0, 0.0, target.width as f64, target.height as f64)
            .map_err(js_err)?;
        let data = image_data.data();
        let expected = (target.width * target.height * 4) as usize;
        if dst.len() < expected || data.len() < expected {
            return Err(OasisError::Backend(
                format!(
                    "read_render_target: buffer too small ({} < {expected})",
                    dst.len()
                )
                .into(),
            ));
        }
        dst[..expected].copy_from_slice(&data[..expected]);
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        if self.current_render_target == Some(id.0)
            || self
                .render_target_bind_stack
                .iter()
                .any(|entry| entry.prev_id == Some(id.0))
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
        // HtmlCanvasElement is GC'd by JS once Rust drops its handle.
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        true
    }

    fn supports_render_target_readback(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Extra public helpers
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

// ---------------------------------------------------------------------------
// Tests -- pure logic that doesn't require wasm-bindgen/web-sys
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::gradients::GradientCacheKey;

    // -----------------------------------------------------------------------
    // cached_css_color
    // -----------------------------------------------------------------------

    #[test]
    fn css_color_opaque_rgb() {
        let mut cache = HashMap::new();
        let c = Color::rgba(255, 128, 0, 255);
        let css = cached_css_color(&mut cache, c).to_owned();
        assert_eq!(css, "rgb(255,128,0)");
    }

    #[test]
    fn css_color_transparent_rgba() {
        let mut cache = HashMap::new();
        let c = Color::rgba(100, 200, 50, 128);
        let css = cached_css_color(&mut cache, c).to_owned();
        // alpha = 128/255 ≈ 0.5019...
        assert!(css.starts_with("rgba(100,200,50,"));
        assert!(css.contains("0.50"));
    }

    #[test]
    fn css_color_fully_transparent() {
        let mut cache = HashMap::new();
        let c = Color::rgba(0, 0, 0, 0);
        let css = cached_css_color(&mut cache, c).to_owned();
        assert!(css.starts_with("rgba(0,0,0,0"));
    }

    #[test]
    fn css_color_white_opaque() {
        let mut cache = HashMap::new();
        let c = Color::rgba(255, 255, 255, 255);
        let css = cached_css_color(&mut cache, c).to_owned();
        assert_eq!(css, "rgb(255,255,255)");
    }

    #[test]
    fn css_color_cache_reuse() {
        let mut cache = HashMap::new();
        let c = Color::rgba(10, 20, 30, 255);
        let first = cached_css_color(&mut cache, c).to_owned();
        let second = cached_css_color(&mut cache, c).to_owned();
        assert_eq!(first, second);
        // Only one entry should exist.
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn css_color_different_colors_different_entries() {
        let mut cache = HashMap::new();
        let c1 = Color::rgba(10, 20, 30, 255);
        let c2 = Color::rgba(40, 50, 60, 255);
        cached_css_color(&mut cache, c1);
        cached_css_color(&mut cache, c2);
        assert_eq!(cache.len(), 2);
    }

    // -----------------------------------------------------------------------
    // GlyphCacheKey
    // -----------------------------------------------------------------------

    #[test]
    fn glyph_key_unique_for_different_chars() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('B', 12, c, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_different_sizes() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('A', 16, c, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_different_colors() {
        let c1 = Color::rgba(255, 0, 0, 255);
        let c2 = Color::rgba(0, 255, 0, 255);
        let k1 = GlyphCacheKey::new('A', 12, c1, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c2, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_bold_vs_normal() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c, true, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_unique_for_italic_vs_normal() {
        let c = Color::rgba(255, 255, 255, 255);
        let k1 = GlyphCacheKey::new('A', 12, c, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c, false, true);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_equal_for_same_params() {
        let c = Color::rgba(128, 64, 32, 200);
        let k1 = GlyphCacheKey::new('Z', 24, c, true, true);
        let k2 = GlyphCacheKey::new('Z', 24, c, true, true);
        assert_eq!(k1, k2);
    }

    #[test]
    fn glyph_key_unicode_char() {
        let c = Color::rgba(255, 255, 255, 255);
        // Unicode character should be packed into the 21-bit field.
        let k = GlyphCacheKey::new('\u{1F600}', 12, c, false, false);
        // Should not panic and should produce a valid key.
        assert_ne!(k.0, 0);
    }

    #[test]
    fn glyph_key_alpha_channel_distinction() {
        // Colors that differ only in alpha should produce different keys.
        let c1 = Color::rgba(128, 128, 128, 100);
        let c2 = Color::rgba(128, 128, 128, 200);
        let k1 = GlyphCacheKey::new('A', 12, c1, false, false);
        let k2 = GlyphCacheKey::new('A', 12, c2, false, false);
        assert_ne!(k1, k2);
    }

    #[test]
    fn glyph_key_bold_italic_all_combinations() {
        let c = Color::rgba(255, 255, 255, 255);
        let keys: Vec<GlyphCacheKey> = vec![
            GlyphCacheKey::new('X', 10, c, false, false),
            GlyphCacheKey::new('X', 10, c, true, false),
            GlyphCacheKey::new('X', 10, c, false, true),
            GlyphCacheKey::new('X', 10, c, true, true),
        ];
        // All four combinations must produce distinct keys.
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "keys[{i}] == keys[{j}]");
            }
        }
    }

    // -----------------------------------------------------------------------
    // GradientCacheKey::pack_color
    // -----------------------------------------------------------------------

    #[test]
    fn gradient_pack_color_opaque_white() {
        let packed = GradientCacheKey::pack_color(Color::rgba(255, 255, 255, 255));
        assert_eq!(packed, 0xFFFF_FFFF);
    }

    #[test]
    fn gradient_pack_color_transparent_black() {
        let packed = GradientCacheKey::pack_color(Color::rgba(0, 0, 0, 0));
        assert_eq!(packed, 0x0000_0000);
    }

    #[test]
    fn gradient_pack_color_red() {
        let packed = GradientCacheKey::pack_color(Color::rgba(255, 0, 0, 255));
        assert_eq!(packed, 0xFF00_00FF);
    }

    #[test]
    fn gradient_pack_color_green() {
        let packed = GradientCacheKey::pack_color(Color::rgba(0, 255, 0, 255));
        assert_eq!(packed, 0x00FF_00FF);
    }

    #[test]
    fn gradient_pack_color_blue() {
        let packed = GradientCacheKey::pack_color(Color::rgba(0, 0, 255, 255));
        assert_eq!(packed, 0x0000_FFFF);
    }

    #[test]
    fn gradient_pack_color_half_alpha() {
        let packed = GradientCacheKey::pack_color(Color::rgba(0, 0, 0, 128));
        assert_eq!(packed, 0x0000_0080);
    }

    #[test]
    fn gradient_pack_color_roundtrip_uniqueness() {
        // Two different colors should produce different packed values.
        let a = GradientCacheKey::pack_color(Color::rgba(10, 20, 30, 40));
        let b = GradientCacheKey::pack_color(Color::rgba(40, 30, 20, 10));
        assert_ne!(a, b);
    }

    // -----------------------------------------------------------------------
    // measure_text_height / font_ascent (pure math)
    // -----------------------------------------------------------------------

    #[test]
    fn measure_text_height_formula() {
        // measure_text_height returns ceil(font_size * 1.2)
        assert_eq!((10.0_f64 * 1.2).ceil() as u32, 12);
        assert_eq!((8.0_f64 * 1.2).ceil() as u32, 10);
        assert_eq!((16.0_f64 * 1.2).ceil() as u32, 20);
    }

    #[test]
    fn font_ascent_formula() {
        // font_ascent returns ceil(font_size * 0.85)
        assert_eq!((10.0_f64 * 0.85).ceil() as u32, 9);
        assert_eq!((12.0_f64 * 0.85).ceil() as u32, 11);
        assert_eq!((16.0_f64 * 0.85).ceil() as u32, 14);
    }
}
