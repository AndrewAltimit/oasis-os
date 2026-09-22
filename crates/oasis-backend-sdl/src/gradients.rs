//! Gradient fill implementations for the SDL3 backend.
//!
//! Implements `SdiGradients` for `SdlBackend`, supporting vertical,
//! horizontal, and four-corner gradient fills on both rectangular and
//! rounded-rectangular regions.

use oasis_core::backend::{Color, GradientStyle, SdiGradients};
use oasis_core::error::Result;
use oasis_types::color::lerp_color_ratio;

use super::{SdlBackend, frect};

impl SdlBackend {
    /// Fill `x0..x1` of row `y` with the columns' colors of a horizontal
    /// gradient `color_of(dx)` (relative to `origin_x`), one `fill_rect`
    /// per run of equal colors (`h` rows tall).
    fn fill_gradient_columns(
        &mut self,
        origin_x: i32,
        y: i32,
        h: u32,
        (x0, x1): (i32, i32),
        color_of: impl Fn(u32) -> Color,
    ) {
        if x0 >= x1 {
            return;
        }
        let mut run_start = x0;
        let mut run_color = color_of((x0 - origin_x) as u32);
        for x in (x0 + 1)..=x1 {
            let color = if x < x1 {
                color_of((x - origin_x) as u32)
            } else {
                run_color
            };
            if x == x1 || color != run_color {
                self.set_color(run_color);
                let _ = self
                    .canvas
                    .fill_rect(frect(run_start, y, (x - run_start) as u32, h));
                run_start = x;
                run_color = color;
            }
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
        let (tx, ty) = self.translate(x, y);
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;
        let h_max = h.saturating_sub(1).max(1);
        let w_max = w.saturating_sub(1).max(1);

        // Exactly the rows of `fill_rounded_rect` (shared with the
        // software rasterizer), so a gradient card and a flat card with
        // the same radius have the same silhouette. Horizontal and
        // four-corner gradients used to degrade to a flat fill.
        let mut rows = std::mem::take(&mut self.rect_batch);
        rows.clear();
        oasis_rasterize::rounded_rect_rows(w as i32, h as i32, r, |dy, x0, x1| {
            rows.push(frect(x0, dy, (x1 - x0) as u32, 1));
        });
        rows.sort_by_key(|row| row.y as i32);
        match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                // Coalesce rows of the same color and span into one rect.
                let mut i = 0;
                while i < rows.len() {
                    let first = rows[i];
                    let color = lerp_color_ratio(top, bottom, first.y as u32, h_max);
                    let mut j = i + 1;
                    while j < rows.len()
                        && rows[j].x == first.x
                        && rows[j].w == first.w
                        && lerp_color_ratio(top, bottom, rows[j].y as u32, h_max) == color
                    {
                        j += 1;
                    }
                    self.set_color(color);
                    let _ = self.canvas.fill_rect(frect(
                        tx + first.x as i32,
                        ty + first.y as i32,
                        first.w as u32,
                        (j - i) as u32,
                    ));
                    i = j;
                }
            },
            GradientStyle::Horizontal { left, right } => {
                // Rows with the same span share their column runs.
                let mut i = 0;
                while i < rows.len() {
                    let first = rows[i];
                    let mut j = i + 1;
                    while j < rows.len() && rows[j].x == first.x && rows[j].w == first.w {
                        j += 1;
                    }
                    let span = (tx + first.x as i32, tx + (first.x + first.w) as i32);
                    self.fill_gradient_columns(
                        tx,
                        ty + first.y as i32,
                        (j - i) as u32,
                        span,
                        |dx| lerp_color_ratio(left, right, dx, w_max),
                    );
                    i = j;
                }
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                for row in rows.iter().copied() {
                    let dy = row.y as u32;
                    let l = lerp_color_ratio(top_left, bottom_left, dy, h_max);
                    let rt = lerp_color_ratio(top_right, bottom_right, dy, h_max);
                    let span = (tx + row.x as i32, tx + (row.x + row.w) as i32);
                    self.fill_gradient_columns(tx, ty + dy as i32, 1, span, |dx| {
                        lerp_color_ratio(l, rt, dx, w_max)
                    });
                }
            },
        }
        self.rect_batch = rows;
        Ok(())
    }
}
