use oasis_core::apps::AppAction;
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Key, KeyTwinFilter, Modifiers, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::sdi::SdiRegistry;
use oasis_core::startmenu::StartMenuAction;
use oasis_core::transition;
use oasis_core::ui_sound::UiSound;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wm::manager::WmEvent;

use crate::app_state::{AppState, Mode};
use crate::launch::{BROWSER_WIN, LaunchOpts, TERMINAL_WIN};
use crate::windows::{CloseHow, close_app_window};
use oasis_core::terminal_sdi;

use crate::{icon_drag, launch, terminal_input};

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

/// Overlay a fade on dashboard page changes for skins with
/// `[transition] page_style = "fade"` (the icon slide is suppressed by
/// `DashboardConfig::page_style`). Default "slide" skins are untouched.
fn apply_page_change_fade(state: &mut AppState) {
    if state.active_theme.transition_page_style == "fade" {
        state.active_transition = Some(transition::fade_in_custom(
            state.config.screen_width,
            state.config.screen_height,
            state.active_theme.page_slide_duration.max(1),
        ));
    }
}

/// Id of the window whose titlebar close button is under `(x, y)` (only
/// the topmost window at that point counts, and only while no other
/// window is modal: the WM ignores clicks outside a modal).
fn close_button_under(state: &AppState, x: i32, y: i32) -> Option<String> {
    let id = state.wm.window_at(x, y)?;
    if state.wm.topmost_modal().is_some_and(|m| m != id) {
        return None;
    }
    let win = state.wm.get_window(id).filter(|w| !w.fullscreen_kiosk)?;
    let (bx, by, bw, bh) = win.close_btn_rect(state.wm.theme())?;
    let hit = x >= bx && y >= by && x < bx + bw as i32 && y < by + bh as i32;
    hit.then(|| id.to_string())
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

/// Handle input in OSK mode.
pub fn handle_osk_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
) -> InputResult {
    if let Some(ref mut osk_state) = state.osk {
        match event {
            InputEvent::Quit => return InputResult::Quit,
            InputEvent::Backspace => {
                osk_state.buffer.pop();
            },
            InputEvent::ButtonPress(btn) => {
                osk_state.handle_input(btn);
                let for_terminal = osk_state.config.title == TERMINAL_OSK_TITLE;
                let back_to = if for_terminal {
                    Mode::Terminal
                } else {
                    Mode::Dashboard
                };
                if let Some(text) = osk_state.confirmed_text() {
                    if for_terminal {
                        // The OSK edited the prompt line: put it back.
                        state.terminal.session.set_line(text);
                    } else {
                        state.terminal.print(format!("[OSK] Input: {text}"));
                        terminal_sdi::trim_scrollback(&mut state.terminal.output_lines);
                    }
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = back_to;
                } else if osk_state.is_cancelled() {
                    if !for_terminal {
                        state.terminal.print("[OSK] Cancelled");
                        terminal_sdi::trim_scrollback(&mut state.terminal.output_lines);
                    }
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = back_to;
                }
            },
            _ => {},
        }
    }
    InputResult::Continue
}

/// OSK title for the general-purpose keyboard (result echoed to the
/// terminal scrollback).
const DEFAULT_OSK_TITLE: &str = "On-Screen Keyboard";
/// OSK title when it edits the terminal prompt (result returns to it).
const TERMINAL_OSK_TITLE: &str = "Terminal Input";

/// Handle input in Desktop (windowed WM) mode: one handler per event kind.
pub fn handle_desktop_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    match event {
        InputEvent::Quit => return InputResult::Quit,
        InputEvent::PointerClick { x, y } => return desktop_click(*x, *y, state, sdi, vfs),
        InputEvent::CursorMove { x, y } => desktop_cursor_move(*x, *y, state, sdi, vfs),
        InputEvent::PointerRelease { x, y } => desktop_release(*x, *y, state, sdi, vfs),
        InputEvent::ToggleFullscreen => toggle_fullscreen(state, sdi),
        // An open start menu owns the d-pad, Confirm and Cancel, ahead of
        // the focused window.
        InputEvent::ButtonPress(btn) if state.ui.start_menu.open => {
            return start_menu_button(*btn, state, sdi, vfs);
        },
        // Text typed while the menu is open is not meant for the window.
        InputEvent::TextInput(_) | InputEvent::Backspace if state.ui.start_menu.open => {},
        InputEvent::ButtonPress(Button::Cancel) => desktop_cancel(state, sdi, vfs),
        InputEvent::ButtonPress(Button::Start) if !state.skin.features.window_manager => {
            state.mode = Mode::Terminal;
        },
        // Windowed terminal: line editing (text, Backspace, Tab, d-pad,
        // Confirm, Square) goes to the shell session.
        InputEvent::TextInput(_)
        | InputEvent::Backspace
        | InputEvent::Tab
        | InputEvent::ButtonPress(_)
            if state.wm.active_window() == Some(TERMINAL_WIN)
                && terminal_input::handle_event(event, state, sdi, vfs) => {},
        InputEvent::TextInput(_) | InputEvent::Backspace => desktop_text(event, state, vfs),
        InputEvent::MouseWheel { delta } => desktop_wheel(*delta, state, vfs),
        InputEvent::ButtonPress(btn) => desktop_button(*btn, state, sdi, vfs),
        // L/R triggers switch virtual desktops.
        InputEvent::TriggerPress(Trigger::Left) => {
            state.ui.desktops.switch_prev();
        },
        InputEvent::TriggerPress(Trigger::Right) => {
            state.ui.desktops.switch_next();
        },
        _ => {},
    }
    InputResult::Continue
}

