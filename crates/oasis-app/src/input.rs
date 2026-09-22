use oasis_audio::RADIO_APP_TITLE;
use oasis_core::apps::{AppAction, AppRunner};
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
use oasis_core::terminal_sdi;

use crate::{commands, icon_drag, launch, terminal_input};

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
    let Some(app) = state.ui.dashboard.selected_app() else {
        return;
    };
    log::info!("Click-launching app: {}", app.title);
    let app = app.clone();
    let result = launch::launch_app_window(
        &app,
        &mut state.wm,
        sdi,
        &mut state.content.open_runners,
        &mut state.content.browser,
        &state.browser_config,
        vfs,
        &state.net.tls_provider,
        state.skin.features.window_manager,
        &state.plugin_manager,
    );
    launch::apply_launch(result, &mut state.mode);
    state.ui_sounds.push(UiSound::Open);
    if fade {
        state.active_transition = Some(launch::make_transition(
            state.config.screen_width,
            state.config.screen_height,
            state.skin.features.transition_fade_frames.unwrap_or(15),
        ));
    }
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

/// Tear down active radio playback and any pending network work.
///
/// Safe to call unconditionally: all fields are cleared idempotently.
fn stop_radio(state: &mut AppState) {
    let _ = state
        .radio_manager
        .process_request("stop", &mut state.audio_backend);
    state.archive_catalog = None;
    state.pending_catalog_fetch = None;
    state.pending_source_fetch = None;
    if let Some(mut src) = state.radio_source.take() {
        src.disconnect();
    }
}

/// Stop the radio if the closing runner is the Internet Radio app.
/// Closing the app window should also stop playback — otherwise audio
/// keeps playing with no UI to control it.
fn stop_radio_if_radio_runner(state: &mut AppState, id: &str) {
    let is_radio = state
        .content
        .open_runners
        .iter()
        .any(|(rid, runner)| rid == id && runner.title == RADIO_APP_TITLE);
    if is_radio {
        stop_radio(state);
    }
}

/// If the closing runner is the Music Player, tear down its playing
/// track. The app itself emits a `stop` VFS IPC on Cancel, but the
/// window-manager close button bypasses that path — the runner is
/// dropped before `tick()` gets another chance to read the IPC.
fn stop_music_if_music_runner(state: &mut AppState, id: &str) {
    const MUSIC_APP_TITLE: &str = "Music Player";
    let is_music = state
        .content
        .open_runners
        .iter()
        .any(|(rid, runner)| rid == id && runner.title == MUSIC_APP_TITLE);
    if is_music {
        crate::media_controller::shutdown(state);
    }
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
                if let Some(text) = osk_state.confirmed_text() {
                    state
                        .terminal
                        .output_lines
                        .push(format!("[OSK] Input: {text}"));
                    commands::trim_output(&mut state.terminal.output_lines);
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = Mode::Dashboard;
                } else if osk_state.is_cancelled() {
                    state
                        .terminal
                        .output_lines
                        .push("[OSK] Cancelled".to_string());
                    commands::trim_output(&mut state.terminal.output_lines);
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = Mode::Dashboard;
                }
            },
            _ => {},
        }
    }
    InputResult::Continue
}

