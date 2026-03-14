//! Built-in commands for the OASIS_OS terminal.
//!
//! This module registers all built-in commands by delegating to focused
//! command modules. The actual command implementations live in:
//! - [`core_commands`] -- filesystem and shell basics (ls, cd, cat, echo, etc.)
//! - [`platform_commands`] -- platform service queries (power, clock, memory, usb)
//! - [`remote_commands`] -- remote terminal (listen, remote, hosts)
//! - Plus existing modules: network, audio, radio, skin, text, file, system,
//!   dev, fun, ui, security, doc.

use oasis_types::error::Result;

use crate::interpreter::{Command, CommandOutput, Environment};

/// Register all built-in commands into a registry.
///
/// This registers the core commands (fs, system, network) plus audio/network/skin
/// command modules. Additional command modules (agent, plugin, script, transfer,
/// update) are registered by `oasis-core` via `register_all_commands`.
pub fn register_builtins(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(HelpCmd));
    // Core filesystem and shell commands.
    crate::register_core_commands(reg);
    // Platform service commands.
    crate::register_platform_commands(reg);
    // Remote terminal commands.
    crate::register_remote_commands(reg);
    // Network commands.
    crate::register_network_commands(reg);
    // Audio commands.
    crate::register_audio_commands(reg);
    // Internet radio commands.
    crate::register_radio_commands(reg);
    // Skin switching commands.
    crate::register_skin_commands(reg);
    // Text processing commands.
    crate::register_text_commands(reg);
    // File utility commands.
    crate::register_file_commands(reg);
    // System commands.
    crate::register_system_commands(reg);
    // Developer tool commands.
    crate::register_dev_commands(reg);
    // Fun/utility commands.
    crate::register_fun_commands(reg);
    // UI control commands.
    crate::register_ui_commands(reg);
    // Security commands.
    crate::register_security_commands(reg);
    // Documentation commands (man, tutorial, motd).
    crate::register_doc_commands(reg);
}

// ---------------------------------------------------------------------------
// help
// ---------------------------------------------------------------------------

