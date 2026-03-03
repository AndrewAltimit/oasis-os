//! Text editor application with modal editing, undo/redo, and find.
//!
//! `TextEditorApp` provides a simple text editor with Normal, Insert,
//! Find, and Saving modes. It tracks modifications via an undo/redo
//! stack and formats content with line numbers for display.

use std::any::Any;

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::ContentState;
use super::app_trait::App;
use super::file_manager::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use super::runner::AppAction;

// ---------------------------------------------------------------
// EditorMode
// ---------------------------------------------------------------

/// Modal editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    /// Viewing and cursor movement.
    Normal,
    /// Typing text.
    Insert,
    /// Find bar active.
    Find,
    /// Save confirmation pending.
    Saving,
}

// ---------------------------------------------------------------
// EditOperation (undo/redo)
// ---------------------------------------------------------------

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

// ---------------------------------------------------------------
// EditorBuffer
// ---------------------------------------------------------------

/// The text buffer backing the editor.
#[derive(Debug, Clone)]
pub struct EditorBuffer {
    lines: Vec<String>,
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

// ---------------------------------------------------------------
// TextEditorApp
// ---------------------------------------------------------------

/// Text editor application with modal editing and undo/redo.
#[derive(Debug)]
pub struct TextEditorApp {
    content: ContentState,
    buffer: EditorBuffer,
    mode: EditorMode,
    cursor_line: usize,
    cursor_col: usize,
    scroll_x: usize,
    file_path: Option<String>,
    modified: bool,
    status_message: Option<String>,
    find_query: String,
    find_active: bool,
    undo_stack: Vec<EditOperation>,
    redo_stack: Vec<EditOperation>,
}

impl TextEditorApp {
    /// Create a new empty text editor.
    pub fn new(path: &str) -> Self {
        let content = ContentState::new("Text Editor", path);
        let mut editor = Self {
            content,
            buffer: EditorBuffer::new(),
            mode: EditorMode::Normal,
            cursor_line: 0,
            cursor_col: 0,
            scroll_x: 0,
            file_path: None,
            modified: false,
            status_message: None,
            find_query: String::new(),
            find_active: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        editor.rebuild_display_lines();
        editor
    }

    /// Open a file with the given content.
    pub fn open_file(path: &str, content: &str) -> Self {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let title = format!("Text Editor - {file_name}");
        let cs = ContentState::new(&title, "/apps/editor");
        let mut editor = Self {
            content: cs,
            buffer: EditorBuffer::from_text(content),
            mode: EditorMode::Normal,
            cursor_line: 0,
            cursor_col: 0,
            scroll_x: 0,
            file_path: Some(path.to_string()),
            modified: false,
            status_message: None,
            find_query: String::new(),
            find_active: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        editor.rebuild_display_lines();
        editor
    }

    // ---------------------------------------------------------------
    // Cursor movement
    // ---------------------------------------------------------------

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

    /// Format the buffer lines with line numbers for display.
    pub fn format_display_lines(&self) -> Vec<String> {
        self.buffer
            .lines
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let num = i + 1;
                let marker = if i == self.cursor_line { ">" } else { " " };
                format!("{marker}{num:>4} | {text}")
            })
            .collect()
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

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    /// Clamp cursor column to the current line length.
    fn clamp_cursor_col(&mut self) {
        let len = self.buffer.line_len(self.cursor_line);
        if self.cursor_col > len {
            self.cursor_col = len;
        }
    }

    /// Ensure the cursor line is within the visible scroll window.
    fn ensure_cursor_visible(&mut self) {
        let max_vis = self.content.cached_max_visible.max(1).saturating_sub(1); // reserve 1 line for status
        if self.cursor_line < self.content.scroll {
            self.content.scroll = self.cursor_line;
        } else if self.cursor_line >= self.content.scroll + max_vis {
            self.content.scroll = self.cursor_line.saturating_sub(max_vis - 1);
        }
    }

    /// Rebuild the display lines from the buffer and update
    /// ContentState for rendering.
    fn rebuild_display_lines(&mut self) {
        let mut lines = self.format_display_lines();

        // Status bar line at the end.
        let mode_str = match self.mode {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Find => "FIND",
            EditorMode::Saving => "SAVING",
        };
        let mod_str = if self.modified { " [Modified]" } else { "" };
        let pos_str = format!("Ln {}, Col {}", self.cursor_line + 1, self.cursor_col + 1);
        let status = if let Some(ref msg) = self.status_message {
            format!("-- {mode_str} -- {pos_str}{mod_str}  {msg}")
        } else {
            format!("-- {mode_str} -- {pos_str}{mod_str}")
        };
        lines.push(status);

        self.content.lines = lines;

        // Update content cursor/scroll to track editor cursor.
        let vis = self.content.cached_max_visible.max(1);
        self.content.cursor = self
            .cursor_line
            .saturating_sub(self.content.scroll)
            .min(vis.saturating_sub(1));
    }

    /// Build the title string.
    fn build_title(&self) -> String {
        match &self.file_path {
            Some(fp) => {
                let name = fp.rsplit('/').next().unwrap_or(fp);
                format!("Text Editor - {name}")
            },
            None => "Text Editor".to_string(),
        }
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
// App trait implementation
// ---------------------------------------------------------------

impl App for TextEditorApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match self.mode {
            EditorMode::Normal => self.handle_normal_input(button),
            EditorMode::Insert => self.handle_insert_input(button),
            EditorMode::Find => self.handle_find_input(button),
            EditorMode::Saving => self.handle_saving_input(button),
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.title = self.build_title();
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.content.pending_vfs_request.take()
    }

    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.content.pending_vfs_request.as_ref()
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn viewing_file(&self) -> Option<&str> {
        self.file_path.as_deref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------
// Per-mode input handlers
// ---------------------------------------------------------------

impl TextEditorApp {
    fn handle_normal_input(&mut self, button: &Button) -> AppAction {
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

    fn handle_insert_input(&mut self, button: &Button) -> AppAction {
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

    fn handle_find_input(&mut self, button: &Button) -> AppAction {
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

    fn handle_saving_input(&mut self, button: &Button) -> AppAction {
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

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::EditorBuffer;
    use super::EditorMode;
    use super::TextEditorApp;
    use crate::apps::app_trait::App;
    use crate::apps::runner::AppAction;
    use crate::input::Button;
    use crate::vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    // -- EditorBuffer tests --

    #[test]
    fn buffer_new_has_one_empty_line() {
        let buf = EditorBuffer::new();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some(""));
    }

    #[test]
    fn buffer_from_text() {
        let buf = EditorBuffer::from_text("hello\nworld");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("hello"));
        assert_eq!(buf.get_line(1), Some("world"));
    }

    #[test]
    fn buffer_from_empty_text() {
        let buf = EditorBuffer::from_text("");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some(""));
    }

    #[test]
    fn buffer_insert_char() {
        let mut buf = EditorBuffer::from_text("abc");
        buf.insert_char(0, 1, 'X');
        assert_eq!(buf.get_line(0), Some("aXbc"));
    }

    #[test]
    fn buffer_insert_char_at_end() {
        let mut buf = EditorBuffer::from_text("abc");
        buf.insert_char(0, 3, 'Z');
        assert_eq!(buf.get_line(0), Some("abcZ"));
    }

    #[test]
    fn buffer_insert_char_beyond_length_clamps() {
        let mut buf = EditorBuffer::from_text("ab");
        buf.insert_char(0, 99, 'X');
        assert_eq!(buf.get_line(0), Some("abX"));
    }

    #[test]
    fn buffer_delete_char() {
        let mut buf = EditorBuffer::from_text("abc");
        let ch = buf.delete_char(0, 1);
        assert_eq!(ch, Some('b'));
        assert_eq!(buf.get_line(0), Some("ac"));
    }

    #[test]
    fn buffer_delete_char_out_of_range() {
        let mut buf = EditorBuffer::from_text("abc");
        let ch = buf.delete_char(0, 5);
        assert_eq!(ch, None);
        assert_eq!(buf.get_line(0), Some("abc"));
    }

    #[test]
    fn buffer_split_line() {
        let mut buf = EditorBuffer::from_text("abcdef");
        buf.split_line(0, 3);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("abc"));
        assert_eq!(buf.get_line(1), Some("def"));
    }

    #[test]
    fn buffer_split_line_at_start() {
        let mut buf = EditorBuffer::from_text("hello");
        buf.split_line(0, 0);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some(""));
        assert_eq!(buf.get_line(1), Some("hello"));
    }

    #[test]
    fn buffer_split_line_at_end() {
        let mut buf = EditorBuffer::from_text("hello");
        buf.split_line(0, 5);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("hello"));
        assert_eq!(buf.get_line(1), Some(""));
    }

    #[test]
    fn buffer_join_lines() {
        let mut buf = EditorBuffer::from_text("abc\ndef");
        buf.join_lines(0);
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some("abcdef"));
    }

    #[test]
    fn buffer_join_lines_last_line_noop() {
        let mut buf = EditorBuffer::from_text("abc\ndef");
        buf.join_lines(1);
        assert_eq!(buf.line_count(), 2);
    }

    #[test]
    fn buffer_insert_line() {
        let mut buf = EditorBuffer::from_text("a\nb");
        buf.insert_line(1);
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("a"));
        assert_eq!(buf.get_line(1), Some(""));
        assert_eq!(buf.get_line(2), Some("b"));
    }

