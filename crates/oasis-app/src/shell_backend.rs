//! Render-backend abstraction for the desktop shell.
//!
//! The shell ([`crate::shell::Shell`]) is generic over [`ShellBackend`]: the
//! portable [`SdiBackend`] rendering surface plus the handful of host
//! operations the desktop shell uses that are not part of the SDI traits
//! (window resize, host pointer visibility, the shader-wallpaper blit).
//! The desktop binary uses [`SdlBackend`]; the headless e2e harness uses
//! [`crate::headless::HeadlessBackend`], a software framebuffer.

use oasis_backend_sdl::SdlBackend;
use oasis_backend_sdl::shader_bridge::ShaderBlitTarget;
use oasis_core::backend::SdiBackend;
use oasis_core::error::Result;

/// A rendering backend the desktop shell can run on.
pub trait ShellBackend: SdiBackend + ShaderBlitTarget {
    /// Human-readable backend name, published with the runtime state in
    /// the VFS (read by Settings) and shown on the boot splash (`"SDL3"`).
    const NAME: &'static str;

    /// Show or hide the host OS pointer (software-cursor skins hide it).
    fn set_host_cursor_visible(&mut self, visible: bool);

    /// Resize the output surface to `width` x `height` (live resolution
    /// change from Settings).
    fn set_window_size(&mut self, width: u32, height: u32) -> Result<()>;
}

impl ShellBackend for SdlBackend {
    const NAME: &'static str = "SDL3";

    fn set_host_cursor_visible(&mut self, visible: bool) {
        SdlBackend::set_host_cursor_visible(self, visible);
    }

    fn set_window_size(&mut self, width: u32, height: u32) -> Result<()> {
        SdlBackend::set_window_size(self, width, height)
    }
}
