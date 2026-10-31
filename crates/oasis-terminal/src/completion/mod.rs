//! Enhanced tab completion engine for the shell.
//!
//! Provides context-aware completion for commands, variables, and file
//! paths, including quoted and backslash-escaped path arguments.
//!
//! This module holds the result types and the [`Completer`];
//! submodules hold the rest:
//!
//! - `candidates` -- the stateless completers ([`detect_context`],
//!   [`complete_command`], [`complete_variable`], [`complete_path`],
//!   [`longest_common_prefix`])
//! - `helpers` -- input scanning and path-splitting helpers

mod candidates;
mod helpers;
#[cfg(test)]
mod tests;

pub use candidates::{
    complete_command, complete_path, complete_variable, detect_context, longest_common_prefix,
};

use oasis_vfs::Vfs;

use helpers::{extract_partial, requote, unquote_word};

/// What kind of completion to perform based on cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// First token: complete command names, aliases, functions.
    Command,
    /// After `$`: complete variable names.
    Variable,
    /// Argument position: complete file paths.
    FilePath,
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

/// Tab completion engine for the shell (stateless: each Tab press
/// completes afresh).
#[derive(Debug, Default, Clone, Copy)]
pub struct Completer;

// ---------------------------------------------------------------------------
// Completer implementation
// ---------------------------------------------------------------------------

impl Completer {
    /// Create a completer.
    pub fn new() -> Self {
        Self
    }

    /// Main completion entry point.
    ///
    /// Analyses the input at the cursor position, determines the completion
    /// context, and returns a result (or `None` if nothing matches).
    /// `commands` are the names completed in command position (commands,
    /// builtins, aliases, functions).
    ///
    /// A path argument may be partly quoted or backslash-escaped
    /// (`cat "my f`, `cat my\ f`); the replacement keeps an opening quote
    /// (closing it once the match is complete) and otherwise escapes
    /// spaces and shell metacharacters, so the result reads back as one
    /// word.
    pub fn complete(
        &self,
        input: &str,
        cursor: usize,
        commands: &[String],
        variables: &[String],
        cwd: &str,
        vfs: &dyn Vfs,
    ) -> Option<CompletionResult> {
        let clamped = cursor.min(input.len());
        let context = detect_context(input, clamped);

        let (partial, start) = extract_partial(input, clamped, &context);

        let result = match context {
            CompletionContext::Command => complete_command(&partial, commands, &[], &[]),
            CompletionContext::Variable => complete_variable(&partial, variables),
            CompletionContext::FilePath => {
                let (word, quote) = unquote_word(&partial);
                let mut result = complete_path(&word, cwd, vfs);
                result.replacement = requote(&result.replacement, quote, result.is_complete);
                result
            },
            CompletionContext::None => return None,
        };

        if result.candidates.is_empty() {
            return None;
        }

        // Offsets relative to the full input line.
        Some(CompletionResult {
            replacement: result.replacement,
            start,
            end: clamped,
            candidates: result.candidates,
            is_complete: result.is_complete,
        })
    }
}
