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

/// Downscale factor for internal rendering. Render at 1/SCALE in each
/// dimension, then nearest-neighbour upscale. 3 = 9x fewer pixels.
const RENDER_SCALE: u32 = 3;

/// CPU-based shader renderer.
///
/// Renders at reduced internal resolution (`1/RENDER_SCALE` in each dimension)
/// and upscales to the output buffer with nearest-neighbour for performance.
pub struct SoftwareShaderRenderer {
    width: u32,
    height: u32,
    pixel_buf: Vec<u8>,
    lo_buf: Vec<[u8; 4]>,
}

impl SoftwareShaderRenderer {
    /// Create a new software renderer at the given output resolution.
    pub fn new(width: u32, height: u32) -> Self {
        let iw = width.div_ceil(RENDER_SCALE);
        let ih = height.div_ceil(RENDER_SCALE);
        Self {
            width,
            height,
            pixel_buf: vec![0u8; (width * height * 4) as usize],
            lo_buf: vec![[0u8; 4]; (iw * ih) as usize],
        }
    }

    /// Resize the output buffer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixel_buf.resize((width * height * 4) as usize, 0);
        let iw = width.div_ceil(RENDER_SCALE);
        let ih = height.div_ceil(RENDER_SCALE);
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);
    }

    /// Render a named shader. Dispatches to the appropriate implementation.
    pub fn render_shader(&mut self, name: &str, time: f32, params: &ShaderParams) -> &[u8] {
        match name {
            "voronoi" => self.render_voronoi(time, params),
            "city_lights" => self.render_city_lights(time, params),
            "ocean_waves" => self.render_ocean_waves(time, params),
            "calm_waves" => self.render_calm_waves(time, params),
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

        let iw = self.width.div_ceil(RENDER_SCALE);
        let ih = self.height.div_ceil(RENDER_SCALE);
        let scale_f = RENDER_SCALE as f32;
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        for iy in 0..ih {
            for ix in 0..iw {
                let (ox, oy) = lo_to_out(ix, iy, scale_f);
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

                self.lo_buf[(iy * iw + ix) as usize] =
                    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        }

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

        for iy in 0..ih {
            for ix in 0..iw {
                let (ox, oy) = lo_to_out(ix, iy, scale_f);
                let mut uvx = (ox - 0.5 * w) / w + off_x;
                let mut uvy = (oy - 0.5 * h) / w + off_y;
                uvx *= size;
                uvy *= size;

                let gvx = fract(uvx) - 0.5;
                let gvy = fract(uvy) - 0.5;
                let idx = uvx.floor();
                let idy = uvy.floor();

                let mut mindist: f32 = 1e9;
                let mut vorv_x: f32 = 0.0;
                let mut vorv_y: f32 = 0.0;

                for i in -1..=1 {
                    for j in -1..=1 {
                        let fi = i as f32;
                        let fj = j as f32;
                        let nid_x = idx + fi;
                        let nid_y = idy + fj;
                        let (px, py) = voronoi_pt(t, nid_x, nid_y);
                        let dx = gvx + px - fi;
                        let dy = gvy + py - fj;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < mindist {
                            mindist = dist;
                            vorv_x = (idx + px + fi) / size - off_x;
                            vorv_y = (idy + py + fj) / size - off_y;
                        }
                    }
                }

                let blend = (vorv_x * 2.2 + vorv_y).clamp(-1.0, 1.0) * 0.5 + 0.5;
                let r = lerp(col1[0], col2[0], blend).clamp(0.0, 1.0);
                let g = lerp(col1[1], col2[1], blend).clamp(0.0, 1.0);
                let b = lerp(col1[2], col2[2], blend).clamp(0.0, 1.0);

                self.lo_buf[(iy * iw + ix) as usize] =
                    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        }

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

        for iy in 0..ih {
            for ix in 0..iw {
                let (ox, oy) = lo_to_out(ix, iy, scale_f);
                let uvx = ox / mx * size;
                let uvy = oy / mx * size;

                let idx = uvx.floor();
                let idy = uvy.floor();
                let gvx = fract(uvx) - 0.5;
                let gvy = fract(uvy) - 0.5;

                let rb = cell_bright(time * speed, idx, idy);
                let cs = hash2(idx, idy) * 0.1;

                // 0.6 + 0.5*cos(time + id.xyx*0.1 + vec3(4,2,1) + colorShift)
                let cr = 0.6 + 0.5 * (anim_time + idx * 0.1 + 4.0 + cs).cos();
                let cg = 0.6 + 0.5 * (anim_time + idy * 0.1 + 2.0 + cs).cos();
                let cb = 0.6 + 0.5 * (anim_time + idx * 0.1 + 1.0 + cs).cos();

                // Shadows.
                let left_diff = cell_bright(time * speed, idx - 1.0, idy) - rb;
                let top_diff = cell_bright(time * speed, idx, idy + 1.0) - rb;
                let s1 = smoothstep(0.0, 0.7, gvx * left_diff.min(0.0));
                let s2 = smoothstep(0.0, 0.7, -gvy * top_diff.min(0.0));
                let shadow = (s1 + s2) * 0.4;

                let dim = 1.0 - rb * 0.2;
                let r = ((cr - shadow) * dim).clamp(0.0, 1.0);
                let g = ((cg - shadow) * dim).clamp(0.0, 1.0);
                let b = ((cb - shadow) * dim).clamp(0.0, 1.0);

                self.lo_buf[(iy * iw + ix) as usize] =
                    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        }

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

        for iy in 0..ih {
            for ix in 0..iw {
                let (ox, oy) = lo_to_out(ix, iy, scale_f);
                let uv_x = 2.0 * (2.0 * ox - self.width as f32) / res_y;
                let uv_y = 2.0 * (2.0 * oy - h) / res_y;

                let bg_t = oy / h;
                let mut r = lerp(bg_bot[0], bg_top[0], bg_t);
                let mut g = lerp(bg_bot[1], bg_top[1], bg_t);
                let mut b = lerp(bg_bot[2], bg_top[2], bg_t);

                let mut f: f32 = 0.0;
                f += ocean_wave(
                    t,
                    uv_x,
                    uv_y,
                    [0.1, 0.2, 0.3, 0.4],
                    [0.1, 0.4, 0.8, 0.3],
                    [PI, 1.5 * PI, 2.0 * PI, 2.5 * PI],
                );
                f += ocean_wave(
                    t,
                    uv_x,
                    uv_y,
                    [0.1, 0.3, 0.4, 0.1],
                    [0.8, 0.5, 0.4, 0.3],
                    [5.0, 2.0, 1.0, 3.0],
                );
                f += ocean_wave(
                    t,
                    uv_x,
                    uv_y,
                    [0.3, 0.2, 0.1, 0.2],
                    [0.9, 0.5, 0.1, 0.1],
                    [1.0, 2.0, 2.0, 3.0],
                );

                r = (r + f * wave_col[0]).clamp(0.0, 1.0);
                g = (g + f * wave_col[1]).clamp(0.0, 1.0);
                b = (b + f * wave_col[2]).clamp(0.0, 1.0);

                self.lo_buf[(iy * iw + ix) as usize] =
                    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255];
            }
        }

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

        for iy in 0..ih {
            for ix in 0..iw {
                let (ox, oy) = lo_to_out(ix, iy, scale_f);
                let xy_x = ox / w;
                let xy_y = oy / h;

                let mut r = bg[0] - 0.1 * xy_y;
                let mut g = bg[1] - 0.15 * xy_y;
                let mut b = bg[2] - 0.05 * xy_y;

                // First wave (cosine).
                let w1_val =
                    ((xy_x + t.cos() / wave_diff + t * 0.05) * wave_width).cos() / wave_height;
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
                let w2_val =
                    ((xy_x + t.sin() / wave_diff + t * 0.05) * wave_width).sin() / wave_height;
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

                self.lo_buf[(iy * iw + ix) as usize] = [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ];
            }
        }

        self.upscale(iw, ih)
    }

    // -- helpers --

    fn lo_dims(&self) -> (u32, u32, f32) {
        (
            self.width.div_ceil(RENDER_SCALE),
            self.height.div_ceil(RENDER_SCALE),
            RENDER_SCALE as f32,
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
        );
        &self.pixel_buf
    }
}

// -- Free helper functions --

/// Nearest-neighbour upscale from low-res buffer to output RGBA buffer.
fn upscale_nn(lo_buf: &[[u8; 4]], pixel_buf: &mut [u8], width: u32, height: u32, iw: u32, ih: u32) {
    for y in 0..height {
        let sy = (y / RENDER_SCALE).min(ih - 1);
        let src_row = (sy * iw) as usize;
        let dst_row = (y * width * 4) as usize;
        for x in 0..width {
            let sx = (x / RENDER_SCALE).min(iw - 1) as usize;
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
fn voronoi_ran(x: f32, y: f32) -> (f32, f32) {
    let ax = x * (x * 127.1 + y * 311.7);
    let ay = x * (x * 227.1 + y * 521.7);
    let fx =
        1.0 - fract((ax.cos() * 123.6).tan() * 3533.3) * fract((ax.cos() * 123.6).tan() * 3533.3);
    let fy =
        1.0 - fract((ay.cos() * 123.6).tan() * 3533.3) * fract((ay.cos() * 123.6).tan() * 3533.3);
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

/// Single ocean wave layer evaluation.
fn ocean_wave(
    time: f32,
    uv_x: f32,
    uv_y: f32,
    amps: [f32; 4],
    freqs: [f32; 4],
    offsets: [f32; 4],
) -> f32 {
    let mut y: f32 = 0.0;
    for i in 0..4 {
        y += amps[i] * (freqs[i] * uv_x + time + offsets[i]).sin();
    }
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
        ] {
            let pixels = r.render_shader(name, 0.5, &params);
            assert_eq!(pixels.len(), 8 * 8 * 4);
        }
    }
}
