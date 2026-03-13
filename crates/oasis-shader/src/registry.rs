//! Built-in shader name → source mapping.

use crate::shaders;

/// Look up a built-in fragment shader source by name.
pub fn get_shader_source(name: &str) -> Option<&'static str> {
    match name {
        "balatro" => Some(shaders::BALATRO_FRAG),
        _ => None,
    }
}
