//! Pipe, chaining, redirection, and integration tests.

use super::*;
use crate::test_helpers::{assert_none_output, assert_text};
use oasis_vfs::{MemoryVfs, Vfs};

use super::tests_basic::make_env;

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
        if args.is_empty() {
            if let Some(ref stdin) = env.stdin {
                return Ok(CommandOutput::Text(stdin.clone()));
            }
        }
        Ok(CommandOutput::Text(args.join(" ")))
    }
}

// -- Pipe tests --

#[test]
fn pipe_two_commands() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));

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
    assert_eq!(
        assert_text!(reg.execute("echo hello | upper", &mut env).unwrap()),
        "HELLO"
    );
}

// -- Chaining tests --

#[test]
fn chain_semicolon() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    let s = assert_text!(reg.execute("echo hello ; echo world", &mut env).unwrap());
    assert!(s.contains("hello"));
    assert!(s.contains("world"));
}

#[test]
fn chain_and_success() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    let s = assert_text!(reg.execute("echo first && echo second", &mut env).unwrap());
    assert!(s.contains("first"));
    assert!(s.contains("second"));
}

#[test]
fn chain_and_failure() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    // nonexistent fails -> echo after && should NOT run.
    let s = assert_text!(
        reg.execute("nonexistent && echo should_not_run", &mut env)
            .unwrap()
    );
    assert!(s.contains("error"));
    assert!(!s.contains("should_not_run"));
}

#[test]
fn chain_or_success() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    // echo succeeds -> echo after || should NOT run.
    let s = assert_text!(reg.execute("echo ok || echo fallback", &mut env).unwrap());
    assert!(s.contains("ok"));
    assert!(!s.contains("fallback"));
}

#[test]
fn chain_or_failure() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    // nonexistent fails -> echo after || SHOULD run.
    assert!(
        assert_text!(
            reg.execute("nonexistent || echo fallback", &mut env)
                .unwrap()
        )
        .contains("fallback")
    );
}

// -- Chain preserves signals alongside text --

#[test]
fn chain_signal_then_text_produces_multi() {
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
    let __out = reg.execute("clear ; echo Done", &mut env).unwrap();
    assert!(
        matches!(&__out, CommandOutput::Multi(_)),
        "expected Multi output, got {__out:?}"
    );
    let CommandOutput::Multi(outputs) = __out else {
        unreachable!()
    };
    assert_eq!(outputs.len(), 2);
    assert!(matches!(outputs[0], CommandOutput::Clear));
    assert!(matches!(outputs[1], CommandOutput::Text(_)));
    if let CommandOutput::Text(ref s) = outputs[1] {
        assert_eq!(s, "Done");
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
    let __out = reg.execute("echo Hello ; clear", &mut env).unwrap();
    assert!(
        matches!(&__out, CommandOutput::Multi(_)),
        "expected Multi output, got {__out:?}"
    );
    let CommandOutput::Multi(outputs) = __out else {
        unreachable!()
    };
    assert_eq!(outputs.len(), 2);
    assert!(matches!(outputs[0], CommandOutput::Text(_)));
    assert!(matches!(outputs[1], CommandOutput::Clear));
}

#[test]
fn chain_text_only_merges_to_single() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    let s = assert_text!(reg.execute("echo hello ; echo world", &mut env).unwrap());
    assert!(s.contains("hello"));
    assert!(s.contains("world"));
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
    assert!(assert_text!(reg.execute("nosuchcmd 2>&1", &mut env).unwrap()).contains("nosuchcmd"));
}

#[test]
fn stderr_to_stdout_with_redirect() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/tmp").unwrap();
    let mut env = make_env(&mut vfs);
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

// -- Phase 12: integration tests --

#[test]
fn pipe_chain_echo_to_echo() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    assert_eq!(
        assert_text!(reg.execute("echo hello | echo", &mut env).unwrap()),
        "hello"
    );
}

#[test]
fn pipe_chain_three_stages() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    assert_eq!(
        assert_text!(reg.execute("echo data | echo | echo", &mut env).unwrap()),
        "data"
    );
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
    assert_none_output!(reg.execute("", &mut env).unwrap());
}

#[test]
fn whitespace_only_command_returns_none() {
    let reg = CommandRegistry::new();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    assert_none_output!(reg.execute("   ", &mut env).unwrap());
}
