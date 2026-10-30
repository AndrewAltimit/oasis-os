//! Stateless completers: context detection plus command, variable and
//! path candidates.

use oasis_vfs::{EntryKind, Vfs};

use super::helpers::{
    build_result, find_variable_start, normalize_path, resolve_simple, split_path,
};
use super::{CompletionContext, CompletionResult};

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
