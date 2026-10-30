//! Procedural shaders: starfield, plasma and matrix rain.

use super::*;

impl SoftwareShaderRenderer {
    /// Starfield with twinkling multi-layer stars.
    pub(super) fn render_starfield(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::Starfield;
        let speed = sh.float(params, "speed");
        let t = time * speed;
        let star_col = sh.color(params, 0);
        let bg_col = sh.color(params, 1);

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
    pub(super) fn render_plasma(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::Plasma;
        let speed = sh.float(params, "speed");
        let t = time * speed;
        let c1 = sh.color(params, 0);
        let c2 = sh.color(params, 1);
        let c3 = sh.color(params, 2);

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
    pub(super) fn render_matrix_rain(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::MatrixRain;
        let speed = sh.float(params, "speed");
        let t = time * speed;
        let rain_col = sh.color(params, 0);
        let bg_col = sh.color(params, 1);

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

            let fall = rain_fall(t * col_speed + col_offset, cell_y);

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
}

/// Matrix rain: distance in cells (within one 40-cell cycle) from a
/// column's falling head *up* to the cell at `cell_y`. The head sits where
/// this is 0; `progress` (time x column speed + column offset) grows, so the
/// head moves toward larger `cell_y` -- down the screen -- with its trail
/// (small positive values) above it. It used to be `cell_y + progress`,
/// which moved the head up: the rain rose. `rem_euclid` keeps the result
/// in `0..40` like GLSL `mod`.
pub(super) fn rain_fall(progress: f32, cell_y: f32) -> f32 {
    (progress - cell_y).rem_euclid(40.0)
}
