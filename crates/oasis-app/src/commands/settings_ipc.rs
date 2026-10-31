//! The Settings app's IPC: change requests it posts under `/system/ipc`
//! (skin, resolution, theme preview, save-as-custom-skin, and the
//! preferences `user_prefs` handles) and the `/system/state` files the
//! shell publishes for it to read back. Paths live in `oasis_core::ipc`.

use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_core::ipc::{
    BACKEND_STATE_PATH, FONT_SCALE_STATE_PATH, LOCALE_STATE_PATH, REDUCED_MOTION_STATE_PATH,
    RESOLUTION_CHANGE_REQUEST_PATH, RESOLUTION_STATE_PATH, SKIN_APPLY_THEME_REQUEST_PATH,
    SKIN_CHANGE_REQUEST_PATH, SKIN_SAVE_CUSTOM_REQUEST_PATH, SKIN_STATE_PATH, VOLUME_STATE_PATH,
    parse_resolution,
};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::SkinTheme;
use oasis_core::vfs::{MemoryVfs, Vfs};

use super::skin::{apply_resolution_change, apply_skin_object, apply_skin_swap};
use super::take_request;
use crate::app_state::AppState;
use crate::shell_backend::ShellBackend;

/// Publish the current runtime state (skin, resolution, backend) to VFS so
/// the Settings app and any other UI can read it on demand. Called on
/// startup and after every apply.
pub fn publish_runtime_state(state: &AppState, backend_name: &str, vfs: &mut MemoryVfs) {
    // `MemoryVfs::write` requires the parent directory to exist, so we
    // proactively create every directory both the state publisher and the
    // IPC request poller will touch. Without `/system/ipc`, the shell's
    // pending-VFS-request block silently fails to write skin / resolution
    // change requests, and `poll_settings_ipc` never sees them.
    let _ = vfs.mkdir("/system");
    let _ = vfs.mkdir("/system/state");
    let _ = vfs.mkdir("/system/ipc");
    let prefs = crate::user_prefs::current(state);
    let (w, h) = (state.active_theme.screen_w, state.active_theme.screen_h);
    let entries: [(&str, String); 7] = [
        (SKIN_STATE_PATH, state.skin.manifest.name.clone()),
        (RESOLUTION_STATE_PATH, format!("{w}x{h}")),
        (BACKEND_STATE_PATH, backend_name.to_string()),
        // User preferences shown by the Settings Audio / Language /
        // Accessibility categories.
        (
            VOLUME_STATE_PATH,
            state.audio_backend.get_volume().to_string(),
        ),
        (LOCALE_STATE_PATH, prefs.locale.clone()),
        (FONT_SCALE_STATE_PATH, prefs.font_scale.to_string()),
        (
            REDUCED_MOTION_STATE_PATH,
            if prefs.reduced_motion { "1" } else { "0" }.to_string(),
        ),
    ];
    for (path, value) in entries {
        let _ = vfs.write(path, value.as_bytes());
    }
}

/// Poll the Settings IPC paths once per frame and dispatch any pending
/// change. Clears each request immediately after reading so the shell
/// doesn't reapply on every subsequent frame.
pub fn poll_settings_ipc(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut impl ShellBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &mut MemoryVfs,
    backend_name: &str,
) {
    let skin_request =
        take_request(vfs, SKIN_CHANGE_REQUEST_PATH).filter(|req| *req != state.skin.manifest.name);

    let resolution_request = take_request(vfs, RESOLUTION_CHANGE_REQUEST_PATH).and_then(|req| {
        let parsed = parse_resolution(&req);
        if parsed.is_none() {
            state
                .terminal
                .print(format!("Ignoring malformed resolution request: {req}"));
        }
        parsed
    });

    // "Apply" from the Settings Appearance editor: an in-memory theme
    // preview. The payload is a serialized `SkinTheme` TOML document; the
    // current skin's layout/features/strings are kept and only the theme is
    // replaced. Nothing is written to disk.
    let theme_preview = take_request(vfs, SKIN_APPLY_THEME_REQUEST_PATH).and_then(|req| {
        SkinTheme::from_toml_str(&req)
            .map_err(|e| {
                state
                    .terminal
                    .print(format!("Ignoring malformed theme preview: {e}"));
            })
            .ok()
    });

    // "Save as custom skin" from the Settings Appearance editor. Payload is
    // `<name>\n<theme toml>`; the skin is written to `skins/<name>/` in the
    // standard directory format and then swapped in by name through the
    // normal resolution path (which validates the save round-trips).
    let save_custom = take_request(vfs, SKIN_SAVE_CUSTOM_REQUEST_PATH).and_then(|req| {
        parse_save_custom_request(&req)
            .map_err(|e| {
                state
                    .terminal
                    .print(format!("Not saving the custom skin: {e}"));
            })
            .ok()
    });

    let mut changed = false;
    if let Some(name) = skin_request {
        apply_skin_swap(&name, state, sdi, vfs);
        // Persist only a swap that actually took.
        if state.skin.manifest.name == name {
            crate::user_prefs::update(state, vfs, |p| p.skin = Some(name));
        }
        changed = true;
    }
    if let Some(theme) = theme_preview {
        let mut preview = state.skin.clone();
        preview.theme = theme;
        apply_skin_object(preview, state, sdi, vfs);
        changed = true;
    }
    if let Some((name, theme)) = save_custom {
        let mut custom = state.skin.clone();
        custom.theme = theme;
        custom.manifest.name.clone_from(&name);
        let dir = state.custom_skin_root.join(&name);
        match custom.save_to_directory(&dir) {
            Ok(()) => {
                state
                    .terminal
                    .print(format!("Saved custom skin to {}", dir.display()));
                // Swap to the written directory through the normal
                // resolution path so the running session uses exactly what
                // was written to disk.
                apply_skin_swap(&dir.to_string_lossy(), state, sdi, vfs);
                if state.skin.manifest.name == name {
                    crate::user_prefs::update(state, vfs, |p| p.skin = Some(name));
                }
                changed = true;
            },
            Err(e) => {
                state
                    .terminal
                    .print(format!("Failed to save custom skin: {e}"));
            },
        }
    }
    if let Some((w, h)) = resolution_request {
        apply_resolution_change(w, h, state, sdi, backend, shader_bridge, vfs);
        // Persist only a change that took: persisting the current size
        // after a rejected (out-of-range) request pinned the resolution,
        // so later skins no longer opened at their own size.
        let applied = (state.active_theme.screen_w, state.active_theme.screen_h);
        if applied == (w, h) {
            crate::user_prefs::update(state, vfs, |p| p.resolution = Some(applied));
        }
        changed = true;
    }

    // Volume / locale / accessibility requests.
    changed |= crate::user_prefs::poll_prefs_ipc(state, sdi, vfs);

    if changed {
        publish_runtime_state(state, backend_name, vfs);
    }
}

