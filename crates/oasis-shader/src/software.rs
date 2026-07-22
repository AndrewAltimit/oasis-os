//! CPU software fallback renderer.
//!
//! Evaluates shader math per-pixel in Rust. Used for UE5 fallback or
//! testing without GPU. Renders at configurable resolution (default:
//! half-res for performance), upscaled when blitted.

use crate::uniforms::ShaderParams;

const PI: f32 = std::f32::consts::PI;

/// Default Balatro colors (matching the fragment shader defaults).
const DEFAULT_COLOR1: [f32; 4] = [0.871, 0.267, 0.251, 1.0]; // #DE4440
const DEFAULT_COLOR2: [f32; 4] = [0.0, 0.420, 0.706, 1.0]; // #006BB4
const DEFAULT_COLOR3: [f32; 4] = [0.086, 0.137, 0.145, 1.0]; // #162325

/// Minimum downscale factor for internal rendering. Render at 1/SCALE in
/// each dimension, then nearest-neighbour upscale. 3 = 9x fewer pixels.
const RENDER_SCALE: u32 = 3;

/// Cap on the low-res buffer dimensions. The effective scale grows with
/// the output resolution so the number of per-pixel kernel evaluations
/// stays roughly constant (~130k cells) regardless of window size — a 4K
/// canvas must not cost 9x a 720p one. At or below 1440x810 the minimum
/// [`RENDER_SCALE`] of 3 already satisfies the cap, so smaller outputs
/// (including the golden-checksum test sizes) are unaffected.
const MAX_LO_W: u32 = 480;
const MAX_LO_H: u32 = 270;

/// Effective downscale factor for a given output resolution: the smallest
/// factor >= [`RENDER_SCALE`] that keeps the low-res buffer within
/// [`MAX_LO_W`] x [`MAX_LO_H`].
fn render_scale_for(width: u32, height: u32) -> u32 {
    RENDER_SCALE
        .max(width.div_ceil(MAX_LO_W))
        .max(height.div_ceil(MAX_LO_H))
}

/// CPU-based shader renderer.
///
/// Renders at reduced internal resolution (`1/scale` in each dimension;
/// at least 1/3, growing with the output size so the internal buffer
/// stays within 480x270 cells) and upscales to the output buffer with
/// nearest-neighbour for performance.
pub struct SoftwareShaderRenderer {
    width: u32,
    height: u32,
    /// Effective downscale factor for the current output resolution.
    scale: u32,
    pixel_buf: Vec<u8>,
    lo_buf: Vec<[u8; 4]>,
}

impl SoftwareShaderRenderer {
    /// Create a new software renderer at the given output resolution.
    pub fn new(width: u32, height: u32) -> Self {
        let scale = render_scale_for(width, height);
        let iw = width.div_ceil(scale);
        let ih = height.div_ceil(scale);
        Self {
            width,
            height,
            scale,
            pixel_buf: vec![0u8; (width * height * 4) as usize],
            lo_buf: vec![[0u8; 4]; (iw * ih) as usize],
        }
    }

