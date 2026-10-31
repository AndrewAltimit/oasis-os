//! Host side of terminal command results and of the shell's VFS IPC.
//!
//! Terminal commands only produce a [`CommandOutput`]; this module turns
//! it into terminal lines and the side effects its signals ask for. The
//! local terminal ([`process_command_output`] / [`apply_command_output`])
//! may toggle servers; remote callers (the remote-terminal listener and
//! MCP `run_command`, both through [`run_remote_command`]) get a response
//! string and may not.
//!
//! The submodules hold the per-frame services the shell step runs:
//!
//! - `skin`: skin swaps, live resolution changes and the backend-side skin
//!   assets (wallpaper, layout textures, cursor) they invalidate.
//! - `settings_ipc`: the Settings app's `/system/ipc` requests and the
//!   `/system/state` files it reads back.
//! - `services`: the remote-terminal listener, FTP and MCP servers, the
//!   outbound remote client and `/system/state/net`.
//! - `ipc`: the `wm` and `plugin` request / status files.

mod ipc;
mod services;
mod settings_ipc;
mod skin;

pub use ipc::{poll_plugin_ipc, poll_wm_ipc, update_plugins};
#[cfg(feature = "mcp")]
pub use services::{mcp_start_from_env, poll_mcp_server};
pub use services::{poll_ftp_server, poll_remote_client, poll_remote_listener, publish_net_state};
pub use settings_ipc::{poll_settings_ipc, publish_runtime_state};
pub(crate) use skin::dashboard_apps;
pub use skin::{
    apply_skin_object, apply_skin_swap, refresh_skin_assets, refresh_wallpaper_if_pending,
};

use oasis_core::browser::BrowserWidget;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin_request};
use oasis_core::terminal::{CommandOutput, CommandSignal};
use oasis_core::terminal_sdi;
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::AppState;

