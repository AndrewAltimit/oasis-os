//! Command interpreter and terminal subsystem.
//!
//! The terminal is a registry-based dispatch system. Commands implement the
//! `Command` trait and are registered by name. The interpreter parses input
//! lines, resolves the command name, and dispatches `execute()`.

pub mod audio_commands;
mod commands;
mod interpreter;
pub mod network_commands;
pub mod skin_commands;

/// Register audio playback commands (music) into a registry.
pub use audio_commands::register_audio_commands;
/// Register all built-in commands (fs, system, network, audio, skin) into a registry.
pub use commands::register_builtins;
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
/// Register skin management commands (skin list/switch) into a registry.
pub use skin_commands::register_skin_commands;
