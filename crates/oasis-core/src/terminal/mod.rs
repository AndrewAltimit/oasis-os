//! Command interpreter and terminal subsystem.
//!
//! Core command types, registry, and built-in commands are provided by
//! the `oasis-terminal` crate. Agent and plugin commands remain here
//! because they depend on oasis-core modules (agent, plugin).

pub mod agent_commands;
pub mod browser_commands;
pub mod plugin_commands;
pub mod tv_commands;

// Explicit re-exports from the oasis-terminal crate.
pub use oasis_terminal::{
    Command, CommandOutput, CommandRegistry, CommandSignal, Environment, cmd_helpers,
    populate_man_pages, populate_motd, populate_profile, register_builtins,
};

pub use agent_commands::register_agent_commands;
pub use browser_commands::register_browser_commands;
pub use plugin_commands::register_plugin_commands;
pub use tv_commands::register_tv_commands;
