//! GPU shader background renderer for OASIS_OS.
//!
//! Provides Shadertoy-style fragment shader rendering via `glow` (OpenGL/WebGL2).
//! Each backend initializes its own GL context and passes it in. The crate handles
//! shader compilation, fullscreen-quad rendering, FBO management, and pixel readback.
//!
//! # Feature flags
//!
//! - `gl-native` — `glow` with native OpenGL loader (SDL3, headless EGL)
//! - `gl-web` — `glow` with `web_sys::WebGl2RenderingContext`
//! - `software` — CPU fallback (evaluates shader math in Rust, no GL)

mod compile;
mod quad;
pub mod registry;
mod shaders;
pub mod software;
pub mod uniforms;

pub use uniforms::ShaderParams;

#[cfg(feature = "glow")]
use std::collections::HashMap;

#[cfg(feature = "glow")]
use glow::HasContext;

/// GPU shader renderer. Owns GL resources (program, FBO, VAO).
///
/// Created from an existing `glow::Context` that the backend provides.
/// Call [`render_to_pixels`](ShaderRenderer::render_to_pixels) to render to an
/// FBO and read back RGBA pixels, or [`render_to_screen`](ShaderRenderer::render_to_screen)
/// to render directly to the default framebuffer (for WASM).
#[cfg(feature = "glow")]
pub struct ShaderRenderer {
    gl: glow::Context,
    programs: HashMap<String, glow::Program>,
    fbo: glow::Framebuffer,
    fbo_texture: glow::Texture,
    vao: glow::VertexArray,
    width: u32,
    height: u32,
    pixel_buf: Vec<u8>,
}

#[cfg(feature = "glow")]
impl ShaderRenderer {
    /// Create from an existing glow context (backend provides this).
    ///
    /// Sets up the FBO, texture attachment, and empty VAO for fullscreen draws.
    ///
    /// # Errors
    ///
    /// Returns an error string if GL resource creation fails.
    pub fn new(gl: glow::Context, w: u32, h: u32) -> Result<Self, String> {
        let vao = quad::create_fullscreen_vao(&gl)?;

        // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
        let (fbo, fbo_texture) = unsafe {
            let tex = gl.create_texture().map_err(|e| e.to_string())?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            let fb = gl.create_framebuffer().map_err(|e| e.to_string())?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );

            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("FBO incomplete: 0x{status:X}"));
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);

            (fb, tex)
        };

        // Auto-register built-in shaders.
        let mut renderer = Self {
            gl,
            programs: HashMap::new(),
            fbo,
            fbo_texture,
            vao,
            width: w,
            height: h,
            pixel_buf: vec![0u8; (w * h * 4) as usize],
        };

        // Register all known shaders.
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
            if let Some(src) = registry::get_shader_source(name)
                && let Err(e) = renderer.register(name, src)
            {
                log::warn!("failed to compile built-in shader '{name}': {e}");
            }
        }

        Ok(renderer)
    }

    /// Register a fragment shader by name.
    ///
    /// Compiles and links with the common vertex shader.
    ///
    /// # Errors
    ///
    /// Returns an error string on compile/link failure.
    pub fn register(&mut self, name: &str, frag_src: &str) -> Result<(), String> {
        let vert = compile::compile_shader(&self.gl, glow::VERTEX_SHADER, shaders::VERTEX_SHADER)?;
        let frag = compile::compile_shader(&self.gl, glow::FRAGMENT_SHADER, frag_src)?;
        let program = compile::link_program(&self.gl, vert, frag)?;
        self.programs.insert(name.to_string(), program);
        Ok(())
    }

    /// Render shader to FBO, read pixels back. Returns RGBA slice.
    ///
    /// # Errors
    ///
    /// Returns an error if the shader name is not registered.
    pub fn render_to_pixels(
        &mut self,
        name: &str,
        time: f32,
        params: &ShaderParams,
    ) -> Result<&[u8], String> {
        let program = *self
            .programs
            .get(name)
            .ok_or_else(|| format!("shader '{name}' not registered"))?;

        // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
        unsafe {
            gl_render(
                &self.gl,
                program,
                Some(self.fbo),
                self.vao,
                self.width,
                self.height,
                time,
                params,
            );

            // Read pixels from FBO.
            gl_read_pixels(
                &self.gl,
                self.fbo,
                self.width,
                self.height,
                &mut self.pixel_buf,
            );
        }

        Ok(&self.pixel_buf)
    }

    /// Render shader to default framebuffer (WASM: renders to canvas).
    ///
    /// # Errors
    ///
    /// Returns an error if the shader name is not registered.
    pub fn render_to_screen(
        &mut self,
        name: &str,
        time: f32,
        params: &ShaderParams,
    ) -> Result<(), String> {
        let program = *self
            .programs
            .get(name)
            .ok_or_else(|| format!("shader '{name}' not registered"))?;

        // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
        unsafe {
            gl_render(
                &self.gl,
                program,
                None, // default framebuffer
                self.vao,
                self.width,
                self.height,
                time,
                params,
            );
        }

        Ok(())
    }

    /// Resize FBO and pixel buffer.
    pub fn resize(&mut self, w: u32, h: u32) {
        if self.width == w && self.height == h {
            return;
        }
        self.width = w;
        self.height = h;
        self.pixel_buf.resize((w * h * 4) as usize, 0);

        // Resize FBO texture.
        // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
        unsafe {
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.fbo_texture));
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}

