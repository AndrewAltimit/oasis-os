//! Command interpreter and terminal subsystem.
//!
//! Core command types, registry, and built-in commands are provided by
//! the `oasis-terminal` crate. Agent and plugin commands remain here
//! because they depend on oasis-core modules (agent, plugin).

pub mod agent_commands;
pub mod plugin_commands;
pub mod tv_commands;

// Re-export everything from the oasis-terminal crate.
pub use oasis_terminal::*;

pub use agent_commands::register_agent_commands;
pub use plugin_commands::register_plugin_commands;
pub use tv_commands::register_tv_commands;
