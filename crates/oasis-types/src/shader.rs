//! Shader-specific parameter types shared across oasis-vector and oasis-shader.

use std::collections::HashMap;

/// Shader-specific parameters passed from TOML skin config to the renderer.
///
/// Used by `oasis-shader` and `oasis-vector` to configure animated wallpaper
/// shaders with skin-defined colors and tuning parameters.
#[derive(Debug, Clone, Default)]
pub struct ShaderParams {
    /// Up to 4 configurable RGBA colors (each component 0.0-1.0).
    pub colors: Vec<[f32; 4]>,
    /// Named float uniforms (e.g. "speed", "contrast").
    pub floats: HashMap<String, f32>,
}
