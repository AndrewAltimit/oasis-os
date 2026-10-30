//! Wave shaders: ocean waves and calm waves.

use super::*;

impl SoftwareShaderRenderer {
    /// Ocean waves (port of shadertoy.com/view/33t3WB).
    pub(super) fn render_ocean_waves(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::OceanWaves;
        let speed = sh.float(params, "speed");
        let grad_top = sh.color(params, 0);
        let grad_bottom = sh.color(params, 1);
        let wave_col = sh.color(params, 2);

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
            let base_r = lerp(grad_top[0], grad_bottom[0], bg_t);
            let base_g = lerp(grad_top[1], grad_bottom[1], bg_t);
            let base_b = lerp(grad_top[2], grad_bottom[2], bg_t);
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
    pub(super) fn render_calm_waves(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::CalmWaves;
        let speed = sh.float(params, "speed");
        let t = time * speed;
        // Default sky-blue (102, 204, 255).
        let bg = sh.color(params, 0);

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
