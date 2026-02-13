use oasis_core::active_theme::ActiveTheme;
use oasis_core::browser::BrowserConfig;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::net::{ListenerConfig, RemoteClient, RemoteListener};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin};
use oasis_core::startmenu::StartMenuState;
use oasis_core::terminal::{CommandOutput, Environment};
use oasis_core::vfs::MemoryVfs;

use crate::app_state::AppState;
use crate::terminal_sdi;

/// Process a local terminal command result. Returns a pending skin swap name
/// if the command was `SkinSwap`.
pub fn process_command_output(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
) -> Option<String> {
    match result {
        Ok(CommandOutput::Text(text)) => {
            for l in text.lines() {
                state.output_lines.push(l.to_string());
            }
        },
        Ok(CommandOutput::Table { headers, rows }) => {
            state.output_lines.push(headers.join(" | "));
            for row in &rows {
                state.output_lines.push(row.join(" | "));
            }
        },
        Ok(CommandOutput::Clear) => state.output_lines.clear(),
        Ok(CommandOutput::None) => {},
        Ok(CommandOutput::ListenToggle { port }) => {
            if port == 0 {
                if let Some(ref mut l) = state.listener {
                    l.stop();
                    state.listener = None;
                    state
                        .output_lines
                        .push("Remote listener stopped.".to_string());
                } else {
                    state.output_lines.push("No listener running.".to_string());
                }
            } else if state.listener.is_some() {
                state
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
                match l.start(&mut state.net_backend) {
                    Ok(()) => {
                        state
                            .output_lines
                            .push(format!("Listening on port {port}."));
                        state.listener = Some(l);
                    },
                    Err(e) => {
                        state.output_lines.push(format!("Listen error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::RemoteConnect { address, port, psk }) => {
            if state.remote_client.is_some() {
                state
                    .output_lines
                    .push("Already connected. Disconnect first.".to_string());
            } else {
                let mut client = RemoteClient::new();
                match client.connect(&mut state.net_backend, &address, port, psk.as_deref()) {
                    Ok(()) => {
                        state
                            .output_lines
                            .push(format!("Connected to {address}:{port}."));
                        state.remote_client = Some(client);
                    },
                    Err(e) => {
                        state.output_lines.push(format!("Connect error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::BrowserSandbox { enable }) => {
            if let Some(ref mut bw) = state.browser {
                bw.config.features.sandbox_only = enable;
            }
            let st = if enable {
                "on (VFS only)"
            } else {
                "off (HTTP enabled)"
            };
            state.output_lines.push(format!("Browser sandbox: {st}"));
        },
        Ok(CommandOutput::SkinSwap { name }) => {
            return Some(name);
        },
        Err(e) => {
            state.output_lines.push(format!("error: {e}"));
        },
    }
    None
}

/// Apply a skin swap after the Environment borrow has been dropped.
pub fn apply_skin_swap(name: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    match resolve_skin(name) {
        Ok(new_skin) => {
            let swapped = Skin::swap(&state.skin, new_skin, sdi);
            state.active_theme = ActiveTheme::from_skin(&swapped.theme);
            state.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
            state.wm.set_theme(swapped.theme.build_wm_theme());
            let dash_config =
                DashboardConfig::from_features(&swapped.features, &state.active_theme);
            let apps = discover_apps(vfs, "/apps", Some("OASISOS")).unwrap_or_default();
            state.dashboard = DashboardState::new(dash_config, apps);
            state.bottom_bar.total_pages = state.dashboard.page_count();
            state.bottom_bar.current_page = 0;
            state.start_menu = StartMenuState::new_with_theme(
                StartMenuState::default_items(),
                &state.active_theme,
            );
            state
                .output_lines
                .push(format!("Switched to skin: {}", swapped.manifest.name));
            state.skin = swapped;
        },
        Err(e) => {
            state.output_lines.push(format!("Skin error: {e}"));
        },
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
        Ok(CommandOutput::ListenToggle { .. }) | Ok(CommandOutput::RemoteConnect { .. }) => {
            "Not available via remote.".to_string()
        },
        Ok(CommandOutput::BrowserSandbox { enable }) => {
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
        Ok(CommandOutput::SkinSwap { name }) => match resolve_skin(&name) {
            Ok(new_skin) => {
                let swapped = Skin::swap(skin, new_skin, sdi);
                *active_theme = ActiveTheme::from_skin(&swapped.theme);
                *browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                wm.set_theme(swapped.theme.build_wm_theme());
                let msg = format!("Switched to skin: {}", swapped.manifest.name);
                *skin = swapped;
                msg
            },
            Err(e) => format!("Skin error: {e}"),
        },
        Err(e) => format!("error: {e}"),
    }
}

/// Poll the remote listener for incoming commands and execute them.
pub fn poll_remote_listener(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    // Destructure to allow field-level borrow splitting.
    let AppState {
        ref mut listener,
        ref mut net_backend,
        ref mut cmd_reg,
        ref mut cwd,
        ref platform,
        ref tls_provider,
        ref mut browser,
        ref mut skin,
        ref mut active_theme,
        ref mut browser_config,
        ref mut wm,
        ..
    } = *state;

    let Some(l) = listener else { return };

    let remote_cmds = l.poll(net_backend);
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
        };
        let result = cmd_reg.execute(&cmd_line, &mut env);
        *cwd = env.cwd;
        let response =
            format_remote_response(result, browser, skin, active_theme, browser_config, wm, sdi);
        let _ = l.send_response(conn_idx, &response);
    }
}

/// Poll the remote client for received data.
pub fn poll_remote_client(state: &mut AppState) {
    let Some(ref mut client) = state.remote_client else {
        return;
    };
    let lines = client.poll();
    for line in lines {
        state.output_lines.push(format!("[remote] {line}"));
    }
    if !client.is_connected() {
        state
            .output_lines
            .push("[remote] Disconnected.".to_string());
        state.remote_client = None;
    }
    trim_output(&mut state.output_lines);
}

/// Truncate output lines to `MAX_OUTPUT_LINES`.
pub fn trim_output(output_lines: &mut Vec<String>) {
    while output_lines.len() > terminal_sdi::MAX_OUTPUT_LINES {
        output_lines.remove(0);
    }
}
