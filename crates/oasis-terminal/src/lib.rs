//! Command interpreter and terminal subsystem.
//!
//! The terminal is a registry-based dispatch system. Commands implement the
//! `Command` trait and are registered by name. The interpreter parses input
//! lines, resolves the command name, and dispatches `execute()`.

pub mod audio_commands;
mod commands;
pub mod dev_commands;
pub mod doc_commands;
pub mod file_commands;
pub mod fun_commands;
mod interpreter;
pub mod network_commands;
pub mod security_commands;
pub mod skin_commands;
pub mod system_commands;
pub mod text_commands;
pub mod ui_commands;

/// Register audio playback commands (music) into a registry.
pub use audio_commands::register_audio_commands;
/// Register all built-in commands (fs, system, network, audio, skin) into a registry.
pub use commands::register_builtins;
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
/// Shared mutable environment passed to every command.
pub use interpreter::Environment;
/// Register network commands (wifi, ping, http) into a registry.
pub use network_commands::register_network_commands;
/// Register security commands (chmod, chown, passwd, audit).
pub use security_commands::register_security_commands;
/// Register skin management commands (skin list/switch) into a registry.
pub use skin_commands::register_skin_commands;
/// Register system commands (uptime, df, whoami, hostname, date, sleep).
pub use system_commands::register_system_commands;
/// Register text processing commands (head, tail, wc, grep, sort, uniq, tee, tr, cut, diff).
pub use text_commands::register_text_commands;
/// Register UI control commands (wm, sdi, theme, notify, screenshot).
pub use ui_commands::register_ui_commands;
