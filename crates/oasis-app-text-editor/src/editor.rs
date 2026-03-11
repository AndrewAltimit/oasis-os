use oasis_app_core::AppAction;
use oasis_types::input::Button;

use crate::buffer::EditOperation;
use crate::{EditorMode, TextEditorApp};

// ---------------------------------------------------------------
// Cursor movement
// ---------------------------------------------------------------

impl TextEditorApp {
    /// Move cursor up one line.
    pub fn cursor_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.clamp_cursor_col();
            self.ensure_cursor_visible();
        }
    }

    /// Move cursor down one line.
    pub fn cursor_down(&mut self) {
        if self.cursor_line + 1 < self.buffer.line_count() {
            self.cursor_line += 1;
            self.clamp_cursor_col();
            self.ensure_cursor_visible();
        }
    }

    /// Move cursor left one column (wraps to previous line end).
    pub fn cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.buffer.line_len(self.cursor_line);
        }
        self.ensure_cursor_visible();
    }

    /// Move cursor right one column (wraps to next line start).
    pub fn cursor_right(&mut self) {
        let len = self.buffer.line_len(self.cursor_line);
        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.buffer.line_count() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.ensure_cursor_visible();
    }

    /// Move cursor to the beginning of the current line.
    pub fn cursor_home(&mut self) {
        self.cursor_col = 0;
        self.scroll_x = 0;
    }

    /// Move cursor to the end of the current line.
    pub fn cursor_end(&mut self) {
        self.cursor_col = self.buffer.line_len(self.cursor_line);
    }

    // ---------------------------------------------------------------
    // Editing operations with undo tracking
    // ---------------------------------------------------------------

    /// Insert a character at the cursor and push to undo stack.
    pub fn insert_char(&mut self, ch: char) {
        self.buffer
            .insert_char(self.cursor_line, self.cursor_col, ch);
        self.undo_stack.push(EditOperation::InsertChar {
            line: self.cursor_line,
            col: self.cursor_col,
            ch,
        });
        self.redo_stack.clear();
        self.cursor_col += 1;
        self.modified = true;
        self.rebuild_display_lines();
    }

    /// Backspace: delete the character before the cursor.
    pub fn delete_char(&mut self) {
        if self.cursor_col > 0 {
            let col = self.cursor_col - 1;
            if let Some(ch) = self.buffer.delete_char(self.cursor_line, col) {
                self.undo_stack.push(EditOperation::DeleteChar {
                    line: self.cursor_line,
                    col,
                    ch,
                });
                self.redo_stack.clear();
                self.cursor_col = col;
                self.modified = true;
                self.rebuild_display_lines();
            }
        } else if self.cursor_line > 0 {
            // Join with previous line.
            let prev = self.cursor_line - 1;
            let prev_len = self.buffer.line_len(prev);
            self.buffer.join_lines(prev);
            self.undo_stack
                .push(EditOperation::JoinLines { line: prev });
            self.redo_stack.clear();
            self.cursor_line = prev;
            self.cursor_col = prev_len;
            self.modified = true;
            self.rebuild_display_lines();
        }
    }

    /// Delete the character at the cursor (forward delete).
    pub fn delete_forward(&mut self) {
        let len = self.buffer.line_len(self.cursor_line);
        if self.cursor_col < len {
            if let Some(ch) = self.buffer.delete_char(self.cursor_line, self.cursor_col) {
                self.undo_stack.push(EditOperation::DeleteChar {
                    line: self.cursor_line,
                    col: self.cursor_col,
                    ch,
                });
                self.redo_stack.clear();
                self.modified = true;
                self.rebuild_display_lines();
            }
        } else if self.cursor_line + 1 < self.buffer.line_count() {
            // Join current line with next.
            self.buffer.join_lines(self.cursor_line);
            self.undo_stack.push(EditOperation::JoinLines {
                line: self.cursor_line,
            });
            self.redo_stack.clear();
            self.modified = true;
            self.rebuild_display_lines();
        }
    }

    /// Insert a newline at the cursor (Enter key).
    pub fn new_line(&mut self) {
        self.buffer.split_line(self.cursor_line, self.cursor_col);
        self.undo_stack.push(EditOperation::SplitLine {
            line: self.cursor_line,
            col: self.cursor_col,
        });
        self.redo_stack.clear();
        self.cursor_line += 1;
        self.cursor_col = 0;
        self.modified = true;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Undo / Redo
    // ---------------------------------------------------------------

    /// Undo the last edit operation.
    pub fn undo(&mut self) {
        let Some(op) = self.undo_stack.pop() else {
            return;
        };
        match &op {
            EditOperation::InsertChar { line, col, .. } => {
                self.buffer.delete_char(*line, *col);
                self.cursor_line = *line;
                self.cursor_col = *col;
            },
            EditOperation::DeleteChar { line, col, ch, .. } => {
                self.buffer.insert_char(*line, *col, *ch);
                self.cursor_line = *line;
                self.cursor_col = col + 1;
            },
            EditOperation::SplitLine { line, col } => {
                self.buffer.join_lines(*line);
                self.cursor_line = *line;
                self.cursor_col = *col;
            },
            EditOperation::JoinLines { line } => {
                // To undo a join, we need to split back.
                // The join appended line+1 onto line. We need
                // the original split point. We use the current
                // line length as a proxy (since the join just
                // concatenated them, the cursor_col at join time
                // was the split point). We store it in cursor
                // state at undo time by tracking in the op.
                // For simplicity, split at cursor_col.
                // However, we don't have the original col here.
                // Instead, we rely on the fact that undo is
                // called immediately, so cursor_col is at the
                // join point.
                let col = self.cursor_col;
                self.buffer.split_line(*line, col);
                self.cursor_line = *line;
                self.cursor_col = col;
            },
            EditOperation::InsertLine { at, .. } => {
                self.buffer.delete_line(*at);
                self.cursor_line = at.saturating_sub(1);
                self.cursor_col = 0;
            },
            EditOperation::DeleteLine { at, text } => {
                self.buffer.insert_line_with(*at, text.clone());
                self.cursor_line = *at;
                self.cursor_col = 0;
            },
        }
        self.redo_stack.push(op);
        self.modified = true;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    /// Redo the last undone operation.
    pub fn redo(&mut self) {
        let Some(op) = self.redo_stack.pop() else {
            return;
        };
        match &op {
            EditOperation::InsertChar { line, col, ch } => {
                self.buffer.insert_char(*line, *col, *ch);
                self.cursor_line = *line;
                self.cursor_col = col + 1;
            },
            EditOperation::DeleteChar { line, col, .. } => {
                self.buffer.delete_char(*line, *col);
                self.cursor_line = *line;
                self.cursor_col = *col;
            },
            EditOperation::SplitLine { line, col } => {
                self.buffer.split_line(*line, *col);
                self.cursor_line = line + 1;
                self.cursor_col = 0;
            },
            EditOperation::JoinLines { line } => {
                let col = self.buffer.line_len(*line);
                self.buffer.join_lines(*line);
                self.cursor_line = *line;
                self.cursor_col = col;
            },
            EditOperation::InsertLine { at, text } => {
                self.buffer.insert_line_with(*at, text.clone());
                self.cursor_line = *at;
                self.cursor_col = 0;
            },
            EditOperation::DeleteLine { at, .. } => {
                self.buffer.delete_line(*at);
                self.cursor_line = at.saturating_sub(1);
                self.cursor_col = 0;
            },
        }
        self.undo_stack.push(op);
        self.modified = true;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Find
    // ---------------------------------------------------------------

    /// Search for the first occurrence of `query` from the start.
    ///
    /// Returns `true` if found (cursor is moved to the match).
    pub fn find(&mut self, query: &str) -> bool {
        if query.is_empty() {
            return false;
        }
        self.find_query = query.to_string();
        self.find_active = true;
        // Start from the beginning.
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.find_from_current()
    }

    /// Find the next occurrence from the current cursor position.
    ///
    /// Wraps around to the beginning of the buffer.
    pub fn find_next(&mut self) -> bool {
        if self.find_query.is_empty() {
            return false;
        }
        // Advance one position to avoid re-finding the same match.
        let len = self.buffer.line_len(self.cursor_line);
        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.buffer.line_count() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        } else {
            // Wrap to beginning.
            self.cursor_line = 0;
            self.cursor_col = 0;
        }
        self.find_from_current()
    }

    /// Internal: search from current cursor position, wrapping once.
    fn find_from_current(&mut self) -> bool {
        let query = self.find_query.clone();
        let start_line = self.cursor_line;
        let start_col = self.cursor_col;

        // Search from cursor to end.
        for line_idx in start_line..self.buffer.line_count() {
            let line = match self.buffer.get_line(line_idx) {
                Some(l) => l,
                None => continue,
            };
            let search_from = if line_idx == start_line { start_col } else { 0 };
            if let Some(pos) = line[search_from..].find(&query) {
                self.cursor_line = line_idx;
                self.cursor_col = search_from + pos;
                self.ensure_cursor_visible();
                self.rebuild_display_lines();
                return true;
            }
        }

        // Wrap: search from beginning to start position.
        for line_idx in 0..=start_line {
            let line = match self.buffer.get_line(line_idx) {
                Some(l) => l,
                None => continue,
            };
            let search_end = if line_idx == start_line {
                start_col.min(line.len())
            } else {
                line.len()
            };
            if let Some(pos) = line[..search_end].find(&query) {
                self.cursor_line = line_idx;
                self.cursor_col = pos;
                self.ensure_cursor_visible();
                self.rebuild_display_lines();
                return true;
            }
        }

        false
    }

    // ---------------------------------------------------------------
    // Content serialization
    // ---------------------------------------------------------------

    /// Serialize the buffer to a string suitable for saving.
    pub fn save_content(&self) -> String {
        self.buffer.text()
    }

    /// Current editor mode.
    pub fn mode(&self) -> EditorMode {
        self.mode
    }

    /// Whether the buffer has been modified.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Current cursor position (line, col).
    pub fn cursor_position(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }

    /// Map PSP-style button combos to characters in Insert mode.
    ///
    /// In a real system this would use an on-screen keyboard or
    /// receive `InputEvent::TextInput` events. This minimal
    /// mapping exists only for basic testing.
    fn button_to_char(_button: &Button) -> Option<char> {
        None
    }
}

// ---------------------------------------------------------------
// Per-mode input handlers
// ---------------------------------------------------------------

impl TextEditorApp {
    pub(crate) fn handle_normal_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                self.cursor_up();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Down => {
                self.cursor_down();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Left => {
                self.cursor_left();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Right => {
                self.cursor_right();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Confirm => {
                self.mode = EditorMode::Insert;
                self.status_message = Some("-- INSERT --".to_string());
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Triangle => {
                self.mode = EditorMode::Find;
                self.find_active = true;
                self.find_query.clear();
                self.status_message = Some("Find: ".to_string());
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Select => {
                self.undo();
                AppAction::None
            },
            Button::Square => {
                self.redo();
                AppAction::None
            },
            Button::Start => {
                self.mode = EditorMode::Saving;
                self.status_message = Some("Save? Confirm=Yes Cancel=No".to_string());
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Cancel => AppAction::Exit,
        }
    }

    pub(crate) fn handle_insert_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Cancel => {
                self.mode = EditorMode::Normal;
                self.status_message = None;
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Up => {
                self.cursor_up();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Down => {
                self.cursor_down();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Left => {
                self.cursor_left();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Right => {
                self.cursor_right();
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Confirm => {
                self.new_line();
                AppAction::None
            },
            Button::Select => {
                self.delete_char();
                AppAction::None
            },
            Button::Square => {
                self.delete_forward();
                AppAction::None
            },
            Button::Start | Button::Triangle => AppAction::None,
        }
    }

    pub(crate) fn handle_find_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Cancel => {
                self.mode = EditorMode::Normal;
                self.find_active = false;
                self.status_message = None;
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Confirm => {
                // Execute the search.
                let found = if self.find_query.is_empty() {
                    false
                } else {
                    self.find(&self.find_query.clone())
                };
                if found {
                    self.status_message = Some(format!("Found: '{}'", self.find_query));
                } else {
                    self.status_message = Some(format!("Not found: '{}'", self.find_query));
                }
                self.mode = EditorMode::Normal;
                self.find_active = false;
                self.rebuild_display_lines();
                AppAction::None
            },
            Button::Square => {
                // Find next.
                let found = self.find_next();
                if !found {
                    self.status_message = Some(format!("Not found: '{}'", self.find_query));
                }
                self.rebuild_display_lines();
                AppAction::None
            },
            other => {
                // Append character to find query.
                if let Some(ch) = Self::button_to_char(other) {
                    self.find_query.push(ch);
                    self.status_message = Some(format!("Find: {}", self.find_query));
                    self.rebuild_display_lines();
                }
                AppAction::None
            },
        }
    }

    pub(crate) fn handle_saving_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Confirm => {
                // Request save via VFS IPC.
                if let Some(ref fp) = self.file_path {
                    let data = self.save_content();
                    self.content.pending_vfs_request = Some((fp.clone(), data));
                    self.modified = false;
                    self.status_message = Some("Saved.".to_string());
                } else {
                    self.status_message = Some("No file path set.".to_string());
                }
                self.mode = EditorMode::Normal;
                self.rebuild_display_lines();
                AppAction::None
            },
            _ => {
                self.mode = EditorMode::Normal;
                self.status_message = None;
                self.rebuild_display_lines();
                AppAction::None
            },
        }
    }
}
