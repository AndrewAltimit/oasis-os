//! Cell-grid shaders: Voronoi and city lights.

use super::*;

impl SoftwareShaderRenderer {
    /// Voronoi cell pattern (port of shadertoy.com/view/WdlyRS).
    pub(super) fn render_voronoi(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::Voronoi;
        let speed = sh.float(params, "speed");
        let size = cell_size(sh.float(params, "size"));
        let col1 = sh.color(params, 0);
        let col2 = sh.color(params, 1);

        let t = time * speed * 2.0;
        let w = self.width as f32;
        let h = self.height as f32;
        let off_x = time / 50.0;
        let off_y = time / 30.0;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        // The 3x3 neighbour search evaluates `voronoi_pt` (4 sins) nine
        // times per pixel, but the animated points depend only on the
        // integer cell id — precompute them once for every cell in view
        // (plus a 1-cell margin for the neighbour taps). uv is monotonic
        // in the pixel position, so the corner pixels bound the id range.
        let uv_x = |ox: f32| ((ox - 0.5 * w) / w + off_x) * size;
        let uv_y = |oy: f32| ((oy - 0.5 * h) / w + off_y) * size;
        let (ox0, oy0) = lo_to_out(0, 0, scale_f);
        let (ox1, oy1) = lo_to_out(iw - 1, ih - 1, scale_f);
        // min/max over both corners so a negative `size` (reversed uv
        // direction) still yields valid bounds.
        let (ex0, ex1) = (uv_x(ox0).floor() as i32, uv_x(ox1).floor() as i32);
        let (ey0, ey1) = (uv_y(oy0).floor() as i32, uv_y(oy1).floor() as i32);
        let idx_min = ex0.min(ex1) - 1;
        let idy_min = ey0.min(ey1) - 1;
        let gw = (ex0.max(ex1) + 1 - idx_min + 1) as usize;
        let gh = (ey0.max(ey1) + 1 - idy_min + 1) as usize;
        // Pathological params (huge/NaN `size`) could explode the grid;
        // fall back to direct per-pixel evaluation instead of allocating.
        let use_grid = gw.saturating_mul(gh) <= (1 << 20);
        let mut pts = vec![(0.0f32, 0.0f32); if use_grid { gw * gh } else { 0 }];
        if use_grid {
            for gy in 0..gh {
                for gx in 0..gw {
                    let nid_x = (idx_min + gx as i32) as f32;
                    let nid_y = (idy_min + gy as i32) as f32;
                    pts[gy * gw + gx] = voronoi_pt(t, nid_x, nid_y);
                }
            }
        }

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            for (ix, out) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let uvx = uv_x(ox);
                let uvy = uv_y(oy);

                let gvx = fract(uvx) - 0.5;
                let gvy = fract(uvy) - 0.5;
                let idx = uvx.floor();
                let idy = uvy.floor();
                let gx0 = (idx as i32 - idx_min) as usize;
                let gy0 = (idy as i32 - idy_min) as usize;

                let mut mindist2: f32 = 1e9;
                let mut vorv_x: f32 = 0.0;
                let mut vorv_y: f32 = 0.0;

                for i in -1i32..=1 {
                    for j in -1i32..=1 {
                        let fi = i as f32;
                        let fj = j as f32;
                        let (px, py) = if use_grid {
                            let gx = (gx0 as i32 + i) as usize;
                            let gy = (gy0 as i32 + j) as usize;
                            pts[gy * gw + gx]
                        } else {
                            voronoi_pt(t, idx + fi, idy + fj)
                        };
                        let dx = gvx + px - fi;
                        let dy = gvy + py - fj;
                        let dist2 = dx * dx + dy * dy;
                        if dist2 < mindist2 {
                            mindist2 = dist2;
                            vorv_x = (idx + px + fi) / size - off_x;
                            vorv_y = (idy + py + fj) / size - off_y;
                        }
                    }
                }

                let blend = (vorv_x * 2.2 + vorv_y).clamp(-1.0, 1.0) * 0.5 + 0.5;
                let r = lerp(col1[0], col2[0], blend).clamp(0.0, 1.0);
                let g = lerp(col1[1], col2[1], blend).clamp(0.0, 1.0);
                let b = lerp(col1[2], col2[2], blend).clamp(0.0, 1.0);

                *out = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        });

        self.upscale(iw, ih)
    }

    /// City-lights colour grid (port of shadertoy.com/view/wscGWl).
    pub(super) fn render_city_lights(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::CityLights;
        let speed = sh.float(params, "speed");
        let size = cell_size(sh.float(params, "size"));
        let w = self.width as f32;
        let h = self.height as f32;
        let mx = w.max(h);
        let anim_time = time * 0.5 * speed;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        // Everything except the two shadow smoothsteps depends only on
        // the integer cell id (~8 sin/cos per pixel otherwise). Values
        // per cell: [dim, cr, cg, cb, left_diff.min(0), top_diff.min(0)].
        let cell_vals = |idx: f32, idy: f32| -> [f32; 6] {
            let rb = cell_bright(time * speed, idx, idy);
            let cs = hash2(idx, idy) * 0.1;
            // 0.6 + 0.5*cos(time + id.xyx*0.1 + vec3(4,2,1) + colorShift)
            let cr = 0.6 + 0.5 * (anim_time + idx * 0.1 + 4.0 + cs).cos();
            let cg = 0.6 + 0.5 * (anim_time + idy * 0.1 + 2.0 + cs).cos();
            let cb = 0.6 + 0.5 * (anim_time + idx * 0.1 + 1.0 + cs).cos();
            let ldm = (cell_bright(time * speed, idx - 1.0, idy) - rb).min(0.0);
            let tdm = (cell_bright(time * speed, idx, idy + 1.0) - rb).min(0.0);
            [1.0 - rb * 0.2, cr, cg, cb, ldm, tdm]
        };

        let (ox1, oy1) = lo_to_out(iw - 1, ih - 1, scale_f);
        let uv0 = lo_to_out(0, 0, scale_f);
        let (ex0, ex1) = (
            (uv0.0 / mx * size).floor() as i32,
            (ox1 / mx * size).floor() as i32,
        );
        let (ey0, ey1) = (
            (uv0.1 / mx * size).floor() as i32,
            (oy1 / mx * size).floor() as i32,
        );
        let idx_min = ex0.min(ex1);
        let idy_min = ey0.min(ey1);
        let gw = (ex0.max(ex1) - idx_min + 1) as usize;
        let gh = (ey0.max(ey1) - idy_min + 1) as usize;
        let use_grid = gw.saturating_mul(gh) <= (1 << 20);
        let mut cells = vec![[0.0f32; 6]; if use_grid { gw * gh } else { 0 }];
        if use_grid {
            for gy in 0..gh {
                for gx in 0..gw {
                    cells[gy * gw + gx] =
                        cell_vals((idx_min + gx as i32) as f32, (idy_min + gy as i32) as f32);
                }
            }
        }

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            for (ix, px) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let uvx = ox / mx * size;
                let uvy = oy / mx * size;

                let idx = uvx.floor();
                let idy = uvy.floor();
                let gvx = fract(uvx) - 0.5;
                let gvy = fract(uvy) - 0.5;

                let [dim, cr, cg, cb, ldm, tdm] = if use_grid {
                    let gx = (idx as i32 - idx_min) as usize;
                    let gy = (idy as i32 - idy_min) as usize;
                    cells[gy * gw + gx]
                } else {
                    cell_vals(idx, idy)
                };

                // Shadows.
                let s1 = smoothstep(0.0, 0.7, gvx * ldm);
                let s2 = smoothstep(0.0, 0.7, -gvy * tdm);
                let shadow = (s1 + s2) * 0.4;

                let r = ((cr - shadow) * dim).clamp(0.0, 1.0);
                let g = ((cg - shadow) * dim).clamp(0.0, 1.0);
                let b = ((cb - shadow) * dim).clamp(0.0, 1.0);

                *px = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        });

        self.upscale(iw, ih)
    }
}

