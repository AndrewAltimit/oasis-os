//! Opening app windows: the one launch path every caller goes through
//! (dashboard click / Confirm, start menu, `OASIS_APP`, MCP, the browser
//! IPC, "open with" hand-offs between apps).

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::browser::BrowserWidget;
use oasis_core::dashboard::AppEntry;
use oasis_core::sdi::SdiRegistry;
use oasis_core::transition;
use oasis_core::ui_sound::UiSound;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wm::manager::WindowManager;
use oasis_core::wm::window::{WindowConfig, WindowState, WindowType};

use crate::app_state::{AppState, Mode};

/// Window id of the browser (a [`BrowserWidget`], not an `App` runner).
pub const BROWSER_WIN: &str = "browser";
/// Window id of the windowed terminal.
pub const TERMINAL_WIN: &str = "terminal";
/// Dashboard title of the browser.
pub const BROWSER_TITLE: &str = "Browser";
/// Dashboard title of the terminal.
pub const TERMINAL_TITLE: &str = "Terminal";

/// Window id of the app titled `title` ("Photo Viewer" -> "photo_viewer").
pub fn window_id(title: &str) -> String {
    title.to_lowercase().replace(' ', "_")
}

/// Size of a newly opened app window: 380x220 (the PSP-native size, which
/// is what small screens get), growing with the resolution to 40% x 45%
/// of the screen so windows don't open as postage stamps on large screens
/// while several still fit side by side over the dashboard. (Scaling the
/// PSP proportions 1:1, as the WASM host does, opens every window at ~80%
/// of a 1280x720 screen, covering the dashboard icons.)
pub fn default_window_size(at: &ActiveTheme) -> (u32, u32) {
    let w = (at.screen_w * 2 / 5).max(380).min(at.screen_w);
    let h = (at.screen_h * 9 / 20).max(220).min(at.screen_h);
    (w, h)
}

/// Feedback for a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchOpts {
    /// Play the skin's "open" UI sound.
    pub sound: bool,
    /// Overlay the fullscreen fade transition. Desktop-mode launches skip
    /// it: other windows are already on screen and a fade would briefly
    /// cover them.
    pub fade: bool,
}

impl LaunchOpts {
    /// Programmatic launch (MCP, `OASIS_APP`, IPC): no sound, no fade.
    pub const QUIET: Self = Self {
        sound: false,
        fade: false,
    };
    /// User launch from the otherwise idle dashboard: sound and fade.
    pub const USER: Self = Self {
        sound: true,
        fade: true,
    };
    /// User launch with windows on screen: sound, no fade.
    pub const USER_NO_FADE: Self = Self {
        sound: true,
        fade: false,
    };
}

/// Bring an already-open window back to the user: restore it if it is
/// minimized, then focus it. (Focusing alone would make a minimized,
/// invisible window the keyboard target.)
fn raise_window(wm: &mut WindowManager, id: &str, sdi: &mut SdiRegistry) {
    if wm
        .get_window(id)
        .is_some_and(|w| w.state == WindowState::Minimized)
    {
        let _ = wm.restore_window(id, sdi);
    }
    let _ = wm.focus_window(id, sdi);
}

