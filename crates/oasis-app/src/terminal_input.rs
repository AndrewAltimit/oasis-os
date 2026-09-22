//! Terminal line editing and command execution for the desktop host.
//!
//! Input for the terminal (fullscreen `Mode::Terminal` or the focused
//! windowed terminal) goes through the terminal layer's
//! [`ShellSession`](oasis_core::terminal::ShellSession): raw key shortcuts
//! via [`handle_key`], text / Backspace / Tab / d-pad / Confirm / Square
//! via [`handle_event`]. Keyboard and gamepad share the event path, so the
//! PSP-style buttons keep working (Up/Down = history, Left/Right = cursor,
//! Square = backspace, Cross = run).

use oasis_core::input::{InputEvent, Key, Modifiers};
use oasis_core::sdi::SdiRegistry;
use oasis_core::terminal::{Environment, SessionEvent};
use oasis_core::terminal_sdi;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, Mode};
use crate::commands;

/// Whether keyboard input currently belongs to the terminal.
pub fn focused(state: &AppState) -> bool {
    match state.mode {
        Mode::Terminal => true,
        Mode::Desktop => state.wm.active_window() == Some("terminal"),
        _ => false,
    }
}

/// Offer a raw key press to the line editor. Returns `true` when it was a
/// line-editing shortcut (applied; the caller must drop the key's twin).
pub fn handle_key(
    key: Key,
    mods: Modifiers,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> bool {
    let term = &mut state.terminal;
    let Some(event) = term
        .session
        .handle_key(key, mods, &term.cmd_reg, &term.cwd, vfs)
    else {
        return false;
    };
    apply_session_event(event, state, sdi, vfs);
    true
}

/// Offer a text / Backspace / Tab / button event to the line editor.
/// Returns `true` when the session handled it.
pub fn handle_event(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> bool {
    let term = &mut state.terminal;
    let Some(event) = term
        .session
        .handle_event(event, &term.cmd_reg, &term.cwd, vfs)
    else {
        return false;
    };
    apply_session_event(event, state, sdi, vfs);
    true
}

fn apply_session_event(
    event: SessionEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) {
    match event {
        SessionEvent::Redraw => {},
        SessionEvent::Submit(line) => run_line(&line, state, sdi, vfs),
        SessionEvent::Interrupted(line) => {
            state.terminal.output_lines.push(format!("> {line}^C"));
            state.terminal.scroll_offset = 0;
            commands::trim_output(&mut state.terminal.output_lines);
        },
        SessionEvent::ClearScreen => {
            state.terminal.output_lines.clear();
            state.terminal.scroll_offset = 0;
        },
        SessionEvent::Candidates(candidates) => {
            let term = &mut state.terminal;
            term.output_lines
                .push(format!("> {}", term.session.buffer()));
            term.output_lines
                .extend(candidate_rows(&candidates, CANDIDATE_ROW_WIDTH));
            term.scroll_offset = 0;
            commands::trim_output(&mut term.output_lines);
        },
    }
}

/// Approximate character budget for one row of completion candidates.
const CANDIDATE_ROW_WIDTH: usize = 60;

/// Lay out completion candidates in rows of at most `width` characters.
fn candidate_rows(candidates: &[String], width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for c in candidates {
        if !row.is_empty() && row.len() + 2 + c.len() > width {
            rows.push(std::mem::take(&mut row));
        }
        if !row.is_empty() {
            row.push_str("  ");
        }
        row.push_str(c);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn make_env<'a>(state: &'a AppState, vfs: &'a mut MemoryVfs) -> Environment<'a> {
    Environment {
        cwd: state.terminal.cwd.clone(),
        vfs,
        power: Some(&state.platform),
        time: Some(&state.platform),
        usb: Some(&state.platform),
        network: None,
        tls: Some(&state.net.tls_provider),
        stdin: None,
        stderr: String::new(),
    }
}

/// Echo and execute one submitted terminal line, then persist history.
pub fn run_line(line: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    state.terminal.scroll_offset = 0;
    if !line.trim().is_empty() {
        state.terminal.output_lines.push(format!("> {line}"));
        let (result, cwd) = {
            let mut env = make_env(state, vfs);
            let result = state
                .terminal
                .cmd_reg
                .execute(line, &mut env)
                .map(|out| terminal_sdi::resolve_sdi_inspect(out, sdi));
            (result, env.cwd)
        };
        state.terminal.cwd = cwd;
        let pending_skin_swap = commands::process_command_output(result, state);
        if let Some(name) = pending_skin_swap {
            commands::apply_skin_swap(&name, state, sdi, vfs);
        }
        let term = &state.terminal;
        if let Err(e) = term.session.save_history(&term.cmd_reg, vfs) {
            log::warn!("terminal history not saved: {e}");
        }
    }
    commands::trim_output(&mut state.terminal.output_lines);
}

/// Run the next queued background job (`cmd &`), if any. Called once per
/// frame; at most one job runs per call.
pub fn poll_jobs(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    if state.terminal.cmd_reg.pending_jobs() == 0 {
        return;
    }
    let (output, cwd) = {
        let mut env = make_env(state, vfs);
        let output = state.terminal.cmd_reg.poll_jobs(&mut env);
        (output, env.cwd)
    };
    state.terminal.cwd = cwd;
    let Some(output) = output else {
        return;
    };
    let output = terminal_sdi::resolve_sdi_inspect(output, sdi);
    if let Some(name) = commands::process_command_output(Ok(output), state) {
        commands::apply_skin_swap(&name, state, sdi, vfs);
    }
    commands::trim_output(&mut state.terminal.output_lines);
    state.terminal.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_rows_wrap() {
        let c: Vec<String> = ["alpha", "beta", "gamma", "delta"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(candidate_rows(&c, 12), ["alpha  beta", "gamma  delta"]);
        assert_eq!(candidate_rows(&c, 100), ["alpha  beta  gamma  delta"]);
        assert!(candidate_rows(&[], 10).is_empty());
    }
}
