//! Enhanced tab completion engine for the shell.
//!
//! Provides context-aware completion for commands, variables, and file
//! paths. Supports cycling through candidates on repeated tab presses.

use oasis_vfs::{EntryKind, Vfs};

/// What kind of completion to perform based on cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// First token: complete command names, aliases, functions.
    Command,
    /// After `$`: complete variable names.
    Variable,
    /// Argument position: complete file paths.
    FilePath,
    /// After specific commands: contextual completion.
    Subcommand {
        /// The command name preceding the cursor.
        command: String,
    },
    /// No completion available.
    None,
}

/// Result of a completion attempt.
#[derive(Debug, Clone)]
pub struct CompletionResult {
    /// Replacement text (common prefix or single match).
    pub replacement: String,
    /// Start byte offset in the input where replacement begins.
    pub start: usize,
    /// End byte offset in the input where replacement ends.
    pub end: usize,
    /// All candidates (for display on double-tab).
    pub candidates: Vec<String>,
    /// Whether completion is complete (single match).
    pub is_complete: bool,
}

/// Tab completion engine for the shell.
pub struct Completer {
    /// Current index when cycling through candidates.
    cycle_index: Option<usize>,
    /// Candidates available for cycling.
    cycle_candidates: Vec<String>,
    /// Start byte offset for the cycling region.
    cycle_start: usize,
    /// End byte offset for the cycling region.
    cycle_end: usize,
    /// The original (uncompleted) text for the cycling region.
    cycle_original: String,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Determine what kind of completion to perform based on input and cursor.
///
/// Rules:
/// - If the cursor is within or immediately after a `$` or `${...}` token,
///   return `Variable`.
/// - If the cursor is in the first whitespace-delimited token (command
///   position), return `Command`.
/// - Otherwise return `FilePath`.
/// - Empty input yields `Command`.
pub fn detect_context(input: &str, cursor: usize) -> CompletionContext {
    let clamped = cursor.min(input.len());
    let before = &input[..clamped];

    if before.is_empty() {
        return CompletionContext::Command;
    }

    // Check for variable context: scan backwards from cursor to see if
    // we are inside a `$VAR` or `${VAR` region.
    if let Some(var_start) = find_variable_start(before) {
        // We found a $ that starts a variable reference touching the cursor.
        let _ = var_start; // already validated by helper
        return CompletionContext::Variable;
    }

    // Determine whether we are still in the first token (command position).
    // The first token is everything before the first unquoted whitespace.
    let in_first_token = !before.contains(' ') && !before.contains('\t');

    if in_first_token {
        return CompletionContext::Command;
    }

    CompletionContext::FilePath
}

/// Complete command names, aliases, and functions.
///
/// Returns `None` when there are no matches.
pub fn complete_command(
    partial: &str,
    commands: &[String],
    aliases: &[String],
    functions: &[String],
) -> CompletionResult {
    let lower = partial.to_ascii_lowercase();
    let mut candidates: Vec<String> = Vec::new();

    for name in commands.iter().chain(aliases).chain(functions) {
        if name.to_ascii_lowercase().starts_with(&lower) {
            candidates.push(name.clone());
        }
    }
    candidates.sort();
    candidates.dedup();

    build_result(partial, &candidates)
}

/// Complete variable names after `$`.
///
/// The `partial` should be the text after the `$` (or `${`) up to the cursor.
pub fn complete_variable(partial: &str, variables: &[String]) -> CompletionResult {
    let mut candidates: Vec<String> = variables
        .iter()
        .filter(|v| v.starts_with(partial))
        .cloned()
        .collect();
    candidates.sort();

    build_result(partial, &candidates)
}

/// Complete file paths using the VFS.
///
/// The `partial` is the raw text the user typed (may be relative or absolute).
/// It is resolved against `cwd`. Directories get a trailing `/` appended.
pub fn complete_path(partial: &str, cwd: &str, vfs: &dyn Vfs) -> CompletionResult {
    let (dir_part, file_prefix) = split_path(partial);

    // Resolve the directory to list.
    let abs_dir = if dir_part.starts_with('/') {
        normalize_path(&dir_part)
    } else if dir_part.is_empty() {
        cwd.to_string()
    } else {
        resolve_simple(cwd, &dir_part)
    };

    let entries = match vfs.readdir(&abs_dir) {
        Ok(e) => e,
        Err(_) => return build_result(partial, &[]),
    };

    let mut candidates: Vec<String> = Vec::new();
    for entry in &entries {
        if entry.name.starts_with(&file_prefix) {
            let suffix = if entry.kind == EntryKind::Directory {
                "/"
            } else {
                ""
            };
            // Reconstruct the user-visible path with the original dir prefix.
            let display = format!("{}{}{}", dir_part, entry.name, suffix);
            candidates.push(display);
        }
    }
    candidates.sort();

    build_result(partial, &candidates)
}

/// Compute the longest common prefix of a set of strings.
///
/// Returns an empty string if the slice is empty.
pub fn longest_common_prefix(candidates: &[String]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let first = &candidates[0];
    let mut len = first.len();

    for other in &candidates[1..] {
        len = len.min(other.len());
        for (i, (a, b)) in first.bytes().zip(other.bytes()).enumerate() {
            if a != b {
                len = len.min(i);
                break;
            }
        }
    }

    first[..len].to_string()
}

// ---------------------------------------------------------------------------
// Completer implementation
// ---------------------------------------------------------------------------

impl Completer {
    /// Create a new completer with no cycling state.
    pub fn new() -> Self {
        Self {
            cycle_index: None,
            cycle_candidates: Vec::new(),
            cycle_start: 0,
            cycle_end: 0,
            cycle_original: String::new(),
        }
    }

