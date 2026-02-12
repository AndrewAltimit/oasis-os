//! OASIS_OS desktop entry point.
//!
//! PSIX-style UI with wallpaper, mouse cursor, status bar, 6x3 icon grid
//! dashboard, and bottom bar with media category tabs.
//! L trigger cycles top tabs, R trigger cycles media categories,
//! D-pad navigates the grid. Click to select/launch icons.
//! Press F1 to toggle terminal, F2 to toggle on-screen keyboard, Escape to quit.

use anyhow::Result;

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppAction, AppRunner};
use oasis_core::backend::{Color, InputBackend, SdiBackend, TextureId};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::config::OasisConfig;
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::net::{
    ListenerConfig, RemoteClient, RemoteListener, RustlsTlsProvider, StdNetworkBackend,
};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin};
use oasis_core::startmenu::{StartMenuAction, StartMenuState};
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{CommandOutput, CommandRegistry, Environment, register_builtins};
use oasis_core::transition;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wallpaper;
use oasis_core::wm::manager::{WindowManager, WmEvent};
use oasis_core::wm::window::{WindowConfig, WindowType};

/// Maximum lines visible in the terminal output area.
const MAX_OUTPUT_LINES: usize = 12;

/// The UI modes the demo supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dashboard,
    Terminal,
    App,
    Osk,
    Desktop,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = OasisConfig::default();
    log::info!(
        "Starting OASIS_OS ({}x{})",
        config.screen_width,
        config.screen_height,
    );

    let mut backend = SdlBackend::new(
        &config.window_title,
        config.screen_width,
        config.screen_height,
    )?;
    backend.init(config.screen_width, config.screen_height)?;

    // Resolve skin from CLI arg, OASIS_SKIN env var, or config.
    let skin_name = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("OASIS_SKIN").ok())
        .unwrap_or_else(|| config.skin_path.to_string_lossy().into_owned());
    let mut skin = resolve_skin(&skin_name)?;
    log::info!(
        "Loaded skin: {} v{}",
        skin.manifest.name,
        skin.manifest.version
    );

    // Derive runtime theme from the active skin.
    let mut active_theme = ActiveTheme::from_skin(&skin.theme);
    let mut browser_config = BrowserConfig::from_skin_theme(&skin.theme);

    // Set up platform services.
    let platform = DesktopPlatform::new();

    // Set up VFS with demo content + apps.
    let mut vfs = MemoryVfs::new();
    populate_demo_vfs(&mut vfs);

    // Discover apps.
    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"))?;
    log::info!("Discovered {} apps", apps.len());

    // Set up dashboard.
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let mut dashboard = DashboardState::new(dash_config, apps);

    // Set up PSIX-style bars.
    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();
    let mut start_menu =
        StartMenuState::new_with_theme(StartMenuState::default_items(), &active_theme);

    // Set up command interpreter.
    let mut cmd_reg = CommandRegistry::new();
    register_builtins(&mut cmd_reg);

    // Terminal state.
    let mut cwd = "/".to_string();
    let mut input_buf = String::new();
    let mut output_lines: Vec<String> = vec![
        "OASIS_OS v0.1.0 -- Type 'help' for commands".to_string(),
        "F1=terminal  F2=on-screen keyboard  Escape=quit".to_string(),
        String::new(),
    ];

    // On-screen keyboard state.
    let mut osk: Option<OskState> = None;

    // Running app state.
    let mut app_runner: Option<AppRunner> = None;

    // Window manager state (Desktop mode).
    let mut wm = WindowManager::with_theme(
        config.screen_width,
        config.screen_height,
        skin.theme.build_wm_theme(),
    );
    let mut open_runners: Vec<(String, AppRunner)> = Vec::new();

    // Browser widget state (lives outside open_runners because it's not an AppRunner).
    let mut browser: Option<BrowserWidget> = None;

    // Networking state.
    let mut net_backend = StdNetworkBackend::new();
    let mut listener: Option<RemoteListener> = None;
    let mut remote_client: Option<RemoteClient> = None;

    // TLS provider (independent of net_backend to avoid borrow conflicts).
    let tls_provider = RustlsTlsProvider::new();

    // Set up scene graph and apply skin layout.
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);

    // -- Wallpaper: generate from skin config and load as texture --
    let wallpaper_tex = {
        let wp_data = wallpaper::generate_from_config(
            config.screen_width,
            config.screen_height,
            &active_theme,
        );
        backend.load_texture(config.screen_width, config.screen_height, &wp_data)?
    };
    setup_wallpaper(
        &mut sdi,
        wallpaper_tex,
        config.screen_width,
        config.screen_height,
    );
    log::info!("Wallpaper loaded");

    // -- Mouse cursor: generate procedural arrow and load as texture --
    let mut mouse_cursor = CursorState::new(config.screen_width, config.screen_height);
    {
        let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels();
        let cursor_tex = backend.load_texture(cw, ch, &cursor_pixels)?;
        // Set texture on the cursor SDI object after first update_sdi creates it.
        mouse_cursor.update_sdi(&mut sdi);
        if let Ok(obj) = sdi.get_mut("mouse_cursor") {
            obj.texture = Some(cursor_tex);
        }
    }
    log::info!("Mouse cursor loaded");

    let mut mode = Mode::Dashboard;
    let bg_color = Color::rgb(10, 10, 18);

    // Boot transition: fade in from black.
    let fade_frames = skin.features.transition_fade_frames.unwrap_or(15);
    let mut active_transition: Option<transition::TransitionState> = Some(
        transition::fade_in_custom(config.screen_width, config.screen_height, fade_frames),
    );

    // Frame counter for periodic updates (clock, battery).
    let mut frame_counter: u64 = 0;

    'running: loop {
        frame_counter += 1;

        // Update system info every ~60 frames (~1s at 60fps).
        if frame_counter.is_multiple_of(60) {
            let time = platform.now().ok();
            let power = platform.power_info().ok();
            status_bar.update_info(time.as_ref(), power.as_ref());
        }

        let events = backend.poll_events();
        for event in &events {
            // Always update mouse cursor position.
            mouse_cursor.handle_input(event);

            // OSK intercepts input when active.
            if mode == Mode::Osk {
                if let Some(ref mut osk_state) = osk {
                    match event {
                        InputEvent::Quit => break 'running,
                        InputEvent::Backspace => {
                            osk_state.buffer.pop();
                        },
                        InputEvent::ButtonPress(btn) => {
                            osk_state.handle_input(btn);
                            if let Some(text) = osk_state.confirmed_text() {
                                output_lines.push(format!("[OSK] Input: {text}"));
                                osk_state.hide_sdi(&mut sdi);
                                osk = None;
                                mode = Mode::Dashboard;
                            } else if osk_state.is_cancelled() {
                                output_lines.push("[OSK] Cancelled".to_string());
                                osk_state.hide_sdi(&mut sdi);
                                osk = None;
                                mode = Mode::Dashboard;
                            }
                        },
                        _ => {},
                    }
                }
                continue;
            }

            // Desktop mode: windowed WM rendering.
            if mode == Mode::Desktop {
                match event {
                    InputEvent::Quit => break 'running,
                    InputEvent::PointerClick { x, y } => {
                        let wm_event =
                            wm.handle_input(&InputEvent::PointerClick { x: *x, y: *y }, &mut sdi);
                        match wm_event {
                            WmEvent::WindowClosed(id) => {
                                open_runners.retain(|(rid, _)| *rid != id);
                                if id == "browser" {
                                    browser = None;
                                }
                                if wm.window_count() == 0 {
                                    mode = Mode::Dashboard;
                                }
                            },
                            WmEvent::ContentClick(id, lx, ly) => {
                                if id == "browser"
                                    && let Some(ref mut bw) = browser
                                {
                                    // Convert content-local coords to absolute.
                                    let abs_x = bw.window_x() + lx;
                                    let abs_y = bw.window_y() + ly;
                                    bw.handle_input(
                                        &InputEvent::PointerClick { x: abs_x, y: abs_y },
                                        &vfs,
                                    );
                                }
                            },
                            WmEvent::DesktopClick(_, _) => {
                                // Click on empty desktop -- return to Dashboard.
                                if wm.window_count() == 0 {
                                    mode = Mode::Dashboard;
                                }
                            },
                            _ => {},
                        }
                    },
                    InputEvent::CursorMove { x, y } => {
                        wm.handle_input(&InputEvent::CursorMove { x: *x, y: *y }, &mut sdi);
                    },
                    InputEvent::PointerRelease { x, y } => {
                        wm.handle_input(&InputEvent::PointerRelease { x: *x, y: *y }, &mut sdi);
                    },
                    InputEvent::ButtonPress(Button::Cancel) => {
                        // Close the active window, or return to Dashboard.
                        if let Some(active_id) = wm.active_window().map(|s| s.to_string()) {
                            let _ = wm.close_window(&active_id, &mut sdi);
                            open_runners.retain(|(rid, _)| *rid != active_id);
                            if active_id == "browser" {
                                browser = None;
                            }
                            if wm.window_count() == 0 {
                                mode = Mode::Dashboard;
                            }
                        } else {
                            mode = Mode::Dashboard;
                        }
                    },
                    InputEvent::ButtonPress(Button::Start) => {
                        mode = Mode::Terminal;
                    },
                    InputEvent::TextInput(ch) => {
                        if wm.active_window() == Some("browser")
                            && let Some(ref mut bw) = browser
                        {
                            bw.handle_input(&InputEvent::TextInput(*ch), &vfs);
                        }
                    },
                    InputEvent::Backspace => {
                        if wm.active_window() == Some("browser")
                            && let Some(ref mut bw) = browser
                        {
                            bw.handle_input(&InputEvent::Backspace, &vfs);
                        }
                    },
                    InputEvent::ButtonPress(btn) => {
                        if let Some(active_id) = wm.active_window().map(|s| s.to_string()) {
                            if active_id == "browser" {
                                // Route input to browser widget.
                                if let Some(ref mut bw) = browser {
                                    bw.handle_input(&InputEvent::ButtonPress(*btn), &vfs);
                                }
                            } else if let Some((_, runner)) =
                                open_runners.iter_mut().find(|(id, _)| *id == active_id)
                            {
                                match runner.handle_input(btn, &vfs) {
                                    AppAction::Exit => {
                                        let _ = wm.close_window(&active_id, &mut sdi);
                                        open_runners.retain(|(rid, _)| *rid != active_id);
                                        if wm.window_count() == 0 {
                                            mode = Mode::Dashboard;
                                        }
                                    },
                                    AppAction::SwitchToTerminal => {
                                        mode = Mode::Terminal;
                                    },
                                    AppAction::None => {},
                                }
                            }
                        }
                    },
                    _ => {},
                }
                continue;
            }

            // App mode: fullscreen SDI rendering.
            if mode == Mode::App {
                if let Some(ref mut runner) = app_runner {
                    match event {
                        InputEvent::Quit => break 'running,
                        InputEvent::ButtonPress(btn) => match runner.handle_input(btn, &vfs) {
                            AppAction::Exit => {
                                AppRunner::hide_sdi(&mut sdi);
                                app_runner = None;
                                mode = Mode::Dashboard;
                            },
                            AppAction::SwitchToTerminal => {
                                AppRunner::hide_sdi(&mut sdi);
                                app_runner = None;
                                mode = Mode::Terminal;
                            },
                            AppAction::None => {},
                        },
                        _ => {},
                    }
                }
                continue;
            }

            match event {
                InputEvent::Quit => break 'running,
                InputEvent::ButtonPress(Button::Cancel) if mode == Mode::Dashboard => {
                    break 'running;
                },

                // Launch app from dashboard as floating window.
                InputEvent::ButtonPress(Button::Confirm) if mode == Mode::Dashboard => {
                    if bottom_bar.active_tab == MediaTab::None
                        && let Some(app) = dashboard.selected_app()
                    {
                        log::info!("Launching app: {}", app.title);
                        if app.title == "Terminal" {
                            mode = Mode::Terminal;
                        } else if app.title == "Browser" {
                            let win_id = "browser";
                            if wm.get_window(win_id).is_some() {
                                let _ = wm.focus_window(win_id, &mut sdi);
                            } else {
                                let wc = WindowConfig {
                                    id: win_id.to_string(),
                                    title: "Browser".to_string(),
                                    x: None,
                                    y: None,
                                    width: 380,
                                    height: 220,
                                    window_type: WindowType::AppWindow,
                                };
                                let _ = wm.create_window(&wc, &mut sdi);
                                let mut bw = BrowserWidget::new(browser_config.clone());
                                bw.set_tls_provider(Box::new(tls_provider.clone()));
                                bw.set_window(0, 0, 380, 220);
                                let home = bw.config.features.home_url.clone();
                                bw.navigate_vfs(&home, &vfs);
                                browser = Some(bw);
                            }
                            mode = Mode::Desktop;
                        } else {
                            let win_id = app.title.to_lowercase().replace(' ', "_");
                            if wm.get_window(&win_id).is_some() {
                                let _ = wm.focus_window(&win_id, &mut sdi);
                            } else {
                                let wc = WindowConfig {
                                    id: win_id.clone(),
                                    title: app.title.clone(),
                                    x: None,
                                    y: None,
                                    width: 380,
                                    height: 220,
                                    window_type: WindowType::AppWindow,
                                };
                                let _ = wm.create_window(&wc, &mut sdi);
                                open_runners.push((win_id, AppRunner::launch(app, &vfs)));
                            }
                            mode = Mode::Desktop;
                        }
                        active_transition = Some(transition::fade_in_custom(
                            config.screen_width,
                            config.screen_height,
                            skin.features.transition_fade_frames.unwrap_or(15),
                        ));
                    }
                },

                // Pointer click on dashboard: start menu takes priority.
                InputEvent::PointerClick { x, y } if mode == Mode::Dashboard => {
                    // Start button click toggles the menu.
                    if start_menu.hit_test_button(*x, *y) {
                        start_menu.toggle();
                        continue;
                    }
                    // If menu is open, check for item clicks or close on outside.
                    if start_menu.open {
                        if let Some(action) = start_menu.hit_test_item(*x, *y) {
                            start_menu.close();
                            if action == StartMenuAction::Exit {
                                break 'running;
                            }
                            handle_start_action(
                                &action,
                                &mut mode,
                                &dashboard,
                                &mut wm,
                                &mut sdi,
                                &mut open_runners,
                                &mut browser,
                                &browser_config,
                                &vfs,
                                &config,
                                &mut active_transition,
                                &tls_provider,
                            );
                        } else {
                            start_menu.close();
                        }
                        continue;
                    }
                    if bottom_bar.active_tab == MediaTab::None {
                        let cfg = &dashboard.config;
                        let gx = *x - cfg.grid_x;
                        let gy = *y - cfg.grid_y;
                        if gx >= 0 && gy >= 0 {
                            let col = gx as usize / cfg.cell_w as usize;
                            let row = gy as usize / cfg.cell_h as usize;
                            if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                                let idx = row * cfg.grid_cols as usize + col;
                                let page_apps = dashboard.current_page_apps().len();
                                if idx < page_apps {
                                    if dashboard.selected == idx {
                                        // Double-click (already selected) -- launch.
                                        if let Some(app) = dashboard.selected_app() {
                                            log::info!("Click-launching app: {}", app.title);
                                            if app.title == "Terminal" {
                                                mode = Mode::Terminal;
                                            } else if app.title == "Browser" {
                                                let win_id = "browser";
                                                if wm.get_window(win_id).is_some() {
                                                    let _ = wm.focus_window(win_id, &mut sdi);
                                                } else {
                                                    let wc = WindowConfig {
                                                        id: win_id.to_string(),
                                                        title: "Browser".to_string(),
                                                        x: None,
                                                        y: None,
                                                        width: 380,
                                                        height: 220,
                                                        window_type: WindowType::AppWindow,
                                                    };
                                                    let _ = wm.create_window(&wc, &mut sdi);
                                                    let mut bw =
                                                        BrowserWidget::new(browser_config.clone());
                                                    bw.set_tls_provider(Box::new(
                                                        tls_provider.clone(),
                                                    ));
                                                    bw.set_window(0, 0, 380, 220);
                                                    let home = bw.config.features.home_url.clone();
                                                    bw.navigate_vfs(&home, &vfs);
                                                    browser = Some(bw);
                                                }
                                                mode = Mode::Desktop;
                                            } else {
                                                let win_id =
                                                    app.title.to_lowercase().replace(' ', "_");
                                                if wm.get_window(&win_id).is_some() {
                                                    let _ = wm.focus_window(&win_id, &mut sdi);
                                                } else {
                                                    let wc = WindowConfig {
                                                        id: win_id.clone(),
                                                        title: app.title.clone(),
                                                        x: None,
                                                        y: None,
                                                        width: 380,
                                                        height: 220,
                                                        window_type: WindowType::AppWindow,
                                                    };
                                                    let _ = wm.create_window(&wc, &mut sdi);
                                                    open_runners.push((
                                                        win_id,
                                                        AppRunner::launch(app, &vfs),
                                                    ));
                                                }
                                                mode = Mode::Desktop;
                                            }
                                            active_transition = Some(transition::fade_in_custom(
                                                config.screen_width,
                                                config.screen_height,
                                                skin.features.transition_fade_frames.unwrap_or(15),
                                            ));
                                        }
                                    } else {
                                        dashboard.selected = idx;
                                    }
                                }
                            }
                        }
                    }
                },

                InputEvent::ButtonPress(Button::Start) => {
                    // F1 toggles between dashboard and terminal.
                    mode = match mode {
                        Mode::Dashboard => Mode::Terminal,
                        Mode::Terminal => Mode::Dashboard,
                        Mode::App => Mode::App,
                        Mode::Osk => Mode::Osk,
                        Mode::Desktop => Mode::Desktop, // handled above
                    };
                },
                InputEvent::ButtonPress(Button::Select) => {
                    // F2 opens the on-screen keyboard.
                    if mode != Mode::Osk {
                        let osk_cfg = OskConfig {
                            title: "On-Screen Keyboard".to_string(),
                            ..OskConfig::default()
                        };
                        osk = Some(OskState::new(osk_cfg, ""));
                        mode = Mode::Osk;
                        log::info!("OSK opened");
                    }
                },

                // L trigger: cycle top tabs (status bar).
                InputEvent::TriggerPress(Trigger::Left) if mode == Mode::Dashboard => {
                    status_bar.next_tab();
                    bottom_bar.l_pressed = true;
                },
                InputEvent::TriggerRelease(Trigger::Left) => {
                    bottom_bar.l_pressed = false;
                },

                // R trigger: cycle media category tabs (bottom bar).
                InputEvent::TriggerPress(Trigger::Right) if mode == Mode::Dashboard => {
                    bottom_bar.next_tab();
                    bottom_bar.r_pressed = true;
                    active_transition = Some(transition::fade_in_custom(
                        config.screen_width,
                        config.screen_height,
                        skin.features.transition_fade_frames.unwrap_or(15),
                    ));
                },
                InputEvent::TriggerRelease(Trigger::Right) => {
                    bottom_bar.r_pressed = false;
                },

                // Start menu intercepts input when open.
                InputEvent::ButtonPress(btn) if mode == Mode::Dashboard && start_menu.open => {
                    let action = start_menu.handle_input(btn);
                    if action == StartMenuAction::Exit {
                        break 'running;
                    }
                    if action != StartMenuAction::None {
                        handle_start_action(
                            &action,
                            &mut mode,
                            &dashboard,
                            &mut wm,
                            &mut sdi,
                            &mut open_runners,
                            &mut browser,
                            &browser_config,
                            &vfs,
                            &config,
                            &mut active_transition,
                            &tls_provider,
                        );
                    }
                },

                // Dashboard input: D-pad navigation.
                InputEvent::ButtonPress(btn) if mode == Mode::Dashboard => match btn {
                    Button::Up | Button::Down | Button::Left | Button::Right => {
                        if bottom_bar.active_tab == MediaTab::None {
                            dashboard.handle_input(btn);
                        }
                    },
                    Button::Triangle => {
                        if bottom_bar.active_tab == MediaTab::None {
                            dashboard.next_page();
                            bottom_bar.current_page = dashboard.page;
                        }
                    },
                    Button::Square => {
                        if bottom_bar.active_tab == MediaTab::None {
                            dashboard.prev_page();
                            bottom_bar.current_page = dashboard.page;
                        }
                    },
                    _ => {},
                },

                // Terminal input.
                InputEvent::TextInput(ch) if mode == Mode::Terminal => {
                    input_buf.push(*ch);
                },
                InputEvent::Backspace if mode == Mode::Terminal => {
                    input_buf.pop();
                },
                InputEvent::ButtonPress(Button::Confirm) if mode == Mode::Terminal => {
                    let line = input_buf.clone();
                    input_buf.clear();
                    if !line.is_empty() {
                        output_lines.push(format!("> {line}"));
                        let mut pending_skin_swap: Option<String> = None;
                        {
                            let mut env = Environment {
                                cwd: cwd.clone(),
                                vfs: &mut vfs,
                                power: Some(&platform),
                                time: Some(&platform),
                                usb: Some(&platform),
                                network: None,
                                tls: Some(&tls_provider),
                            };
                            match cmd_reg.execute(&line, &mut env) {
                                Ok(CommandOutput::Text(text)) => {
                                    for l in text.lines() {
                                        output_lines.push(l.to_string());
                                    }
                                },
                                Ok(CommandOutput::Table { headers, rows }) => {
                                    output_lines.push(headers.join(" | "));
                                    for row in &rows {
                                        output_lines.push(row.join(" | "));
                                    }
                                },
                                Ok(CommandOutput::Clear) => output_lines.clear(),
                                Ok(CommandOutput::None) => {},
                                Ok(CommandOutput::ListenToggle { port }) => {
                                    if port == 0 {
                                        if let Some(ref mut l) = listener {
                                            l.stop();
                                            listener = None;
                                            output_lines
                                                .push("Remote listener stopped.".to_string());
                                        } else {
                                            output_lines.push("No listener running.".to_string());
                                        }
                                    } else if listener.is_some() {
                                        output_lines.push(
                                            "Listener already running. Use 'listen stop' first."
                                                .to_string(),
                                        );
                                    } else {
                                        let cfg = ListenerConfig {
                                            port,
                                            psk: String::new(),
                                            max_connections: 4,
                                        };
                                        let mut l = RemoteListener::new(cfg);
                                        match l.start(&mut net_backend) {
                                            Ok(()) => {
                                                output_lines
                                                    .push(format!("Listening on port {port}."));
                                                listener = Some(l);
                                            },
                                            Err(e) => {
                                                output_lines.push(format!("Listen error: {e}"));
                                            },
                                        }
                                    }
                                },
                                Ok(CommandOutput::RemoteConnect { address, port, psk }) => {
                                    if remote_client.is_some() {
                                        output_lines.push(
                                            "Already connected. Disconnect first.".to_string(),
                                        );
                                    } else {
                                        let mut client = RemoteClient::new();
                                        match client.connect(
                                            &mut net_backend,
                                            &address,
                                            port,
                                            psk.as_deref(),
                                        ) {
                                            Ok(()) => {
                                                output_lines.push(format!(
                                                    "Connected to {address}:{port}."
                                                ));
                                                remote_client = Some(client);
                                            },
                                            Err(e) => {
                                                output_lines.push(format!("Connect error: {e}"));
                                            },
                                        }
                                    }
                                },
                                Ok(CommandOutput::BrowserSandbox { enable }) => {
                                    if let Some(ref mut bw) = browser {
                                        bw.config.features.sandbox_only = enable;
                                    }
                                    let state = if enable {
                                        "on (VFS only)"
                                    } else {
                                        "off (HTTP enabled)"
                                    };
                                    output_lines.push(format!("Browser sandbox: {state}"));
                                },
                                Ok(CommandOutput::SkinSwap { name }) => {
                                    pending_skin_swap = Some(name);
                                },
                                Err(e) => output_lines.push(format!("error: {e}")),
                            }
                            cwd = env.cwd;
                        } // drop env (releases &mut vfs)
                        if let Some(name) = pending_skin_swap {
                            match resolve_skin(&name) {
                                Ok(new_skin) => {
                                    let swapped = Skin::swap(&skin, new_skin, &mut sdi);
                                    active_theme = ActiveTheme::from_skin(&swapped.theme);
                                    browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                                    wm.set_theme(swapped.theme.build_wm_theme());
                                    let dash_config = DashboardConfig::from_features(
                                        &swapped.features,
                                        &active_theme,
                                    );
                                    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"))
                                        .unwrap_or_default();
                                    dashboard = DashboardState::new(dash_config, apps);
                                    bottom_bar.total_pages = dashboard.page_count();
                                    bottom_bar.current_page = 0;
                                    start_menu = StartMenuState::new_with_theme(
                                        StartMenuState::default_items(),
                                        &active_theme,
                                    );
                                    output_lines.push(format!(
                                        "Switched to skin: {}",
                                        swapped.manifest.name
                                    ));
                                    skin = swapped;
                                },
                                Err(e) => {
                                    output_lines.push(format!("Skin error: {e}"));
                                },
                            }
                        }
                    }
                    while output_lines.len() > MAX_OUTPUT_LINES {
                        output_lines.remove(0);
                    }
                },
                InputEvent::ButtonPress(Button::Square) if mode == Mode::Terminal => {
                    input_buf.pop();
                },
                InputEvent::ButtonPress(Button::Cancel) if mode == Mode::Terminal => {
                    set_terminal_visible(&mut sdi, false);
                    mode = Mode::Dashboard;
                },

                _ => {},
            }
        }

        // Poll remote listener for incoming commands.
        if let Some(ref mut l) = listener {
            let remote_cmds = l.poll(&mut net_backend);
            for (cmd_line, conn_idx) in remote_cmds {
                log::info!("Remote command from #{conn_idx}: {cmd_line}");
                let mut env = Environment {
                    cwd: cwd.clone(),
                    vfs: &mut vfs,
                    power: Some(&platform),
                    time: Some(&platform),
                    usb: Some(&platform),
                    network: None,
                    tls: Some(&tls_provider),
                };
                let response = match cmd_reg.execute(&cmd_line, &mut env) {
                    Ok(CommandOutput::Text(text)) => text,
                    Ok(CommandOutput::Table { headers, rows }) => {
                        let mut out = headers.join(" | ");
                        for row in &rows {
                            out.push('\n');
                            out.push_str(&row.join(" | "));
                        }
                        out
                    },
                    Ok(CommandOutput::Clear) => "OK".to_string(),
                    Ok(CommandOutput::None) => "OK".to_string(),
                    Ok(CommandOutput::ListenToggle { .. })
                    | Ok(CommandOutput::RemoteConnect { .. }) => {
                        "Not available via remote.".to_string()
                    },
                    Ok(CommandOutput::BrowserSandbox { enable }) => {
                        if let Some(ref mut bw) = browser {
                            bw.config.features.sandbox_only = enable;
                        }
                        let state = if enable {
                            "on (VFS only)"
                        } else {
                            "off (HTTP enabled)"
                        };
                        format!("Browser sandbox: {state}")
                    },
                    Ok(CommandOutput::SkinSwap { name }) => match resolve_skin(&name) {
                        Ok(new_skin) => {
                            let swapped = Skin::swap(&skin, new_skin, &mut sdi);
                            active_theme = ActiveTheme::from_skin(&swapped.theme);
                            browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                            wm.set_theme(swapped.theme.build_wm_theme());
                            let msg = format!("Switched to skin: {}", swapped.manifest.name);
                            skin = swapped;
                            msg
                        },
                        Err(e) => format!("Skin error: {e}"),
                    },
                    Err(e) => format!("error: {e}"),
                };
                cwd = env.cwd;
                let _ = l.send_response(conn_idx, &response);
            }
        }

        // Poll remote client for received data.
        if let Some(ref mut client) = remote_client {
            let lines = client.poll();
            for line in lines {
                output_lines.push(format!("[remote] {line}"));
            }
            if !client.is_connected() {
                output_lines.push("[remote] Disconnected.".to_string());
                remote_client = None;
            }
            while output_lines.len() > MAX_OUTPUT_LINES {
                output_lines.remove(0);
            }
        }

        // Update SDI based on active mode.
        match mode {
            Mode::Dashboard => {
                set_terminal_visible(&mut sdi, false);
                AppRunner::hide_sdi(&mut sdi);

                if bottom_bar.active_tab == MediaTab::None {
                    dashboard.update_sdi(&mut sdi, &active_theme);
                } else {
                    dashboard.hide_sdi(&mut sdi);
                    update_media_page(&mut sdi, &bottom_bar);
                }

                status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
                bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
                if skin.features.start_menu {
                    start_menu.update_sdi(&mut sdi, &active_theme);
                }
            },
            Mode::Terminal => {
                dashboard.hide_sdi(&mut sdi);
                AppRunner::hide_sdi(&mut sdi);
                StatusBar::hide_sdi(&mut sdi);
                BottomBar::hide_sdi(&mut sdi);
                start_menu.close();
                start_menu.hide_sdi(&mut sdi);
                hide_media_page(&mut sdi);
                setup_terminal_objects(&mut sdi, &output_lines, &cwd, &input_buf);
            },
            Mode::App => {
                dashboard.hide_sdi(&mut sdi);
                set_terminal_visible(&mut sdi, false);
                hide_media_page(&mut sdi);
                start_menu.close();
                start_menu.hide_sdi(&mut sdi);
                // Show bars behind windows in app mode.
                status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
                bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
                if let Some(ref runner) = app_runner {
                    runner.update_sdi(&mut sdi);
                }
            },
            Mode::Desktop => {
                set_terminal_visible(&mut sdi, false);
                AppRunner::hide_sdi(&mut sdi);
                dashboard.hide_sdi(&mut sdi);
                hide_media_page(&mut sdi);
                start_menu.close();
                start_menu.hide_sdi(&mut sdi);
                status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
                bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
            },
            Mode::Osk => {
                if let Some(ref osk_state) = osk {
                    osk_state.update_sdi(&mut sdi);
                }
            },
        }

        // Update cursor SDI position (always on top).
        mouse_cursor.update_sdi(&mut sdi);

        // Ensure wallpaper is visible and at lowest z.
        if let Ok(obj) = sdi.get_mut("wallpaper") {
            obj.visible = true;
        }

        // -- Render --
        backend.clear(bg_color)?;
        if mode == Mode::Desktop && wm.window_count() > 0 {
            wm.draw_with_clips(&sdi, &mut backend, |window_id, cx, cy, cw, ch, be| {
                if window_id == "browser" {
                    if let Some(ref mut bw) = browser {
                        bw.set_window(cx, cy, cw, ch);
                        bw.paint(be)
                    } else {
                        Ok(())
                    }
                } else if let Some((_, runner)) =
                    open_runners.iter().find(|(id, _)| id == window_id)
                {
                    runner.draw_windowed(cx, cy, cw, ch, be)
                } else {
                    Ok(())
                }
            })?;
        } else {
            sdi.draw(&mut backend)?;
        }

        // Draw transition overlay if active.
        if let Some(ref mut trans) = active_transition {
            trans.draw_overlay(&mut backend)?;
            trans.tick();
            if trans.is_done() {
                active_transition = None;
            }
        }

        backend.swap_buffers()?;
    }

    backend.shutdown()?;
    log::info!("OASIS_OS shut down cleanly");
    Ok(())
}

