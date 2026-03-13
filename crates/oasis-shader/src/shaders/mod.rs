//! Built-in shader source constants.

/// Passthrough vertex shader (GLSL ES 3.00).
///
/// Draws a single oversized triangle covering the viewport.
/// No vertex buffer needed — uses `gl_VertexID`.
#[cfg(feature = "glow")]
pub const VERTEX_SHADER: &str = include_str!("common.vert");

/// Balatro card-game swirl effect (GLSL ES 3.00).
///
/// Port of <https://www.shadertoy.com/view/XXtBRr>.
pub const BALATRO_FRAG: &str = include_str!("balatro.frag");

/// Animated Voronoi cell pattern (GLSL ES 3.00).
///
/// Port of <https://www.shadertoy.com/view/WdlyRS>.
pub const VORONOI_FRAG: &str = include_str!("voronoi.frag");

/// Colour grid with animated cell brightness and shadows (GLSL ES 3.00).
///
/// Port of <https://www.shadertoy.com/view/wscGWl>.
pub const CITY_LIGHTS_FRAG: &str = include_str!("city_lights.frag");

/// Layered sine-wave ocean (GLSL ES 3.00).
///
/// Port of <https://www.shadertoy.com/view/33t3WB>.
pub const OCEAN_WAVES_FRAG: &str = include_str!("ocean_waves.frag");

/// Gentle blue waves with subtle cosine/sine ripples (GLSL ES 3.00).
///
/// Port of <https://www.shadertoy.com/view/3fBBDc>.
pub const CALM_WAVES_FRAG: &str = include_str!("calm_waves.frag");