    /// Main completion entry point.
    ///
    /// Analyses the input at the cursor position, determines the completion
    /// context, and returns a result (or `None` if nothing matches).
    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        &mut self,
        input: &str,
        cursor: usize,
        commands: &[String],
        aliases: &[String],
        functions: &[String],
        variables: &[String],
        cwd: &str,
        vfs: &dyn Vfs,
    ) -> Option<CompletionResult> {
        let clamped = cursor.min(input.len());
        let context = detect_context(input, clamped);

        let (partial, start) = extract_partial(input, clamped, &context);

        let result = match context {
            CompletionContext::Command => complete_command(&partial, commands, aliases, functions),
            CompletionContext::Variable => complete_variable(&partial, variables),
            CompletionContext::FilePath | CompletionContext::Subcommand { .. } => {
                complete_path(&partial, cwd, vfs)
            },
            CompletionContext::None => return None,
        };

        if result.candidates.is_empty() {
            self.reset();
            return None;
        }

        // Adjust offsets to be relative to the full input line.
        let adjusted = CompletionResult {
            replacement: result.replacement,
            start,
            end: clamped,
            candidates: result.candidates.clone(),
            is_complete: result.is_complete,
        };

        // Set up cycling state.
        self.cycle_candidates = result.candidates;
        self.cycle_start = start;
        self.cycle_end = clamped;
        self.cycle_original = partial;
        self.cycle_index = if adjusted.is_complete { None } else { Some(0) };

        Some(adjusted)
    }

    /// Cycle to the next candidate on repeated tab presses.
    ///
    /// Returns `None` if there is nothing to cycle (single match or no prior
    /// completion).
    pub fn cycle_next(&mut self, input: &str, cursor: usize) -> Option<CompletionResult> {
        let count = self.cycle_candidates.len();
        if count <= 1 {
            return None;
        }

        let idx = match self.cycle_index {
            Some(i) => (i + 1) % count,
            None => 0,
        };
        self.cycle_index = Some(idx);

        let replacement = self.cycle_candidates[idx].clone();
        let _ = cursor; // cursor provided for future use
        let _ = input;

        Some(CompletionResult {
            replacement,
            start: self.cycle_start,
            end: self.cycle_end,
            candidates: self.cycle_candidates.clone(),
            is_complete: true,
        })
    }

    /// Clear cycling state.
    pub fn reset(&mut self) {
        self.cycle_index = None;
        self.cycle_candidates.clear();
        self.cycle_start = 0;
        self.cycle_end = 0;
        self.cycle_original.clear();
    }

    /// Whether the completer is currently cycling through candidates.
    pub fn is_cycling(&self) -> bool {
        self.cycle_index.is_some() && self.cycle_candidates.len() > 1
    }
}

impl Default for Completer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a `CompletionResult` from a partial string and candidate list.
fn build_result(partial: &str, candidates: &[String]) -> CompletionResult {
    match candidates.len() {
        0 => CompletionResult {
            replacement: partial.to_string(),
            start: 0,
            end: partial.len(),
            candidates: Vec::new(),
            is_complete: false,
        },
        1 => CompletionResult {
            replacement: candidates[0].clone(),
            start: 0,
            end: partial.len(),
            candidates: candidates.to_vec(),
            is_complete: true,
        },
        _ => {
            let prefix = longest_common_prefix(candidates);
            CompletionResult {
                replacement: prefix,
                start: 0,
                end: partial.len(),
                candidates: candidates.to_vec(),
                is_complete: false,
            }
        },
    }
}

/// Extract the partial text that should be completed, plus its start offset
/// in the input.
fn extract_partial(input: &str, cursor: usize, context: &CompletionContext) -> (String, usize) {
    let before = &input[..cursor];

    match context {
        CompletionContext::Variable => {
            // Walk backwards from cursor to find the `$` or `${`.
            if let Some(dollar_pos) = find_variable_start(before) {
                let after_dollar = dollar_pos + 1;
                let skip_brace = if before.as_bytes().get(after_dollar) == Some(&b'{') {
                    after_dollar + 1
                } else {
                    after_dollar
                };
                let partial = &before[skip_brace..];
                (partial.to_string(), skip_brace)
            } else {
                (String::new(), cursor)
            }
        },
        CompletionContext::Command => {
            // The first token up to cursor.
            let start = 0;
            let partial = before.trim_start();
            let trimmed_start = before.len() - partial.len();
            (partial.to_string(), trimmed_start + start)
        },
        CompletionContext::FilePath | CompletionContext::Subcommand { .. } => {
            // Find the start of the current argument (last unquoted space).
            let arg_start = last_arg_start(before);
            let partial = &before[arg_start..];
            (partial.to_string(), arg_start)
        },
        CompletionContext::None => (String::new(), cursor),
    }
}