/// Dispatch a start menu action (launch app, open terminal, exit).
#[allow(clippy::too_many_arguments)]
fn handle_start_action(
    action: &StartMenuAction,
    mode: &mut Mode,
    dashboard: &DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    open_runners: &mut Vec<(String, AppRunner)>,
    browser: &mut Option<BrowserWidget>,
    browser_config: &BrowserConfig,
    vfs: &MemoryVfs,
    config: &OasisConfig,
    active_transition: &mut Option<transition::TransitionState>,
    tls_provider: &RustlsTlsProvider,
) {
    match action {
        StartMenuAction::LaunchApp(title) => {
            // Find matching app in the dashboard's app list.
            let app = dashboard.apps.iter().find(|a| a.title == *title);
            if let Some(app) = app {
                if app.title == "Terminal" {
                    *mode = Mode::Terminal;
                } else if app.title == "Browser" {
                    let win_id = "browser";
                    if wm.get_window(win_id).is_some() {
                        let _ = wm.focus_window(win_id, sdi);
                    } else {
                        let wc = WindowConfig {
                            id: win_id.to_string(),
                            title: "Browser".to_string(),
                            x: None,
                            y: None,
                            width: 380,
                            height: 220,
                            window_type: WindowType::AppWindow,
                        };
                        let _ = wm.create_window(&wc, sdi);
                        let mut bw = BrowserWidget::new(browser_config.clone());
                        bw.set_tls_provider(Box::new(tls_provider.clone()));
                        bw.set_window(0, 0, 380, 220);
                        let home = bw.config.features.home_url.clone();
                        bw.navigate_vfs(&home, vfs);
                        *browser = Some(bw);
                    }
                    *mode = Mode::Desktop;
                } else {
                    let win_id = app.title.to_lowercase().replace(' ', "_");
                    if wm.get_window(&win_id).is_some() {
                        let _ = wm.focus_window(&win_id, sdi);
                    } else {
                        let wc = WindowConfig {
                            id: win_id.clone(),
                            title: app.title.clone(),
                            x: None,
                            y: None,
                            width: 380,
                            height: 220,
                            window_type: WindowType::AppWindow,
                        };
                        let _ = wm.create_window(&wc, sdi);
                        open_runners.push((win_id, AppRunner::launch(app, vfs)));
                    }
                    *mode = Mode::Desktop;
                }
                *active_transition = Some(transition::fade_in_custom(
                    config.screen_width,
                    config.screen_height,
                    15,
                ));
            }
        },
        StartMenuAction::OpenTerminal => {
            *mode = Mode::Terminal;
        },
        StartMenuAction::Exit => {
            // Signal exit by switching to a terminal + immediate quit isn't
            // possible from here, so we set a sentinel that the main loop
            // handles. For simplicity, just switch to terminal with a log.
            log::info!("Start menu: Exit requested");
            // We can't break from here, but the caller checks for Exit.
        },
        StartMenuAction::RunCommand(_) | StartMenuAction::None => {},
    }
}