/// Create the app window `id` titled `title` at the default size.
fn create_window(state: &mut AppState, sdi: &mut SdiRegistry, id: &str, title: &str) {
    let (width, height) = default_window_size(&state.active_theme);
    let wc = WindowConfig {
        id: id.to_string(),
        title: title.to_string(),
        x: None,
        y: None,
        width,
        height,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    if let Err(e) = state.wm.create_window(&wc, sdi) {
        log::warn!("create_window({id}): {e}");
    }
}

/// Launch `app`: open its window (or raise the open one) and switch to
/// Desktop mode. The Terminal on a skin without a window manager opens
/// fullscreen (Terminal mode) instead. Plugin-registered apps are looked
/// up before the built-in registry.
pub fn launch(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    app: &AppEntry,
    opts: LaunchOpts,
) {
    log::info!("Launching app: {}", app.title);
    if app.title == TERMINAL_TITLE && !state.skin.features.window_manager {
        state.mode = Mode::Terminal;
    } else if app.title == BROWSER_TITLE {
        if state.wm.get_window(BROWSER_WIN).is_some() {
            raise_window(&mut state.wm, BROWSER_WIN, sdi);
        } else {
            create_window(state, sdi, BROWSER_WIN, BROWSER_TITLE);
            let (w, h) = default_window_size(&state.active_theme);
            let mut bw = BrowserWidget::new(state.browser_config.clone());
            bw.set_tls_provider(Box::new(state.net.tls_provider.clone()));
            bw.set_window(0, 0, w, h);
            let home = bw.config.features.home_url.clone();
            bw.navigate_vfs(&home, vfs);
            state.content.browser = Some(bw);
        }
        state.mode = Mode::Desktop;
    } else {
        let win_id = window_id(&app.title);
        if state.wm.get_window(&win_id).is_some() {
            raise_window(&mut state.wm, &win_id, sdi);
        } else {
            create_window(state, sdi, &win_id, &app.title);
            let runner = match state.plugin_manager.create_plugin_app(&app.title, vfs) {
                Some(delegate) => AppRunner::from_delegate(delegate),
                None => AppRunner::launch(app, vfs),
            };
            state.content.open_runners.push((win_id, runner));
        }
        state.mode = Mode::Desktop;
    }
    if opts.sound {
        state.ui_sounds.push(UiSound::Open);
    }
    if opts.fade {
        state.active_transition = Some(fade_transition(state));
    }
}

/// [`launch`] the app titled `title` (no dashboard tile needed).
pub fn launch_title(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    title: &str,
    opts: LaunchOpts,
) {
    launch(state, sdi, vfs, &AppEntry::for_title(title), opts);
}

/// Open `file_path` in the app titled `app_title` (File Manager handing
/// off to Photo Viewer / Music Player / Text Editor, MCP `open_app` with
/// a file) and switch to Desktop mode.
///
/// An already-open window of that app is reused (opening five PNGs in a
/// row keeps one Photo Viewer window): its runner is replaced by one with
/// the file pre-loaded, and the old runner is retired so its backend
/// resources (textures) are released like on a close.
pub fn launch_for_file(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    app_title: &str,
    file_path: &str,
) {
    let win_id = window_id(app_title);
    let entry = AppEntry::for_title(app_title);
    let runner = AppRunner::launch_with_file(&entry, file_path, vfs);
    if state.wm.get_window(&win_id).is_some() {
        raise_window(&mut state.wm, &win_id, sdi);
    } else {
        create_window(state, sdi, &win_id, app_title);
    }
    let content = &mut state.content;
    match content
        .open_runners
        .iter_mut()
        .find(|(id, _)| *id == win_id)
    {
        Some((_, slot)) => {
            let old = std::mem::replace(slot, runner);
            content.retired_runners.push(old);
        },
        None => content.open_runners.push((win_id, runner)),
    }
    state.mode = Mode::Desktop;
}

/// Title of the browser window: the page's `<title>` (or a
/// `document.title` set by script) followed by the app name, or just the
/// app name while no page title exists (the navigation controller
/// falls back to the URL for untitled pages).
pub fn browser_window_title(bw: &BrowserWidget) -> String {
    match bw.title().map(str::trim) {
        Some(t) if !t.is_empty() && Some(t) != bw.current_url() => format!("{t} - Browser"),
        _ => BROWSER_TITLE.to_string(),
    }
}

/// The skin's fade-in transition for app launches and media-tab changes.
pub fn fade_transition(state: &AppState) -> transition::TransitionState {
    transition::fade_in_custom(
        state.config.screen_width,
        state.config.screen_height,
        state.skin.features.transition_fade_frames.unwrap_or(15),
    )
}

/// Build the skin's entrance transition for boot and skin swaps.
///
/// `[transition] entrance` selects the style: "fade" (default), "assemble"
/// (PSIX-style bar slide + shrinking iris), or "none". Assemble falls back
/// to a fade under `reduced_motion`. A `[transition] easing` name overrides
/// the effect's built-in curve.
pub fn make_entrance(
    at: &ActiveTheme,
    fade_frames: u32,
    w: u32,
    h: u32,
) -> Option<transition::TransitionState> {
    let ts = match at.transition_entrance.as_str() {
        "none" => return None,
        "assemble" if !at.background_reduced_motion => transition::TransitionState::new(
            transition::TransitionEffect::Assemble,
            at.transition_entrance_frames,
            w,
            h,
        ),
        _ => transition::fade_in_custom(w, h, fade_frames),
    };
    Some(if at.transition_easing.is_empty() {
        ts
    } else {
        ts.with_easing(oasis_core::skin::theme::resolve_easing(
            &at.transition_easing,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_id_lowercases_and_joins_words() {
        assert_eq!(window_id("Photo Viewer"), "photo_viewer");
        assert_eq!(window_id("Terminal"), TERMINAL_WIN);
        assert_eq!(window_id(BROWSER_TITLE), BROWSER_WIN);
    }

    #[test]
    fn default_window_size_scales_with_the_screen() {
        let mut at = ActiveTheme::default();
        at.screen_w = 480;
        at.screen_h = 272;
        assert_eq!(default_window_size(&at), (380, 220));
        at.screen_w = 1280;
        at.screen_h = 720;
        assert_eq!(default_window_size(&at), (512, 324));
        at.screen_w = 1920;
        at.screen_h = 1080;
        assert_eq!(default_window_size(&at), (768, 486));
        // Never larger than the screen.
        at.screen_w = 320;
        at.screen_h = 200;
        assert_eq!(default_window_size(&at), (320, 200));
    }
}
