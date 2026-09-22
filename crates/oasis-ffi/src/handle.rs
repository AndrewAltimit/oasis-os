//! `OasisInstance` struct definition and safe handle access helpers.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use oasis_backend_ue5::{FfiInputBackend, Ue5AudioBackend, Ue5Backend};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::bottombar::BottomBar;
use oasis_core::dashboard::DashboardState;
use oasis_core::platform::DesktopPlatform;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
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
    /// Top status bar chrome (drawn each tick when a skin is loaded).
    pub(crate) status_bar: StatusBar,
    /// Bottom bar / taskbar chrome (drawn each tick when a skin is loaded).
    pub(crate) bottom_bar: BottomBar,
    /// Start button + start menu (drawn when the skin enables `start_menu`).
    pub(crate) start_menu: StartMenuState,
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
    /// Scratch full-resolution buffer holding the upscaled shader wallpaper,
    /// reused across renders to avoid reallocating each frame.
    pub(crate) shader_cache: Vec<u8>,
    /// `shader_time` at the last frame that actually rendered. Drives the render
    /// scheduler that skips the expensive render when nothing visible changed.
    pub(crate) last_render_time: f32,
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
// Panic containment
// ---------------------------------------------------------------------------

/// Run the body of an `extern "C"` export, converting any Rust panic into
/// `default` instead of letting it unwind across the C ABI (which is
/// undefined behavior and, under `panic = "unwind"`, aborts the host).
///
/// Every exported `oasis_*` function wraps its body in this guard. The panic
/// message is logged at `error` level together with the export `name`.
///
/// **Handle policy:** a caught panic does *not* poison the instance -- the
/// handle stays usable and later calls proceed normally. A panic can only leave
/// the instance in a state reachable by safe Rust (no memory unsafety), and the
/// worst realistic outcome is one partially-updated frame that the next
/// `oasis_tick` redraws. Tearing down a UE5 widget because one frame hit a bug
/// would be a worse failure mode for the host.
///
/// Note: catching requires the library to be built with `panic = "unwind"`
/// (the `release-ffi` Cargo profile). Under the workspace `release` profile
/// (`panic = "abort"`) the process aborts before this guard ever runs.
pub(crate) fn ffi_guard<R>(name: &'static str, default: R, f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic payload>");
            log::error!("FFI: panic caught in {name}: {msg}");
            default
        },
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only fault injection: when set, the next [`maybe_inject_panic`]
    /// call panics (and clears the flag). Thread-local so parallel tests
    /// cannot trip each other.
    static INJECT_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Arm the test-only panic injection for the current thread.
#[cfg(test)]
pub(crate) fn arm_panic_injection() {
    INJECT_PANIC.with(|f| f.set(true));
}

/// Panic if [`arm_panic_injection`] was called on this thread (test builds only).
#[cfg(test)]
pub(crate) fn maybe_inject_panic() {
    if INJECT_PANIC.with(|f| f.replace(false)) {
        panic!("injected test panic");
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
