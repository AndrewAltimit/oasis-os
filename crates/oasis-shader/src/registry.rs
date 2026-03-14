//! Built-in shader name -> source mapping.

use crate::shaders;

/// Look up a built-in fragment shader source by name.
pub fn get_shader_source(name: &str) -> Option<&'static str> {
    match name {
        "balatro" => Some(shaders::BALATRO_FRAG),
        "voronoi" => Some(shaders::VORONOI_FRAG),
        "city_lights" => Some(shaders::CITY_LIGHTS_FRAG),
        "ocean_waves" => Some(shaders::OCEAN_WAVES_FRAG),
        "calm_waves" => Some(shaders::CALM_WAVES_FRAG),
        "starfield" => Some(shaders::STARFIELD_FRAG),
        "plasma" => Some(shaders::PLASMA_FRAG),
        "matrix_rain" => Some(shaders::MATRIX_RAIN_FRAG),
        _ => None,
    }
}
