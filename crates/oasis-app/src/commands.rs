use oasis_backend_sdl::SdlBackend;
use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::SdiCore;
use oasis_core::browser::BrowserConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::net::{ListenerConfig, RemoteClient, RemoteListener};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin};
use oasis_core::startmenu::StartMenuState;
use oasis_core::terminal::{CommandOutput, CommandSignal, Environment};
use oasis_core::terminal_sdi;
use oasis_core::transfer::FtpServer;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;

#[cfg(test)]
use crate::app_state::UiLayer;
use crate::app_state::{AppState, ContentLayer, NetworkLayer, TerminalLayer};

/// Process a local terminal command result. Returns a pending skin swap name
/// if the command was `SkinSwap`.
pub fn process_command_output(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
) -> Option<String> {
    match result {
        Ok(CommandOutput::Text(text)) => {
            for l in text.lines() {
                state.terminal.output_lines.push(l.to_string());
            }
        },
        Ok(CommandOutput::Table { headers, rows }) => {
            state.terminal.output_lines.push(headers.join(" | "));
            for row in &rows {
                state.terminal.output_lines.push(row.join(" | "));
            }
        },
        Ok(CommandOutput::Clear) => state.terminal.output_lines.clear(),
        Ok(CommandOutput::None) => {},
        Ok(CommandOutput::Signal(CommandSignal::ListenToggle { port })) => {
            if port == 0 {
                if let Some(ref mut l) = state.net.listener {
                    l.stop();
                    state.net.listener = None;
                    state
                        .terminal
                        .output_lines
                        .push("Remote listener stopped.".to_string());
                } else {
                    state
                        .terminal
                        .output_lines
                        .push("No listener running.".to_string());
                }
            } else if state.net.listener.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("Listener already running. Use 'listen stop' first.".to_string());
            } else {
                let cfg = ListenerConfig {
                    port,
                    psk: String::new(),
                    max_connections: 4,
                    ..ListenerConfig::default()
                };
                let mut l = RemoteListener::new(cfg);
                match l.start(&mut state.net.backend) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Listening on port {port}."));
                        state.net.listener = Some(l);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Listen error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::RemoteConnect { address, port, psk })) => {
            if state.net.remote_client.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("Already connected. Disconnect first.".to_string());
            } else {
                let mut client = RemoteClient::new();
                match client.connect(&mut state.net.backend, &address, port, psk.as_deref()) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Connected to {address}:{port}."));
                        state.net.remote_client = Some(client);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Connect error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox { enable })) => {
            if let Some(ref mut bw) = state.content.browser {
                bw.config.features.sandbox_only = enable;
            }
            let st = if enable {
                "on (VFS only)"
            } else {
                "off (HTTP enabled)"
            };
            state
                .terminal
                .output_lines
                .push(format!("Browser sandbox: {st}"));
        },
        Ok(CommandOutput::Signal(CommandSignal::FtpToggle { port, password })) => {
            if port == 0 {
                if let Some(ref mut f) = state.net.ftp_server {
                    f.stop();
                    state.net.ftp_server = None;
                    state
                        .terminal
                        .output_lines
                        .push("FTP server stopped.".to_string());
                } else {
                    state
                        .terminal
                        .output_lines
                        .push("No FTP server running.".to_string());
                }
            } else if state.net.ftp_server.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("FTP server already running. Use 'ftp stop' first.".to_string());
            } else {
                let mut server = FtpServer::new(port);
                if let Some(pass) = password {
                    server = server.with_password(pass);
                }
                match server.start(&mut state.net.backend) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("FTP server listening on port {port}."));
                        state.net.ftp_server = Some(server);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("FTP server error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap { name })) => {
            return Some(name);
        },
        Ok(CommandOutput::Multi(outputs)) => {
            let mut skin_swap = None;
            for output in outputs {
                let result = process_command_output(Ok(output), state);
                if result.is_some() {
                    skin_swap = result;
                }
            }
            return skin_swap;
        },
        Err(e) => {
            state.terminal.output_lines.push(format!("error: {e}"));
        },
    }
    None
}

