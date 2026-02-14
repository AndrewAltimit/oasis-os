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
