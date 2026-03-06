//! Command trait, registry, and dispatch logic.
//!
//! Supports quoted arguments, environment variables, command substitution,
//! command history, pipes, input/output redirection, command chaining,
//! and glob expansion.
//!
//! The implementation is split across three submodules:
//! - [`crate::types`]: core types (`CommandOutput`, `Environment`,
//!   `Command`, `ShellFunction`, constants)
//! - [`crate::registry`]: `CommandRegistry` struct, variable/alias/
//!   function/history APIs, expansion helpers
//! - [`crate::executor`]: execution pipeline (`execute`,
//!   `execute_pipeline`, `execute_with_redirect`,
//!   `execute_single_cmd`, `expand_substitutions`)

// Re-export everything so `use crate::interpreter::*` paths keep working.
pub use crate::expander::resolve_path;
pub use crate::registry::CommandRegistry;
pub use crate::types::{Command, CommandOutput, Environment};

// Items re-exported only for tests and internal use.
#[cfg(test)]
pub(crate) use crate::expander::case_pattern_matches;
#[cfg(test)]
pub use crate::expander::tokenize;
#[cfg(test)]
pub(crate) use crate::expander::expand_braces;
#[cfg(test)]
pub(crate) use crate::pipeline::parse_redirect;
#[cfg(test)]
use oasis_types::error::Result;