/// Apply a skin swap after the Environment borrow has been dropped.
pub fn apply_skin_swap(name: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    match resolve_skin(name) {
        Ok(new_skin) => {
            let sw = state.active_theme.screen_w;
            let sh = state.active_theme.screen_h;
            let swapped = Skin::swap_scaled(&state.skin, new_skin, sdi, sw, sh);
            state.active_theme = ActiveTheme::from_skin(&swapped.theme)
                .with_screen_size(sw, sh)
                .with_features(&swapped.features);
            state.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
            state.wm.set_theme(swapped.theme.build_wm_theme());
            let dash_config =
                DashboardConfig::from_features(&swapped.features, &state.active_theme);
            let apps = discover_apps(vfs, "/apps", Some("OASISOS")).unwrap_or_default();
            state.ui.dashboard = DashboardState::new(dash_config, apps);
            state.ui.bottom_bar.total_pages = state.ui.dashboard.page_count();
            state.ui.bottom_bar.current_page = 0;
            state.ui.start_menu = StartMenuState::new_with_theme(
                StartMenuState::default_items(&state.active_theme),
                &state.active_theme,
            );
            state
                .terminal
                .output_lines
                .push(format!("Switched to skin: {}", swapped.manifest.name));
            state.skin = swapped;
        },
        Err(e) => {
            state.terminal.output_lines.push(format!("Skin error: {e}"));
        },
    }
}

/// Minimum virtual resolution accepted from a live resize request. Anything
/// smaller makes the dashboard/window-manager layout unusable.
const MIN_RESOLUTION_W: u32 = 320;
const MIN_RESOLUTION_H: u32 = 240;
/// Maximum virtual resolution — primarily a sanity bound. Requests beyond
/// this are rejected rather than clamped so the caller notices.
const MAX_RESOLUTION_W: u32 = 3840;
const MAX_RESOLUTION_H: u32 = 2160;

