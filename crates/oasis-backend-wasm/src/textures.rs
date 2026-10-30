//! `SdiTextures` and `SdiClipTransform` implementations for the WASM backend.

use oasis_types::backend::{
    Color, SdiClipTransform, SdiCore, SdiTextures, TextureId, texture_not_found,
};
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
        // Restore before propagating an error: an early return would leave
        // the flip transform and an extra save level on the context, which
        // the clip bookkeeping (one save level per applied clip) would then
        // pop instead of its own.
        let drawn = self.ctx.scale(sx, sy).and_then(|()| {
            self.ctx
                .draw_image_with_html_canvas_element_and_dw_and_dh(&td.canvas, dx, dy, fw, fh)
        });
        self.ctx.restore();
        drawn.map_err(js_err)
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
