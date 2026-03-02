//! Readline-style line editing for the OASIS terminal.
//!
//! Provides cursor movement, kill/yank, history navigation, reverse
//! incremental search, and tab completion cycling -- all operating on a
//! UTF-8 string buffer with a byte-offset cursor.

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
            | EditAction::AcceptSearch
            | EditAction::CancelSearch => {},
        }

        EditResult::Continue
    }

    // -- Cursor movement (private) ----------------------------------------

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = prev_char_boundary(&self.buffer, self.cursor);
            self.cursor = prev;
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            let next = next_char_boundary(&self.buffer, self.cursor);
            self.cursor = next;
        }
    }

    fn move_word_left(&mut self) {
        self.cursor = word_start(&self.buffer, self.cursor);
    }

    fn move_word_right(&mut self) {
        self.cursor = word_end(&self.buffer, self.cursor);
    }

    // -- Deletion (private) -----------------------------------------------

    fn delete_char_back(&mut self) {
        if self.cursor > 0 {
            let prev = prev_char_boundary(&self.buffer, self.cursor);
            self.buffer.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    fn delete_char_forward(&mut self) {
        if self.cursor < self.buffer.len() {
            let next = next_char_boundary(&self.buffer, self.cursor);
            self.buffer.drain(self.cursor..next);
        }
    }

    fn delete_word_back(&mut self) {
        let start = word_start(&self.buffer, self.cursor);
        if start < self.cursor {
            self.kill_ring = self.buffer[start..self.cursor].to_string();
            self.buffer.drain(start..self.cursor);
            self.cursor = start;
        }
    }

    fn kill_to_end(&mut self) {
        if self.cursor < self.buffer.len() {
            self.kill_ring = self.buffer[self.cursor..].to_string();
            self.buffer.truncate(self.cursor);
        }
    }

    fn kill_to_start(&mut self) {
        if self.cursor > 0 {
            self.kill_ring = self.buffer[..self.cursor].to_string();
            self.buffer.drain(..self.cursor);
            self.cursor = 0;
        }
    }

    // -- Text insertion ---------------------------------------------------

    fn insert_char(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    // -- Swap chars -------------------------------------------------------

    fn swap_chars(&mut self) {
        // Need at least two characters and cursor must not be at position 0.
        let char_count = self.buffer.chars().count();
        if char_count < 2 || self.cursor == 0 {
            return;
        }

        // If cursor is at the end, swap the two preceding characters and
        // keep cursor at end.  Otherwise swap the character before cursor
        // with the one at cursor, then advance cursor by one.
        let at_end = self.cursor == self.buffer.len();

        let (a_start, a_end, b_start, b_end) = if at_end {
            let b_end = self.buffer.len();
            let b_start = prev_char_boundary(&self.buffer, b_end);
            let a_start = prev_char_boundary(&self.buffer, b_start);
            (a_start, b_start, b_start, b_end)
        } else {
            let a_start = prev_char_boundary(&self.buffer, self.cursor);
            let b_end = next_char_boundary(&self.buffer, self.cursor);
            (a_start, self.cursor, self.cursor, b_end)
        };

        let a: String = self.buffer[a_start..a_end].to_string();
        let b: String = self.buffer[b_start..b_end].to_string();

        let mut new = String::with_capacity(self.buffer.len());
        new.push_str(&self.buffer[..a_start]);
        new.push_str(&b);
        new.push_str(&a);
        new.push_str(&self.buffer[b_end..]);
        self.buffer = new;

        if at_end {
            self.cursor = self.buffer.len();
        } else {
            // Advance past the swapped pair.
            self.cursor = a_start + b.len();
        }
    }

    // -- History navigation -----------------------------------------------

    fn history_prev(&mut self, history: &[String]) -> EditResult {
        if history.is_empty() {
            return EditResult::Continue;
        }

        let idx = match self.history_index {
            None => {
                // First press -- save the current line and go to newest.
                self.saved_line = self.buffer.clone();
                history.len() - 1
            },
            Some(i) => {
                if i == 0 {
                    return EditResult::Continue; // already at oldest
                }
                i - 1
            },
        };

        self.history_index = Some(idx);
        self.buffer = history[idx].clone();
        self.cursor = self.buffer.len();
        EditResult::Continue
    }

    fn history_next(&mut self, history: &[String]) -> EditResult {
        match self.history_index {
            None => {}, // nothing to do
            Some(i) => {
                if i + 1 < history.len() {
                    let idx = i + 1;
                    self.history_index = Some(idx);
                    self.buffer = history[idx].clone();
                    self.cursor = self.buffer.len();
                } else {
                    // Past the newest -- restore saved line.
                    self.history_index = None;
                    self.buffer = self.saved_line.clone();
                    self.cursor = self.buffer.len();
                }
            },
        }
        EditResult::Continue
    }

    // -- Reverse incremental search ---------------------------------------

    fn search_char(&mut self, ch: char, history: &[String]) -> EditResult {
        self.search_query.push(ch);
        self.find_search_match(history);
        EditResult::Continue
    }

    fn search_next(&mut self, history: &[String]) -> EditResult {
        // Move to the next older match.
        if let Some(idx) = self.search_match_index
            && idx > 0
        {
            self.search_match_index = Some(idx - 1);
            if !self.check_match_at(history) {
                self.find_search_match_from(history, idx - 1);
            }
        }
        EditResult::Continue
    }

    fn accept_search(&mut self) -> EditResult {
        self.search_mode = false;
        self.search_query.clear();
        self.search_match_index = None;
        // Buffer already contains the matched line.
        self.cursor = self.buffer.len();
        EditResult::Continue
    }

    fn cancel_search(&mut self) -> EditResult {
        self.search_mode = false;
        self.search_query.clear();
        self.search_match_index = None;
        self.buffer = self.saved_line.clone();
        self.cursor = self.buffer.len();
        EditResult::Continue
    }

    /// Search backwards through history starting at the current match
    /// position (or the end if no match yet).
    fn find_search_match(&mut self, history: &[String]) {
        let start = self
            .search_match_index
            .unwrap_or(history.len().saturating_sub(1));
        self.find_search_match_from(history, start);
    }

    fn find_search_match_from(&mut self, history: &[String], start: usize) {
        if history.is_empty() || self.search_query.is_empty() {
            self.search_match_index = None;
            return;
        }

        for i in (0..=start).rev() {
            if history[i].contains(self.search_query.as_str()) {
                self.search_match_index = Some(i);
                self.buffer = history[i].clone();
                self.cursor = self.buffer.len();
                return;
            }
        }

        // No match found -- clear.
        self.search_match_index = None;
    }

    /// Check whether the entry at `search_match_index` still matches.
    fn check_match_at(&mut self, history: &[String]) -> bool {
        if let Some(idx) = self.search_match_index
            && idx < history.len()
            && history[idx].contains(self.search_query.as_str())
        {
            self.buffer = history[idx].clone();
            self.cursor = self.buffer.len();
            return true;
        }
        false
    }

    // -- Tab completion ---------------------------------------------------

    fn complete(&mut self, _history: &[String]) -> EditResult {
        if !self.completion_candidates.is_empty() {
            // Cycle through existing candidates.
            self.completion_index = (self.completion_index + 1) % self.completion_candidates.len();
            let candidate = self.completion_candidates[self.completion_index].clone();
            self.buffer.clear();
            self.buffer.push_str(&candidate);
            // Add a trailing space for convenience.
            if !self.buffer.ends_with(' ') {
                self.buffer.push(' ');
            }
            self.cursor = self.buffer.len();
            return EditResult::Complete(self.completion_candidates.clone());
        }

        // First tab -- the caller is responsible for providing candidates
        // via the returned `EditResult::Complete`.  We store the base text
        // so we can cycle later.
        self.completion_base = self.buffer.clone();
        EditResult::Complete(Vec::new())
    }

    /// Feed externally-computed completion candidates into the editor.
    ///
    /// If there is exactly one candidate the buffer is replaced
    /// immediately.  If there are multiple, a common prefix is filled and
    /// subsequent Tab presses will cycle through the list.
    pub fn set_completions(&mut self, candidates: Vec<String>) {
        if candidates.is_empty() {
            self.reset_completion();
            return;
        }

        if candidates.len() == 1 {
            self.buffer.clear();
            self.buffer.push_str(&candidates[0]);
            if !self.buffer.ends_with(' ') {
                self.buffer.push(' ');
            }
            self.cursor = self.buffer.len();
            self.reset_completion();
            return;
        }

        // Fill common prefix.
        if let Some(prefix) = common_prefix(&candidates)
            && prefix.len() > self.completion_base.len()
        {
            self.buffer.clear();
            self.buffer.push_str(&prefix);
            self.cursor = self.buffer.len();
        }

        self.completion_candidates = candidates;
        self.completion_index = 0;
    }

    fn reset_completion(&mut self) {
        self.completion_candidates.clear();
        self.completion_index = 0;
        self.completion_base.clear();
    }
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the byte offset of the previous character boundary, or 0.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Return the byte offset of the next character boundary, or `s.len()`.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// True if `ch` is a word-separating character.
fn is_word_sep(ch: char) -> bool {
    ch.is_whitespace() || ch.is_ascii_punctuation()
}

/// Find the byte offset of the start of the word to the left of `pos`.
fn word_start(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let bytes = s.as_bytes();
    let mut p = prev_char_boundary(s, pos);

    // Skip trailing separators.
    while p > 0 && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if !is_word_sep(ch) {
            break;
        }
        p = prev_char_boundary(s, p);
    }

    // Walk back through the word body.
    loop {
        if p == 0 {
            break;
        }
        let prev = prev_char_boundary(s, p);
        let ch = char_at(bytes, prev);
        if is_word_sep(ch) {
            break;
        }
        p = prev;
    }

    p
}

/// Find the byte offset of the end of the word to the right of `pos`.
fn word_end(s: &str, pos: usize) -> usize {
    let len = s.len();
    if pos >= len {
        return len;
    }
    let bytes = s.as_bytes();
    let mut p = pos;

    // Skip leading separators.
    while p < len && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if !is_word_sep(ch) {
            break;
        }
        p = next_char_boundary(s, p);
    }

    // Walk forward through the word body.
    while p < len && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if is_word_sep(ch) {
            break;
        }
        p = next_char_boundary(s, p);
    }

    p
}