/// Apply a live resolution change. Rebuilds skin layout at the new size,
/// resizes the SDL window + shader bridge, and re-derives the dashboard,
/// window manager, and cursor state. No-op if `(new_w, new_h)` already
/// matches the active resolution.
pub fn apply_resolution_change(
    new_w: u32,
    new_h: u32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut SdlBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &MemoryVfs,
) {
    if new_w < MIN_RESOLUTION_W
        || new_h < MIN_RESOLUTION_H
        || new_w > MAX_RESOLUTION_W
        || new_h > MAX_RESOLUTION_H
    {
        state.terminal.output_lines.push(format!(
            "Resolution {new_w}x{new_h} out of range ({MIN_RESOLUTION_W}x{MIN_RESOLUTION_H} \
             to {MAX_RESOLUTION_W}x{MAX_RESOLUTION_H})"
        ));
        return;
    }

    if state.active_theme.screen_w == new_w && state.active_theme.screen_h == new_h {
        return;
    }

    // Resize the host window first so any subsequent render call sees the
    // new viewport.
    if let Err(e) = backend.set_window_size(new_w, new_h) {
        state
            .terminal
            .output_lines
            .push(format!("Resolution change failed: {e}"));
        return;
    }

    // Resize the shader compositor if present.
    if let Some(bridge) = shader_bridge.as_mut() {
        bridge.resize(new_w, new_h);
    }

    state.config.screen_width = new_w;
    state.config.screen_height = new_h;

    // Re-apply the current skin's layout at the new target size. This rebuilds
    // every skin-owned SDI object (taskbar, dashboard tiles, etc.) for the new
    // canvas. We clone the current skin name because `Skin::swap_scaled`
    // consumes the new skin, and we want to reuse the already-resolved skin
    // rather than re-reading it from disk.
    let current_skin_name = state.skin.manifest.name.clone();
    match resolve_skin(&current_skin_name) {
        Ok(fresh_skin) => {
            let swapped = Skin::swap_scaled(&state.skin, fresh_skin, sdi, new_w, new_h);
            state.active_theme = ActiveTheme::from_skin(&swapped.theme)
                .with_screen_size(new_w, new_h)
                .with_features(&swapped.features);
            state.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
            state.wm.set_theme(swapped.theme.build_wm_theme());
            state.skin = swapped;
        },
        Err(e) => {
            // Reloading the skin failed (shouldn't happen for a built-in),
            // but we still need to keep the theme dimensions consistent with
            // the resized window so rendering doesn't draw to stale coords.
            // We also refresh the WM theme from the existing `state.skin` so
            // `state.wm` stays consistent with the skin that's actually
            // still active (rather than holding whatever theme it was
            // constructed with before this fallback path ran).
            state
                .terminal
                .output_lines
                .push(format!("Warning: skin reload failed: {e}"));
            state.active_theme = state
                .active_theme
                .clone()
                .with_screen_size(new_w, new_h)
                .with_features(&state.skin.features);
            state.wm.set_theme(state.skin.theme.build_wm_theme());
        },
    }

    // Rebuild dashboard + bars for the new layout grid.
    let dash_config = DashboardConfig::from_features(&state.skin.features, &state.active_theme);
    let apps = discover_apps(vfs, "/apps", Some("OASISOS")).unwrap_or_default();
    state.ui.dashboard = DashboardState::new(dash_config, apps);
    state.ui.bottom_bar.total_pages = state.ui.dashboard.page_count();
    state.ui.bottom_bar.current_page = 0;
    state.ui.start_menu = StartMenuState::new_with_theme(
        StartMenuState::default_items(&state.active_theme),
        &state.active_theme,
    );

    state.wm.set_screen_size(new_w, new_h);
    // `set_screen_size` updates the viewport bounds but leaves open windows
    // at their original coordinates. On a downward resize a window near the
    // old right/bottom edge can end up fully off-screen and unreachable.
    // `move_window(id, 0, 0, sdi)` is a no-op delta but runs the positions
    // through `clamp_position`, which pulls each titlebar back on-screen.
    let window_ids: Vec<String> = state
        .wm
        .windows()
        .iter()
        .map(|w| w.id.as_str().to_string())
        .collect();
    for id in window_ids {
        let _ = state.wm.move_window(&id, 0, 0, sdi);
    }
    state.ui.mouse_cursor = CursorState::new(new_w, new_h);
    state.ui.mouse_cursor.scale = state.active_theme.cursor_scale;

    // Regenerate the wallpaper texture at the new size. The old wallpaper
    // object survived `swap_scaled` (it isn't a skin-layout object). We load
    // the new texture first, and only destroy the old one once the SDI
    // wallpaper has been re-pointed at the fresh id — otherwise a
    // `load_texture` failure would leave the wallpaper object holding an
    // already-destroyed id and the next render would dereference it.
    let old_wallpaper_tex = sdi.get("wallpaper").ok().and_then(|o| o.texture);
    let wp_data = wallpaper::generate_from_config(new_w, new_h, &state.active_theme);
    match backend.load_texture(new_w, new_h, &wp_data) {
        Ok(new_tex) => {
            // Recreate the SDI object so size + texture are consistent. The
            // `wallpaper.style == "none"` branch is handled downstream by
            // the render pipeline toggling visibility.
            terminal_sdi::setup_wallpaper(sdi, new_tex, new_w, new_h);
            // Preserve the "hidden under shader wallpaper" behaviour from
            // boot so shader-driven skins don't double-paint the background.
            if oasis_core::vector_overlay::get_shader_layer(&state.active_theme).is_some()
                && let Ok(obj) = sdi.get_mut("wallpaper")
            {
                obj.visible = false;
            }
            // Now that the wallpaper points at `new_tex`, drop the old id.
            if let Some(tex) = old_wallpaper_tex {
                let _ = backend.destroy_texture(tex);
            }
        },
        Err(e) => {
            state
                .terminal
                .output_lines
                .push(format!("Warning: wallpaper reload failed: {e}"));
        },
    }

    state
        .terminal
        .output_lines
        .push(format!("Resolution: {new_w}x{new_h}"));
}

/// Publish the current runtime state (skin, resolution, backend) to VFS so
/// the Settings app and any other UI can read it on demand. Called on
/// startup and after every apply.
pub fn publish_runtime_state(state: &AppState, backend_name: &str, vfs: &mut MemoryVfs) {
    // `MemoryVfs::write` requires the parent directory to exist, so we
    // proactively create every directory both the state publisher and the
    // IPC request poller will touch. Without `/system/ipc`, the shell's
    // pending-VFS-request block silently fails to write skin / resolution
    // change requests, and `poll_settings_ipc` never sees them.
    let _ = vfs.mkdir("/system");
    let _ = vfs.mkdir("/system/state");
    let _ = vfs.mkdir("/system/ipc");
    let _ = vfs.write(
        oasis_app_settings::SKIN_STATE_PATH,
        state.skin.manifest.name.as_bytes(),
    );
    let res = format!(
        "{}x{}",
        state.active_theme.screen_w, state.active_theme.screen_h
    );
    let _ = vfs.write(oasis_app_settings::RESOLUTION_STATE_PATH, res.as_bytes());
    let _ = vfs.write(
        oasis_app_settings::BACKEND_STATE_PATH,
        backend_name.as_bytes(),
    );
}

