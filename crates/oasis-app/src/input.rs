use oasis_core::apps::{AppAction, AppRunner};
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::sdi::SdiRegistry;
use oasis_core::startmenu::StartMenuAction;
use oasis_core::terminal::Environment;
use oasis_core::transition;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wm::manager::WmEvent;

use crate::app_state::{AppState, Mode};
use crate::{commands, launch, terminal_sdi};

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
                    state.output_lines.push(format!("[OSK] Input: {text}"));
                    commands::trim_output(&mut state.output_lines);
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = Mode::Dashboard;
                } else if osk_state.is_cancelled() {
                    state.output_lines.push("[OSK] Cancelled".to_string());
                    commands::trim_output(&mut state.output_lines);
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
    vfs: &MemoryVfs,
) -> InputResult {
    match event {
        InputEvent::Quit => return InputResult::Quit,
        InputEvent::PointerClick { x, y } => {
            let wm_event = state
                .wm
                .handle_input(&InputEvent::PointerClick { x: *x, y: *y }, sdi);
            match wm_event {
                WmEvent::WindowClosed(id) => {
                    state.open_runners.retain(|(rid, _)| *rid != id);
                    if id == "browser" {
                        state.browser = None;
                    }
                    if state.wm.window_count() == 0 {
                        state.mode = Mode::Dashboard;
                    }
                },
                WmEvent::ContentClick(id, lx, ly) => {
                    if id == "browser"
                        && let Some(ref mut bw) = state.browser
                    {
                        let abs_x = bw.window_x() + lx;
                        let abs_y = bw.window_y() + ly;
                        bw.handle_input(&InputEvent::PointerClick { x: abs_x, y: abs_y }, vfs);
                    }
                },
                WmEvent::DesktopClick(_, _) => {
                    if state.wm.window_count() == 0 {
                        state.mode = Mode::Dashboard;
                    }
                },
                _ => {},
            }
        },
        InputEvent::CursorMove { x, y } => {
            state
                .wm
                .handle_input(&InputEvent::CursorMove { x: *x, y: *y }, sdi);
        },
        InputEvent::PointerRelease { x, y } => {
            state
                .wm
                .handle_input(&InputEvent::PointerRelease { x: *x, y: *y }, sdi);
        },
        InputEvent::ButtonPress(Button::Cancel) => {
            if let Some(active_id) = state.wm.active_window().map(|s| s.to_string()) {
                let _ = state.wm.close_window(&active_id, sdi);
                state.open_runners.retain(|(rid, _)| *rid != active_id);
                if active_id == "browser" {
                    state.browser = None;
                }
                if state.wm.window_count() == 0 {
                    state.mode = Mode::Dashboard;
                }
            } else {
                state.mode = Mode::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Start) => {
            state.mode = Mode::Terminal;
        },
        InputEvent::TextInput(ch) => {
            if state.wm.active_window() == Some("browser")
                && let Some(ref mut bw) = state.browser
            {
                bw.handle_input(&InputEvent::TextInput(*ch), vfs);
            }
        },
        InputEvent::Backspace => {
            if state.wm.active_window() == Some("browser")
                && let Some(ref mut bw) = state.browser
            {
                bw.handle_input(&InputEvent::Backspace, vfs);
            }
        },
        InputEvent::ButtonPress(btn) => {
            if let Some(active_id) = state.wm.active_window().map(|s| s.to_string()) {
                if active_id == "browser" {
                    if let Some(ref mut bw) = state.browser {
                        bw.handle_input(&InputEvent::ButtonPress(*btn), vfs);
                    }
                } else if let Some((_, runner)) = state
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| *id == active_id)
                {
                    match runner.handle_input(btn, vfs) {
                        AppAction::Exit => {
                            let _ = state.wm.close_window(&active_id, sdi);
                            state.open_runners.retain(|(rid, _)| *rid != active_id);
                            if state.wm.window_count() == 0 {
                                state.mode = Mode::Dashboard;
                            }
                        },
                        AppAction::SwitchToTerminal => {
                            state.mode = Mode::Terminal;
                        },
                        AppAction::None => {},
                    }
                }
            }
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
    if let Some(ref mut runner) = state.app_runner {
        match event {
            InputEvent::Quit => return InputResult::Quit,
            InputEvent::ButtonPress(btn) => match runner.handle_input(btn, vfs) {
                AppAction::Exit => {
                    AppRunner::hide_sdi(sdi);
                    state.app_runner = None;
                    state.mode = Mode::Dashboard;
                },
                AppAction::SwitchToTerminal => {
                    AppRunner::hide_sdi(sdi);
                    state.app_runner = None;
                    state.mode = Mode::Terminal;
                },
                AppAction::None => {},
            },
            _ => {},
        }
    }
    InputResult::Continue
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
            if state.bottom_bar.active_tab == MediaTab::None
                && let Some(app) = state.dashboard.selected_app()
            {
                log::info!("Launching app: {}", app.title);
                let app = app.clone();
                let result = launch::launch_app_window(
                    &app,
                    &mut state.wm,
                    sdi,
                    &mut state.open_runners,
                    &mut state.browser,
                    &state.browser_config,
                    vfs,
                    &state.tls_provider,
                );
                launch::apply_launch(result, &mut state.mode);
                state.active_transition = Some(launch::make_transition(
                    state.config.screen_width,
                    state.config.screen_height,
                    state.skin.features.transition_fade_frames.unwrap_or(15),
                ));
            }
        },

        // Pointer click on dashboard: start menu takes priority.
        InputEvent::PointerClick { x, y } if state.mode == Mode::Dashboard => {
            if state.start_menu.hit_test_button(*x, *y) {
                state.start_menu.toggle();
                return InputResult::Continue;
            }
            if state.start_menu.open {
                if let Some(action) = state.start_menu.hit_test_item(*x, *y) {
                    state.start_menu.close();
                    if action == StartMenuAction::Exit {
                        return InputResult::Quit;
                    }
                    handle_start_menu_action(&action, state, sdi, vfs);
                } else {
                    state.start_menu.close();
                }
                return InputResult::Continue;
            }
            if state.bottom_bar.active_tab == MediaTab::None {
                let cfg = &state.dashboard.config;
                let gx = *x - cfg.grid_x;
                let gy = *y - cfg.grid_y;
                if gx >= 0 && gy >= 0 {
                    let col = gx as usize / cfg.cell_w as usize;
                    let row = gy as usize / cfg.cell_h as usize;
                    if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                        let idx = row * cfg.grid_cols as usize + col;
                        let page_apps = state.dashboard.current_page_apps().len();
                        if idx < page_apps {
                            if state.dashboard.selected == idx {
                                if let Some(app) = state.dashboard.selected_app() {
                                    log::info!("Click-launching app: {}", app.title);
                                    let app = app.clone();
                                    let result = launch::launch_app_window(
                                        &app,
                                        &mut state.wm,
                                        sdi,
                                        &mut state.open_runners,
                                        &mut state.browser,
                                        &state.browser_config,
                                        vfs,
                                        &state.tls_provider,
                                    );
                                    launch::apply_launch(result, &mut state.mode);
                                    state.active_transition = Some(launch::make_transition(
                                        state.config.screen_width,
                                        state.config.screen_height,
                                        state.skin.features.transition_fade_frames.unwrap_or(15),
                                    ));
                                }
                            } else {
                                state.dashboard.selected = idx;
                            }
                        }
                    }
                }
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
        InputEvent::ButtonPress(Button::Select) => {
            if state.mode != Mode::Osk {
                let osk_cfg = OskConfig {
                    title: "On-Screen Keyboard".to_string(),
                    ..OskConfig::default()
                };
                state.osk = Some(OskState::new(osk_cfg, ""));
                state.mode = Mode::Osk;
                log::info!("OSK opened");
            }
        },

        // L trigger: cycle top tabs (status bar).
        InputEvent::TriggerPress(Trigger::Left) if state.mode == Mode::Dashboard => {
            state.status_bar.next_tab();
            state.bottom_bar.l_pressed = true;
        },
        InputEvent::TriggerRelease(Trigger::Left) => {
            state.bottom_bar.l_pressed = false;
        },

        // R trigger: cycle media category tabs (bottom bar).
        InputEvent::TriggerPress(Trigger::Right) if state.mode == Mode::Dashboard => {
            state.bottom_bar.next_tab();
            state.bottom_bar.r_pressed = true;
            state.active_transition = Some(transition::fade_in_custom(
                state.config.screen_width,
                state.config.screen_height,
                state.skin.features.transition_fade_frames.unwrap_or(15),
            ));
        },
        InputEvent::TriggerRelease(Trigger::Right) => {
            state.bottom_bar.r_pressed = false;
        },

        // Start menu intercepts input when open.
        InputEvent::ButtonPress(btn) if state.mode == Mode::Dashboard && state.start_menu.open => {
            let action = state.start_menu.handle_input(btn);
            if action == StartMenuAction::Exit {
                return InputResult::Quit;
            }
            if action != StartMenuAction::None {
                handle_start_menu_action(&action, state, sdi, vfs);
            }
        },

        // Dashboard input: D-pad navigation.
        InputEvent::ButtonPress(btn) if state.mode == Mode::Dashboard => match btn {
            Button::Up | Button::Down | Button::Left | Button::Right => {
                if state.bottom_bar.active_tab == MediaTab::None {
                    state.dashboard.handle_input(btn);
                }
            },
            Button::Triangle => {
                if state.bottom_bar.active_tab == MediaTab::None {
                    state.dashboard.next_page();
                    state.bottom_bar.current_page = state.dashboard.page;
                }
            },
            Button::Square => {
                if state.bottom_bar.active_tab == MediaTab::None {
                    state.dashboard.prev_page();
                    state.bottom_bar.current_page = state.dashboard.page;
                }
            },
            _ => {},
        },

        // Terminal input.
        InputEvent::TextInput(ch) if state.mode == Mode::Terminal => {
            state.input_buf.push(*ch);
        },
        InputEvent::Backspace if state.mode == Mode::Terminal => {
            state.input_buf.pop();
        },
        InputEvent::ButtonPress(Button::Confirm) if state.mode == Mode::Terminal => {
            let line = state.input_buf.clone();
            state.input_buf.clear();
            if !line.is_empty() {
                state.output_lines.push(format!("> {line}"));
                let pending_skin_swap;
                {
                    let mut env = Environment {
                        cwd: state.cwd.clone(),
                        vfs,
                        power: Some(&state.platform),
                        time: Some(&state.platform),
                        usb: Some(&state.platform),
                        network: None,
                        tls: Some(&state.tls_provider),
                        stdin: None,
                        stderr: String::new(),
                    };
                    let result = state.cmd_reg.execute(&line, &mut env);
                    state.cwd = env.cwd;
                    pending_skin_swap = commands::process_command_output(result, state);
                }
                if let Some(name) = pending_skin_swap {
                    commands::apply_skin_swap(&name, state, sdi, vfs);
                }
            }
            commands::trim_output(&mut state.output_lines);
        },
        InputEvent::ButtonPress(Button::Square) if state.mode == Mode::Terminal => {
            state.input_buf.pop();
        },
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Terminal => {
            terminal_sdi::set_terminal_visible(sdi, false);
            state.mode = Mode::Dashboard;
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
            let app = state.dashboard.apps.iter().find(|a| a.title == *title);
            if let Some(app) = app {
                let app = app.clone();
                let result = launch::launch_app_window(
                    &app,
                    &mut state.wm,
                    sdi,
                    &mut state.open_runners,
                    &mut state.browser,
                    &state.browser_config,
                    vfs,
                    &state.tls_provider,
                );
                launch::apply_launch(result, &mut state.mode);
                state.active_transition = Some(launch::make_transition(
                    state.config.screen_width,
                    state.config.screen_height,
                    15,
                ));
            }
        },
        StartMenuAction::OpenTerminal => {
            state.mode = Mode::Terminal;
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

        let skin = load_builtin("terminal").unwrap();
        let active_theme = ActiveTheme::from_skin(&skin.theme);
        let dash_cfg = DashboardConfig::from_features(&SkinFeatures::default(), &active_theme);

        let state = AppState {
            config: OasisConfig::default(),
            skin,
            active_theme: active_theme.clone(),
            browser_config: BrowserConfig::default(),
            platform: DesktopPlatform::new(),
            dashboard: DashboardState::new(dash_cfg, vec![]),
            status_bar: StatusBar::new(),
            bottom_bar: BottomBar::new(),
            start_menu: StartMenuState::new(StartMenuState::default_items()),
            cmd_reg: CommandRegistry::new(),
            cwd: "/".to_string(),
            input_buf: String::new(),
            output_lines: Vec::new(),
            osk: None,
            app_runner: None,
            wm: WindowManager::new(480, 272),
            open_runners: Vec::new(),
            browser: None,
            net_backend: StdNetworkBackend::new(),
            listener: None,
            ftp_server: None,
            remote_client: None,
            tls_provider: RustlsTlsProvider::new(),
            mouse_cursor: CursorState::default(),
            mode: Mode::Dashboard,
            bg_color: oasis_core::backend::Color::rgb(0, 0, 0),
            active_transition: None,
            frame_counter: 0,
            radio_manager: RadioManager::new(),
            radio_source: None,
            audio_backend: SdlAudioBackend::new(),
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
        assert_eq!(state.input_buf, "hi");
    }

    #[test]
    fn terminal_backspace() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.input_buf = "abc".to_string();
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert_eq!(state.input_buf, "ab");
    }

    #[test]
    fn terminal_confirm_executes_command() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.input_buf = "echo hello".to_string();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Input buffer should be cleared.
        assert!(state.input_buf.is_empty());
        // The command prompt should be in output.
        assert!(
            state
                .output_lines
                .iter()
                .any(|l| l.contains("> echo hello"))
        );
    }

    #[test]
    fn terminal_confirm_empty_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.input_buf.clear();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Empty command should not add to output.
        assert!(state.output_lines.is_empty());
    }

    #[test]
    fn terminal_cancel_returns_to_dashboard() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        // First create terminal objects so set_terminal_visible can hide them.
        terminal_sdi::setup_terminal_objects(&mut sdi, &[], "/", "");
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
        state.input_buf = "xyz".to_string();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.input_buf, "xy");
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
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::App;
        state.app_runner = None;
        // Without a runner, all events (including Quit) are no-ops.
        let result = handle_app_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(result, InputResult::Continue);
        let result = handle_app_input(&InputEvent::Quit, &mut state, &mut sdi, &vfs);
        assert_eq!(result, InputResult::Continue);
    }

    // -- handle_desktop_input --

    #[test]
    fn desktop_quit_returns_quit() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(&InputEvent::Quit, &mut state, &mut sdi, &vfs);
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn desktop_start_switches_to_terminal() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn desktop_cancel_no_windows_returns_to_dashboard() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        // No windows open.
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Cancel),
            &mut state,
            &mut sdi,
            &vfs,
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
        assert_eq!(state.input_buf, "hello world");
    }

    #[test]
    fn terminal_backspace_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.input_buf.clear();
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert!(state.input_buf.is_empty());
    }

    #[test]
    fn terminal_square_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.input_buf.clear();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.input_buf.is_empty());
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
        assert!(state.bottom_bar.l_pressed);
        handle_default_input(
            &InputEvent::TriggerRelease(Trigger::Left),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(!state.bottom_bar.l_pressed);
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
        assert!(state.bottom_bar.r_pressed);
        assert!(state.active_transition.is_some());
        handle_default_input(
            &InputEvent::TriggerRelease(Trigger::Right),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(!state.bottom_bar.r_pressed);
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
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::CursorMove { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Desktop);
    }

    #[test]
    fn desktop_pointer_release_does_not_change_mode() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::PointerRelease { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn desktop_click_no_windows_returns_to_dashboard() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        let result = handle_desktop_input(
            &InputEvent::PointerClick { x: 100, y: 50 },
            &mut state,
            &mut sdi,
            &vfs,
        );
        assert_eq!(result, InputResult::Continue);
        assert_eq!(state.mode, Mode::Dashboard);
    }

    #[test]
    fn desktop_text_input_without_browser_is_noop() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        state.browser = None;
        let result = handle_desktop_input(&InputEvent::TextInput('a'), &mut state, &mut sdi, &vfs);
        assert_eq!(result, InputResult::Continue);
    }

    #[test]
    fn desktop_backspace_without_browser_is_noop() {
        let (mut state, mut sdi, vfs) = make_test_state();
        state.mode = Mode::Desktop;
        state.browser = None;
        let result = handle_desktop_input(&InputEvent::Backspace, &mut state, &mut sdi, &vfs);
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
}
