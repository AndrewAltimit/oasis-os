//! `SdiShapes` implementation for the WASM backend.

use oasis_types::backend::{Color, SdiShapes};
use oasis_types::error::Result;

use crate::renderer::{WasmBackend, cached_css_color, js_err};

impl SdiShapes for WasmBackend {
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
}
