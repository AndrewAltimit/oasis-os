//! Balatro swirl (port of shadertoy.com/view/XXtBRr).

use super::*;

impl SoftwareShaderRenderer {
    /// Render the Balatro shader to the pixel buffer. Returns RGBA slice.
    ///
    /// Mirrors the GLSL `balatro.frag` logic: polar-coordinate twist,
    /// 5-iteration distortion, distance-based 3-colour weighting.
    ///
    /// Renders at reduced internal resolution and upscales for performance.
    pub(super) fn render_balatro(&mut self, time: f32, params: &ShaderParams) -> &[u8] {
        let sh = BuiltinShader::Balatro;
        let c1 = sh.color(params, 0);
        let c2 = sh.color(params, 1);
        let c3 = sh.color(params, 2);

        let speed = sh.float(params, "speed");
        let contrast = sh.float(params, "contrast");
        let spin_speed = sh.float(params, "spin_speed");
        let spin_amount = sh.float(params, "spin_amount");
        let pixel_filter = sh.float(params, "pixel_filter");
        let is_rotate = sh.float(params, "is_rotate") > 0.5;
        let lighting = sh.float(params, "lighting");
        let spin_ease = sh.float(params, "spin_ease");

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
}