struct HelpCmd;
impl Command for HelpCmd {
    fn name(&self) -> &str {
        "help"
    }
    fn description(&self) -> &str {
        "List available commands"
    }
    fn usage(&self) -> &str {
        "help"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        // We can't access the registry from here, so we return a static list.
        // The caller wraps this with the actual registry listing.
        // Instead, produce a marker that the registry intercepts.
        Ok(CommandOutput::Text(
            "Use 'help' at the terminal for a list of commands.".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, CommandSignal};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_builtins(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.write("/home/user/readme.txt", b"Hello OASIS").unwrap();
        (reg, vfs)
    }

    fn exec(
        reg: &CommandRegistry,
        vfs: &mut MemoryVfs,
        cwd: &mut String,
        line: &str,
    ) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: cwd.clone(),
            vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let result = reg.execute(line, &mut env);
        *cwd = env.cwd;
        result
    }

    // =================================================================
    // Integration tests: multi-step terminal sessions
    // =================================================================

    #[test]
    fn session_mkdir_cd_touch_cat() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Create a project directory structure.
        exec(&reg, &mut vfs, &mut cwd, "mkdir /projects").unwrap();
        exec(&reg, &mut vfs, &mut cwd, "mkdir /projects/myapp").unwrap();

        // Navigate into it.
        exec(&reg, &mut vfs, &mut cwd, "cd /projects/myapp").unwrap();
        assert_eq!(cwd, "/projects/myapp");

        // Create and write a file.
        exec(&reg, &mut vfs, &mut cwd, "touch config.txt").unwrap();
        assert!(vfs.exists("/projects/myapp/config.txt"));

        // Write content via VFS directly, then verify cat reads it.
        vfs.write("/projects/myapp/config.txt", b"debug=true")
            .unwrap();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "cat config.txt").unwrap()),
            "debug=true"
        );

        // ls should show the file.
        assert!(assert_text!(exec(&reg, &mut vfs, &mut cwd, "ls").unwrap()).contains("config.txt"));
    }

    #[test]
    fn session_cp_mv_find_workflow() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Start with the file from setup: /home/user/readme.txt
        // Copy it to a backup.
        exec(
            &reg,
            &mut vfs,
            &mut cwd,
            "cp /home/user/readme.txt /home/user/readme.bak",
        )
        .unwrap();
        assert!(vfs.exists("/home/user/readme.bak"));
        assert!(vfs.exists("/home/user/readme.txt"));

        // Move the original to a new location.
        exec(&reg, &mut vfs, &mut cwd, "mkdir /archive").unwrap();
        exec(
            &reg,
            &mut vfs,
            &mut cwd,
            "mv /home/user/readme.txt /archive/readme.txt",
        )
        .unwrap();
        assert!(!vfs.exists("/home/user/readme.txt"));
        assert!(vfs.exists("/archive/readme.txt"));

        // Find should locate both copies.
        let s = assert_text!(exec(&reg, &mut vfs, &mut cwd, "find / readme").unwrap());
        assert!(s.contains("/home/user/readme.bak"));
        assert!(s.contains("/archive/readme.txt"));
    }

    #[test]
    fn session_cwd_tracking_across_commands() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // cd to a directory, use relative paths.
        exec(&reg, &mut vfs, &mut cwd, "cd /home").unwrap();
        assert_eq!(cwd, "/home");

        exec(&reg, &mut vfs, &mut cwd, "cd user").unwrap();
        assert_eq!(cwd, "/home/user");

        // pwd should reflect current cwd.
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "pwd").unwrap()),
            "/home/user"
        );

        // Go up with ..
        exec(&reg, &mut vfs, &mut cwd, "cd ..").unwrap();
        assert_eq!(cwd, "/home");

        exec(&reg, &mut vfs, &mut cwd, "cd ..").unwrap();
        assert_eq!(cwd, "/");

        // Verify we can't go above root.
        exec(&reg, &mut vfs, &mut cwd, "cd ..").unwrap();
        assert_eq!(cwd, "/");
    }

    #[test]
    fn session_file_lifecycle() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Create a temp directory and file.
        exec(&reg, &mut vfs, &mut cwd, "mkdir /tmp").unwrap();
        exec(&reg, &mut vfs, &mut cwd, "touch /tmp/data.log").unwrap();
        assert!(vfs.exists("/tmp/data.log"));

        // Write content then cat to verify.
        vfs.write("/tmp/data.log", b"line 1\nline 2").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, &mut cwd, "cat /tmp/data.log").unwrap());
        assert!(s.contains("line 1"));
        assert!(s.contains("line 2"));

        // Remove the file.
        exec(&reg, &mut vfs, &mut cwd, "rm /tmp/data.log").unwrap();
        assert!(!vfs.exists("/tmp/data.log"));

        // Cat should now fail.
        assert!(exec(&reg, &mut vfs, &mut cwd, "cat /tmp/data.log").is_err());
    }

    #[test]
    fn session_relative_paths_with_cwd() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Create structure via relative paths after cd.
        exec(&reg, &mut vfs, &mut cwd, "cd /home/user").unwrap();
        exec(&reg, &mut vfs, &mut cwd, "mkdir docs").unwrap();
        assert!(vfs.exists("/home/user/docs"));

        exec(&reg, &mut vfs, &mut cwd, "touch docs/notes.txt").unwrap();
        assert!(vfs.exists("/home/user/docs/notes.txt"));

        // ls relative path.
        assert!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "ls docs").unwrap()).contains("notes.txt")
        );

        // cp with relative paths.
        exec(
            &reg,
            &mut vfs,
            &mut cwd,
            "cp docs/notes.txt docs/notes2.txt",
        )
        .unwrap();
        assert!(vfs.exists("/home/user/docs/notes2.txt"));
    }

    #[test]
    fn session_skin_commands() {
        use crate::register_skin_commands;
        let mut reg = CommandRegistry::new();
        register_builtins(&mut reg);
        register_skin_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut cwd = "/".to_string();

        // List skins.
        let s = assert_text!(exec(&reg, &mut vfs, &mut cwd, "skin list").unwrap());
        assert!(s.contains("terminal"));
        assert!(s.contains("modern"));

        // Switch to a skin.
        let __out = exec(&reg, &mut vfs, &mut cwd, "skin modern").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected SkinSwap, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::SkinSwap { name }) = __out else {
            unreachable!()
        };
        assert_eq!(name, "modern");

        // Switch to another skin.
        let __out = exec(&reg, &mut vfs, &mut cwd, "skin terminal").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected SkinSwap, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::SkinSwap { name }) = __out else {
            unreachable!()
        };
        assert_eq!(name, "terminal");
    }

    #[test]
    fn session_error_recovery() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Attempt invalid operations.
        assert!(exec(&reg, &mut vfs, &mut cwd, "cd /nonexistent").is_err());
        // CWD should be unchanged after failed cd.
        assert_eq!(cwd, "/");

        assert!(exec(&reg, &mut vfs, &mut cwd, "cat /no/such/file").is_err());
        assert!(exec(&reg, &mut vfs, &mut cwd, "rm /no/such/file").is_err());

        // Valid commands should still work after errors.
        exec(&reg, &mut vfs, &mut cwd, "mkdir /tmp").unwrap();
        assert!(vfs.exists("/tmp"));

        // CWD still correct.
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "pwd").unwrap()),
            "/"
        );
    }

    #[test]
    fn session_nested_directory_creation() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();

        // Create deeply nested structure.
        exec(&reg, &mut vfs, &mut cwd, "mkdir /a/b/c/d").unwrap();
        assert!(vfs.exists("/a"));
        assert!(vfs.exists("/a/b"));
        assert!(vfs.exists("/a/b/c"));
        assert!(vfs.exists("/a/b/c/d"));

        // Navigate through it.
        exec(&reg, &mut vfs, &mut cwd, "cd /a/b/c/d").unwrap();
        assert_eq!(cwd, "/a/b/c/d");

        // Create file at the deepest level.
        exec(&reg, &mut vfs, &mut cwd, "touch leaf.txt").unwrap();
        assert!(vfs.exists("/a/b/c/d/leaf.txt"));

        // Find the file from root.
        exec(&reg, &mut vfs, &mut cwd, "cd /").unwrap();
        assert!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "find / leaf").unwrap())
                .contains("/a/b/c/d/leaf.txt")
        );
    }
}