    /// Resize the output buffer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.scale = render_scale_for(width, height);
        self.pixel_buf.resize((width * height * 4) as usize, 0);
        let iw = width.div_ceil(self.scale);
        let ih = height.div_ceil(self.scale);
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);
    }

    /// Render a named shader. Dispatches to the appropriate implementation.
    pub fn render_shader(&mut self, name: &str, time: f32, params: &ShaderParams) -> &[u8] {
        match name {
            "voronoi" => self.render_voronoi(time, params),
            "city_lights" => self.render_city_lights(time, params),
            "ocean_waves" => self.render_ocean_waves(time, params),
            "calm_waves" => self.render_calm_waves(time, params),
            "starfield" => self.render_starfield(time, params),
            "plasma" => self.render_plasma(time, params),
            "matrix_rain" => self.render_matrix_rain(time, params),
            _ => self.render_balatro(time, params),
        }
    }

    /// Render the Balatro shader to the pixel buffer. Returns RGBA slice.
    ///
    /// Mirrors the GLSL `balatro.frag` logic: polar-coordinate twist,
    /// 5-iteration distortion, distance-based 3-colour weighting.
    ///
    /// Renders at reduced internal resolution and upscales for performance.
    pub fn render_balatro(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let c1 = params.colors.first().copied().unwrap_or(DEFAULT_COLOR1);
        let c2 = params.colors.get(1).copied().unwrap_or(DEFAULT_COLOR2);
        let c3 = params.colors.get(2).copied().unwrap_or(DEFAULT_COLOR3);

        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let contrast = *params.floats.get("contrast").unwrap_or(&3.5);
        let spin_speed = *params.floats.get("spin_speed").unwrap_or(&1.0);
        let spin_amount = *params.floats.get("spin_amount").unwrap_or(&0.25);
        let pixel_filter = *params.floats.get("pixel_filter").unwrap_or(&745.0);
        let is_rotate = *params.floats.get("is_rotate").unwrap_or(&0.0) > 0.5;
        let lighting = *params.floats.get("lighting").unwrap_or(&0.4);
        let spin_ease = *params.floats.get("spin_ease").unwrap_or(&1.0);

        let w = self.width as f32;
        let h = self.height as f32;
        let res_len = (w * w + h * h).sqrt();
        let pixel_size = res_len / pixel_filter;

        let rot = if is_rotate {
            time * spin_speed * (2.0 * PI / 60.0)
        } else {
            1.0
        };

        let phase = time * speed * (2.0 * PI / 10.0);
        let contrast_mod = 0.25 * contrast + 0.5 * spin_amount + 1.2;
        let base_w = 0.3 / contrast;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row_par(lo, iw, |iy, row| {
            for (ix, px) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let qx = (ox / pixel_size).floor() * pixel_size;
                let qy = (oy / pixel_size).floor() * pixel_size;
                let mut ux = (qx - 0.5 * w) / res_len;
                let mut uy = (qy - 0.5 * h) / res_len;
                let uv_len = (ux * ux + uy * uy).sqrt();

                let angle = uy.atan2(ux) + rot
                    - spin_ease * 20.0 * (spin_amount * uv_len + (1.0 - spin_amount));
                ux = uv_len * angle.cos();
                uy = uv_len * angle.sin();
                ux *= 30.0;
                uy *= 30.0;

                let mut uv2x = ux + uy;
                let mut uv2y = ux + uy;
                for _ in 0..5 {
                    let mx = ux.max(uy);
                    let s = mx.sin();
                    uv2x += s + ux;
                    uv2y += s + uy;
                    ux += 0.5 * (5.1123314 + 0.353 * uv2y + phase).cos();
                    uy += 0.5 * (uv2x - phase).sin();
                    let cxy = (ux + uy).cos();
                    let sxy = (ux * 0.711 - uy).sin();
                    let d = cxy - sxy;
                    ux -= d;
                    uy -= d;
                }

                let dist = (ux * ux + uy * uy).sqrt();
                let paint_res = (dist * 0.035 * contrast_mod).clamp(0.0, 2.0);
                let c1p = (1.0 - contrast_mod * (1.0 - paint_res).abs()).max(0.0);
                let c2p = (1.0 - contrast_mod * paint_res.abs()).max(0.0);
                let c3p = 1.0 - (c1p + c2p).min(1.0);

                let light = (lighting - 0.2) * (c1p * 5.0 - 4.0).max(0.0)
                    + lighting * (c2p * 5.0 - 4.0).max(0.0);

                let blend = 1.0 - base_w;
                let r =
                    (base_w * c1[0] + blend * (c1[0] * c1p + c2[0] * c2p + c3[0] * c3p) + light)
                        .clamp(0.0, 1.0);
                let g =
                    (base_w * c1[1] + blend * (c1[1] * c1p + c2[1] * c2p + c3[1] * c3p) + light)
                        .clamp(0.0, 1.0);
                let b =
                    (base_w * c1[2] + blend * (c1[2] * c1p + c2[2] * c2p + c3[2] * c3p) + light)
                        .clamp(0.0, 1.0);

                *px = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        });

        self.upscale(iw, ih)
    }

    /// Voronoi cell pattern (port of shadertoy.com/view/WdlyRS).
    fn render_voronoi(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let size = *params.floats.get("size").unwrap_or(&30.0);
        let col1 = params.colors.first().copied().unwrap_or([
            193.0 / 255.0,
            41.0 / 255.0,
            46.0 / 255.0,
            1.0,
        ]);
        let col2 = params.colors.get(1).copied().unwrap_or([
            241.0 / 255.0,
            211.0 / 255.0,
            2.0 / 255.0,
            1.0,
        ]);

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
    fn render_city_lights(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let size = *params.floats.get("size").unwrap_or(&30.0);
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

    /// Ocean waves (port of shadertoy.com/view/33t3WB).
    fn render_ocean_waves(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let bg_bot = params
            .colors
            .first()
            .copied()
            .unwrap_or([0.0, 0.05, 0.4, 1.0]);
        let bg_top = params
            .colors
            .get(1)
            .copied()
            .unwrap_or([0.0, 0.9, 0.9, 1.0]);
        let wave_col = params
            .colors
            .get(2)
            .copied()
            .unwrap_or([0.6, 0.6, 0.6, 1.0]);

        let t = time * speed;
        let h = self.height as f32;
        let res_y = h;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        // Each wave's height is a function of the column (uv_x) and time
        // only — hoist the 12 sines per pixel to 12 per column.
        let w_f = self.width as f32;
        let heights: Vec<[f32; 3]> = (0..iw)
            .map(|ix| {
                let ox = lo_to_out(ix, 0, scale_f).0;
                let uv_x = 2.0 * (2.0 * ox - w_f) / res_y;
                [
                    ocean_wave_height(
                        t,
                        uv_x,
                        [0.1, 0.2, 0.3, 0.4],
                        [0.1, 0.4, 0.8, 0.3],
                        [PI, 1.5 * PI, 2.0 * PI, 2.5 * PI],
                    ),
                    ocean_wave_height(
                        t,
                        uv_x,
                        [0.1, 0.3, 0.4, 0.1],
                        [0.8, 0.5, 0.4, 0.3],
                        [5.0, 2.0, 1.0, 3.0],
                    ),
                    ocean_wave_height(
                        t,
                        uv_x,
                        [0.3, 0.2, 0.1, 0.2],
                        [0.9, 0.5, 0.1, 0.1],
                        [1.0, 2.0, 2.0, 3.0],
                    ),
                ]
            })
            .collect();

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            let oy = lo_to_out(0, iy, scale_f).1;
            let uv_y = 2.0 * (2.0 * oy - h) / res_y;
            let bg_t = oy / h;
            let base_r = lerp(bg_bot[0], bg_top[0], bg_t);
            let base_g = lerp(bg_bot[1], bg_top[1], bg_t);
            let base_b = lerp(bg_bot[2], bg_top[2], bg_t);
            for (ix, px) in row.iter_mut().enumerate() {
                let hs = heights[ix];
                let mut f: f32 = 0.0;
                f += ocean_wave_shade(hs[0], uv_y);
                f += ocean_wave_shade(hs[1], uv_y);
                f += ocean_wave_shade(hs[2], uv_y);

                let r = (base_r + f * wave_col[0]).clamp(0.0, 1.0);
                let g = (base_g + f * wave_col[1]).clamp(0.0, 1.0);
                let b = (base_b + f * wave_col[2]).clamp(0.0, 1.0);

                *px = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        });

        self.upscale(iw, ih)
    }

    /// Calm waves (port of shadertoy.com/view/3fBBDc).
    fn render_calm_waves(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let t = time * speed;
        // Default sky-blue (102, 204, 255).
        let bg = params
            .colors
            .first()
            .copied()
            .unwrap_or([102.0 / 255.0, 204.0 / 255.0, 1.0, 1.0]);

        let w = self.width as f32;
        let h = self.height as f32;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        let edge_blur: f32 = 0.007;
        let wave_diff: f32 = 100.0;
        let wave_width: f32 = 5.0;
        let wave_height: f32 = 10.0;
        let col_diff: [f32; 3] = [0.1, 0.05, 0.1];

        // t.cos()/t.sin() are loop constants; the wave values depend
        // only on the column.
        let (t_cos, t_sin) = (t.cos(), t.sin());
        let waves: Vec<(f32, f32)> = (0..iw)
            .map(|ix| {
                let xy_x = lo_to_out(ix, 0, scale_f).0 / w;
                let w1 = ((xy_x + t_cos / wave_diff + t * 0.05) * wave_width).cos() / wave_height;
                let w2 = ((xy_x + t_sin / wave_diff + t * 0.05) * wave_width).sin() / wave_height;
                (w1, w2)
            })
            .collect();

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            let xy_y = lo_to_out(0, iy, scale_f).1 / h;
            for (ix, px) in row.iter_mut().enumerate() {
                let (w1_val, w2_val) = waves[ix];

                let mut r = bg[0] - 0.1 * xy_y;
                let mut g = bg[1] - 0.15 * xy_y;
                let mut b = bg[2] - 0.05 * xy_y;

                // First wave (cosine).
                let w1_line = w1_val + 0.35;
                let w1b = w1_val + 0.25;

                if xy_y <= w1_line {
                    r += col_diff[0];
                    g += col_diff[1];
                    b += col_diff[2];
                } else if (w1b - xy_y).abs() < edge_blur {
                    let factor = (edge_blur + (w1b - xy_y)) / edge_blur;
                    r += col_diff[0] * factor;
                    g += col_diff[1] * factor;
                    b += col_diff[2] * factor;
                }

                // Second wave (sine).
                let w2_line = w2_val + 0.5;
                let w2b = w2_val + 0.25;

                if xy_y <= w2_line {
                    r += col_diff[0];
                    g += col_diff[1];
                    b += col_diff[2];
                } else if (w2b - xy_y).abs() < edge_blur {
                    let factor = (edge_blur + (w2b - xy_y)) / edge_blur;
                    r += col_diff[0] * factor;
                    g += col_diff[1] * factor;
                    b += col_diff[2] * factor;
                }

                *px = [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ];
            }
        });

        self.upscale(iw, ih)
    }

    /// Starfield with twinkling multi-layer stars.
    fn render_starfield(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let t = time * speed;
        let star_col = params
            .colors
            .first()
            .copied()
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let bg_col = params
            .colors
            .get(1)
            .copied()
            .unwrap_or([0.0, 0.0, 0.05, 1.0]);

        let w = self.width as f32;
        let h = self.height as f32;
        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row_par(lo, iw, |iy, row| {
            for (ix, px) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let uv_x = (ox - 0.5 * w) / h;
                let uv_y = (oy - 0.5 * h) / h;

                let mut r = bg_col[0];
                let mut g = bg_col[1];
                let mut b = bg_col[2];

                for layer in 0..4 {
                    let depth = 1.0 + layer as f32 * 0.5;
                    let scale = 20.0 * depth;
                    let fade = 1.0 / depth;

                    let st_x = uv_x * scale + t * 0.1 * depth;
                    let st_y = uv_y * scale + t * 0.05;
                    let cell_x = st_x.floor();
                    let cell_y = st_y.floor();
                    let f_x = st_x - cell_x - 0.5;
                    let f_y = st_y - cell_y - 0.5;

                    let layer_off = layer as f32 * 100.0;
                    let h_val = hash2(cell_x + layer_off, cell_y + layer_off);
                    let ox_off = h_val - 0.5;
                    let oy_off = hash2(cell_x + 0.5, cell_y + 0.5) - 0.5;
                    let dx = f_x - ox_off * 0.6;
                    let dy = f_y - oy_off * 0.6;
                    let d = (dx * dx + dy * dy).sqrt();

                    let twinkle = 0.5 + 0.5 * (t * 3.0 + h_val * std::f32::consts::TAU).sin();
                    let brightness = smoothstep(0.05, 0.0, d) * twinkle * fade * h_val;

                    r += star_col[0] * brightness;
                    g += star_col[1] * brightness;
                    b += star_col[2] * brightness;
                }

                *px = [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ];
            }
        });

        self.upscale(iw, ih)
    }

    /// Classic plasma effect with overlapping sine waves.
    fn render_plasma(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let t = time * speed;
        let c1 = params
            .colors
            .first()
            .copied()
            .unwrap_or([1.0, 0.0, 0.5, 1.0]);
        let c2 = params
            .colors
            .get(1)
            .copied()
            .unwrap_or([0.0, 0.8, 1.0, 1.0]);
        let c3 = params
            .colors
            .get(2)
            .copied()
            .unwrap_or([1.0, 0.9, 0.0, 1.0]);

        let w = self.width as f32;
        let h = self.height as f32;
        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        // Time-only trig and the column-only v1 term are loop constants.
        let (ts05, tc03) = ((t * 0.5).sin(), (t * 0.3).cos());
        let v1s: Vec<f32> = (0..iw)
            .map(|ix| {
                let uv_x = lo_to_out(ix, 0, scale_f).0 / w;
                (uv_x * 10.0 + t).sin()
            })
            .collect();

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            for (ix, px) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let uv_x = ox / w;
                let uv_y = oy / h;

                let v1 = v1s[ix];
                let v2 = (10.0 * (uv_x * ts05 + uv_y * tc03) + t).sin();
                let dx1 = uv_x - 0.5;
                let dy1 = uv_y - 0.5;
                let v3 = (((dx1 * dx1 + dy1 * dy1) * 100.0 + 1.0).sqrt() + t).sin();
                let dx2 = uv_x - 0.3;
                let dy2 = uv_y - 0.7;
                let v4 = (((dx2 * dx2 + dy2 * dy2) * 100.0 + 1.0).sqrt() + t * 0.7).sin();

                let v = (v1 + v2 + v3 + v4) * 0.25;
                let val = v * 0.5 + 0.5;

                let (r, g, b) = if val < 0.33 {
                    let f = val * 3.0;
                    (
                        lerp(c1[0], c2[0], f),
                        lerp(c1[1], c2[1], f),
                        lerp(c1[2], c2[2], f),
                    )
                } else if val < 0.66 {
                    let f = (val - 0.33) * 3.0;
                    (
                        lerp(c2[0], c3[0], f),
                        lerp(c2[1], c3[1], f),
                        lerp(c2[2], c3[2], f),
                    )
                } else {
                    let f = (val - 0.66) * 3.0;
                    (
                        lerp(c3[0], c1[0], f),
                        lerp(c3[1], c1[1], f),
                        lerp(c3[2], c1[2], f),
                    )
                };

                *px = [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ];
            }
        });

        self.upscale(iw, ih)
    }

    /// Matrix digital rain effect.
    fn render_matrix_rain(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let speed = *params.floats.get("speed").unwrap_or(&1.0);
        let t = time * speed;
        let rain_col = params
            .colors
            .first()
            .copied()
            .unwrap_or([0.0, 1.0, 0.3, 1.0]);
        let bg_col = params
            .colors
            .get(1)
            .copied()
            .unwrap_or([0.0, 0.02, 0.0, 1.0]);

        let w = self.width as f32;
        let columns = 40.0;
        let cell_w = w / columns;
        let cell_h = cell_w * 1.2;

        let (iw, ih, scale_f) = self.lo_dims();
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        // The pixel color depends only on the (cell_x, cell_y) glyph
        // cell (~40 x ~25 cells) — shade each cell once and fill the
        // pixel grid by lookup.
        let cell_rgba = |cell_x: f32, cell_y: f32| -> [u8; 4] {
            let col_hash = hash2(cell_x, 0.0);
            let col_speed = 1.0 + col_hash * 3.0;
            let col_offset = col_hash * 100.0;

            let fall = (cell_y + t * col_speed + col_offset) % 40.0;

            let intensity = smoothstep(20.0, 0.0, fall) * smoothstep(-1.0, 0.0, fall);
            let char_hash = hash2(cell_x + (t * 4.0).floor(), cell_y + (t * 4.0).floor());
            let intensity = intensity * (0.7 + 0.3 * char_hash);

            let head = smoothstep(1.0, 0.0, fall) * 2.0;

            let r = bg_col[0] + rain_col[0] * intensity + head * 0.5;
            let g = bg_col[1] + rain_col[1] * intensity + head * 0.5;
            let b = bg_col[2] + rain_col[2] * intensity + head * 0.5;
            [
                (r.clamp(0.0, 1.0) * 255.0) as u8,
                (g.clamp(0.0, 1.0) * 255.0) as u8,
                (b.clamp(0.0, 1.0) * 255.0) as u8,
                255,
            ]
        };

        let (ox1, oy1) = lo_to_out(iw - 1, ih - 1, scale_f);
        let (o0x, o0y) = lo_to_out(0, 0, scale_f);
        let (ex0, ex1) = ((o0x / cell_w).floor() as i32, (ox1 / cell_w).floor() as i32);
        let (ey0, ey1) = ((o0y / cell_h).floor() as i32, (oy1 / cell_h).floor() as i32);
        let cx_min = ex0.min(ex1);
        let cy_min = ey0.min(ey1);
        let gw = (ex0.max(ex1) - cx_min + 1) as usize;
        let gh = (ey0.max(ey1) - cy_min + 1) as usize;
        let use_grid = gw.saturating_mul(gh) <= (1 << 20);
        let mut cells = vec![[0u8; 4]; if use_grid { gw * gh } else { 0 }];
        if use_grid {
            for gy in 0..gh {
                for gx in 0..gw {
                    cells[gy * gw + gx] =
                        cell_rgba((cx_min + gx as i32) as f32, (cy_min + gy as i32) as f32);
                }
            }
        }

        let lo = &mut self.lo_buf[..(iw * ih) as usize];
        for_each_row(lo, iw, |iy, row| {
            for (ix, px) in row.iter_mut().enumerate() {
                let (ox, oy) = lo_to_out(ix as u32, iy, scale_f);
                let cell_x = (ox / cell_w).floor();
                let cell_y = (oy / cell_h).floor();
                *px = if use_grid {
                    let gx = (cell_x as i32 - cx_min) as usize;
                    let gy = (cell_y as i32 - cy_min) as usize;
                    cells[gy * gw + gx]
                } else {
                    cell_rgba(cell_x, cell_y)
                };
            }
        });

        self.upscale(iw, ih)
    }

    // -- helpers --

    fn lo_dims(&self) -> (u32, u32, f32) {
        (
            self.width.div_ceil(self.scale),
            self.height.div_ceil(self.scale),
            self.scale as f32,
        )
    }

    fn upscale(&mut self, iw: u32, ih: u32) -> &[u8] {
        upscale_nn(
            &self.lo_buf,
            &mut self.pixel_buf,
            self.width,
            self.height,
            iw,
            ih,
            self.scale,
        );
        &self.pixel_buf
    }
}

