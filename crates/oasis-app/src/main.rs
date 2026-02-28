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
use oasis_core::terminal_sdi;
mod vfs_setup;

use anyhow::Result;

use app_state::{AppState, Mode};
use oasis_audio::RadioManager;
use oasis_backend_sdl::SdlAudioBackend;
use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{AudioBackend, InputBackend, NetworkBackend, SdiBackend};
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
use oasis_core::toast::{ToastLevel, ToastManager};
use oasis_core::transition;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut config = OasisConfig::default();

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

    // Use the skin's screen dimensions (e.g. 1024x768 for xp, 480x272 for classic).
    config.screen_width = skin.manifest.screen_width;
    config.screen_height = skin.manifest.screen_height;
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

    // Derive runtime theme from the active skin, applying screen dimensions.
    let active_theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(config.screen_width, config.screen_height);
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

    let mut mouse_cursor = CursorState::new(config.screen_width, config.screen_height);
    mouse_cursor.scale = active_theme.cursor_scale;

    let start_menu =
        StartMenuState::new_with_theme(StartMenuState::default_items(&active_theme), &active_theme);

    // Assemble application state.
    let clear_color = active_theme.clear_color;
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
        ftp_server: None,
        remote_client: None,
        tls_provider: RustlsTlsProvider::new(),
        mouse_cursor,
        mode: Mode::Dashboard,
        bg_color: clear_color,
        active_transition,
        frame_counter: 0,
        radio_manager: RadioManager::new(),
        radio_source: None,
        archive_catalog: None,
        audio_backend: {
            let mut ab = SdlAudioBackend::new();
            ab.init().ok();
            ab
        },
        terminal_scroll_offset: 0,
        toasts: ToastManager::new(),
    };

    // Show a welcome toast.
    state.toasts.show(
        format!("Skin: {}", state.skin.manifest.name),
        ToastLevel::Info,
        state.active_theme.toast_ttl,
    );

    // Load radio stations from VFS.
    state
        .radio_manager
        .load_stations(&vfs, "/etc/radio/stations.toml")
        .ok();

    // Auto-launch app via OASIS_APP env var (e.g. OASIS_APP=Browser).
    // Optionally OASIS_URL sets the initial URL for the browser.
    let auto_launch_app = std::env::var("OASIS_APP").ok();
    let auto_launch_url = std::env::var("OASIS_URL").ok();

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
        let (cursor_pixels, cw, ch) =
            cursor::generate_cursor_pixels(state.active_theme.cursor_scale);
        let cursor_tex = backend.load_texture(cw, ch, &cursor_pixels)?;
        // Set texture on the cursor SDI object after first update_sdi creates it.
        state.mouse_cursor.update_sdi(&mut sdi);
        if let Ok(obj) = sdi.get_mut("mouse_cursor") {
            obj.texture = Some(cursor_tex);
        }
    }
    log::info!("Mouse cursor loaded");

    // Apply auto-launch (after scene graph is fully set up).
    if let Some(ref app_name) = auto_launch_app {
        if let Some(app) = state
            .dashboard
            .apps
            .iter()
            .find(|a| a.title.eq_ignore_ascii_case(app_name))
        {
            let app = app.clone();
            let result = launch::launch_app_window(
                &app,
                &mut state.wm,
                &mut sdi,
                &mut state.open_runners,
                &mut state.browser,
                &state.browser_config,
                &vfs,
                &state.tls_provider,
            );
            launch::apply_launch(result, &mut state.mode);
            log::info!("Auto-launched app: {}", app.title);

            // Navigate browser to OASIS_URL if specified.
            if let Some(ref url) = auto_launch_url
                && let Some(ref mut bw) = state.browser
            {
                bw.navigate_vfs(url, &vfs);
                log::info!("Auto-navigated to: {url}");
            }
        } else {
            log::warn!("OASIS_APP={app_name}: app not found in dashboard");
        }
    }

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

        // Poll FTP server for incoming connections.
        commands::poll_ftp_server(&mut state, &mut vfs);

        // Poll remote client for received data.
        commands::poll_remote_client(&mut state);

        // Process pending VFS requests from app runners (e.g. radio tune).
        {
            let mut pending = None;
            if let Some(ref mut runner) = state.app_runner {
                pending = runner.take_pending_request();
            }
            if pending.is_none() {
                for (_, runner) in &mut state.open_runners {
                    if let Some(req) = runner.take_pending_request() {
                        pending = Some(req);
                        break;
                    }
                }
            }
            if let Some((path, data)) = pending {
                let _ = vfs.write(&path, data.as_bytes());
            }
        }

        // Tick radio manager: process VFS requests and drive streaming.
        {
            use oasis_audio::RADIO_REQUEST_PATH;

            if vfs.exists(RADIO_REQUEST_PATH)
                && let Ok(data) = vfs.read(RADIO_REQUEST_PATH)
            {
                let request = String::from_utf8_lossy(&data).to_string();
                if !request.is_empty() {
                    // Clear the request immediately.
                    let _ = vfs.write(RADIO_REQUEST_PATH, b"");

                    if let Some(target) = request.strip_prefix("tune ") {
                        // Resolve station by index or case-insensitive name.
                        let station = if let Ok(idx) = target.parse::<usize>() {
                            state.radio_manager.registry.stations.get(idx).cloned()
                        } else {
                            state
                                .radio_manager
                                .registry
                                .stations
                                .iter()
                                .find(|s| s.name.eq_ignore_ascii_case(target.trim()))
                                .cloned()
                        };
                        if let Some(station) = station {
                            let _ = state.radio_manager.tune(
                                &station.name,
                                station.bitrate,
                                &mut state.audio_backend,
                            );

                            if station.source_type == "archive" && !station.collection.is_empty() {
                                // Internet Archive: fetch catalog, build playlist.
                                let search_path = format!(
                                    "/advancedsearch.php?\
                                     q=collection:{col}+AND+mediatype:audio\
                                     &fl=identifier,title,creator\
                                     &sort=random&rows=50&output=json",
                                    col = station.collection
                                );
                                match https_get_body(
                                    &mut state.net_backend,
                                    &state.tls_provider,
                                    "archive.org",
                                    &search_path,
                                ) {
                                    Ok(body) => {
                                        let items = oasis_audio::radio::ArchiveCatalog
                                            ::parse_search_response(&body);
                                        if items.is_empty() {
                                            state
                                                .radio_manager
                                                .set_error("no items found in collection");
                                        } else {
                                            // Pick the first item and fetch its files.
                                            let (item_id, _title, creator) = &items[0];
                                            let files_path =
                                                oasis_audio::radio::ArchiveCatalog::files_api_path(
                                                    item_id,
                                                );
                                            match https_get_body(
                                                &mut state.net_backend,
                                                &state.tls_provider,
                                                "archive.org",
                                                &files_path,
                                            ) {
                                                Ok(files_body) => {
                                                    // Build catalog from all items.
                                                    let mut catalog =
                                                        oasis_audio::radio::ArchiveCatalog::new(
                                                            &station.collection,
                                                        );
                                                    // Add tracks from first item.
                                                    let tracks = oasis_audio::radio::ArchiveCatalog
                                                        ::parse_files_response(
                                                            &files_body, item_id, creator,
                                                        );
                                                    catalog.tracks.extend(tracks);
                                                    // Add tracks from remaining items
                                                    // (fetch up to 4 more for variety).
                                                    for (id, _t, c) in items.iter().skip(1).take(4)
                                                    {
                                                        let fp = oasis_audio::radio
                                                            ::ArchiveCatalog::files_api_path(id);
                                                        if let Ok(fb) = https_get_body(
                                                            &mut state.net_backend,
                                                            &state.tls_provider,
                                                            "archive.org",
                                                            &fp,
                                                        ) {
                                                            let ts = oasis_audio::radio
                                                                ::ArchiveCatalog
                                                                ::parse_files_response(&fb, id, c);
                                                            catalog.tracks.extend(ts);
                                                        }
                                                    }

                                                    if catalog.tracks.is_empty() {
                                                        state
                                                            .radio_manager
                                                            .set_error("no MP3 files found");
                                                    } else {
                                                        catalog.shuffle(state.frame_counter);
                                                        // Connect to first track.
                                                        if let Some(track) =
                                                            catalog.current_track().cloned()
                                                        {
                                                            match connect_archive_source(
                                                                &mut state.net_backend,
                                                                &state.tls_provider,
                                                                &track,
                                                            ) {
                                                                Ok(source) => {
                                                                    state.radio_source =
                                                                        Some(source);
                                                                },
                                                                Err(e) => {
                                                                    state
                                                                        .radio_manager
                                                                        .set_error(&e);
                                                                },
                                                            }
                                                        }
                                                        state.archive_catalog = Some(catalog);
                                                    }
                                                },
                                                Err(e) => {
                                                    state
                                                        .radio_manager
                                                        .set_error(&format!("files API: {e}"));
                                                },
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        state.radio_manager.set_error(&format!("search API: {e}"));
                                    },
                                }
                            } else if let Some((host, port, path, _tls)) =
                                parse_stream_url(&station.url)
                            {
                                // Icecast: connect to stream directly.
                                match state.net_backend.connect(&host, port) {
                                    Ok(stream) => {
                                        let source = oasis_audio::radio::IcecastSource::new(
                                            stream, &host, &path,
                                        );
                                        state.radio_source = Some(Box::new(source));
                                    },
                                    Err(e) => {
                                        state.radio_manager.set_error(&format!("connect: {e}"));
                                    },
                                }
                            } else {
                                state.radio_manager.set_error("invalid stream URL");
                            }
                        } else {
                            state
                                .radio_manager
                                .set_error(&format!("station not found: {target}"));
                        }
                    } else {
                        let _ = state
                            .radio_manager
                            .process_request(&request, &mut state.audio_backend);
                    }
                }
            }

            // Drive the radio state machine.
            let _ = state
                .radio_manager
                .tick(&mut state.radio_source, &mut state.audio_backend);

            // Auto-advance to next track for archive stations.
            if state.radio_manager.needs_next_track()
                && let Some(ref mut catalog) = state.archive_catalog
                && let Some(track) = catalog.next_track().cloned()
            {
                match connect_archive_source(&mut state.net_backend, &state.tls_provider, &track) {
                    Ok(source) => {
                        state.radio_source = Some(source);
                        state.radio_manager.continue_playing();
                    },
                    Err(e) => {
                        state.radio_manager.set_error(&e);
                    },
                }
            }

            // Publish radio status periodically (~4 times per second).
            if state.frame_counter.is_multiple_of(15) {
                let _ = state.radio_manager.publish_status(&mut vfs);
            }

            // Refresh radio app display if visible.
            if let Some(ref mut runner) = state.app_runner {
                runner.refresh_radio(&vfs);
            }
            for (_, runner) in &mut state.open_runners {
                runner.refresh_radio(&vfs);
            }
        }

        // Update SDI scene graph for the active mode.
        render::update_sdi(&mut state, &mut sdi);

        // Drive browser image streaming (progressive loading).
        if let Some(ref mut bw) = state.browser {
            bw.tick(&vfs);
        }

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
                        runner.draw_windowed(cx, cy, cw, ch, be, &state.active_theme)
                    } else {
                        Ok(())
                    }
                })?;
        } else {
            sdi.draw(&mut backend)?;
        }

        // Paint terminal scrollbar when in terminal mode.
        if state.mode == Mode::Terminal {
            terminal_sdi::paint_terminal_scrollbar(
                &mut backend,
                state.output_lines.len(),
                state.terminal_scroll_offset,
                &state.active_theme,
            )?;
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

/// Parse an HTTP/HTTPS stream URL into (host, port, path, use_tls).
fn parse_stream_url(url: &str) -> Option<(String, u16, String, bool)> {
    let (remainder, tls) = if let Some(r) = url.strip_prefix("https://") {
        (r, true)
    } else if let Some(r) = url.strip_prefix("http://") {
        (r, false)
    } else {
        return None;
    };
    let (host_port, path) = if let Some(idx) = remainder.find('/') {
        (&remainder[..idx], remainder[idx..].to_string())
    } else {
        (remainder, "/".to_string())
    };
    let default_port = if tls { 443 } else { 80 };
    let (host, port) = if let Some(idx) = host_port.rfind(':') {
        let port: u16 = host_port[idx + 1..].parse().ok()?;
        (host_port[..idx].to_string(), port)
    } else {
        (host_port.to_string(), default_port)
    };
    Some((host, port, path, tls))
}

/// Perform a blocking HTTPS GET and return the response body as a string.
///
/// Used for Internet Archive API calls (small JSON responses).
fn https_get_body(
    net_backend: &mut oasis_core::net::StdNetworkBackend,
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    host: &str,
    path: &str,
) -> std::result::Result<String, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let tcp = net_backend
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;

    let mut stream = tls_provider
        .connect_tls(tcp, host)
        .map_err(|e| format!("TLS: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Connection: close\r\nAccept: */*\r\n\r\n"
    );
    let req_bytes = request.as_bytes();
    let mut written = 0;
    while written < req_bytes.len() {
        written += stream
            .write(&req_bytes[written..])
            .map_err(|e| format!("write: {e}"))?;
    }

    // Read full response.
    let mut buf = [0u8; 8192];
    let mut response = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    continue;
                }
                if !response.is_empty() {
                    break;
                }
                return Err(format!("read: {e}"));
            },
        }
    }

    let text = String::from_utf8_lossy(&response).into_owned();
    // Strip HTTP headers.
    if let Some(idx) = text.find("\r\n\r\n") {
        Ok(text[idx + 4..].to_string())
    } else {
        Ok(text)
    }
}

/// Connect to archive.org over TLS and create an ArchiveSource for the given track.
fn connect_archive_source(
    net_backend: &mut oasis_core::net::StdNetworkBackend,
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    track: &oasis_audio::radio::ArchiveTrack,
) -> std::result::Result<Box<dyn oasis_audio::radio::RadioSource>, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let tcp = net_backend
        .connect("archive.org", 443)
        .map_err(|e| format!("connect: {e}"))?;
    let stream = tls_provider
        .connect_tls(tcp, "archive.org")
        .map_err(|e| format!("TLS: {e}"))?;

    let path = oasis_audio::radio::ArchiveCatalog::download_path(track);
    Ok(Box::new(oasis_audio::radio::ArchiveSource::new(
        stream,
        "archive.org",
        &path,
        &track.title,
        &track.creator,
    )))
}