/// Poll the Settings IPC paths once per frame and dispatch any pending
/// change. Clears each request immediately after reading so the shell
/// doesn't reapply on every subsequent frame.
pub fn poll_settings_ipc(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut SdlBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &mut MemoryVfs,
    backend_name: &str,
) {
    let mut skin_request: Option<String> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).trim().to_string();
        // Always clear the request so we don't loop on malformed input.
        let _ = vfs.write(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH, b"");
        if !req.is_empty() && req != state.skin.manifest.name {
            skin_request = Some(req);
        }
    }

    let mut resolution_request: Option<(u32, u32)> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).trim().to_string();
        let _ = vfs.write(oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH, b"");
        if let Some((w, h)) = oasis_app_settings::parse_resolution(&req) {
            resolution_request = Some((w, h));
        } else if !req.is_empty() {
            state
                .terminal
                .output_lines
                .push(format!("Ignoring malformed resolution request: {req}"));
        }
    }

    let mut changed = false;
    if let Some(name) = skin_request {
        apply_skin_swap(&name, state, sdi, vfs);
        changed = true;
    }
    if let Some((w, h)) = resolution_request {
        apply_resolution_change(w, h, state, sdi, backend, shader_bridge, vfs);
        changed = true;
    }

    if changed {
        publish_runtime_state(state, backend_name, vfs);
    }
}

/// Format a remote command result as a response string, applying side effects
/// (browser sandbox, skin swap) as needed.
fn format_remote_response(
    result: oasis_core::error::Result<CommandOutput>,
    browser: &mut Option<oasis_core::browser::BrowserWidget>,
    skin: &mut Skin,
    active_theme: &mut ActiveTheme,
    browser_config: &mut BrowserConfig,
    wm: &mut oasis_core::wm::manager::WindowManager,
    sdi: &mut SdiRegistry,
) -> String {
    match result {
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
        Ok(CommandOutput::Signal(
            CommandSignal::ListenToggle { .. }
            | CommandSignal::RemoteConnect { .. }
            | CommandSignal::FtpToggle { .. },
        )) => "Not available via remote.".to_string(),
        Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox { enable })) => {
            if let Some(bw) = browser {
                bw.config.features.sandbox_only = enable;
            }
            let st = if enable {
                "on (VFS only)"
            } else {
                "off (HTTP enabled)"
            };
            format!("Browser sandbox: {st}")
        },
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap { name })) => match resolve_skin(&name) {
            Ok(new_skin) => {
                let sw = active_theme.screen_w;
                let sh = active_theme.screen_h;
                let swapped = Skin::swap_scaled(skin, new_skin, sdi, sw, sh);
                *active_theme = ActiveTheme::from_skin(&swapped.theme)
                    .with_screen_size(sw, sh)
                    .with_features(&swapped.features);
                *browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                wm.set_theme(swapped.theme.build_wm_theme());
                let msg = format!("Switched to skin: {}", swapped.manifest.name);
                *skin = swapped;
                msg
            },
            Err(e) => format!("Skin error: {e}"),
        },
        Ok(CommandOutput::Multi(outputs)) => {
            let mut parts = Vec::new();
            for output in outputs {
                let resp = format_remote_response(
                    Ok(output),
                    browser,
                    skin,
                    active_theme,
                    browser_config,
                    wm,
                    sdi,
                );
                if !resp.is_empty() {
                    parts.push(resp);
                }
            }
            parts.join("\n")
        },
        Err(e) => format!("error: {e}"),
    }
}

