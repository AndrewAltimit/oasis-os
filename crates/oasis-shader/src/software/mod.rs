//! CPU software renderer -- the shader implementation every shipping
//! backend (SDL, WASM, UE5/FFI, PSP) draws with.
//!
//! Evaluates the shader math per pixel in Rust at a reduced internal
//! resolution and nearest-neighbour upscales the result. Coordinates are
//! top-left origin, y down (`(0, 0)` is the first row of the returned
//! buffer); the GLSL sources mirror this by flipping `gl_FragCoord.y`, so
//! both renderers produce the same image. Parameters and their defaults
//! come from [`BuiltinShader::color`] / [`BuiltinShader::float`].
//!
//! Kernels are grouped by family: `swirl` (balatro), `cells` (voronoi,
//! city lights), `waves` (ocean, calm) and `procedural` (starfield,
//! plasma, matrix rain).

use crate::registry::BuiltinShader;
use crate::uniforms::ShaderParams;

mod cells;
mod procedural;
mod swirl;
mod waves;

const PI: f32 = std::f32::consts::PI;

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
    /// Set after the first unknown-name fallback so it is logged once.
    warned_unknown: bool,
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
            warned_unknown: false,
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

    /// Render a shader by skin name. Returns the RGBA output buffer.
    ///
    /// Unknown names fall back to [`BuiltinShader::Balatro`] (logged once
    /// per renderer) so a typo in a skin still yields a wallpaper; use
    /// [`render_builtin`](Self::render_builtin) to dispatch on the enum
    /// directly.
    pub fn render_shader(&mut self, name: &str, time: f32, params: &ShaderParams) -> &[u8] {
        let shader = BuiltinShader::from_name(name).unwrap_or_else(|| {
            if !self.warned_unknown {
                self.warned_unknown = true;
                log::warn!("unknown shader '{name}', rendering 'balatro' instead");
            }
            BuiltinShader::Balatro
        });
        self.render_builtin(shader, time, params)
    }

    /// Render a built-in shader. Returns the RGBA output buffer.
    pub fn render_builtin(
        &mut self,
        shader: BuiltinShader,
        time: f32,
        params: &ShaderParams,
    ) -> &[u8] {
        match shader {
            BuiltinShader::Balatro => self.render_balatro(time, params),
            BuiltinShader::Voronoi => self.render_voronoi(time, params),
            BuiltinShader::CityLights => self.render_city_lights(time, params),
            BuiltinShader::OceanWaves => self.render_ocean_waves(time, params),
            BuiltinShader::CalmWaves => self.render_calm_waves(time, params),
            BuiltinShader::Starfield => self.render_starfield(time, params),
            BuiltinShader::Plasma => self.render_plasma(time, params),
            BuiltinShader::MatrixRain => self.render_matrix_rain(time, params),
        }
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

#[cfg(test)]
mod tests;
