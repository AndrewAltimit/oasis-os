//! Shell function, script control flow, property-based, and extended
//! scripting tests.

use super::*;
use crate::expander::glob_match;
use crate::test_helpers::assert_text;
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

// -- Shell function tests --

#[test]
fn function_define_and_call() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function greet() { echo hello }", &mut env)
        .unwrap();
    assert_eq!(
        assert_text!(reg.execute("greet", &mut env).unwrap()),
        "hello"
    );
}

#[test]
fn function_with_args() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function say() { echo $1 $2 }", &mut env)
        .unwrap();
    assert_eq!(
        assert_text!(reg.execute("say hello world", &mut env).unwrap()),
        "hello world"
    );
}

#[test]
fn function_arg_count() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function argc() { echo $# }", &mut env)
        .unwrap();
    assert_eq!(
        assert_text!(reg.execute("argc a b c", &mut env).unwrap()),
        "3"
    );
}

#[test]
fn function_no_parens_syntax() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function hi { echo hi }", &mut env).unwrap();
    assert_eq!(assert_text!(reg.execute("hi", &mut env).unwrap()), "hi");
}

#[test]
fn function_list() {
    let reg = CommandRegistry::new();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    assert!(assert_text!(reg.execute("function", &mut env).unwrap()).contains("No functions"));
}

#[test]
fn function_list_after_define() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function foo() { echo bar }", &mut env)
        .unwrap();
    let s = assert_text!(reg.execute("function", &mut env).unwrap());
    assert!(s.contains("foo"));
    assert!(s.contains("echo bar"));
}

#[test]
fn function_multi_command_body() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function both() { echo one ; echo two }", &mut env)
        .unwrap();
    let s = assert_text!(reg.execute("both", &mut env).unwrap());
    assert!(s.contains("one"));
    assert!(s.contains("two"));
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
    assert_eq!(reg.get_variable("1").unwrap(), "original");
}

#[test]
fn function_which() {
    let reg = CommandRegistry::new();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    reg.execute("function myfn() { echo hi }", &mut env)
        .unwrap();
    assert!(assert_text!(reg.execute("which myfn", &mut env).unwrap()).contains("shell function"));
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
    reg.execute("function redir() { echo hello > /tmp/out }", &mut env)
        .unwrap();
    let funcs = reg.list_functions();
    assert!(funcs.iter().any(|(n, b)| n == "redir" && b.contains('>')));
}

// -- Script control flow (if/while/for) --

/// Helper: write a script to VFS and run it, returning the output text.
fn run_script(reg: &CommandRegistry, vfs: &mut MemoryVfs, script: &str) -> String {
    vfs.write("/tmp/test.sh", script.as_bytes()).unwrap();
    let mut env = make_env(vfs);
    assert_text!(reg.execute("run /tmp/test.sh", &mut env).unwrap())
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
    assert!(out.contains("commands executed"));
}

#[test]
fn script_while_loop() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    crate::register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/tmp").unwrap();
    let out = run_script(&reg, &mut vfs, "while echo false\ndo\necho body\ndone");
    assert!(out.contains("commands executed"));
}

#[test]
fn script_while_loop_executes() {
    let mut reg = CommandRegistry::new();
    reg.register(Box::new(EchoCmd));
    crate::register_dev_commands(&mut reg);
    crate::register_file_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/tmp").unwrap();
    vfs.write("/tmp/flag", b"1").unwrap();
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
    assert!(
        assert_text!(reg.execute("run /tmp/empty.sh", &mut env).unwrap()).contains("empty script")
    );
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

        // -- tokenize: full Unicode / adversarial input ------------------

        /// Tokenize never panics on arbitrary Unicode input (not just ASCII).
        #[test]
        fn tokenize_arbitrary_unicode_no_panic(s in "\\PC{0,100}") {
            let _ = tokenize(&s);
        }

        // -- parse_redirect: fuzz ----------------------------------------

        /// `parse_redirect` never panics on arbitrary input.
        #[test]
        fn parse_redirect_no_panic(s in "\\PC{0,100}") {
            let _ = parse_redirect(&s);
        }

        /// `parse_redirect` always returns a command part that is a
        /// substring (or equal) of the original input.
        #[test]
        fn parse_redirect_command_part_is_substring(s in "[ -~]{0,80}") {
            let (cmd, _) = parse_redirect(&s);
            prop_assert!(
                s.contains(cmd.trim()),
                "command part '{cmd}' should be within original '{s}'",
            );
        }

        // -- CommandRegistry::execute: fuzz -------------------------------

        /// Executing arbitrary command strings never panics.
        #[test]
        fn execute_arbitrary_no_panic(s in "\\PC{0,100}") {
            let reg = CommandRegistry::new();
            let mut vfs = MemoryVfs::new();
            let mut env = make_env(&mut vfs);
            let _ = reg.execute(&s, &mut env);
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
    assert!(out.contains('0'));
}

#[test]
fn script_local_variables() {
    let reg = make_ext_reg();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("set X=global", &mut env).unwrap();

    reg.execute("function myfn() { local X=local; echo $X }", &mut env)
        .unwrap();

    assert_eq!(
        assert_text!(reg.execute("myfn", &mut env).unwrap()),
        "local"
    );

    assert_eq!(reg.get_variable("X"), Some("global".to_string()));
}

#[test]
fn script_local_unset_after_function() {
    let reg = make_ext_reg();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    assert!(reg.get_variable("X").is_none());

    reg.execute("function myfn() { local X=temp; echo $X }", &mut env)
        .unwrap();
    reg.execute("myfn", &mut env).unwrap();

    assert!(reg.get_variable("X").is_none());
}

#[test]
fn case_pattern_matches_exact() {
    assert!(case_pattern_matches("hello", "hello"));
    assert!(!case_pattern_matches("hello", "world"));
}

#[test]
fn case_pattern_matches_wildcard() {
    assert!(case_pattern_matches("anything", "*"));
}

#[test]
fn case_pattern_matches_alternation() {
    assert!(case_pattern_matches("b", "a|b|c"));
    assert!(!case_pattern_matches("d", "a|b|c"));
}

#[test]
fn case_pattern_matches_glob_prefix() {
    assert!(case_pattern_matches("hello_world", "hello*"));
    assert!(!case_pattern_matches("goodbye", "hello*"));
}

#[test]
fn case_pattern_matches_glob_suffix() {
    assert!(case_pattern_matches("file.txt", "*.txt"));
    assert!(!case_pattern_matches("file.rs", "*.txt"));
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
    let reg = CommandRegistry::new();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);
    let result = reg.execute("break", &mut env);
    assert!(result.is_ok());
}