// -- Free helper functions --

/// Run `shade` over every low-res row (`lo_buf` must be exactly
/// `iw * ih` cells), serially.
fn for_each_row<F>(lo_buf: &mut [[u8; 4]], iw: u32, shade: F)
where
    F: Fn(u32, &mut [[u8; 4]]) + Send + Sync,
{
    for (iy, row) in lo_buf.chunks_mut(iw as usize).enumerate() {
        shade(iy as u32, row);
    }
}

/// Like [`for_each_row`], but with the `parallel` feature the rows are
/// spread across the rayon pool. Each pixel's math is identical either
/// way, so the output is bit-identical to the serial path (locked by
/// the `golden_output_checksums` test).
///
/// Only worthwhile for shaders with heavy per-pixel transcendental work
/// (balatro's 5-iteration distortion, starfield's 4 hash layers) — for
/// the memoized shaders the pool dispatch costs more than the shading,
/// so they call [`for_each_row`] directly.
fn for_each_row_par<F>(lo_buf: &mut [[u8; 4]], iw: u32, shade: F)
where
    F: Fn(u32, &mut [[u8; 4]]) + Send + Sync,
{
    // Below ~16k cells (e.g. the PSP's 32x32 / 64x64 targets) the pool
    // dispatch overhead exceeds the shading work — stay serial.
    #[cfg(feature = "parallel")]
    if lo_buf.len() >= 16 * 1024 {
        use rayon::prelude::*;
        lo_buf
            .par_chunks_mut(iw as usize)
            .enumerate()
            .for_each(|(iy, row)| shade(iy as u32, row));
        return;
    }
    for_each_row(lo_buf, iw, shade);
}