/// Poll the remote listener for incoming commands and execute them.
pub fn poll_remote_listener(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    // Destructure to allow field-level borrow splitting.
    let AppState {
        ref mut net,
        ref mut terminal,
        ref mut content,
        ref platform,
        ref mut skin,
        ref mut active_theme,
        ref mut browser_config,
        ref mut wm,
        ..
    } = *state;

    let NetworkLayer {
        ref mut listener,
        ref mut backend,
        ref tls_provider,
        ..
    } = *net;

    let TerminalLayer {
        ref mut cmd_reg,
        ref mut cwd,
        ..
    } = *terminal;

    let ContentLayer {
        ref mut browser, ..
    } = *content;

    let Some(l) = listener else { return };

    let remote_cmds = l.poll(backend);
    for (cmd_line, conn_idx) in remote_cmds {
        log::info!("Remote command from #{conn_idx}: {cmd_line}");
        let mut env = Environment {
            cwd: cwd.clone(),
            vfs,
            power: Some(platform),
            time: Some(platform),
            usb: Some(platform),
            network: None,
            tls: Some(tls_provider),
            stdin: None,
            stderr: String::new(),
        };
        let result = cmd_reg.execute(&cmd_line, &mut env);
        *cwd = env.cwd;
        let response =
            format_remote_response(result, browser, skin, active_theme, browser_config, wm, sdi);
        let _ = l.send_response(conn_idx, &response);
    }
}

/// Poll the FTP server for incoming connections and commands.
pub fn poll_ftp_server(state: &mut AppState, vfs: &mut MemoryVfs) {
    let NetworkLayer {
        ref mut ftp_server,
        ref mut backend,
        ..
    } = state.net;

    let Some(server) = ftp_server else { return };

    if let Err(e) = server.poll(backend, vfs) {
        log::warn!("FTP server poll error: {e}");
    }
}

/// Poll the remote client for received data.
pub fn poll_remote_client(state: &mut AppState) {
    let Some(ref mut client) = state.net.remote_client else {
        return;
    };
    let lines = client.poll();
    for line in lines {
        state.terminal.output_lines.push(format!("[remote] {line}"));
    }
    if !client.is_connected() {
        state
            .terminal
            .output_lines
            .push("[remote] Disconnected.".to_string());
        state.net.remote_client = None;
    }
    trim_output(&mut state.terminal.output_lines);
}

