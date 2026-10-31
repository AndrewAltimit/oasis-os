//! Readline-style line editing for the OASIS terminal.
//!
//! Provides cursor movement, kill/yank, history navigation, reverse
//! incremental search, and tab completion cycling -- all operating on a
//! UTF-8 string buffer with a byte-offset cursor.
//!
//! This module holds the public types ([`EditAction`], [`EditResult`],
//! [`LineEditor`]) and the [`LineEditor::apply`] dispatcher. The editing
//! primitives live in submodules:
//!
//! - `editing` -- cursor movement, deletion, kill ring, insertion, swap
//! - `history` -- history browsing and Ctrl+R reverse incremental search
//! - `complete` -- tab completion cycling
//! - `text` -- UTF-8 char-boundary and word-motion helpers

mod complete;
mod editing;
mod history;
#[cfg(test)]
mod tests;
mod text;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Actions that can be performed on the line buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    // -- Cursor movement --
    /// Move cursor to the start of the line (Ctrl+A).
    MoveToStart,
    /// Move cursor to the end of the line (Ctrl+E).
    MoveToEnd,
    /// Move cursor one character to the left.
    MoveLeft,
    /// Move cursor one character to the right.
    MoveRight,
    /// Move cursor one word to the left (Alt+B / Ctrl+Left).
    MoveWordLeft,
    /// Move cursor one word to the right (Alt+F / Ctrl+Right).
    MoveWordRight,

    // -- Deletion --
    /// Delete the character before the cursor (Backspace).
    DeleteCharBack,
    /// Delete the character under the cursor (Delete / Ctrl+D).
    DeleteCharForward,
    /// Delete the word before the cursor (Ctrl+W / Alt+Backspace).
    DeleteWordBack,
    /// Kill text from cursor to end of line (Ctrl+K).
    KillToEnd,
    /// Kill text from start of line to cursor (Ctrl+U).
    KillToStart,

    // -- Text insertion --
    /// Insert a single character at the cursor.
    InsertChar(char),

    // -- History --
    /// Navigate to the previous history entry (Up arrow).
    HistoryPrev,
    /// Navigate to the next history entry (Down arrow).
    HistoryNext,

    // -- Reverse incremental search --
    /// Enter reverse-search mode (Ctrl+R).
    StartSearch,
    /// Append a character to the search query while searching.
    SearchChar(char),
    /// Jump to the next (older) match (Ctrl+R again).
    SearchNext,
    /// Remove the last character of the search query (Backspace while
    /// searching) and re-run the search from the newest entry.
    SearchBackspace,
    /// Accept the current search result (Enter / Right arrow).
    AcceptSearch,
    /// Cancel the search and restore the previous line (Escape).
    CancelSearch,

    // -- Completion --
    /// Request tab completion.
    Complete,

    // -- Line operations --
    /// Swap the two characters before the cursor (Ctrl+T).
    SwapChars,
    /// Paste the last killed text at the cursor (Ctrl+Y).
    Yank,
    /// Signal the host to clear the screen (Ctrl+L).
    ClearScreen,
    /// Accept the current line (Enter).
    AcceptLine,
    /// Cancel the current input (Ctrl+C).
    Cancel,
}

/// Result returned by [`LineEditor::apply`] after processing an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditResult {
    /// The editor is still active -- keep accepting input.
    Continue,
    /// The user accepted the line (Enter). Contains the final text.
    Accept(String),
    /// The user cancelled input (Ctrl+C).
    Cancel,
    /// The host should clear the screen (Ctrl+L).
    ClearScreen,
    /// Tab completion was requested. Contains matching candidates.
    Complete(Vec<String>),
}

// ---------------------------------------------------------------------------
// LineEditor
// ---------------------------------------------------------------------------

/// Readline-style line editor operating on a single input line.
///
/// All cursor positions are stored as **byte offsets** into the UTF-8
/// buffer.  Public display helpers convert to character offsets where
/// needed.
pub struct LineEditor {
    /// The editable text buffer.
    buffer: String,
    /// Byte offset of the cursor within `buffer`.
    cursor: usize,
    /// Last killed text (populated by Ctrl+K / Ctrl+U / Ctrl+W).
    kill_ring: String,

    // -- History navigation --
    /// Index into the history slice while browsing (`None` = current line).
    history_index: Option<usize>,
    /// Saved content of the current line before the user started browsing
    /// history, so it can be restored when they press Down past the end.
    saved_line: String,

    // -- Reverse incremental search --
    /// `true` while the user is in Ctrl+R search mode.
    search_mode: bool,
    /// Characters typed so far as the search query.
    search_query: String,
    /// Index into the history slice of the current match (if any).
    search_match_index: Option<usize>,

    // -- Tab completion cycling --
    /// Candidate list from the last `Complete` action.
    completion_candidates: Vec<String>,
    /// Current position within `completion_candidates`.
    completion_index: usize,
    /// The original buffer text that was present before completion cycling
    /// began, so we can restore it when the candidates change.
    completion_base: String,
}

