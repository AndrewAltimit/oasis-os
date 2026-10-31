//! Command interpreter and terminal subsystem.
//!
//! The terminal is a registry-based dispatch system. Commands implement the
//! `Command` trait and are registered by name. The interpreter parses input
//! lines, resolves the command name, and dispatches `execute()`.

#[macro_use]
mod command_macro;
mod audio_commands;
mod builtins;
mod commands;
pub mod completion;
#[cfg(test)]
mod control_flow;
mod core_commands;
mod dev_commands;
mod doc_commands;
mod executor;
pub(crate) mod expander;
mod expansion;
mod file_commands;
mod fun_commands;
pub mod highlight;
mod interpreter;
pub mod jobs;
pub mod line_edit;
mod network_commands;
pub(crate) mod pipeline;
mod platform_commands;
mod radio_commands;
mod redirect;
mod registry;
mod remote_commands;
mod script;
mod security_commands;
pub mod session;
mod system_commands;
mod text_commands;
mod types;
mod ui_commands;

#[cfg(test)]
mod test_helpers;

/// Register audio playback commands (music) into a registry.
pub use audio_commands::register_audio_commands;
/// Register all of this crate's commands (fs, system, network, audio, radio, ...).
pub use commands::register_builtins;
/// Register core filesystem and shell commands (ls, cd, pwd, cat, echo, etc.).
pub use core_commands::register_core_commands;
/// Register developer tool commands (base64, json, uuid, seq, expr, test, xargs).
pub use dev_commands::register_dev_commands;
/// Populate default man pages in the VFS.
pub use doc_commands::populate_man_pages;
/// Populate default MOTD in the VFS.
pub use doc_commands::populate_motd;
/// Populate default shell profile in the VFS.
pub use doc_commands::populate_profile;
/// Register documentation commands (man, tutorial, motd).
pub use doc_commands::register_doc_commands;
/// Register file utility commands (write, append, tree, du, stat, xxd, checksum).
pub use file_commands::register_file_commands;
/// Register fun/utility commands (cal, fortune, banner, matrix, yes, watch, time).
pub use fun_commands::register_fun_commands;
/// A single executable command trait.
pub use interpreter::Command;
/// Output produced by a command (text, table, signals).
pub use interpreter::CommandOutput;
/// Registry of available commands with dispatch.
pub use interpreter::CommandRegistry;
/// Signals sent from commands to the app layer (network, skin, etc.).
pub use interpreter::CommandSignal;
/// Shared mutable environment passed to every command.
pub use interpreter::Environment;
/// Resolve a (possibly relative) path against a working directory.
pub use interpreter::resolve_path;
/// Register network commands (wifi, ping, http) into a registry.
pub use network_commands::register_network_commands;
/// Register platform service commands (power, clock, memory, usb) into a registry.
pub use platform_commands::register_platform_commands;
/// Register internet radio commands (radio) into a registry.
pub use radio_commands::register_radio_commands;
/// Register remote terminal commands (listen, remote, hosts) into a registry.
pub use remote_commands::register_remote_commands;
/// Register security commands (chmod, chown, passwd, audit).
pub use security_commands::register_security_commands;
/// Register system commands (uptime, df, whoami, hostname, date, sleep).
pub use system_commands::register_system_commands;
/// Register text processing commands (head, tail, wc, grep, sort, uniq, tee, tr, cut, diff).
pub use text_commands::register_text_commands;
/// Register UI control commands (wm, sdi, theme, notify, screenshot).
pub use ui_commands::register_ui_commands;

pub mod cmd_helpers;
pub use cmd_helpers::{FlagSpec, Flags, parse_flags, require_args, require_args_exact, time_seed};
/// How deeply shell constructs may nest (`$(...)`, function calls, scripts,
/// `if` / loop / `case` blocks, all together).
pub use types::MAX_EXEC_DEPTH;
/// What [`Command::execute`] returns (the alias `define_command!` expands to).
pub type CommandResult = oasis_types::error::Result<CommandOutput>;
/// Shell job (background or stopped command).
pub use jobs::Job;
/// Manages shell jobs (background and stopped commands).
pub use jobs::JobManager;
/// Current state of a job (Running, Stopped, Done).
pub use jobs::JobState;
/// Parse a job specifier like `%1`, `%%`, `%+`, `%-`.
pub use jobs::parse_job_spec;
/// Readline-style line editing: actions, results, and the editor state machine.
pub use line_edit::{EditAction, EditResult, LineEditor};
/// Interactive shell session (line editor + history + completion) for hosts.
pub use session::{SessionEvent, ShellSession};