/// Parse a save-custom-skin IPC payload (`<name>\n<theme toml>`).
///
/// The name is restricted to `[A-Za-z0-9_-]` so it stays a safe directory
/// name under `skins/`.
fn parse_save_custom_request(req: &str) -> Result<(String, SkinTheme), String> {
    let (name, theme_toml) = req
        .split_once('\n')
        .ok_or_else(|| "missing name line".to_string())?;
    let name = name.trim();
    if name.is_empty() {
        return Err("empty skin name".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid skin name '{name}'"));
    }
    // A built-in's name would overwrite its source directory when the
    // shell runs from the repository (built-ins are compiled from
    // `skins/<name>/`), and the saved copy would never load again: the
    // built-in wins name resolution.
    if oasis_core::skin::builtin::is_builtin(name) {
        return Err(format!(
            "'{name}' is a built-in skin; save the custom skin under another name"
        ));
    }
    let theme = SkinTheme::from_toml_str(theme_toml).map_err(|e| e.to_string())?;
    Ok((name.to_string(), theme))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_state() -> AppState {
        AppState::new_for_test()
    }
    // -- Settings preference IPC (volume / locale / accessibility) --

    use crate::settings::{SettingsStore, UserPrefs};
    use oasis_apps::settings as settings_app;

    /// A VFS with the runtime-state / IPC directories the shell creates.
    fn prefs_vfs(state: &AppState) -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        publish_runtime_state(state, "test", &mut vfs);
        vfs
    }

    /// Post one Settings IPC request and let the shell dispatch it.
    fn post(state: &mut AppState, vfs: &mut MemoryVfs, path: &str, payload: &str) -> bool {
        let mut sdi = SdiRegistry::new();
        vfs.write(path, payload.as_bytes()).expect("write request");
        crate::user_prefs::poll_prefs_ipc(state, &mut sdi, vfs)
    }

    /// Simulate a restart: reload the settings store from the VFS file.
    fn reloaded_prefs(vfs: &MemoryVfs) -> UserPrefs {
        let mut store = SettingsStore::new();
        store.load(vfs);
        UserPrefs::from_store(&store)
    }

    #[test]
    fn volume_request_applies_persists_and_restores() {
        let mut state = make_test_state();
        let mut vfs = prefs_vfs(&state);
        assert!(post(
            &mut state,
            &mut vfs,
            settings_app::VOLUME_CHANGE_REQUEST_PATH,
            "35"
        ));
        // Actually applied to the audio backend ...
        assert_eq!(state.audio_backend.get_volume(), 35);
        // ... the request is consumed ...
        let req = vfs
            .read(settings_app::VOLUME_CHANGE_REQUEST_PATH)
            .expect("request path");
        assert!(req.is_empty());
        // ... published for the Settings UI ...
        publish_runtime_state(&state, "test", &mut vfs);
        let published = vfs
            .read(settings_app::VOLUME_STATE_PATH)
            .expect("volume state");
        assert_eq!(published, b"35");
        // ... and restored from /system/settings.toml after a reload.
        assert_eq!(reloaded_prefs(&vfs).volume, 35);
    }

    #[test]
    fn settings_ui_volume_change_reaches_audio_backend() {
        use oasis_core::apps::App;
        use oasis_core::input::Button;

        let mut state = make_test_state();
        let mut vfs = prefs_vfs(&state);
        let mut app = settings_app::SettingsApp::from_vfs(
            "/apps/settings",
            &vfs,
            "classic",
            480,
            272,
            "test",
        );
        // Display -> Appearance -> Resolution -> Audio, then Volume Up.
        for _ in 0..3 {
            app.handle_input(&Button::Right, &vfs);
        }
        app.handle_input(&Button::Up, &vfs);
        let mut reqs = Vec::new();
        app.drain_requests(&mut reqs);
        let req = reqs.pop().expect("volume IPC");
        let (path, payload) = (req.path, req.data);
        assert!(post(&mut state, &mut vfs, &path, &payload));
        assert_eq!(state.audio_backend.get_volume(), 85);
    }

    #[test]
    fn unsupported_locale_is_persisted_but_ui_stays_english() {
        let mut state = make_test_state();
        let mut vfs = prefs_vfs(&state);
        assert!(post(
            &mut state,
            &mut vfs,
            settings_app::LOCALE_CHANGE_REQUEST_PATH,
            "ja"
        ));
        assert_eq!(reloaded_prefs(&vfs).locale, "ja");
        assert_eq!(
            oasis_core::i18n::ui_locale_for(oasis_core::i18n::Locale::Japanese),
            oasis_core::i18n::Locale::English
        );
        publish_runtime_state(&state, "test", &mut vfs);
        let published = vfs
            .read(settings_app::LOCALE_STATE_PATH)
            .expect("locale state");
        assert_eq!(published, b"ja");
    }

    #[test]
    fn font_scale_request_rescales_theme_and_persists() {
        let mut state = make_test_state();
        let base_body = state.active_theme.font_body;
        let mut vfs = prefs_vfs(&state);
        assert!(post(
            &mut state,
            &mut vfs,
            settings_app::FONT_SCALE_REQUEST_PATH,
            "1.5"
        ));
        assert_eq!(state.active_theme.font_scale, 1.5);
        assert!(state.active_theme.font_body > base_body);
        assert_eq!(reloaded_prefs(&vfs).font_scale, 1.5);
        // Scaling is applied to a fresh theme, never compounded.
        post(
            &mut state,
            &mut vfs,
            settings_app::FONT_SCALE_REQUEST_PATH,
            "1.0",
        );
        assert_eq!(state.active_theme.font_body, base_body);
    }

    #[test]
    fn reduced_motion_request_toggles_theme_motion() {
        let mut state = make_test_state();
        let mut vfs = prefs_vfs(&state);
        post(
            &mut state,
            &mut vfs,
            settings_app::REDUCED_MOTION_REQUEST_PATH,
            "1",
        );
        assert!(state.skin.features.reduced_motion);
        assert!(state.active_theme.background_reduced_motion);
        assert!(reloaded_prefs(&vfs).reduced_motion);
        post(
            &mut state,
            &mut vfs,
            settings_app::REDUCED_MOTION_REQUEST_PATH,
            "0",
        );
        assert!(!state.skin.features.reduced_motion);
        assert!(!reloaded_prefs(&vfs).reduced_motion);
    }

    #[test]
    fn high_contrast_shortcut_swaps_and_restores_skin() {
        let mut state = make_test_state();
        let original = state.skin.manifest.name.clone();
        let mut vfs = prefs_vfs(&state);
        post(
            &mut state,
            &mut vfs,
            settings_app::HIGH_CONTRAST_REQUEST_PATH,
            "on",
        );
        assert_eq!(state.skin.manifest.name, settings_app::HIGH_CONTRAST_SKIN);
        assert_eq!(
            reloaded_prefs(&vfs).skin.as_deref(),
            Some(settings_app::HIGH_CONTRAST_SKIN)
        );
        post(
            &mut state,
            &mut vfs,
            settings_app::HIGH_CONTRAST_REQUEST_PATH,
            "off",
        );
        assert_eq!(state.skin.manifest.name, original);
        assert_eq!(reloaded_prefs(&vfs).skin, Some(original));
    }

    #[test]
    fn high_contrast_off_on_another_skin_is_a_noop() {
        let mut state = make_test_state();
        // Not classic: the old fallback target.
        state.skin = oasis_core::skin::builtin::load_builtin("xp").expect("xp");
        let original = state.skin.manifest.name.clone();
        let mut vfs = prefs_vfs(&state);
        post(
            &mut state,
            &mut vfs,
            settings_app::HIGH_CONTRAST_REQUEST_PATH,
            "off",
        );
        assert_eq!(state.skin.manifest.name, original);
        assert_eq!(reloaded_prefs(&vfs).skin, None, "nothing persisted");
    }
}