/// Read the char that starts at byte offset `pos` in the given bytes.
///
/// Assumes `pos` is a valid char boundary.
fn char_at(bytes: &[u8], pos: usize) -> char {
    // Decode one UTF-8 character starting at `pos`.
    let rest = &bytes[pos..];
    let s = core::str::from_utf8(rest).unwrap_or("\0");
    s.chars().next().unwrap_or('\0')
}

/// Find the longest common prefix among a set of strings.
fn common_prefix(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let first = &candidates[0];
    let mut prefix_len = first.len();
    for c in &candidates[1..] {
        prefix_len = prefix_len.min(c.len());
        for (i, (a, b)) in first.bytes().zip(c.bytes()).enumerate() {
            if a != b {
                prefix_len = prefix_len.min(i);
                break;
            }
        }
    }
    // Make sure we land on a char boundary.
    let bounded = first.floor_char_boundary(prefix_len);
    Some(first[..bounded].to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand: build a simple history vector.
    fn hist(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_string()).collect()
    }

    // -- Basic insertion --------------------------------------------------

    #[test]
    fn insert_chars() {
        let mut ed = LineEditor::new();
        ed.apply(EditAction::InsertChar('h'), &[]);
        ed.apply(EditAction::InsertChar('i'), &[]);
        assert_eq!(ed.buffer(), "hi");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn insert_at_middle() {
        let mut ed = LineEditor::new();
        ed.set_buffer("ac");
        ed.cursor = 1; // between 'a' and 'c'
        ed.apply(EditAction::InsertChar('b'), &[]);
        assert_eq!(ed.buffer(), "abc");
        assert_eq!(ed.cursor(), 2);
    }

    // -- Cursor movement --------------------------------------------------

    #[test]
    fn move_to_start_and_end() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello");
        assert_eq!(ed.cursor(), 5);
        ed.apply(EditAction::MoveToStart, &[]);
        assert_eq!(ed.cursor(), 0);
        ed.apply(EditAction::MoveToEnd, &[]);
        assert_eq!(ed.cursor(), 5);
    }

    #[test]
    fn move_left_right() {
        let mut ed = LineEditor::new();
        ed.set_buffer("abc");
        ed.apply(EditAction::MoveLeft, &[]);
        assert_eq!(ed.cursor(), 2);
        ed.apply(EditAction::MoveRight, &[]);
        assert_eq!(ed.cursor(), 3);
    }

    #[test]
    fn move_left_at_start_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        ed.cursor = 0;
        ed.apply(EditAction::MoveLeft, &[]);
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn move_right_at_end_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        ed.apply(EditAction::MoveRight, &[]);
        assert_eq!(ed.cursor(), 1);
    }

    // -- Word movement ----------------------------------------------------

    #[test]
    fn move_word_left() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        // cursor at end
        ed.apply(EditAction::MoveWordLeft, &[]);
        assert_eq!(ed.display_cursor(), 6); // before 'w'
        ed.apply(EditAction::MoveWordLeft, &[]);
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn move_word_right() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        ed.cursor = 0;
        ed.apply(EditAction::MoveWordRight, &[]);
        assert_eq!(ed.display_cursor(), 5); // after 'hello'
        ed.apply(EditAction::MoveWordRight, &[]);
        assert_eq!(ed.cursor(), ed.buffer().len());
    }

    #[test]
    fn move_word_with_punctuation() {
        let mut ed = LineEditor::new();
        ed.set_buffer("foo.bar baz");
        // cursor at end
        ed.apply(EditAction::MoveWordLeft, &[]);
        // should be at 'b' of baz (position 8)
        assert_eq!(ed.display_cursor(), 8);
        ed.apply(EditAction::MoveWordLeft, &[]);
        // should be at 'b' of bar (position 4)
        assert_eq!(ed.display_cursor(), 4);
    }

    // -- Deletion ---------------------------------------------------------

    #[test]
    fn delete_char_back() {
        let mut ed = LineEditor::new();
        ed.set_buffer("abc");
        ed.apply(EditAction::DeleteCharBack, &[]);
        assert_eq!(ed.buffer(), "ab");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn delete_char_back_at_start_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        ed.cursor = 0;
        ed.apply(EditAction::DeleteCharBack, &[]);
        assert_eq!(ed.buffer(), "x");
    }

    #[test]
    fn delete_char_forward() {
        let mut ed = LineEditor::new();
        ed.set_buffer("abc");
        ed.cursor = 1;
        ed.apply(EditAction::DeleteCharForward, &[]);
        assert_eq!(ed.buffer(), "ac");
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn delete_char_forward_at_end_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        ed.apply(EditAction::DeleteCharForward, &[]);
        assert_eq!(ed.buffer(), "x");
    }

    #[test]
    fn delete_word_back() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        ed.apply(EditAction::DeleteWordBack, &[]);
        assert_eq!(ed.buffer(), "hello ");
        assert_eq!(ed.cursor(), 6);
        assert_eq!(ed.kill_ring, "world");
    }

    #[test]
    fn delete_word_back_at_start() {
        let mut ed = LineEditor::new();
        ed.set_buffer("test");
        ed.cursor = 0;
        ed.apply(EditAction::DeleteWordBack, &[]);
        assert_eq!(ed.buffer(), "test");
    }

    // -- Kill / yank ------------------------------------------------------

    #[test]
    fn kill_to_end() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        ed.cursor = 5;
        ed.apply(EditAction::KillToEnd, &[]);
        assert_eq!(ed.buffer(), "hello");
        assert_eq!(ed.kill_ring, " world");
    }

    #[test]
    fn kill_to_start() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        ed.cursor = 5;
        ed.apply(EditAction::KillToStart, &[]);
        assert_eq!(ed.buffer(), " world");
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.kill_ring, "hello");
    }

    #[test]
    fn kill_to_end_at_end_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("test");
        ed.apply(EditAction::KillToEnd, &[]);
        assert_eq!(ed.buffer(), "test");
    }

    #[test]
    fn yank_pastes_kill_ring() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello world");
        ed.cursor = 5;
        ed.apply(EditAction::KillToEnd, &[]);
        assert_eq!(ed.buffer(), "hello");

        // Move to end and yank.
        ed.apply(EditAction::MoveToEnd, &[]);
        ed.yank();
        assert_eq!(ed.buffer(), "hello world");
    }

    #[test]
    fn yank_empty_kill_ring_is_noop() {
        let mut ed = LineEditor::new();
        ed.set_buffer("abc");
        ed.yank();
        assert_eq!(ed.buffer(), "abc");
    }

    // -- SwapChars --------------------------------------------------------

    #[test]
    fn swap_chars_at_end() {
        let mut ed = LineEditor::new();
        ed.set_buffer("ab");
        // cursor at end
        ed.apply(EditAction::SwapChars, &[]);
        assert_eq!(ed.buffer(), "ba");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn swap_chars_in_middle() {
        let mut ed = LineEditor::new();
        ed.set_buffer("abc");
        ed.cursor = 2; // between 'b' and 'c'
        ed.apply(EditAction::SwapChars, &[]);
        assert_eq!(ed.buffer(), "acb");
    }

    #[test]
    fn swap_chars_too_short() {
        let mut ed = LineEditor::new();
        ed.set_buffer("a");
        ed.apply(EditAction::SwapChars, &[]);
        assert_eq!(ed.buffer(), "a");
    }

    #[test]
    fn swap_chars_cursor_at_zero() {
        let mut ed = LineEditor::new();
        ed.set_buffer("ab");
        ed.cursor = 0;
        ed.apply(EditAction::SwapChars, &[]);
        assert_eq!(ed.buffer(), "ab"); // no-op
    }

    // -- History ----------------------------------------------------------

    #[test]
    fn history_prev_next() {
        let mut ed = LineEditor::new();
        let h = hist(&["first", "second", "third"]);

        ed.set_buffer("current");
        ed.apply(EditAction::HistoryPrev, &h);
        assert_eq!(ed.buffer(), "third");
        ed.apply(EditAction::HistoryPrev, &h);
        assert_eq!(ed.buffer(), "second");
        ed.apply(EditAction::HistoryNext, &h);
        assert_eq!(ed.buffer(), "third");
        ed.apply(EditAction::HistoryNext, &h);
        assert_eq!(ed.buffer(), "current");
    }

    #[test]
    fn history_prev_empty() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        let result = ed.apply(EditAction::HistoryPrev, &[]);
        assert_eq!(result, EditResult::Continue);
        assert_eq!(ed.buffer(), "x");
    }

    #[test]
    fn history_next_without_prev() {
        let mut ed = LineEditor::new();
        ed.set_buffer("x");
        ed.apply(EditAction::HistoryNext, &hist(&["a"]));
        assert_eq!(ed.buffer(), "x"); // no-op
    }

    #[test]
    fn history_prev_at_oldest_is_noop() {
        let mut ed = LineEditor::new();
        let h = hist(&["only"]);
        ed.apply(EditAction::HistoryPrev, &h);
        assert_eq!(ed.buffer(), "only");
        ed.apply(EditAction::HistoryPrev, &h);
        assert_eq!(ed.buffer(), "only"); // still at oldest
    }

    #[test]
    fn history_saves_current_line() {
        let mut ed = LineEditor::new();
        let h = hist(&["old"]);
        ed.set_buffer("typed");
        ed.apply(EditAction::HistoryPrev, &h);
        assert_eq!(ed.buffer(), "old");
        ed.apply(EditAction::HistoryNext, &h);
        assert_eq!(ed.buffer(), "typed");
    }

    // -- Reverse search ---------------------------------------------------

    #[test]
    fn search_basic() {
        let mut ed = LineEditor::new();
        let h = hist(&["ls -la", "echo hello", "ls /tmp"]);

        ed.apply(EditAction::StartSearch, &h);
        assert!(ed.is_searching());

        ed.apply(EditAction::SearchChar('l'), &h);
        ed.apply(EditAction::SearchChar('s'), &h);
        // Should match "ls /tmp" (most recent containing "ls")
        assert_eq!(ed.buffer(), "ls /tmp");

        ed.apply(EditAction::AcceptSearch, &h);
        assert!(!ed.is_searching());
        assert_eq!(ed.buffer(), "ls /tmp");
    }

    #[test]
    fn search_next_match() {
        let mut ed = LineEditor::new();
        let h = hist(&["ls -la", "echo hello", "ls /tmp"]);

        ed.apply(EditAction::StartSearch, &h);
        ed.apply(EditAction::SearchChar('l'), &h);
        ed.apply(EditAction::SearchChar('s'), &h);
        assert_eq!(ed.buffer(), "ls /tmp");

        // Ctrl+R again -> next older match
        ed.apply(EditAction::SearchNext, &h);
        assert_eq!(ed.buffer(), "ls -la");
    }

    #[test]
    fn search_cancel_restores() {
        let mut ed = LineEditor::new();
        let h = hist(&["old command"]);

        ed.set_buffer("my input");
        ed.apply(EditAction::StartSearch, &h);
        ed.apply(EditAction::SearchChar('o'), &h);
        assert_eq!(ed.buffer(), "old command");

        ed.apply(EditAction::CancelSearch, &h);
        assert!(!ed.is_searching());
        assert_eq!(ed.buffer(), "my input");
    }

    #[test]
    fn search_display_returns_query_and_match() {
        let mut ed = LineEditor::new();
        let h = hist(&["cargo test"]);

        ed.apply(EditAction::StartSearch, &h);
        ed.apply(EditAction::SearchChar('c'), &h);

        let (query, matched) = ed.search_display().expect("in search");
        assert_eq!(query, "c");
        assert_eq!(matched, "cargo test");
    }

    #[test]
    fn search_display_none_when_not_searching() {
        let ed = LineEditor::new();
        assert!(ed.search_display().is_none());
    }

    // -- Unicode ----------------------------------------------------------

    #[test]
    fn unicode_insertion_and_movement() {
        let mut ed = LineEditor::new();
        ed.apply(EditAction::InsertChar('a'), &[]);
        ed.apply(EditAction::InsertChar('\u{00e9}'), &[]); // e-acute
        ed.apply(EditAction::InsertChar('b'), &[]);
        assert_eq!(ed.buffer(), "a\u{00e9}b");
        assert_eq!(ed.display_cursor(), 3);

        ed.apply(EditAction::MoveLeft, &[]);
        assert_eq!(ed.display_cursor(), 2);
        ed.apply(EditAction::MoveLeft, &[]);
        assert_eq!(ed.display_cursor(), 1);
    }

    #[test]
    fn unicode_delete_back() {
        let mut ed = LineEditor::new();
        // Japanese character (3 bytes in UTF-8)
        ed.set_buffer("a\u{3042}b"); // a + hiragana-a + b
        ed.cursor = 4; // after the hiragana
        ed.apply(EditAction::DeleteCharBack, &[]);
        assert_eq!(ed.buffer(), "ab");
    }

    #[test]
    fn unicode_delete_forward() {
        let mut ed = LineEditor::new();
        ed.set_buffer("\u{1f600}end"); // emoji (4 bytes) + "end"
        ed.cursor = 0;
        ed.apply(EditAction::DeleteCharForward, &[]);
        assert_eq!(ed.buffer(), "end");
    }

    // -- AcceptLine / Cancel ----------------------------------------------

    #[test]
    fn accept_line_returns_content() {
        let mut ed = LineEditor::new();
        ed.set_buffer("echo hi");
        let result = ed.apply(EditAction::AcceptLine, &[]);
        assert_eq!(result, EditResult::Accept("echo hi".into()));
    }

    #[test]
    fn cancel_returns_cancel() {
        let mut ed = LineEditor::new();
        ed.set_buffer("partial");
        let result = ed.apply(EditAction::Cancel, &[]);
        assert_eq!(result, EditResult::Cancel);
    }

    #[test]
    fn clear_screen_returns_clear() {
        let mut ed = LineEditor::new();
        let result = ed.apply(EditAction::ClearScreen, &[]);
        assert_eq!(result, EditResult::ClearScreen);
    }

    // -- Empty buffer edge cases ------------------------------------------

    #[test]
    fn empty_buffer_operations() {
        let mut ed = LineEditor::new();
        // None of these should panic.
        ed.apply(EditAction::DeleteCharBack, &[]);
        ed.apply(EditAction::DeleteCharForward, &[]);
        ed.apply(EditAction::DeleteWordBack, &[]);
        ed.apply(EditAction::KillToEnd, &[]);
        ed.apply(EditAction::KillToStart, &[]);
        ed.apply(EditAction::MoveLeft, &[]);
        ed.apply(EditAction::MoveRight, &[]);
        ed.apply(EditAction::MoveWordLeft, &[]);
        ed.apply(EditAction::MoveWordRight, &[]);
        ed.apply(EditAction::SwapChars, &[]);
        assert_eq!(ed.buffer(), "");
    }

    // -- set_buffer / clear -----------------------------------------------

    #[test]
    fn set_buffer_places_cursor_at_end() {
        let mut ed = LineEditor::new();
        ed.set_buffer("hello");
        assert_eq!(ed.cursor(), 5);
        assert_eq!(ed.buffer(), "hello");
    }

    #[test]
    fn clear_resets_everything() {
        let mut ed = LineEditor::new();
        ed.set_buffer("data");
        ed.history_index = Some(2);
        ed.search_mode = true;
        ed.clear();
        assert_eq!(ed.buffer(), "");
        assert_eq!(ed.cursor(), 0);
        assert!(!ed.is_searching());
        assert!(ed.history_index.is_none());
    }

    // -- Completion -------------------------------------------------------

    #[test]
    fn first_tab_returns_empty_candidates() {
        let mut ed = LineEditor::new();
        ed.set_buffer("ec");
        let result = ed.apply(EditAction::Complete, &[]);
        assert_eq!(result, EditResult::Complete(vec![]));
    }

    #[test]
    fn set_completions_single_fills() {
        let mut ed = LineEditor::new();
        ed.set_buffer("ec");
        ed.apply(EditAction::Complete, &[]);
        ed.set_completions(vec!["echo".into()]);
        assert_eq!(ed.buffer(), "echo ");
    }

    #[test]
    fn set_completions_multiple_fills_prefix() {
        let mut ed = LineEditor::new();
        ed.set_buffer("h");
        ed.apply(EditAction::Complete, &[]);
        ed.set_completions(vec!["help".into(), "history".into()]);
        // Common prefix is "h" which equals the base -- no change
        // but candidates are stored for cycling.
        assert!(!ed.completion_candidates.is_empty());
    }

    #[test]
    fn tab_cycling() {
        let mut ed = LineEditor::new();
        ed.set_buffer("h");
        ed.apply(EditAction::Complete, &[]);
        ed.set_completions(vec!["help".into(), "history".into()]);
        // Second tab should cycle to first candidate.
        ed.apply(EditAction::Complete, &[]);
        let buf1 = ed.buffer().trim().to_string();
        // Third tab should cycle to second candidate.
        ed.apply(EditAction::Complete, &[]);
        let buf2 = ed.buffer().trim().to_string();
        assert_ne!(buf1, buf2);
        // Candidates should be help and history.
        assert!((buf1 == "history" && buf2 == "help") || (buf1 == "help" && buf2 == "history"));
    }

    // -- display_cursor ---------------------------------------------------

    #[test]
    fn display_cursor_counts_chars_not_bytes() {
        let mut ed = LineEditor::new();
        ed.set_buffer("\u{00e9}\u{00e9}"); // two 2-byte chars
        assert_eq!(ed.cursor(), 4); // 4 bytes
        assert_eq!(ed.display_cursor(), 2); // 2 chars
    }

    // -- Default trait ----------------------------------------------------

    #[test]
    fn default_creates_empty() {
        let ed = LineEditor::default();
        assert_eq!(ed.buffer(), "");
        assert_eq!(ed.cursor(), 0);
    }
}
