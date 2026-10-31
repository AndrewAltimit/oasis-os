//! Unit tests for the completion engine.

use super::helpers::{find_variable_start, last_arg_start, normalize_path, split_path};

use super::*;
use oasis_vfs::MemoryVfs;

// -- detect_context ---------------------------------------------------

#[test]
fn context_empty_input_is_command() {
    assert_eq!(detect_context("", 0), CompletionContext::Command);
}

#[test]
fn context_first_token_is_command() {
    assert_eq!(detect_context("ech", 3), CompletionContext::Command);
}

#[test]
fn context_first_token_partial_is_command() {
    assert_eq!(detect_context("ls", 2), CompletionContext::Command);
}

#[test]
fn context_after_space_is_filepath() {
    assert_eq!(detect_context("cat foo", 7), CompletionContext::FilePath);
}

#[test]
fn context_second_token_start_is_filepath() {
    assert_eq!(detect_context("ls ", 3), CompletionContext::FilePath);
}

#[test]
fn context_dollar_is_variable() {
    assert_eq!(detect_context("echo $HO", 8), CompletionContext::Variable);
}

#[test]
fn context_dollar_brace_is_variable() {
    assert_eq!(detect_context("echo ${PA", 9), CompletionContext::Variable);
}

#[test]
fn context_dollar_at_start_is_variable() {
    assert_eq!(detect_context("$PA", 3), CompletionContext::Variable);
}

// -- complete_command -------------------------------------------------

#[test]
fn command_unique_match() {
    let cmds = vec!["echo".into(), "exit".into(), "ls".into()];
    let result = complete_command("ech", &cmds, &[], &[]);
    assert_eq!(result.replacement, "echo");
    assert!(result.is_complete);
    assert_eq!(result.candidates.len(), 1);
}

#[test]
fn command_multiple_matches_common_prefix() {
    let cmds = vec!["echo".into(), "exit".into(), "env".into()];
    let result = complete_command("e", &cmds, &[], &[]);
    assert_eq!(result.candidates.len(), 3);
    assert!(!result.is_complete);
    // Common prefix of echo, env, exit is "e".
    assert_eq!(result.replacement, "e");
}

#[test]
fn command_no_match() {
    let cmds = vec!["echo".into(), "ls".into()];
    let result = complete_command("xyz", &cmds, &[], &[]);
    assert!(result.candidates.is_empty());
    assert!(!result.is_complete);
}

#[test]
fn command_includes_aliases() {
    let cmds = vec!["echo".into()];
    let aliases = vec!["emacs".into()];
    let result = complete_command("e", &cmds, &aliases, &[]);
    assert!(result.candidates.contains(&"echo".into()));
    assert!(result.candidates.contains(&"emacs".into()));
}

#[test]
fn command_includes_functions() {
    let cmds: Vec<String> = vec![];
    let funcs = vec!["greet".into()];
    let result = complete_command("gr", &cmds, &[], &funcs);
    assert_eq!(result.replacement, "greet");
    assert!(result.is_complete);
}

#[test]
fn command_case_insensitive() {
    let cmds = vec!["Echo".into()];
    let result = complete_command("ech", &cmds, &[], &[]);
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0], "Echo");
}

// -- complete_variable ------------------------------------------------

#[test]
fn variable_unique_match() {
    let vars = vec!["HOME".into(), "SHELL".into(), "PATH".into()];
    let result = complete_variable("HO", &vars);
    assert_eq!(result.replacement, "HOME");
    assert!(result.is_complete);
}

#[test]
fn variable_multiple_matches() {
    let vars = vec!["HOME".into(), "HOST".into(), "PATH".into()];
    let result = complete_variable("HO", &vars);
    assert_eq!(result.candidates.len(), 2);
    assert_eq!(result.replacement, "HO");
    assert!(!result.is_complete);
}

#[test]
fn variable_no_match() {
    let vars = vec!["HOME".into(), "SHELL".into()];
    let result = complete_variable("XY", &vars);
    assert!(result.candidates.is_empty());
}

// -- complete_path ----------------------------------------------------

