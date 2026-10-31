//! Input routing tests against a real `AppState`.

use super::*;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::input::Trigger;
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::terminal_sdi;

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
    terminal_sdi::setup_terminal_objects(&mut sdi, &[], "/", "", 0, &ActiveTheme::default(), true);
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
    let mut reqs = Vec::new();
    runner.drain_requests(&mut reqs);
    let req = reqs.pop().expect("Confirm should post an IPC request");
    let (path, data) = (req.path, req.data);
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
    let result = handle_desktop_input(&InputEvent::TextInput('a'), &mut state, &mut sdi, &mut vfs);
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
    // Space typed a space instead of Triangle opening Find mode.
    let text = runner.lines().join("\n");
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
