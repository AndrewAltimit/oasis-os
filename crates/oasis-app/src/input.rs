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
use oasis_core::terminal_sdi;

use crate::{commands, launch};

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
                    if state.content.fullscreen_app.as_deref() == Some(id.as_str()) {
                        state.content.fullscreen_app = None;
                    }
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
                    } else if state.ui.bottom_bar.active_tab == MediaTab::None {
                        // Forward desktop clicks to dashboard icons.
                        let cfg = &state.ui.dashboard.config;
                        let gx = dx - cfg.grid_x;
                        let gy = dy - cfg.grid_y;
                        if gx >= 0 && gy >= 0 {
                            let col = gx as usize / cfg.cell_w as usize;
                            let row = gy as usize / cfg.cell_h as usize;
                            if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                                let idx = row * cfg.grid_cols as usize + col;
                                let page_apps = state.ui.dashboard.current_page_apps().len();
                                if idx < page_apps {
                                    if state.ui.dashboard.selected == idx {
                                        if let Some(app) = state.ui.dashboard.selected_app() {
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
                                        }
                                    } else {
                                        state.ui.dashboard.selected = idx;
                                    }
                                }
                            }
                        }
                    }
                },
                _ => {},
            }
        },
        InputEvent::CursorMove { x, y } => {
            state.ui.taskbar.set_hover(*x, *y);
            state
                .wm
                .handle_input(&InputEvent::CursorMove { x: *x, y: *y }, sdi);
        },
        InputEvent::PointerRelease { x, y } => {
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
                // If closing the fullscreen window, clear fullscreen state first.
                if state.content.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                    let _ = state.wm.exit_fullscreen(&active_id, sdi);
                    state.content.fullscreen_app = None;
                }
                let _ = state.wm.close_window(&active_id, sdi);
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
        InputEvent::ButtonPress(Button::Start) => {
            if !state.skin.features.window_manager {
                state.mode = Mode::Terminal;
            }
        },
        InputEvent::TextInput(ch) => match state.wm.active_window() {
            Some("browser") => {
                if let Some(ref mut bw) = state.content.browser {
                    bw.handle_input(&InputEvent::TextInput(*ch), vfs);
                }
            },
            Some("terminal") => {
                state.terminal.input_buf.push(*ch);
            },
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
            Some("terminal") => {
                state.terminal.input_buf.pop();
            },
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
                } else if active_id == "terminal" && *btn == Button::Confirm {
                    // Execute command in windowed terminal.
                    let line = state.terminal.input_buf.clone();
                    state.terminal.input_buf.clear();
                    state.terminal.scroll_offset = 0;
                    if !line.is_empty() {
                        state.terminal.output_lines.push(format!("> {line}"));
                        let pending_skin_swap;
                        {
                            let mut env = Environment {
                                cwd: state.terminal.cwd.clone(),
                                vfs,
                                power: Some(&state.platform),
                                time: Some(&state.platform),
                                usb: Some(&state.platform),
                                network: None,
                                tls: Some(&state.net.tls_provider),
                                stdin: None,
                                stderr: String::new(),
                            };
                            let result = state.terminal.cmd_reg.execute(&line, &mut env);
                            state.terminal.cwd = env.cwd;
                            pending_skin_swap = commands::process_command_output(result, state);
                        }
                        if let Some(name) = pending_skin_swap {
                            commands::apply_skin_swap(&name, state, sdi, vfs);
                        }
                    }
                    commands::trim_output(&mut state.terminal.output_lines);
                } else if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| *id == active_id)
                {
                    match runner.handle_input(btn, vfs) {
                        AppAction::Exit => {
                            if state.content.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                                let _ = state.wm.exit_fullscreen(&active_id, sdi);
                                state.content.fullscreen_app = None;
                            }
                            let _ = state.wm.close_window(&active_id, sdi);
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
                        AppAction::None => {},
                    }
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
            InputEvent::ButtonPress(btn) => match runner.handle_input(btn, vfs) {
                AppAction::Exit => {
                    AppRunner::hide_sdi(sdi);
                    state.content.app_runner = None;
                    state.mode = Mode::Dashboard;
                },
                AppAction::SwitchToTerminal => {
                    AppRunner::hide_sdi(sdi);
                    state.content.app_runner = None;
                    state.mode = Mode::Terminal;
                },
                AppAction::RequestFullscreen | AppAction::None => {},
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
            if state.ui.bottom_bar.active_tab == MediaTab::None {
                let cfg = &state.ui.dashboard.config;
                let gx = *x - cfg.grid_x;
                let gy = *y - cfg.grid_y;
                if gx >= 0 && gy >= 0 {
                    let col = gx as usize / cfg.cell_w as usize;
                    let row = gy as usize / cfg.cell_h as usize;
                    if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                        let idx = row * cfg.grid_cols as usize + col;
                        let page_apps = state.ui.dashboard.current_page_apps().len();
                        if idx < page_apps {
                            if state.ui.dashboard.selected == idx {
                                if let Some(app) = state.ui.dashboard.selected_app() {
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
                                    state.active_transition = Some(launch::make_transition(
                                        state.config.screen_width,
                                        state.config.screen_height,
                                        state.skin.features.transition_fade_frames.unwrap_or(15),
                                    ));
                                }
                            } else {
                                state.ui.dashboard.selected = idx;
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
                    ..OskConfig::for_screen(
                        state.active_theme.screen_w,
                        state.active_theme.screen_h,
                    )
                };
                state.osk = Some(OskState::new(osk_cfg, ""));
                state.mode = Mode::Osk;
                log::info!("OSK opened");
            }
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
            Button::Up | Button::Down | Button::Left | Button::Right => {
                if state.ui.bottom_bar.active_tab == MediaTab::None {
                    state.ui.dashboard.handle_input(btn);
                }
            },
            Button::Triangle => {
                if state.ui.bottom_bar.active_tab == MediaTab::None {
                    state.ui.dashboard.next_page();
                    state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                }
            },
            Button::Square => {
                if state.ui.bottom_bar.active_tab == MediaTab::None {
                    state.ui.dashboard.prev_page();
                    state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                }
            },
            _ => {},
        },

        // Terminal input.
        InputEvent::TextInput(ch) if state.mode == Mode::Terminal => {
            state.terminal.input_buf.push(*ch);
        },
        InputEvent::Backspace if state.mode == Mode::Terminal => {
            state.terminal.input_buf.pop();
        },
        InputEvent::ButtonPress(Button::Confirm) if state.mode == Mode::Terminal => {
            let line = state.terminal.input_buf.clone();
            state.terminal.input_buf.clear();
            state.terminal.scroll_offset = 0;
            if !line.is_empty() {
                state.terminal.output_lines.push(format!("> {line}"));
                let pending_skin_swap;
                {
                    let mut env = Environment {
                        cwd: state.terminal.cwd.clone(),
                        vfs,
                        power: Some(&state.platform),
                        time: Some(&state.platform),
                        usb: Some(&state.platform),
                        network: None,
                        tls: Some(&state.net.tls_provider),
                        stdin: None,
                        stderr: String::new(),
                    };
                    let result = state.terminal.cmd_reg.execute(&line, &mut env);
                    state.terminal.cwd = env.cwd;
                    pending_skin_swap = commands::process_command_output(result, state);
                }
                if let Some(name) = pending_skin_swap {
                    commands::apply_skin_swap(&name, state, sdi, vfs);
                }
            }
            commands::trim_output(&mut state.terminal.output_lines);
        },
        InputEvent::ButtonPress(Button::Square) if state.mode == Mode::Terminal => {
            state.terminal.input_buf.pop();
        },
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Terminal => {
            terminal_sdi::set_terminal_visible(sdi, false);
            state.mode = Mode::Dashboard;
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

        let skin = load_builtin("terminal").unwrap();
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
                desktops: oasis_core::wm::DesktopManager::new(4),
            },
            terminal: TerminalLayer {
                cmd_reg: CommandRegistry::new(),
                cwd: "/".to_string(),
                input_buf: String::new(),
                output_lines: Vec::new(),
                scroll_offset: 0,
                dirty: true,
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
            radio_manager: RadioManager::new(),
            radio_source: None,
            archive_catalog: None,
            pending_catalog_fetch: None,
            pending_source_fetch: None,
            audio_backend: SdlAudioBackend::new(),
            toasts: oasis_core::toast::ToastManager::new(),
            pending_tv_catalog_fetch: None,
            tv_fetch_start: None,
            video_player: crate::video_player::VideoPlayer::new(),
            tv_audio_track: None,
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
        assert_eq!(state.terminal.input_buf, "hi");
    }

    #[test]
    fn terminal_backspace() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.input_buf = "abc".to_string();
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert_eq!(state.terminal.input_buf, "ab");
    }

    #[test]
    fn terminal_confirm_executes_command() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.input_buf = "echo hello".to_string();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Confirm),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        // Input buffer should be cleared.
        assert!(state.terminal.input_buf.is_empty());
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
        state.terminal.input_buf.clear();
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
        state.terminal.input_buf = "xyz".to_string();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.terminal.input_buf, "xy");
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
        handle_desktop_input(
            &InputEvent::ButtonPress(Button::Start),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert_eq!(state.mode, Mode::Terminal);
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
        assert_eq!(state.terminal.input_buf, "hello world");
    }

    #[test]
    fn terminal_backspace_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.input_buf.clear();
        handle_default_input(&InputEvent::Backspace, &mut state, &mut sdi, &mut vfs);
        assert!(state.terminal.input_buf.is_empty());
    }

    #[test]
    fn terminal_square_on_empty_is_noop() {
        let (mut state, mut sdi, mut vfs) = make_test_state();
        state.mode = Mode::Terminal;
        state.terminal.input_buf.clear();
        handle_default_input(
            &InputEvent::ButtonPress(Button::Square),
            &mut state,
            &mut sdi,
            &mut vfs,
        );
        assert!(state.terminal.input_buf.is_empty());
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
}
