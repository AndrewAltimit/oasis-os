//! `SdiTextures` and `SdiClipTransform` implementations for the WASM backend.

use oasis_types::backend::{Color, SdiClipTransform, SdiTextures, TextureId, texture_not_found};
use oasis_types::error::Result;
use oasis_types::geometry::ClipRect;

use crate::renderer::{WasmBackend, cached_css_color, js_err};

// -------------------------------------------------------------------
// SdiTextures: Texture operations
// -------------------------------------------------------------------

impl SdiTextures for WasmBackend {
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
            .ok_or_else(|| texture_not_found(tex))?;
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

    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        self.blit_tinted_region(tex, None, (x, y, w, h), tint)
    }
}

impl WasmBackend {
    /// Blit `src` of `tex` (the whole texture when `None`) to `dst`
    /// multiplied by `tint`: RGB times the tint's RGB, alpha times its
    /// alpha, as on SDL and the software rasterizer.
    ///
    /// The tint is composed on an offscreen scratch canvas, never on the
    /// surface: the texture is copied there, `multiply`-filled with the
    /// opaque tint colour and cut back to the texture's own alpha with
    /// `destination-in`, then drawn with `globalAlpha` = the tint's alpha.
    /// (A `multiply` rectangle straight over the surface, the previous
    /// approach, also painted the tint into the texture's transparent
    /// pixels: a tinted icon became a tinted square.) Opaque texels, and
    /// every texel of a white mask such as a glyph, come out exact; a
    /// partially transparent coloured texel leans slightly towards the
    /// tint, since canvas compositing cannot un-premultiply.
    fn blit_tinted_region(
        &mut self,
        tex: TextureId,
        src: Option<(u32, u32, u32, u32)>,
        dst: (i32, i32, u32, u32),
        tint: Color,
    ) -> Result<()> {
        let source = self
            .textures
            .get(&tex.0)
            .ok_or_else(|| texture_not_found(tex))?
            .canvas
            .clone();
        let (sx, sy, sw, sh) = src.unwrap_or_else(|| (0, 0, source.width(), source.height()));
        if tint.a == 0 || sw == 0 || sh == 0 || dst.2 == 0 || dst.3 == 0 {
            return Ok(());
        }
        let opaque = Color::rgb(tint.r, tint.g, tint.b);
        let css = cached_css_color(&mut self.color_cache, opaque).to_owned();
        let scratch = self.tint_scratch(sw, sh)?;
        let (canvas, sctx) = (scratch.canvas.clone(), scratch.ctx.clone());
        let (sx, sy, sw, sh) = (sx as f64, sy as f64, sw as f64, sh as f64);
        let copy_source = |op: &str| {
            sctx.set_global_composite_operation(op)?;
            sctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &source, sx, sy, sw, sh, 0.0, 0.0, sw, sh,
            )
        };
        // `copy` also clears whatever an earlier tint left on the scratch.
        let composed = copy_source("copy")
            .and_then(|()| sctx.set_global_composite_operation("multiply"))
            .and_then(|()| {
                sctx.set_fill_style_str(&css);
                sctx.fill_rect(0.0, 0.0, sw, sh);
                copy_source("destination-in")
            });
        let _ = sctx.set_global_composite_operation("source-over");
        composed.map_err(js_err)?;

        let (tx, ty) = self.translate(dst.0, dst.1);
        let prev_alpha = self.ctx.global_alpha();
        self.ctx
            .set_global_alpha(prev_alpha * f64::from(tint.a) / 255.0);
        let drawn = self
            .ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &canvas,
                0.0,
                0.0,
                sw,
                sh,
                tx,
                ty,
                dst.2 as f64,
                dst.3 as f64,
            );
        self.ctx.set_global_alpha(prev_alpha);
        drawn.map_err(js_err)
    }
}

// -------------------------------------------------------------------
// SdiClipTransform: Clip and transform stacks
// -------------------------------------------------------------------

impl SdiClipTransform for WasmBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.clip.push(&self.ctx, x, y, w, h);
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.clip.pop(&self.ctx);
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<ClipRect> {
        self.clip.stack.current()
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.clip.translate.push(dx, dy);
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        self.clip.translate.pop();
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.clip.translate.current()
    }
}
