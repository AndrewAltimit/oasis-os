/// A single reversible edit for the undo/redo stack.
#[derive(Debug, Clone)]
pub enum EditOperation {
    InsertChar { line: usize, col: usize, ch: char },
    DeleteChar { line: usize, col: usize, ch: char },
    SplitLine { line: usize, col: usize },
    JoinLines { line: usize },
    InsertLine { at: usize, text: String },
    DeleteLine { at: usize, text: String },
}

/// The text buffer backing the editor.
#[derive(Debug, Clone)]
pub struct EditorBuffer {
    pub(crate) lines: Vec<String>,
}

impl Default for EditorBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorBuffer {
    /// Create an empty buffer with one blank line.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
        }
    }

    /// Build a buffer from a multi-line text string.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        if lines.is_empty() {
            Self::new()
        } else {
            Self { lines }
        }
    }

    /// Number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Length (in chars) of a given line, or 0 if out of range.
    pub fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |l| l.len())
    }

    /// Get a line by index.
    pub fn get_line(&self, line: usize) -> Option<&str> {
        self.lines.get(line).map(String::as_str)
    }

    /// Replace a line's contents entirely.
    pub fn set_line(&mut self, line: usize, text: String) {
        if line < self.lines.len() {
            self.lines[line] = text;
        }
    }

    /// Insert a character at the given line and column.
    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        if line < self.lines.len() {
            let col = col.min(self.lines[line].len());
            self.lines[line].insert(col, ch);
        }
    }

    /// Delete the character at the given line and column.
    ///
    /// Returns the deleted character, or `None` if out of range.
    pub fn delete_char(&mut self, line: usize, col: usize) -> Option<char> {
        if line < self.lines.len() && col < self.lines[line].len() {
            Some(self.lines[line].remove(col))
        } else {
            None
        }
    }

    /// Insert a new empty line at `at`.
    pub fn insert_line(&mut self, at: usize) {
        let at = at.min(self.lines.len());
        self.lines.insert(at, String::new());
    }

    /// Insert a new line with content at `at`.
    pub fn insert_line_with(&mut self, at: usize, text: String) {
        let at = at.min(self.lines.len());
        self.lines.insert(at, text);
    }

    /// Delete the line at `at`, returning its content.
    pub fn delete_line(&mut self, at: usize) -> Option<String> {
        if at < self.lines.len() && self.lines.len() > 1 {
            Some(self.lines.remove(at))
        } else {
            None
        }
    }

    /// Split a line at `col`, pushing the remainder to a new line.
    pub fn split_line(&mut self, line: usize, col: usize) {
        if line < self.lines.len() {
            let col = col.min(self.lines[line].len());
            let remainder = self.lines[line][col..].to_string();
            self.lines[line].truncate(col);
            self.lines.insert(line + 1, remainder);
        }
    }

    /// Join line `line` with the line below it.
    pub fn join_lines(&mut self, line: usize) {
        if line + 1 < self.lines.len() {
            let next = self.lines.remove(line + 1);
            self.lines[line].push_str(&next);
        }
    }

    /// Serialize the entire buffer to a single string.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}
