//! Instance lifecycle: `oasis_create` and `oasis_destroy`.

use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::Once;

use oasis_backend_ue5::{FfiInputBackend, Ue5AudioBackend, Ue5Backend};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{AudioBackend, SdiCore};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::platform::DesktopPlatform;
use oasis_core::sdi::SdiRegistry;
use oasis_core::terminal::{CommandRegistry, register_builtins};
use oasis_core::vfs::GameAssetVfs;

use crate::handle::{OasisInstance, c_str_to_str};

static INIT_LOGGER: Once = Once::new();

/// Create a new OASIS_OS instance.
///
/// `width` and `height` set the virtual screen resolution.
/// `skin_toml`, `layout_toml`, and `features_toml` are optional null-terminated
/// TOML strings. Pass null for any to use defaults.
///
/// Returns an opaque handle, or null on failure.
///
/// # Safety
///
/// String pointers must be null or valid null-terminated C strings.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the returned handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_create(
    width: u32,
    height: u32,
    skin_toml: *const c_char,
    layout_toml: *const c_char,
    features_toml: *const c_char,
) -> *mut OasisInstance {
    INIT_LOGGER.call_once(|| {
        let _ = env_logger::try_init();
    });

    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        log::error!(
            "oasis_create: invalid dimensions {}x{} (must be 1..=4096)",
            width,
            height,
        );
        return std::ptr::null_mut();
    }

    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let skin_str = unsafe { c_str_to_str(skin_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let layout_str = unsafe { c_str_to_str(layout_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let features_str = unsafe { c_str_to_str(features_toml) };

    let mut backend = Ue5Backend::new(width, height);
    if backend.init(width, height).is_err() {
        return std::ptr::null_mut();
    }

    let input = FfiInputBackend::new();

    let mut audio = Ue5AudioBackend::new();
    let _ = audio.init();
    let mut sdi = SdiRegistry::new();
    let mut cmd_reg = CommandRegistry::new();
    register_builtins(&mut cmd_reg);

    let mut vfs = GameAssetVfs::new();
    vfs.add_base_dir("/home");
    vfs.add_base_dir("/etc");
    vfs.add_base_dir("/tmp");

    let platform = DesktopPlatform::new();

    // Try to load skin if all three TOML strings are provided.
    let skin = match (skin_str, layout_str, features_str) {
        (Some(s), Some(l), Some(f)) => oasis_core::skin::Skin::from_toml(s, l, f).ok(),
        _ => None,
    };

    let active_theme = skin
        .as_ref()
        .map(|s| ActiveTheme::from_skin(&s.theme).with_features(&s.features))
        .unwrap_or_default();

    // Apply skin layout if available.
    let dashboard = if let Some(ref skin) = skin {
        skin.apply_layout(&mut sdi);
        let apps = discover_apps(&vfs, "/apps", None).unwrap_or_default();
        let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
        Some(DashboardState::new(dash_config, apps))
    } else {
        None
    };

    let instance = OasisInstance {
        backend,
        input,
        audio,
        sdi,
        cmd_reg,
        vfs,
        platform,
        skin,
        active_theme,
        dashboard,
        cwd: "/".to_string(),
        output_lines: Vec::new(),
        callbacks: HashMap::new(),
        width,
        height,
        software_shader: None,
        shader_time: 0.0,
        #[cfg(feature = "_video")]
        video_state: None,
    };

    Box::into_raw(Box::new(instance))
}

/// Destroy an OASIS_OS instance and free its memory.
///
/// # Safety
///
/// `handle` must be a valid pointer returned by `oasis_create`, or null.
/// After this call, `handle` is invalid and must not be used.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_destroy(handle: *mut OasisInstance) {
    if !handle.is_null() {
        // SAFETY: Reclaiming ownership of handle allocated by `oasis_create` via `Box::into_raw`.
        let mut instance = unsafe { Box::from_raw(handle) };
        let _ = instance.audio.shutdown();
        let _ = instance.backend.shutdown();
        drop(instance);
    }
}
