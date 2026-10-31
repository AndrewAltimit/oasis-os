//! Shell semantics: exit-status conditions, `!`, `$?`, word-level
//! expansion of `$(...)` / `$VAR`, `&` anywhere, `xargs` and `#` comments.

use super::*;
use crate::test_helpers::make_env;
use oasis_vfs::{MemoryVfs, Vfs};

fn shell() -> (CommandRegistry, MemoryVfs) {
    let mut reg = CommandRegistry::new();
    crate::register_builtins(&mut reg);
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/tmp").unwrap();
    (reg, vfs)
}

/// Run `line` and return its text (errors as `ERR: ...`).
fn run(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> String {
    let mut env = make_env(vfs);
    match reg.execute(line, &mut env) {
        Ok(out) => crate::pipeline::output_to_text(&out),
        Err(e) => format!("ERR: {e}"),
    }
}

fn status(reg: &CommandRegistry) -> String {
    reg.get_variable("?").unwrap_or_default()
}

// -- Exit status drives conditions --

#[test]
fn if_uses_exit_status_not_printed_text() {
    let (reg, mut vfs) = shell();
    // `echo false` succeeds: its text no longer makes the condition false.
    assert_eq!(
        run(&reg, &mut vfs, "if echo false; then echo yes; fi"),
        "false\nyes"
    );
    assert_eq!(
        run(&reg, &mut vfs, "if false; then echo a; else echo b; fi"),
        "b"
    );
    assert_eq!(
        run(&reg, &mut vfs, "if test -d /tmp; then echo dir; fi"),
        "dir"
    );
    assert_eq!(
        run(
            &reg,
            &mut vfs,
            "if [ x = y ]; then echo eq; elif true; then echo ne; fi"
        ),
        "ne"
    );
    // A failing command (an `Err`) is a false condition.
    assert_eq!(
        run(
            &reg,
            &mut vfs,
            "if cat /nope 2>/tmp/err; then echo y; else echo n; fi"
        ),
        "n"
    );
}

#[test]
fn grep_and_diff_report_status() {
    let (reg, mut vfs) = shell();
    vfs.write("/tmp/a", b"alpha\nbeta").unwrap();
    vfs.write("/tmp/b", b"alpha").unwrap();
    assert_eq!(
        run(&reg, &mut vfs, "if grep beta /tmp/a; then echo found; fi"),
        "beta\nfound"
    );
    run(&reg, &mut vfs, "grep gamma /tmp/a");
    assert_eq!(status(&reg), "1");
    run(&reg, &mut vfs, "diff /tmp/a /tmp/b");
    assert_eq!(status(&reg), "1");
    run(&reg, &mut vfs, "diff /tmp/a /tmp/a");
    assert_eq!(status(&reg), "0");
}

#[test]
fn while_loop_counts_with_test_and_expr() {
    let (reg, mut vfs) = shell();
    let out = run(
        &reg,
        &mut vfs,
        "set i=0; while test $i -lt 3; do echo $i; set i=$(expr $i + 1); done",
    );
    assert_eq!(out, "0\n1\n2");
    let out = run(
        &reg,
        &mut vfs,
        "set i=0; until [ $i -eq 2 ]; do set i=$(expr $i + 1); done; echo $i",
    );
    assert_eq!(out, "2");
}

#[test]
fn bang_negates_and_sets_status() {
    let (reg, mut vfs) = shell();
    assert_eq!(run(&reg, &mut vfs, "! false && echo ok"), "ok");
    assert_eq!(run(&reg, &mut vfs, "! true || echo neg"), "neg");
    run(&reg, &mut vfs, "! true");
    assert_eq!(status(&reg), "1");
    run(&reg, &mut vfs, "! false");
    assert_eq!(status(&reg), "0");
    // Negating a failing command yields success, with its error shown.
    let out = run(&reg, &mut vfs, "! cat /missing");
    assert!(out.starts_with("error:"), "{out}");
    assert_eq!(status(&reg), "0");
    assert_eq!(
        run(
            &reg,
            &mut vfs,
            "if ! test -f /tmp/zzz; then echo absent; fi"
        ),
        "absent"
    );
}

#[test]
fn dollar_question_tracks_each_command() {
    let (reg, mut vfs) = shell();
    assert_eq!(run(&reg, &mut vfs, "false; echo $?"), "1");
    assert_eq!(run(&reg, &mut vfs, "true; echo $?"), "0");
    assert_eq!(run(&reg, &mut vfs, "test 1 -gt 2; echo $?"), "1");
    assert_eq!(
        run(&reg, &mut vfs, "if false; then echo x; fi; echo $?"),
        "0"
    );
}

// -- Expansion results are words, not syntax --

#[test]
fn substitution_output_is_not_reparsed() {
    let (reg, mut vfs) = shell();
    reg.set_variable("HOME", "/home");
    assert_eq!(run(&reg, &mut vfs, "echo $(echo '$HOME')"), "$HOME");
    assert_eq!(run(&reg, &mut vfs, "echo $(echo '\"q\"')"), "\"q\"");
    assert_eq!(run(&reg, &mut vfs, "echo $(echo 'a;b')"), "a;b");
    assert_eq!(run(&reg, &mut vfs, "echo $(echo 'x\\y')"), "x\\y");
    // Variables holding syntax are inert too.
    reg.set_variable("V", "it's \"fine\" $HOME");
    assert_eq!(run(&reg, &mut vfs, "echo $V"), "it's \"fine\" $HOME");
}

#[test]
fn unquoted_substitution_is_field_split_quoted_is_not() {
    let (reg, mut vfs) = shell();
    assert_eq!(
        run(
            &reg,
            &mut vfs,
            "for w in $(echo a b  c); do echo [$w]; done"
        ),
        "[a]\n[b]\n[c]"
    );
    assert_eq!(
        run(
            &reg,
            &mut vfs,
            "for w in \"$(echo a b)\"; do echo [$w]; done"
        ),
        "[a b]"
    );
    assert_eq!(run(&reg, &mut vfs, "echo \"$(echo 'a   b')\""), "a   b");
}

#[test]
fn globs_follow_quoting() {
    let (reg, mut vfs) = shell();
    vfs.write("/tmp/a.txt", b"").unwrap();
    vfs.write("/tmp/b.txt", b"").unwrap();
    assert_eq!(
        run(&reg, &mut vfs, "echo /tmp/*.txt"),
        "/tmp/a.txt /tmp/b.txt"
    );
    assert_eq!(run(&reg, &mut vfs, "echo \"/tmp/*.txt\""), "/tmp/*.txt");
    // An unquoted expansion is a pattern (POSIX); a quoted one is not.
    reg.set_variable("P", "/tmp/a*");
    assert_eq!(run(&reg, &mut vfs, "echo $P"), "/tmp/a.txt");
    assert_eq!(run(&reg, &mut vfs, "echo \"$P\""), "/tmp/a*");
    // A quoted star next to an unquoted one stays literal.
    assert_eq!(run(&reg, &mut vfs, "echo /tmp/\"*\"*"), "/tmp/**");
}

#[test]
fn alias_text_is_expanded_with_the_line() {
    let (reg, mut vfs) = shell();
    reg.set_variable("WHO", "world");
    run(&reg, &mut vfs, "alias greet='echo hello $WHO'");
    assert_eq!(run(&reg, &mut vfs, "greet again"), "hello world again");
}

// -- Background jobs anywhere --

#[test]
fn ampersand_backgrounds_inside_a_line() {
    let (reg, mut vfs) = shell();
    let out = run(&reg, &mut vfs, "echo first & echo second");
    assert_eq!(out, "[1] echo first\nsecond");
    assert_eq!(reg.pending_jobs(), 1);
    // The whole and-or list before `&` is one job.
    let out = run(&reg, &mut vfs, "true && echo x & echo y");
    assert_eq!(out, "[2] true && echo x\ny");
    // Redirections with `&` are not background operators.
    run(&reg, &mut vfs, "cat /missing > /tmp/e 2>&1");
    assert_eq!(reg.pending_jobs(), 2);
}

#[test]
fn ampersand_backgrounds_inside_loops_with_frozen_variables() {
    let (reg, mut vfs) = shell();
    let out = run(&reg, &mut vfs, "for i in 1 2; do echo job$i & done");
    assert_eq!(out, "[1] echo job1\n[2] echo job2");
    let mut env = make_env(&mut vfs);
    let first = crate::pipeline::output_to_text(&reg.poll_jobs(&mut env).unwrap());
    assert!(first.starts_with("job1\n"), "{first}");
    let second = crate::pipeline::output_to_text(&reg.poll_jobs(&mut env).unwrap());
    assert!(second.starts_with("job2\n"), "{second}");
}

#[test]
fn ampersand_backgrounds_inside_scripts() {
    let (reg, mut vfs) = shell();
    vfs.write("/tmp/s.sh", b"echo fg\necho bg > /tmp/bg.txt &\necho done")
        .unwrap();
    let out = run(&reg, &mut vfs, "run /tmp/s.sh");
    assert_eq!(out, "fg\n[1] echo bg > /tmp/bg.txt\ndone");
    assert!(!vfs.exists("/tmp/bg.txt"));
    let mut env = make_env(&mut vfs);
    reg.poll_jobs(&mut env).unwrap();
    assert_eq!(vfs.read("/tmp/bg.txt").unwrap(), b"bg");
}

// -- xargs --

#[test]
fn xargs_runs_its_command() {
    let (reg, mut vfs) = shell();
    assert_eq!(run(&reg, &mut vfs, "echo a b c | xargs echo x"), "x a b c");
    assert_eq!(
        run(&reg, &mut vfs, "echo a b c | xargs -n 2 echo"),
        "a b\nc"
    );
    assert_eq!(run(&reg, &mut vfs, "echo a b | xargs"), "a b");
    vfs.write("/tmp/list", b"one\ntwo").unwrap();
    assert_eq!(
        run(&reg, &mut vfs, "cat /tmp/list | xargs -I {} echo [{}]"),
        "[one]\n[two]"
    );
    // Items are words: shell syntax in them is not interpreted.
    vfs.write("/tmp/odd", b"$HOME;rm").unwrap();
    assert_eq!(run(&reg, &mut vfs, "cat /tmp/odd | xargs echo"), "$HOME;rm");
    // xargs reaches builtins too, and reports failures with status 123.
    run(&reg, &mut vfs, "echo v | xargs -I X set GOT=X");
    assert_eq!(reg.get_variable("GOT").as_deref(), Some("v"));
    let out = run(&reg, &mut vfs, "echo /nope | xargs cat");
    assert!(out.contains("xargs: cat:"), "{out}");
    assert_eq!(status(&reg), "123");
    assert!(run(&reg, &mut vfs, "echo a | xargs -z").contains("usage: xargs"));
}

// -- Comments --

#[test]
fn comments_on_interactive_lines_are_ignored() {
    let (reg, mut vfs) = shell();
    assert_eq!(run(&reg, &mut vfs, "echo hi # trailing comment"), "hi");
    assert_eq!(run(&reg, &mut vfs, "# whole line"), "");
    assert_eq!(
        run(&reg, &mut vfs, "echo a#b '# not' \"#\" \\#x"),
        "a#b # not # #x"
    );
    // `$#` is a parameter, not a comment.
    assert_eq!(run(&reg, &mut vfs, "echo a$#b"), "ab");
    // As in a POSIX shell, a comment runs to the end of the line.
    assert_eq!(run(&reg, &mut vfs, "echo x; # echo y; echo z"), "x");
    assert_eq!(run(&reg, &mut vfs, "if true; then echo t; fi # done"), "t");
}
