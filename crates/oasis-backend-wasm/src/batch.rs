//! `SdiBatch` overrides for the WASM backend.
//!
//! The default implementations in `oasis-types` issue one `fill_rect` /
//! `draw_text` per item, which on WASM means one wasm-bindgen → JS round
//! trip per item. For a 200-rect frame that is 200 boundary crossings
//! plus 200 separate `set_fill_style_str` JS string conversions; the
//! Canvas2D draws themselves are a small fraction of the cost.
//!
//! These overrides collapse a batch into a single round trip:
//!
//! - **`submit_rect_batch`** marshals the slice as one `Float32Array`
//!   view over WASM linear memory (zero-copy) and runs the per-rect
//!   `fillRect` loop inside a small inline-JS helper. Layout is
//!   `[x, y, w, h, r, g, b, a, ...]`, eight floats per rect, with
//!   coordinates already screen-translated and colors in 0..=255.
//!
//! - **`submit_text_batch`** preserves the cached-bitmap-glyph rendering
//!   that desktop SDL uses, so visual output stays byte-identical to the
//!   default. Glyph offscreen canvases are pre-resolved (and the LRU
//!   touched) on the Rust side, then a `js_sys::Array` of canvas
//!   handles plus a `Float32Array` of `[x0, y0, x1, y1, ...]` positions
//!   is passed to JS, which issues all `drawImage` calls in one
//!   round-trip.
//!
//! Both helpers create the `Float32Array` view *immediately* before the
//! JS call and do not allocate between view creation and the call —
//! `Float32Array::view` aliases WASM memory and is invalidated by any
//! intervening `Vec` growth or wasm allocator activity.

use wasm_bindgen::prelude::*;
use web_sys::CanvasRenderingContext2d;

use oasis_rasterize::GlyphCacheKey;
use oasis_types::backend::{BatchRect, BatchText, SdiBatch};
use oasis_types::error::Result;

use crate::font;
use crate::renderer::WasmBackend;

#[wasm_bindgen(inline_js = r#"
// Fill N rectangles with their own colors in a single round trip.
//
// `data` is a Float32Array of [x, y, w, h, r, g, b, a, ...] — eight
// floats per rect, alpha in 0..=255 (matches the rest of the channels
// so the marshal layout is uniform). `n` is the rect count.
export function oasis_submit_rects(ctx, data, n) {
  for (let i = 0; i < n; i++) {
    const o = i * 8;
    const r = data[o + 4] | 0;
    const g = data[o + 5] | 0;
    const b = data[o + 6] | 0;
    const a = data[o + 7] | 0;
    if (a >= 255) {
      ctx.fillStyle = "rgb(" + r + "," + g + "," + b + ")";
    } else {
      ctx.fillStyle = "rgba(" + r + "," + g + "," + b + "," + (a / 255) + ")";
    }
    ctx.fillRect(data[o], data[o + 1], data[o + 2], data[o + 3]);
  }
}

// Draw N pre-rendered glyph canvases at their target positions.
//
// `glyphs` is a JS Array of HTMLCanvasElement (one per glyph instance,
// in draw order). `positions` is a Float32Array of [x0, y0, x1, y1, ...]
// (two floats per glyph). `n` is the glyph count.
export function oasis_submit_glyphs(ctx, glyphs, positions, n) {
  for (let i = 0; i < n; i++) {
    const o = i * 2;
    ctx.drawImage(glyphs[i], positions[o], positions[o + 1]);
  }
}
"#)]
extern "C" {
    fn oasis_submit_rects(ctx: &CanvasRenderingContext2d, data: &js_sys::Float32Array, n: u32);
    fn oasis_submit_glyphs(
        ctx: &CanvasRenderingContext2d,
        glyphs: &js_sys::Array,
        positions: &js_sys::Float32Array,
        n: u32,
    );
}

impl SdiBatch for WasmBackend {
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        if rects.is_empty() {
            return Ok(());
        }

        // Pack [x, y, w, h, r, g, b, a] per rect, screen-translated.
        // Skip zero-area and fully transparent rects to match `fill_rect`.
        let mut data: Vec<f32> = Vec::with_capacity(rects.len() * 8);
        for r in rects {
            if r.w == 0 || r.h == 0 || r.color.a == 0 {
                continue;
            }
            let (tx, ty) = self.translate(r.x, r.y);
            data.extend_from_slice(&[
                tx as f32,
                ty as f32,
                r.w as f32,
                r.h as f32,
                f32::from(r.color.r),
                f32::from(r.color.g),
                f32::from(r.color.b),
                f32::from(r.color.a),
            ]);
        }
        if data.is_empty() {
            return Ok(());
        }
        let n = (data.len() / 8) as u32;
        let ctx = self.ctx();

        // SAFETY: the Float32Array view aliases WASM linear memory backing
        // `data`. We do not allocate or grow any Vec between view creation
        // and the JS call, so the view's pointer stays valid for the
        // duration of the call. `data` is dropped after JS returns.
        unsafe {
            let view = js_sys::Float32Array::view(&data);
            oasis_submit_rects(ctx, &view, n);
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
        // `italic` is intentionally unread: matches `draw_text_impl`'s
        // batched route, which also drops it (faux-italic is rendered
        // through the per-glyph path, not the batched draw list).
        let _ = italic;
        if texts.is_empty() || font_size == 0 {
            return Ok(());
        }

        // Single-pass build: ensure each glyph is in the cache and
        // immediately push its canvas into the JS-side draw list. The
        // JS Array holds its own reference to every canvas it receives,
        // so subsequent LRU evictions from the Rust-side `glyph_cache`
        // are harmless — a previously-pushed canvas stays alive on the
        // JS heap. This avoids a hazard the older two-pass version had,
        // where a batch with more unique glyph keys than
        // `MAX_GLYPH_CACHE_SIZE` could silently drop characters because
        // pass-1 inserts evicted earlier pass-1 inserts before pass 2
        // looked them up.
        //
        // Default-impl bold path: double-strike of the regular
        // (non-bold) glyph at x+1, so the cached glyph variant we need
        // is always (bold=false, italic=false).
        let glyphs = js_sys::Array::new();
        let mut positions: Vec<f32> = Vec::new();
        for t in texts {
            if t.text.is_empty() || t.color.a == 0 {
                continue;
            }
            let (tx, ty) = self.translate(t.x, t.y);
            let mut cx = tx as f32;
            for ch in t.text.chars() {
                self.ensure_glyph(ch, font_size, t.color, false, false)?;
                let key = GlyphCacheKey::new(ch, font_size, t.color, false, false);
                let canvas = self
                    .glyph_canvas(&key)
                    .expect("ensure_glyph just inserted this key");
                glyphs.push(canvas);
                positions.push(cx);
                positions.push(ty as f32);
                if bold {
                    glyphs.push(canvas);
                    positions.push(cx + 1.0);
                    positions.push(ty as f32);
                }
                cx += font::glyph_advance_scaled(ch, font_size) as f32;
            }
        }

        if positions.is_empty() {
            return Ok(());
        }
        let n = (positions.len() / 2) as u32;
        let ctx = self.ctx();

        // SAFETY: same invariant as `submit_rect_batch` — no allocations
        // between view creation and the JS call. `glyphs` is a JS Array
        // (lives on the JS heap), so it is unaffected by WASM memory
        // movement.
        unsafe {
            let view = js_sys::Float32Array::view(&positions);
            oasis_submit_glyphs(ctx, &glyphs, &view, n);
        }
        Ok(())
    }
}
