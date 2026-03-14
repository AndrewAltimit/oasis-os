//! Integration tests for the oasis-terminal command interpreter.
//!
//! These tests exercise multi-step workflows through the full interpreter
//! chain: command registration, variable expansion, alias resolution,
//! piping, redirection, chaining, history, and glob expansion.

#![allow(clippy::unwrap_used)]

use oasis_terminal::{CommandOutput, CommandRegistry, Environment, register_builtins};
use oasis_vfs::{MemoryVfs, Vfs};

/// Build an [`Environment`] rooted at `/` with no platform services.
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

/// Create a registry with all built-in commands registered.
fn make_registry() -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    register_builtins(&mut reg);
    reg
}

/// Extract the text payload from a `CommandOutput`, panicking on
/// any other variant.
fn extract_text(output: CommandOutput) -> String {
    assert!(
        matches!(&output, CommandOutput::Text(_) | CommandOutput::Multi(_)),
        "expected CommandOutput::Text or Multi, got {output:?}"
    );
    match output {
        CommandOutput::Text(s) => s,
        CommandOutput::Multi(parts) => {
            // Collect all Text parts.
            parts
                .into_iter()
                .filter_map(|p| {
                    if let CommandOutput::Text(s) = p {
                        Some(s)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        _ => unreachable!(),
    }
}

// -----------------------------------------------------------------------
// 1. Command pipeline: echo hello | wc
// -----------------------------------------------------------------------

#[test]
fn pipeline_echo_into_wc() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    let output = reg.execute("echo hello | wc -w", &mut env).unwrap();
    let text = extract_text(output);
    // "hello" is 1 word.
    assert_eq!(text.trim(), "1");
}

#[test]
fn pipeline_echo_multiword_into_wc() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    let output = reg.execute("echo one two three | wc -w", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text.trim(), "3");
}

// -----------------------------------------------------------------------
// 2. Variable expansion
// -----------------------------------------------------------------------

#[test]
fn variable_expansion_set_and_echo() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // Set a variable, then use it.
    reg.execute("set GREETING=hello_world", &mut env).unwrap();
    let output = reg.execute("echo $GREETING", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "hello_world");
}

#[test]
fn variable_expansion_curly_brace_syntax() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("set NAME=oasis", &mut env).unwrap();
    let output = reg.execute("echo ${NAME}_os", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "oasis_os");
}

#[test]
fn variable_expansion_exit_code() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // Successful command -> $? = 0.
    reg.execute("echo ok", &mut env).unwrap();
    let output = reg.execute("echo $?", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "0");
}

// -----------------------------------------------------------------------
// 3. Alias system
// -----------------------------------------------------------------------

#[test]
fn alias_define_and_use() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();

    // Create a directory with a file so ls has something to list.
    vfs.mkdir("/testdir").unwrap();
    vfs.write("/testdir/file.txt", b"data").unwrap();

    let mut env = make_env(&mut vfs);
    env.cwd = "/testdir".to_string();

    // Define alias: ll -> ls
    reg.execute("alias ll=ls", &mut env).unwrap();

    let output = reg.execute("ll", &mut env).unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("file.txt"),
        "alias 'll' should resolve to 'ls' and list file.txt, got: {text}"
    );
}

#[test]
fn alias_with_arguments() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("alias greet=echo", &mut env).unwrap();
    let output = reg.execute("greet hello world", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "hello world");
}

// -----------------------------------------------------------------------
// 4. History
// -----------------------------------------------------------------------

#[test]
fn history_populated_after_commands() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("echo first", &mut env).unwrap();
    reg.execute("echo second", &mut env).unwrap();
    reg.execute("echo third", &mut env).unwrap();

    let hist = reg.history();
    assert!(hist.len() >= 3, "history should have at least 3 entries");
    assert!(hist.contains(&"echo first".to_string()));
    assert!(hist.contains(&"echo second".to_string()));
    assert!(hist.contains(&"echo third".to_string()));
}

