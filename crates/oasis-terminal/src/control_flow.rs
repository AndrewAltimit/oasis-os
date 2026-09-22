//! Inline shell control flow (`if`, `while`, `until`, `for`, `case`).
//!
//! This module is a thin public facade: parsing and execution live in the
//! crate's script engine (`script.rs`), which is shared with `run <path>`
//! script files so one-liners and scripts support exactly the same syntax
//! (nested blocks, `elif`, `until`, `case`, quote-aware `;` splitting).
//! [`CommandRegistry::execute`] routes compound lines there automatically;
//! call [`parse_and_execute`] directly only to bypass the registry's
//! history and job handling.

use crate::interpreter::{CommandOutput, CommandRegistry, Environment};
use oasis_types::error::Result;

/// Execute `input` if it contains a compound command.
///
/// Recognizes (with `;` or newlines as separators, nested arbitrarily):
/// - `if COND; then BODY; [elif COND; then BODY;]... [else BODY;] fi`
/// - `for VAR in WORDS; do BODY; done`
/// - `while COND; do BODY; done` / `until COND; do BODY; done`
/// - `case WORD in PATTERN) BODY ;; ... esac`
///
/// Returns `None` if the input contains no compound command.
pub fn parse_and_execute(
    input: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Option<Result<CommandOutput>> {
    if crate::script::is_compound(input) {
        Some(registry.execute_compound(input.trim(), env))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::Command;
    use oasis_vfs::MemoryVfs;

    // Minimal echo command for tests.
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

    fn make_reg() -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::dev_commands::register_dev_commands(&mut reg);
        reg
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

    fn run(input: &str) -> String {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);
        match parse_and_execute(input, &reg, &mut env) {
            Some(Ok(CommandOutput::Text(t))) => t,
            Some(Ok(CommandOutput::None)) => String::new(),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn if_then_fi_true_branch() {
        assert_eq!(run("if test 1 -eq 1; then echo yes; fi"), "yes");
    }

    #[test]
    fn if_then_else_fi_false_branch() {
        assert_eq!(
            run("if test 1 -eq 2; then echo yes; else echo no; fi"),
            "no"
        );
    }

    #[test]
    fn for_loop_collects_every_iteration() {
        assert_eq!(run("for x in a b c; do echo $x; done"), "a\nb\nc");
    }

    #[test]
    fn non_control_flow_returns_none() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);
        assert!(parse_and_execute("echo hello", &reg, &mut env).is_none());
    }

    #[test]
    fn if_missing_fi_errors() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);
        let result = parse_and_execute("if true; then echo yes", &reg, &mut env);
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn for_missing_done_errors() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);
        let result = parse_and_execute("for x in a b; do echo $x", &reg, &mut env);
        assert!(result.unwrap().is_err());
    }
}