/// Set up the wallpaper SDI object at z=-1000 (behind everything).
fn setup_wallpaper(sdi: &mut SdiRegistry, tex: TextureId, w: u32, h: u32) {
    let obj = sdi.create("wallpaper");
    obj.x = 0;
    obj.y = 0;
    obj.w = w;
    obj.h = h;
    obj.texture = Some(tex);
    obj.z = -1000;
}

/// Update SDI objects for the currently selected media category page.
fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = 14;
        obj.text_color = Color::rgb(160, 200, 180);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = 160;
        obj.y = 120;
        obj.visible = true;
        obj.text = Some(format!("[ {} Page ]", bottom_bar.active_tab.label()));
    }

    let hint_name = "media_page_hint";
    if !sdi.contains(hint_name) {
        let obj = sdi.create(hint_name);
        obj.font_size = 10;
        obj.text_color = Color::rgb(100, 130, 110);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(hint_name) {
        obj.x = 130;
        obj.y = 145;
        obj.visible = true;
        obj.text = Some("Press R to cycle categories".to_string());
    }
}

/// Hide media page SDI objects.
fn hide_media_page(sdi: &mut SdiRegistry) {
    for name in &["media_page_text", "media_page_hint"] {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

/// Create demo VFS content including fake apps.
fn populate_demo_vfs(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/tmp").unwrap();
    vfs.write(
        "/home/user/readme.txt",
        b"Welcome to OASIS_OS!\nType 'help' for available commands.",
    )
    .unwrap();
    vfs.write("/etc/hostname", b"oasis").unwrap();
    vfs.write("/etc/version", b"0.1.0").unwrap();
    vfs.write(
        "/etc/hosts.toml",
        b"[[host]]\nname = \"briefcase\"\naddress = \"192.168.0.50\"\nport = 9000\nprotocol = \"oasis-terminal\"\n",
    )
    .unwrap();

    vfs.mkdir("/apps").unwrap();
    for name in &[
        "File Manager",
        "Settings",
        "Network",
        "Terminal",
        "Music Player",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
        "Browser",
    ] {
        vfs.mkdir(&format!("/apps/{name}")).unwrap();
    }

    // Browser home page content.
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/home").unwrap();
    vfs.write(
        "/sites/home/index.html",
        b"<html><head><title>OASIS Home</title></head><body>\
          <h1>Welcome to OASIS Browser</h1>\
          <p>A lightweight HTML/CSS browser for OASIS_OS.</p>\
          <ul>\
          <li><a href=\"/sites/home/about.html\">About OASIS Browser</a></li>\
          </ul>\
          </body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/home/about.html",
        b"<html><head><title>About</title></head><body>\
          <h1>About OASIS Browser</h1>\
          <p>Supports HTML, CSS, and Gemini protocol.</p>\
          <p><a href=\"/sites/home/index.html\">Back to home</a></p>\
          </body></html>",
    )
    .unwrap();

    vfs.mkdir("/home/user/music").unwrap();
    vfs.mkdir("/home/user/photos").unwrap();

    load_disk_samples(vfs);

    vfs.mkdir("/home/user/scripts").unwrap();
    vfs.write(
        "/home/user/scripts/hello.sh",
        b"# Demo script\necho Hello from OASIS_OS!\nstatus\npwd\n",
    )
    .unwrap();

    vfs.mkdir("/var").unwrap();
    vfs.mkdir("/var/audio").unwrap();
}

/// Try to load real sample files from the `samples/` directory on disk.
fn load_disk_samples(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;
    use std::path::Path;

    let samples_dir = Path::new("samples");

    let music_files = ["ambient_dawn.mp3", "nightfall_theme.mp3"];
    for name in &music_files {
        let disk_path = samples_dir.join(name);
        let vfs_path = format!("/home/user/music/{name}");
        if disk_path.exists()
            && let Ok(data) = std::fs::read(&disk_path)
        {
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            vfs.write(&vfs_path, &data).unwrap();
            continue;
        }
        vfs.write(
            &vfs_path,
            format!("(placeholder: run samples/fetch-samples.sh for real audio)\nFile: {name}\n")
                .as_bytes(),
        )
        .unwrap();
    }

    let photo_files = ["sample_landscape.png"];
    for name in &photo_files {
        let disk_path = samples_dir.join(name);
        let vfs_path = format!("/home/user/photos/{name}");
        if disk_path.exists()
            && let Ok(data) = std::fs::read(&disk_path)
        {
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            vfs.write(&vfs_path, &data).unwrap();
            continue;
        }
        vfs.write(
            &vfs_path,
            format!("(placeholder: run samples/fetch-samples.sh for real image)\nFile: {name}\n")
                .as_bytes(),
        )
        .unwrap();
    }

    load_disk_dir(vfs, &samples_dir.join("music"), "/home/user/music");
    load_disk_dir(vfs, &samples_dir.join("photos"), "/home/user/photos");
}

/// Load all files from a real disk directory into the VFS.
fn load_disk_dir(vfs: &mut MemoryVfs, disk_dir: &std::path::Path, vfs_dir: &str) {
    use oasis_core::vfs::Vfs;

    let Ok(entries) = std::fs::read_dir(disk_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && let Ok(data) = std::fs::read(&path)
        {
            let vfs_path = format!("{vfs_dir}/{name}");
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            vfs.write(&vfs_path, &data).unwrap();
        }
    }
}

/// Set terminal-mode SDI objects visible/hidden.
fn set_terminal_visible(sdi: &mut SdiRegistry, visible: bool) {
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = visible;
    }
    for i in 0..MAX_OUTPUT_LINES {
        let name = format!("term_line_{i}");
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.visible = visible;
        }
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = visible;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.visible = visible;
    }
}

