//! Gradient fill implementations for the SDL3 backend.
//!
//! Implements `SdiGradients` for `SdlBackend`, supporting vertical,
//! horizontal, and four-corner gradient fills on both rectangular and
//! rounded-rectangular regions.

use oasis_core::backend::{Color, GradientStyle, SdiGradients, SdiShapes};
use oasis_core::error::Result;
use oasis_types::color::lerp_color_ratio;
use oasis_types::geometry::rounded_rect_inset;

use super::{SdlBackend, frect};

impl SdlBackend {
    /// Fill one coalesced run of rounded-gradient scanlines
    /// (`start_dy..end_dy`, same color and corner inset).
    #[allow(clippy::too_many_arguments)]
    fn flush_rounded_gradient_run(
        &mut self,
        tx: i32,
        ty: i32,
        w: u32,
        start_dy: i32,
        end_dy: i32,
        color: Color,
        inset: i32,
    ) {
        let lx = tx + inset;
        let rx = tx + w as i32 - 1 - inset;
        if lx <= rx && end_dy > start_dy {
            self.set_color(color);
            let _ = self.canvas.fill_rect(frect(
                lx,
                ty + start_dy,
                (rx - lx + 1) as u32,
                (end_dy - start_dy) as u32,
            ));
        }
    }
}

impl SdiGradients for SdlBackend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        // Consecutive scanlines/columns often quantize to the same color
        // (any span longer than ~256px must repeat); coalescing those runs
        // into one fill_rect cuts the SDL call count without changing a
        // single output pixel.
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                let h_max = h.saturating_sub(1).max(1);
                let mut run_start = 0i32;
                let mut run_color = lerp_color_ratio(top, bottom, 0, h_max);
                for dy in 1..=h as i32 {
                    let color = if (dy as u32) < h {
                        lerp_color_ratio(top, bottom, dy as u32, h_max)
                    } else {
                        run_color // sentinel comparison never matches below
                    };
                    if dy as u32 == h || color != run_color {
                        self.set_color(run_color);
                        let run_h = (dy - run_start) as u32;
                        let _ = self.canvas.fill_rect(frect(tx, ty + run_start, w, run_h));
                        run_start = dy;
                        run_color = color;
                    }
                }
            },
            GradientStyle::Horizontal { left, right } => {
                let w_max = w.saturating_sub(1).max(1);
                let mut run_start = 0i32;
                let mut run_color = lerp_color_ratio(left, right, 0, w_max);
                for dx in 1..=w as i32 {
                    let color = if (dx as u32) < w {
                        lerp_color_ratio(left, right, dx as u32, w_max)
                    } else {
                        run_color
                    };
                    if dx as u32 == w || color != run_color {
                        self.set_color(run_color);
                        let run_w = (dx - run_start) as u32;
                        let _ = self.canvas.fill_rect(frect(tx + run_start, ty, run_w, h));
                        run_start = dx;
                        run_color = color;
                    }
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
                    let left = lerp_color_ratio(top_left, bottom_left, dy as u32, h_max);
                    let right = lerp_color_ratio(top_right, bottom_right, dy as u32, h_max);
                    // Coalesce horizontal runs of identical color within the row.
                    let mut run_start = 0i32;
                    let mut run_color = lerp_color_ratio(left, right, 0, w_max);
                    for dx in 1..=w as i32 {
                        let color = if (dx as u32) < w {
                            lerp_color_ratio(left, right, dx as u32, w_max)
                        } else {
                            run_color
                        };
                        if dx as u32 == w || color != run_color {
                            self.set_color(run_color);
                            let run_w = (dx - run_start) as u32;
                            let _ = self
                                .canvas
                                .fill_rect(frect(tx + run_start, ty + dy, run_w, 1));
                            run_start = dx;
                            run_color = color;
                        }
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
        // Runs of rows with the same color AND the same corner inset (the
        // whole straight middle section, plus repeated colors on tall
        // rects) collapse into one fill_rect — identical pixels, far
        // fewer SDL calls.
        let mut run_start: Option<(i32, Color, i32)> = None; // (start_dy, color, inset)
        for dy in 0..h as i32 {
            let color = lerp_color_ratio(top_color, bottom_color, dy as u32, h_max as u32);
            let inset = rounded_rect_inset(dy, h as i32, r);

            let extends = matches!(run_start, Some((_, rc, ri)) if rc == color && ri == inset);
            if !extends {
                if let Some((start, rc, ri)) = run_start.take() {
                    self.flush_rounded_gradient_run(tx, ty, w, start, dy, rc, ri);
                }
                run_start = Some((dy, color, inset));
            }
        }
        if let Some((start, run_color, run_inset)) = run_start {
            self.flush_rounded_gradient_run(tx, ty, w, start, h as i32, run_color, run_inset);
        }
        Ok(())
    }
}
