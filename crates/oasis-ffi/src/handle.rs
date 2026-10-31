//! `OasisInstance` struct definition and safe handle access helpers.

use std::collections::{BTreeSet, HashMap};
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
use oasis_core::vector_overlay::LayerOpsCache;
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
    /// Static background layers tessellated (and, with render-target
    /// support, baked) once and replayed each frame. The skin never
    /// changes after `oasis_create_full`, so it is never invalidated.
    pub(crate) background_layer_cache: LayerOpsCache,
    /// Render stages whose failure was already logged at `warn`
    /// ([`OasisInstance::report`]).
    pub(crate) reported_render_errors: BTreeSet<&'static str>,
    pub(crate) callbacks: HashMap<u32, OasisCallback>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Software shader renderer (CPU fallback for GPU shaders).
    pub(crate) software_shader: Option<oasis_shader::software::SoftwareShaderRenderer>,
    /// Instance clock: the sum of the (sanitized) `oasis_tick` deltas, in
    /// seconds. `f64` so it keeps advancing at frame-sized steps for years
    /// (an `f32` stops absorbing 1/60 s after about six days).
    pub(crate) clock: f64,
    /// Scratch full-resolution buffer holding the upscaled shader wallpaper,
    /// reused across renders to avoid reallocating each frame.
    pub(crate) shader_cache: Vec<u8>,
    /// `clock` at the last frame that actually rendered. Drives the render
    /// scheduler that skips the expensive render when nothing visible changed.
    pub(crate) last_render_time: f64,
    /// Runs the `cron` jobs from `oasis_tick`.
    pub(crate) cron: oasis_core::script::CronScheduler,
    /// Background video decode thread state (when `video-decode` feature is enabled).
    #[cfg(feature = "_video")]
    pub(crate) video_state: Option<VideoThreadState>,
}

impl OasisInstance {
    /// Log a failed render stage: at `warn` the first time `stage` fails,
    /// at `debug` afterwards (a broken skin fails every frame).
    pub(crate) fn report(&mut self, stage: &'static str, result: oasis_core::error::Result<()>) {
        if let Err(e) = result {
            if self.reported_render_errors.insert(stage) {
                log::warn!("oasis_tick: {stage} failed: {e} (further failures logged at debug)");
            } else {
                log::debug!("oasis_tick: {stage} failed: {e}");
            }
        }
    }

    /// Fire a callback if registered. Interior NULs in `detail` become
    /// U+FFFD (as in `oasis_send_command`'s result) rather than dropping
    /// the event.
    ///
    /// `c_detail` lives for the entire block body, so the raw pointer from
    /// `as_ptr()` is valid for the duration of the `cb()` call.
    pub(crate) fn fire_callback(&self, event: u32, detail: &str) {
        if let Some(cb) = self.callbacks.get(&event) {
            let c_detail = to_c_string_lossy(detail.to_string());
            cb(event, c_detail.as_ptr());
        }
    }
}

/// `text` as a C string, each interior NUL (unrepresentable) replaced with
/// U+FFFD.
pub(crate) fn to_c_string_lossy(text: String) -> CString {
    CString::new(text).unwrap_or_else(|e| {
        let text = String::from_utf8_lossy(&e.into_vec()).replace('\0', "\u{FFFD}");
        CString::new(text).unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// Re-entrancy guard
// ---------------------------------------------------------------------------

thread_local! {
    /// Addresses of the handles an export on this thread is using right
    /// now. Callbacks run inside the export that fired them, while that
    /// export holds `&mut OasisInstance`; a callback calling back in with
    /// the same handle would alias it.
    static ACTIVE_HANDLES: std::cell::RefCell<Vec<usize>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Marks a handle as in use until dropped (also on unwind).
pub(crate) struct ActiveHandle(usize);

impl ActiveHandle {
    /// Mark `handle` in use, or return `None` (and log) when an export on
    /// this thread is already using it: a re-entrant call from a callback.
    pub(crate) fn enter(handle: *const OasisInstance, export: &'static str) -> Option<Self> {
        let addr = handle as usize;
        let reentrant = ACTIVE_HANDLES.with(|a| {
            let mut active = a.borrow_mut();
            if active.contains(&addr) {
                return true;
            }
            active.push(addr);
            false
        });
        if reentrant {
            log::error!("FFI: {export} called from a callback with the same handle; ignored");
            return None;
        }
        Some(Self(addr))
    }
}

impl Drop for ActiveHandle {
    fn drop(&mut self) {
        // `try_with`: the thread-local may already be gone at thread exit.
        let _ = ACTIVE_HANDLES.try_with(|a| {
            let mut active = a.borrow_mut();
            if let Some(i) = active.iter().rposition(|&h| h == self.0) {
                active.swap_remove(i);
            }
        });
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
/// Returns `default` if `handle` is null or already in use by an export
/// on this thread (a callback re-entering the API, see [`ActiveHandle`]);
/// otherwise calls `f` with an exclusive reference to the instance.
///
/// # Safety
///
/// `handle` must be null or a valid pointer previously returned by
/// `oasis_create`.
pub(crate) unsafe fn with_instance<F, R>(handle: *mut OasisInstance, default: R, f: F) -> R
where
    F: FnOnce(&mut OasisInstance) -> R,
{
    if handle.is_null() {
        return default;
    }
    let Some(_active) = ActiveHandle::enter(handle, "oasis_*") else {
        return default;
    };
    // SAFETY: Caller guarantees `handle` is null or valid per function safety
    // contract; it is non-null here, and the `ActiveHandle` guard ensures no
    // other reference to it is live on this thread.
    let Some(instance) = (unsafe { handle.as_mut() }) else {
        return default;
    };
    f(instance)
}

/// Safely access an immutable `OasisInstance` from a raw pointer.
///
/// Returns `default` if `handle` is null or already in use (as
/// [`with_instance`]); otherwise calls `f` with a shared reference.
///
/// # Safety
///
/// `handle` must be null or a valid pointer previously returned by
/// `oasis_create`.
pub(crate) unsafe fn with_instance_ref<F, R>(handle: *const OasisInstance, default: R, f: F) -> R
where
    F: FnOnce(&OasisInstance) -> R,
{
    if handle.is_null() {
        return default;
    }
    let Some(_active) = ActiveHandle::enter(handle, "oasis_*") else {
        return default;
    };
    // SAFETY: Caller guarantees `handle` is null or valid per function safety
    // contract; no `&mut` to it is live on this thread (guard above).
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
