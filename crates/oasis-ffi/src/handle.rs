//! `OasisInstance` struct definition and safe handle access helpers.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use oasis_backend_ue5::{FfiInputBackend, Ue5AudioBackend, Ue5Backend};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardState;
use oasis_core::platform::DesktopPlatform;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::terminal::CommandRegistry;
use oasis_core::vfs::GameAssetVfs;

use crate::types::OasisCallback;
#[cfg(feature = "_video")]
use crate::video::VideoThreadState;

/// The full internal state of an OASIS_OS instance.
///
/// Opaque to C callers -- they only hold a `*mut OasisInstance`.
pub struct OasisInstance {
    pub(crate) backend: Ue5Backend,
    pub(crate) input: FfiInputBackend,
    pub(crate) audio: Ue5AudioBackend,
    pub(crate) sdi: SdiRegistry,
    pub(crate) cmd_reg: CommandRegistry,
    pub(crate) vfs: GameAssetVfs,
    pub(crate) platform: DesktopPlatform,
    #[allow(dead_code)]
    pub(crate) skin: Option<Skin>,
    pub(crate) active_theme: ActiveTheme,
    pub(crate) dashboard: Option<DashboardState>,
    pub(crate) cwd: String,
    #[allow(dead_code)]
    pub(crate) output_lines: Vec<String>,
    pub(crate) callbacks: HashMap<u32, OasisCallback>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Software shader renderer (CPU fallback for GPU shaders).
    pub(crate) software_shader: Option<oasis_shader::software::SoftwareShaderRenderer>,
    /// Accumulated time for shader animation (seconds).
    pub(crate) shader_time: f32,
    /// Background video decode thread state (when `video-decode` feature is enabled).
    #[cfg(feature = "_video")]
    pub(crate) video_state: Option<VideoThreadState>,
}

impl OasisInstance {
    /// Fire a callback if registered.
    ///
    /// `c_detail` is bound by the `let Ok(...)` in the `if let` chain, so the
    /// `CString` lives for the entire block body -- the raw pointer from
    /// `as_ptr()` is valid for the duration of the `cb()` call.
    pub(crate) fn fire_callback(&self, event: u32, detail: &str) {
        if let Some(cb) = self.callbacks.get(&event)
            && let Ok(c_detail) = CString::new(detail)
        {
            cb(event, c_detail.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Safe handle access helpers
// ---------------------------------------------------------------------------

/// Safely access a mutable `OasisInstance` from a raw pointer.
///
/// Returns `default` if `handle` is null; otherwise calls `f` with
/// an exclusive reference to the instance.
///
/// # Safety
///
/// `handle` must be null or a valid pointer previously returned by
/// `oasis_create`.
pub(crate) unsafe fn with_instance<F, R>(handle: *mut OasisInstance, default: R, f: F) -> R
where
    F: FnOnce(&mut OasisInstance) -> R,
{
    // SAFETY: Caller guarantees `handle` is null or valid per function safety contract.
    let Some(instance) = (unsafe { handle.as_mut() }) else {
        return default;
    };
    f(instance)
}

/// Safely access an immutable `OasisInstance` from a raw pointer.
///
/// Returns `default` if `handle` is null; otherwise calls `f` with
/// a shared reference to the instance.
///
/// # Safety
///
/// `handle` must be null or a valid pointer previously returned by
/// `oasis_create`.
pub(crate) unsafe fn with_instance_ref<F, R>(handle: *mut OasisInstance, default: R, f: F) -> R
where
    F: FnOnce(&OasisInstance) -> R,
{
    // SAFETY: Caller guarantees `handle` is null or valid per function safety contract.
    let Some(instance) = (unsafe { handle.as_ref() }) else {
        return default;
    };
    f(instance)
}

// ---------------------------------------------------------------------------
// Helper: convert C string to Rust
// ---------------------------------------------------------------------------

/// # Safety
/// Caller must ensure `ptr` is null or a valid null-terminated C string.
pub(crate) unsafe fn c_str_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller guarantees valid null-terminated string.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("FFI: invalid UTF-8 in C string: {e}");
            None
        },
    }
}