/// Read and clear the IPC request file `path`, returning its trimmed
/// payload. `None` when the file is missing, empty or only whitespace (an
/// empty file is not rewritten, so idle frames do no VFS writes). The file
/// is always cleared once read, so a malformed request cannot loop.
pub(crate) fn take_request(vfs: &mut dyn Vfs, path: &str) -> Option<String> {
    // Every frame polls ~15 request files that are almost always missing
    // or empty: settle that with a cheap existence + size check instead of
    // a permission-checked read (whose missing-file error formats a
    // message). A file that has content is read exactly as before.
    if !vfs.exists(path) || vfs.stat(path).is_ok_and(|m| m.size == 0) {
        return None;
    }
    let data = vfs.read(path).ok().filter(|d| !d.is_empty())?;
    let _ = vfs.write(path, b"");
    let text = String::from_utf8_lossy(&data).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Write the status file `path` only when its contents differ from `text`
/// (status files are republished every frame). A size mismatch settles it
/// from a stat, without copying the file out of the VFS.
pub(crate) fn publish_if_changed(vfs: &mut dyn Vfs, path: &str, text: &str) {
    let same_size = vfs.stat(path).is_ok_and(|m| m.size == text.len() as u64);
    if !same_size || vfs.read(path).ok().as_deref() != Some(text.as_bytes()) {
        let _ = vfs.write(path, text.as_bytes());
    }
}

/// Line printed in place of a server toggle a background job asked for.
pub(crate) const JOB_TOGGLE_REFUSED: &str = "Not available from a background job.";

/// Whether `signal` starts or stops a server or an outbound connection:
/// local authority only (never from a remote caller or a background job).
fn is_server_toggle(signal: &CommandSignal) -> bool {
    matches!(
        signal,
        CommandSignal::ListenToggle { .. }
            | CommandSignal::RemoteConnect { .. }
            | CommandSignal::FtpToggle { .. }
            | CommandSignal::McpToggle { .. }
    )
}

/// Neutralize the signals a remote caller may not trigger (the server
/// toggles [`format_remote_response`] refuses) in a background job's
/// output.
///
/// Jobs (`cmd &`) sit in the one shared command registry and run a frame
/// later through the local output path, so a job queued over the remote
/// listener or MCP `run_command` would otherwise start `ftp` / `listen` /
/// `connect` / `mcp-server` with local authority. Where a job came from is
/// not recorded (and a script, alias or function can queue one), so no
/// job may toggle a server.
pub(crate) fn refuse_job_server_toggles(output: CommandOutput) -> CommandOutput {
    match output {
        CommandOutput::Signal(signal) if is_server_toggle(&signal) => {
            CommandOutput::Text(JOB_TOGGLE_REFUSED.to_string())
        },
        CommandOutput::Multi(outputs) => {
            CommandOutput::Multi(outputs.into_iter().map(refuse_job_server_toggles).collect())
        },
        CommandOutput::Status { code, output } => CommandOutput::Status {
            code,
            output: Box::new(refuse_job_server_toggles(*output)),
        },
        other => other,
    }
}

/// Apply `browser sandbox on|off` to the open browser (if any) and return
/// the line reporting it.
fn set_browser_sandbox(browser: &mut Option<BrowserWidget>, enable: bool) -> String {
    if let Some(bw) = browser {
        bw.config.features.sandbox_only = enable;
    }
    let st = if enable {
        "on (VFS only)"
    } else {
        "off (HTTP enabled)"
    };
    format!("Browser sandbox: {st}")
}

/// Resolve a `skin` command's request against the skin that will be active
/// once the earlier swaps of the same command line are applied (so
/// `skin xp; skin @variant:dark` derives the dark variant of xp), storing
/// it in `pending`. Returns the error line for a bad request.
fn resolve_pending_skin(
    name: &str,
    current: &Skin,
    pending: &mut Option<Skin>,
) -> Result<String, String> {
    let base = pending.as_ref().unwrap_or(current);
    match resolve_skin_request(name, base) {
        Ok(new_skin) => {
            let msg = format!("Switched to skin: {}", new_skin.manifest.name);
            *pending = Some(new_skin);
            Ok(msg)
        },
        Err(e) => Err(format!("Skin error: {e}")),
    }
}

/// Process a local terminal command result: print it and carry out its
/// signals. Returns the skin a `skin` command resolved (the last one when
/// the line swapped several times); the caller applies it with
/// [`apply_skin_object`] once the command's borrows are gone (or calls
/// [`apply_command_output`], which does both).
pub fn process_command_output(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
) -> Option<Skin> {
    let mut pending = None;
    process_into(result, state, &mut pending);
    pending
}

/// [`process_command_output`] plus applying the resolved skin swap.
pub fn apply_command_output(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    if let Some(skin) = process_command_output(result, state) {
        apply_skin_object(skin, state, sdi, vfs);
    }
}

fn process_into(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
    pending: &mut Option<Skin>,
) {
    let output = match result {
        Ok(output) => output,
        Err(e) => {
            // Red via the skin's ANSI palette (SGR 31); the terminal
            // renderer resolves the escape into a themed colored run.
            let line = oasis_core::ansi::colorize(&format!("error: {e}"), 31);
            state.terminal.print(line);
            return;
        },
    };
    let term = &mut state.terminal;
    match output {
        CommandOutput::Text(text) => {
            for l in text.lines() {
                term.print(l);
            }
        },
        CommandOutput::Table { headers, rows } => {
            term.print(headers.join(" | "));
            for row in &rows {
                term.print(row.join(" | "));
            }
        },
        CommandOutput::Clear => term.output_lines.clear(),
        CommandOutput::None => {},
        CommandOutput::Signal(CommandSignal::ListenToggle { port }) => {
            services::toggle_listener(term, &mut state.net, port);
        },
        CommandOutput::Signal(CommandSignal::RemoteConnect { address, port, psk }) => {
            services::connect_remote(term, &mut state.net, &address, port, psk.as_deref());
        },
        CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) => {
            services::toggle_ftp(term, &mut state.net, port, password);
        },
        CommandOutput::Signal(CommandSignal::McpToggle { start, port, token }) => {
            services::toggle_mcp(state, start, port, token);
        },
        CommandOutput::Signal(CommandSignal::BrowserSandbox { enable }) => {
            let line = set_browser_sandbox(&mut state.content.browser, enable);
            term.print(line);
        },
        CommandOutput::Signal(CommandSignal::SkinSwap { name }) => {
            if let Err(line) = resolve_pending_skin(&name, &state.skin, pending) {
                term.print(line);
            }
        },
        CommandOutput::Signal(CommandSignal::SdiInspect { .. }) => {
            // Callers with scene access resolve this first via
            // `terminal_sdi::resolve_sdi_inspect`.
            term.print("sdi: scene inspection not available here");
        },
        CommandOutput::Multi(outputs) => {
            for output in outputs {
                process_into(Ok(output), state, pending);
            }
        },
        // Unwrapped by the executor; handled for completeness.
        CommandOutput::Status { output, .. } => process_into(Ok(*output), state, pending),
    }
}

