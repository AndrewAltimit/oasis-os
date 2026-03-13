//! CPU software shader background rendering bridge for the SDL3 backend.
//!
//! Uses the software (CPU) Balatro renderer to generate pixels, then uploads
//! them via the backend's own `load_texture` / `blit` path — the same pipeline
//! that renders wallpaper, icons, and every other texture. This avoids any
//! subtle differences between a separate streaming texture and the backend's
//! internal texture management.

use oasis_shader::ShaderParams;
use oasis_shader::software::SoftwareShaderRenderer;
use oasis_types::backend::{SdiCore, TextureId};

/// Shader rendering state for the SDL3 backend.
pub struct SdlShaderBridge {
    renderer: SoftwareShaderRenderer,
    /// Cached texture ID managed by the backend's own texture system.
    cached_tex: Option<TextureId>,
    width: u32,
    height: u32,
}

impl SdlShaderBridge {
    /// Create a software shader bridge.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        log::info!("shader bridge: software renderer initialized {width}x{height}");
        Some(Self {
            renderer: SoftwareShaderRenderer::new(width, height),
            cached_tex: None,
            width,
            height,
        })
    }

    /// Render a shader and blit the result to the SDL canvas.
    ///
    /// Uses the backend's standard `load_texture` → `blit` → `destroy_texture`
    /// path so the pixels go through the exact same pipeline as wallpaper.
    pub fn render_and_blit(
        &mut self,
        backend: &mut super::SdlBackend,
        shader_name: &str,
        time: f32,
        params: &ShaderParams,
    ) {
        let pixels = self.renderer.render_shader(shader_name, time, params);

        // Destroy previous frame's texture.
        if let Some(old) = self.cached_tex.take() {
            let _ = backend.destroy_texture(old);
        }

        // Upload new pixels through the backend's own texture system.
        match backend.load_texture(self.width, self.height, pixels) {
            Ok(tex) => {
                let _ = backend.blit(tex, 0, 0, self.width, self.height);
                self.cached_tex = Some(tex);
            },
            Err(e) => {
                log::warn!("shader texture upload failed: {e}");
            },
        }
    }

    /// Resize the renderer.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.renderer.resize(width, height);
    }

    /// Clean up the cached texture.
    pub fn destroy(&mut self, backend: &mut super::SdlBackend) {
        if let Some(tex) = self.cached_tex.take() {
            let _ = backend.destroy_texture(tex);
        }
    }
}
