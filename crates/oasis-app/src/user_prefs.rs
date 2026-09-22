//! User preferences: boot restore, disk persistence and the Settings IPC
//! handlers for volume, locale, font scale, reduced motion and the
//! high-contrast shortcut.
//!
//! The desktop VFS is an in-memory `MemoryVfs` rebuilt on every launch, so
//! `/system/settings.toml` alone would not survive a restart. The host
//! mirrors that file to real storage: [`load_from_disk`] seeds it at boot
//! (before the skin is resolved, so the persisted skin/resolution apply to
//! the very first frame) and [`DiskMirror`] writes it back whenever its
//! contents change — which also persists every other key in the store
//! (e.g. free-layout icon positions).
//!
//! Location: `$OASIS_SETTINGS_FILE` if set, else
//! `<config dir>/oasis-os/settings.toml` where the config dir is
//! `$XDG_CONFIG_HOME`, `%APPDATA%` or `~/.config`.

use std::path::PathBuf;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::AudioBackend;
use oasis_core::i18n::{self, Locale};
use oasis_core::sdi::SdiRegistry;
use oasis_core::settings::{self, SettingsStore, UserPrefs, pref_keys};
use oasis_core::skin::resolve_skin_request;
use oasis_core::startmenu::StartMenuState;
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::AppState;

/// Real-storage location of the settings file, if one can be determined.
pub fn disk_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OASIS_SETTINGS_FILE") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("oasis-os").join("settings.toml"))
}

/// Read the persisted settings from real storage (empty store when the
/// file is missing or unreadable).
pub fn load_from_disk(path: Option<&PathBuf>) -> SettingsStore {
    let mut store = SettingsStore::new();
    if let Some(path) = path {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                store.load_from_str(&text);
                log::info!("Loaded settings from {}", path.display());
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => log::warn!("Reading settings {} failed: {e}", path.display()),
        }
    }
    store
}

/// Mirrors the VFS settings file to real storage whenever it changes.
pub struct DiskMirror {
    path: Option<PathBuf>,
    last: Option<Vec<u8>>,
}

impl DiskMirror {
    /// Create a mirror for `path`, treating the current VFS contents as
    /// already persisted (they were just loaded from there).
    pub fn new(path: Option<PathBuf>, vfs: &dyn Vfs) -> Self {
        Self {
            path,
            last: vfs.read(settings::DEFAULT_PATH).ok(),
        }
    }

    /// Write the VFS settings file to disk if it changed since the last
    /// sync. Cheap when nothing changed (one small in-memory read).
    pub fn sync(&mut self, vfs: &dyn Vfs) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let Ok(data) = vfs.read(settings::DEFAULT_PATH) else {
            return;
        };
        if self.last.as_deref() == Some(data.as_slice()) {
            return;
        }
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("Creating settings dir {} failed: {e}", dir.display());
            return;
        }
        match std::fs::write(path, &data) {
            Ok(()) => self.last = Some(data),
            Err(e) => log::warn!("Writing settings {} failed: {e}", path.display()),
        }
    }
}

/// Read-modify-write the user preferences and save the store to the VFS.
pub fn update(state: &mut AppState, vfs: &mut MemoryVfs, f: impl FnOnce(&mut UserPrefs)) {
    let mut prefs = UserPrefs::from_store(&state.settings);
    f(&mut prefs);
    prefs.write_to(&mut state.settings);
    state.settings.save(vfs);
}

/// Current user preferences.
pub fn current(state: &AppState) -> UserPrefs {
    UserPrefs::from_store(&state.settings)
}

/// Rebuild the active theme from the current skin with the user's font
/// scale / reduced motion applied, plus the theme-derived start menu.
/// Used when only a preference (not the skin) changed.
fn rebuild_theme(state: &mut AppState) {
    let prefs = current(state);
    prefs.patch_features(&mut state.skin.features);
    let sw = state.active_theme.screen_w;
    let sh = state.active_theme.screen_h;
    let mut at = ActiveTheme::from_skin(&state.skin.theme)
        .with_screen_size(sw, sh)
        .with_features(&state.skin.features);
    prefs.apply_font_scale(&mut at);
    state.active_theme = at;
    rebuild_start_menu(state);
}

/// Rebuild the start menu (its labels and geometry come from the theme and
/// the active locale).
pub fn rebuild_start_menu(state: &mut AppState) {
    let open = state.ui.start_menu.open;
    state.ui.start_menu = StartMenuState::new_with_theme(
        StartMenuState::default_items(&state.active_theme),
        &state.active_theme,
    );
    if open {
        state.ui.start_menu.toggle();
    }
}

/// Read and clear one IPC request path, returning its trimmed payload.
fn take_request(vfs: &mut MemoryVfs, path: &str) -> Option<String> {
    let data = vfs.read(path).ok()?;
    let req = String::from_utf8_lossy(&data).trim().to_string();
    // Always clear so malformed input doesn't loop.
    let _ = vfs.write(path, b"");
    (!req.is_empty()).then_some(req)
}

