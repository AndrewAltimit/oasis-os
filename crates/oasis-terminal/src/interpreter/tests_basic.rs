//! Basic command, tokenizer, variable, history, alias, glob, and path tests.

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

pub(super) fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
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

// -- Basic command tests --

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
        CommandOutput::Signal(CommandSignal::ListenToggle { port: 8080 }),
        CommandOutput::Signal(CommandSignal::RemoteConnect {
            address: "1.2.3.4".into(),
            port: 22,
            psk: Some("key".into()),
        }),
        CommandOutput::Signal(CommandSignal::BrowserSandbox { enable: true }),
        CommandOutput::Signal(CommandSignal::SkinSwap { name: "xp".into() }),
        CommandOutput::Signal(CommandSignal::FtpToggle {
            port: 2121,
            password: None,
        }),
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
    match reg
        .execute(
            "echo \u{3053}\u{3093}\u{306b}\u{3061}\u{306f} \u{4e16}\u{754c}",
            &mut env,
        )
        .unwrap()
    {
        CommandOutput::Text(s) => {
            assert!(s.contains("\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}"));
            assert!(s.contains("\u{4e16}\u{754c}"));
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

// -- Path resolution --

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

#[test]
fn glob_match_simple_middle() {
    assert!(glob_match_simple("hello_world", "hello*world"));
    assert!(!glob_match_simple("hello_earth", "hello*world"));
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
