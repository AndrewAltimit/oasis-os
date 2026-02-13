use oasis_core::apps::AppRunner;
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::dashboard::AppEntry;
use oasis_core::net::RustlsTlsProvider;
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
) -> LaunchResult {
    if app.title == "Terminal" {
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
        };
        let _ = wm.create_window(&wc, sdi);
        open_runners.push((win_id, AppRunner::launch(app, vfs)));
    }
    LaunchResult::Desktop
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
