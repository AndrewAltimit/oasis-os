//! Input routing for the desktop shell: [`handle_event`] is the per-event
//! entry point. Raw keys go to window-manager shortcuts, the start menu,
//! the terminal line editor or the focused app; every other event goes to
//! the handler of the current mode (`desktop`: the window manager;
//! `dashboard`: the dashboard, the fullscreen terminal and the OSK).

use oasis_core::apps::AppAction;
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Key, KeyTwinFilter, Modifiers};
use oasis_core::sdi::SdiRegistry;
use oasis_core::startmenu::StartMenuAction;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, Mode};
use crate::launch::{BROWSER_WIN, LaunchOpts, TERMINAL_WIN};
use crate::ui_sound::UiSound;
use crate::windows::{CloseHow, close_app_window};

use crate::{icon_drag, launch, terminal_input};

mod dashboard;
mod desktop;

pub use dashboard::{handle_default_input, handle_osk_input};
pub use desktop::handle_desktop_input;

/// Point the dashboard's hover highlight / lift at the icon under the
/// pointer. `reachable` is false when something (a window) covers the
/// icons at this point; an open start menu, the media tabs, or an icon
/// drag also clear the hover.
fn update_dashboard_hover(state: &mut AppState, x: i32, y: i32, reachable: bool) {
    let target = (reachable
        && !state.ui.start_menu.open
        && state.ui.bottom_bar.active_tab == MediaTab::None
        && !icon_drag::active(state))
    .then(|| state.ui.dashboard.icon_at(x, y))
    .flatten();
    state.ui.dashboard.set_hover(target);
}

/// Launch the dashboard app at page index `idx` as a floating window.
///
/// `fade` adds the fullscreen fade transition (used from dashboard mode
/// where the screen is otherwise idle; desktop mode skips it so open
/// windows aren't covered by the overlay).
fn launch_dashboard_icon(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
    idx: usize,
    fade: bool,
) {
    state.ui.dashboard.selected = idx;
    let Some(app) = state.ui.dashboard.selected_app().cloned() else {
        return;
    };
    let opts = if fade {
        LaunchOpts::USER
    } else {
        LaunchOpts::USER_NO_FADE
    };
    launch::launch(state, sdi, vfs, &app, opts);
}

/// Where leaving the fullscreen terminal lands: back on the desktop while
/// windows are on screen (the terminal can be entered from there, e.g. an
/// app's "switch to terminal"), otherwise the dashboard. The dashboard
/// mode does not route input to windows, so returning there with windows
/// visible would leave them painted but unusable.
fn home_mode(state: &AppState) -> Mode {
    let windows_visible = state
        .wm
        .windows()
        .iter()
        .any(|w| w.state != oasis_core::wm::window::WindowState::Minimized);
    if windows_visible {
        Mode::Desktop
    } else {
        Mode::Dashboard
    }
}

/// Drive the open start menu with a gamepad-style button (d-pad moves the
/// highlight, Confirm picks, Cancel closes).
fn start_menu_button(
    btn: Button,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) -> InputResult {
    if matches!(btn, Button::Up | Button::Down) {
        state.ui_sounds.push_nav(state.frame_counter);
    }
    let action = state.ui.start_menu.handle_input(&btn);
    if action == StartMenuAction::Exit {
        return InputResult::Quit;
    }
    if action != StartMenuAction::None {
        handle_start_menu_action(&action, state, sdi, vfs);
    }
    InputResult::Continue
}

/// Whether the start menu is open and receiving the keyboard.
fn start_menu_focused(state: &AppState) -> bool {
    state.ui.start_menu.open && matches!(state.mode, Mode::Dashboard | Mode::Desktop)
}

/// Result of handling a single input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputResult {
    Continue,
    Quit,
}

/// End the pointer drags when the mode changed since `seen` (and record
/// the new mode): an icon drag and a WM window drag / resize are finished
/// where they are. The release handlers only run in the mode the drag
/// started in, so a drag interrupted by a mode switch (Start to the
/// terminal, an app launch, a remote `open`) used to survive it and resume
/// on the next pointer motion with no button held.
pub fn end_drags_on_mode_change(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
    seen: &mut Mode,
) {
    if state.mode == *seen {
        return;
    }
    *seen = state.mode;
    icon_drag::end_in_place(state, vfs);
    if state.wm.is_dragging() {
        // The WM has no cancel: a release ends the drag in place.
        state
            .wm
            .handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, sdi);
    }
}

