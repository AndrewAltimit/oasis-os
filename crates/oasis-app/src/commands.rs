use oasis_core::active_theme::ActiveTheme;
use oasis_core::browser::BrowserConfig;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::net::{ListenerConfig, RemoteClient, RemoteListener};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin};
use oasis_core::startmenu::StartMenuState;
use oasis_core::terminal::{CommandOutput, Environment};
use oasis_core::transfer::FtpServer;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::AppState;
use oasis_core::terminal_sdi;

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
        Ok(CommandOutput::FtpToggle { port }) => {
            if port == 0 {
                if let Some(ref mut f) = state.ftp_server {
                    f.stop();
                    state.ftp_server = None;
                    state.output_lines.push("FTP server stopped.".to_string());
                } else {
                    state
                        .output_lines
                        .push("No FTP server running.".to_string());
                }
            } else if state.ftp_server.is_some() {
                state
                    .output_lines
                    .push("FTP server already running. Use 'ftp stop' first.".to_string());
            } else {
                let mut server = FtpServer::new(port);
                match server.start(&mut state.net_backend) {
                    Ok(()) => {
                        state
                            .output_lines
                            .push(format!("FTP server listening on port {port}."));
                        state.ftp_server = Some(server);
                    },
                    Err(e) => {
                        state.output_lines.push(format!("FTP server error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::SkinSwap { name }) => {
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
        Ok(CommandOutput::ListenToggle { .. })
        | Ok(CommandOutput::RemoteConnect { .. })
        | Ok(CommandOutput::FtpToggle { .. }) => "Not available via remote.".to_string(),
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
    let AppState {
        ref mut ftp_server,
        ref mut net_backend,
        ..
    } = *state;

    let Some(server) = ftp_server else { return };

    if let Err(e) = server.poll(net_backend, vfs) {
        log::warn!("FTP server poll error: {e}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::terminal::CommandOutput;

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
            mode: crate::app_state::Mode::Dashboard,
            bg_color: Color::rgb(0, 0, 0),
            active_transition: None,
            frame_counter: 0,
            radio_manager: RadioManager::new(),
            radio_source: None,
            audio_backend: SdlAudioBackend::new(),
            terminal_scroll_offset: 0,
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
        assert_eq!(state.output_lines, vec!["hello", "world"]);
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
        assert_eq!(state.output_lines.len(), 2);
        assert_eq!(state.output_lines[0], "Name | Size");
        assert_eq!(state.output_lines[1], "foo.txt | 42");
    }

    #[test]
    fn process_clear_output() {
        let mut state = make_test_state();
        state.output_lines.push("existing".to_string());
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines.is_empty());
    }

    #[test]
    fn process_none_output() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::None), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines.is_empty());
    }

    #[test]
    fn process_skin_swap_returns_name() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::SkinSwap {
                name: "tactical".to_string(),
            }),
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
        assert_eq!(state.output_lines.len(), 1);
        assert!(state.output_lines[0].starts_with("error:"));
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
        assert_eq!(state.output_lines, vec!["first", "second"]);
    }

    #[test]
    fn process_multi_with_skin_swap() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("before".to_string()),
                CommandOutput::SkinSwap {
                    name: "corrupted".to_string(),
                },
            ])),
            &mut state,
        );
        assert_eq!(result, Some("corrupted".to_string()));
        assert_eq!(state.output_lines, vec!["before"]);
    }

    #[test]
    fn process_browser_sandbox_on() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::BrowserSandbox { enable: true }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.output_lines.len(), 1);
        assert!(state.output_lines[0].contains("sandbox"));
        assert!(state.output_lines[0].contains("on"));
    }

    #[test]
    fn process_browser_sandbox_off() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::BrowserSandbox { enable: false }),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.output_lines[0].contains("off"));
    }

    #[test]
    fn process_listen_stop_no_listener() {
        let mut state = make_test_state();
        let result =
            process_command_output(Ok(CommandOutput::ListenToggle { port: 0 }), &mut state);
        assert!(result.is_none());
        assert_eq!(state.output_lines[0], "No listener running.");
    }

    #[test]
    fn process_ftp_stop_no_server() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::FtpToggle { port: 0 }), &mut state);
        assert!(result.is_none());
        assert_eq!(state.output_lines[0], "No FTP server running.");
    }

    #[test]
    fn process_remote_connect_already_connected() {
        let mut state = make_test_state();
        // Simulate an existing client.
        state.remote_client = Some(oasis_core::net::RemoteClient::new());
        let result = process_command_output(
            Ok(CommandOutput::RemoteConnect {
                address: "127.0.0.1".into(),
                port: 9999,
                psk: None,
            }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(
            state.output_lines[0],
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
        state.listener = Some(oasis_core::net::RemoteListener::new(cfg));
        let result =
            process_command_output(Ok(CommandOutput::ListenToggle { port: 8080 }), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_ftp_already_running() {
        let mut state = make_test_state();
        state.ftp_server = Some(oasis_core::transfer::FtpServer::new(19000));
        let result = process_command_output(Ok(CommandOutput::FtpToggle { port: 21 }), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_multi_empty_list() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::Multi(vec![])), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines.is_empty());
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
        assert_eq!(state.output_lines.len(), 3);
        assert_eq!(state.output_lines[0], "alpha");
        assert_eq!(state.output_lines[1], "beta");
        assert_eq!(state.output_lines[2], "gamma");
    }

    #[test]
    fn process_multi_last_skin_swap_wins() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::SkinSwap {
                    name: "first".to_string(),
                },
                CommandOutput::SkinSwap {
                    name: "second".to_string(),
                },
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
        assert_eq!(state.output_lines.len(), 1);
        assert_eq!(state.output_lines[0], "Col1 | Col2");
    }

    #[test]
    fn process_text_multiline() {
        let mut state = make_test_state();
        let text = "line1\nline2\nline3\nline4";
        let result = process_command_output(Ok(CommandOutput::Text(text.to_string())), &mut state);
        assert!(result.is_none());
        assert_eq!(state.output_lines.len(), 4);
    }

    #[test]
    fn process_clear_empties_all() {
        let mut state = make_test_state();
        state.output_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.output_lines.is_empty());
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
        assert!(state.output_lines[0].contains("error:"));
        assert!(state.output_lines[0].contains("file not found"));
    }
}
