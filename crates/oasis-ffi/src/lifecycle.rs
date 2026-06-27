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
use oasis_core::vfs::{GameAssetVfs, Vfs};

use crate::handle::{OasisInstance, c_str_to_str};

static INIT_LOGGER: Once = Once::new();

/// Default dashboard apps seeded into the VFS so `discover_apps` populates the
/// desktop. Mirrors the GUI app set seeded by the desktop app's `populate_demo_vfs`.
/// `discover_apps` only needs a directory per app; the active skin supplies icon art.
const DEMO_APPS: &[&str] = &[
    "Browser",
    "File Manager",
    "Internet Radio",
    "Music Player",
    "Settings",
    "TV Guide",
    "Terminal",
    "Video Embed",
];

/// Create a new OASIS_OS instance.
///
/// `width` and `height` set the virtual screen resolution.
/// `skin_toml`, `layout_toml`, and `features_toml` are optional null-terminated
/// TOML strings. Pass null for any to use defaults.
///
/// This uses the default theme. To supply a skin's `theme.toml` and
/// `strings.toml` as well, use [`oasis_create_full`].
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
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    unsafe {
        create_instance(
            width,
            height,
            skin_toml,
            layout_toml,
            features_toml,
            std::ptr::null(),
            std::ptr::null(),
        )
    }
}

/// Create a new OASIS_OS instance from a skin's full TOML set.
///
/// Identical to [`oasis_create`] but additionally accepts the skin's
/// `theme_toml` (color scheme) and `strings_toml` (localized display strings),
/// so the instance renders with the skin's real theme rather than the default.
///
/// All five TOML pointers are optional. A skin is only loaded when
/// `manifest_toml`, `layout_toml`, and `features_toml` are all non-null;
/// `theme_toml` and `strings_toml` fall back to defaults when null.
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
pub unsafe extern "C" fn oasis_create_full(
    width: u32,
    height: u32,
    manifest_toml: *const c_char,
    layout_toml: *const c_char,
    features_toml: *const c_char,
    theme_toml: *const c_char,
    strings_toml: *const c_char,
) -> *mut OasisInstance {
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    unsafe {
        create_instance(
            width,
            height,
            manifest_toml,
            layout_toml,
            features_toml,
            theme_toml,
            strings_toml,
        )
    }
}

/// Shared instance construction for [`oasis_create`] and [`oasis_create_full`].
///
/// # Safety
///
/// All string pointers must be null or valid null-terminated C strings.
unsafe fn create_instance(
    width: u32,
    height: u32,
    manifest_toml: *const c_char,
    layout_toml: *const c_char,
    features_toml: *const c_char,
    theme_toml: *const c_char,
    strings_toml: *const c_char,
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
    let manifest_str = unsafe { c_str_to_str(manifest_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let layout_str = unsafe { c_str_to_str(layout_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let features_str = unsafe { c_str_to_str(features_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let theme_str = unsafe { c_str_to_str(theme_toml) };
    // SAFETY: Caller guarantees pointers are null or valid C strings per function safety contract.
    let strings_str = unsafe { c_str_to_str(strings_toml) };

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

    // Seed dashboard apps so the desktop is populated. discover_apps scans /apps
    // for one subdirectory per app; without this the dashboard renders empty.
    let _ = vfs.mkdir("/apps");
    for name in DEMO_APPS {
        let _ = vfs.mkdir(&format!("/apps/{name}"));
    }

    let platform = DesktopPlatform::new();

    // Try to load skin if the manifest, layout, and features TOML strings are
    // all provided. Theme and strings fall back to defaults when absent.
    let skin = match (manifest_str, layout_str, features_str) {
        (Some(m), Some(l), Some(f)) => oasis_core::skin::Skin::from_toml_full(
            m,
            l,
            f,
            theme_str.unwrap_or(""),
            strings_str.unwrap_or(""),
        )
        .ok(),
        _ => None,
    };

    // Scale the theme + layout to the requested buffer resolution (the desktop/
    // wasm app does the same via `with_screen_size` + `apply_layout_scaled`).
    // Without this the UI stays laid out for 480x272 regardless of buffer size.
    let active_theme = skin
        .as_ref()
        .map(|s| ActiveTheme::from_skin(&s.theme).with_features(&s.features))
        .unwrap_or_default()
        .with_screen_size(width, height);

    // Apply skin layout (scaled to the target resolution) if available.
    let dashboard = if let Some(ref skin) = skin {
        skin.apply_layout_scaled(&mut sdi, width, height);
        let apps = discover_apps(&vfs, "/apps", None).unwrap_or_default();
        let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
        Some(DashboardState::new(dash_config, apps))
    } else {
        None
    };

    // Start button + menu, sized from the active theme. Items are empty for the
    // embedded demo (the button always renders; the popup is unused here).
    let start_menu =
        oasis_core::startmenu::StartMenuState::new_with_theme(Vec::new(), &active_theme);

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
        status_bar: oasis_core::statusbar::StatusBar::new(),
        bottom_bar: oasis_core::bottombar::BottomBar::new(),
        start_menu,
        cwd: "/".to_string(),
        output_lines: Vec::new(),
        callbacks: HashMap::new(),
        width,
        height,
        software_shader: None,
        shader_time: 0.0,
        shader_cache: Vec::new(),
        last_render_time: -1000.0,
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
