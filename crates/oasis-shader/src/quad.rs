//! Fullscreen triangle rendering (3 vertices, no IBO).
//!
//! Uses `gl_VertexID` in the vertex shader to generate positions for an
//! oversized triangle that covers the entire viewport.

#[cfg(feature = "glow")]
use glow::HasContext;

/// Create an empty VAO for the fullscreen triangle draw call.
///
/// The vertex shader generates positions from `gl_VertexID`, so no
/// vertex buffer is needed — we just need a bound VAO to satisfy the
/// GL ES 3.0 requirement.
#[cfg(feature = "glow")]
pub fn create_fullscreen_vao(gl: &glow::Context) -> Result<glow::VertexArray, String> {
    // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
    unsafe { gl.create_vertex_array().map_err(|e| e.to_string()) }
}

/// Draw the fullscreen triangle (3 vertices, no buffer).
#[cfg(feature = "glow")]
pub fn draw_fullscreen_triangle(gl: &glow::Context, vao: glow::VertexArray) {
    // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
    unsafe {
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
    }
}