/// Create/update terminal-mode SDI objects.
fn setup_terminal_objects(
    sdi: &mut SdiRegistry,
    output_lines: &[String],
    cwd: &str,
    input_buf: &str,
) {
    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = 4;
        obj.y = 26;
        obj.w = 472;
        obj.h = 220;
        obj.color = Color::rgb(12, 12, 20);
        obj.border_radius = Some(4);
        obj.stroke_width = Some(1);
        obj.stroke_color = Some(Color::rgba(255, 255, 255, 30));
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    for i in 0..MAX_OUTPUT_LINES {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = 8;
            obj.y = 28 + (i as i32) * 16;
            obj.font_size = 12;
            obj.text_color = Color::rgb(0, 200, 0);
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = output_lines.get(i).cloned();
            obj.visible = true;
        }
    }

    if !sdi.contains("term_input_bg") {
        let obj = sdi.create("term_input_bg");
        obj.x = 4;
        obj.y = 248;
        obj.w = 472;
        obj.h = 20;
        obj.color = Color::rgb(20, 20, 35);
        obj.border_radius = Some(3);
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = true;
    }

    if !sdi.contains("term_prompt") {
        let obj = sdi.create("term_prompt");
        obj.x = 8;
        obj.y = 250;
        obj.font_size = 12;
        obj.text_color = Color::rgb(100, 200, 255);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.text = Some(format!("{cwd}> {input_buf}_"));
        obj.visible = true;
    }
}