/// Handle input in Desktop (windowed WM) mode.
pub fn handle_desktop_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    match event {
        InputEvent::Quit => return InputResult::Quit,
        InputEvent::PointerClick { x, y } => {
            // Start menu takes priority over everything else so it stays
            // reachable while app windows are open. Skipping this block in
            // desktop mode used to silently drop start-button clicks any
            // time `state.wm.window_count() > 0`.
            if state.skin.features.start_menu && state.ui.start_menu.hit_test_button(*x, *y) {
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
            // Check desktop indicator hit (prev/next arrows).
            if let Some(hit) = state.ui.taskbar.desktop_hit_test(*x, *y) {
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
            if let Some(win_id) = state.ui.taskbar.hit_test(*x, *y) {
                let win_id = win_id.to_string();
                state.ui_sounds.push(UiSound::Click);
                if state.wm.active_window() == Some(win_id.as_str()) {
                    // Active window -- minimize it.
                    let _ = state.wm.minimize_window(&win_id, sdi);
                } else if state
                    .wm
                    .get_window(&win_id)
                    .is_some_and(|w| w.state == oasis_core::wm::window::WindowState::Minimized)
                {
                    // Minimized -- restore and focus.
                    let _ = state.wm.restore_window(&win_id, sdi);
                } else {
                    // Inactive, visible -- bring to front.
                    let _ = state.wm.focus_window(&win_id, sdi);
                }
                return InputResult::Continue;
            }
            let wm_event = state
                .wm
                .handle_input(&InputEvent::PointerClick { x: *x, y: *y }, sdi);
            match wm_event {
                WmEvent::WindowClosed(id) => {
                    state.ui_sounds.push(UiSound::Close);
                    if state.content.fullscreen_app.as_deref() == Some(id.as_str()) {
                        state.content.fullscreen_app = None;
                    }
                    stop_radio_if_radio_runner(state, &id);
                    stop_music_if_music_runner(state, &id);
                    state.content.open_runners.retain(|(rid, _)| *rid != id);
                    if id == "browser" {
                        state.content.browser = None;
                    }
                    if state.wm.window_count() == 0 {
                        state.mode = Mode::Dashboard;
                    }
                },
                WmEvent::ContentClick(id, lx, ly) => {
                    if id == "browser"
                        && let Some(ref mut bw) = state.content.browser
                    {
                        let abs_x = bw.window_x() + lx;
                        let abs_y = bw.window_y() + ly;
                        bw.handle_input(&InputEvent::PointerClick { x: abs_x, y: abs_y }, vfs);
                    } else if let Some((_, runner)) = state
                        .content
                        .open_runners
                        .iter_mut()
                        .find(|(rid, _)| *rid == id)
                        && let Some(win) = state.wm.get_window(&id)
                    {
                        let (_, _, cw, ch) = win.content_rect(state.wm.theme());
                        let action = runner.handle_click(lx, ly, cw, ch, win.fullscreen_kiosk);
                        // Apply any vfs work the click queued (e.g. file
                        // manager folder navigation).
                        runner.refresh_app(vfs);
                        if action == AppAction::RequestFullscreen
                            && state.content.fullscreen_app.is_none()
                        {
                            let _ = state.wm.enter_fullscreen(&id, sdi);
                            state.content.fullscreen_app = Some(id.to_string());
                        }
                    }
                },
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
                            // No fullscreen fade here: in desktop mode other
                            // windows are already on screen, and a fade
                            // overlay would briefly cover them.
                            // Dashboard-mode launches keep the transition
                            // because the screen is otherwise idle.
                            launch_dashboard_icon(state, sdi, vfs, idx, false);
                        }
                    }
                },
                _ => {},
            }
        },
        InputEvent::CursorMove { x, y } => {
            state.ui.taskbar.set_hover(*x, *y);
            state.ui.start_menu.set_hover(*x, *y);
            if icon_drag::active(state) {
                // Desktop-icon drag in progress: the press already missed
                // every window, so the WM has nothing to track.
                icon_drag::on_move(state, *x, *y);
                return InputResult::Continue;
            }
            state
                .wm
                .handle_input(&InputEvent::CursorMove { x: *x, y: *y }, sdi);
        },
        InputEvent::PointerRelease { x, y } => {
            if icon_drag::active(state) {
                if let icon_drag::ReleaseAction::Launch(idx) =
                    icon_drag::on_release(state, vfs, *x, *y)
                {
                    launch_dashboard_icon(state, sdi, vfs, idx, false);
                }
                return InputResult::Continue;
            }
            state
                .wm
                .handle_input(&InputEvent::PointerRelease { x: *x, y: *y }, sdi);
        },
        InputEvent::ToggleFullscreen => {
            if let Some(ref fs_id) = state.content.fullscreen_app {
                let id = fs_id.clone();
                let _ = state.wm.exit_fullscreen(&id, sdi);
                state.content.fullscreen_app = None;
            } else if let Some(active_id) = state.wm.active_window().map(|s| s.to_string()) {
                let _ = state.wm.enter_fullscreen(&active_id, sdi);
                state.content.fullscreen_app = Some(active_id);
            }
        },
        InputEvent::ButtonPress(Button::Cancel) => {
            if let Some(active_id) = state.wm.active_window().map(|s| s.to_string()) {
                state.ui_sounds.push(UiSound::Close);
                // If closing the fullscreen window, clear fullscreen state first.
                if state.content.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                    let _ = state.wm.exit_fullscreen(&active_id, sdi);
                    state.content.fullscreen_app = None;
                }
                let _ = state.wm.close_window(&active_id, sdi);
                stop_radio_if_radio_runner(state, &active_id);
                stop_music_if_music_runner(state, &active_id);
                state
                    .content
                    .open_runners
                    .retain(|(rid, _)| *rid != active_id);
                if active_id == "browser" {
                    state.content.browser = None;
                }
                if state.wm.window_count() == 0 {
                    state.mode = Mode::Dashboard;
                }
            } else {
                state.mode = Mode::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Start) if !state.skin.features.window_manager => {
            state.mode = Mode::Terminal;
        },
        // Windowed terminal: line editing (text, Backspace, Tab, d-pad,
        // Confirm, Square) goes to the shell session.
        InputEvent::TextInput(_)
        | InputEvent::Backspace
        | InputEvent::Tab
        | InputEvent::ButtonPress(_)
            if state.wm.active_window() == Some("terminal")
                && terminal_input::handle_event(event, state, sdi, vfs) => {},
        InputEvent::TextInput(ch) => match state.wm.active_window() {
            Some("browser") => {
                if let Some(ref mut bw) = state.content.browser {
                    bw.handle_input(&InputEvent::TextInput(*ch), vfs);
                }
            },
            Some("terminal") => {},
            Some(active_id) => {
                if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == active_id)
                {
                    runner.handle_text_input(*ch);
                }
            },
            None => {},
        },
        InputEvent::Backspace => match state.wm.active_window() {
            Some("browser") => {
                if let Some(ref mut bw) = state.content.browser {
                    bw.handle_input(&InputEvent::Backspace, vfs);
                }
            },
            Some("terminal") => {},
            Some(active_id) => {
                if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == active_id)
                {
                    runner.handle_backspace();
                }
            },
            None => {},
        },
        InputEvent::MouseWheel { delta } => {
            match state.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = state.content.browser {
                        bw.handle_input(&InputEvent::MouseWheel { delta: *delta }, vfs);
                    }
                },
                Some("terminal") => {
                    let len = state.terminal.output_lines.len() + 1; // +1 for prompt
                    let max_visible = terminal_sdi::visible_output_lines(&state.active_theme);
                    if len > max_visible {
                        let max_offset = len - max_visible;
                        if *delta < 0 {
                            state.terminal.scroll_offset = (state.terminal.scroll_offset
                                + (-*delta as usize) * 3)
                                .min(max_offset);
                        } else {
                            state.terminal.scroll_offset = state
                                .terminal
                                .scroll_offset
                                .saturating_sub(*delta as usize * 3);
                        }
                    }
                },
                _ => {},
            }
        },
        InputEvent::ButtonPress(btn) => {
            if let Some(active_id) = state.wm.active_window().map(|s| s.to_string()) {
                if active_id == "browser" {
                    if let Some(ref mut bw) = state.content.browser {
                        bw.handle_input(&InputEvent::ButtonPress(*btn), vfs);
                    }
                } else if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| *id == active_id)
                {
                    let action = runner.handle_input(btn, vfs);
                    apply_window_action(action, active_id, state, sdi, vfs);
                }
            }
        },
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

