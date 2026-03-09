//! Shape drawing primitives for the SDL3 backend.
//!
//! Contains all shape-related drawing methods (rounded rects, lines,
//! circles, triangles, polygons, arcs) and helper functions used by both
//! this module and the parent `lib.rs` (gradients, clip intersection).

use oasis_core::backend::{Color, SdiCore};
use oasis_core::error::Result;

use super::{ClipRect, SdlBackend, fpoint, frect};

// -------------------------------------------------------------------
// Inherent shape methods on SdlBackend
// -------------------------------------------------------------------

impl SdlBackend {
    /// Fill a triangle using pre-translated screen coordinates.
    ///
    /// Used by `fill_arc` which translates the center once and computes
    /// all vertices in screen space.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_triangle_translated(
        &mut self,
        tx1: i32,
        ty1: i32,
        tx2: i32,
        ty2: i32,
        tx3: i32,
        ty3: i32,
        color: Color,
    ) {
        let mut verts = [(tx1, ty1), (tx2, ty2), (tx3, ty3)];
        verts.sort_by_key(|v| v.1);
        let (vx0, vy0) = verts[0];
        let (vx1, vy1) = verts[1];
        let (vx2, vy2) = verts[2];

        self.set_color(color);
        for y in vy0..=vy2 {
            let mut x_min = i32::MAX;
            let mut x_max = i32::MIN;
            let x_02 = edge_x(vx0, vy0, vx2, vy2, y);
            x_min = x_min.min(x_02);
            x_max = x_max.max(x_02);
            if y <= vy1 && vy0 != vy1 {
                let x_01 = edge_x(vx0, vy0, vx1, vy1, y);
                x_min = x_min.min(x_01);
                x_max = x_max.max(x_01);
            }
            if y >= vy1 && vy1 != vy2 {
                let x_12 = edge_x(vx1, vy1, vx2, vy2, y);
                x_min = x_min.min(x_12);
                x_max = x_max.max(x_12);
            }
            if y == vy1 {
                x_min = x_min.min(vx1);
                x_max = x_max.max(vx1);
            }
            if x_min <= x_max {
                let _ = self.canvas.draw_line(fpoint(x_min, y), fpoint(x_max, y));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let (tx, ty) = self.translate(x, y);
        let r = (radius as u32).min(w / 2).min(h / 2) as i32;
        self.set_color(color);

        // Center body rect.
        let _ = self
            .canvas
            .fill_rect(frect(tx, ty + r, w, h - r as u32 * 2));
        // Top strip.
        let _ = self
            .canvas
            .fill_rect(frect(tx + r, ty, w - r as u32 * 2, r as u32));
        // Bottom strip.
        let _ = self
            .canvas
            .fill_rect(frect(tx + r, ty + h as i32 - r, w - r as u32 * 2, r as u32));

        // Corner fills using midpoint circle horizontal spans.
        let mut cx = 0i32;
        let mut cy = r;
        let mut d = 1 - r;
        while cx <= cy {
            // Top-left + top-right.
            let _ = self.canvas.draw_line(
                fpoint(tx + r - cy, ty + r - cx),
                fpoint(tx + w as i32 - 1 - r + cy, ty + r - cx),
            );
            if cx != cy {
                let _ = self.canvas.draw_line(
                    fpoint(tx + r - cx, ty + r - cy),
                    fpoint(tx + w as i32 - 1 - r + cx, ty + r - cy),
                );
            }
            // Bottom-left + bottom-right.
            if cx != 0 {
                let _ = self.canvas.draw_line(
                    fpoint(tx + r - cy, ty + h as i32 - 1 - r + cx),
                    fpoint(tx + w as i32 - 1 - r + cy, ty + h as i32 - 1 - r + cx),
                );
            }
            let _ = self.canvas.draw_line(
                fpoint(tx + r - cx, ty + h as i32 - 1 - r + cy),
                fpoint(tx + w as i32 - 1 - r + cx, ty + h as i32 - 1 - r + cy),
            );

            cx += 1;
            if d < 0 {
                d += 2 * cx + 1;
            } else {
                cy -= 1;
                d += 2 * (cx - cy) + 1;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.set_color(color);
        if stroke_width == 1 {
            let _ = self.canvas.draw_rect(frect(tx, ty, w, h));
        } else {
            let sw = stroke_width as u32;
            let _ = self.canvas.fill_rect(frect(tx, ty, w, sw));
            let _ = self
                .canvas
                .fill_rect(frect(tx, ty + h as i32 - sw as i32, w, sw));
            let _ = self
                .canvas
                .fill_rect(frect(tx, ty + sw as i32, sw, h.saturating_sub(sw * 2)));
            let _ = self.canvas.fill_rect(frect(
                tx + w as i32 - sw as i32,
                ty + sw as i32,
                sw,
                h.saturating_sub(sw * 2),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.set_color(color);
        if width <= 1 {
            let _ = self.canvas.draw_line(fpoint(tx1, ty1), fpoint(tx2, ty2));
        } else {
            // Draw multiple parallel lines for thickness.
            let half = width as i32 / 2;
            let dx = (tx2 - tx1) as f32;
            let dy = (ty2 - ty1) as f32;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let nx = (-dy / len) as i32;
            let ny = (dx / len) as i32;
            for i in -half..=(width as i32 - half - 1) {
                let ox = nx * i;
                let oy = ny * i;
                let _ = self
                    .canvas
                    .draw_line(fpoint(tx1 + ox, ty1 + oy), fpoint(tx2 + ox, ty2 + oy));
            }
        }
        Ok(())
    }

    pub(crate) fn shape_fill_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        let r = radius as i32;
        self.set_color(color);

        let mut x = 0i32;
        let mut y = r;
        let mut d = 1 - r;
        while x <= y {
            let _ = self
                .canvas
                .draw_line(fpoint(tcx - y, tcy + x), fpoint(tcx + y, tcy + x));
            if x != 0 {
                let _ = self
                    .canvas
                    .draw_line(fpoint(tcx - y, tcy - x), fpoint(tcx + y, tcy - x));
            }
            if x != y {
                let _ = self
                    .canvas
                    .draw_line(fpoint(tcx - x, tcy + y), fpoint(tcx + x, tcy + y));
                let _ = self
                    .canvas
                    .draw_line(fpoint(tcx - x, tcy - y), fpoint(tcx + x, tcy - y));
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.set_color(color);
        let sw = (stroke_width as i32).max(1);

        // Draw concentric circle outlines for the requested stroke width.
        for offset in 0..sw {
            let r = radius as i32 - offset;
            if r <= 0 {
                break;
            }

            let mut x = 0i32;
            let mut y = r;
            let mut d = 1 - r;
            while x <= y {
                // Plot 8 symmetric points on the perimeter.
                for &(px, py) in &[
                    (tcx + x, tcy + y),
                    (tcx - x, tcy + y),
                    (tcx + x, tcy - y),
                    (tcx - x, tcy - y),
                    (tcx + y, tcy + x),
                    (tcx - y, tcy + x),
                    (tcx + y, tcy - x),
                    (tcx - y, tcy - x),
                ] {
                    let _ = self.canvas.draw_point(fpoint(px, py));
                }
                x += 1;
                if d < 0 {
                    d += 2 * x + 1;
                } else {
                    y -= 1;
                    d += 2 * (x - y) + 1;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        let (tx3, ty3) = self.translate(x3, y3);

        // Sort by y.
        let mut verts = [(tx1, ty1), (tx2, ty2), (tx3, ty3)];
        verts.sort_by_key(|v| v.1);
        let (vx0, vy0) = verts[0];
        let (vx1, vy1) = verts[1];
        let (vx2, vy2) = verts[2];

        self.set_color(color);

        for y in vy0..=vy2 {
            let mut x_min = i32::MAX;
            let mut x_max = i32::MIN;

            let x_02 = edge_x(vx0, vy0, vx2, vy2, y);
            x_min = x_min.min(x_02);
            x_max = x_max.max(x_02);

            if y <= vy1 && vy0 != vy1 {
                let x_01 = edge_x(vx0, vy0, vx1, vy1, y);
                x_min = x_min.min(x_01);
                x_max = x_max.max(x_01);
            }
            if y >= vy1 && vy1 != vy2 {
                let x_12 = edge_x(vx1, vy1, vx2, vy2, y);
                x_min = x_min.min(x_12);
                x_max = x_max.max(x_12);
            }
            if y == vy1 {
                x_min = x_min.min(vx1);
                x_max = x_max.max(vx1);
            }

            if x_min <= x_max {
                let _ = self.canvas.draw_line(fpoint(x_min, y), fpoint(x_max, y));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.shape_stroke_rect(x, y, w, h, stroke_width, color);
        }
        let (tx, ty) = self.translate(x, y);
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        self.set_color(color);

        let sw = (stroke_width as i32).max(1);
        for t in 0..sw {
            // Top edge.
            let _ = self.canvas.draw_line(
                fpoint(tx + r, ty + t),
                fpoint(tx + w as i32 - 1 - r, ty + t),
            );
            // Bottom edge.
            let _ = self.canvas.draw_line(
                fpoint(tx + r, ty + h as i32 - 1 - t),
                fpoint(tx + w as i32 - 1 - r, ty + h as i32 - 1 - t),
            );
            // Left edge.
            let _ = self.canvas.draw_line(
                fpoint(tx + t, ty + r),
                fpoint(tx + t, ty + h as i32 - 1 - r),
            );
            // Right edge.
            let _ = self.canvas.draw_line(
                fpoint(tx + w as i32 - 1 - t, ty + r),
                fpoint(tx + w as i32 - 1 - t, ty + h as i32 - 1 - r),
            );

            // Rounded corners via midpoint circle arc.
            let cr = r - t;
            if cr <= 0 {
                continue;
            }
            let mut cx = 0i32;
            let mut cy = cr;
            let mut d = 1 - cr;
            while cx <= cy {
                // Top-left corner.
                let _ = self.canvas.draw_point(fpoint(tx + r - cy, ty + r - cx));
                if cx != cy {
                    let _ = self.canvas.draw_point(fpoint(tx + r - cx, ty + r - cy));
                }
                // Top-right corner.
                let _ = self
                    .canvas
                    .draw_point(fpoint(tx + w as i32 - 1 - r + cy, ty + r - cx));
                if cx != cy {
                    let _ = self
                        .canvas
                        .draw_point(fpoint(tx + w as i32 - 1 - r + cx, ty + r - cy));
                }
                // Bottom-left corner.
                if cx != 0 {
                    let _ = self
                        .canvas
                        .draw_point(fpoint(tx + r - cy, ty + h as i32 - 1 - r + cx));
                }
                let _ = self
                    .canvas
                    .draw_point(fpoint(tx + r - cx, ty + h as i32 - 1 - r + cy));
                // Bottom-right corner.
                if cx != 0 {
                    let _ = self.canvas.draw_point(fpoint(
                        tx + w as i32 - 1 - r + cy,
                        ty + h as i32 - 1 - r + cx,
                    ));
                }
                let _ = self.canvas.draw_point(fpoint(
                    tx + w as i32 - 1 - r + cx,
                    ty + h as i32 - 1 - r + cy,
                ));

                cx += 1;
                if d < 0 {
                    d += 2 * cx + 1;
                } else {
                    cy -= 1;
                    d += 2 * (cx - cy) + 1;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn shape_fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        if points.len() < 3 {
            return Ok(());
        }
        self.set_color(color);

        // Collect translated, y-sorted scanline edges.
        let translated: Vec<(i32, i32)> =
            points.iter().map(|&(x, y)| self.translate(x, y)).collect();

        let y_min = translated.iter().map(|v| v.1).min().unwrap_or(0);
        let y_max = translated.iter().map(|v| v.1).max().unwrap_or(0);

        for y in y_min..=y_max {
            let mut x_intersections = Vec::new();
            let n = translated.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (x0, y0) = translated[i];
                let (x1, y1) = translated[j];
                if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                    let t = (y - y0) as f32 / (y1 - y0) as f32;
                    x_intersections.push(x0 + (t * (x1 - x0) as f32) as i32);
                }
            }
            x_intersections.sort_unstable();
            for pair in x_intersections.chunks_exact(2) {
                let _ = self
                    .canvas
                    .draw_line(fpoint(pair[0], y), fpoint(pair[1], y));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_fill_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    ) -> Result<()> {
        use oasis_types::backend::{arc_segments, cos_approx_f32, sin_approx_f32};
        let (tcx, tcy) = self.translate(cx, cy);
        self.set_color(color);
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;

        // Build triangle fan vertices and scanline-fill each triangle.
        let mut prev_x = tcx + (r * cos_approx_f32(start_angle)) as i32;
        let mut prev_y = tcy + (r * sin_approx_f32(start_angle)) as i32;
        for i in 1..=segments {
            let angle = start_angle + step * i as f32;
            let nx = tcx + (r * cos_approx_f32(angle)) as i32;
            let ny = tcy + (r * sin_approx_f32(angle)) as i32;
            self.fill_triangle_translated(tcx, tcy, prev_x, prev_y, nx, ny, color);
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_stroke_arc(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        use oasis_types::backend::{arc_segments, cos_approx_f32, sin_approx_f32};
        let (tcx, tcy) = self.translate(cx, cy);
        self.set_color(color);
        let segments = arc_segments(radius, start_angle, end_angle);
        let r = radius as f32;
        let step = (end_angle - start_angle) / segments as f32;

        let half = width as i32 / 2;
        let mut prev_x = tcx + (r * cos_approx_f32(start_angle)) as i32;
        let mut prev_y = tcy + (r * sin_approx_f32(start_angle)) as i32;
        for i in 1..=segments {
            let angle = start_angle + step * i as f32;
            let nx = tcx + (r * cos_approx_f32(angle)) as i32;
            let ny = tcy + (r * sin_approx_f32(angle)) as i32;
            // Thicken: draw parallel lines.
            for offset in -half..=(width as i32 - half - 1) {
                let dx = (nx - prev_x) as f32;
                let dy = (ny - prev_y) as f32;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let ox = (-dy / len * offset as f32) as i32;
                let oy = (dx / len * offset as f32) as i32;
                let _ = self
                    .canvas
                    .draw_line(fpoint(prev_x + ox, prev_y + oy), fpoint(nx + ox, ny + oy));
            }
            prev_x = nx;
            prev_y = ny;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_stroke_line_dashed(
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
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.set_color(color);

        let dx = (tx2 - tx1) as f32;
        let dy = (ty2 - ty1) as f32;
        let total_len = (dx * dx + dy * dy).sqrt();
        if total_len < 1.0 {
            return Ok(());
        }
        let ux = dx / total_len;
        let uy = dy / total_len;
        let cycle = dash as f32 + gap as f32;
        let half = width as i32 / 2;
        let mut t = 0.0f32;
        while t < total_len {
            let seg_end = (t + dash as f32).min(total_len);
            let sx = tx1 + (ux * t) as i32;
            let sy = ty1 + (uy * t) as i32;
            let ex = tx1 + (ux * seg_end) as i32;
            let ey = ty1 + (uy * seg_end) as i32;
            for offset in -half..=(width as i32 - half - 1) {
                let ox = (-uy * offset as f32) as i32;
                let oy = (ux * offset as f32) as i32;
                let _ = self
                    .canvas
                    .draw_line(fpoint(sx + ox, sy + oy), fpoint(ex + ox, ey + oy));
            }
            t += cycle;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// Free helper functions
// -------------------------------------------------------------------

/// Compute the intersection of two clip rectangles.
pub(crate) fn intersect_clip(a: &ClipRect, b: &ClipRect) -> Option<ClipRect> {
    let ax2 = a.x.saturating_add(a.w as i32);
    let ay2 = a.y.saturating_add(a.h as i32);
    let bx2 = b.x.saturating_add(b.w as i32);
    let by2 = b.y.saturating_add(b.h as i32);
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let x2 = ax2.min(bx2);
    let y2 = ay2.min(by2);
    if x2 > x && y2 > y {
        Some(ClipRect {
            x,
            y,
            w: (x2 - x) as u32,
            h: (y2 - y) as u32,
        })
    } else {
        None
    }
}

/// Compute the x coordinate along an edge at a given y.
pub(crate) fn edge_x(x0: i32, y0: i32, x1: i32, y1: i32, y: i32) -> i32 {
    if y1 == y0 {
        return x0;
    }
    x0 + (x1 - x0) * (y - y0) / (y1 - y0)
}

/// Integer square root (floor).
pub(crate) fn isqrt(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut x = (n as f32).sqrt() as i32;
    // Newton correction.
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// Linear interpolation between two colors.
pub(crate) fn lerp_color_sdl(a: Color, b: Color, num: u32, den: u32) -> Color {
    if den == 0 {
        return a;
    }
    let inv = den - num;
    Color::rgba(
        ((a.r as u32 * inv + b.r as u32 * num + den / 2) / den) as u8,
        ((a.g as u32 * inv + b.g as u32 * num + den / 2) / den) as u8,
        ((a.b as u32 * inv + b.b as u32 * num + den / 2) / den) as u8,
        ((a.a as u32 * inv + b.a as u32 * num + den / 2) / den) as u8,
    )
}