#[cfg(feature = "glow")]
impl Drop for ShaderRenderer {
    fn drop(&mut self) {
        // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
        unsafe {
            for program in self.programs.values() {
                self.gl.delete_program(*program);
            }
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_framebuffer(self.fbo);
            self.gl.delete_texture(self.fbo_texture);
        }
    }
}

/// Set uniforms and draw the fullscreen triangle.
///
/// # Safety
///
/// Caller must ensure the GL context is current.
#[cfg(feature = "glow")]
#[allow(clippy::too_many_arguments)]
unsafe fn gl_render(
    gl: &glow::Context,
    program: glow::Program,
    fbo: Option<glow::Framebuffer>,
    vao: glow::VertexArray,
    width: u32,
    height: u32,
    time: f32,
    params: &ShaderParams,
) {
    // SAFETY: `gl` is a live `glow::Context` and the program/fbo handles
    // were created from the same context. All glow calls below are valid
    // GL state mutations within a single thread.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, fbo);
        gl.viewport(0, 0, width as i32, height as i32);
        gl.use_program(Some(program));

        // Standard uniforms.
        if let Some(loc) = gl.get_uniform_location(program, "u_time") {
            gl.uniform_1_f32(Some(&loc), time);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_resolution") {
            gl.uniform_2_f32(Some(&loc), width as f32, height as f32);
        }

        // Default Balatro colours.
        let default_c1 = [0.871, 0.267, 0.251, 1.0]; // #DE4440
        let default_c2 = [0.0, 0.420, 0.706, 1.0]; // #006BB4
        let default_c3 = [0.086, 0.137, 0.145, 1.0]; // #162325

        let c1 = params.colors.first().copied().unwrap_or(default_c1);
        let c2 = params.colors.get(1).copied().unwrap_or(default_c2);
        let c3 = params.colors.get(2).copied().unwrap_or(default_c3);

        if let Some(loc) = gl.get_uniform_location(program, "u_color1") {
            gl.uniform_4_f32(Some(&loc), c1[0], c1[1], c1[2], c1[3]);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_color2") {
            gl.uniform_4_f32(Some(&loc), c2[0], c2[1], c2[2], c2[3]);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_color3") {
            gl.uniform_4_f32(Some(&loc), c3[0], c3[1], c3[2], c3[3]);
        }

        // Named float uniforms with defaults.
        let get_f =
            |key: &str, default: f32| -> f32 { params.floats.get(key).copied().unwrap_or(default) };

        if let Some(loc) = gl.get_uniform_location(program, "u_speed") {
            gl.uniform_1_f32(Some(&loc), get_f("speed", 1.0));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_contrast") {
            gl.uniform_1_f32(Some(&loc), get_f("contrast", 3.5));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_spin_speed") {
            gl.uniform_1_f32(Some(&loc), get_f("spin_speed", 1.0));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_spin_amount") {
            gl.uniform_1_f32(Some(&loc), get_f("spin_amount", 0.25));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_pixel_filter") {
            gl.uniform_1_f32(Some(&loc), get_f("pixel_filter", 745.0));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_is_rotate") {
            gl.uniform_1_f32(Some(&loc), get_f("is_rotate", 0.0));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_lighting") {
            gl.uniform_1_f32(Some(&loc), get_f("lighting", 0.4));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_spin_ease") {
            gl.uniform_1_f32(Some(&loc), get_f("spin_ease", 1.0));
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_size") {
            gl.uniform_1_f32(Some(&loc), get_f("size", 30.0));
        }

        quad::draw_fullscreen_triangle(gl, vao);

        gl.use_program(None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
}

/// Read pixels from the FBO into the provided buffer.
///
/// # Safety
///
/// Caller must ensure the GL context is current.
#[cfg(feature = "glow")]
unsafe fn gl_read_pixels(
    gl: &glow::Context,
    fbo: glow::Framebuffer,
    width: u32,
    height: u32,
    buf: &mut [u8],
) {
    // SAFETY: `gl` is a live `glow::Context`, `fbo` is a framebuffer handle
    // from the same context, and `buf` is a slice owned by the caller of
    // length ≥ width*height*4 (RGBA). glow's `read_pixels` writes into the
    // buffer with bounds matching the documented format.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(buf)),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
    // OpenGL reads pixels bottom-up; flip rows to top-down RGBA.
    let stride = (width * 4) as usize;
    let row_count = height as usize;
    for y in 0..row_count / 2 {
        let top = y * stride;
        let bot = (row_count - 1 - y) * stride;
        // Swap rows in-place.
        for x in 0..stride {
            buf.swap(top + x, bot + x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_balatro() {
        assert!(registry::get_shader_source("balatro").is_some());
    }

    #[test]
    fn registry_has_all_shaders() {
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
            assert!(
                registry::get_shader_source(name).is_some(),
                "missing shader: {name}"
            );
        }
    }

    #[test]
    fn registry_unknown_returns_none() {
        assert!(registry::get_shader_source("nonexistent").is_none());
    }

    #[test]
    fn shader_params_default() {
        let params = ShaderParams::default();
        assert!(params.colors.is_empty());
        assert!(params.floats.is_empty());
    }

    #[test]
    fn balatro_frag_source_not_empty() {
        assert!(!shaders::BALATRO_FRAG.is_empty());
        assert!(shaders::BALATRO_FRAG.contains("u_time"));
        assert!(shaders::BALATRO_FRAG.contains("u_resolution"));
    }
}