fn make_test_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.write("/home/user/readme.txt", b"hello").unwrap();
    vfs.write("/home/user/report.csv", b"data").unwrap();
    vfs.mkdir("/home/user/docs").unwrap();
    vfs.write("/home/user/docs/notes.txt", b"notes").unwrap();
    vfs.mkdir("/bin").unwrap();
    vfs.write("/bin/ls", b"").unwrap();
    vfs
}

#[test]
fn path_files_in_cwd() {
    let vfs = make_test_vfs();
    let result = complete_path("re", "/home/user", &vfs);
    assert_eq!(result.candidates.len(), 2);
    assert!(result.candidates.contains(&"readme.txt".into()));
    assert!(result.candidates.contains(&"report.csv".into()));
    assert!(!result.is_complete);
}

#[test]
fn path_unique_file() {
    let vfs = make_test_vfs();
    let result = complete_path("read", "/home/user", &vfs);
    assert_eq!(result.replacement, "readme.txt");
    assert!(result.is_complete);
}

#[test]
fn path_directory_gets_slash() {
    let vfs = make_test_vfs();
    let result = complete_path("doc", "/home/user", &vfs);
    assert_eq!(result.replacement, "docs/");
    assert!(result.is_complete);
}

#[test]
fn path_subdirectory_listing() {
    let vfs = make_test_vfs();
    let result = complete_path("docs/no", "/home/user", &vfs);
    assert_eq!(result.replacement, "docs/notes.txt");
    assert!(result.is_complete);
}

#[test]
fn path_absolute() {
    let vfs = make_test_vfs();
    let result = complete_path("/bin/l", "/home/user", &vfs);
    assert_eq!(result.replacement, "/bin/ls");
    assert!(result.is_complete);
}

#[test]
fn path_no_match() {
    let vfs = make_test_vfs();
    let result = complete_path("zzz", "/home/user", &vfs);
    assert!(result.candidates.is_empty());
}

#[test]
fn path_empty_partial_lists_all() {
    let vfs = make_test_vfs();
    let result = complete_path("", "/home/user", &vfs);
    // /home/user has: readme.txt, report.csv, docs/
    assert_eq!(result.candidates.len(), 3);
}

// -- longest_common_prefix --------------------------------------------

#[test]
fn lcp_empty_slice() {
    let result = longest_common_prefix(&[]);
    assert_eq!(result, "");
}

#[test]
fn lcp_single_item() {
    let items = vec!["hello".into()];
    assert_eq!(longest_common_prefix(&items), "hello");
}

#[test]
fn lcp_common_prefix() {
    let items = vec!["readme.txt".into(), "report.csv".into()];
    assert_eq!(longest_common_prefix(&items), "re");
}

#[test]
fn lcp_identical() {
    let items = vec!["abc".into(), "abc".into()];
    assert_eq!(longest_common_prefix(&items), "abc");
}

#[test]
fn lcp_no_common() {
    let items = vec!["abc".into(), "xyz".into()];
    assert_eq!(longest_common_prefix(&items), "");
}

// -- Completer --------------------------------------------------------

#[test]
fn ambiguous_command_lists_candidates() {
    let vfs = MemoryVfs::new();
    let cmds = vec!["echo".into(), "exit".into(), "env".into()];
    let r = Completer::new()
        .complete("e", 1, &cmds, &[], "/", &vfs)
        .unwrap();
    assert!(!r.is_complete);
    assert_eq!(r.candidates.len(), 3);
}

#[test]
fn single_match_is_complete() {
    let cmds: Vec<String> = vec!["echo".into()];
    let vfs = MemoryVfs::new();
    let result = Completer::new().complete("ech", 3, &cmds, &[], "/", &vfs);
    assert!(result.unwrap().is_complete);
}

#[test]
fn lcp_respects_char_boundaries() {
    let items = vec!["café1".into(), "cafè2".into()];
    assert_eq!(longest_common_prefix(&items), "caf");
    let items = vec!["añb".into(), "añ".into()];
    assert_eq!(longest_common_prefix(&items), "añ");
}

