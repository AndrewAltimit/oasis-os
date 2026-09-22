//! Line-based text buffer plus the reversible edit operations recorded on
//! the undo / redo stacks.
//!
//! Columns are **byte offsets** into a line and are always kept on UTF-8
//! character boundaries (every mutator clamps with
//! `str::floor_char_boundary`), so multi-byte input never panics.

use std::cell::RefCell;

use crate::cache::HighlightCache;
use crate::highlight::{ColorSpan, FileType};

/// A `(line, byte column)` position inside the buffer.
pub type Pos = (usize, usize);

/// A single reversible edit for the undo/redo stack.
///
/// `text` may span several lines (it contains `'\n'` separators), so one
/// operation covers a typed character, a line split, a paste or a deleted
/// selection alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    /// `text` was inserted at `(line, col)`.
    Insert {
        line: usize,
        col: usize,
        text: String,
    },
    /// `text` was removed starting at `(line, col)`.
    Delete {
        line: usize,
        col: usize,
        text: String,
    },
}

impl EditOperation {
    /// The operation that reverts this one.
    pub fn inverse(&self) -> Self {
        match self {
            Self::Insert { line, col, text } => Self::Delete {
                line: *line,
                col: *col,
                text: text.clone(),
            },
            Self::Delete { line, col, text } => Self::Insert {
                line: *line,
                col: *col,
                text: text.clone(),
            },
        }
    }

    /// Start position of the affected range.
    pub fn start(&self) -> Pos {
        match self {
            Self::Insert { line, col, .. } | Self::Delete { line, col, .. } => (*line, *col),
        }
    }

    /// The text inserted or removed.
    pub fn text(&self) -> &str {
        match self {
            Self::Insert { text, .. } | Self::Delete { text, .. } => text,
        }
    }
}

/// Position just past `text` when it is laid down starting at `start`.
pub fn end_pos(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => (start.0, start.1 + text.len()),
        Some(last_nl) => {
            let newlines = text.bytes().filter(|&b| b == b'\n').count();
            (start.0 + newlines, text.len() - last_nl - 1)
        },
    }
}

/// The text buffer backing the editor.
#[derive(Debug, Clone)]
pub struct EditorBuffer {
    pub(crate) lines: Vec<String>,
    /// Bumped on every mutation; keys the visible-span cache.
    generation: u64,
    /// Syntax-highlight cache (per-line block-comment state + spans of the
    /// last drawn window). Interior-mutable because rendering takes
    /// `&self`; every `&mut self` mutator invalidates it from the edited
    /// line down.
    highlight: RefCell<HighlightCache>,
}

impl Default for EditorBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorBuffer {
    /// Create an empty buffer with one blank line.
    pub fn new() -> Self {
        Self::from_lines(vec![String::new()])
    }

    fn from_lines(lines: Vec<String>) -> Self {
        Self {
            lines,
            generation: 0,
            highlight: RefCell::new(HighlightCache::default()),
        }
    }

