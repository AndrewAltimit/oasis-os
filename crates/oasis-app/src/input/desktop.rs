//! Desktop (window manager) mode input.

use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::sdi::SdiRegistry;
use oasis_core::startmenu::StartMenuAction;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wm::manager::WmEvent;

use crate::app_state::{AppState, Mode};
use crate::launch::{BROWSER_WIN, TERMINAL_WIN};
use crate::ui_sound::UiSound;
use crate::windows::{CloseHow, close_app_window};

use crate::{icon_drag, terminal_input};

use super::{
    InputResult, apply_window_action, handle_start_menu_action, launch_dashboard_icon,
    start_menu_button, update_dashboard_hover,
};

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
        let action = runner.handle_click(lx, ly, cw, ch, &state.active_theme, vfs);
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
        Some(id) => {
            if let Some((_, runner)) = state
                .content
                .open_runners
                .iter_mut()
                .find(|(rid, _)| rid == id)
            {
                runner.handle_scroll(delta, vfs);
            }
        },
        None => {},
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