fn spaced_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.write("/my file.txt", b"x").unwrap();
    vfs.write("/it's.txt", b"x").unwrap();
    vfs
}

#[test]
fn path_with_space_is_escaped() {
    let vfs = spaced_vfs();
    let r = Completer::new()
        .complete("cat my", 6, &[], &[], "/", &vfs)
        .unwrap();
    assert_eq!(r.replacement, "my\\ file.txt");
    assert_eq!((r.start, r.end), (4, 6));
}

#[test]
fn escaped_and_quoted_partials_complete() {
    let vfs = spaced_vfs();
    let c = Completer::new();
    let r = c.complete("cat my\\ f", 9, &[], &[], "/", &vfs).unwrap();
    assert_eq!(r.replacement, "my\\ file.txt");
    assert_eq!(r.start, 4);
    let r = c.complete("cat \"my f", 9, &[], &[], "/", &vfs).unwrap();
    assert_eq!(r.replacement, "\"my file.txt\"");
    let r = c.complete("cat 'it", 7, &[], &[], "/", &vfs).unwrap();
    assert_eq!(r.replacement, "'it'\\''s.txt'");
    let r = c.complete("cat it", 6, &[], &[], "/", &vfs).unwrap();
    assert_eq!(r.replacement, "it\\'s.txt");
}

#[test]
fn quoted_space_does_not_split_the_argument() {
    assert_eq!(last_arg_start("cat \"a b"), 4);
    assert_eq!(last_arg_start("cat a\\ b"), 4);
    assert_eq!(last_arg_start("cat 'a b' c"), 10);
}

// -- Edge cases -------------------------------------------------------

#[test]
fn empty_input_returns_all_commands() {
    let completer = Completer::new();
    let cmds: Vec<String> = vec!["ls".into(), "cd".into()];
    let vfs = MemoryVfs::new();

    let result = completer.complete("", 0, &cmds, &[], "/", &vfs);
    assert!(result.is_some());
    let r = result.unwrap();
    assert_eq!(r.candidates.len(), 2);
}

#[test]
fn cursor_at_start_is_command_context() {
    assert_eq!(detect_context("ls foo", 0), CompletionContext::Command);
}

#[test]
fn no_match_returns_none() {
    let completer = Completer::new();
    let cmds: Vec<String> = vec!["ls".into()];
    let vfs = MemoryVfs::new();

    let result = completer.complete("xyz", 3, &cmds, &[], "/", &vfs);
    assert!(result.is_none());
}

// -- Internal helpers -------------------------------------------------

#[test]
fn split_path_no_slash() {
    let (dir, file) = split_path("readme");
    assert_eq!(dir, "");
    assert_eq!(file, "readme");
}

#[test]
fn split_path_with_dir() {
    let (dir, file) = split_path("/home/us");
    assert_eq!(dir, "/home/");
    assert_eq!(file, "us");
}

#[test]
fn split_path_trailing_slash() {
    let (dir, file) = split_path("docs/");
    assert_eq!(dir, "docs/");
    assert_eq!(file, "");
}

#[test]
fn normalize_path_collapses_dotdot() {
    assert_eq!(normalize_path("/a/b/../c"), "/a/c");
}

#[test]
fn normalize_path_root() {
    assert_eq!(normalize_path("/"), "/");
}

#[test]
fn normalize_path_collapses_dot() {
    assert_eq!(normalize_path("/a/./b"), "/a/b");
}

#[test]
fn find_variable_start_dollar_prefix() {
    assert_eq!(find_variable_start("echo $HO"), Some(5));
}

#[test]
fn find_variable_start_brace() {
    assert_eq!(find_variable_start("echo ${PA"), Some(5));
}

#[test]
fn find_variable_start_none() {
    assert_eq!(find_variable_start("echo foo"), None);
}

#[test]
fn last_arg_start_single_token() {
    assert_eq!(last_arg_start("hello"), 0);
}

#[test]
fn last_arg_start_two_tokens() {
    assert_eq!(last_arg_start("cat foo"), 4);
}
