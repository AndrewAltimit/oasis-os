//! Opening app windows: the one launch path every caller goes through
//! (dashboard click / Confirm, start menu, `OASIS_APP`, MCP, the browser
//! IPC, "open with" hand-offs between apps).

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppAction, AppRunner};
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
pub const BROWSER_TITLE: &str = oasis_apps::titles::BROWSER;
/// Dashboard title of the terminal.
pub const TERMINAL_TITLE: &str = oasis_apps::titles::TERMINAL;

pub use oasis_core::apps::window_id;

/// Size of a newly opened app window on the current screen (see
/// [`oasis_core::apps::default_window_size`], shared with the WASM host).
pub fn default_window_size(at: &ActiveTheme) -> (u32, u32) {
    oasis_core::apps::default_window_size(at.screen_w, at.screen_h)
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
pub(crate) fn raise_window(wm: &mut WindowManager, id: &str, sdi: &mut SdiRegistry) {
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
/// fullscreen (Terminal mode) instead. The app is built from the
/// registry (`ContentLayer::apps`: built-in and plugin apps).
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
            let runner = AppRunner::launch(&state.content.apps, app, vfs);
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

/// [`launch`] the app titled `title` (no dashboard tile needed; a
/// registered app launches at its registered path).
pub fn launch_title(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    title: &str,
    opts: LaunchOpts,
) {
    let entry = entry_for_title(state, title);
    launch(state, sdi, vfs, &entry, opts);
}

/// Launch entry for `title`: the registered app's own entry (title and
/// path), or a bare tile-less entry for host-provided apps (Browser) and
/// unknown titles.
fn entry_for_title(state: &AppState, title: &str) -> AppEntry {
    state
        .content
        .apps
        .get(title)
        .map_or_else(|| AppEntry::for_title(title), |reg| reg.to_app_entry())
}

/// Open `file_path` in the app titled `app_title` (File Manager handing
/// off to Photo Viewer / Music Player / Text Editor, MCP `open_app` with
/// a file) and switch to Desktop mode.
///
/// An already-open window of that app is reused (opening five PNGs in a
/// row keeps one Photo Viewer window): its runner is replaced by one with
/// the file pre-loaded, and the old runner is retired so its backend
/// resources (textures) are released like on a close. Replacing it is a
/// close, so the old runner is asked first like on the titlebar close
/// button: one that declines (a Text Editor with unsaved changes, now
/// showing its Save / Discard / Cancel prompt) is kept and raised, and the
/// file is not opened. Returns whether the file was opened.
pub fn launch_for_file(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    app_title: &str,
    file_path: &str,
) -> bool {
    let win_id = window_id(app_title);
    if let Some((_, old)) = state
        .content
        .open_runners
        .iter_mut()
        .find(|(id, _)| *id == win_id)
        && old.request_close(vfs) != AppAction::Exit
    {
        raise_window(&mut state.wm, &win_id, sdi);
        state.mode = Mode::Desktop;
        return false;
    }
    let entry = entry_for_title(state, app_title);
    let runner = AppRunner::launch_with_file(&state.content.apps, &entry, file_path, vfs);
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
    true
}

/// Every title [`open_by_title`] accepts: the registry (built-in and
/// plugin apps) plus the host's Browser.
pub fn launchable_titles(state: &AppState) -> Vec<String> {
    let mut titles: Vec<String> = state.content.apps.titles().map(str::to_string).collect();
    titles.push(BROWSER_TITLE.to_string());
    titles
}

/// Open the app titled `title` (ASCII case-insensitive), optionally with
/// `file` pre-opened: the path behind `open <app> [file]`, MCP `open_app`
/// and `OASIS_APP`. Returns the app's exact title, or a message naming
/// what was wrong (unknown app, or one that does not open files).
pub fn open_by_title(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    title: &str,
    file: Option<&str>,
    opts: LaunchOpts,
) -> Result<String, String> {
    let (exact, opens_files) = if title.eq_ignore_ascii_case(BROWSER_TITLE) {
        (BROWSER_TITLE.to_string(), false)
    } else {
        let Some(reg) = state.content.apps.find(title) else {
            return Err(format!(
                "unknown app '{title}'. Available: {}",
                launchable_titles(state).join(", ")
            ));
        };
        (reg.title.clone(), reg.opens_files())
    };
    match file {
        Some(_) if !opens_files => Err(format!("{exact} does not open files")),
        Some(file) => {
            if launch_for_file(state, sdi, vfs, &exact, file) {
                Ok(exact)
            } else {
                Err(format!(
                    "{exact} is asking for confirmation (e.g. unsaved changes);                      {file} was not opened"
                ))
            }
        },
        None => {
            launch_title(state, sdi, vfs, &exact, opts);
            Ok(exact)
        },
    }
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
    fn window_ids_of_the_host_windows() {
        assert_eq!(window_id(TERMINAL_TITLE), TERMINAL_WIN);
        assert_eq!(window_id(BROWSER_TITLE), BROWSER_WIN);
    }

    #[test]
    fn default_window_size_follows_the_theme_screen() {
        let mut at = ActiveTheme::default();
        at.screen_w = 1280;
        at.screen_h = 720;
        assert_eq!(default_window_size(&at), (512, 324));
    }
}
