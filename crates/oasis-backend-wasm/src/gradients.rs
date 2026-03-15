//! `SdiGradients` implementation for the WASM backend.

use oasis_types::backend::{Color, GradientStyle, SdiGradients};
use oasis_types::error::Result;

use crate::renderer::{WasmBackend, cached_css_color, js_err};

// ---------------------------------------------------------------------------
// Gradient cache key
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GradientCacheKey {
    pub(crate) kind: u8,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) extent: u32,
    pub(crate) color_a: u32,
    pub(crate) color_b: u32,
}

impl GradientCacheKey {
    pub(crate) fn pack_color(c: Color) -> u32 {
        (c.r as u32) << 24 | (c.g as u32) << 16 | (c.b as u32) << 8 | c.a as u32
    }
}

/// Maximum number of cached canvas gradients before the cache is cleared.
const MAX_GRADIENT_CACHE_SIZE: usize = 256;

// ---------------------------------------------------------------------------
// SdiGradients
// ---------------------------------------------------------------------------

impl SdiGradients for WasmBackend {
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
                    if self.gradient_cache.len() >= MAX_GRADIENT_CACHE_SIZE {
                        self.gradient_cache.clear();
                    }
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
                    if self.gradient_cache.len() >= MAX_GRADIENT_CACHE_SIZE {
                        self.gradient_cache.clear();
                    }
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

                // First pass: vertical gradient (top_left -> bottom_left).
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
}
