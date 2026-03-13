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