#[cfg(test)]
mod tests {
    use oasis_core::vfs::{MemoryVfs, Vfs};

    #[test]
    fn populate_creates_home_user() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(vfs.readdir("/home/user").is_ok(), "/home/user should exist");
    }

    #[test]
    fn populate_creates_etc_hostname() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/etc/hostname")
            .expect("/etc/hostname should exist");
        assert_eq!(data, b"oasis");
    }

    #[test]
    fn populate_creates_etc_version() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs.read("/etc/version").expect("/etc/version should exist");
        assert_eq!(data, b"0.1.0");
    }

    #[test]
    fn populate_creates_all_app_dirs() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let expected = [
            "File Manager",
            "Settings",
            "Network",
            "Terminal",
            "Music Player",
            "Photo Viewer",
            "Package Manager",
            "System Monitor",
            "Browser",
        ];
        for name in &expected {
            let path = format!("/apps/{name}");
            assert!(vfs.readdir(&path).is_ok(), "app dir should exist: {path}",);
        }
    }

    #[test]
    fn populate_creates_browser_home() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/sites/home/index.html")
            .expect("/sites/home/index.html should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("OASIS Browser"),
            "index.html should contain 'OASIS Browser', got: {text}",
        );
    }

    #[test]
    fn populate_creates_music_dir() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(
            vfs.readdir("/home/user/music").is_ok(),
            "/home/user/music should exist",
        );
    }

    #[test]
    fn populate_creates_photos_dir() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(
            vfs.readdir("/home/user/photos").is_ok(),
            "/home/user/photos should exist",
        );
    }

    #[test]
    fn populate_creates_scripts() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/home/user/scripts/hello.sh")
            .expect("/home/user/scripts/hello.sh should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("echo"),
            "hello.sh should contain 'echo', got: {text}",
        );
    }

    #[test]
    fn populate_creates_var_audio() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(vfs.readdir("/var/audio").is_ok(), "/var/audio should exist",);
    }

    #[test]
    fn populate_creates_hosts_toml() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/etc/hosts.toml")
            .expect("/etc/hosts.toml should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("briefcase"),
            "hosts.toml should contain 'briefcase', got: {text}",
        );
    }
}