/// Handle input in App (fullscreen) mode.
pub fn handle_app_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) -> InputResult {
    if let Some(ref mut runner) = state.content.app_runner {
        match event {
            InputEvent::Quit => return InputResult::Quit,
            InputEvent::ButtonPress(btn) => {
                let action = runner.handle_input(btn, vfs);
                apply_fullscreen_action(action, state, sdi, vfs);
            },
            _ => {},
        }
    }
    InputResult::Continue
}

/// Close every open app that asked to close itself outside an input
/// handler ([`AppRunner::take_close_request`], e.g. the Text Editor after
/// "Save & close" is written). Call once per frame after the runners'
/// `apply_vfs_ops` and `tick`.
pub fn apply_app_close_requests(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    if state
        .content
        .app_runner
        .as_mut()
        .is_some_and(AppRunner::take_close_request)
    {
        apply_fullscreen_action(AppAction::Exit, state, sdi, vfs);
    }
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

/// Apply an [`AppAction`] returned by the fullscreen (`Mode::App`) runner.
fn apply_fullscreen_action(
    action: AppAction,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    let (is_radio, is_music) = state
        .content
        .app_runner
        .as_ref()
        .map_or((false, false), |r| {
            (r.title == RADIO_APP_TITLE, r.title == "Music Player")
        });
    match action {
        AppAction::Exit => {
            state.ui_sounds.push(UiSound::Close);
            AppRunner::hide_sdi(sdi);
            state.content.app_runner = None;
            state.mode = Mode::Dashboard;
            if is_radio {
                stop_radio(state);
            }
            if is_music {
                crate::media_controller::shutdown(state);
            }
        },
        AppAction::SwitchToTerminal => {
            AppRunner::hide_sdi(sdi);
            state.content.app_runner = None;
            state.mode = Mode::Terminal;
        },
        AppAction::LaunchAppWithFile {
            app_title,
            file_path,
        } => {
            // Replace the current fullscreen runner with the
            // target app, with the file pre-opened.
            AppRunner::hide_sdi(sdi);
            let entry = oasis_core::dashboard::AppEntry {
                title: app_title.clone(),
                path: format!("/apps/{app_title}"),
                icon_png: Vec::new(),
                color: oasis_core::backend::Color::rgb(100, 100, 100),
            };
            state.content.app_runner = Some(AppRunner::launch_with_file(&entry, &file_path, vfs));
        },
        AppAction::RequestFullscreen | AppAction::None => {},
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
            state.ui_sounds.push(UiSound::Close);
            if state.content.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                let _ = state.wm.exit_fullscreen(&active_id, sdi);
                state.content.fullscreen_app = None;
            }
            let _ = state.wm.close_window(&active_id, sdi);
            stop_radio_if_radio_runner(state, &active_id);
            stop_music_if_music_runner(state, &active_id);
            state
                .content
                .open_runners
                .retain(|(rid, _)| *rid != active_id);
            if state.wm.window_count() == 0 {
                state.mode = Mode::Dashboard;
            }
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
            launch::launch_app_window_for_file(
                &app_title,
                &file_path,
                &mut state.wm,
                sdi,
                &mut state.content.open_runners,
                vfs,
            );
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
        Mode::App => state
            .content
            .app_runner
            .as_ref()
            .is_some_and(AppRunner::accepts_text),
        Mode::Desktop => match state.wm.active_window() {
            Some("terminal") => true,
            Some("browser") => state
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
    match state.mode {
        Mode::App => {
            let Some(runner) = state.content.app_runner.as_mut() else {
                return false;
            };
            let Some(action) = runner.handle_key(key, mods, vfs) else {
                return false;
            };
            apply_fullscreen_action(action, state, sdi, vfs);
            true
        },
        Mode::Desktop => {
            // The browser and terminal windows are not `App`s; they keep
            // using the legacy events.
            let Some(active_id) = state
                .wm
                .active_window()
                .filter(|id| *id != "browser" && *id != "terminal")
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
        },
        Mode::Dashboard | Mode::Terminal | Mode::Osk => false,
    }
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
        Mode::App => handle_app_input(event, state, sdi, vfs),
        _ => handle_default_input(event, state, sdi, vfs),
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
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Dashboard => {
            return InputResult::Quit;
        },

        // Launch app from dashboard as floating window.
        InputEvent::ButtonPress(Button::Confirm) if state.mode == Mode::Dashboard => {
            state.ui.dashboard.trigger_press_flash();
            if state.ui.bottom_bar.active_tab == MediaTab::None
                && let Some(app) = state.ui.dashboard.selected_app()
            {
                log::info!("Launching app: {}", app.title);
                let app = app.clone();
                let result = launch::launch_app_window(
                    &app,
                    &mut state.wm,
                    sdi,
                    &mut state.content.open_runners,
                    &mut state.content.browser,
                    &state.browser_config,
                    vfs,
                    &state.net.tls_provider,
                    state.skin.features.window_manager,
                    &state.plugin_manager,
                );
                launch::apply_launch(result, &mut state.mode);
                state.ui_sounds.push(UiSound::Open);
                state.active_transition = Some(launch::make_transition(
                    state.config.screen_width,
                    state.config.screen_height,
                    state.skin.features.transition_fade_frames.unwrap_or(15),
                ));
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
                Mode::Terminal => Mode::Dashboard,
                Mode::App => Mode::App,
                Mode::Osk => Mode::Osk,
                Mode::Desktop => Mode::Desktop,
            };
        },
        InputEvent::ButtonPress(Button::Select) if state.mode != Mode::Osk => {
            let osk_cfg = OskConfig {
                title: "On-Screen Keyboard".to_string(),
                ..OskConfig::for_screen(state.active_theme.screen_w, state.active_theme.screen_h)
            };
            state.osk = Some(OskState::new(osk_cfg, ""));
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

        // Start menu intercepts input when open.
        InputEvent::ButtonPress(btn)
            if state.mode == Mode::Dashboard && state.ui.start_menu.open =>
        {
            if matches!(btn, Button::Up | Button::Down) {
                state.ui_sounds.push_nav(state.frame_counter);
            }
            let action = state.ui.start_menu.handle_input(btn);
            if action == StartMenuAction::Exit {
                return InputResult::Quit;
            }
            if action != StartMenuAction::None {
                handle_start_menu_action(&action, state, sdi, vfs);
            }
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
            state.mode = Mode::Dashboard;
            state.ui_sounds.push(UiSound::Close);
        },

        InputEvent::MouseWheel { delta } if state.mode == Mode::Terminal => {
            let len = state.terminal.output_lines.len();
            let max_visible = terminal_sdi::visible_output_lines(&state.active_theme);
            if len > max_visible {
                let max_offset = len - max_visible;
                if *delta < 0 {
                    // Scroll up (show older lines).
                    state.terminal.scroll_offset =
                        (state.terminal.scroll_offset + (-*delta as usize) * 3).min(max_offset);
                } else {
                    // Scroll down (show newer lines).
                    state.terminal.scroll_offset = state
                        .terminal
                        .scroll_offset
                        .saturating_sub(*delta as usize * 3);
                }
            }
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
            if let Some(app) = app {
                let app = app.clone();
                let result = launch::launch_app_window(
                    &app,
                    &mut state.wm,
                    sdi,
                    &mut state.content.open_runners,
                    &mut state.content.browser,
                    &state.browser_config,
                    vfs,
                    &state.net.tls_provider,
                    state.skin.features.window_manager,
                    &state.plugin_manager,
                );
                launch::apply_launch(result, &mut state.mode);
                state.ui_sounds.push(UiSound::Open);
                state.active_transition = Some(launch::make_transition(
                    state.config.screen_width,
                    state.config.screen_height,
                    15,
                ));
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

    // Integration tests using make_test_state from commands::tests.
    // These test the actual input handlers with real AppState.

    fn make_test_state() -> (AppState, SdiRegistry, MemoryVfs) {
        use oasis_audio::RadioManager;
        use oasis_backend_sdl::SdlAudioBackend;
        use oasis_core::active_theme::ActiveTheme;
        use oasis_core::bottombar::BottomBar;
        use oasis_core::browser::BrowserConfig;
        use oasis_core::config::OasisConfig;
        use oasis_core::cursor::CursorState;
        use oasis_core::dashboard::{DashboardConfig, DashboardState};
        use oasis_core::net::{RustlsTlsProvider, StdNetworkBackend};
        use oasis_core::platform::DesktopPlatform;
        use oasis_core::skin::SkinFeatures;
        use oasis_core::skin::builtin::load_builtin;
        use oasis_core::startmenu::StartMenuState;
        use oasis_core::statusbar::StatusBar;
        use oasis_core::terminal::CommandRegistry;
        use oasis_core::wm::manager::WindowManager;

        use crate::app_state::{ContentLayer, NetworkLayer, TerminalLayer, UiLayer};

        let skin = load_builtin("classic").unwrap();
        let active_theme = ActiveTheme::from_skin(&skin.theme);
        let dash_cfg = DashboardConfig::from_features(&SkinFeatures::default(), &active_theme);

        let state = AppState {
            config: OasisConfig::default(),
            skin,
            active_theme: active_theme.clone(),
            browser_config: BrowserConfig::default(),
            platform: DesktopPlatform::new(),
            ui: UiLayer {
                dashboard: DashboardState::new(dash_cfg, vec![]),
                status_bar: StatusBar::new(),
                bottom_bar: BottomBar::new(),
                taskbar: oasis_core::taskbar::Taskbar::new(),
                start_menu: StartMenuState::new(StartMenuState::default_items(&active_theme)),
                mouse_cursor: CursorState::default(),
                desktops: oasis_core::wm::DesktopManager::new(1),
            },
            terminal: TerminalLayer {
                cmd_reg: CommandRegistry::new(),
                cwd: "/".to_string(),
                session: oasis_core::terminal::ShellSession::new(),
                output_lines: Vec::new(),
                scroll_offset: 0,
                dirty: true,
                sync_signature: None,
                sdi_signature: None,
            },
            net: NetworkLayer {
                backend: StdNetworkBackend::new(),
                listener: None,
                ftp_server: None,
                remote_client: None,
                tls_provider: RustlsTlsProvider::new(),
            },
            content: ContentLayer {
                app_runner: None,
                open_runners: Vec::new(),
                browser: None,
                fullscreen_app: None,
            },
            osk: None,
            plugin_manager: oasis_core::plugin::PluginManager::new(),
            wm: WindowManager::new(480, 272),
            mode: Mode::Dashboard,
            bg_color: oasis_core::backend::Color::rgb(0, 0, 0),
            active_transition: None,
            frame_counter: 0,
            pending_wallpaper_refresh: false,
            skin_layout_textures: Vec::new(),
            image_layers: Vec::new(),
            background_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            chrome_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            icon_drag: None,
            cursor_texture: None,
            settings: oasis_core::settings::SettingsStore::new(),
            radio_manager: RadioManager::new(),
            radio_source: None,
            archive_catalog: None,
            pending_catalog_fetch: None,
            pending_source_fetch: None,
            audio_backend: SdlAudioBackend::new(),
            toasts: oasis_core::toast::ToastManager::new(),
            ui_sounds: oasis_core::ui_sound::UiSoundQueue::new(),
            sfx: oasis_audio::sfx::SfxPlayer::new(),
            pending_tv_catalog_fetch: None,
            tv_fetch_start: None,
            video_player: crate::video_player::VideoPlayer::new(),
            tv_audio_track: None,
            media_track: None,
            tv_audio_chunks_fed: 0,
            tv_audio_samples_fed: 0,
            #[cfg(feature = "_video")]
            pending_video_download: None,
            #[cfg(feature = "_video")]
            tv_video_cache_path: None,
            #[cfg(feature = "_video")]
            pending_video_params: None,
            #[cfg(feature = "_video")]
            tv_download_progress: None,
            #[cfg(feature = "_video")]
            tv_video_cache: Vec::new(),
            #[cfg(feature = "_video")]
            tv_stream_session: None,
            #[cfg(feature = "_video")]
            tv_current_url: None,
            #[cfg(feature = "mcp")]
            mcp: None,
            #[cfg(feature = "mcp")]
            agent_activity: crate::mcp_tools::AgentActivity::default(),
        };
        let sdi = SdiRegistry::new();
        let vfs = MemoryVfs::new();
        (state, sdi, vfs)
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

    // -- handle_app_input --

    #[test]
    fn app_no_runner_continues() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::App;
        state.content.app_runner = None;
        // Without a runner, all events (including Quit) are no-ops.
        let result = handle_app_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(result, InputResult::Continue);
        let result = handle_app_input(&InputEvent::Quit, &mut state, &mut sdi, &mut vfs);
        assert_eq!(result, InputResult::Continue);
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
        use crate::launch;
        use oasis_core::dashboard::AppEntry;

        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Dashboard;

        let app = AppEntry {
            title: "Settings".to_string(),
            path: "/apps/settings".to_string(),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(100, 100, 100),
        };
        let result = launch::launch_app_window(
            &app,
            &mut state.wm,
            &mut sdi,
            &mut state.content.open_runners,
            &mut state.content.browser,
            &state.browser_config,
            &vfs,
            &state.net.tls_provider,
            state.skin.features.window_manager,
            &state.plugin_manager,
        );
        launch::apply_launch(result, &mut state.mode);
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
    fn start_in_app_mode_stays_in_app() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::App;
        handle_default_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::App);
    }

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
        use oasis_core::dashboard::AppEntry;

        let (mut state, mut sdi, vfs) = make_test_state();
        state.ui.desktops = oasis_core::wm::DesktopManager::new(4);
        let app = AppEntry {
            title: title.to_string(),
            path: format!("/apps/{title}"),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(100, 100, 100),
        };
        let result = launch::launch_app_window(
            &app,
            &mut state.wm,
            &mut sdi,
            &mut state.content.open_runners,
            &mut state.content.browser,
            &state.browser_config,
            &vfs,
            &state.net.tls_provider,
            state.skin.features.window_manager,
            &state.plugin_manager,
        );
        launch::apply_launch(result, &mut state.mode);
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
            .find(|(id, _)| Some(&*id) == win_id.as_ref())
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
        use oasis_core::dashboard::AppEntry;

        let app = AppEntry {
            title: title.to_string(),
            path: format!("/apps/{title}"),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(100, 100, 100),
        };
        let result = launch::launch_app_window(
            &app,
            &mut state.wm,
            sdi,
            &mut state.content.open_runners,
            &mut state.content.browser,
            &state.browser_config,
            vfs,
            &state.net.tls_provider,
            state.skin.features.window_manager,
            &state.plugin_manager,
        );
        launch::apply_launch(result, &mut state.mode);
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