/// Desktop-mode click: start menu, desktop indicator, taskbar, titlebar
/// close button, then the window manager (window content / bare desktop).
fn desktop_click(
    x: i32,
    y: i32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    // Start menu takes priority over everything else so it stays
    // reachable while app windows are open. Skipping this block in
    // desktop mode used to silently drop start-button clicks any
    // time `state.wm.window_count() > 0`.
    if state.skin.features.start_menu && state.ui.start_menu.hit_test_button(x, y) {
        state.ui.start_menu.toggle();
        state.ui_sounds.push(UiSound::Click);
        return InputResult::Continue;
    }
    if state.ui.start_menu.open {
        if let Some(action) = state.ui.start_menu.hit_test_item(x, y) {
            state.ui.start_menu.close();
            if action == StartMenuAction::Exit {
                return InputResult::Quit;
            }
            handle_start_menu_action(&action, state, sdi, vfs);
        } else {
            state.ui.start_menu.close();
        }
        return InputResult::Continue;
    }
    // Check desktop indicator hit (prev/next arrows).
    if let Some(hit) = state.ui.taskbar.desktop_hit_test(x, y) {
        match hit {
            oasis_core::taskbar::DesktopHit::Prev => {
                state.ui.desktops.switch_prev();
            },
            oasis_core::taskbar::DesktopHit::Next => {
                state.ui.desktops.switch_next();
            },
        }
        return InputResult::Continue;
    }
    // Check taskbar hit before WM (taskbar sits above bottom bar).
    if let Some(win_id) = state.ui.taskbar.hit_test(x, y) {
        let win_id = win_id.to_string();
        taskbar_click(&win_id, state, sdi);
        return InputResult::Continue;
    }
    // A titlebar close button asks the app first: an editor with unsaved
    // changes answers with its prompt instead of closing (the WM would
    // otherwise drop the window, and the edits, on the spot).
    if let Some(id) = close_button_under(state, x, y) {
        close_app_window(state, sdi, vfs, &id, CloseHow::UserChrome);
        return InputResult::Continue;
    }
    match state
        .wm
        .handle_input(&InputEvent::PointerClick { x, y }, sdi)
    {
        // The close button is intercepted above; this only fires if the
        // WM's own hit test found one ours did not. The window is already
        // gone, so just clean up after it.
        WmEvent::WindowClosed(id) => {
            close_app_window(state, sdi, vfs, &id, CloseHow::AppExit);
        },
        WmEvent::ContentClick(id, lx, ly) => window_content_click(&id, lx, ly, state, sdi, vfs),
        WmEvent::DesktopClick(dx, dy) => {
            if state.wm.window_count() == 0 {
                state.mode = Mode::Dashboard;
            } else if state.ui.bottom_bar.active_tab == MediaTab::None
                && let Some(idx) = state.ui.dashboard.icon_at(dx, dy)
            {
                // Forward desktop clicks to dashboard icons.
                if state.ui.dashboard.config.free_layout {
                    icon_drag::begin(state, idx, dx, dy);
                } else {
                    // No fullscreen fade here: in desktop mode other windows
                    // are already on screen, and a fade overlay would
                    // briefly cover them. Dashboard-mode launches keep the
                    // transition because the screen is otherwise idle.
                    launch_dashboard_icon(state, sdi, vfs, idx, false);
                }
            }
        },
        _ => {},
    }
    InputResult::Continue
}

/// A click on the taskbar button of window `win_id`: minimize the active
/// window, restore a minimized one, raise any other.
fn taskbar_click(win_id: &str, state: &mut AppState, sdi: &mut SdiRegistry) {
    state.ui_sounds.push(UiSound::Click);
    if state.wm.active_window() == Some(win_id) {
        let _ = state.wm.minimize_window(win_id, sdi);
    } else if state
        .wm
        .get_window(win_id)
        .is_some_and(|w| w.state == oasis_core::wm::window::WindowState::Minimized)
    {
        let _ = state.wm.restore_window(win_id, sdi);
        let _ = state.wm.focus_window(win_id, sdi);
    } else {
        let _ = state.wm.focus_window(win_id, sdi);
    }
}

/// A click inside window `id`'s content area at window-local `(lx, ly)`.
fn window_content_click(
    id: &str,
    lx: i32,
    ly: i32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) {
    if id == BROWSER_WIN
        && let Some(ref mut bw) = state.content.browser
    {
        let abs_x = bw.window_x() + lx;
        let abs_y = bw.window_y() + ly;
        bw.handle_input(&InputEvent::PointerClick { x: abs_x, y: abs_y }, vfs);
    } else if let Some((_, runner)) = state
        .content
        .open_runners
        .iter_mut()
        .find(|(rid, _)| rid == id)
        && let Some(win) = state.wm.get_window(id)
    {
        let (_, _, cw, ch) = win.content_rect(state.wm.theme());
        let action = runner.handle_click(lx, ly, cw, ch, win.fullscreen_kiosk);
        // Apply any vfs work the click queued (e.g. file manager folder
        // navigation).
        runner.refresh_app(vfs);
        // Clicks answer with the same actions as buttons: a double-clicked
        // file opens in its app, a File > Close menu item closes the window.
        apply_window_action(action, id.to_string(), state, sdi, vfs);
    }
}

/// Desktop-mode pointer motion: hover feedback, icon drags, WM drags and
/// browser hover.
fn desktop_cursor_move(
    x: i32,
    y: i32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) {
    state.ui.taskbar.set_hover(x, y);
    state.ui.start_menu.set_hover(x, y);
    // Desktop icons sit behind windows: only hover them where the pointer
    // reaches the bare desktop.
    let on_desktop = oasis_core::wm::hit_test::hit_test(state.wm.windows(), x, y, state.wm.theme())
        == oasis_core::wm::HitRegion::Desktop;
    update_dashboard_hover(state, x, y, on_desktop);
    if icon_drag::active(state) {
        // Desktop-icon drag in progress: the press already missed every
        // window, so the WM has nothing to track.
        icon_drag::on_move(state, x, y);
        return;
    }
    state.wm.handle_input(&InputEvent::CursorMove { x, y }, sdi);
    // Hover (links, status-bar URL, JS mouseover) and wheel routing to
    // nested scroll containers need the pointer.
    if !state.wm.is_dragging()
        && state.wm.window_at(x, y) == Some(BROWSER_WIN)
        && let Some(ref mut bw) = state.content.browser
    {
        bw.handle_input(&InputEvent::CursorMove { x, y }, vfs);
    }
}

