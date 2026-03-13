//! Shader-specific parameters from TOML configuration.

use std::collections::HashMap;

/// Shader-specific parameters passed from TOML skin config to the renderer.
#[derive(Debug, Clone, Default)]
pub struct ShaderParams {
    /// Up to 4 configurable RGBA colors (each component 0.0–1.0).
    pub colors: Vec<[f32; 4]>,
    /// Named float uniforms (e.g. "speed", "contrast").
    pub floats: HashMap<String, f32>,
}

/// Parsed information about a shader background layer.
#[derive(Debug, Clone)]
pub struct ShaderLayerInfo {
    /// Shader name (matches registry key, e.g. "balatro").
    pub name: String,
    /// Shader-specific parameters.
    pub params: ShaderParams,
}
