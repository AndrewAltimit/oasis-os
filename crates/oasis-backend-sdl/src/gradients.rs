//! Gradient fill implementations for the SDL3 backend.
//!
//! Implements `SdiGradients` for `SdlBackend`, supporting vertical,
//! horizontal, and four-corner gradient fills on both rectangular and
//! rounded-rectangular regions. A failed SDL fill is reported, not
//! dropped: the first error ends the gradient and is returned.

use oasis_types::backend::{BackendErrExt, Color, GradientStyle, SdiGradients};
use oasis_types::color::lerp_color_ratio;
use oasis_types::error::Result;

use sdl3::render::FRect;

use super::{SdlBackend, frect};

/// The offsets `start..end` of `0..len` (measured from `pos`) that land on
/// `0..screen`, or `None` when none do.
fn visible_span(pos: i32, len: u32, screen: u32) -> Option<(u32, u32)> {
    let start = (-i64::from(pos)).max(0);
    let end = (i64::from(screen) - i64::from(pos)).min(i64::from(len));
    (start < end).then_some((start as u32, end as u32))
}

/// `pos + off` for an offset [`visible_span`] returned (on screen, so in
/// `i32` range).
fn at(pos: i32, off: u32) -> i32 {
    (i64::from(pos) + i64::from(off)) as i32
}

/// Split `start..end` into runs of equal `color_of`, calling
/// `emit(run_start, run_len, color)` for each in order; the first error
/// stops it.
fn color_runs(
    (start, end): (u32, u32),
    color_of: impl Fn(u32) -> Color,
    mut emit: impl FnMut(u32, u32, Color) -> Result<()>,
) -> Result<()> {
    if start >= end {
        return Ok(());
    }
    let mut run_start = start;
    let mut run_color = color_of(start);
    for i in start + 1..end {
        let color = color_of(i);
        if color != run_color {
            emit(run_start, i - run_start, run_color)?;
            run_start = i;
            run_color = color;
        }
    }
    emit(run_start, end - run_start, run_color)
}

impl SdlBackend {
    /// One solid `w` x `h` run of a gradient at `(x, y)` (surface pixels).
    fn fill_run(&mut self, color: Color, (x, y): (i32, i32), (w, h): (u32, u32)) -> Result<()> {
        self.set_color(color);
        self.canvas.fill_rect(frect(x, y, w, h)).backend_err()
    }

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
    ) -> Result<()> {
        if x0 >= x1 {
            return Ok(());
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
                self.canvas
                    .fill_rect(frect(run_start, y, (x - run_start) as u32, h))
                    .backend_err()?;
                run_start = x;
                run_color = color;
            }
        }
        Ok(())
    }

    /// The colour runs of a rounded-rect gradient over `rows` (the shape's
    /// rows relative to `(tx, ty)`, sorted by row).
    fn fill_rounded_gradient_rows(
        &mut self,
        (tx, ty): (i32, i32),
        rows: &[FRect],
        (w_max, h_max): (u32, u32),
        gradient: &GradientStyle,
    ) -> Result<()> {
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
                    self.canvas
                        .fill_rect(frect(
                            tx + first.x as i32,
                            ty + first.y as i32,
                            first.w as u32,
                            (j - i) as u32,
                        ))
                        .backend_err()?;
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
                    )?;
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
                    })?;
                }
            },
        }
        Ok(())
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
        // Only the rows and columns on the bound surface are generated
        // (colors still run over the whole rect): a page-tall browser
        // gradient used to interpolate every off-screen row, and every
        // off-screen pixel of a four-corner one, each frame.
        let (Some((vx0, vx1)), Some((vy0, vy1))) = (
            visible_span(tx, w, self.viewport_w),
            visible_span(ty, h, self.viewport_h),
        ) else {
            return Ok(());
        };
        let (vw, vh) = (vx1 - vx0, vy1 - vy0);
        let h_max = h.saturating_sub(1).max(1);
        let w_max = w.saturating_sub(1).max(1);
        // Consecutive scanlines/columns often quantize to the same color
        // (any span longer than ~256px must repeat); coalescing those runs
        // into one fill_rect cuts the SDL call count without changing a
        // single output pixel.
        match *gradient {
            GradientStyle::Vertical { top, bottom } => color_runs(
                (vy0, vy1),
                |dy| lerp_color_ratio(top, bottom, dy, h_max),
                |start, len, color| self.fill_run(color, (at(tx, vx0), at(ty, start)), (vw, len)),
            ),
            GradientStyle::Horizontal { left, right } => color_runs(
                (vx0, vx1),
                |dx| lerp_color_ratio(left, right, dx, w_max),
                |start, len, color| self.fill_run(color, (at(tx, start), at(ty, vy0)), (len, vh)),
            ),
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                for dy in vy0..vy1 {
                    let left = lerp_color_ratio(top_left, bottom_left, dy, h_max);
                    let right = lerp_color_ratio(top_right, bottom_right, dy, h_max);
                    color_runs(
                        (vx0, vx1),
                        |dx| lerp_color_ratio(left, right, dx, w_max),
                        |start, len, color| {
                            self.fill_run(color, (at(tx, start), at(ty, dy)), (len, 1))
                        },
                    )?;
                }
                Ok(())
            },
        }
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
        // The scratch goes back before an error propagates.
        let drawn = self.fill_rounded_gradient_rows((tx, ty), &rows, (w_max, h_max), gradient);
        self.rect_batch = rows;
        drawn
    }
}