/// Desktop-mode pointer release: finish an icon drag or a WM drag.
fn desktop_release(
    x: i32,
    y: i32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) {
    if icon_drag::active(state) {
        if let icon_drag::ReleaseAction::Launch(idx) = icon_drag::on_release(state, vfs, x, y) {
            launch_dashboard_icon(state, sdi, vfs, idx, false);
        }
        return;
    }
    state
        .wm
        .handle_input(&InputEvent::PointerRelease { x, y }, sdi);
}

/// Toggle kiosk fullscreen for the active window (or leave it).
fn toggle_fullscreen(state: &mut AppState, sdi: &mut SdiRegistry) {
    if let Some(id) = state.content.fullscreen_app.take() {
        let _ = state.wm.exit_fullscreen(&id, sdi);
    } else if let Some(active_id) = state.wm.active_window().map(str::to_string) {
        let _ = state.wm.enter_fullscreen(&active_id, sdi);
        state.content.fullscreen_app = Some(active_id);
    }
}

/// Desktop-mode Cancel: the focused app handles it (and may answer Exit);
/// the browser ends typing first; other windows close.
fn desktop_cancel(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    let Some(active_id) = state.wm.active_window().map(str::to_string) else {
        if let Some(top) = state.wm.topmost_visible().map(str::to_string) {
            // Nothing focused (e.g. after a click on the bare desktop) but
            // windows are still on screen: focus the top one, so the next
            // Cancel reaches it. Switching to the dashboard here would
            // leave the windows painted but dead to input.
            let _ = state.wm.focus_window(&top, sdi);
        } else {
            state.mode = Mode::Dashboard;
        }
        return;
    };
    // App windows own Cancel: it backs out of dialogs, leaves viewers,
    // untunes the TV and raises the Text Editor's unsaved-changes prompt.
    // The window only closes when the app answers `Exit` (which every app
    // does at top level). The browser and the windowed terminal are not
    // `App`s and keep the plain close-on-Cancel behavior.
    if active_id != BROWSER_WIN
        && active_id != TERMINAL_WIN
        && let Some((_, runner)) = state
            .content
            .open_runners
            .iter_mut()
            .find(|(id, _)| *id == active_id)
    {
        let action = runner.handle_input(&Button::Cancel, vfs);
        apply_window_action(action, active_id, state, sdi, vfs);
        return;
    }
    // While the user is typing in the browser (URL bar or a page text
    // field), Cancel ends the typing instead of closing the window.
    if active_id == BROWSER_WIN
        && let Some(ref mut bw) = state.content.browser
        && bw.accepts_text()
    {
        bw.handle_input(&InputEvent::ButtonPress(Button::Cancel), vfs);
        return;
    }
    close_app_window(state, sdi, vfs, &active_id, CloseHow::UserChrome);
}

/// Typed text / Backspace for the focused browser or app window (the
/// windowed terminal is handled before this).
fn desktop_text(event: &InputEvent, state: &mut AppState, vfs: &mut MemoryVfs) {
    match state.wm.active_window() {
        Some(BROWSER_WIN) => {
            if let Some(ref mut bw) = state.content.browser {
                bw.handle_input(event, vfs);
            }
        },
        Some(TERMINAL_WIN) | None => {},
        Some(active_id) => {
            if let Some((_, runner)) = state
                .content
                .open_runners
                .iter_mut()
                .find(|(id, _)| id == active_id)
            {
                match event {
                    InputEvent::TextInput(ch) => runner.handle_text_input(*ch),
                    InputEvent::Backspace => runner.handle_backspace(),
                    _ => {},
                }
            }
        },
    }
}

/// The wheel scrolls the window under the pointer (like desktop OSes),
/// falling back to the focused window when the pointer is over the bare
/// desktop. Scrolling never changes focus.
fn desktop_wheel(delta: i32, state: &mut AppState, vfs: &mut MemoryVfs) {
    let (px, py) = (state.ui.mouse_cursor.x, state.ui.mouse_cursor.y);
    let target = state
        .wm
        .window_at(px, py)
        .or_else(|| state.wm.active_window())
        .map(str::to_string);
    match target.as_deref() {
        Some(BROWSER_WIN) => {
            if let Some(ref mut bw) = state.content.browser {
                bw.handle_input(&InputEvent::MouseWheel { delta }, vfs);
            }
        },
        Some(TERMINAL_WIN) => terminal_input::scroll(state, delta),
        _ => {},
    }
}

