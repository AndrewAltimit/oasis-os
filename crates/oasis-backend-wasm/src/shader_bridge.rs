//! CPU software shader background rendering bridge for the WASM backend.
//!
//! Uses the software Balatro renderer to generate pixels, then blits them
//! onto the main 2D canvas via `putImageData`. No separate WebGL2 canvas
//! or GL context needed — everything draws on the same canvas.

use oasis_shader::ShaderParams;
use oasis_shader::software::SoftwareShaderRenderer;

/// Shader rendering state for the WASM backend.
pub struct WasmShaderBridge {
    renderer: SoftwareShaderRenderer,
    width: u32,
    height: u32,
}

impl WasmShaderBridge {
    /// Create a software shader bridge.
    #[cfg(target_arch = "wasm32")]
    pub fn new(width: u32, height: u32) -> Option<Self> {
        log::info!("shader bridge: software renderer initialized {width}x{height}");
        Some(Self {
            renderer: SoftwareShaderRenderer::new(width, height),
            width,
            height,
        })
    }

    /// Non-WASM stub: always returns `None`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(_width: u32, _height: u32) -> Option<Self> {
        None
    }

    /// Render the shader and blit pixels to the 2D canvas via `putImageData`.
    #[cfg(target_arch = "wasm32")]
    pub fn render_frame(
        &mut self,
        _shader_name: &str,
        time: f32,
        params: &ShaderParams,
        ctx: &web_sys::CanvasRenderingContext2d,
    ) {
        let pixels = self.renderer.render_balatro(time, params);

        // Create ImageData from RGBA pixel buffer and draw to canvas.
        let clamped = wasm_bindgen::Clamped(pixels);
        if let Ok(image_data) =
            web_sys::ImageData::new_with_u8_clamped_array_and_sh(clamped, self.width, self.height)
        {
            let _ = ctx.put_image_data(&image_data, 0.0, 0.0);
        }
    }

    /// Non-WASM stub.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_frame(
        &mut self,
        _shader_name: &str,
        _time: f32,
        _params: &ShaderParams,
        _ctx: &web_sys::CanvasRenderingContext2d,
    ) {
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
}
