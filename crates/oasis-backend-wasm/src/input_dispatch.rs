//! Input dispatch methods for [`OasisWasm`].
//!
//! # Pattern note (D4)
//!
//! This module shares a structural pattern with
//! `oasis-backend-psp/src/input_dispatch/classic.rs`: both dispatch
//! `InputEvent` variants through a mode/view state machine (Dashboard,
//! Terminal, app views).  However, the concrete types diverge completely
//! (PSP takes 20+ arguments with PSP-specific hardware types; WASM uses
//! `&mut self` on `OasisWasm`), so extracting a shared trait or generic
//! dispatcher would add abstraction without reducing code.  If a third
//! backend appears with the same pattern, consider a shared
//! `InputDispatcher` trait in `oasis-types`.

use oasis_core::apps::{AppAction, OwnedMedia};
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Key, Modifiers, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::startmenu::StartMenuAction;
use oasis_core::terminal::SessionEvent;
use oasis_core::terminal_sdi;
use oasis_core::transition;
use oasis_core::wm::manager::WmEvent;
#[cfg(feature = "wasm-youtube")]
use oasis_types::backend::SdiCore;

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
            InputEvent::ButtonPress(Button::Select) if self.mode != Mode::Osk => {
                let osk_cfg = OskConfig {
                    title: "On-Screen Keyboard".to_string(),
                    ..OskConfig::for_screen(self.active_theme.screen_w, self.active_theme.screen_h)
                };
                self.osk = Some(OskState::new(osk_cfg, ""));
                self.mode = Mode::Osk;
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
                Button::Up | Button::Down | Button::Left | Button::Right
                    if self.bottom_bar.active_tab == MediaTab::None =>
                {
                    self.dashboard.handle_input(btn);
                },
                Button::Triangle if self.bottom_bar.active_tab == MediaTab::None => {
                    self.dashboard.next_page();
                    self.bottom_bar.current_page = self.dashboard.page;
                },
                Button::Square if self.bottom_bar.active_tab == MediaTab::None => {
                    self.dashboard.prev_page();
                    self.bottom_bar.current_page = self.dashboard.page;
                },
                _ => {},
            },

            // Terminal input: line editing (text, Backspace, Tab, d-pad,
            // Confirm, Square) goes to the shell session.
            InputEvent::TextInput(_)
            | InputEvent::Backspace
            | InputEvent::Tab
            | InputEvent::ButtonPress(_)
                if self.mode == Mode::Terminal && self.terminal_event(event) => {},
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

                // Desktop indicator hit test.
                if let Some(hit) = self.taskbar.desktop_hit_test(*x, *y) {
                    match hit {
                        oasis_core::taskbar::DesktopHit::Prev => {
                            self.desktops.switch_prev();
                        },
                        oasis_core::taskbar::DesktopHit::Next => {
                            self.desktops.switch_next();
                        },
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

                // A titlebar close button asks the app first: an editor
                // with unsaved changes answers with its prompt instead of
                // closing (the WM would otherwise drop the window, and the
                // edits, on the spot).
                if let Some(id) = self.close_button_under(*x, *y) {
                    self.request_close_window(&id);
                    return;
                }
                let wm_event = self
                    .wm
                    .handle_input(&InputEvent::PointerClick { x: *x, y: *y }, &mut self.sdi);
                match wm_event {
                    // The close button is intercepted above; this only
                    // fires if the WM's own hit test found one ours did
                    // not. The window is already gone: clean up after it.
                    WmEvent::WindowClosed(id) => self.close_app_window(&id),
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
                            let action =
                                runner.handle_click(lx, ly, cw, ch, &self.active_theme, &self.vfs);
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
                self.start_menu.set_hover(*x, *y);
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
            InputEvent::ButtonPress(Button::Cancel) => self.desktop_cancel(),
            InputEvent::ButtonPress(Button::Start) if !self.skin.features.window_manager => {
                self.mode = Mode::Terminal;
            },
            // Windowed terminal: line editing goes to the shell session.
            InputEvent::TextInput(_)
            | InputEvent::Backspace
            | InputEvent::Tab
            | InputEvent::ButtonPress(_)
                if self.wm.active_window() == Some("terminal") && self.terminal_event(event) => {},
            InputEvent::TextInput(ch) => match self.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = self.browser {
                        bw.handle_input(&InputEvent::TextInput(*ch), &self.vfs);
                    }
                },
                Some("terminal") => {},
                Some(id) => {
                    if let Some((_, runner)) =
                        self.open_runners.iter_mut().find(|(rid, _)| rid == id)
                    {
                        runner.handle_text_input(*ch);
                    }
                },
                None => {},
            },
            InputEvent::Backspace => match self.wm.active_window() {
                Some("browser") => {
                    if let Some(ref mut bw) = self.browser {
                        bw.handle_input(&InputEvent::Backspace, &self.vfs);
                    }
                },
                Some("terminal") => {},
                Some(id) => {
                    if let Some((_, runner)) =
                        self.open_runners.iter_mut().find(|(rid, _)| rid == id)
                    {
                        runner.handle_backspace();
                    }
                },
                None => {},
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
                Some(id) => {
                    if let Some((_, runner)) =
                        self.open_runners.iter_mut().find(|(rid, _)| rid == id)
                    {
                        runner.handle_scroll(*delta, &self.vfs);
                    }
                },
                None => {},
            },
            InputEvent::ButtonPress(btn) => {
                if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    if active_id == "browser" {
                        if let Some(ref mut bw) = self.browser {
                            bw.handle_input(&InputEvent::ButtonPress(*btn), &self.vfs);
                        }
                    } else if let Some((_, runner)) = self
                        .open_runners
                        .iter_mut()
                        .find(|(id, _)| *id == active_id)
                    {
                        let action = runner.handle_input(btn, &self.vfs);
                        self.apply_window_action(action, active_id);
                    }
                }
            },
            // L/R triggers switch virtual desktops.
            InputEvent::TriggerPress(Trigger::Left) => {
                self.desktops.switch_prev();
            },
            InputEvent::TriggerPress(Trigger::Right) => {
                self.desktops.switch_next();
            },
            _ => {},
        }
    }

    // -----------------------------------------------------------------------
    // Closing app windows
    // -----------------------------------------------------------------------

    /// Close every open app that asked to close itself outside an input
    /// handler (`AppRunner::take_close_request`). Called once per frame
    /// after the runners' `apply_vfs_ops` and `tick`.
    pub(crate) fn apply_app_close_requests(&mut self) {
        let closing: Vec<String> = self
            .open_runners
            .iter_mut()
            .filter_map(|(id, runner)| runner.take_close_request().then(|| id.clone()))
            .collect();
        for id in closing {
            self.apply_window_action(AppAction::Exit, id);
        }
    }

    /// The window whose titlebar close button is at `(x, y)`, if any (and
    /// clickable: not behind a modal, not a kiosk-fullscreen window).
    fn close_button_under(&self, x: i32, y: i32) -> Option<String> {
        let id = self.wm.window_at(x, y)?;
        if self.wm.topmost_modal().is_some_and(|m| m != id) {
            return None;
        }
        let win = self.wm.get_window(id).filter(|w| !w.fullscreen_kiosk)?;
        let (bx, by, bw, bh) = win.close_btn_rect(self.wm.theme())?;
        let hit = x >= bx && y >= by && x < bx + bw as i32 && y < by + bh as i32;
        hit.then(|| id.to_string())
    }

    /// The user asked to close window `id` (titlebar button, Cancel on a
    /// window that is not an app): the app is asked first
    /// (`AppRunner::request_close`), and an app with unsaved changes
    /// answers with its prompt instead, leaving the window open and
    /// focused. Returns whether the window closed. The desktop shell's
    /// `CloseHow::UserChrome`.
    pub(crate) fn request_close_window(&mut self, id: &str) -> bool {
        if let Some((_, runner)) = self.open_runners.iter_mut().find(|(rid, _)| rid == id)
            && runner.request_close(&self.vfs) != AppAction::Exit
        {
            let _ = self.wm.focus_window(id, &mut self.sdi);
            return false;
        }
        self.close_app_window(id);
        true
    }

    /// Desktop-mode Cancel, as on the desktop shell: app windows own it
    /// (it backs out of dialogs, leaves viewers, raises the Text Editor's
    /// unsaved-changes prompt; the window closes only when the app answers
    /// `Exit`); the browser ends typing first; other windows close.
    fn desktop_cancel(&mut self) {
        let Some(active_id) = self.wm.active_window().map(str::to_string) else {
            if let Some(top) = self.wm.topmost_visible().map(str::to_string) {
                // Windows on screen but none focused (e.g. after a click
                // on the bare desktop): focus the top one so the next
                // Cancel reaches it instead of leaving them dead to input.
                let _ = self.wm.focus_window(&top, &mut self.sdi);
            } else {
                self.mode = Mode::Dashboard;
            }
            return;
        };
        if active_id != "browser"
            && active_id != "terminal"
            && let Some((_, runner)) = self
                .open_runners
                .iter_mut()
                .find(|(id, _)| *id == active_id)
        {
            let action = runner.handle_input(&Button::Cancel, &self.vfs);
            self.apply_window_action(action, active_id);
            return;
        }
        if active_id == "browser"
            && let Some(ref mut bw) = self.browser
            && bw.accepts_text()
        {
            bw.handle_input(&InputEvent::ButtonPress(Button::Cancel), &self.vfs);
            return;
        }
        self.request_close_window(&active_id);
    }

    /// Close window `id` unconditionally (an app's own `Exit`, or a window
    /// the WM already dropped): exit kiosk fullscreen, close the WM window
    /// (if the WM hasn't already), stop the radio / music the app owned,
    /// release the runner's backend resources (textures) before dropping
    /// it, tear down the browser / YouTube overlays, and fall back to the
    /// dashboard when the last window went away. The WASM counterpart of
    /// the desktop shell's `windows::close_app_window`; user-initiated
    /// closes go through [`Self::request_close_window`].
    pub(crate) fn close_app_window(&mut self, id: &str) {
        if self.fullscreen_app.as_deref() == Some(id) {
            let _ = self.wm.exit_fullscreen(id, &mut self.sdi);
            self.fullscreen_app = None;
        }
        if self.wm.get_window(id).is_some() {
            let _ = self.wm.close_window(id, &mut self.sdi);
        }
        if let Some(pos) = self.open_runners.iter().position(|(rid, _)| rid == id) {
            let (_, mut runner) = self.open_runners.remove(pos);
            // Requests the app made this frame (a skin picked right before
            // Escape) are carried out by the next request pass instead of
            // being dropped with the runner.
            runner.drain_requests(&mut self.closed_app_requests);
            match self.apps.owned_media(&runner.title) {
                Some(OwnedMedia::Radio) => self.stop_radio(),
                Some(OwnedMedia::Music) => self.stop_music(),
                None => {},
            }
            runner.release_resources(&mut self.backend);
        }
        if id == "browser" {
            self.browser = None;
            self.iframe.hide();
        }
        #[cfg(feature = "wasm-youtube")]
        if id == "video_embed" {
            self.youtube_active_id = None;
            self.youtube_active_url = None;
            self.iframe.hide();
            for tex in self.youtube_thumb_textures.drain(..) {
                let _ = self.backend.destroy_texture(tex);
            }
        }
        if self.wm.window_count() == 0 && self.mode == Mode::Desktop {
            self.mode = Mode::Dashboard;
        }
    }

    /// Stop radio playback and drop the station's catalog / source, as
    /// the desktop shell does when the Internet Radio window closes:
    /// otherwise the stream keeps playing with no UI to control it.
    fn stop_radio(&mut self) {
        let _ = self.radio_manager.process_request("stop", &mut self.audio);
        self.archive_catalog = None;
        self.pending_catalog = None;
        if let Some(mut src) = self.radio_source.take() {
            src.disconnect();
        }
    }

    /// Apply an [`AppAction`] returned by the windowed runner `active_id`.
    fn apply_window_action(&mut self, action: AppAction, active_id: String) {
        match action {
            AppAction::Exit => self.close_app_window(&active_id),
            AppAction::RequestFullscreen => {
                if self.fullscreen_app.is_none() {
                    let _ = self.wm.enter_fullscreen(&active_id, &mut self.sdi);
                    self.fullscreen_app = Some(active_id);
                }
            },
            AppAction::LaunchAppWithFile {
                app_title,
                file_path,
            } => {
                self.launch_app_window_for_file(&app_title, &file_path);
            },
            AppAction::None => {},
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: raw keyboard keys
    // -----------------------------------------------------------------------

    /// Whether the current input target is a text-entry surface (terminal,
    /// browser URL bar / form field, or an app with `accepts_text()`).
    pub(crate) fn text_focus(&self) -> bool {
        match self.mode {
            Mode::Terminal => true,
            Mode::Desktop => match self.wm.active_window() {
                Some("terminal") => true,
                Some("browser") => self.browser.as_ref().is_some_and(|bw| bw.accepts_text()),
                Some(active_id) => self
                    .open_runners
                    .iter()
                    .any(|(id, runner)| id == active_id && runner.accepts_text()),
                None => false,
            },
            _ => false,
        }
    }

    /// Route a raw key press to the focused app's `handle_key`. Returns
    /// `true` when the app consumed it (its action has been applied).
    fn route_key(&mut self, key: &Key, mods: Modifiers) -> bool {
        match self.mode {
            Mode::Desktop => {
                let Some(active_id) = self
                    .wm
                    .active_window()
                    .filter(|id| *id != "browser" && *id != "terminal")
                    .map(str::to_string)
                else {
                    return false;
                };
                let Some((_, runner)) = self
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| *id == active_id)
                else {
                    return false;
                };
                let Some(action) = runner.handle_key(key, mods, &self.vfs) else {
                    return false;
                };
                self.apply_window_action(action, active_id);
                true
            },
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Terminal line editing (shared ShellSession, same keys as desktop)
    // -----------------------------------------------------------------------

    /// Whether keyboard input currently belongs to the terminal.
    fn terminal_focused(&self) -> bool {
        match self.mode {
            Mode::Terminal => true,
            Mode::Desktop => self.wm.active_window() == Some("terminal"),
            _ => false,
        }
    }

    /// Offer a raw key to the line editor; `true` when it was a shortcut.
    fn terminal_key(&mut self, key: Key, mods: Modifiers) -> bool {
        let Some(ev) = self
            .shell
            .handle_key(key, mods, &self.cmd_reg, &self.cwd, &self.vfs)
        else {
            return false;
        };
        self.apply_shell_event(ev);
        true
    }

    /// Offer a text / Backspace / Tab / button event to the line editor.
    fn terminal_event(&mut self, event: &InputEvent) -> bool {
        let Some(ev) = self
            .shell
            .handle_event(event, &self.cmd_reg, &self.cwd, &self.vfs)
        else {
            return false;
        };
        self.apply_shell_event(ev);
        true
    }

    fn apply_shell_event(&mut self, ev: SessionEvent) {
        match ev {
            SessionEvent::Redraw => {},
            SessionEvent::Submit(line) => {
                self.terminal_scroll_offset = 0;
                if !line.trim().is_empty() {
                    self.output_lines.push(format!("> {line}"));
                    self.execute_terminal_command(&line);
                }
            },
            SessionEvent::Interrupted(line) => {
                self.output_lines.push(format!("> {line}^C"));
                self.terminal_scroll_offset = 0;
            },
            SessionEvent::ClearScreen => {
                self.output_lines.clear();
                self.terminal_scroll_offset = 0;
            },
            SessionEvent::Candidates(candidates) => {
                self.output_lines.push(format!("> {}", self.shell.buffer()));
                self.output_lines.push(candidates.join("  "));
                self.terminal_scroll_offset = 0;
            },
        }
        trim_output(&mut self.output_lines);
    }

    /// Per-event entry point used by `tick()`.
    ///
    /// Routes [`InputEvent::Key`] to the focused app and arms
    /// `self.key_filter` so the key's gamepad-style twin is dropped when
    /// the app consumed it or it merely typed into a text-entry target;
    /// every other event goes to the handler for the current mode.
    pub(crate) fn handle_event(&mut self, event: &InputEvent) {
        if self.key_filter.should_drop(event) {
            return;
        }
        if let InputEvent::Key { key, mods } = event {
            // Terminal line-editing shortcuts (Home/End/Delete, Ctrl+A/E/K/
            // ..., Ctrl+R search); their twins are dropped.
            if self.terminal_focused() && self.terminal_key(*key, *mods) {
                self.key_filter.suppress_twin(*key, *mods);
                return;
            }
            let typing = key.produces_text(*mods) && self.text_focus();
            let consumed = self.route_key(key, *mods);
            if consumed || typing {
                self.key_filter.suppress_twin(*key, *mods);
            }
            return;
        }
        match self.mode {
            Mode::Osk => self.handle_osk_input(event),
            Mode::Desktop => self.handle_desktop_input(event),
            _ => self.handle_default_input(event),
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