/// Any other button press goes to the focused browser or app window.
fn desktop_button(btn: Button, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    let Some(active_id) = state.wm.active_window().map(str::to_string) else {
        return;
    };
    if active_id == BROWSER_WIN {
        if let Some(ref mut bw) = state.content.browser {
            bw.handle_input(&InputEvent::ButtonPress(btn), vfs);
        }
    } else if let Some((_, runner)) = state
        .content
        .open_runners
        .iter_mut()
        .find(|(id, _)| *id == active_id)
    {
        let action = runner.handle_input(&btn, vfs);
        apply_window_action(action, active_id, state, sdi, vfs);
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
        AppAction::SwitchToTerminal => {
            state.mode = Mode::Terminal;
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
    runner.refresh_app(vfs);
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

/// Handle input in Dashboard/Terminal modes and global keys.
pub fn handle_default_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    match event {
        InputEvent::Quit => return InputResult::Quit,
        // An open start menu owns the d-pad, Confirm and Cancel (Cancel
        // closes it): checked before the dashboard's own Confirm (launch
        // the selected icon) and Cancel (quit) bindings.
        InputEvent::ButtonPress(btn)
            if state.mode == Mode::Dashboard && state.ui.start_menu.open =>
        {
            return start_menu_button(*btn, state, sdi, vfs);
        },
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Dashboard => {
            return InputResult::Quit;
        },

        // Launch app from dashboard as floating window.
        InputEvent::ButtonPress(Button::Confirm) if state.mode == Mode::Dashboard => {
            state.ui.dashboard.trigger_press_flash();
            if state.ui.bottom_bar.active_tab == MediaTab::None
                && let Some(app) = state.ui.dashboard.selected_app().cloned()
            {
                launch::launch(state, sdi, vfs, &app, LaunchOpts::USER);
            }
        },

        // Pointer click on dashboard: start menu takes priority.
        InputEvent::PointerClick { x, y } if state.mode == Mode::Dashboard => {
            if state.ui.start_menu.hit_test_button(*x, *y) {
                state.ui.start_menu.toggle();
                state.ui_sounds.push(UiSound::Click);
                return InputResult::Continue;
            }
            if state.ui.start_menu.open {
                if let Some(action) = state.ui.start_menu.hit_test_item(*x, *y) {
                    state.ui.start_menu.close();
                    if action == StartMenuAction::Exit {
                        return InputResult::Quit;
                    }
                    handle_start_menu_action(&action, state, sdi, vfs);
                } else {
                    state.ui.start_menu.close();
                }
                return InputResult::Continue;
            }
            if state.ui.bottom_bar.active_tab == MediaTab::None
                && let Some(idx) = state.ui.dashboard.icon_at(*x, *y)
            {
                if state.ui.dashboard.config.free_layout {
                    // Free layout: arm a drag; the release decides between
                    // drop-commit, select, and launch.
                    icon_drag::begin(state, idx, *x, *y);
                } else {
                    launch_dashboard_icon(state, sdi, vfs, idx, true);
                }
            }
        },

        // Free-layout icon drag tracking (no-ops when nothing is armed).
        InputEvent::CursorMove { x, y } if state.mode == Mode::Dashboard => {
            icon_drag::on_move(state, *x, *y);
            update_dashboard_hover(state, *x, *y, true);
            // Hover focus (B6): in free layout the selection follows the
            // pointer, driving the existing focus_scale / focus_glow /
            // selection-highlight micro-motion. Grid skins keep their
            // click/d-pad selection unchanged.
            if state.ui.dashboard.config.free_layout
                && !icon_drag::active(state)
                && let Some(idx) = state.ui.dashboard.icon_at(*x, *y)
            {
                state.ui.dashboard.selected = idx;
            }
        },
        InputEvent::PointerRelease { x, y } if state.mode == Mode::Dashboard => {
            if let icon_drag::ReleaseAction::Launch(idx) = icon_drag::on_release(state, vfs, *x, *y)
            {
                launch_dashboard_icon(state, sdi, vfs, idx, true);
            }
        },

        InputEvent::ButtonPress(Button::Start) => {
            state.mode = match state.mode {
                Mode::Dashboard => Mode::Terminal,
                Mode::Terminal => home_mode(state),
                Mode::Osk => Mode::Osk,
                Mode::Desktop => Mode::Desktop,
            };
        },
        InputEvent::ButtonPress(Button::Select) if state.mode != Mode::Osk => {
            // Opened from the terminal, the OSK edits the prompt line and
            // hands the result back to it (see `handle_osk_input`).
            let from_terminal = state.mode == Mode::Terminal;
            let (title, initial) = if from_terminal {
                (
                    TERMINAL_OSK_TITLE,
                    state.terminal.session.buffer().to_string(),
                )
            } else {
                (DEFAULT_OSK_TITLE, String::new())
            };
            let osk_cfg = OskConfig {
                title: title.to_string(),
                ..OskConfig::for_screen(state.active_theme.screen_w, state.active_theme.screen_h)
            };
            state.osk = Some(OskState::new(osk_cfg, &initial));
            state.mode = Mode::Osk;
            log::info!("OSK opened");
        },

        // L trigger: cycle top tabs (status bar).
        InputEvent::TriggerPress(Trigger::Left) if state.mode == Mode::Dashboard => {
            state.ui.status_bar.next_tab();
            state.ui.bottom_bar.l_pressed = true;
        },
        InputEvent::TriggerRelease(Trigger::Left) => {
            state.ui.bottom_bar.l_pressed = false;
        },

        // R trigger: cycle media category tabs (bottom bar).
        InputEvent::TriggerPress(Trigger::Right) if state.mode == Mode::Dashboard => {
            state.ui.bottom_bar.next_tab();
            state.ui.bottom_bar.r_pressed = true;
            state.active_transition = Some(transition::fade_in_custom(
                state.config.screen_width,
                state.config.screen_height,
                state.skin.features.transition_fade_frames.unwrap_or(15),
            ));
        },
        InputEvent::TriggerRelease(Trigger::Right) => {
            state.ui.bottom_bar.r_pressed = false;
        },

        // Dashboard input: D-pad navigation.
        InputEvent::ButtonPress(btn) if state.mode == Mode::Dashboard => match btn {
            Button::Up | Button::Down | Button::Left | Button::Right
                if state.ui.bottom_bar.active_tab == MediaTab::None =>
            {
                state.ui.dashboard.handle_input(btn);
                state.ui_sounds.push_nav(state.frame_counter);
            },
            Button::Triangle if state.ui.bottom_bar.active_tab == MediaTab::None => {
                state.ui.dashboard.next_page();
                state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                state.ui_sounds.push_nav(state.frame_counter);
                apply_page_change_fade(state);
            },
            Button::Square if state.ui.bottom_bar.active_tab == MediaTab::None => {
                state.ui.dashboard.prev_page();
                state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                state.ui_sounds.push_nav(state.frame_counter);
                apply_page_change_fade(state);
            },
            _ => {},
        },

        // Terminal input: line editing (text, Backspace, Tab, d-pad,
        // Confirm, Square) goes to the shell session.
        InputEvent::TextInput(_)
        | InputEvent::Backspace
        | InputEvent::Tab
        | InputEvent::ButtonPress(_)
            if state.mode == Mode::Terminal
                && terminal_input::handle_event(event, state, sdi, vfs) => {},
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Terminal => {
            terminal_sdi::set_terminal_visible(sdi, false);
            state.mode = home_mode(state);
            state.ui_sounds.push(UiSound::Close);
        },

        InputEvent::MouseWheel { delta } if state.mode == Mode::Terminal => {
            terminal_input::scroll(state, *delta);
        },

        _ => {},
    }
    InputResult::Continue
}

/// Dispatch a start menu action (launch app, open terminal).
fn handle_start_menu_action(
    action: &StartMenuAction,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    match action {
        StartMenuAction::LaunchApp(title) => {
            let app = state.ui.dashboard.apps.iter().find(|a| a.title == *title);
            if let Some(app) = app.cloned() {
                launch::launch(state, sdi, vfs, &app, LaunchOpts::USER);
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
mod tests {
    use super::*;
    use oasis_core::active_theme::ActiveTheme;

    #[test]
    fn input_result_variants() {
        let cont = InputResult::Continue;
        let quit = InputResult::Quit;
        assert_ne!(cont, quit);
    }

    #[test]
    fn input_result_equality() {
        assert_eq!(InputResult::Continue, InputResult::Continue);
        assert_eq!(InputResult::Quit, InputResult::Quit);
    }

    #[test]
    fn input_result_debug() {
        assert_eq!(format!("{:?}", InputResult::Continue), "Continue");
        assert_eq!(format!("{:?}", InputResult::Quit), "Quit");
    }

    #[test]
    fn input_result_clone() {
        let a = InputResult::Continue;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn input_result_copy() {
        let a = InputResult::Quit;
        let b = a;
        // Both usable after copy.
        assert_eq!(a, InputResult::Quit);
        assert_eq!(b, InputResult::Quit);
    }

    // Integration tests driving the real input handlers with an AppState.

    fn make_test_state() -> (AppState, SdiRegistry, MemoryVfs) {
        (
            AppState::new_for_test(),
            SdiRegistry::new(),
            MemoryVfs::new(),
        )
    }

    // -- handle_default_input --

    #[test]
    fn quit_event_returns_quit() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        let result = handle_default_input(&InputEvent::Quit, &mut state, &mut sdi, &mut vfs);
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn cancel_in_dashboard_returns_quit() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        let result = handle_default_input(
            &InputEvent::ButtonPress(Button::Cancel),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn start_toggles_dashboard_terminal() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        let result = handle_default_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Terminal);

        let result = handle_default_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn select_opens_osk() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        let result = handle_default_input(
            &InputEvent::ButtonPress(Button::Select),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Osk);
        assert!(state.osk.is_some());
    }

    #[test]
    fn terminal_text_input() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        handle_default_input(&InputEvent::TextInput('h'), &mut state, &mut sdi, &mut vfs);
        handle_default_input(&InputEvent::TextInput('i'), &mut state, &mut sdi, &mut vfs);
        assert_eq!(state.terminal.session.buffer(), "hi");
    }

    #[test]
    fn terminal_backspace() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("abc");
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert_eq!(state.terminal.session.buffer(), "ab");
    }

    #[test]
    fn terminal_confirm_executes_command() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("echo hello");
        handle_default_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Input buffer should be cleared.
        assert!(state.terminal.session.buffer().is_empty());
        // The command prompt should be in output.
        assert!(
            state
                .terminal
                .output_lines
                .iter()
                .any(|l| l.contains("> echo hello"))
        );
    }

    #[test]
    fn terminal_confirm_empty_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("");
        handle_default_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Empty command should not add to output.
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn terminal_cancel_returns_to_dashboard() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        // First create terminal objects so set_terminal_visible can hide them.
        terminal_sdi::setup_terminal_objects(
            &mut sdi,
            &[],
            "/",
            "",
            0,
            &ActiveTheme::default(),
            true,
        );
        handle_default_input(
            &InputEvent::ButtonPress(Button::Cancel),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn terminal_square_deletes_char() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("xyz");
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.buffer(), "xy");
    }

    // -- handle_osk_input --

    #[test]
    fn osk_quit_returns_quit() {
        let (mut state, mut sdi, _vfs) = make_test_state();
        state.mode = Mode::Osk;
        state.osk = Some(OskState::new(OskConfig::default(), ""));
        let result = handle_osk_input(&InputEvent::Quit, &mut state, &mut sdi);
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn osk_backspace_removes_char() {
        let (mut state, mut sdi, _vfs) = make_test_state();
        state.mode = Mode::Osk;
        let mut osk = OskState::new(OskConfig::default(), "");
        osk.buffer = "abc".to_string();
        state.osk = Some(osk);
        handle_osk_input(&InputEvent::Backspace, &mut state, &mut sdi);
        assert_eq!(state.osk.as_ref().unwrap().buffer, "ab");
    }

    // -- handle_desktop_input --

    #[test]
    fn desktop_quit_returns_quit() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(&InputEvent::Quit, &mut state, &mut sdi, &mut vfs);
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn desktop_start_switches_to_terminal() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        // The Start-button-to-Terminal transition is gated on
        // `!skin.features.window_manager`, so disable WM for this test —
        // the `classic` skin used by the harness enables it by default.
        state.skin.features.window_manager = false;
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn desktop_confirm_routes_to_settings_runner() {
        // Regression test for the "Enter does nothing in Settings" symptom.
        // Simulates: launch Settings window, press Down to move cursor off
        // the currently-active skin, press Confirm. Verifies that the IPC
        // request reaches the runner's pending slot.
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;

        launch::launch_title(&mut state, &mut sdi, &vfs, "Settings", LaunchOpts::QUIET);
        assert_eq!(state.mode, Mode::Desktop);
        assert_eq!(state.wm.active_window(), Some("settings"));

        // Press Down so the cursor lands on a skin other than the active one.
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Down),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Press Enter.
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );

        let (_, runner) = state
            .content
            .open_runners
            .iter_mut()
            .find(|(id, _)| id == "settings")
            .expect("settings runner should exist");
        let req = runner.take_pending_request();
        let (path, data) = req.expect("Confirm should post an IPC request");
        assert_eq!(path, "/system/ipc/skin-change");
        assert!(!data.is_empty());
    }

    #[test]
    fn desktop_cancel_no_windows_returns_to_dashboard() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        // No windows open.
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Cancel),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Dashboard);
    }

    // -- dashboard d-pad navigation --

    #[test]
    fn dashboard_dpad_navigation() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        // These shouldn't panic even with empty app list.
        handle_default_input(
            &InputEvent::ButtonPress(Button::Right),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        handle_default_input(
            &InputEvent::ButtonPress(Button::Down),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        handle_default_input(
            &InputEvent::ButtonPress(Button::Left),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        handle_default_input(
            &InputEvent::ButtonPress(Button::Up),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Dashboard);
    }

    // -- Additional input dispatch tests --

    #[test]
    fn start_in_osk_mode_stays_in_osk() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Osk;
        handle_default_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Osk);
    }

    #[test]
    fn start_in_desktop_mode_stays_in_desktop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        handle_default_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Desktop);
    }

    #[test]
    fn select_in_osk_mode_does_not_reopen() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Osk;
        state.osk = Some(OskState::new(OskConfig::default(), ""));
        handle_default_input(
            &InputEvent::ButtonPress(Button::Select),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Should still be in OSK mode, not open a second one.
        assert_eq!(state.mode, Mode::Osk);
    }

    #[test]
    fn terminal_text_builds_input_buffer() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        for ch in "hello world".chars() {
            handle_default_input(&InputEvent::TextInput(ch), &mut state, &mut sdi, &mut vfs);
        }
        assert_eq!(state.terminal.session.buffer(), "hello world");
    }

    #[test]
    fn terminal_backspace_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("");
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert!(state.terminal.session.buffer().is_empty());
    }

    #[test]
    fn terminal_square_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.session.set_line("");
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.terminal.session.buffer().is_empty());
    }

    #[test]
    fn dashboard_triangle_next_page() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        // Should not panic even with zero pages.
        handle_default_input(
            &InputEvent::ButtonPress(Button::Triangle),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn dashboard_square_prev_page() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn trigger_left_cycles_status_tab() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        handle_default_input(
            &InputEvent::TriggerPress(Trigger::Left),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.ui.bottom_bar.l_pressed);
        handle_default_input(
            &InputEvent::TriggerRelease(Trigger::Left),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(!state.ui.bottom_bar.l_pressed);
    }

    #[test]
    fn trigger_right_cycles_media_tab() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        handle_default_input(
            &InputEvent::TriggerPress(Trigger::Right),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.ui.bottom_bar.r_pressed);
        assert!(state.active_transition.is_some());
        handle_default_input(
            &InputEvent::TriggerRelease(Trigger::Right),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(!state.ui.bottom_bar.r_pressed);
    }

    #[test]
    fn osk_no_state_is_noop() {
        let (mut state, mut sdi, _vfs) = make_test_state();
        state.mode = Mode::Osk;
        state.osk = None;
        let result = handle_osk_input(&InputEvent::Backspace, &mut state, &mut sdi);
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn osk_button_press_without_confirm_stays() {
        let (mut state, mut sdi, _vfs) = make_test_state();
        state.mode = Mode::Osk;
        state.osk = Some(OskState::new(OskConfig::default(), ""));
        let result = handle_osk_input(&InputEvent::ButtonPress(Button::Up), &mut state, &mut sdi);
        assert_eq!(result, InputResult::Continue);
        assert!(state.osk.is_some());
    }

    #[test]
    fn dashboard_pointer_move_updates_hover() {
        use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState};
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        let cfg = DashboardConfig::from_features(
            &oasis_core::skin::SkinFeatures::default(),
            &state.active_theme,
        );
        let apps = (0..3)
            .map(|i| AppEntry {
                title: format!("App {i}"),
                path: format!("/apps/app{i}"),
                icon_png: Vec::new(),
                color: oasis_core::backend::Color::WHITE,
            })
            .collect();
        state.ui.dashboard = DashboardState::new(cfg, apps);
        let (x, y, w, h) = state.ui.dashboard.icon_rect(1).unwrap();
        let mut move_to = |state: &mut AppState, x: i32, y: i32| {
            handle_default_input(&InputEvent::CursorMove { x, y }, state, &mut sdi, &mut vfs);
        };
        move_to(&mut state, x + w as i32 / 2, y + h as i32 / 2);
        assert_eq!(state.ui.dashboard.hover_index, Some(1));
        move_to(&mut state, -10, -10);
        assert_eq!(state.ui.dashboard.hover_index, None);
        // An open start menu takes the pointer.
        state.ui.start_menu.open = true;
        move_to(&mut state, x + w as i32 / 2, y + h as i32 / 2);
        assert_eq!(state.ui.dashboard.hover_index, None);
    }

    #[test]
    fn desktop_cursor_move_does_not_change_mode() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::CursorMove { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Desktop);
    }

    #[test]
    fn desktop_pointer_release_does_not_change_mode() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::PointerRelease { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn desktop_click_no_windows_returns_to_dashboard() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::PointerClick { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn desktop_text_input_without_browser_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        state.content.browser = None;
        let result =
            handle_desktop_input(&InputEvent::TextInput('a'), &mut state, &mut sdi, &mut vfs);
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn desktop_backspace_without_browser_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Desktop;
        state.content.browser = None;
        let result = handle_desktop_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn unhandled_event_returns_continue() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;
        let result = handle_default_input(
            &InputEvent::CursorMove { x: 0, y: 0 },
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(result, InputResult::Continue);
    }

    // -- raw Key events / typing collisions --

    /// Open `title` as a desktop window on a 4-desktop manager.
    fn open_window(title: &str) -> (AppState, SdiRegistry, MemoryVfs) {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.ui.desktops = oasis_core::wm::DesktopManager::new(4);
        launch::launch_title(&mut state, &mut sdi, &vfs, title, LaunchOpts::QUIET);
        assert_eq!(state.mode, Mode::Desktop);
        (state, sdi, vfs)
    }

    /// The event stream the SDL backend produces for typing `ch`.
    fn sdl_typing(ch: char) -> Vec<InputEvent> {
        let key = if ch == ' ' { Key::Space } else { Key::Char(ch) };
        let mut events = vec![InputEvent::Key {
            key,
            mods: Modifiers::NONE,
        }];
        events.extend(key.legacy_press(Modifiers::NONE));
        events.push(InputEvent::TextInput(ch));
        events
    }

    #[test]
    fn typing_qe_space_in_text_editor_fires_no_shortcuts() {
        let (mut state, mut sdi, mut vfs) = open_window("Text Editor");
        let win_id = state.wm.active_window().map(str::to_string);
        assert!(text_focus(&state), "text editor must accept text");
        let mut filter = KeyTwinFilter::default();
        for ch in ['q', 'e', ' '] {
            for ev in sdl_typing(ch) {
                handle_event(&ev, &mut filter, &mut state, &mut sdi, &mut vfs);
                // No trigger may switch the virtual desktop mid-stream.
                assert_eq!(state.ui.desktops.active_desktop(), 0, "{ev:?}");
            }
        }
        assert_eq!(state.mode, Mode::Desktop);
        assert_eq!(state.wm.active_window().map(str::to_string), win_id);
        let (_, runner) = state
            .content
            .open_runners
            .iter_mut()
            .find(|(id, _)| Some(id) == win_id.as_ref())
            .expect("editor window still open");
        // Sync the runner's cached display lines from the editor.
        runner.refresh_app(&vfs);
        // Space typed a space instead of Triangle opening Find mode.
        let text = runner.lines.join("\n");
        assert!(text.contains("qe "), "editor text: {text:?}");
        assert!(!text.contains("Find:"), "Triangle leaked: {text:?}");
    }

    /// Launch another desktop window into an existing state.
    fn launch_more(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs, title: &str) {
        launch::launch_title(state, sdi, vfs, title, LaunchOpts::QUIET);
    }

    /// Feed a key (plus its legacy twin, as the SDL backend does).
    fn press(
        key: Key,
        mods: Modifiers,
        filter: &mut KeyTwinFilter,
        state: &mut AppState,
        sdi: &mut SdiRegistry,
        vfs: &mut MemoryVfs,
    ) {
        let mut events = vec![InputEvent::Key { key, mods }];
        events.extend(key.legacy_press(mods));
        for ev in events {
            handle_event(&ev, filter, state, sdi, vfs);
        }
    }

    #[test]
    fn alt_tab_cycles_focus_skipping_minimized() {
        let (mut state, mut sdi, mut vfs) = open_window("Settings");
        launch_more(&mut state, &mut sdi, &vfs, "Calculator");
        launch_more(&mut state, &mut sdi, &vfs, "Text Editor");
        assert_eq!(state.wm.window_count(), 3);
        let ids: Vec<String> = state
            .wm
            .windows()
            .iter()
            .map(|w| w.id.to_string())
            .collect();
        state
            .wm
            .minimize_window(&ids[1], &mut sdi)
            .expect("minimize");
        let mut filter = KeyTwinFilter::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            press(
                Key::Tab,
                Modifiers::ALT,
                &mut filter,
                &mut state,
                &mut sdi,
                &mut vfs,
            );
            seen.push(state.wm.active_window().map(str::to_string));
        }
        assert!(seen.iter().all(|a| a.as_deref() != Some(ids[1].as_str())));
        assert!(seen.contains(&Some(ids[0].clone())));
        assert!(seen.contains(&Some(ids[2].clone())));
        // Alt+Shift+Tab goes back the other way.
        let before = state.wm.active_window().map(str::to_string);
        press(
            Key::Tab,
            Modifiers::ALT | Modifiers::SHIFT,
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_ne!(state.wm.active_window().map(str::to_string), before);
    }

    #[test]
    fn plain_tab_in_text_editor_is_not_a_wm_shortcut() {
        let (mut state, mut sdi, mut vfs) = open_window("Settings");
        launch_more(&mut state, &mut sdi, &vfs, "Text Editor");
        assert!(text_focus(&state));
        let active = state.wm.active_window().map(str::to_string);
        assert!(!handle_wm_shortcut(
            &Key::Tab,
            Modifiers::NONE,
            &mut state,
            &mut sdi
        ));
        assert!(!handle_wm_shortcut(
            &Key::Left,
            Modifiers::NONE,
            &mut state,
            &mut sdi
        ));
        assert!(!handle_wm_shortcut(
            &Key::Char('t'),
            Modifiers::SHIFT,
            &mut state,
            &mut sdi
        ));
        let mut filter = KeyTwinFilter::default();
        press(
            Key::Tab,
            Modifiers::NONE,
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.wm.active_window().map(str::to_string), active);
    }

    #[test]
    fn super_and_ctrl_alt_arrows_snap_active_window() {
        let (mut state, mut sdi, mut vfs) = open_window("Settings");
        let id = state
            .wm
            .active_window()
            .map(str::to_string)
            .expect("active");
        let area = state.wm.work_area();
        let mut filter = KeyTwinFilter::default();
        press(
            Key::Left,
            Modifiers::SUPER,
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        let w = state.wm.get_window(&id).expect("window");
        assert_eq!((w.x, w.y, w.outer_w), (0, area.y, area.w / 2));
        press(
            Key::Up,
            Modifiers::CTRL | Modifiers::ALT,
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(
            state.wm.get_window(&id).expect("window").state,
            oasis_core::wm::window::WindowState::Maximized
        );
        // Super+T toggles tiling on.
        press(
            Key::Char('t'),
            Modifiers::SUPER,
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.wm.tiling_layout().is_some());
    }

    #[test]
    fn q_without_text_focus_still_switches_desktop() {
        let (mut state, mut sdi, mut vfs) = open_window("Settings");
        assert!(!text_focus(&state));
        let mut filter = KeyTwinFilter::default();
        for ev in sdl_typing('e') {
            handle_event(&ev, &mut filter, &mut state, &mut sdi, &mut vfs);
        }
        assert_eq!(state.ui.desktops.active_desktop(), 1);
    }

    #[test]
    fn gamepad_trigger_unaffected_by_text_focus() {
        // A bare TriggerPress (PSP / gamepad, no preceding Key) is never
        // filtered, even while a text window has focus.
        let (mut state, mut sdi, mut vfs) = open_window("Text Editor");
        let mut filter = KeyTwinFilter::default();
        handle_event(
            &InputEvent::TriggerPress(Trigger::Right),
            &mut filter,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.ui.desktops.active_desktop(), 1);
    }

    /// Events a keyboard backend emits for one key press: `Key`, its
    /// gamepad-style twin (if any), then `TextInput` for typing keys.
    fn sdl_key(key: Key, mods: Modifiers) -> Vec<InputEvent> {
        let mut events = vec![InputEvent::Key { key, mods }];
        events.extend(key.legacy_press(mods));
        if key.produces_text(mods) {
            match key {
                Key::Char(c) => events.push(InputEvent::TextInput(c)),
                Key::Space => events.push(InputEvent::TextInput(' ')),
                _ => {},
            }
        }
        events
    }

    fn feed(
        events: &[InputEvent],
        filter: &mut KeyTwinFilter,
        state: &mut AppState,
        sdi: &mut SdiRegistry,
        vfs: &mut MemoryVfs,
    ) {
        for ev in events {
            handle_event(ev, filter, state, sdi, vfs);
        }
    }

    #[test]
    fn terminal_keyboard_line_editing_history_and_completion() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        oasis_core::terminal::register_builtins(&mut state.terminal.cmd_reg);
        let mut f = KeyTwinFilter::default();
        for ch in "echo wrld".chars() {
            feed(&sdl_typing(ch), &mut f, &mut state, &mut sdi, &mut vfs);
        }
        // Arrow keys arrive as Key + d-pad twin; the twin moves the cursor.
        for _ in 0..3 {
            feed(
                &sdl_key(Key::Left, Modifiers::NONE),
                &mut f,
                &mut state,
                &mut sdi,
                &mut vfs,
            );
        }
        feed(&sdl_typing('o'), &mut f, &mut state, &mut sdi, &mut vfs);
        assert_eq!(state.terminal.session.buffer(), "echo world");
        assert_eq!(state.terminal.session.cursor_col(), 7);
        feed(
            &sdl_key(Key::Home, Modifiers::NONE),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.cursor_col(), 0);
        feed(
            &sdl_key(Key::Char('e'), Modifiers::CTRL),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.cursor_col(), 10);

        // Enter runs it and records it in the persisted history file.
        feed(
            &sdl_key(Key::Enter, Modifiers::NONE),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.terminal.output_lines.iter().any(|l| l == "world"));
        let saved = oasis_core::vfs::Vfs::read(&vfs, "/home/user/.oasis_history")
            .expect("history file written");
        assert_eq!(saved, b"echo world\n");

        // Up recalls it.
        feed(
            &sdl_key(Key::Up, Modifiers::NONE),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.buffer(), "echo world");

        // Ctrl+U clears; Tab completes a command name.
        feed(
            &sdl_key(Key::Char('u'), Modifiers::CTRL),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        for ch in "hist".chars() {
            feed(&sdl_typing(ch), &mut f, &mut state, &mut sdi, &mut vfs);
        }
        feed(
            &sdl_key(Key::Tab, Modifiers::NONE),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.buffer(), "history ");

        // Ctrl+C abandons the line with a ^C echo.
        feed(
            &sdl_key(Key::Char('c'), Modifiers::CTRL),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.terminal.session.buffer().is_empty());
        assert_eq!(
            state.terminal.output_lines.last().map(String::as_str),
            Some("> history ^C")
        );
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn windowed_terminal_ctrl_e_does_not_switch_desktop() {
        let (mut state, mut sdi, mut vfs) = open_window("Terminal");
        assert_eq!(state.wm.active_window(), Some("terminal"));
        let mut f = KeyTwinFilter::default();
        for ch in "ls".chars() {
            feed(&sdl_typing(ch), &mut f, &mut state, &mut sdi, &mut vfs);
        }
        feed(
            &sdl_key(Key::Char('a'), Modifiers::CTRL),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.cursor_col(), 0);
        // Ctrl+E's twin is the R-trigger (next desktop); it must be dropped.
        feed(
            &sdl_key(Key::Char('e'), Modifiers::CTRL),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.cursor_col(), 2);
        assert_eq!(state.ui.desktops.active_desktop(), 0);
        // Gamepad d-pad still edits: Left moves the cursor.
        handle_event(
            &InputEvent::ButtonPress(Button::Left),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.session.cursor_col(), 1);
        // Confirm runs the line in the windowed terminal.
        handle_event(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut f,
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.terminal.output_lines.iter().any(|l| l == "> ls"));
    }

    #[test]
    fn terminal_background_job_runs_on_poll() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        oasis_core::terminal::register_builtins(&mut state.terminal.cmd_reg);
        crate::terminal_input::run_line("echo later &", &mut state, &mut sdi, &mut vfs);
        assert!(!state.terminal.output_lines.iter().any(|l| l == "later"));
        crate::terminal_input::poll_jobs(&mut state, &mut sdi, &mut vfs);
        assert!(state.terminal.output_lines.iter().any(|l| l == "later"));
        assert!(
            state
                .terminal
                .output_lines
                .iter()
                .any(|l| l.contains("Done") && l.contains("echo later"))
        );
    }

    #[test]
    fn terminal_sdi_get_prints_fields() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        oasis_core::terminal::register_builtins(&mut state.terminal.cmd_reg);
        sdi.create("probe").x = 42;
        crate::terminal_input::run_line("sdi get probe", &mut state, &mut sdi, &mut vfs);
        assert!(state.terminal.output_lines.iter().any(|l| l == "probe:"));
        assert!(
            state
                .terminal
                .output_lines
                .iter()
                .any(|l| l.contains("pos") && l.contains("42"))
        );
    }

    #[test]
    fn terminal_mode_has_text_focus() {
        let (mut state, _sdi, _vfs) = make_test_state();
        state.mode = Mode::Terminal;
        assert!(text_focus(&state));
        state.mode = Mode::Dashboard;
        assert!(!text_focus(&state));
    }
}
