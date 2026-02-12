//! Command trait, registry, and dispatch logic.

use std::collections::HashMap;

use crate::error::{OasisError, Result};
use crate::platform::{NetworkService, PowerService, TimeService, UsbService};
use crate::vfs::Vfs;

/// Output produced by a command.
#[derive(Debug, Clone)]
pub enum CommandOutput {
    /// Plain text lines.
    Text(String),
    /// Tabular data (header row + data rows).
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Command produced no visible output.
    None,
    /// Signal to clear the terminal output buffer.
    Clear,
    /// Signal to the app to start/stop the remote terminal listener.
    ListenToggle {
        /// Port to listen on (0 = stop).
        port: u16,
    },
    /// Signal to the app to connect to a remote host.
    RemoteConnect {
        address: String,
        port: u16,
        psk: Option<String>,
    },
    /// Signal to the app to toggle browser sandbox mode.
    BrowserSandbox {
        /// `true` = sandbox on (VFS only), `false` = networking enabled.
        enable: bool,
    },
    /// Signal to the app to swap the active skin.
    SkinSwap {
        /// Skin name or path to load.
        name: String,
    },
}

/// Shared mutable environment passed to every command.
pub struct Environment<'a> {
    /// Current working directory (VFS path).
    pub cwd: String,
    /// The virtual file system.
    pub vfs: &'a mut dyn Vfs,
    /// Power service for battery/CPU queries.
    pub power: Option<&'a dyn PowerService>,
    /// Time service for clock/uptime queries.
    pub time: Option<&'a dyn TimeService>,
    /// USB service for status queries.
    pub usb: Option<&'a dyn UsbService>,
    /// Network service for WiFi status queries.
    pub network: Option<&'a dyn NetworkService>,
    /// TLS provider for HTTPS connections.
    pub tls: Option<&'a dyn crate::net::tls::TlsProvider>,
}

/// A single executable command.
pub trait Command {
    /// The command name (what the user types).
    fn name(&self) -> &str;

    /// One-line description for `help`.
    fn description(&self) -> &str;

    /// Usage string (e.g. "ls [path]").
    fn usage(&self) -> &str;

    /// Execute the command with the given arguments and environment.
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput>;
}

