//! Terminal line editing and command execution for the desktop host.
//!
//! Input for the terminal (fullscreen `Mode::Terminal` or the focused
//! windowed terminal) goes through the terminal layer's
//! [`ShellSession`](oasis_core::terminal::ShellSession): raw key shortcuts
//! via [`handle_key`], text / Backspace / Tab / d-pad / Confirm / Square
//! via [`handle_event`]. Keyboard and gamepad share the event path, so the
//! PSP-style buttons keep working (Up/Down = history, Left/Right = cursor,
//! Square = backspace, Cross = run).
//!
//! [`scroll`] is the one mouse-wheel path for both terminal views.

use oasis_app_core::render::WINDOWED_TOP_PAD;
use oasis_core::input::{InputEvent, Key, Modifiers};
use oasis_core::sdi::SdiRegistry;
use oasis_core::terminal::{Environment, SessionEvent};
use oasis_core::terminal_sdi;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, Mode};
use crate::commands;
use crate::launch::TERMINAL_WIN;

/// Lines one wheel notch scrolls.
const WHEEL_LINES: usize = 3;

/// Whether keyboard input currently belongs to the terminal.
pub fn focused(state: &AppState) -> bool {
    match state.mode {
        Mode::Terminal => true,
        Mode::Desktop => state.wm.active_window() == Some(TERMINAL_WIN),
        _ => false,
    }
}

/// Rows of text the windowed terminal shows: the line count
/// `oasis_app_core::render::draw_content_windowed` fits in the terminal
/// window's content area (it has no context header row). `None` while no
/// terminal window is open.
pub fn windowed_rows(state: &AppState) -> Option<usize> {
    let win = state.wm.get_window(TERMINAL_WIN)?;
    let (_, _, _, ch) = win.content_rect(state.wm.theme());
    let line_h = state.active_theme.terminal_line_height.max(12) as i32;
    let rows = (ch as i32 - WINDOWED_TOP_PAD as i32 - 16) / line_h;
    Some(rows.max(1) as usize)
}

/// `(lines, rows)` of the visible terminal view: the windowed terminal
/// scrolls its scrollback plus the prompt line through the window's rows,
/// the fullscreen one its scrollback through `visible_output_lines` (the
/// prompt has its own row there).
fn scroll_extent(state: &AppState) -> (usize, usize) {
    let len = state.terminal.output_lines.len();
    match windowed_rows(state).filter(|_| state.mode == Mode::Desktop) {
        Some(rows) => (len + 1, rows),
        None => (len, terminal_sdi::visible_output_lines(&state.active_theme)),
    }
}

/// Mouse wheel over the terminal (fullscreen, or the windowed one):
/// negative `delta` scrolls back into older lines, positive returns toward
/// the prompt. Clamped so the oldest line is the furthest you can go.
pub fn scroll(state: &mut AppState, delta: i32) {
    let (len, rows) = scroll_extent(state);
    let max_offset = len.saturating_sub(rows);
    let step = delta.unsigned_abs() as usize * WHEEL_LINES;
    let term = &mut state.terminal;
    term.scroll_offset = if delta < 0 {
        term.scroll_offset.saturating_add(step)
    } else {
        term.scroll_offset.saturating_sub(step)
    }
    .min(max_offset);
    term.dirty = true;
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
            state.terminal.print(format!("> {line}^C"));
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

/// The command environment of the shell's terminal: its working directory,
/// the VFS and the platform services (no raw network access).
pub(crate) fn make_env<'a>(state: &'a AppState, vfs: &'a mut MemoryVfs) -> Environment<'a> {
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

/// Run the boot scripts (`/home/.profile`, then `/etc/startup.sh`) through
/// the shell's command registry, printing their output to the terminal.
pub fn run_boot_scripts(state: &mut AppState, vfs: &mut MemoryVfs) {
    let lines = {
        let mut env = make_env(state, vfs);
        oasis_core::script::run_boot_scripts(&state.terminal.cmd_reg, &mut env)
    };
    for line in lines {
        state.terminal.print(line);
    }
}

/// Run the `cron` jobs due at `now` (time since shell start), printing
/// their output to the terminal.
pub fn poll_cron(
    state: &mut AppState,
    vfs: &mut MemoryVfs,
    cron: &mut oasis_core::script::CronScheduler,
    now: std::time::Duration,
) {
    // Jobs start in `/` and their `cd`s are dropped: a job firing must not
    // move the interactive shell's working directory.
    let lines = {
        let mut env = make_env(state, vfs);
        env.cwd = "/".to_string();
        cron.tick(now, &state.terminal.cmd_reg, &mut env)
    };
    if lines.is_empty() {
        return;
    }
    for line in lines {
        state.terminal.print(line);
    }
    state.terminal.dirty = true;
    commands::trim_output(&mut state.terminal.output_lines);
}

/// Echo and execute one submitted terminal line, then persist history.
pub fn run_line(line: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    state.terminal.scroll_offset = 0;
    if !line.trim().is_empty() {
        state.terminal.print(format!("> {line}"));
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
        commands::apply_command_output(result, state, sdi, vfs);
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
    // A job may have been queued over a remote channel: never let it
    // toggle a server (see `commands::refuse_job_server_toggles`).
    let output = commands::refuse_job_server_toggles(output);
    let output = terminal_sdi::resolve_sdi_inspect(output, sdi);
    commands::apply_command_output(Ok(output), state, sdi, vfs);
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
