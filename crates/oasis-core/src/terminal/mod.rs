//! Command interpreter and terminal subsystem.
//!
//! Core command types, registry, and built-in commands are provided by
//! the `oasis-terminal` crate. Commands that depend on oasis-core modules
//! or on the UI/skin stack (agent, plugin, browser, tv, skin) live here so
//! `oasis-terminal` stays free of those dependencies.

pub mod agent_commands;
pub mod browser_commands;
pub mod plugin_commands;
pub mod skin_commands;
pub mod tv_commands;

// Explicit re-exports from the oasis-terminal crate.
pub use oasis_terminal::{
    Command, CommandOutput, CommandRegistry, CommandSignal, Environment, SessionEvent,
    ShellSession, cmd_helpers, populate_man_pages, populate_motd, populate_profile,
};

pub use agent_commands::register_agent_commands;
pub use browser_commands::{BROWSER_REQUEST_PATH, register_browser_commands};
pub use plugin_commands::register_plugin_commands;
pub use skin_commands::register_skin_commands;
pub use tv_commands::register_tv_commands;

/// Register every `oasis-terminal` command plus the `skin` command.
///
/// This is the base command set every host (desktop, WASM, UE5 FFI) gets;
/// hosts add the oasis-core subsystem commands (agent, plugin, script,
/// transfer, update, tv, browser) on top as they need them.
pub fn register_builtins(reg: &mut CommandRegistry) {
    oasis_terminal::register_builtins(reg);
    register_skin_commands(reg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    #[test]
    fn register_builtins_includes_skin() {
        let mut reg = CommandRegistry::new();
        register_builtins(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let out = reg.execute("skin modern", &mut env).unwrap();
        let CommandOutput::Signal(CommandSignal::SkinSwap { name }) = out else {
            panic!("expected SkinSwap, got {out:?}");
        };
        assert_eq!(name, "modern");
        assert!(reg.execute("music status", &mut env).is_ok());
    }
}