/// Truncate output lines to `MAX_OUTPUT_LINES`.
pub fn trim_output(output_lines: &mut Vec<String>) {
    while output_lines.len() > terminal_sdi::MAX_OUTPUT_LINES {
        output_lines.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::terminal::{CommandOutput, CommandSignal};

    // -- trim_output --

    #[test]
    fn trim_output_noop_under_limit() {
        let mut lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "line0");
    }

    #[test]
    fn trim_output_noop_at_limit() {
        let mut lines: Vec<String> = (0..terminal_sdi::MAX_OUTPUT_LINES)
            .map(|i| format!("line{i}"))
            .collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
    }

    #[test]
    fn trim_output_trims_excess() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 50;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        // Oldest lines should have been removed.
        assert_eq!(lines[0], "line50");
        assert_eq!(lines.last().unwrap(), &format!("line{}", count - 1));
    }

    #[test]
    fn trim_output_empty() {
        let mut lines: Vec<String> = vec![];
        trim_output(&mut lines);
        assert!(lines.is_empty());
    }

    // -- process_command_output (using a real AppState) --

    fn make_test_state() -> AppState {
        use oasis_audio::RadioManager;
        use oasis_backend_sdl::SdlAudioBackend;
        use oasis_core::active_theme::ActiveTheme;
        use oasis_core::backend::Color;
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

        AppState {
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
            mode: crate::app_state::Mode::Dashboard,
            bg_color: Color::rgb(0, 0, 0),
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
        }
    }

    #[test]
    fn process_text_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Text("hello\nworld".to_string())),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines, vec!["hello", "world"]);
    }

    #[test]
    fn process_table_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Table {
                headers: vec!["Name".into(), "Size".into()],
                rows: vec![vec!["foo.txt".into(), "42".into()]],
            }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 2);
        assert_eq!(state.terminal.output_lines[0], "Name | Size");
        assert_eq!(state.terminal.output_lines[1], "foo.txt | 42");
    }

    #[test]
    fn process_clear_output() {
        let mut state = make_test_state();
        state.terminal.output_lines.push("existing".to_string());
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_none_output() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::None), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_skin_swap_returns_name() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::SkinSwap {
                name: "tactical".to_string(),
            })),
            &mut state,
        );
        assert_eq!(result, Some("tactical".to_string()));
    }

    #[test]
    fn process_error_output() {
        let mut state = make_test_state();
        let err = oasis_core::error::OasisError::Command("test error".into());
        let result = process_command_output(Err(err), &mut state);
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        assert!(state.terminal.output_lines[0].starts_with("error:"));
    }

    #[test]
    fn process_multi_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("first".to_string()),
                CommandOutput::Text("second".to_string()),
            ])),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines, vec!["first", "second"]);
    }

    #[test]
    fn process_multi_with_skin_swap() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("before".to_string()),
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "corrupted".to_string(),
                }),
            ])),
            &mut state,
        );
        assert_eq!(result, Some("corrupted".to_string()));
        assert_eq!(state.terminal.output_lines, vec!["before"]);
    }

    #[test]
    fn process_browser_sandbox_on() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox {
                enable: true,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        assert!(state.terminal.output_lines[0].contains("sandbox"));
        assert!(state.terminal.output_lines[0].contains("on"));
    }

    #[test]
    fn process_browser_sandbox_off() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox {
                enable: false,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("off"));
    }

    #[test]
    fn process_listen_stop_no_listener() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::ListenToggle {
                port: 0,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines[0], "No listener running.");
    }

    #[test]
    fn process_ftp_stop_no_server() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::FtpToggle {
                port: 0,
                password: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines[0], "No FTP server running.");
    }

    #[test]
    fn process_remote_connect_already_connected() {
        let mut state = make_test_state();
        // Simulate an existing client.
        state.net.remote_client = Some(oasis_core::net::RemoteClient::new());
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::RemoteConnect {
                address: "127.0.0.1".into(),
                port: 9999,
                psk: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(
            state.terminal.output_lines[0],
            "Already connected. Disconnect first."
        );
    }

    // -- Additional command handler tests --

    #[test]
    fn process_listen_already_running() {
        let mut state = make_test_state();
        // Start a listener first.
        let cfg = oasis_core::net::ListenerConfig {
            port: 19999,
            psk: String::new(),
            max_connections: 1,
            ..oasis_core::net::ListenerConfig::default()
        };
        state.net.listener = Some(oasis_core::net::RemoteListener::new(cfg));
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::ListenToggle {
                port: 8080,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_ftp_already_running() {
        let mut state = make_test_state();
        state.net.ftp_server = Some(oasis_core::transfer::FtpServer::new(19000));
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::FtpToggle {
                port: 21,
                password: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_multi_empty_list() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::Multi(vec![])), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_multi_preserves_order() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("alpha".to_string()),
                CommandOutput::Text("beta".to_string()),
                CommandOutput::Text("gamma".to_string()),
            ])),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 3);
        assert_eq!(state.terminal.output_lines[0], "alpha");
        assert_eq!(state.terminal.output_lines[1], "beta");
        assert_eq!(state.terminal.output_lines[2], "gamma");
    }

    #[test]
    fn process_multi_last_skin_swap_wins() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "first".to_string(),
                }),
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "second".to_string(),
                }),
            ])),
            &mut state,
        );
        assert_eq!(result, Some("second".to_string()));
    }

    #[test]
    fn process_table_empty_rows() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Table {
                headers: vec!["Col1".into(), "Col2".into()],
                rows: vec![],
            }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        assert_eq!(state.terminal.output_lines[0], "Col1 | Col2");
    }

    #[test]
    fn process_text_multiline() {
        let mut state = make_test_state();
        let text = "line1\nline2\nline3\nline4";
        let result = process_command_output(Ok(CommandOutput::Text(text.to_string())), &mut state);
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 4);
    }

    #[test]
    fn process_clear_empties_all() {
        let mut state = make_test_state();
        state.terminal.output_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn trim_output_single_excess() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 1;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        assert_eq!(lines[0], "line1");
    }

    #[test]
    fn process_error_format() {
        let mut state = make_test_state();
        let err = oasis_core::error::OasisError::Vfs("file not found".into());
        process_command_output(Err(err), &mut state);
        assert!(state.terminal.output_lines[0].contains("error:"));
        assert!(state.terminal.output_lines[0].contains("file not found"));
    }
}
