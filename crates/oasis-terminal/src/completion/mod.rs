//! Enhanced tab completion engine for the shell.
//!
//! Provides context-aware completion for commands, variables, and file
//! paths. Supports cycling through candidates on repeated tab presses.
//!
//! This module holds the result types and the stateful [`Completer`];
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

use helpers::extract_partial;

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
