//! Shader compilation and linking via `glow`.

#[cfg(feature = "glow")]
use glow::HasContext;

/// Compile a GLSL shader from source and return the shader handle.
///
/// # Errors
///
/// Returns the info log on compilation failure.
#[cfg(feature = "glow")]
pub fn compile_shader(
    gl: &glow::Context,
    shader_type: u32,
    source: &str,
) -> Result<glow::Shader, String> {
    // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
    unsafe {
        let shader = gl.create_shader(shader_type).map_err(|e| e.to_string())?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(log);
        }
        Ok(shader)
    }
}

/// Link a vertex and fragment shader into a program.
///
/// Both shaders are detached and deleted after successful linking.
///
/// # Errors
///
/// Returns the info log on link failure.
#[cfg(feature = "glow")]
pub fn link_program(
    gl: &glow::Context,
    vert: glow::Shader,
    frag: glow::Shader,
) -> Result<glow::Program, String> {
    // SAFETY: glow wraps raw GL calls; no unsafe Rust required.
    unsafe {
        let program = gl.create_program().map_err(|e| e.to_string())?;
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            gl.delete_shader(vert);
            gl.delete_shader(frag);
            return Err(log);
        }
        gl.detach_shader(program, vert);
        gl.detach_shader(program, frag);
        gl.delete_shader(vert);
        gl.delete_shader(frag);
        Ok(program)
    }
}