/// Registry of available commands with dispatch.
pub struct CommandRegistry {
    commands: HashMap<String, Box<dyn Command>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Register a command. Replaces any existing command with the same name.
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        self.commands.insert(cmd.name().to_string(), cmd);
    }

    /// Parse and execute a command line.
    pub fn execute(&self, line: &str, env: &mut Environment<'_>) -> Result<CommandOutput> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(CommandOutput::None);
        }
        let name = parts[0];
        let args = &parts[1..];

        match self.commands.get(name) {
            Some(cmd) => cmd.execute(args, env),
            None => Err(OasisError::Command(format!("unknown command: {name}"))),
        }
    }

    /// Return a sorted list of (name, description) pairs.
    pub fn list_commands(&self) -> Vec<(&str, &str)> {
        let mut cmds: Vec<(&str, &str)> = self
            .commands
            .values()
            .map(|c| (c.name(), c.description()))
            .collect();
        cmds.sort_by_key(|(name, _)| *name);
        cmds
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    struct EchoCmd;
    impl Command for EchoCmd {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Print arguments"
        }
        fn usage(&self) -> &str {
            "echo [text...]"
        }
        fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
            Ok(CommandOutput::Text(args.join(" ")))
        }
    }

    #[test]
    fn register_and_execute() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
        };
        match reg.execute("echo hello world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn unknown_command() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
        };
        assert!(reg.execute("nonexistent", &mut env).is_err());
    }

    #[test]
    fn empty_input() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
        };
        match reg.execute("", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for empty input"),
        }
    }

    #[test]
    fn list_commands_sorted() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let cmds = reg.list_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, "echo");
    }

    // -- Additional interpreter hardening tests --

    #[test]
    fn whitespace_only_input_returns_none() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        match reg.execute("   \t  ", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for whitespace-only input"),
        }
    }

    #[test]
    fn multiple_spaces_between_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        // split_whitespace collapses multiple spaces
        match reg.execute("echo   hello    world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn leading_trailing_whitespace() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        match reg.execute("  echo hi  ", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hi"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn command_no_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        match reg.execute("echo", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, ""),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn unknown_command_error_message_contains_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        let err = reg.execute("foobar", &mut env).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("foobar"), "error should contain command name");
    }

    #[test]
    fn register_replaces_existing_command() {
        struct CmdA;
        impl Command for CmdA {
            fn name(&self) -> &str { "test" }
            fn description(&self) -> &str { "version A" }
            fn usage(&self) -> &str { "test" }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Text("A".into()))
            }
        }
        struct CmdB;
        impl Command for CmdB {
            fn name(&self) -> &str { "test" }
            fn description(&self) -> &str { "version B" }
            fn usage(&self) -> &str { "test" }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Text("B".into()))
            }
        }

        let mut reg = CommandRegistry::new();
        reg.register(Box::new(CmdA));
        reg.register(Box::new(CmdB));

        let cmds = reg.list_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].1, "version B");
    }

    #[test]
    fn list_commands_sorted_multiple() {
        struct Named(&'static str);
        impl Command for Named {
            fn name(&self) -> &str { self.0 }
            fn description(&self) -> &str { "desc" }
            fn usage(&self) -> &str { self.0 }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::None)
            }
        }

        let mut reg = CommandRegistry::new();
        reg.register(Box::new(Named("zebra")));
        reg.register(Box::new(Named("alpha")));
        reg.register(Box::new(Named("middle")));

        let cmds = reg.list_commands();
        assert_eq!(cmds[0].0, "alpha");
        assert_eq!(cmds[1].0, "middle");
        assert_eq!(cmds[2].0, "zebra");
    }

    #[test]
    fn default_creates_empty_registry() {
        let reg = CommandRegistry::default();
        assert!(reg.list_commands().is_empty());
    }

    #[test]
    fn command_output_variants_are_debug() {
        let outputs = vec![
            CommandOutput::Text("hi".into()),
            CommandOutput::Table {
                headers: vec!["a".into()],
                rows: vec![vec!["b".into()]],
            },
            CommandOutput::None,
            CommandOutput::Clear,
            CommandOutput::ListenToggle { port: 8080 },
            CommandOutput::RemoteConnect {
                address: "1.2.3.4".into(),
                port: 22,
                psk: Some("key".into()),
            },
            CommandOutput::BrowserSandbox { enable: true },
            CommandOutput::SkinSwap { name: "xp".into() },
        ];
        for o in &outputs {
            // Just ensure Debug doesn't panic.
            let _ = format!("{o:?}");
        }
    }

    #[test]
    fn many_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        };
        let long_input = format!("echo {}", (0..100).map(|i| i.to_string()).collect::<Vec<_>>().join(" "));
        match reg.execute(&long_input, &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("99")),
            _ => panic!("expected text output"),
        }
    }

    // -- Robustness / edge cases ----------------------------------------

    fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
        Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
        }
    }

    #[test]
    fn very_long_command_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_name = "a".repeat(10_000);
        assert!(reg.execute(&long_name, &mut env).is_err());
    }

    #[test]
    fn very_long_argument() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_arg = "x".repeat(50_000);
        let input = format!("echo {long_arg}");
        match reg.execute(&input, &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s.len(), 50_000),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn null_bytes_in_input() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Input with null bytes should not panic.
        let input = "echo hello\0world";
        let result = reg.execute(input, &mut env);
        assert!(result.is_ok());
    }

    #[test]
    fn tab_separated_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo\thello\tworld", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("hello")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn newline_in_input() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Newline in input -- might split or include in args.
        let result = reg.execute("echo line1\nline2", &mut env);
        assert!(result.is_ok());
    }

    #[test]
    fn only_spaces() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("     ", &mut env).unwrap() {
            CommandOutput::None => {}
            _ => panic!("expected None for whitespace-only"),
        }
    }

    #[test]
    fn command_case_sensitivity() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Commands are case-sensitive; ECHO should not find echo.
        let result = reg.execute("ECHO hello", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn register_many_commands() {
        let mut reg = CommandRegistry::new();
        for _ in 0..100 {
            reg.register(Box::new(EchoCmd));
        }
        // All register to same name "echo" -- last one wins.
        let cmds = reg.list_commands();
        assert!(cmds.iter().any(|(name, _)| *name == "echo"));
    }

    #[test]
    fn execute_with_special_chars_in_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo @#$%^&*()!<>", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("@#$%^&*()!<>")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn execute_unicode_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo こんにちは 世界", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("こんにちは"));
                assert!(s.contains("世界"));
            }
            _ => panic!("expected text output"),
        }
    }
}