/// Cell brightness for city_lights.
fn cell_bright(time: f32, x: f32, y: f32) -> f32 {
    ((time + 2.0) * hash2(x, y) * 2.0).sin() * 0.5 + 0.5
}

/// Voronoi `ran` helper — pseudo-random 2D → 2D.
///
/// Uses a single-sin hash per component (same pattern as `hash2`) instead
/// of the original cos/tan chain, which is ~3× cheaper on software CPUs.
fn voronoi_ran(x: f32, y: f32) -> (f32, f32) {
    let fx = fract((x * 127.1 + y * 311.7).sin() * 43758.547);
    let fy = fract((x * 269.5 + y * 183.3).sin() * 43758.547);
    (fx, fy)
}

/// Voronoi `pt` helper — animated point offset.
fn voronoi_pt(t: f32, id_x: f32, id_y: f32) -> (f32, f32) {
    let (rx, ry) = voronoi_ran(id_x + 0.5, id_y + 0.5);
    let (r2x, r2y) = voronoi_ran(id_x - 20.1, id_y - 20.1);
    let px = (t * (rx - 0.5) + r2x * 8.0).sin() * 0.5;
    let py = (t * (ry - 0.5) + r2y * 8.0).sin() * 0.5;
    (px, py)
}

/// Cells per screen width: a non-positive (or NaN) `size` would divide
/// by zero, so it falls back to the default -- the GLSL does the same.
fn cell_size(size: f32) -> f32 {
    if size > 0.0 { size } else { 30.0 }
}