#[test]
fn history_command_output() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("echo alpha", &mut env).unwrap();
    reg.execute("echo beta", &mut env).unwrap();

    let output = reg.execute("history", &mut env).unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("echo alpha"),
        "history output should contain 'echo alpha', got: {text}"
    );
    assert!(
        text.contains("echo beta"),
        "history output should contain 'echo beta', got: {text}"
    );
}

#[test]
fn history_bang_bang_repeats_last() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("echo repeated", &mut env).unwrap();
    let output = reg.execute("!!", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "repeated");
}

// -----------------------------------------------------------------------
// 5. File operations chain (MemoryVfs)
// -----------------------------------------------------------------------

#[test]
fn file_operations_create_cat_remove() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();

    // Create a file via VFS directly, then cat it.
    vfs.write("/hello.txt", b"hello from file").unwrap();

    let mut env = make_env(&mut vfs);
    let output = reg.execute("cat /hello.txt", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text, "hello from file");

    // Remove the file.
    reg.execute("rm /hello.txt", &mut env).unwrap();

    // Verify the file is gone.
    assert!(
        !env.vfs.exists("/hello.txt"),
        "file should be removed after rm"
    );
}

#[test]
fn file_operations_touch_and_ls() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("touch /newfile.txt", &mut env).unwrap();
    assert!(env.vfs.exists("/newfile.txt"), "touch should create file");

    let output = reg.execute("ls /", &mut env).unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("newfile.txt"),
        "ls should show the new file, got: {text}"
    );
}

#[test]
fn file_operations_mkdir_and_cd() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("mkdir /mydir", &mut env).unwrap();
    reg.execute("cd /mydir", &mut env).unwrap();

    let output = reg.execute("pwd", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text.trim(), "/mydir");
}

// -----------------------------------------------------------------------
// 6. Redirect: echo > file
// -----------------------------------------------------------------------

#[test]
fn redirect_stdout_to_file() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("echo redirected output > /out.txt", &mut env)
        .unwrap();

    // Read the file from VFS and verify contents.
    let data = env.vfs.read("/out.txt").unwrap();
    let content = String::from_utf8_lossy(&data);
    assert_eq!(content.trim(), "redirected output");
}

#[test]
fn redirect_append_to_file() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("echo line1 > /append.txt", &mut env).unwrap();
    reg.execute("echo line2 >> /append.txt", &mut env).unwrap();

    let data = env.vfs.read("/append.txt").unwrap();
    let content = String::from_utf8_lossy(&data);
    assert!(
        content.contains("line1"),
        "appended file should contain line1"
    );
    assert!(
        content.contains("line2"),
        "appended file should contain line2"
    );
}

// -----------------------------------------------------------------------
// 7. Multiple commands: cmd1 && cmd2
// -----------------------------------------------------------------------

#[test]
fn chaining_and_both_execute() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    let output = reg.execute("echo first && echo second", &mut env).unwrap();
    let text = extract_text(output);
    assert!(text.contains("first"), "should contain first: {text}");
    assert!(text.contains("second"), "should contain second: {text}");
}

#[test]
fn chaining_semicolon_both_execute() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    let output = reg.execute("echo alpha ; echo beta", &mut env).unwrap();
    let text = extract_text(output);
    assert!(text.contains("alpha"), "should contain alpha: {text}");
    assert!(text.contains("beta"), "should contain beta: {text}");
}

#[test]
fn chaining_and_skips_on_failure() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // nonexistent should fail, so the && echo should NOT execute.
    // Use semicolon chains to wrap: the first segment errors, setting
    // exit code to 1, then && should skip.
    let output = reg
        .execute(
            "cat /no_such_file 2>/dev/null && echo should_not_appear",
            &mut env,
        )
        .unwrap();
    let text = extract_text(output);
    assert!(
        !text.contains("should_not_appear"),
        "second command should be skipped after failure: {text}"
    );
}

#[test]
fn chaining_or_runs_on_failure() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // First command fails, || should run the fallback.
    let output = reg
        .execute("cat /no_such_file 2>/dev/null || echo fallback", &mut env)
        .unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("fallback"),
        "|| should run fallback on failure: {text}"
    );
}