impl LineEditor {
    /// Create a new, empty line editor.
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            kill_ring: String::new(),
            history_index: None,
            saved_line: String::new(),
            search_mode: false,
            search_query: String::new(),
            search_match_index: None,
            completion_candidates: Vec::new(),
            completion_index: 0,
            completion_base: String::new(),
        }
    }

    // -- Accessors --------------------------------------------------------

    /// Current buffer contents.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Cursor position as a byte offset into the buffer.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Cursor position in **characters** (for display purposes).
    pub fn display_cursor(&self) -> usize {
        self.buffer[..self.cursor].chars().count()
    }

    /// Whether the editor is currently in reverse-search mode.
    pub fn is_searching(&self) -> bool {
        self.search_mode
    }

    /// If searching, returns `(query, matched_line)` for display.
    ///
    /// Returns `None` when not in search mode.
    pub fn search_display(&self) -> Option<(&str, &str)> {
        if !self.search_mode {
            return None;
        }
        let matched = self.buffer.as_str();
        Some((&self.search_query, matched))
    }

    /// Replace the buffer with `s`, placing the cursor at the end.
    pub fn set_buffer(&mut self, s: &str) {
        self.buffer.clear();
        self.buffer.push_str(s);
        self.cursor = self.buffer.len();
        self.reset_completion();
    }

    /// Replace the buffer with `s` and put the cursor at byte offset
    /// `cursor` (clamped to the buffer and moved back to a char boundary).
    pub fn set_buffer_with_cursor(&mut self, s: &str, cursor: usize) {
        self.set_buffer(s);
        self.cursor = self
            .buffer
            .floor_char_boundary(cursor.min(self.buffer.len()));
    }

    /// The current reverse-search query (empty when not searching).
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Reset the editor to an empty state.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_index = None;
        self.saved_line.clear();
        self.search_mode = false;
        self.search_query.clear();
        self.search_match_index = None;
        self.reset_completion();
    }

    /// Paste the contents of the kill ring at the cursor (Ctrl+Y).
    pub fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        let text = self.kill_ring.clone();
        self.buffer.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    // -- Main dispatch ----------------------------------------------------

    /// Apply an [`EditAction`] and return the resulting editor state.
    ///
    /// `history` should be a slice of previously entered lines (oldest
    /// first).
    pub fn apply(&mut self, action: EditAction, history: &[String]) -> EditResult {
        // Any action that is *not* another Tab press resets the
        // completion cycling state.
        if !matches!(action, EditAction::Complete) {
            self.reset_completion();
        }

        // If we are in search mode, delegate to the search handler for
        // search-specific actions.
        if self.search_mode {
            match action {
                EditAction::SearchChar(ch) => {
                    return self.search_char(ch, history);
                },
                EditAction::SearchNext => {
                    return self.search_next(history);
                },
                EditAction::SearchBackspace => {
                    self.search_query.pop();
                    self.search_match_index = None;
                    if self.search_query.is_empty() {
                        self.buffer = self.saved_line.clone();
                        self.cursor = self.buffer.len();
                    } else {
                        self.find_search_match(history);
                    }
                    return EditResult::Continue;
                },
                EditAction::AcceptSearch => {
                    return self.accept_search();
                },
                EditAction::CancelSearch => {
                    return self.cancel_search();
                },
                // Any non-search action implicitly accepts the search
                // result and then falls through.
                _ => {
                    self.search_mode = false;
                    self.search_query.clear();
                    self.search_match_index = None;
                },
            }
        }

        match action {
            EditAction::MoveToStart => self.move_to_start(),
            EditAction::MoveToEnd => self.move_to_end(),
            EditAction::MoveLeft => self.move_left(),
            EditAction::MoveRight => self.move_right(),
            EditAction::MoveWordLeft => self.move_word_left(),
            EditAction::MoveWordRight => self.move_word_right(),

            EditAction::DeleteCharBack => self.delete_char_back(),
            EditAction::DeleteCharForward => {
                self.delete_char_forward();
            },
            EditAction::DeleteWordBack => self.delete_word_back(),
            EditAction::KillToEnd => self.kill_to_end(),
            EditAction::KillToStart => self.kill_to_start(),

            EditAction::InsertChar(ch) => self.insert_char(ch),

            EditAction::HistoryPrev => {
                return self.history_prev(history);
            },
            EditAction::HistoryNext => {
                return self.history_next(history);
            },

            EditAction::StartSearch => {
                self.search_mode = true;
                self.search_query.clear();
                self.search_match_index = None;
                self.saved_line = self.buffer.clone();
            },

            EditAction::Complete => {
                return self.complete(history);
            },

            EditAction::SwapChars => self.swap_chars(),
            EditAction::Yank => self.yank(),
            EditAction::ClearScreen => return EditResult::ClearScreen,

            EditAction::AcceptLine => {
                let line = self.buffer.clone();
                return EditResult::Accept(line);
            },
            EditAction::Cancel => return EditResult::Cancel,

            // Search variants handled above; list them here so the
            // match is exhaustive.
            EditAction::SearchChar(_)
            | EditAction::SearchNext
            | EditAction::SearchBackspace
            | EditAction::AcceptSearch
            | EditAction::CancelSearch => {},
        }

        EditResult::Continue
    }
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}