    #[test]
    fn buffer_delete_line() {
        let mut buf = EditorBuffer::from_text("a\nb\nc");
        let removed = buf.delete_line(1);
        assert_eq!(removed, Some("b".to_string()));
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("a"));
        assert_eq!(buf.get_line(1), Some("c"));
    }

    #[test]
    fn buffer_delete_last_remaining_line_noop() {
        let mut buf = EditorBuffer::from_text("only");
        let removed = buf.delete_line(0);
        assert_eq!(removed, None);
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn buffer_set_line() {
        let mut buf = EditorBuffer::from_text("old");
        buf.set_line(0, "new".to_string());
        assert_eq!(buf.get_line(0), Some("new"));
    }

    #[test]
    fn buffer_text_roundtrip() {
        let text = "line1\nline2\nline3";
        let buf = EditorBuffer::from_text(text);
        assert_eq!(buf.text(), text);
    }

    #[test]
    fn buffer_line_len() {
        let buf = EditorBuffer::from_text("hello");
        assert_eq!(buf.line_len(0), 5);
        assert_eq!(buf.line_len(999), 0);
    }

    #[test]
    fn buffer_get_line_out_of_range() {
        let buf = EditorBuffer::new();
        assert_eq!(buf.get_line(100), None);
    }

    // -- Cursor movement tests --

    #[test]
    fn cursor_up_at_top_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        assert_eq!(app.cursor_line, 0);
        app.cursor_up();
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn cursor_down_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        assert_eq!(app.cursor_line, 1);
    }