/// Find the byte offset of the `$` that starts a variable reference
/// touching the end of `before`, or `None` if we are not in a variable
/// context.
fn find_variable_start(before: &str) -> Option<usize> {
    let bytes = before.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    // Scan backwards from end.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'$' => return Some(i),
            // Inside `${...}` -- keep scanning past identifier chars and `{`.
            b'{' if i > 0 && bytes[i - 1] == b'$' => return Some(i - 1),
            c if c.is_ascii_alphanumeric() || c == b'_' => continue,
            _ => return None,
        }
    }
    None
}

/// Find the start byte offset of the last whitespace-delimited argument.
fn last_arg_start(before: &str) -> usize {
    let bytes = before.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b' ' || bytes[i] == b'\t' {
            return i + 1;
        }
    }
    0
}

/// Split a user-typed path into (directory_prefix, filename_prefix).
///
/// Examples:
/// - `"foo"` -> `("", "foo")`
/// - `"/home/us"` -> `("/home/", "us")`
/// - `"sub/"` -> `("sub/", "")`
/// - `""` -> `("", "")`
fn split_path(path: &str) -> (String, String) {
    match path.rfind('/') {
        Some(pos) => {
            let dir = &path[..=pos];
            let file = &path[pos + 1..];
            (dir.to_string(), file.to_string())
        },
        None => (String::new(), path.to_string()),
    }
}

/// Resolve a relative path against cwd (simplified, no `.` / `..` handling
/// beyond basic normalization).
fn resolve_simple(cwd: &str, relative: &str) -> String {
    let combined = if cwd == "/" {
        format!("/{relative}")
    } else {
        format!("{cwd}/{relative}")
    };
    normalize_path(&combined)
}

/// Normalize a path: collapse `.`, `..`, and double slashes.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {},
            ".." => {
                parts.pop();
            },
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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

    // -- Completer cycling ------------------------------------------------

    #[test]
    fn cycle_through_candidates() {
        let mut vfs = make_test_vfs();
        let mut completer = Completer::new();
        let cmds = vec!["echo".into(), "exit".into(), "env".into()];

        let result = completer.complete("e", 1, &cmds, &[], &[], &[], "/", &vfs);
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_complete);
        assert_eq!(r.candidates.len(), 3);

        // Cycle: should get each candidate in order.
        let c1 = completer.cycle_next("e", 1).unwrap();
        let c2 = completer.cycle_next("e", 1).unwrap();
        let c3 = completer.cycle_next("e", 1).unwrap();
        // After 3 cycles we wrap around.
        let c4 = completer.cycle_next("e", 1).unwrap();
        assert_eq!(c1.replacement, c4.replacement);

        // All three candidates should appear.
        let cycled: Vec<String> = vec![c1.replacement, c2.replacement, c3.replacement];
        assert!(cycled.contains(&"echo".into()));
        assert!(cycled.contains(&"env".into()));
        assert!(cycled.contains(&"exit".into()));

        let _ = &mut vfs; // suppress unused warning
    }

    #[test]
    fn reset_clears_cycling() {
        let mut completer = Completer::new();
        let cmds: Vec<String> = vec!["echo".into(), "exit".into()];
        let vfs = MemoryVfs::new();

        completer.complete("e", 1, &cmds, &[], &[], &[], "/", &vfs);
        assert!(completer.is_cycling());

        completer.reset();
        assert!(!completer.is_cycling());
        assert!(completer.cycle_next("e", 1).is_none());
    }

    #[test]
    fn single_match_not_cycling() {
        let mut completer = Completer::new();
        let cmds: Vec<String> = vec!["echo".into()];
        let vfs = MemoryVfs::new();

        let result = completer.complete("ech", 3, &cmds, &[], &[], &[], "/", &vfs);
        assert!(result.unwrap().is_complete);
        assert!(!completer.is_cycling());
    }

    // -- Edge cases -------------------------------------------------------

    #[test]
    fn empty_input_returns_all_commands() {
        let mut completer = Completer::new();
        let cmds: Vec<String> = vec!["ls".into(), "cd".into()];
        let vfs = MemoryVfs::new();

        let result = completer.complete("", 0, &cmds, &[], &[], &[], "/", &vfs);
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
        let mut completer = Completer::new();
        let cmds: Vec<String> = vec!["ls".into()];
        let vfs = MemoryVfs::new();

        let result = completer.complete("xyz", 3, &cmds, &[], &[], &[], "/", &vfs);
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
}
