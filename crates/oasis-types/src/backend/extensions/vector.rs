//! `SdiVector` extension trait.

use super::spans::SpanFill;
use crate::backend::SdiShapes;
use crate::backend::{Color, arc_segments, cos_approx_f32, sin_approx_f32};
use crate::error::Result;
use crate::rasterize::polygon_rows;

// ---------------------------------------------------------------------------
// SdiVector
// ---------------------------------------------------------------------------

/// Vector graphics primitives (polygons, arcs, dashed lines).
#[allow(clippy::too_many_arguments)]
pub trait SdiVector: SdiShapes {
    /// Draw a filled polygon under the even-odd rule ([`polygon_rows`]),
    /// so concave and self-intersecting paths fill as on the desktop
    /// backends.
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        let mut xs = Vec::new();
        let mut fill = SpanFill::new(self, color);
        polygon_rows(points, &mut xs, |y, x0, x1| fill.span(y, x0, x1));
        fill.finish()
    }

    /// Draw the outline of a polygon (closed path).
    fn stroke_polygon(&mut self, points: &[(i32, i32)], width: u16, color: Color) -> Result<()> {
        if points.len() < 2 {
            return Ok(());
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            self.draw_line(
                points[i].0,
                points[i].1,
                points[j].0,
                points[j].1,
                width,
                color,
            )?;
        }
        Ok(())
    }

    /// Draw an open polyline (does not close back to the first point).
    fn stroke_polyline(&mut self, points: &[(i32, i32)], width: u16, color: Color) -> Result<()> {
        for pair in points.windows(2) {
            self.draw_line(pair[0].0, pair[0].1, pair[1].0, pair[1].1, width, color)?;
        }
        Ok(())
    }

    /// Draw a filled arc: the pie wedge from `start_angle` to `end_angle`
    /// (radians, clockwise in screen space), approximated by a polygon of
    /// the centre and [`arc_segments`] points along the arc.
    ///
    /// The wedge is filled as **one** polygon ([`fill_polygon`]), so every
    /// pixel is covered once and a translucent wedge blends uniformly (a
    /// fan of triangles covers each shared spoke twice). A sweep beyond a
    /// full turn is clamped to one turn.
    ///
    /// [`fill_polygon`]: Self::fill_polygon
    fn fill_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) -> Result<()> {
        let tau = core::f32::consts::TAU;
        let end_angle = start_angle + (end_angle - start_angle).clamp(-tau, tau);
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;
        let mut points = Vec::with_capacity(segments + 2);
        points.push((cx, cy));
        for i in 0..=segments {
            let angle = start_angle + step * i as f32;
            points.push((
                cx + (r * cos_approx_f32(angle)) as i32,
                cy + (r * sin_approx_f32(angle)) as i32,
            ));
        }
        self.fill_polygon(&points, color)
    }

    /// Draw an arc stroke (line segments along arc).
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
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;
        let mut prev_x = cx + (r * cos_approx_f32(start_angle)) as i32;
        let mut prev_y = cy + (r * sin_approx_f32(start_angle)) as i32;
        for i in 1..=segments {
            let angle = start_angle + step * i as f32;
            let nx = cx + (r * cos_approx_f32(angle)) as i32;
            let ny = cy + (r * sin_approx_f32(angle)) as i32;
            self.draw_line(prev_x, prev_y, nx, ny, width, color)?;
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    /// Draw a dashed line between two points: `dash` pixels drawn, `gap`
    /// pixels skipped, repeating from `(x1, y1)`.
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
        // A zero gap is a solid line and a zero dash draws nothing, as
        // with Canvas 2D `setLineDash`; it also keeps `t` advancing below.
        if gap == 0 {
            return self.draw_line(x1, y1, x2, y2, width, color);
        }
        if dash == 0 {
            return Ok(());
        }
        let dx = x2 as f32 - x1 as f32;
        let dy = y2 as f32 - y1 as f32;
        let total_len = (dx * dx + dy * dy).sqrt();
        if total_len < 1.0 {
            return Ok(());
        }
        let ux = dx / total_len;
        let uy = dy / total_len;
        let cycle = dash as f32 + gap as f32;
        let mut t = 0.0f32;
        while t < total_len {
            let seg_end = (t + dash as f32).min(total_len);
            let sx = x1 + (ux * t) as i32;
            let sy = y1 + (uy * t) as i32;
            let ex = x1 + (ux * seg_end) as i32;
            let ey = y1 + (uy * seg_end) as i32;
            self.draw_line(sx, sy, ex, ey, width, color)?;
            t += cycle;
        }
        Ok(())
    }
}
