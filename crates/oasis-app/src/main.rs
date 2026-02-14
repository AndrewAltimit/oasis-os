//! OASIS_OS desktop entry point.
//!
//! PSIX-style UI with wallpaper, mouse cursor, status bar, 6x3 icon grid
//! dashboard, and bottom bar with media category tabs.
//! L trigger cycles top tabs, R trigger cycles media categories,
//! D-pad navigates the grid. Click to select/launch icons.
//! Press F1 to toggle terminal, F2 to toggle on-screen keyboard, Escape to quit.

mod app_state;
mod commands;
mod input;
mod launch;
mod render;
mod terminal_sdi;
mod vfs_setup;

use anyhow::Result;

use app_state::{AppState, Mode};
use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{Color, InputBackend, SdiBackend};
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::BrowserConfig;
use oasis_core::config::OasisConfig;
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::net::{RustlsTlsProvider, StdNetworkBackend};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::resolve_skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{
    CommandRegistry, register_agent_commands, register_builtins, register_plugin_commands,
};
use oasis_core::transition;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;

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
    let skin = resolve_skin(&skin_name)?;
    log::info!(
        "Loaded skin: {} v{}",
        skin.manifest.name,
        skin.manifest.version
    );

    // Derive runtime theme from the active skin.
    let active_theme = ActiveTheme::from_skin(&skin.theme);
    let browser_config = BrowserConfig::from_skin_theme(&skin.theme);

    // Set up platform services.
    let platform = DesktopPlatform::new();

    // Set up VFS with demo content + apps.
    let mut vfs = MemoryVfs::new();
    vfs_setup::populate_demo_vfs(&mut vfs);

    // Populate terminal documentation and shell profile in VFS.
    oasis_core::terminal::populate_man_pages(&mut vfs);
    oasis_core::terminal::populate_motd(&mut vfs);
    oasis_core::terminal::populate_profile(&mut vfs);

    // Discover apps.
    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"))?;
    log::info!("Discovered {} apps", apps.len());

    // Set up dashboard.
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let dashboard = DashboardState::new(dash_config, apps);

    // Set up PSIX-style bars.
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

    // Set up command interpreter.
    let mut cmd_reg = CommandRegistry::new();
    register_builtins(&mut cmd_reg);
    // Register additional command modules (script, transfer, update, plugin, agent, browser).
    oasis_core::script::register_script_commands(&mut cmd_reg);
    oasis_core::transfer::register_transfer_commands(&mut cmd_reg);
    oasis_core::update::register_update_commands(&mut cmd_reg);
    register_plugin_commands(&mut cmd_reg);
    register_agent_commands(&mut cmd_reg);
    oasis_core::browser::commands::register_browser_commands(&mut cmd_reg);

    // Window manager state (Desktop mode).
    let wm = WindowManager::with_theme(
        config.screen_width,
        config.screen_height,
        skin.theme.build_wm_theme(),
    );

    // Boot transition: fade in from black.
    let fade_frames = skin.features.transition_fade_frames.unwrap_or(15);
    let active_transition = Some(transition::fade_in_custom(
        config.screen_width,
        config.screen_height,
        fade_frames,
    ));

    let mouse_cursor = CursorState::new(config.screen_width, config.screen_height);

    let start_menu = StartMenuState::new_with_theme(StartMenuState::default_items(), &active_theme);

    // Assemble application state.
    let mut state = AppState {
        config,
        skin,
        active_theme,
        browser_config,
        platform,
        dashboard,
        status_bar: StatusBar::new(),
        bottom_bar,
        start_menu,
        cmd_reg,
        cwd: "/".to_string(),
        input_buf: String::new(),
        output_lines: vec![
            "OASIS_OS v0.1.0 -- Type 'help' for commands".to_string(),
            "F1=terminal  F2=on-screen keyboard  Escape=quit".to_string(),
            String::new(),
        ],
        osk: None,
        app_runner: None,
        wm,
        open_runners: Vec::new(),
        browser: None,
        net_backend: StdNetworkBackend::new(),
        listener: None,
        remote_client: None,
        tls_provider: RustlsTlsProvider::new(),
        mouse_cursor,
        mode: Mode::Dashboard,
        bg_color: Color::rgb(10, 10, 18),
        active_transition,
        frame_counter: 0,
    };

    // Set up scene graph and apply skin layout.
    let mut sdi = SdiRegistry::new();
    state.skin.apply_layout(&mut sdi);

    // -- Wallpaper: generate from skin config and load as texture --
    let wallpaper_tex = {
        let wp_data = wallpaper::generate_from_config(
            state.config.screen_width,
            state.config.screen_height,
            &state.active_theme,
        );
        backend.load_texture(
            state.config.screen_width,
            state.config.screen_height,
            &wp_data,
        )?
    };
    terminal_sdi::setup_wallpaper(
        &mut sdi,
        wallpaper_tex,
        state.config.screen_width,
        state.config.screen_height,
    );
    log::info!("Wallpaper loaded");

    // -- Mouse cursor: generate procedural arrow and load as texture --
    {
        let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels();
        let cursor_tex = backend.load_texture(cw, ch, &cursor_pixels)?;
        // Set texture on the cursor SDI object after first update_sdi creates it.
        state.mouse_cursor.update_sdi(&mut sdi);
        if let Ok(obj) = sdi.get_mut("mouse_cursor") {
            obj.texture = Some(cursor_tex);
        }
    }
    log::info!("Mouse cursor loaded");

    'running: loop {
        state.frame_counter += 1;

        // Update system info every ~60 frames (~1s at 60fps).
        if state.frame_counter.is_multiple_of(60) {
            let time = state.platform.now().ok();
            let power = state.platform.power_info().ok();
            state.status_bar.update_info(time.as_ref(), power.as_ref());
        }

        let events = backend.poll_events();
        for event in &events {
            state.mouse_cursor.handle_input(event);

            let result = match state.mode {
                Mode::Osk => input::handle_osk_input(event, &mut state, &mut sdi),
                Mode::Desktop => input::handle_desktop_input(event, &mut state, &mut sdi, &vfs),
                Mode::App => input::handle_app_input(event, &mut state, &mut sdi, &vfs),
                _ => input::handle_default_input(event, &mut state, &mut sdi, &mut vfs),
            };
            if result == input::InputResult::Quit {
                break 'running;
            }
        }

        // Poll remote listener for incoming commands.
        commands::poll_remote_listener(&mut state, &mut sdi, &mut vfs);

        // Poll remote client for received data.
        commands::poll_remote_client(&mut state);

        // Update SDI scene graph for the active mode.
        render::update_sdi(&mut state, &mut sdi);

        // -- Render --
        backend.clear(state.bg_color)?;
        if state.mode == Mode::Desktop && state.wm.window_count() > 0 {
            state
                .wm
                .draw_with_clips(&mut sdi, &mut backend, |window_id, cx, cy, cw, ch, be| {
                    if window_id == "browser" {
                        if let Some(ref mut bw) = state.browser {
                            bw.set_window(cx, cy, cw, ch);
                            bw.paint(be)
                        } else {
                            Ok(())
                        }
                    } else if let Some((_, runner)) =
                        state.open_runners.iter().find(|(id, _)| id == window_id)
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
        if let Some(ref mut trans) = state.active_transition {
            trans.draw_overlay(&mut backend)?;
            trans.tick();
            if trans.is_done() {
                state.active_transition = None;
            }
        }

        backend.swap_buffers()?;
    }

    backend.shutdown()?;
    log::info!("OASIS_OS shut down cleanly");
    Ok(())
}