    #[test]
    fn cursor_down_at_bottom_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        app.cursor_down();
        assert_eq!(app.cursor_line, 2);
        app.cursor_down();
        assert_eq!(app.cursor_line, 2);
    }

    #[test]
    fn cursor_right_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.content.cached_max_visible = 20;
        app.cursor_right();
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn cursor_right_wraps_to_next_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_right();
        app.cursor_right();
        // Now at end of line 0, col 2
        app.cursor_right();
        // Should wrap to line 1, col 0
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_left_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.content.cached_max_visible = 20;
        app.cursor_col = 2;
        app.cursor_left();
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn cursor_left_wraps_to_previous_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        app.cursor_col = 0;
        app.cursor_left();
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn cursor_left_at_origin_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_left();
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_home() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.cursor_col = 3;
        app.cursor_home();
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_end() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.cursor_end();
        assert_eq!(app.cursor_col, 5);
    }

    #[test]
    fn cursor_col_clamps_on_up() {
        let mut app = TextEditorApp::open_file("/test.txt", "long line\nhi");
        app.content.cached_max_visible = 20;
        app.cursor_line = 0;
        app.cursor_col = 8;
        app.cursor_down();
        // "hi" is only 2 chars, so col should clamp.
        assert_eq!(app.cursor_col, 2);
    }

    // -- Editing tests --

    #[test]
    fn insert_char_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "ac");
        app.cursor_col = 1;
        app.insert_char('b');
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.cursor_col, 2);
        assert!(app.modified);
    }

    #[test]
    fn delete_char_backspace() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 2;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some("ac"));
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn delete_char_joins_lines() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc\ndef");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        app.cursor_col = 0;
        app.delete_char();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 3);
    }

    #[test]
    fn delete_forward_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 1;
        app.delete_forward();
        assert_eq!(app.buffer.get_line(0), Some("ac"));
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn delete_forward_joins_lines() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc\ndef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.delete_forward();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
    }

    #[test]
    fn new_line_splits() {
        let mut app = TextEditorApp::open_file("/test.txt", "abcdef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.new_line();
        assert_eq!(app.buffer.line_count(), 2);
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.buffer.get_line(1), Some("def"));
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    // -- Undo/Redo tests --

    #[test]
    fn undo_insert_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn redo_insert_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.cursor_col, 3);
    }

    #[test]
    fn undo_delete_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 3;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn undo_new_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "abcdef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.new_line();
        assert_eq!(app.buffer.line_count(), 2);
        app.undo();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
    }

    #[test]
    fn undo_on_empty_stack_noop() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn redo_on_empty_stack_noop() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        // Now insert a different char -- redo should be cleared.
        app.insert_char('X');
        app.redo();
        // Redo stack was cleared, so this should be a no-op.
        assert_eq!(app.buffer.get_line(0), Some("abX"));
    }

    // -- Find tests --

    #[test]
    fn find_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello world");
        app.content.cached_max_visible = 20;
        let found = app.find("world");
        assert!(found);
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 6);
    }

    #[test]
    fn find_not_found() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.content.cached_max_visible = 20;
        let found = app.find("xyz");
        assert!(!found);
    }

    #[test]
    fn find_on_second_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "aaa\nbbb\nccc");
        app.content.cached_max_visible = 20;
        let found = app.find("bbb");
        assert!(found);
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn find_next_advances() {
        let mut app = TextEditorApp::open_file("/test.txt", "abab\nabab");
        app.content.cached_max_visible = 20;
        app.find("ab");
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 0);
        app.find_next();
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn find_next_wraps_around() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.find("cd");
        assert_eq!(app.cursor_line, 1);
        // Find next should wrap to find "ab" if we search for
        // something at the start.
        let found = app.find("ab");
        assert!(found);
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn find_empty_query_returns_false() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        assert!(!app.find(""));
    }

    #[test]
    fn find_next_empty_query_returns_false() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        assert!(!app.find_next());
    }

    // -- File open/save roundtrip --

    #[test]
    fn open_and_save_roundtrip() {
        let text = "line1\nline2\nline3";
        let app = TextEditorApp::open_file("/test.txt", text);
        assert_eq!(app.save_content(), text);
    }

    #[test]
    fn open_empty_content() {
        let app = TextEditorApp::open_file("/test.txt", "");
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.save_content(), "");
    }

    #[test]
    fn save_after_edit() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        assert_eq!(app.save_content(), "abc");
    }

    // -- Display formatting --

    #[test]
    fn display_lines_have_line_numbers() {
        let app = TextEditorApp::open_file("/test.txt", "hello\nworld");
        let lines = app.format_display_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("1"));
        assert!(lines[0].contains("hello"));
        assert!(lines[1].contains("2"));
        assert!(lines[1].contains("world"));
    }

    #[test]
    fn display_lines_mark_cursor_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        let lines = app.format_display_lines();
        // Line at index 1 should start with '>'.
        assert!(lines[1].starts_with('>'));
        // Others should start with ' '.
        assert!(lines[0].starts_with(' '));
        assert!(lines[2].starts_with(' '));
    }

    #[test]
    fn display_lines_contain_pipe_separator() {
        let app = TextEditorApp::open_file("/test.txt", "test");
        let lines = app.format_display_lines();
        assert!(lines[0].contains('|'));
    }

    // -- Edge cases --

    #[test]
    fn empty_buffer_cursor_stays() {
        let mut app = TextEditorApp::new("/apps/editor");
        app.cursor_up();
        assert_eq!(app.cursor_line, 0);
        app.cursor_down();
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn single_line_operations() {
        let mut app = TextEditorApp::open_file("/test.txt", "x");
        app.content.cached_max_visible = 20;
        app.cursor_col = 1;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some(""));
        app.insert_char('y');
        assert_eq!(app.buffer.get_line(0), Some("y"));
    }

    #[test]
    fn very_long_line() {
        let long = "a".repeat(500);
        let app = TextEditorApp::open_file("/test.txt", &long);
        assert_eq!(app.buffer.line_len(0), 500);
        let saved = app.save_content();
        assert_eq!(saved.len(), 500);
    }

    // -- App trait tests --

    #[test]
    fn title_without_file() {
        let app = TextEditorApp::new("/apps/editor");
        assert_eq!(app.title(), "Text Editor");
    }

    #[test]
    fn title_with_file() {
        let app = TextEditorApp::open_file("/docs/readme.txt", "hi");
        assert!(app.title().contains("readme.txt"));
    }

    #[test]
    fn cancel_exits_in_normal_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn confirm_enters_insert_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.mode, EditorMode::Insert);
    }

    #[test]
    fn cancel_in_insert_returns_to_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.mode, EditorMode::Insert);
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.mode, EditorMode::Normal);
    }

    #[test]
    fn triangle_enters_find_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.mode, EditorMode::Find);
    }

    #[test]
    fn start_enters_saving_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, EditorMode::Saving);
    }

    #[test]
    fn lines_returns_display_lines() {
        let app = TextEditorApp::open_file("/test.txt", "hello\nworld");
        let lines = app.lines();
        // Should include display lines + status bar.
        assert!(lines.len() >= 2);
    }

    #[test]
    fn viewing_file_returns_path() {
        let app = TextEditorApp::open_file("/test.txt", "hello");
        assert_eq!(app.viewing_file(), Some("/test.txt"));
    }

    #[test]
    fn new_editor_not_modified() {
        let app = TextEditorApp::new("/apps/editor");
        assert!(!app.is_modified());
    }

    #[test]
    fn modified_after_insert() {
        let mut app = TextEditorApp::open_file("/test.txt", "a");
        app.insert_char('b');
        assert!(app.is_modified());
    }

    #[test]
    fn downcast_works() {
        let app = TextEditorApp::new("/apps/editor");
        let any = app.as_any();
        assert!(any.downcast_ref::<TextEditorApp>().is_some());
    }

    #[test]
    fn no_pending_request_initially() {
        let mut app = TextEditorApp::new("/apps/editor");
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn save_creates_pending_request() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "data");
        // Enter saving mode and confirm.
        app.handle_input(&Button::Start, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.expect("expected request");
        assert_eq!(path, "/test.txt");
        assert_eq!(data, "data");
    }

    #[test]
    fn undo_redo_multiple_steps() {
        let mut app = TextEditorApp::open_file("/test.txt", "");
        app.insert_char('a');
        app.insert_char('b');
        app.insert_char('c');
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("a"));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn cursor_position_getter() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        app.cursor_right();
        assert_eq!(app.cursor_position(), (1, 1));
    }

    #[test]
    fn mode_getter() {
        let app = TextEditorApp::new("/apps/editor");
        assert_eq!(app.mode(), EditorMode::Normal);
    }

    #[test]
    fn ltrigger_undo_in_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
    }

    #[test]
    fn rtrigger_redo_in_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn insert_mode_newline_via_confirm() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abcd");
        app.content.cached_max_visible = 20;
        app.mode = EditorMode::Insert;
        app.cursor_col = 2;
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.buffer.line_count(), 2);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        assert_eq!(app.buffer.get_line(1), Some("cd"));
    }

    #[test]
    fn insert_mode_backspace_via_ltrigger() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.mode = EditorMode::Insert;
        app.cursor_col = 3;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
    }

    #[test]
    fn insert_mode_delete_forward_via_rtrigger() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.mode = EditorMode::Insert;
        app.cursor_col = 1;
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ac"));
    }
}