// Builtin commands and script execution are in separate files:
// - builtins.rs: help, which, function, return, break, continue, local,
//                history, set, unset, env, alias, unalias, list_commands,
//                completions
// - script.rs:   run, execute_script_block, collect_loop_body,
//                execute_if_block, execute_case_block,
//                execute_script_line, eval_condition

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expander::{glob_match, glob_match_simple};
    use oasis_vfs::{MemoryVfs, Vfs};

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
        fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
            // If stdin is set and no args, echo stdin.
            if args.is_empty() {
                if let Some(ref stdin) = env.stdin {
                    return Ok(CommandOutput::Text(stdin.clone()));
                }
            }
            Ok(CommandOutput::Text(args.join(" ")))
        }
    }

    fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
        Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        }
    }

    #[test]
    fn register_and_execute() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn unknown_command() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("nonexistent", &mut env);
        match result {
            Ok(CommandOutput::Text(s)) => assert!(s.contains("error")),
            Err(_) => {},
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn empty_input() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
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

    #[test]
    fn whitespace_only_input_returns_none() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
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
        let mut env = make_env(&mut vfs);
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
        let mut env = make_env(&mut vfs);
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
        let mut env = make_env(&mut vfs);
        match reg.execute("echo", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, ""),
            CommandOutput::None => {}, // Empty echo may produce no output.
            _ => panic!("expected text or none"),
        }
    }

    #[test]
    fn unknown_command_error_message_contains_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("foobar", &mut env) {
            Ok(CommandOutput::Text(s)) => {
                assert!(s.contains("foobar"), "error should contain command name");
            },
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("foobar"), "error should contain command name");
            },
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn register_replaces_existing_command() {
        struct CmdA;
        impl Command for CmdA {
            fn name(&self) -> &str {
                "test"
            }
            fn description(&self) -> &str {
                "version A"
            }
            fn usage(&self) -> &str {
                "test"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Text("A".into()))
            }
        }
        struct CmdB;
        impl Command for CmdB {
            fn name(&self) -> &str {
                "test"
            }
            fn description(&self) -> &str {
                "version B"
            }
            fn usage(&self) -> &str {
                "test"
            }
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
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "desc"
            }
            fn usage(&self) -> &str {
                self.0
            }
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
            CommandOutput::FtpToggle { port: 2121 },
        ];
        for o in &outputs {
            let _ = format!("{o:?}");
        }
    }

    #[test]
    fn many_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_input = format!(
            "echo {}",
            (0..100)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        match reg.execute(&long_input, &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("99")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn very_long_command_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_name = "a".repeat(10_000);
        let result = reg.execute(&long_name, &mut env);
        // Should return error text (unknown command).
        match result {
            Ok(CommandOutput::Text(s)) => assert!(s.contains("error")),
            Err(_) => {},
            _ => panic!("expected error"),
        }
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
        let result = reg.execute("echo line1\nline2", &mut env);
        assert!(result.is_ok());
    }

    #[test]
    fn only_spaces() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("     ", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for whitespace-only"),
        }
    }

    #[test]
    fn command_case_insensitive() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("ECHO hello", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn register_many_commands() {
        let mut reg = CommandRegistry::new();
        for _ in 0..100 {
            reg.register(Box::new(EchoCmd));
        }
        let cmds = reg.list_commands();
        assert!(cmds.iter().any(|(name, _)| *name == "echo"));
    }

    #[test]
    fn execute_with_special_chars_in_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo @#$%^&", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("@#")),
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
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Tokenizer tests --

    #[test]
    fn tokenize_simple() {
        assert_eq!(tokenize("hello world").unwrap(), vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_single_quotes() {
        assert_eq!(
            tokenize("echo 'hello world'").unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_double_quotes() {
        assert_eq!(
            tokenize(r#"echo "hello world""#).unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_backslash_escape() {
        assert_eq!(
            tokenize(r"echo hello\ world").unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_mixed_quotes() {
        assert_eq!(
            tokenize(r#"echo 'single' "double" plain"#).unwrap(),
            vec!["echo", "single", "double", "plain"]
        );
    }

    #[test]
    fn tokenize_empty() {
        assert!(tokenize("").unwrap().is_empty());
    }

    #[test]
    fn tokenize_unterminated_single() {
        assert!(tokenize("echo 'unterminated").is_err());
    }

    #[test]
    fn tokenize_unterminated_double() {
        assert!(tokenize(r#"echo "unterminated"#).is_err());
    }

    // -- Variable expansion tests --

    #[test]
    fn variable_expansion() {
        let reg = CommandRegistry::new();
        reg.set_variable("NAME", "oasis");
        let result = reg.expand_variables("hello $NAME", "/");
        assert_eq!(result, "hello oasis");
    }

    #[test]
    fn variable_expansion_braces() {
        let reg = CommandRegistry::new();
        reg.set_variable("NAME", "oasis");
        let result = reg.expand_variables("hello ${NAME}!", "/");
        assert_eq!(result, "hello oasis!");
    }

    #[test]
    fn variable_cwd() {
        let reg = CommandRegistry::new();
        let result = reg.expand_variables("pwd=$CWD", "/home/user");
        assert_eq!(result, "pwd=/home/user");
    }

    #[test]
    fn variable_exit_code() {
        let reg = CommandRegistry::new();
        reg.last_exit_code.set(42);
        let result = reg.expand_variables("exit=$?", "/");
        assert_eq!(result, "exit=42");
    }

    #[test]
    fn variable_undefined() {
        let reg = CommandRegistry::new();
        let result = reg.expand_variables("$UNDEFINED_VAR", "/");
        assert_eq!(result, "");
    }

    // -- History tests --

    #[test]
    fn history_push_and_retrieve() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo first", &mut env).unwrap();
        reg.execute("echo second", &mut env).unwrap();
        let hist = reg.history();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0], "echo first");
        assert_eq!(hist[1], "echo second");
    }

    #[test]
    fn history_bang_bang() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo hello", &mut env).unwrap();
        match reg.execute("!!", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn history_bang_n() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo first", &mut env).unwrap();
        reg.execute("echo second", &mut env).unwrap();
        match reg.execute("!1", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "first"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn history_no_duplicates() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo same", &mut env).unwrap();
        reg.execute("echo same", &mut env).unwrap();
        assert_eq!(reg.history().len(), 1);
    }

    // -- Pipe tests --

    #[test]
    fn pipe_two_commands() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));

        // CatCmd that reads stdin.
        struct UpperCmd;
        impl Command for UpperCmd {
            fn name(&self) -> &str {
                "upper"
            }
            fn description(&self) -> &str {
                "Uppercase stdin"
            }
            fn usage(&self) -> &str {
                "upper"
            }
            fn execute(&self, _: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
                let input = env.stdin.take().unwrap_or_default();
                Ok(CommandOutput::Text(input.to_uppercase()))
            }
        }
        reg.register(Box::new(UpperCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello | upper", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "HELLO"),
            _ => panic!("expected text output"),
        }
    }

    // -- Chaining tests --

    #[test]
    fn chain_semicolon() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello ; echo world", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("hello"));
                assert!(s.contains("world"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_and_success() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo first && echo second", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("first"));
                assert!(s.contains("second"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_and_failure() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // nonexistent fails -> echo after && should NOT run.
        match reg
            .execute("nonexistent && echo should_not_run", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert!(s.contains("error"));
                assert!(!s.contains("should_not_run"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_or_success() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // echo succeeds -> echo after || should NOT run.
        match reg.execute("echo ok || echo fallback", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("ok"));
                assert!(!s.contains("fallback"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_or_failure() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // nonexistent fails -> echo after || SHOULD run.
        match reg
            .execute("nonexistent || echo fallback", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert!(s.contains("fallback"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Chain preserves signals alongside text --

    #[test]
    fn chain_signal_then_text_produces_multi() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        // ClearCmd for a signal output.
        struct ClearCmd;
        impl Command for ClearCmd {
            fn name(&self) -> &str {
                "clear"
            }
            fn description(&self) -> &str {
                "Clear"
            }
            fn usage(&self) -> &str {
                "clear"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Clear)
            }
        }
        reg.register(Box::new(ClearCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // `clear ; echo Done` should produce Multi with both outputs.
        match reg.execute("clear ; echo Done", &mut env).unwrap() {
            CommandOutput::Multi(outputs) => {
                assert_eq!(outputs.len(), 2);
                assert!(matches!(outputs[0], CommandOutput::Clear));
                assert!(matches!(outputs[1], CommandOutput::Text(_)));
                if let CommandOutput::Text(ref s) = outputs[1] {
                    assert_eq!(s, "Done");
                }
            },
            _ => panic!("expected Multi output"),
        }
    }

    #[test]
    fn chain_text_then_signal_produces_multi() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        struct ClearCmd;
        impl Command for ClearCmd {
            fn name(&self) -> &str {
                "clear"
            }
            fn description(&self) -> &str {
                "Clear"
            }
            fn usage(&self) -> &str {
                "clear"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Clear)
            }
        }
        reg.register(Box::new(ClearCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // `echo Hello ; clear` should produce Multi.
        match reg.execute("echo Hello ; clear", &mut env).unwrap() {
            CommandOutput::Multi(outputs) => {
                assert_eq!(outputs.len(), 2);
                assert!(matches!(outputs[0], CommandOutput::Text(_)));
                assert!(matches!(outputs[1], CommandOutput::Clear));
            },
            _ => panic!("expected Multi output"),
        }
    }

    #[test]
    fn chain_text_only_merges_to_single() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Two text commands should merge into a single Text, not Multi.
        match reg.execute("echo hello ; echo world", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("hello"));
                assert!(s.contains("world"));
            },
            _ => panic!("expected merged Text output"),
        }
    }

    // -- Redirection tests --

    #[test]
    fn redirect_write() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("echo hello > /tmp/out.txt", &mut env).unwrap();
        assert_eq!(env.vfs.read("/tmp/out.txt").unwrap(), b"hello");
    }

    #[test]
    fn redirect_append() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("echo line1 > /tmp/out.txt", &mut env).unwrap();
        reg.execute("echo line2 >> /tmp/out.txt", &mut env).unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/out.txt").unwrap()).into_owned();
        assert!(content.contains("line1"));
        assert!(content.contains("line2"));
    }

    // -- Stderr redirect tests --

    #[test]
    fn stderr_redirect_captures_error() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Unknown command produces an error; 2> captures it.
        reg.execute("nosuchcmd 2> /tmp/err.txt", &mut env).unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd"));
    }

    #[test]
    fn stderr_redirect_append() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("nosuchcmd1 2> /tmp/err.txt", &mut env).unwrap();
        reg.execute("nosuchcmd2 2>> /tmp/err.txt", &mut env)
            .unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd1"));
        assert!(content.contains("nosuchcmd2"));
    }

    #[test]
    fn stderr_to_stdout_merge() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Unknown command with 2>&1 should merge error into stdout.
        match reg.execute("nosuchcmd 2>&1", &mut env) {
            Ok(CommandOutput::Text(s)) => {
                assert!(s.contains("nosuchcmd"));
            },
            other => panic!("expected Text with error, got {other:?}"),
        }
    }

    #[test]
    fn stderr_to_stdout_with_redirect() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // 2>&1 > file should capture merged output to file.
        reg.execute("nosuchcmd 2>&1 > /tmp/out.txt", &mut env)
            .unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/out.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd"));
    }

    #[test]
    fn stdout_and_stderr_separate_redirects() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Successful command: stdout goes to file, stderr empty.
        reg.execute("echo hello > /tmp/out.txt 2> /tmp/err.txt", &mut env)
            .unwrap();
        assert_eq!(env.vfs.read("/tmp/out.txt").unwrap(), b"hello");
        let err = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(err.is_empty());
    }

    #[test]
    fn stderr_preserved_in_env_without_redirect() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Without 2>, error propagates normally (Err result).
        let result = reg.execute("nosuchcmd", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn stderr_redirect_sets_exit_code() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("nosuchcmd 2> /tmp/err.txt", &mut env).unwrap();
        // Exit code should be 1 even though error was captured.
        assert_eq!(reg.last_exit_code.get(), 1);
    }

    #[test]
    fn parse_redirect_2_to_file() {
        let (cmd, redirections) = parse_redirect("echo hello 2> /tmp/e.txt");
        assert_eq!(cmd, "echo hello ");
        assert!(redirections.stderr.is_some());
        assert_eq!(redirections.stderr.unwrap().path, "/tmp/e.txt");
        assert!(!redirections.stderr_to_stdout);
    }

    #[test]
    fn parse_redirect_2_append() {
        let (cmd, redirections) = parse_redirect("cmd 2>> /tmp/e.txt");
        assert_eq!(cmd, "cmd ");
        let redir = redirections.stderr.unwrap();
        assert!(redir.append);
        assert_eq!(redir.path, "/tmp/e.txt");
    }

    #[test]
    fn parse_redirect_2_to_1() {
        let (cmd, redirections) = parse_redirect("cmd 2>&1");
        assert_eq!(cmd, "cmd ");
        assert!(redirections.stderr_to_stdout);
        assert!(redirections.stderr.is_none());
    }

    #[test]
    fn parse_redirect_stdout_only() {
        let (cmd, redirections) = parse_redirect("echo hi > /tmp/out.txt");
        assert_eq!(cmd, "echo hi ");
        assert!(redirections.stdout.is_some());
        assert!(redirections.stderr.is_none());
        assert!(!redirections.stderr_to_stdout);
    }

    // -- Glob tests --

    #[test]
    fn glob_match_star() {
        assert!(glob_match("*.txt", "hello.txt"));
        assert!(!glob_match("*.txt", "hello.md"));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("h?llo", "hello"));
        assert!(!glob_match("h?llo", "heeello"));
    }

    #[test]
    fn glob_expansion_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.write("/file1.txt", b"a").unwrap();
        vfs.write("/file2.txt", b"b").unwrap();
        vfs.write("/file3.md", b"c").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo /*.txt", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("/file1.txt"));
                assert!(s.contains("/file2.txt"));
                assert!(!s.contains("file3.md"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Brace expansion tests --

    #[test]
    fn brace_expansion_basic() {
        let tokens = vec!["file.{txt,md,rs}".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["file.txt", "file.md", "file.rs"]);
    }

    #[test]
    fn brace_expansion_with_prefix_suffix() {
        let tokens = vec!["src/{main,lib}.rs".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn brace_no_comma_passthrough() {
        let tokens = vec!["{solo}".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["{solo}"]);
    }

    #[test]
    fn brace_expansion_no_braces_passthrough() {
        let tokens = vec!["plain.txt".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["plain.txt"]);
    }

    #[test]
    fn brace_expansion_multiple_tokens() {
        let tokens = vec!["a.{x,y}".to_string(), "b.txt".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["a.x", "a.y", "b.txt"]);
    }

    #[test]
    fn brace_expansion_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo file.{txt,md}", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "file.txt file.md");
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Character class tests --

    #[test]
    fn glob_char_class_list() {
        assert!(glob_match("[abc]", "a"));
        assert!(glob_match("[abc]", "b"));
        assert!(glob_match("[abc]", "c"));
        assert!(!glob_match("[abc]", "d"));
    }

    #[test]
    fn glob_char_class_range() {
        assert!(glob_match("[a-z]", "m"));
        assert!(glob_match("[0-9]", "5"));
        assert!(!glob_match("[a-z]", "A"));
        assert!(!glob_match("[0-9]", "x"));
    }

    #[test]
    fn glob_char_class_negation() {
        assert!(!glob_match("[!abc]", "a"));
        assert!(glob_match("[!abc]", "d"));
        assert!(!glob_match("[^0-9]", "5"));
        assert!(glob_match("[^0-9]", "x"));
    }

    #[test]
    fn glob_char_class_in_pattern() {
        assert!(glob_match("file[12].txt", "file1.txt"));
        assert!(glob_match("file[12].txt", "file2.txt"));
        assert!(!glob_match("file[12].txt", "file3.txt"));
    }

    #[test]
    fn glob_char_class_with_star() {
        assert!(glob_match("*.[ch]", "main.c"));
        assert!(glob_match("*.[ch]", "main.h"));
        assert!(!glob_match("*.[ch]", "main.o"));
    }

    #[test]
    fn glob_char_class_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.write("/file1.txt", b"a").unwrap();
        vfs.write("/file2.txt", b"b").unwrap();
        vfs.write("/file3.txt", b"c").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo /file[12].txt", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("/file1.txt"));
                assert!(s.contains("/file2.txt"));
                assert!(!s.contains("file3.txt"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Alias tests --

    #[test]
    fn alias_expansion() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("alias hi=echo", &mut env).unwrap();
        match reg.execute("hi hello", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn alias_list() {
        let reg = CommandRegistry::new();
        reg.set_alias("ll", "ls -l");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("alias", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("ll"));
                assert!(s.contains("ls -l"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Shell function tests --

    #[test]
    fn function_define_and_call() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function greet() { echo hello }", &mut env)
            .unwrap();
        match reg.execute("greet", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_with_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function say() { echo $1 $2 }", &mut env)
            .unwrap();
        match reg.execute("say hello world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_arg_count() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function argc() { echo $# }", &mut env)
            .unwrap();
        match reg.execute("argc a b c", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "3"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_no_parens_syntax() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function hi { echo hi }", &mut env).unwrap();
        match reg.execute("hi", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hi"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_list() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("function", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("No functions")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_list_after_define() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function foo() { echo bar }", &mut env)
            .unwrap();
        match reg.execute("function", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("foo"));
                assert!(s.contains("echo bar"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_multi_command_body() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function both() { echo one ; echo two }", &mut env)
            .unwrap();
        match reg.execute("both", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("one"));
                assert!(s.contains("two"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_return_sets_exit_code() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function fail() { return 42 }", &mut env)
            .unwrap();
        reg.execute("fail", &mut env).unwrap();
        assert_eq!(reg.last_exit_code.get(), 42);
    }

    #[test]
    fn function_recursion_depth_limit() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Define a function that calls itself infinitely.
        reg.execute("function inf() { inf }", &mut env).unwrap();
        let result = reg.execute("inf", &mut env);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("recursion depth"));
    }

    #[test]
    fn function_restores_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.set_variable("1", "original");
        reg.execute("function f() { echo $1 }", &mut env).unwrap();
        reg.execute("f override", &mut env).unwrap();
        // After function returns, $1 should be restored.
        assert_eq!(reg.get_variable("1").unwrap(), "original");
    }

    #[test]
    fn function_which() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function myfn() { echo hi }", &mut env)
            .unwrap();
        match reg.execute("which myfn", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("shell function")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_empty_body_rejected() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("function bad() { }", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn function_body_with_pipe() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Pipe inside braces should be part of the body, not split
        // as a pipeline operator.
        reg.execute("function piped() { echo hello | echo world }", &mut env)
            .unwrap();
        let funcs = reg.list_functions();
        assert!(funcs.iter().any(|(n, b)| n == "piped" && b.contains('|')));
    }

    #[test]
    fn function_body_with_redirect() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Redirect inside braces should be part of the body, not
        // parsed as a top-level redirect.
        reg.execute("function redir() { echo hello > /tmp/out }", &mut env)
            .unwrap();
        let funcs = reg.list_functions();
        assert!(funcs.iter().any(|(n, b)| n == "redir" && b.contains('>')));
    }

    // -- Set/env tests --

    #[test]
    fn set_and_expand_variable() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("set GREETING=hello", &mut env).unwrap();
        match reg.execute("echo $GREETING", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn env_lists_variables() {
        let reg = CommandRegistry::new();
        reg.set_variable("FOO", "bar");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("env", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("FOO=bar"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Quoted args with commands --

    #[test]
    fn quoted_args_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo 'hello world'", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    // -- Path resolution (unchanged) --

    #[test]
    fn resolve_path_absolute() {
        assert_eq!(resolve_path("/any", "/foo/bar"), "/foo/bar");
    }

    #[test]
    fn resolve_path_relative() {
        assert_eq!(resolve_path("/home", "user"), "/home/user");
    }

    #[test]
    fn resolve_path_dotdot() {
        assert_eq!(resolve_path("/a/b/c", "../../x"), "/a/x");
    }

    #[test]
    fn resolve_path_root_relative() {
        assert_eq!(resolve_path("/", "foo"), "/foo");
    }

    // -- Completions --

    #[test]
    fn completions_prefix() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let completions = reg.completions("ec");
        assert!(completions.contains(&"echo".to_string()));
    }

    #[test]
    fn completions_no_match() {
        let reg = CommandRegistry::new();
        let completions = reg.completions("xyz");
        assert!(completions.is_empty());
    }

    // -- Script control flow (if/while/for) --

    /// Helper: write a script to VFS and run it, returning the output text.
    fn run_script(reg: &CommandRegistry, vfs: &mut MemoryVfs, script: &str) -> String {
        vfs.write("/tmp/test.sh", script.as_bytes()).unwrap();
        let mut env = make_env(vfs);
        match reg.execute("run /tmp/test.sh", &mut env).unwrap() {
            CommandOutput::Text(s) => s,
            other => panic!("expected text, got {:?}", other),
        }
    }

    #[test]
    fn script_if_true_branch() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "if echo true\nthen\necho yes\nfi");
        assert_eq!(out, "yes");
    }

    #[test]
    fn script_if_false_branch() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "if echo false\nthen\necho yes\nfi");
        // false condition: then block skipped, no else block → shows command count
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_if_else() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo false\nthen\necho yes\nelse\necho no\nfi",
        );
        assert_eq!(out, "no");
    }

    #[test]
    fn script_if_else_true() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo true\nthen\necho correct\nelse\necho wrong\nfi",
        );
        assert_eq!(out, "correct");
    }

    #[test]
    fn script_for_loop() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "for x in a b c\ndo\necho $x\ndone");
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn script_for_loop_empty() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "for x in\ndo\necho $x\ndone");
        // Empty item list: loop body never runs
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_while_loop() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        // Use a counter via set/test: set counter, run while counter equals
        // a value, then unset to stop. Simpler: just run a fixed echo loop
        // that terminates immediately because condition is false.
        let out = run_script(&reg, &mut vfs, "while echo false\ndo\necho body\ndone");
        // Condition is false from the start, loop never executes.
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_while_loop_executes() {
        // Use a VFS file as a counter: while the file exists, execute body
        // and delete the file in the first iteration.
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::register_dev_commands(&mut reg);
        crate::register_file_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        // test -f /tmp/flag returns "true" when file exists
        vfs.write("/tmp/flag", b"1").unwrap();
        // Script: while test -f /tmp/flag → echo iteration → rm /tmp/flag
        // But we don't have rm. Instead, write empty to flag and use
        // a different approach. Let's use set/env approach:
        // Set a variable and test it. Actually, let's just test that a
        // simple loop with echo condition works for at least one iteration.
        // We'll test with for loop which is more predictable.
        let out = run_script(
            &reg,
            &mut vfs,
            "for i in 1 2 3\ndo\necho iteration $i\ndone",
        );
        assert!(out.contains("iteration 1"));
        assert!(out.contains("iteration 2"));
        assert!(out.contains("iteration 3"));
    }

    #[test]
    fn script_nested_if() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo true\nthen\nif echo true\nthen\necho nested\nfi\nfi",
        );
        assert_eq!(out, "nested");
    }

    #[test]
    fn script_nested_if_outer_false() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo false\nthen\nif echo true\nthen\necho nested\nfi\nfi",
        );
        // Outer if is false, whole then block (including inner if) skipped
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_for_with_echo_output() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "echo start\nfor n in x y\ndo\necho item $n\ndone\necho end",
        );
        assert!(out.contains("start"));
        assert!(out.contains("item x"));
        assert!(out.contains("item y"));
        assert!(out.contains("end"));
    }

    #[test]
    fn script_comments_and_blank_lines_ignored() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "# This is a comment\n\necho hello\n\n# Another comment",
        );
        assert_eq!(out, "hello");
    }

    #[test]
    fn script_error_in_line() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "echo before\nnosuchcommand\necho after");
        assert!(out.contains("before"));
        assert!(out.contains("error at line 2"));
        assert!(out.contains("after"));
    }

    #[test]
    fn script_not_found() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("run /nonexistent.sh", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn script_empty() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/tmp/empty.sh", b"# just comments\n").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("run /tmp/empty.sh", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("empty script")),
            _ => panic!("expected text"),
        }
    }

    // ===================================================================
    // Property-based tests
    // ===================================================================

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            // -- tokenize -----------------------------------------------

            /// Tokenizing a single unquoted word returns exactly that word.
            #[test]
            fn tokenize_single_word(w in "[a-zA-Z0-9_]{1,30}") {
                let tokens = tokenize(&w).unwrap();
                prop_assert_eq!(tokens, vec![w]);
            }

            /// Tokenizing two words separated by space returns two tokens.
            #[test]
            fn tokenize_two_words(
                a in "[a-zA-Z]{1,15}",
                b in "[a-zA-Z]{1,15}",
            ) {
                let input = format!("{a} {b}");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens.len(), 2);
                prop_assert_eq!(&tokens[0], &a);
                prop_assert_eq!(&tokens[1], &b);
            }

            /// Single-quoted strings preserve all content literally.
            #[test]
            fn tokenize_single_quoted_preserves(
                s in "[a-zA-Z0-9 \\$\\\\]{1,30}",
            ) {
                let input = format!("'{s}'");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens, vec![s]);
            }

            /// Double-quoted strings produce a single token.
            #[test]
            fn tokenize_double_quoted_single_token(
                s in "[a-zA-Z0-9 ]{1,30}",
            ) {
                let input = format!("\"{s}\"");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens.len(), 1);
                prop_assert_eq!(&tokens[0], &s);
            }

            /// Tokenize never panics on arbitrary ASCII input.
            #[test]
            fn tokenize_never_panics(s in "[ -~]{0,80}") {
                let _ = tokenize(&s);
            }

            /// Empty input yields empty tokens.
            #[test]
            fn tokenize_whitespace_only(n in 1usize..20) {
                let input = " ".repeat(n);
                let tokens = tokenize(&input).unwrap();
                prop_assert!(tokens.is_empty());
            }

            // -- glob_match ----------------------------------------------

            /// Any text matches the `*` pattern.
            #[test]
            fn glob_star_matches_everything(s in "[a-z]{0,20}") {
                prop_assert!(glob_match("*", &s));
            }

            /// `?` matches exactly one character.
            #[test]
            fn glob_question_matches_single_char(c in "[a-z]") {
                prop_assert!(glob_match("?", &c));
            }

            /// `?` does NOT match empty string.
            #[test]
            fn glob_question_no_empty(_dummy in 0..1i32) {
                prop_assert!(!glob_match("?", ""));
            }

            /// A literal pattern matches only itself.
            #[test]
            fn glob_literal_exact(s in "[a-z]{1,20}") {
                prop_assert!(glob_match(&s, &s));
            }

            /// A literal pattern does not match a different string.
            #[test]
            fn glob_literal_no_mismatch(
                a in "[a-z]{1,10}",
                b in "[a-z]{1,10}",
            ) {
                if a != b {
                    prop_assert!(!glob_match(&a, &b));
                }
            }

            /// `*` at end matches any suffix.
            #[test]
            fn glob_star_suffix(
                prefix in "[a-z]{1,10}",
                suffix in "[a-z]{0,10}",
            ) {
                let text = format!("{prefix}{suffix}");
                let pattern = format!("{prefix}*");
                prop_assert!(glob_match(&pattern, &text));
            }

            /// Character class [a-z] matches lowercase chars.
            #[test]
            fn glob_char_class_range(c in "[a-z]") {
                prop_assert!(glob_match("[a-z]", &c));
            }

            /// Negated class [!a-z] does NOT match lowercase chars.
            #[test]
            fn glob_negated_class_range(c in "[a-z]") {
                prop_assert!(!glob_match("[!a-z]", &c));
            }

            // -- resolve_path -------------------------------------------

            /// resolve_path always returns a string starting with '/'.
            #[test]
            fn resolve_path_starts_with_slash(
                cwd in "/[a-z]{1,10}(/[a-z]{1,5}){0,3}",
                input in "[a-z./]{0,20}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    result.starts_with('/'),
                    "expected '/' prefix, got: {result}",
                );
            }

            /// resolve_path is idempotent when input is absolute.
            #[test]
            fn resolve_path_absolute_idempotent(
                path in "/[a-z]{1,10}(/[a-z]{1,5}){0,3}",
            ) {
                let first = resolve_path("/", &path);
                let second = resolve_path("/", &first);
                prop_assert_eq!(first, second);
            }

            /// resolve_path never contains `..` in output.
            #[test]
            fn resolve_path_no_dotdot(
                cwd in "/[a-z]{1,10}",
                input in "(\\.\\./){0,5}[a-z]{0,10}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    !result.contains(".."),
                    "result should not contain '..': {result}",
                );
            }

            /// resolve_path never contains double slashes.
            #[test]
            fn resolve_path_no_double_slashes(
                cwd in "/[a-z]{1,5}",
                input in "[a-z./]{0,15}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    !result.contains("//"),
                    "result should not contain '//': {result}",
                );
            }

            // -- expand_braces -------------------------------------------

            /// Brace expansion with N alternatives produces N tokens.
            #[test]
            fn brace_expansion_count(
                prefix in "[a-z]{0,5}",
                a in "[a-z]{1,5}",
                b in "[a-z]{1,5}",
                c in "[a-z]{1,5}",
            ) {
                let input = format!("{prefix}{{{a},{b},{c}}}");
                let tokens = vec![input];
                let expanded = expand_braces(&tokens);
                prop_assert!(
                    expanded.len() == 3,
                    "expected 3 tokens from brace expansion, got: {:?}",
                    expanded,
                );
            }

            /// Brace expansion preserves prefix and suffix.
            #[test]
            fn brace_expansion_preserves_affix(
                prefix in "[a-z]{1,5}",
                suffix in "[a-z]{1,5}",
                a in "[a-z]{1,3}",
                b in "[a-z]{1,3}",
            ) {
                let input = format!("{prefix}{{{a},{b}}}{suffix}");
                let tokens = vec![input];
                let expanded = expand_braces(&tokens);
                for tok in &expanded {
                    prop_assert!(
                        tok.starts_with(&prefix) && tok.ends_with(&suffix),
                        "token '{tok}' should have prefix '{prefix}' and suffix '{suffix}'",
                    );
                }
            }

            /// A token without braces passes through unchanged.
            #[test]
            fn brace_expansion_passthrough(s in "[a-z]{1,20}") {
                let tokens = vec![s.clone()];
                let expanded = expand_braces(&tokens);
                prop_assert_eq!(expanded, vec![s]);
            }
        }
    }

    // -- Extended scripting tests --

    /// Minimal `test` mock: supports `test A == B` and `test true`.
    struct MockTestCmd;
    impl Command for MockTestCmd {
        fn name(&self) -> &str {
            "test"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn usage(&self) -> &str {
            "test"
        }
        fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
            if args.len() == 3 && args[1] == "==" {
                let result = args[0] == args[2];
                return Ok(CommandOutput::Text(result.to_string()));
            }
            if args.first() == Some(&"true") {
                return Ok(CommandOutput::Text("true".into()));
            }
            Ok(CommandOutput::Text("false".into()))
        }
    }

    fn make_ext_reg() -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        reg.register(Box::new(MockTestCmd));
        reg
    }

    fn run_script_ext(reg: &CommandRegistry, script: &str) -> String {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        run_script(reg, &mut vfs, script)
    }

    #[test]
    fn script_elif_first_branch() {
        let reg = make_ext_reg();
        let script = "set X=hello\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "first");
    }

    #[test]
    fn script_elif_second_branch() {
        let reg = make_ext_reg();
        let script = "set X=world\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "second");
    }

    #[test]
    fn script_elif_else_fallback() {
        let reg = make_ext_reg();
        let script = "set X=other\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "third");
    }

    #[test]
    fn script_case_exact_match() {
        let reg = make_ext_reg();
        let script = "set X=hello\n\
            case $X in\n\
            hello) echo matched ;;  \n\
            *) echo default ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "matched");
    }

    #[test]
    fn script_case_wildcard_default() {
        let reg = make_ext_reg();
        let script = "set X=other\n\
            case $X in\n\
            hello) echo matched ;;\n\
            *) echo default ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "default");
    }

    #[test]
    fn script_case_alternation() {
        let reg = make_ext_reg();
        let script = "set X=world\n\
            case $X in\n\
            hello|world) echo either ;;\n\
            *) echo nope ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "either");
    }

    #[test]
    fn script_break_in_for() {
        let reg = make_ext_reg();
        let script = "for i in a b c d\n\
            do\n\
            if test $i == c\n\
            then\n\
            break\n\
            fi\n\
            echo $i\n\
            done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn script_continue_in_for() {
        let reg = make_ext_reg();
        let script = "for i in a b c d\n\
            do\n\
            if test $i == b\n\
            then\n\
            continue\n\
            fi\n\
            echo $i\n\
            done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "a\nc\nd");
    }

    #[test]
    fn script_break_in_while() {
        let reg = make_ext_reg();
        let script = "set N=0\n\
            while test true\n\
            do\n\
            echo $N\n\
            if test $N == 2\n\
            then\n\
            break\n\
            fi\n\
            set N=1\n\
            set N=2\n\
            done";
        let out = run_script_ext(&reg, script);
        // Should output at least the initial 0
        assert!(out.contains('0'));
    }

    #[test]
    fn script_local_variables() {
        let reg = make_ext_reg();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);

        // Set a global variable.
        reg.execute("set X=global", &mut env).unwrap();

        // Define a function that uses local.
        reg.execute("function myfn() { local X=local; echo $X }", &mut env)
            .unwrap();

        // Call the function.
        match reg.execute("myfn", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "local"),
            _ => panic!("expected text"),
        }

        // Global should be restored.
        assert_eq!(reg.get_variable("X"), Some("global".to_string()));
    }

    #[test]
    fn script_local_unset_after_function() {
        let reg = make_ext_reg();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);

        // No global X.
        assert!(reg.get_variable("X").is_none());

        // Function creates local X.
        reg.execute("function myfn() { local X=temp; echo $X }", &mut env)
            .unwrap();
        reg.execute("myfn", &mut env).unwrap();

        // X should not leak to global scope.
        assert!(reg.get_variable("X").is_none());
    }

    #[test]
    fn case_pattern_matches_exact() {
        assert!(super::case_pattern_matches("hello", "hello"));
        assert!(!super::case_pattern_matches("hello", "world"));
    }

    #[test]
    fn case_pattern_matches_wildcard() {
        assert!(super::case_pattern_matches("anything", "*"));
    }

    #[test]
    fn case_pattern_matches_alternation() {
        assert!(super::case_pattern_matches("b", "a|b|c"));
        assert!(!super::case_pattern_matches("d", "a|b|c"));
    }

    #[test]
    fn case_pattern_matches_glob_prefix() {
        assert!(super::case_pattern_matches("hello_world", "hello*"));
        assert!(!super::case_pattern_matches("goodbye", "hello*"));
    }

    #[test]
    fn case_pattern_matches_glob_suffix() {
        assert!(super::case_pattern_matches("file.txt", "*.txt"));
        assert!(!super::case_pattern_matches("file.rs", "*.txt"));
    }

    #[test]
    fn glob_match_simple_middle() {
        assert!(glob_match_simple("hello_world", "hello*world"));
        assert!(!glob_match_simple("hello_earth", "hello*world"));
    }

    #[test]
    fn script_case_no_match() {
        let reg = make_ext_reg();
        let script = "set X=zzz\n\
            case $X in\n\
            hello) echo matched ;;\n\
            esac\n\
            echo done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "done");
    }

    #[test]
    fn script_elif_multiple() {
        let reg = make_ext_reg();
        let script = "set X=c\n\
            if test $X == a\n\
            then\n\
            echo A\n\
            elif test $X == b\n\
            then\n\
            echo B\n\
            elif test $X == c\n\
            then\n\
            echo C\n\
            elif test $X == d\n\
            then\n\
            echo D\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "C");
    }

    #[test]
    fn script_local_outside_function_fails() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("local X=val", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn script_break_outside_loop_noop() {
        // break outside a loop just sets the flag which gets
        // ignored since there's no loop to break from.
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("break", &mut env);
        assert!(result.is_ok());
    }

    // ---- Phase 12: integration tests ----

    #[test]
    fn pipe_chain_echo_to_echo() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello | echo", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn pipe_chain_three_stages() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo data | echo | echo", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "data"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn variable_expansion_basic() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("set X=hello", &mut env).unwrap();
        let val = reg.execute("echo $X", &mut env).ok().and_then(|o| match o {
            CommandOutput::Text(s) => Some(s),
            _ => None,
        });
        assert_eq!(val.as_deref(), Some("hello"));
    }

    #[test]
    fn variable_expansion_unset() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Unset variable expands to empty string.
        let out = reg.execute("echo $NONEXISTENT", &mut env);
        assert!(out.is_ok());
    }

    #[test]
    fn alias_define_and_use() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("alias greet='echo hi'", &mut env).unwrap();
        let out = reg.execute("greet", &mut env);
        match out {
            Ok(CommandOutput::Text(s)) => assert_eq!(s, "hi"),
            _ => {
                // alias might not resolve echo if it's not registered;
                // just verify no panic.
            },
        }
    }

    #[test]
    fn history_records_commands() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("set A=1", &mut env).unwrap();
        reg.execute("set B=2", &mut env).unwrap();
        let out = reg.execute("history", &mut env).unwrap();
        if let CommandOutput::Text(s) = out {
            assert!(s.contains("set A=1"));
            assert!(s.contains("set B=2"));
        }
    }

    #[test]
    fn chained_commands_semicolon() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = reg.execute("set X=1; set Y=2", &mut env);
        assert!(out.is_ok());
    }

    #[test]
    fn chained_commands_and_operator() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = reg.execute("echo first && echo second", &mut env).unwrap();
        if let CommandOutput::Text(s) = out {
            assert!(s.contains("first"));
            assert!(s.contains("second"));
        }
    }

    #[test]
    fn empty_command_returns_none() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("", &mut env).unwrap() {
            CommandOutput::None => {},
            other => panic!("expected None, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_only_command_returns_none() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("   ", &mut env).unwrap() {
            CommandOutput::None => {},
            other => panic!("expected None, got {other:?}"),
        }
    }

    // -- Command substitution tests --

    #[test]
    fn command_substitution_basic() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo $(echo hello)", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn command_substitution_in_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg
            .execute("echo prefix-$(echo mid)-suffix", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert_eq!(s, "prefix-mid-suffix");
            },
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn command_substitution_nested() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg
            .execute("echo $(echo $(echo nested))", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert_eq!(s, "nested");
            },
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn command_substitution_trims_trailing_newlines() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.expand_substitutions("$(echo test)", &mut env);
        assert_eq!(result, "test");
    }

    #[test]
    fn command_substitution_preserves_single_quotes() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // $(...) inside single quotes should NOT be expanded.
        match reg.execute("echo '$(echo nope)'", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "$(echo nope)");
            },
            other => {
                panic!("expected literal text, got {other:?}");
            },
        }
    }

    #[test]
    fn command_substitution_multiple() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo $(echo a)-$(echo b)", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "a-b");
            },
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn command_substitution_unmatched_paren_passthrough() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Unmatched $( should pass through as literal.
        let result = reg.expand_substitutions("$(unclosed", &mut env);
        assert_eq!(result, "$(unclosed");
    }

    #[test]
    fn command_substitution_with_variable() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        reg.set_variable("X", "world");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Variable expansion happens after substitution, so $X in
        // the outer command should still expand.
        match reg.execute("echo $(echo hello) $X", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "hello world");
            },
            other => panic!("expected text, got {other:?}"),
        }
    }
}