/// Format a remote command result as a response string, applying side effects
/// that only need the browser (sandbox toggle).
///
/// A skin swap is *resolved* here (so a bad name is reported to the remote
/// caller) but not applied: the resolved skin is stored in `pending_skin`
/// and the caller applies it with [`apply_skin_object`] once it holds the
/// whole [`AppState`]. Applying it here with only the theme/WM borrows
/// left the dashboard, clear color, wallpaper and SFX on the old skin.
pub(crate) fn format_remote_response(
    result: oasis_core::error::Result<CommandOutput>,
    browser: &mut Option<BrowserWidget>,
    skin: &Skin,
    sdi: &SdiRegistry,
    pending_skin: &mut Option<Skin>,
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
        Ok(CommandOutput::Clear | CommandOutput::None) => "OK".to_string(),
        Ok(CommandOutput::Signal(signal)) if is_server_toggle(&signal) => {
            "Not available via remote.".to_string()
        },
        Ok(CommandOutput::Signal(CommandSignal::SdiInspect { name })) => {
            terminal_sdi::inspect_sdi(sdi, name.as_deref())
        },
        Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox { enable })) => {
            set_browser_sandbox(browser, enable)
        },
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap { name })) => {
            resolve_pending_skin(&name, skin, pending_skin).unwrap_or_else(|line| line)
        },
        // Every server toggle is matched above.
        Ok(CommandOutput::Signal(_)) => "Not available via remote.".to_string(),
        Ok(CommandOutput::Multi(outputs)) => {
            let mut parts = Vec::new();
            for output in outputs {
                let resp = format_remote_response(Ok(output), browser, skin, sdi, pending_skin);
                if !resp.is_empty() {
                    parts.push(resp);
                }
            }
            parts.join("\n")
        },
        // Unwrapped by the executor; handled for completeness.
        Ok(CommandOutput::Status { output, .. }) => {
            format_remote_response(Ok(*output), browser, skin, sdi, pending_skin)
        },
        Err(e) => format!("error: {e}"),
    }
}

/// Run `line` for a remote caller (the remote-terminal listener, MCP
/// `run_command`) in the shell's command registry and working directory,
/// and return the response. Server toggles are refused; a skin swap is
/// applied to the whole shell before returning.
pub(crate) fn run_remote_command(
    line: &str,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> String {
    let (result, cwd) = {
        let mut env = crate::terminal_input::make_env(state, vfs);
        let result = state.terminal.cmd_reg.execute(line, &mut env);
        (result, env.cwd)
    };
    state.terminal.cwd = cwd;
    let mut pending_skin = None;
    let response = format_remote_response(
        result,
        &mut state.content.browser,
        &state.skin,
        sdi,
        &mut pending_skin,
    );
    if let Some(new_skin) = pending_skin {
        apply_skin_object(new_skin, state, sdi, vfs);
    }
    response
}

/// Truncate output lines to `MAX_OUTPUT_LINES`.
pub fn trim_output(output_lines: &mut Vec<String>) {
    terminal_sdi::trim_scrollback(output_lines);
}

#[cfg(test)]
mod tests;