    /// Build a buffer from a multi-line text string.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        if lines.is_empty() {
            Self::new()
        } else {
            Self::from_lines(lines)
        }
    }

    /// Mutation counter: changes whenever the buffer content changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Record a mutation at `line`: bump the generation and drop cached
    /// highlight state for `line` and everything below it.
    fn touch(&mut self, line: usize) {
        self.generation = self.generation.wrapping_add(1);
        self.highlight.get_mut().invalidate_from(line);
    }

    /// Number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Length (in bytes) of a given line, or 0 if out of range.
    pub fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |l| l.len())
    }

    /// Get a line by index.
    pub fn get_line(&self, line: usize) -> Option<&str> {
        self.lines.get(line).map(String::as_str)
    }

    /// Clamp `pos` into the buffer, onto a character boundary.
    pub fn clamp_pos(&self, pos: Pos) -> Pos {
        let line = pos.0.min(self.lines.len().saturating_sub(1));
        let text = self.lines.get(line).map_or("", String::as_str);
        (line, text.floor_char_boundary(pos.1))
    }

    /// Byte column of the character before `col` on `line` (0 at start).
    pub fn prev_col(&self, line: usize, col: usize) -> usize {
        let text = self.get_line(line).unwrap_or("");
        let col = text.floor_char_boundary(col);
        text[..col].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    /// Byte column of the character after `col` on `line` (line length at
    /// the end).
    pub fn next_col(&self, line: usize, col: usize) -> usize {
        let text = self.get_line(line).unwrap_or("");
        let col = text.floor_char_boundary(col);
        text[col..]
            .chars()
            .next()
            .map_or(col, |c| col + c.len_utf8())
    }

    /// Replace a line's contents entirely.
    pub fn set_line(&mut self, line: usize, text: String) {
        if line < self.lines.len() {
            self.lines[line] = text;
            self.touch(line);
        }
    }

    /// Insert a character at the given line and column.
    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        if line < self.lines.len() {
            let col = self.lines[line].floor_char_boundary(col);
            self.lines[line].insert(col, ch);
            self.touch(line);
        }
    }

    /// Delete the character at the given line and column.
    ///
    /// Returns the deleted character, or `None` if out of range.
    pub fn delete_char(&mut self, line: usize, col: usize) -> Option<char> {
        let text = self.lines.get(line)?;
        if col < text.len() && text.is_char_boundary(col) {
            let ch = self.lines[line].remove(col);
            self.touch(line);
            Some(ch)
        } else {
            None
        }
    }

    /// Insert a new empty line at `at`.
    pub fn insert_line(&mut self, at: usize) {
        self.insert_line_with(at, String::new());
    }

    /// Insert a new line with content at `at`.
    pub fn insert_line_with(&mut self, at: usize, text: String) {
        let at = at.min(self.lines.len());
        self.lines.insert(at, text);
        self.touch(at);
    }

    /// Delete the line at `at`, returning its content.
    pub fn delete_line(&mut self, at: usize) -> Option<String> {
        if at < self.lines.len() && self.lines.len() > 1 {
            let removed = self.lines.remove(at);
            self.touch(at);
            Some(removed)
        } else {
            None
        }
    }

    /// Split a line at `col`, pushing the remainder to a new line.
    pub fn split_line(&mut self, line: usize, col: usize) {
        if line < self.lines.len() {
            let col = self.lines[line].floor_char_boundary(col);
            let remainder = self.lines[line].split_off(col);
            self.lines.insert(line + 1, remainder);
            self.touch(line);
        }
    }

    /// Join line `line` with the line below it.
    pub fn join_lines(&mut self, line: usize) {
        if line + 1 < self.lines.len() {
            let next = self.lines.remove(line + 1);
            self.lines[line].push_str(&next);
            self.touch(line);
        }
    }

    /// Insert `text` (which may contain `'\n'`) at `pos`. Returns the
    /// position just past the inserted text.
    pub fn insert_text(&mut self, pos: Pos, text: &str) -> Pos {
        let (line, col) = self.clamp_pos(pos);
        if text.is_empty() {
            return (line, col);
        }
        let tail = self.lines[line].split_off(col);
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            self.lines[line].push_str(first);
        }
        let mut cur = line;
        for part in parts {
            cur += 1;
            self.lines.insert(cur, part.to_string());
        }
        let end = (cur, self.lines[cur].len());
        self.lines[cur].push_str(&tail);
        self.touch(line);
        end
    }

    /// The text between `start` and `end` (`start <= end`), lines joined
    /// with `'\n'`.
    pub fn text_range(&self, start: Pos, end: Pos) -> String {
        let (start, end) = (self.clamp_pos(start), self.clamp_pos(end));
        if end <= start {
            return String::new();
        }
        if start.0 == end.0 {
            return self.lines[start.0][start.1..end.1].to_string();
        }
        let mut out = String::from(&self.lines[start.0][start.1..]);
        for line in &self.lines[start.0 + 1..end.0] {
            out.push('\n');
            out.push_str(line);
        }
        out.push('\n');
        out.push_str(&self.lines[end.0][..end.1]);
        out
    }

    /// Remove the text between `start` and `end`, returning it.
    pub fn remove_range(&mut self, start: Pos, end: Pos) -> String {
        let (start, end) = (self.clamp_pos(start), self.clamp_pos(end));
        if end <= start {
            return String::new();
        }
        let removed = self.text_range(start, end);
        let tail = self.lines[end.0][end.1..].to_string();
        self.lines[start.0].truncate(start.1);
        self.lines[start.0].push_str(&tail);
        if end.0 > start.0 {
            self.lines.drain(start.0 + 1..=end.0);
        }
        self.touch(start.0);
        removed
    }

    /// Apply an edit operation, returning where the cursor lands.
    pub fn apply(&mut self, op: &EditOperation) -> Pos {
        match op {
            EditOperation::Insert { line, col, text } => self.insert_text((*line, *col), text),
            EditOperation::Delete { line, col, text } => {
                let start = (*line, *col);
                self.remove_range(start, end_pos(start, text));
                start
            },
        }
    }

    /// Serialize the entire buffer to a single string.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Run `f` over the highlighted spans of lines `first..first + count`
    /// (clamped to the buffer). Spans are recomputed only when the buffer
    /// generation, file type or window changed since the last call.
    pub(crate) fn with_visible_spans<R>(
        &self,
        file_type: FileType,
        first: usize,
        count: usize,
        f: impl FnOnce(&[Vec<ColorSpan>]) -> R,
    ) -> R {
        let mut cache = self.highlight.borrow_mut();
        f(cache.visible(&self.lines, file_type, self.generation, first, count))
    }

    /// Number of `highlight_line` calls the syntax cache has made so far
    /// (diagnostics: a steady frame must not increase it).
    pub fn highlight_calls(&self) -> u64 {
        self.highlight.borrow().calls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_text_multiline_and_remove_roundtrip() {
        let mut buf = EditorBuffer::from_text("hello world");
        let end = buf.insert_text((0, 5), ",\nbig\n");
        assert_eq!(buf.text(), "hello,\nbig\n world");
        assert_eq!(end, (2, 0));
        let removed = buf.remove_range((0, 5), end);
        assert_eq!(removed, ",\nbig\n");
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn text_range_spans_lines() {
        let buf = EditorBuffer::from_text("abc\ndef\nghi");
        assert_eq!(buf.text_range((0, 1), (2, 2)), "bc\ndef\ngh");
        assert_eq!(buf.text_range((1, 0), (1, 3)), "def");
        assert_eq!(buf.text_range((1, 2), (1, 1)), "");
    }

    #[test]
    fn end_pos_counts_newlines() {
        assert_eq!(end_pos((3, 4), "ab"), (3, 6));
        assert_eq!(end_pos((3, 4), "ab\ncde"), (4, 3));
        assert_eq!(end_pos((0, 0), "\n"), (1, 0));
    }

    #[test]
    fn apply_and_inverse_restore_text() {
        let mut buf = EditorBuffer::from_text("one\ntwo");
        let op = EditOperation::Delete {
            line: 0,
            col: 2,
            text: "e\ntw".into(),
        };
        buf.apply(&op);
        assert_eq!(buf.text(), "ono");
        buf.apply(&op.inverse());
        assert_eq!(buf.text(), "one\ntwo");
    }

    #[test]
    fn multibyte_columns_never_split_chars() {
        let mut buf = EditorBuffer::from_text("aé");
        // Column 2 is inside 'é' (bytes 1..3): clamps down to 1.
        buf.insert_char(0, 2, 'x');
        assert_eq!(buf.get_line(0), Some("axé"));
        assert_eq!(buf.next_col(0, 2), 4);
        assert_eq!(buf.prev_col(0, 4), 2);
        assert_eq!(buf.delete_char(0, 3), None, "mid-char delete is refused");
    }

    #[test]
    fn mutations_bump_generation() {
        let mut buf = EditorBuffer::from_text("a");
        let g = buf.generation();
        buf.insert_char(0, 0, 'b');
        assert_ne!(buf.generation(), g);
    }
}
