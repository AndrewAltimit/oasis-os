//! Gradient fill implementations for the SDL3 backend.
//!
//! Implements `SdiGradients` for `SdlBackend`, supporting vertical,
//! horizontal, and four-corner gradient fills on both rectangular and
//! rounded-rectangular regions.

use oasis_core::backend::{GradientStyle, SdiGradients, SdiShapes};
use oasis_core::error::Result;
use oasis_types::color::lerp_color_ratio;
use oasis_types::geometry::rounded_rect_inset;

use super::{SdlBackend, frect};

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
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                let h_max = h.saturating_sub(1).max(1);
                for dy in 0..h as i32 {
                    let color = lerp_color_ratio(top, bottom, dy as u32, h_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(frect(tx, ty + dy, w, 1));
                }
            },
            GradientStyle::Horizontal { left, right } => {
                let w_max = w.saturating_sub(1).max(1);
                for dx in 0..w as i32 {
                    let color = lerp_color_ratio(left, right, dx as u32, w_max);
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(frect(tx + dx, ty, 1, h));
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
                    for dx in 0..w as i32 {
                        let color = lerp_color_ratio(left, right, dx as u32, w_max);
                        self.set_color(color);
                        let _ = self.canvas.fill_rect(frect(tx + dx, ty + dy, 1, 1));
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
        for dy in 0..h as i32 {
            let color = lerp_color_ratio(top_color, bottom_color, dy as u32, h_max as u32);
            self.set_color(color);

            // Compute horizontal inset for rounded corners.
            let inset = rounded_rect_inset(dy, h as i32, r);

            let lx = tx + inset;
            let rx = tx + w as i32 - 1 - inset;
            if lx <= rx {
                let _ = self
                    .canvas
                    .fill_rect(frect(lx, ty + dy, (rx - lx + 1) as u32, 1));
            }
        }
        Ok(())
    }
}