// -----------------------------------------------------------------------
// 8. Error handling: non-existent command
// -----------------------------------------------------------------------

#[test]
fn error_unknown_command() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    let result = reg.execute("totally_bogus_command", &mut env);
    assert!(result.is_err(), "unknown command should return an error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unknown command"),
        "error should mention 'unknown command', got: {err_msg}"
    );
}

#[test]
fn error_unknown_command_in_chain_does_not_abort() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // In a chain, the error is captured as text, subsequent commands
    // still run.
    let output = reg.execute("bogus_cmd ; echo survived", &mut env).unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("survived"),
        "second command should still run: {text}"
    );
}

// -----------------------------------------------------------------------
// 9. Glob expansion
// -----------------------------------------------------------------------

#[test]
fn glob_expansion_star_txt() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();

    // Create several files.
    vfs.write("/notes.txt", b"notes").unwrap();
    vfs.write("/readme.txt", b"readme").unwrap();
    vfs.write("/image.png", b"image").unwrap();

    let mut env = make_env(&mut vfs);

    // `echo *.txt` should expand the glob and print the matched paths.
    let output = reg.execute("echo *.txt", &mut env).unwrap();
    let text = extract_text(output);
    assert!(
        text.contains("notes.txt"),
        "glob should match notes.txt: {text}"
    );
    assert!(
        text.contains("readme.txt"),
        "glob should match readme.txt: {text}"
    );
    assert!(
        !text.contains("image.png"),
        "glob should NOT match image.png: {text}"
    );
}

#[test]
fn glob_expansion_question_mark() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();

    vfs.write("/a1.txt", b"a1").unwrap();
    vfs.write("/a2.txt", b"a2").unwrap();
    vfs.write("/b1.txt", b"b1").unwrap();

    let mut env = make_env(&mut vfs);

    // `echo a?.txt` should match a1.txt and a2.txt but not b1.txt.
    let output = reg.execute("echo a?.txt", &mut env).unwrap();
    let text = extract_text(output);
    assert!(text.contains("a1.txt"), "should match a1.txt: {text}");
    assert!(text.contains("a2.txt"), "should match a2.txt: {text}");
    assert!(!text.contains("b1.txt"), "should NOT match b1.txt: {text}");
}

// -----------------------------------------------------------------------
// Bonus: combined workflows
// -----------------------------------------------------------------------

#[test]
fn workflow_write_redirect_then_cat_pipe_wc() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // Write multi-line content to a file via redirect.
    reg.execute("echo line_one > /data.txt", &mut env).unwrap();
    reg.execute("echo line_two >> /data.txt", &mut env).unwrap();
    reg.execute("echo line_three >> /data.txt", &mut env)
        .unwrap();

    // Cat the file and pipe to wc -l to count lines.
    let output = reg.execute("cat /data.txt | wc -l", &mut env).unwrap();
    let text = extract_text(output);
    let line_count: usize = text.trim().parse().unwrap();
    assert!(
        line_count >= 3,
        "should have at least 3 lines, got {line_count}"
    );
}

#[test]
fn workflow_command_substitution() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    // Use command substitution: $(echo inner) should expand to "inner".
    let output = reg
        .execute("echo prefix_$(echo inner)_suffix", &mut env)
        .unwrap();
    let text = extract_text(output);
    assert_eq!(text, "prefix_inner_suffix");
}

#[test]
fn workflow_alias_in_pipeline() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    let mut env = make_env(&mut vfs);

    reg.execute("alias say=echo", &mut env).unwrap();
    let output = reg.execute("say hello world | wc -w", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text.trim(), "2");
}

#[test]
fn stdin_redirect_from_file() {
    let reg = make_registry();
    let mut vfs = MemoryVfs::new();
    vfs.write("/input.txt", b"alpha beta gamma").unwrap();

    let mut env = make_env(&mut vfs);

    let output = reg.execute("wc -w < /input.txt", &mut env).unwrap();
    let text = extract_text(output);
    assert_eq!(text.trim(), "3");
}