/// Dispatch the preference IPC requests posted by the Settings app
/// (volume, locale, font scale, reduced motion, high contrast). Applies
/// each change live and persists it. Returns `true` when anything changed
/// (the caller then republishes the runtime state).
pub fn poll_prefs_ipc(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) -> bool {
    use oasis_app_settings as s;
    let mut changed = false;

    if let Some(req) = take_request(vfs, s::VOLUME_CHANGE_REQUEST_PATH) {
        match req.parse::<u32>() {
            Ok(v) => {
                let v = v.min(100) as u8;
                if let Err(e) = state.audio_backend.set_volume(v) {
                    log::warn!("set_volume({v}) failed: {e}");
                }
                update(state, vfs, |p| p.volume = v);
                changed = true;
            },
            Err(_) => log::warn!("Ignoring malformed volume request: {req}"),
        }
    }

    if let Some(req) = take_request(vfs, s::LOCALE_CHANGE_REQUEST_PATH) {
        match Locale::from_code(&req) {
            Some(locale) => {
                update(state, vfs, |p| p.locale = locale.code().to_string());
                let effective = i18n::set_ui_locale(locale);
                rebuild_start_menu(state);
                state.ui.bottom_bar.update_info(state_time(state).as_ref());
                if effective != locale {
                    state.toasts.show(
                        format!(
                            "{}: font lacks glyphs, UI stays English",
                            locale.english_name()
                        ),
                        oasis_core::toast::ToastLevel::Warning,
                        state.active_theme.toast.ttl,
                    );
                }
                changed = true;
            },
            None => log::warn!("Ignoring unknown locale request: {req}"),
        }
    }

    if let Some(req) = take_request(vfs, s::FONT_SCALE_REQUEST_PATH) {
        match req.parse::<f32>() {
            Ok(f) if f.is_finite() => {
                update(state, vfs, |p| p.font_scale = settings::clamp_font_scale(f));
                rebuild_theme(state);
                changed = true;
            },
            _ => log::warn!("Ignoring malformed font-scale request: {req}"),
        }
    }

    if let Some(req) = take_request(vfs, s::REDUCED_MOTION_REQUEST_PATH) {
        let on = matches!(req.as_str(), "1" | "true" | "on");
        update(state, vfs, |p| p.reduced_motion = on);
        // Start from the skin's own flag so switching the preference off
        // really re-enables motion, then re-apply the skin so dashboard,
        // wallpaper layers and transitions all pick it up.
        let name = state.skin.manifest.name.clone();
        let mut skin = state.skin.clone();
        skin.features.reduced_motion = resolve_skin_request(&name, &state.skin)
            .map(|fresh| fresh.features.reduced_motion)
            .unwrap_or(false);
        crate::commands::apply_skin_object(skin, state, sdi, vfs);
        changed = true;
    }

    if let Some(req) = take_request(vfs, s::HIGH_CONTRAST_REQUEST_PATH) {
        let current_skin = state.skin.manifest.name.clone();
        let target = if req == "on" {
            if current_skin != s::HIGH_CONTRAST_SKIN {
                state
                    .settings
                    .set_string(pref_keys::SKIN_BEFORE_HIGH_CONTRAST, current_skin.clone());
            }
            s::HIGH_CONTRAST_SKIN.to_string()
        } else {
            state
                .settings
                .get_string(pref_keys::SKIN_BEFORE_HIGH_CONTRAST)
                .filter(|n| *n != s::HIGH_CONTRAST_SKIN)
                .unwrap_or("classic")
                .to_string()
        };
        if target != current_skin {
            crate::commands::apply_skin_swap(&target, state, sdi, vfs);
            let applied = state.skin.manifest.name.clone();
            update(state, vfs, |p| p.skin = Some(applied));
            changed = true;
        }
    }

    changed
}

/// Wall-clock time for refreshing localized date strings.
fn state_time(state: &AppState) -> Option<oasis_core::platform::SystemTime> {
    use oasis_core::platform::TimeService;
    state.platform.now().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_mirror_writes_only_on_change() {
        let dir = std::env::temp_dir().join(format!("oasis-prefs-test-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let mut vfs = MemoryVfs::new();
        let mut store = SettingsStore::new();
        store.set_int(pref_keys::VOLUME, 42);
        store.save(&mut vfs);

        let mut mirror = DiskMirror::new(Some(path.clone()), &vfs);
        mirror.sync(&vfs);
        assert!(!path.exists(), "unchanged contents are not rewritten");

        store.set_int(pref_keys::VOLUME, 43);
        store.save(&mut vfs);
        mirror.sync(&vfs);
        let reloaded = load_from_disk(Some(&path));
        assert_eq!(UserPrefs::from_store(&reloaded).volume, 43);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_missing_disk_file_is_empty() {
        let path = std::env::temp_dir().join("oasis-prefs-test-missing/settings.toml");
        let store = load_from_disk(Some(&path));
        assert_eq!(store.keys().count(), 0);
        assert_eq!(load_from_disk(None).keys().count(), 0);
    }
}
