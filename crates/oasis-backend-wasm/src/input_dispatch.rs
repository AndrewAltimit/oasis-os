//! Input dispatch methods for [`OasisWasm`].

use oasis_core::apps::{AppAction, AppRunner};
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::startmenu::StartMenuAction;
use oasis_core::terminal_sdi;
use oasis_core::transition;
use oasis_core::wm::manager::WmEvent;

use crate::vfs_content::trim_output;
use crate::{Mode, OasisWasm};

impl OasisWasm {
    // -----------------------------------------------------------------------
    // Input dispatch: default (Dashboard / Terminal)
    // -----------------------------------------------------------------------

    pub(crate) fn handle_default_input(&mut self, event: &InputEvent) {
        match event {
            // Launch app from dashboard.
            InputEvent::ButtonPress(Button::Confirm) if self.mode == Mode::Dashboard => {
                self.dashboard.trigger_press_flash();
                if self.bottom_bar.active_tab == MediaTab::None
                    && let Some(app) = self.dashboard.selected_app()
                {
                    let app = app.clone();
                    self.launch_app_window(&app);
                }
            },

            // Pointer click on dashboard: start menu takes priority.
            InputEvent::PointerClick { x, y } if self.mode == Mode::Dashboard => {
                if self.start_menu.hit_test_button(*x, *y) {
                    self.start_menu.toggle();
                    return;
                }
                if self.start_menu.open {
                    if let Some(action) = self.start_menu.hit_test_item(*x, *y) {
                        self.start_menu.close();
                        self.handle_start_menu_action(&action);
                    } else {
                        self.start_menu.close();
                    }
                    return;
                }
                if self.bottom_bar.active_tab == MediaTab::None {
                    let cfg = &self.dashboard.config;
                    let gx = *x - cfg.grid_x;
                    let gy = *y - cfg.grid_y;
                    if gx >= 0 && gy >= 0 {
                        let col = gx as usize / cfg.cell_w as usize;
                        let row = gy as usize / cfg.cell_h as usize;
                        if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                            let idx = row * cfg.grid_cols as usize + col;
                            let page_apps = self.dashboard.current_page_apps().len();
                            if idx < page_apps {
                                if self.dashboard.selected == idx {
                                    if let Some(app) = self.dashboard.selected_app() {
                                        let app = app.clone();
                                        self.launch_app_window(&app);
                                    }
                                } else {
                                    self.dashboard.selected = idx;
                                }
                            }
                        }
                    }
                }
            },

            InputEvent::ButtonPress(Button::Start) => {
                self.mode = match self.mode {
                    Mode::Dashboard => Mode::Terminal,
                    Mode::Terminal => Mode::Dashboard,
                    other => other,
                };
            },
            InputEvent::ButtonPress(Button::Select) => {
                if self.mode != Mode::Osk {
                    let osk_cfg = OskConfig {
                        title: "On-Screen Keyboard".to_string(),
                        ..OskConfig::for_screen(
                            self.active_theme.screen_w,
                            self.active_theme.screen_h,
                        )
                    };
                    self.osk = Some(OskState::new(osk_cfg, ""));
                    self.mode = Mode::Osk;
                }
            },

            InputEvent::ButtonPress(Button::Cancel) if self.mode == Mode::Dashboard => {},

            // L trigger: cycle top tabs.
            InputEvent::TriggerPress(Trigger::Left) if self.mode == Mode::Dashboard => {
                self.status_bar.next_tab();
                self.bottom_bar.l_pressed = true;
            },
            InputEvent::TriggerRelease(Trigger::Left) => {
                self.bottom_bar.l_pressed = false;
            },

            // R trigger: cycle media category tabs.
            InputEvent::TriggerPress(Trigger::Right) if self.mode == Mode::Dashboard => {
                self.bottom_bar.next_tab();
                self.bottom_bar.r_pressed = true;
                self.active_transition = Some(transition::fade_in_custom(
                    self.width,
                    self.height,
                    self.skin.features.transition_fade_frames.unwrap_or(15),
                ));
            },
            InputEvent::TriggerRelease(Trigger::Right) => {
                self.bottom_bar.r_pressed = false;
            },

            // Start menu intercepts input when open.
            InputEvent::ButtonPress(btn)
                if self.mode == Mode::Dashboard && self.start_menu.open =>
            {
                let action = self.start_menu.handle_input(btn);
                if action != StartMenuAction::None {
                    self.handle_start_menu_action(&action);
                }
            },

            // Dashboard D-pad navigation.
            InputEvent::ButtonPress(btn) if self.mode == Mode::Dashboard => match btn {
                Button::Up | Button::Down | Button::Left | Button::Right => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.handle_input(btn);
                    }
                },
                Button::Triangle => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.next_page();
                        self.bottom_bar.current_page = self.dashboard.page;
                    }
                },
                Button::Square => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.prev_page();
                        self.bottom_bar.current_page = self.dashboard.page;
                    }
                },
                _ => {},
            },

            // Terminal input.
            InputEvent::TextInput(ch) if self.mode == Mode::Terminal => {
                self.input_buf.push(*ch);
            },
            InputEvent::Backspace if self.mode == Mode::Terminal => {
                self.input_buf.pop();
            },
            InputEvent::ButtonPress(Button::Confirm) if self.mode == Mode::Terminal => {
                let line = self.input_buf.clone();
                self.input_buf.clear();
                self.terminal_scroll_offset = 0;
                if !line.is_empty() {
                    self.output_lines.push(format!("> {line}"));
                    self.execute_terminal_command(&line);
                    trim_output(&mut self.output_lines);
                }
            },
            InputEvent::ButtonPress(Button::Square) if self.mode == Mode::Terminal => {
                self.input_buf.pop();
            },
            InputEvent::ButtonPress(Button::Cancel) if self.mode == Mode::Terminal => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                self.mode = Mode::Dashboard;
            },
            InputEvent::MouseWheel { delta } if self.mode == Mode::Terminal => {
                let len = self.output_lines.len();
                let max_visible = terminal_sdi::visible_output_lines(&self.active_theme);
                if len > max_visible {
                    let max_offset = len - max_visible;
                    if *delta < 0 {
                        self.terminal_scroll_offset =
                            (self.terminal_scroll_offset + (-*delta as usize) * 3).min(max_offset);
                    } else {
                        self.terminal_scroll_offset = self
                            .terminal_scroll_offset
                            .saturating_sub(*delta as usize * 3);
                    }
                }
            },

            _ => {},
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: Desktop (windowed WM) mode
    // -----------------------------------------------------------------------

    pub(crate) fn handle_desktop_input(&mut self, event: &InputEvent) {
        match event {
            InputEvent::PointerClick { x, y } => {
                // Start menu takes priority over window manager.
                if self.start_menu.hit_test_button(*x, *y) {
                    self.start_menu.toggle();
                    return;
                }
                if self.start_menu.open {
                    if let Some(action) = self.start_menu.hit_test_item(*x, *y) {
                        self.start_menu.close();
                        self.handle_start_menu_action(&action);
                    } else {
                        self.start_menu.close();
                    }
                    return;
                }

                // Taskbar hit test before WM.
                if let Some(win_id) = self.taskbar.hit_test(*x, *y) {
                    let win_id = win_id.to_string();
                    if self.wm.active_window() == Some(win_id.as_str()) {
                        let _ = self.wm.minimize_window(&win_id, &mut self.sdi);
                    } else if self
                        .wm
                        .get_window(&win_id)
                        .is_some_and(|w| w.state == oasis_core::wm::window::WindowState::Minimized)
                    {
                        let _ = self.wm.restore_window(&win_id, &mut self.sdi);
                    } else {
                        let _ = self.wm.focus_window(&win_id, &mut self.sdi);
                    }
                    return;
                }

                let wm_event = self
                    .wm
                    .handle_input(&InputEvent::PointerClick { x: *x, y: *y }, &mut self.sdi);
                match wm_event {
                    WmEvent::WindowClosed(id) => {
                        if self.fullscreen_app.as_deref() == Some(id.as_str()) {
                            self.fullscreen_app = None;
                        }
                        self.open_runners.retain(|(rid, _)| *rid != id);
                        if id == "browser" {
                            self.browser = None;
                            self.iframe.hide();
                        }
                        if self.wm.window_count() == 0 {
                            self.mode = Mode::Dashboard;
                        }
                    },
                    WmEvent::ContentClick(id, lx, ly) => {
                        if id == "browser"
                            && let Some(ref mut bw) = self.browser
                        {
                            // When the iframe is showing a real web page,
                            // only forward clicks in the URL bar area to
                            // the browser widget — the iframe handles its
                            // own content input natively.
                            let in_url_bar = ly < bw.config.url_bar_height as i32;
                            if in_url_bar || !self.iframe.is_visible() {
                                let abs_x = bw.window_x() + lx;
                                let abs_y = bw.window_y() + ly;
                                bw.handle_input(
                                    &InputEvent::PointerClick { x: abs_x, y: abs_y },
                                    &self.vfs,
                                );
                            }
                        } else if let Some((_, runner)) =
                            self.open_runners.iter_mut().find(|(rid, _)| *rid == id)
                            && let Some(win) = self.wm.get_window(&id)
                        {
                            let (_, _, cw, ch) = win.content_rect(self.wm.theme());
                            let action = runner.handle_click(lx, ly, cw, ch, win.fullscreen_kiosk);
                            if action == AppAction::RequestFullscreen
                                && self.fullscreen_app.is_none()
                            {
                                let _ = self.wm.enter_fullscreen(&id, &mut self.sdi);
                                self.fullscreen_app = Some(id.to_string());
                            }
                        }
                    },
                    WmEvent::DesktopClick(dx, dy) => {
                        if self.wm.window_count() == 0 {
                            self.mode = Mode::Dashboard;
                        } else if self.bottom_bar.active_tab == MediaTab::None {
                            // Forward desktop clicks to dashboard icons.
                            let cfg = &self.dashboard.config;
                            let gx = dx - cfg.grid_x;
                            let gy = dy - cfg.grid_y;
                            if gx >= 0 && gy >= 0 {
                                let col = gx as usize / cfg.cell_w as usize;
                                let row = gy as usize / cfg.cell_h as usize;
                                if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                                    let idx = row * cfg.grid_cols as usize + col;
                                    let page_apps = self.dashboard.current_page_apps().len();
                                    if idx < page_apps {
                                        if self.dashboard.selected == idx {
                                            if let Some(app) = self.dashboard.selected_app() {
                                                let app = app.clone();
                                                self.launch_app_window(&app);
                                            }
                                        } else {
                                            self.dashboard.selected = idx;
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
                self.taskbar.set_hover(*x, *y);
                self.wm
                    .handle_input(&InputEvent::CursorMove { x: *x, y: *y }, &mut self.sdi);
            },
            InputEvent::PointerRelease { x, y } => {
                self.wm
                    .handle_input(&InputEvent::PointerRelease { x: *x, y: *y }, &mut self.sdi);
            },
            InputEvent::ToggleFullscreen => {
                if let Some(ref fs_id) = self.fullscreen_app {
                    let id = fs_id.clone();
                    let _ = self.wm.exit_fullscreen(&id, &mut self.sdi);
                    self.fullscreen_app = None;
                } else if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    let _ = self.wm.enter_fullscreen(&active_id, &mut self.sdi);
                    self.fullscreen_app = Some(active_id);
                }
            },
            InputEvent::ButtonPress(Button::Cancel) => {
                if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    if self.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                        let _ = self.wm.exit_fullscreen(&active_id, &mut self.sdi);
                        self.fullscreen_app = None;
                    }
                    let _ = self.wm.close_window(&active_id, &mut self.sdi);
                    self.open_runners.retain(|(rid, _)| *rid != active_id);
                    if active_id == "browser" {
                        self.browser = None;
                        self.iframe.hide();
                    }
                    if self.wm.window_count() == 0 {
                        self.mode = Mode::Dashboard;
                    }
                } else {
                    self.mode = Mode::Dashboard;
                }
            },
            InputEvent::ButtonPress(Button::Start) => {
                if !self.skin.features.window_manager {
                    self.mode = Mode::Terminal;
                }
            },
            InputEvent::TextInput(ch) => match self.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = self.browser {
                        bw.handle_input(&InputEvent::TextInput(*ch), &self.vfs);
                    }
                },
                Some("terminal") => {
                    self.input_buf.push(*ch);
                },
                _ => {},
            },
            InputEvent::Backspace => match self.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = self.browser {
                        bw.handle_input(&InputEvent::Backspace, &self.vfs);
                    }
                },
                Some("terminal") => {
                    self.input_buf.pop();
                },
                _ => {},
            },
            InputEvent::MouseWheel { delta } => match self.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = self.browser {
                        bw.handle_input(&InputEvent::MouseWheel { delta: *delta }, &self.vfs);
                    }
                },
                Some("terminal") => {
                    let len = self.output_lines.len() + 1;
                    let max_visible = terminal_sdi::visible_output_lines(&self.active_theme);
                    if len > max_visible {
                        let max_offset = len - max_visible;
                        if *delta < 0 {
                            self.terminal_scroll_offset = (self.terminal_scroll_offset
                                + (-*delta as usize) * 3)
                                .min(max_offset);
                        } else {
                            self.terminal_scroll_offset = self
                                .terminal_scroll_offset
                                .saturating_sub(*delta as usize * 3);
                        }
                    }
                },
                _ => {},
            },
            InputEvent::ButtonPress(btn) => {
                if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    if active_id == "browser" {
                        if let Some(ref mut bw) = self.browser {
                            bw.handle_input(&InputEvent::ButtonPress(*btn), &self.vfs);
                        }
                    } else if active_id == "terminal" && *btn == Button::Confirm {
                        // Execute command in windowed terminal.
                        let line = self.input_buf.clone();
                        self.input_buf.clear();
                        self.terminal_scroll_offset = 0;
                        if !line.is_empty() {
                            self.output_lines.push(format!("> {line}"));
                            self.execute_terminal_command(&line);
                            trim_output(&mut self.output_lines);
                        }
                    } else if let Some((_, runner)) = self
                        .open_runners
                        .iter_mut()
                        .find(|(id, _)| *id == active_id)
                    {
                        match runner.handle_input(btn, &self.vfs) {
                            AppAction::Exit => {
                                if self.fullscreen_app.as_deref() == Some(active_id.as_str()) {
                                    let _ = self.wm.exit_fullscreen(&active_id, &mut self.sdi);
                                    self.fullscreen_app = None;
                                }
                                let _ = self.wm.close_window(&active_id, &mut self.sdi);
                                self.open_runners.retain(|(rid, _)| *rid != active_id);
                                if self.wm.window_count() == 0 {
                                    self.mode = Mode::Dashboard;
                                }
                            },
                            AppAction::SwitchToTerminal => {
                                self.mode = Mode::Terminal;
                            },
                            AppAction::RequestFullscreen => {
                                if self.fullscreen_app.is_none() {
                                    let _ = self.wm.enter_fullscreen(&active_id, &mut self.sdi);
                                    self.fullscreen_app = Some(active_id);
                                }
                            },
                            AppAction::None => {},
                        }
                    }
                }
            },
            _ => {},
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: App (fullscreen) mode
    // -----------------------------------------------------------------------

    pub(crate) fn handle_app_input(&mut self, event: &InputEvent) {
        if let Some(ref mut runner) = self.app_runner
            && let InputEvent::ButtonPress(btn) = event
        {
            match runner.handle_input(btn, &self.vfs) {
                AppAction::Exit => {
                    AppRunner::hide_sdi(&mut self.sdi);
                    self.app_runner = None;
                    self.mode = Mode::Dashboard;
                },
                AppAction::SwitchToTerminal => {
                    AppRunner::hide_sdi(&mut self.sdi);
                    self.app_runner = None;
                    self.mode = Mode::Terminal;
                },
                AppAction::RequestFullscreen | AppAction::None => {},
            }
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: OSK mode
    // -----------------------------------------------------------------------

    pub(crate) fn handle_osk_input(&mut self, event: &InputEvent) {
        if let Some(ref mut osk_state) = self.osk {
            match event {
                InputEvent::Backspace => {
                    osk_state.buffer.pop();
                },
                InputEvent::ButtonPress(btn) => {
                    osk_state.handle_input(btn);
                    if let Some(text) = osk_state.confirmed_text() {
                        self.output_lines.push(format!("[OSK] Input: {text}"));
                        trim_output(&mut self.output_lines);
                        osk_state.hide_sdi(&mut self.sdi);
                        self.osk = None;
                        self.mode = Mode::Dashboard;
                    } else if osk_state.is_cancelled() {
                        self.output_lines.push("[OSK] Cancelled".to_string());
                        trim_output(&mut self.output_lines);
                        osk_state.hide_sdi(&mut self.sdi);
                        self.osk = None;
                        self.mode = Mode::Dashboard;
                    }
                },
                _ => {},
            }
        }
    }
}