/// Close every open app that asked to close itself outside an input
/// handler ([`AppRunner::take_close_request`], e.g. the Text Editor after
/// "Save & close" is written). Call once per frame after the runners'
/// `apply_vfs_ops` and `tick`.
///
/// [`AppRunner::take_close_request`]: oasis_core::apps::AppRunner::take_close_request
pub fn apply_app_close_requests(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    let closing: Vec<String> = state
        .content
        .open_runners
        .iter_mut()
        .filter_map(|(id, runner)| runner.take_close_request().then(|| id.clone()))
        .collect();
    for id in closing {
        apply_window_action(AppAction::Exit, id, state, sdi, vfs);
    }
}

/// Apply an [`AppAction`] returned by the windowed runner `active_id`.
fn apply_window_action(
    action: AppAction,
    active_id: String,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    match action {
        AppAction::Exit => {
            close_app_window(state, sdi, vfs, &active_id, CloseHow::AppExit);
        },
        AppAction::RequestFullscreen => {
            if state.content.fullscreen_app.is_none() {
                let _ = state.wm.enter_fullscreen(&active_id, sdi);
                state.content.fullscreen_app = Some(active_id);
            }
        },
        AppAction::LaunchAppWithFile {
            app_title,
            file_path,
        } => {
            launch::launch_for_file(state, sdi, vfs, &app_title, &file_path);
        },
        AppAction::None => {},
    }
}

/// Whether the current input target is a text-entry surface: the terminal
/// (fullscreen or windowed), the browser while its URL bar / a form field
/// has focus, or an app whose [`App::accepts_text`] is true.
///
/// [`App::accepts_text`]: oasis_core::apps::App::accepts_text
pub fn text_focus(state: &AppState) -> bool {
    match state.mode {
        Mode::Terminal => true,
        Mode::Desktop => match state.wm.active_window() {
            Some(TERMINAL_WIN) => true,
            Some(BROWSER_WIN) => state
                .content
                .browser
                .as_ref()
                .is_some_and(|bw| bw.accepts_text()),
            Some(active_id) => state
                .content
                .open_runners
                .iter()
                .any(|(id, runner)| id == active_id && runner.accepts_text()),
            None => false,
        },
        Mode::Dashboard | Mode::Osk => false,
    }
}

