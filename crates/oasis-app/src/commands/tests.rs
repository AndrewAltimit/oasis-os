//! Tests of the command-output paths (local terminal and remote callers).

use super::*;
use oasis_core::terminal::{CommandOutput, CommandSignal};

// -- refuse_job_server_toggles --

#[test]
fn background_jobs_cannot_toggle_servers() {
    let refused =
        |out: &CommandOutput| matches!(out, CommandOutput::Text(t) if t == JOB_TOGGLE_REFUSED);
    for signal in [
        CommandOutput::listen_toggle(2323),
        CommandOutput::remote_connect("10.0.0.1".into(), 23, None),
    ] {
        assert!(refused(&refuse_job_server_toggles(signal)));
    }
    // Nested in a chain / with a status, and other output untouched.
    let out = refuse_job_server_toggles(CommandOutput::Multi(vec![
        CommandOutput::Text("before".into()),
        CommandOutput::status(0, CommandOutput::listen_toggle(0)),
        CommandOutput::skin_swap("xp".into()),
    ]));
    let CommandOutput::Multi(parts) = out else {
        panic!("multi kept");
    };
    assert!(matches!(&parts[0], CommandOutput::Text(t) if t == "before"));
    assert!(matches!(&parts[1], CommandOutput::Status { output, .. } if refused(output)));
    assert!(matches!(
        &parts[2],
        CommandOutput::Signal(CommandSignal::SkinSwap { .. })
    ));
}

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
    AppState::new_for_test()
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
    state.terminal.print("existing".to_string());
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
fn process_skin_swap_resolves_the_skin() {
    let mut state = make_test_state();
    let result = process_command_output(
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap {
            name: "modern".to_string(),
        })),
        &mut state,
    );
    assert_eq!(result.map(|s| s.manifest.name).as_deref(), Some("modern"));
}

#[test]
fn process_error_output() {
    let mut state = make_test_state();
    let err = oasis_core::error::OasisError::Command("test error".into());
    let result = process_command_output(Err(err), &mut state);
    assert!(result.is_none());
    assert_eq!(state.terminal.output_lines.len(), 1);
    // Errors are wrapped in a red SGR sequence for the terminal UI.
    let line = &state.terminal.output_lines[0];
    assert!(line.starts_with("\u{1b}[31m"));
    assert!(oasis_core::ansi::strip_sgr(line).starts_with("error:"));
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
    assert_eq!(
        result.map(|s| s.manifest.name).as_deref(),
        Some("corrupted")
    );
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
            CommandOutput::skin_swap("xp".into()),
            CommandOutput::skin_swap("modern".into()),
        ])),
        &mut state,
    );
    assert_eq!(result.map(|s| s.manifest.name).as_deref(), Some("modern"));
}

/// `skin xp; skin @variant:dark` derives the dark variant of xp (the skin
/// the line swapped to), not of the skin active before the line ran.
#[test]
fn chained_variant_swap_derives_from_the_earlier_swap() {
    let mut state = make_test_state();
    assert_ne!(state.skin.manifest.name, "xp");
    let result = process_command_output(
        Ok(CommandOutput::Multi(vec![
            CommandOutput::skin_swap("xp".into()),
            CommandOutput::skin_swap("@variant:dark".into()),
        ])),
        &mut state,
    );
    assert_eq!(result.map(|s| s.manifest.name).as_deref(), Some("xp-dark"));
}

/// A bad name is reported where it occurs and leaves the earlier swap.
#[test]
fn unknown_skin_in_a_chain_is_reported_inline() {
    let mut state = make_test_state();
    let result = process_command_output(
        Ok(CommandOutput::Multi(vec![
            CommandOutput::skin_swap("xp".into()),
            CommandOutput::skin_swap("no-such-skin".into()),
            CommandOutput::Text("after".into()),
        ])),
        &mut state,
    );
    assert_eq!(result.map(|s| s.manifest.name).as_deref(), Some("xp"));
    let lines = &state.terminal.output_lines;
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("Skin error:"), "{lines:?}");
    assert_eq!(lines[1], "after");
}

/// The remote path resolves chains the same way.
#[test]
fn remote_chained_variant_swap_derives_from_the_earlier_swap() {
    let state = make_test_state();
    let mut pending = None;
    let mut browser = None;
    let resp = format_remote_response(
        Ok(CommandOutput::Multi(vec![
            CommandOutput::skin_swap("xp".into()),
            CommandOutput::skin_swap("@variant:dark".into()),
        ])),
        &mut browser,
        &state.skin,
        &SdiRegistry::new(),
        &mut pending,
    );
    assert!(resp.contains("xp-dark"), "{resp}");
    assert_eq!(pending.map(|s| s.manifest.name).as_deref(), Some("xp-dark"));
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