/// Nearest-neighbour upscale from low-res buffer to output RGBA buffer.
fn upscale_nn(
    lo_buf: &[[u8; 4]],
    pixel_buf: &mut [u8],
    width: u32,
    height: u32,
    iw: u32,
    ih: u32,
    scale: u32,
) {
    for y in 0..height {
        let sy = (y / scale).min(ih - 1);
        let src_row = (sy * iw) as usize;
        let dst_row = (y * width * 4) as usize;
        for x in 0..width {
            let sx = (x / scale).min(iw - 1) as usize;
            let rgba = lo_buf[src_row + sx];
            let idx = dst_row + (x * 4) as usize;
            pixel_buf[idx] = rgba[0];
            pixel_buf[idx + 1] = rgba[1];
            pixel_buf[idx + 2] = rgba[2];
            pixel_buf[idx + 3] = rgba[3];
        }
    }
}

fn lo_to_out(ix: u32, iy: u32, scale: f32) -> (f32, f32) {
    (
        ix as f32 * scale + scale * 0.5,
        iy as f32 * scale + scale * 0.5,
    )
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < 1e-10 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Hash for city_lights (matches GLSL `rand`).
fn hash2(x: f32, y: f32) -> f32 {
    fract((x * 12.9898 + y * 78.233).sin() * 43_758.547)
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

/// Ocean wave layer height at a column (independent of `uv_y`, so it is
/// computed once per column and shared down the rows).
fn ocean_wave_height(
    time: f32,
    uv_x: f32,
    amps: [f32; 4],
    freqs: [f32; 4],
    offsets: [f32; 4],
) -> f32 {
    let mut y: f32 = 0.0;
    for i in 0..4 {
        y += amps[i] * (freqs[i] * uv_x + time + offsets[i]).sin();
    }
    y
}

/// Ocean wave layer shading for a pixel below/above the wave height.
fn ocean_wave_shade(y: f32, uv_y: f32) -> f32 {
    let blur: f32 = 0.025;
    let top = smoothstep(y + blur, y, uv_y);
    let bot = smoothstep(y - 1.0, y, uv_y) * 0.4;
    top * bot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_renderer_produces_pixels() {
        let mut renderer = SoftwareShaderRenderer::new(16, 16);
        let params = ShaderParams::default();
        let pixels = renderer.render_balatro(1.0, &params);
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color, "software renderer should produce colored pixels");
    }

    #[test]
    fn software_renderer_resize() {
        let mut renderer = SoftwareShaderRenderer::new(8, 8);
        assert_eq!(renderer.pixel_buf.len(), 8 * 8 * 4);
        renderer.resize(16, 16);
        assert_eq!(renderer.pixel_buf.len(), 16 * 16 * 4);
    }

    #[test]
    fn render_scale_stays_minimum_up_to_1440x810() {
        // Everything at or below the cap boundary keeps the fixed 1/3
        // scale — including the golden-checksum sizes below, whose
        // outputs must not change.
        assert_eq!(render_scale_for(64, 64), 3);
        assert_eq!(render_scale_for(241, 135), 3);
        assert_eq!(render_scale_for(480, 272), 3);
        assert_eq!(render_scale_for(1280, 720), 3);
        assert_eq!(render_scale_for(1440, 810), 3);
    }

    #[test]
    fn render_scale_caps_lo_buffer_for_large_outputs() {
        // 1080p: scale 4 → 480x270 cells.
        assert_eq!(render_scale_for(1920, 1080), 4);
        // 4K: scale 8 → 480x270 cells, not 9x the 720p cost.
        assert_eq!(render_scale_for(3840, 2160), 8);
        let r = SoftwareShaderRenderer::new(3840, 2160);
        let (iw, ih, scale_f) = r.lo_dims();
        assert_eq!((iw, ih), (480, 270));
        assert_eq!(scale_f, 8.0);
    }

    #[test]
    fn adaptive_scale_resize_updates_lo_dims() {
        let mut r = SoftwareShaderRenderer::new(1280, 720);
        assert_eq!(r.lo_dims().2, 3.0);
        r.resize(2560, 1440);
        let (iw, ih, scale_f) = r.lo_dims();
        assert_eq!(scale_f, 6.0);
        assert!(iw <= 480 && ih <= 270, "lo buffer exceeds cap: {iw}x{ih}");
        // Output buffer still matches the full resolution.
        let pixels = r.render_shader("plasma", 1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 2560 * 1440 * 4);
    }

    #[test]
    fn software_renderer_deterministic() {
        let mut r1 = SoftwareShaderRenderer::new(8, 8);
        let mut r2 = SoftwareShaderRenderer::new(8, 8);
        let params = ShaderParams::default();
        let p1 = r1.render_balatro(0.5, &params).to_vec();
        let p2 = r2.render_balatro(0.5, &params).to_vec();
        assert_eq!(p1, p2, "same inputs should produce same output");
    }

    #[test]
    fn voronoi_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_voronoi(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn city_lights_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_city_lights(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn ocean_waves_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_ocean_waves(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn calm_waves_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_calm_waves(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn starfield_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_starfield(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
    }

    #[test]
    fn plasma_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_plasma(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn matrix_rain_produces_pixels() {
        let mut r = SoftwareShaderRenderer::new(16, 16);
        let pixels = r.render_matrix_rain(1.0, &ShaderParams::default());
        assert_eq!(pixels.len(), 16 * 16 * 4);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(has_color);
    }

    #[test]
    fn render_shader_dispatches_correctly() {
        let mut r = SoftwareShaderRenderer::new(8, 8);
        let params = ShaderParams::default();
        // Each shader should produce some output without panicking.
        for name in &[
            "balatro",
            "voronoi",
            "city_lights",
            "ocean_waves",
            "calm_waves",
            "starfield",
            "plasma",
            "matrix_rain",
        ] {
            let pixels = r.render_shader(name, 0.5, &params);
            assert_eq!(pixels.len(), 8 * 8 * 4);
        }
    }

    // -----------------------------------------------------------------------
    // PSP-specific shader validation
    // -----------------------------------------------------------------------
    // These tests validate shaders at the 64x64 resolution used by the PSP
    // backend, with skin-matched parameters, verifying correctness and
    // performance without requiring the PSP target.

    /// PSP shader skin configurations — mirrors `PspSkinPreset::shader_config()`
    /// from oasis-backend-psp so we can test without the PSP crate dependency.
    fn psp_shader_configs() -> Vec<(&'static str, &'static str, ShaderParams)> {
        use std::collections::HashMap;
        fn hex(r: u8, g: u8, b: u8) -> [f32; 4] {
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        }
        vec![
            (
                "Balatro",
                "balatro",
                ShaderParams {
                    colors: vec![
                        hex(0x00, 0xF0, 0xFF),
                        hex(0x00, 0x6B, 0xB4),
                        hex(0x16, 0x23, 0x25),
                    ],
                    floats: HashMap::from([
                        ("speed".into(), 1.0),
                        ("contrast".into(), 3.5),
                        ("spin_speed".into(), 1.0),
                        ("spin_amount".into(), 0.25),
                        ("pixel_filter".into(), 745.0),
                        ("lighting".into(), 0.4),
                        ("spin_ease".into(), 1.0),
                    ]),
                },
            ),
            (
                "RetroCga",
                "voronoi",
                ShaderParams {
                    colors: vec![hex(0x55, 0xFF, 0x55), hex(0xFF, 0x55, 0xFF)],
                    floats: HashMap::from([("speed".into(), 0.5), ("size".into(), 6.0)]),
                },
            ),
            (
                "Solarized",
                "ocean_waves",
                ShaderParams {
                    colors: vec![
                        hex(0x00, 0x2B, 0x36),
                        hex(0x00, 0xE0, 0xE0),
                        hex(0x58, 0x6E, 0x75),
                    ],
                    floats: HashMap::from([("speed".into(), 0.6)]),
                },
            ),
            (
                "Terminal",
                "matrix_rain",
                ShaderParams {
                    colors: vec![hex(0x00, 0xFF, 0x00), hex(0x00, 0x02, 0x00)],
                    floats: HashMap::from([("speed".into(), 0.8)]),
                },
            ),
            (
                "Altimit",
                "starfield",
                ShaderParams {
                    colors: vec![hex(0x00, 0xCC, 0x88), hex(0x08, 0x08, 0x16)],
                    floats: HashMap::from([("speed".into(), 0.6)]),
                },
            ),
            (
                "Tactical",
                "plasma",
                ShaderParams {
                    colors: vec![
                        hex(0xCC, 0x88, 0x00),
                        hex(0x66, 0x33, 0x00),
                        hex(0x33, 0x1A, 0x00),
                    ],
                    floats: HashMap::from([("speed".into(), 0.4)]),
                },
            ),
        ]
    }

    #[test]
    fn psp_shaders_render_at_64x64() {
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            let pixels = r.render_shader(name, 1.0, &params);
            assert_eq!(
                pixels.len(),
                64 * 64 * 4,
                "{skin} ({name}): wrong buffer size"
            );
        }
    }

    #[test]
    fn psp_shaders_produce_visible_output() {
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            let pixels = r.render_shader(name, 1.0, &params);
            let has_color = pixels
                .chunks(4)
                .any(|px| px[0] > 10 || px[1] > 10 || px[2] > 10);
            assert!(has_color, "{skin} ({name}): all-black output");
        }
    }

    #[test]
    fn psp_shaders_animate_over_time() {
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            let p0 = r.render_shader(name, 0.0, &params).to_vec();
            let p5 = r.render_shader(name, 5.0, &params).to_vec();
            assert_ne!(p0, p5, "{skin} ({name}): no animation between t=0 and t=5");
        }
    }

    #[test]
    fn psp_shaders_deterministic() {
        let params = psp_shader_configs();
        for (skin, name, p) in &params {
            let mut r1 = SoftwareShaderRenderer::new(64, 64);
            let mut r2 = SoftwareShaderRenderer::new(64, 64);
            let a = r1.render_shader(name, 2.5, p).to_vec();
            let b = r2.render_shader(name, 2.5, p).to_vec();
            assert_eq!(a, b, "{skin} ({name}): non-deterministic output");
        }
    }

    #[test]
    fn psp_shaders_no_nan_pixels() {
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            // Test at several time points including edge cases.
            for &t in &[0.0, 0.001, 1.0, 10.0, 100.0] {
                let pixels = r.render_shader(name, t, &params);
                // Alpha should be 255 for all pixels (opaque).
                let all_opaque = pixels.chunks(4).all(|px| px[3] == 255);
                assert!(
                    all_opaque,
                    "{skin} ({name}) t={t}: not all pixels are opaque"
                );
            }
        }
    }

    #[test]
    fn psp_shader_render_performance() {
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            // Warm up.
            let _ = r.render_shader(name, 0.0, &params);

            let start = std::time::Instant::now();
            let frames = 30;
            for i in 0..frames {
                let _ = r.render_shader(name, i as f32 / 30.0, &params);
            }
            let elapsed = start.elapsed();
            let per_frame_us = elapsed.as_micros() / frames;

            // On host, each 64x64 frame should be well under 1ms.
            // PSP is ~10-20x slower, so budget 2ms host = ~20-40ms PSP.
            // At 30fps shader update, 33ms budget per frame.
            assert!(
                per_frame_us < 2000,
                "{skin} ({name}): {per_frame_us}us/frame exceeds 2ms host budget \
                 (~20ms PSP estimate, 33ms budget at 30fps)",
            );
        }
    }

    /// FNV-1a over an RGBA buffer, for pixel-exact golden comparisons.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// Pixel-exact golden checksums, captured before the loop-invariant
    /// hoisting / per-cell memoization / row-parallel refactor. Any
    /// optimization of the shade loops must keep these outputs
    /// bit-identical. 241x135 exercises the `div_ceil` partial edge
    /// cell; 64x64 matches the PSP path. (Values depend on the platform
    /// libm's sin/cos, so this test is gated to the x86_64 targets CI
    /// and dev machines use.)
    #[test]
    #[cfg(target_arch = "x86_64")]
    fn golden_output_checksums() {
        #[rustfmt::skip]
        let golden: &[(u32, u32, &str, f32, u64)] = &[
            (241, 135, "balatro", 0.0, 0xe683273248fbb563),
            (241, 135, "balatro", 1.7, 0x599b552b17fc1aa0),
            (241, 135, "voronoi", 0.0, 0x134812c5bae9ebbb),
            (241, 135, "voronoi", 1.7, 0xb27f2d3a263e33e1),
            (241, 135, "city_lights", 0.0, 0x825ee4062f67f80f),
            (241, 135, "city_lights", 1.7, 0x80ef6e80d0efb168),
            (241, 135, "ocean_waves", 0.0, 0x99f4708b04fc8143),
            (241, 135, "ocean_waves", 1.7, 0xa48f2758a91c1d67),
            (241, 135, "calm_waves", 0.0, 0xd8ad25f237f7b56c),
            (241, 135, "calm_waves", 1.7, 0x4c44798775101aef),
            (241, 135, "starfield", 0.0, 0xc021031ba10c5906),
            (241, 135, "starfield", 1.7, 0x904b735e365f7d00),
            (241, 135, "plasma", 0.0, 0x810a16afc41d5d28),
            (241, 135, "plasma", 1.7, 0xa5399775c2dccdfd),
            (241, 135, "matrix_rain", 0.0, 0xef631f7232e84993),
            (241, 135, "matrix_rain", 1.7, 0x60f5c2b6de9f4dbe),
            (64, 64, "balatro", 0.0, 0x780a6abc3968051a),
            (64, 64, "balatro", 1.7, 0x6fef1da00cbf99e0),
            (64, 64, "voronoi", 0.0, 0x42d9eb449b82e9aa),
            (64, 64, "voronoi", 1.7, 0xff9584adb8fccee3),
            (64, 64, "city_lights", 0.0, 0x454467ba33118c7d),
            (64, 64, "city_lights", 1.7, 0x95a3c1f11c325ef8),
            (64, 64, "ocean_waves", 0.0, 0x14bf42f68ce6d2e7),
            (64, 64, "ocean_waves", 1.7, 0xf33c870531ea0a98),
            (64, 64, "calm_waves", 0.0, 0xb929ccf472509a87),
            (64, 64, "calm_waves", 1.7, 0x786122825e84babd),
            (64, 64, "starfield", 0.0, 0x9d8047079aa4c57b),
            (64, 64, "starfield", 1.7, 0xf8c72c091c63d3ba),
            (64, 64, "plasma", 0.0, 0x65e84b38260695b4),
            (64, 64, "plasma", 1.7, 0x9b6b56f561f587ef),
            (64, 64, "matrix_rain", 0.0, 0xb07189b1c8442261),
            (64, 64, "matrix_rain", 1.7, 0xd75904872f0f1830),
        ];
        for &(w, h, name, t, want) in golden {
            let mut r = SoftwareShaderRenderer::new(w, h);
            let got = fnv1a(r.render_shader(name, t, &ShaderParams::default()));
            assert_eq!(
                got, want,
                "{name} {w}x{h} t={t}: output changed (got {got:#018x})"
            );
        }
    }

    #[test]
    fn psp_shader_multi_frame_transition() {
        // Simulate 5 seconds of shader wallpaper at 30fps (every other
        // frame at 60fps). Verify each frame produces valid output and
        // the overall animation progresses.
        let mut r = SoftwareShaderRenderer::new(64, 64);
        for (skin, name, params) in psp_shader_configs() {
            let mut prev: Option<Vec<u8>> = None;
            let mut change_count = 0u32;

            for frame in (0..300).step_by(2) {
                let time = frame as f32 / 60.0;
                let pixels = r.render_shader(name, time, &params);
                assert_eq!(pixels.len(), 64 * 64 * 4);

                if let Some(ref p) = prev {
                    if pixels != p.as_slice() {
                        change_count += 1;
                    }
                }
                prev = Some(pixels.to_vec());
            }

            // Over 150 shader frames (5s), expect at least 50% to differ
            // from the previous frame (animation is continuous).
            assert!(
                change_count > 75,
                "{skin} ({name}): only {change_count}/150 frames changed \
                 (expected >75 for smooth animation)",
            );
        }
    }
}