/// Route a raw key press to the focused app's [`App::handle_key`].
///
/// Returns `true` when the app consumed the key (its action has been
/// applied and the key's gamepad-style twin must be dropped).
///
/// [`App::handle_key`]: oasis_core::apps::App::handle_key
fn route_key(
    key: &Key,
    mods: Modifiers,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> bool {
    if state.mode != Mode::Desktop {
        return false;
    }
    // The browser takes its own keyboard shortcuts (reload, history, URL
    // bar, paging); everything else reaches it as the legacy twin events.
    if state.wm.active_window() == Some(BROWSER_WIN) {
        return state
            .content
            .browser
            .as_mut()
            .is_some_and(|bw| bw.handle_key(*key, mods, vfs));
    }
    // The browser and terminal windows are not `App`s; they keep using the
    // legacy events.
    let Some(active_id) = state
        .wm
        .active_window()
        .filter(|id| *id != BROWSER_WIN && *id != TERMINAL_WIN)
        .map(str::to_string)
    else {
        return false;
    };
    let Some((_, runner)) = state
        .content
        .open_runners
        .iter_mut()
        .find(|(id, _)| *id == active_id)
    else {
        return false;
    };
    let Some(action) = runner.handle_key(key, mods, vfs) else {
        return false;
    };
    apply_window_action(action, active_id, state, sdi, vfs);
    true
}

/// Keyboard window management in desktop mode. Returns `true` when the
/// key was a window-management shortcut (and has been handled):
///
/// | Shortcut | Action |
/// |---|---|
/// | Alt+Tab / Alt+Shift+Tab | Cycle focus forward / backward (minimized windows skipped) |
/// | Super+Left/Right or Ctrl+Alt+Left/Right | Snap active window to that half (opposite half unsnaps) |
/// | Super+Up or Ctrl+Alt+Up | Maximize active window |
/// | Super+Down or Ctrl+Alt+Down | Restore a maximized/snapped window, else minimize |
/// | Super+T or Ctrl+Alt+T | Cycle tiling layouts (last step returns to floating) |
///
/// Every combo includes Ctrl, Alt or Super, so a text-entry window keeps
/// receiving plain Tab, arrows and letters. Ctrl+Alt+Arrow exists because
/// desktop OSes commonly swallow Super+Arrow before it reaches the app.
pub fn handle_wm_shortcut(
    key: &Key,
    mods: Modifiers,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
) -> bool {
    use oasis_core::wm::KeyboardSnapDirection as Dir;

    if !mods.has_command() || !state.skin.features.window_manager {
        return false;
    }
    // A kiosk-fullscreen app owns the whole screen (and its keys).
    if state.content.fullscreen_app.is_some() {
        return false;
    }
    let alt_only = mods.only(Modifiers::ALT) || mods.only(Modifiers::ALT | Modifiers::SHIFT);
    let wm_mod = mods.only(Modifiers::SUPER) || mods.only(Modifiers::CTRL | Modifiers::ALT);

    match key {
        Key::Tab if alt_only => {
            // Modal dialogs keep focus until dismissed.
            if !state.wm.has_modal() {
                state.wm.cycle_focus(!mods.shift(), sdi);
            }
            true
        },
        Key::Left | Key::Right | Key::Up | Key::Down if wm_mod => {
            let dir = match key {
                Key::Left => Dir::Left,
                Key::Right => Dir::Right,
                Key::Up => Dir::Up,
                _ => Dir::Down,
            };
            if let Some(active) = state.wm.active_window().map(str::to_string) {
                state.wm.keyboard_snap_window(&active, dir, sdi);
            }
            true
        },
        Key::Char('t') if wm_mod => {
            let msg = match state.wm.cycle_tiling(sdi) {
                Some(layout) => format!("Tiling: {layout:?}"),
                None => "Tiling off".to_string(),
            };
            state.toasts.show(
                msg,
                oasis_core::toast::ToastLevel::Info,
                state.active_theme.toast.ttl,
            );
            true
        },
        _ => false,
    }
}

/// Top-level per-event entry point used by the main loop.
///
/// Handles [`InputEvent::Key`] (routing it to the focused app and arming
/// `key_filter` so the key's gamepad-style twin is dropped when the app
/// consumed it or when it merely typed text into a text-entry target),
/// then dispatches every other event to the handler for the current mode.
pub fn handle_event(
    event: &InputEvent,
    key_filter: &mut KeyTwinFilter,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    if key_filter.should_drop(event) {
        return InputResult::Continue;
    }
    if let InputEvent::Key { key, mods } = event {
        // Window-management shortcuts win over the focused app, like an
        // OS-level hotkey. They all need Ctrl/Alt/Super, so plain keys
        // (Tab, arrows, letters) still reach text-entry windows.
        if state.mode == Mode::Desktop && handle_wm_shortcut(key, *mods, state, sdi) {
            key_filter.suppress_twin(*key, *mods);
            return InputResult::Continue;
        }
        // An open start menu takes the keyboard: the key's gamepad twin
        // (arrows, Enter, Escape) navigates it instead of the focused app.
        if start_menu_focused(state) {
            return InputResult::Continue;
        }
        // Terminal line-editing shortcuts (Home/End/Delete, Ctrl+A/E/K/...,
        // Ctrl+R search). Their twins (e.g. Ctrl+E's R-trigger) are dropped.
        if terminal_input::focused(state)
            && terminal_input::handle_key(*key, *mods, state, sdi, vfs)
        {
            key_filter.suppress_twin(*key, *mods);
            return InputResult::Continue;
        }
        let typing = key.produces_text(*mods) && text_focus(state);
        let consumed = route_key(key, *mods, state, sdi, vfs);
        if consumed || typing {
            key_filter.suppress_twin(*key, *mods);
        }
        return InputResult::Continue;
    }
    match state.mode {
        Mode::Osk => handle_osk_input(event, state, sdi),
        Mode::Desktop => handle_desktop_input(event, state, sdi, vfs),
        Mode::Dashboard | Mode::Terminal => handle_default_input(event, state, sdi, vfs),
    }
}

/// Dispatch a start menu action (launch app, open terminal).
fn handle_start_menu_action(
    action: &StartMenuAction,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    match action {
        // Any registered app, whether or not the skin's dashboard shows a
        // tile for it (a lookup in the dashboard tiles dropped the pick
        // silently otherwise).
        StartMenuAction::LaunchApp(title) => {
            if let Err(msg) = launch::open_by_title(state, sdi, vfs, title, None, LaunchOpts::USER)
            {
                log::warn!("start menu: {msg}");
            }
        },
        StartMenuAction::OpenTerminal => {
            state.mode = Mode::Terminal;
            state.ui_sounds.push(UiSound::Open);
        },
        StartMenuAction::Exit => {
            log::info!("Start menu: Exit requested");
        },
        StartMenuAction::RunCommand(_) | StartMenuAction::None => {},
    }
}

#[cfg(test)]
mod tests;
