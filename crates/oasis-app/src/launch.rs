use oasis_core::apps::AppRunner;
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::dashboard::AppEntry;
use oasis_core::net::RustlsTlsProvider;
use oasis_core::plugin::PluginManager;
use oasis_core::sdi::SdiRegistry;
use oasis_core::transition;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wm::manager::WindowManager;
use oasis_core::wm::window::{WindowConfig, WindowType};

use crate::app_state::Mode;

/// Result of launching an app.
pub enum LaunchResult {
    Terminal,
    Desktop,
}

/// Launch an app as a floating window (Browser, generic app, or Terminal).
///
/// Returns the mode to switch to. Caller must set `state.mode` accordingly.
/// When `window_manager` is true, Terminal opens as a window; otherwise it
/// uses fullscreen mode.
#[allow(clippy::too_many_arguments)]
pub fn launch_app_window(
    app: &AppEntry,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    open_runners: &mut Vec<(String, AppRunner)>,
    browser: &mut Option<BrowserWidget>,
    browser_config: &BrowserConfig,
    vfs: &MemoryVfs,
    tls_provider: &RustlsTlsProvider,
    window_manager: bool,
    plugin_manager: &PluginManager,
) -> LaunchResult {
    // Terminal: fullscreen mode for non-WM skins; windowed for WM skins.
    if app.title == "Terminal" && !window_manager {
        return LaunchResult::Terminal;
    }

    if app.title == "Browser" {
        let win_id = "browser";
        if wm.get_window(win_id).is_some() {
            let _ = wm.focus_window(win_id, sdi);
        } else {
            let wc = WindowConfig {
                id: win_id.to_string(),
                title: "Browser".to_string(),
                x: None,
                y: None,
                width: 380,
                height: 220,
                window_type: WindowType::AppWindow,
                always_on_top: false,
                modal: false,
            };
            let _ = wm.create_window(&wc, sdi);
            let mut bw = BrowserWidget::new(browser_config.clone());
            bw.set_tls_provider(Box::new(tls_provider.clone()));
            bw.set_window(0, 0, 380, 220);
            let home = bw.config.features.home_url.clone();
            bw.navigate_vfs(&home, vfs);
            *browser = Some(bw);
        }
        return LaunchResult::Desktop;
    }

    let win_id = app.title.to_lowercase().replace(' ', "_");
    if wm.get_window(&win_id).is_some() {
        let _ = wm.focus_window(&win_id, sdi);
    } else {
        let wc = WindowConfig {
            id: win_id.clone(),
            title: app.title.clone(),
            x: None,
            y: None,
            width: 380,
            height: 220,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        let _ = wm.create_window(&wc, sdi);
        // Check plugin registry before hardcoded app match.
        let runner = if let Some(delegate) = plugin_manager.create_plugin_app(&app.title, vfs) {
            AppRunner::from_delegate(delegate)
        } else {
            AppRunner::launch(app, vfs)
        };
        open_runners.push((win_id, runner));
    }
    LaunchResult::Desktop
}

/// Launch an app as a floating window with `file_path` pre-loaded in its
/// viewer. Used when File Manager hands off to Photo Viewer / Music Player
/// / Text Editor on Confirm of a typed file.
///
/// Reuses an existing window if one of the same id is already open (so
/// opening five PNGs in a row reuses the Photo Viewer window rather than
/// stacking five of them).
pub fn launch_app_window_for_file(
    app_title: &str,
    file_path: &str,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    open_runners: &mut Vec<(String, AppRunner)>,
    vfs: &MemoryVfs,
) {
    let win_id = app_title.to_lowercase().replace(' ', "_");
    let entry = AppEntry {
        title: app_title.to_string(),
        path: format!("/apps/{app_title}"),
        icon_png: Vec::new(),
        color: oasis_core::backend::Color::rgb(100, 100, 100),
    };

    if wm.get_window(&win_id).is_some() {
        // Replace the existing runner so the file is pre-loaded.
        let _ = wm.focus_window(&win_id, sdi);
        if let Some(entry_mut) = open_runners.iter_mut().find(|(id, _)| *id == win_id) {
            let new_runner = AppRunner::launch_with_file(&entry, file_path, vfs);
            // Hand any pending GPU textures from the outgoing runner's
            // BrowsingApp (Photo Viewer) over to the new one so they
            // get destroyed on its next render frame — otherwise the
            // old textures would leak as the runner is dropped.
            if let (Some(old_app), Some(new_app)) = (
                entry_mut.1.delegate_as::<oasis_app_media::BrowsingApp>(),
                new_runner.delegate_as::<oasis_app_media::BrowsingApp>(),
            ) {
                new_app.inherit_textures_from(old_app);
            }
            entry_mut.1 = new_runner;
        } else {
            open_runners.push((win_id, AppRunner::launch_with_file(&entry, file_path, vfs)));
        }
        return;
    }

    let wc = WindowConfig {
        id: win_id.clone(),
        title: app_title.to_string(),
        x: None,
        y: None,
        width: 380,
        height: 220,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    let _ = wm.create_window(&wc, sdi);
    open_runners.push((win_id, AppRunner::launch_with_file(&entry, file_path, vfs)));
}

/// Create a fade-in transition.
pub fn make_transition(w: u32, h: u32, fade_frames: u32) -> transition::TransitionState {
    transition::fade_in_custom(w, h, fade_frames)
}

/// Apply a launch result to update the mode.
pub fn apply_launch(result: LaunchResult, mode: &mut Mode) {
    match result {
        LaunchResult::Terminal => *mode = Mode::Terminal,
        LaunchResult::Desktop => *mode = Mode::Desktop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launch_result_variants_exist() {
        // Ensure both LaunchResult variants can be constructed.
        let _terminal = LaunchResult::Terminal;
        let _desktop = LaunchResult::Desktop;
    }

    #[test]
    fn test_apply_launch_terminal() {
        let mut mode = Mode::Dashboard;
        apply_launch(LaunchResult::Terminal, &mut mode);
        assert_eq!(mode, Mode::Terminal);
    }

    #[test]
    fn test_apply_launch_desktop() {
        let mut mode = Mode::Terminal;
        apply_launch(LaunchResult::Desktop, &mut mode);
        assert_eq!(mode, Mode::Desktop);
    }

    #[test]
    fn test_apply_launch_preserves_other_modes() {
        // Verify apply_launch correctly overwrites any starting mode.
        let mut mode = Mode::App;
        apply_launch(LaunchResult::Terminal, &mut mode);
        assert_eq!(mode, Mode::Terminal);

        let mut mode = Mode::Osk;
        apply_launch(LaunchResult::Desktop, &mut mode);
        assert_eq!(mode, Mode::Desktop);
    }

    #[test]
    fn test_make_transition_returns_transition_state() {
        // Verify make_transition produces a valid TransitionState.
        let transition = make_transition(480, 272, 30);
        // TransitionState is opaque, but we can verify it was created.
        let _ = transition;
    }

    #[test]
    fn test_make_transition_with_different_dimensions() {
        // Ensure make_transition works with various dimensions.
        let _t1 = make_transition(640, 480, 60);
        let _t2 = make_transition(1920, 1080, 120);
        let _t3 = make_transition(100, 100, 10);
    }

    #[test]
    fn test_launch_result_pattern_matching() {
        let result = LaunchResult::Terminal;
        match result {
            LaunchResult::Terminal => {},
            _ => panic!("Expected Terminal"),
        }

        let result = LaunchResult::Desktop;
        match result {
            LaunchResult::Desktop => {},
            _ => panic!("Expected Desktop"),
        }
    }

    // NOTE: The `launch_app_window` function is heavily coupled to multiple mutable state
    // parameters (WindowManager, SdiRegistry, Vec, Option, etc.) and requires complex
    // setup including VFS and TLS providers. Unit testing this function would require
    // extensive mocking that provides little value compared to integration tests.
    //
    // The function's logic is primarily:
    // 1. String matching on app.title
    // 2. Window creation/focus delegation to WindowManager
    // 3. Browser/AppRunner initialization
    //
    // These behaviors are better validated through integration tests that exercise
    // the full application flow with a real or test backend.
}
