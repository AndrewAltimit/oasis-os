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
/// dimension, then nearest-neighbour upscale. 3 = 9× fewer pixels.
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

        // Rotation angle (only active when is_rotate is on).
        let rot = if is_rotate {
            time * spin_speed * (2.0 * PI / 60.0)
        } else {
            1.0
        };

        // Distortion phase.
        let phase = time * speed * (2.0 * PI / 10.0);

        let contrast_mod = 0.25 * contrast + 0.5 * spin_amount + 1.2;
        let base_w = 0.3 / contrast;

        // Render at reduced resolution for performance.
        let iw = self.width.div_ceil(RENDER_SCALE);
        let ih = self.height.div_ceil(RENDER_SCALE);
        let scale_f = RENDER_SCALE as f32;

        // Reuse cached low-res buffer.
        self.lo_buf.resize((iw * ih) as usize, [0u8; 4]);

        for iy in 0..ih {
            for ix in 0..iw {
                // Map internal pixel to output-space coordinate (centre of block).
                let ox = ix as f32 * scale_f + scale_f * 0.5;
                let oy = iy as f32 * scale_f + scale_f * 0.5;

                // Pixel quantisation (same as GLSL).
                let qx = (ox / pixel_size).floor() * pixel_size;
                let qy = (oy / pixel_size).floor() * pixel_size;
                let mut ux = (qx - 0.5 * w) / res_len;
                let mut uy = (qy - 0.5 * h) / res_len;
                let uv_len = (ux * ux + uy * uy).sqrt();

                // Polar-coordinate twist.
                let angle = uy.atan2(ux) + rot
                    - spin_ease * 20.0 * (spin_amount * uv_len + (1.0 - spin_amount));
                ux = uv_len * angle.cos();
                uy = uv_len * angle.sin();

                // Scale up for distortion detail.
                ux *= 30.0;
                uy *= 30.0;

                // 5-iteration distortion loop.
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

                // Distance-based colour weighting.
                let dist = (ux * ux + uy * uy).sqrt();
                let paint_res = (dist * 0.035 * contrast_mod).clamp(0.0, 2.0);
                let c1p = (1.0 - contrast_mod * (1.0 - paint_res).abs()).max(0.0);
                let c2p = (1.0 - contrast_mod * paint_res.abs()).max(0.0);
                let c3p = 1.0 - (c1p + c2p).min(1.0);

                // Lighting highlights.
                let light = (lighting - 0.2) * (c1p * 5.0 - 4.0).max(0.0)
                    + lighting * (c2p * 5.0 - 4.0).max(0.0);

                // Final colour.
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

        // Nearest-neighbour upscale to output buffer.
        for y in 0..self.height {
            let sy = (y / RENDER_SCALE).min(ih - 1);
            let src_row = (sy * iw) as usize;
            let dst_row = (y * self.width * 4) as usize;
            for x in 0..self.width {
                let sx = (x / RENDER_SCALE).min(iw - 1) as usize;
                let rgba = self.lo_buf[src_row + sx];
                let idx = dst_row + (x * 4) as usize;
                self.pixel_buf[idx] = rgba[0];
                self.pixel_buf[idx + 1] = rgba[1];
                self.pixel_buf[idx + 2] = rgba[2];
                self.pixel_buf[idx + 3] = rgba[3];
            }
        }
        &self.pixel_buf
    }
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
        // Should have non-zero pixels (not all black).
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
}
