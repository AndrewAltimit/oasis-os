//! Editing core: cursor motion, selection, grouped undo/redo, clipboard,
//! find/replace, go-to-line and the save / close flows.

use oasis_app_core::AppAction;
use oasis_types::backend::ClipboardBackend;

use crate::buffer::{EditOperation, EditorBuffer, Pos, end_pos};
use crate::highlight::detect_file_type;
use crate::{EditorMode, PendingClose, TextEditorApp};

/// Maximum number of undo groups kept.
const UNDO_LIMIT: usize = 1000;

/// Spaces inserted by the Tab key.
const TAB_TEXT: &str = "    ";

/// What kind of edit an undo group holds. Consecutive `Typing` edits (and
/// consecutive `Deleting` edits) coalesce; `Other` edits (newline, paste,
/// cut, replace) always stand alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// One user-visible undo step: a run of operations plus the cursor
/// positions before and after it.
#[derive(Debug, Clone)]
pub(crate) struct UndoGroup {
    pub(crate) ops: Vec<EditOperation>,
    kind: EditKind,
    before: Pos,
    after: Pos,
}

impl UndoGroup {
    /// Try to fold `op` into this group's last operation.
    ///
    /// Typing merges while the insertion point is contiguous, except that a
    /// word character typed after whitespace starts a new group, so undo
    /// removes one word (plus its trailing spaces) at a time. Deletions merge
    /// while contiguous in either direction (Backspace or Delete runs).
    fn try_merge(&mut self, op: &EditOperation) -> bool {
        let Some(last) = self.ops.last_mut() else {
            return false;
        };
        match (last, op) {
            (
                EditOperation::Insert { line, col, text },
                EditOperation::Insert {
                    line: l2,
                    col: c2,
                    text: t2,
                },
            ) => {
                if t2.contains('\n') || end_pos((*line, *col), text) != (*l2, *c2) {
                    return false;
                }
                let prev_ws = text.chars().next_back().is_some_and(char::is_whitespace);
                let next_ws = t2.chars().next().is_some_and(char::is_whitespace);
                if prev_ws && !next_ws {
                    return false;
                }
                text.push_str(t2);
                true
            },
            (
                EditOperation::Delete { line, col, text },
                EditOperation::Delete {
                    line: l2,
                    col: c2,
                    text: t2,
                },
            ) => {
                if end_pos((*l2, *c2), t2) == (*line, *col) {
                    // Backspace run: the new deletion sits just before.
                    let mut joined = t2.clone();
                    joined.push_str(text);
                    *text = joined;
                    *line = *l2;
                    *col = *c2;
                    true
                } else if (*l2, *c2) == (*line, *col) {
                    // Forward-delete run: same start, text grows rightwards.
                    text.push_str(t2);
                    true
                } else {
                    false
                }
            },
            _ => false,
        }
    }
}

/// Whether `c` belongs to a word for Ctrl+Left/Right jumps.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextEditorApp {
    // ---------------------------------------------------------------
    // Cursor & selection
    // ---------------------------------------------------------------

    /// Current cursor as a position.
    pub(crate) fn cursor(&self) -> Pos {
        (self.cursor_line, self.cursor_col)
    }

    /// Selected range `(start, end)` with `start < end`, if any.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.anchor?;
        let cur = self.cursor();
        match anchor.cmp(&cur) {
            std::cmp::Ordering::Less => Some((anchor, cur)),
            std::cmp::Ordering::Greater => Some((cur, anchor)),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(s, e)| self.buffer.text_range(s, e))
    }

    /// Move the cursor to `pos`, extending the selection when `extend` is
    /// set (Shift held) and clearing it otherwise.
    pub(crate) fn move_to(&mut self, pos: Pos, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor());
            }
        } else {
            self.anchor = None;
        }
        let (line, col) = self.buffer.clamp_pos(pos);
        self.cursor_line = line;
        self.cursor_col = col;
        self.group_open = false;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    fn pos_left(&self) -> Pos {
        let (line, col) = self.cursor();
        if col > 0 {
            (line, self.buffer.prev_col(line, col))
        } else if line > 0 {
            (line - 1, self.buffer.line_len(line - 1))
        } else {
            (0, 0)
        }
    }

    fn pos_right(&self) -> Pos {
        let (line, col) = self.cursor();
        if col < self.buffer.line_len(line) {
            (line, self.buffer.next_col(line, col))
        } else if line + 1 < self.buffer.line_count() {
            (line + 1, 0)
        } else {
            (line, col)
        }
    }

    fn pos_vertical(&self, delta: isize) -> Pos {
        let last = self.buffer.line_count().saturating_sub(1) as isize;
        let line = (self.cursor_line as isize + delta).clamp(0, last) as usize;
        (line, self.cursor_col)
    }

    /// Start of the previous word (Ctrl+Left).
    pub(crate) fn pos_word_left(&self) -> Pos {
        let (line, col) = self.cursor();
        if col == 0 {
            return self.pos_left();
        }
        let text = self.buffer.get_line(line).unwrap_or("");
        let before = &text[..text.floor_char_boundary(col)];
        let mut iter = before.char_indices().rev().peekable();
        let mut pos = before.len();
        while let Some(&(i, c)) = iter.peek() {
            if is_word_char(c) {
                break;
            }
            pos = i;
            iter.next();
        }
        while let Some(&(i, c)) = iter.peek() {
            if !is_word_char(c) {
                break;
            }
            pos = i;
            iter.next();
        }
        (line, pos)
    }

    /// End of the next word (Ctrl+Right).
    pub(crate) fn pos_word_right(&self) -> Pos {
        let (line, col) = self.cursor();
        let text = self.buffer.get_line(line).unwrap_or("");
        if col >= text.len() {
            return self.pos_right();
        }
        let col = text.floor_char_boundary(col);
        let mut pos = col;
        let mut iter = text[col..].chars().peekable();
        while let Some(&c) = iter.peek() {
            if is_word_char(c) {
                break;
            }
            pos += c.len_utf8();
            iter.next();
        }
        while let Some(&c) = iter.peek() {
            if !is_word_char(c) {
                break;
            }
            pos += c.len_utf8();
            iter.next();
        }
        (line, pos)
    }

    /// Smart Home: first non-blank column, or column 0 when already there.
    fn pos_home(&self) -> Pos {
        let text = self.buffer.get_line(self.cursor_line).unwrap_or("");
        let indent = text.len() - text.trim_start().len();
        let col = if self.cursor_col == indent { 0 } else { indent };
        (self.cursor_line, col)
    }

    /// Move cursor up one line.
    pub fn cursor_up(&mut self) {
        self.move_to(self.pos_vertical(-1), false);
    }

    /// Move cursor down one line.
    pub fn cursor_down(&mut self) {
        self.move_to(self.pos_vertical(1), false);
    }

    /// Move cursor left one character (wraps to previous line end). With a
    /// selection, collapses to its start instead.
    pub fn cursor_left(&mut self) {
        let target = match self.selection() {
            Some((start, _)) => start,
            None => self.pos_left(),
        };
        self.move_to(target, false);
    }

    /// Move cursor right one character (wraps to next line start). With a
    /// selection, collapses to its end instead.
    pub fn cursor_right(&mut self) {
        let target = match self.selection() {
            Some((_, end)) => end,
            None => self.pos_right(),
        };
        self.move_to(target, false);
    }

    /// Move cursor to the beginning of the current line (smart Home).
    pub fn cursor_home(&mut self) {
        self.move_to(self.pos_home(), false);
        self.scroll_x = 0;
    }

    /// Move cursor to the end of the current line.
    pub fn cursor_end(&mut self) {
        self.move_to((self.cursor_line, usize::MAX), false);
    }

    /// Named cursor motions, shared by keyboard and gamepad paths.
    pub(crate) fn motion(&mut self, motion: Motion, extend: bool) {
        if !extend {
            match motion {
                Motion::Left => return self.cursor_left(),
                Motion::Right => return self.cursor_right(),
                _ => {},
            }
        }
        let page = self.page_lines().max(1) as isize;
        let target = match motion {
            Motion::Left => self.pos_left(),
            Motion::Right => self.pos_right(),
            Motion::Up => self.pos_vertical(-1),
            Motion::Down => self.pos_vertical(1),
            Motion::Home => self.pos_home(),
            Motion::End => (self.cursor_line, usize::MAX),
            Motion::WordLeft => self.pos_word_left(),
            Motion::WordRight => self.pos_word_right(),
            Motion::DocStart => (0, 0),
            Motion::DocEnd => (usize::MAX, usize::MAX),
            Motion::PageUp | Motion::PageDown => {
                let delta = if motion == Motion::PageUp {
                    -page
                } else {
                    page
                };
                let target = self.pos_vertical(delta);
                // Scroll the viewport by the same amount so the caret keeps
                // its on-screen row (like every desktop editor).
                let max_scroll = self.buffer.line_count().saturating_sub(1) as isize;
                self.content.scroll =
                    (self.content.scroll as isize + delta).clamp(0, max_scroll) as usize;
                target
            },
        };
        self.move_to(target, extend);
    }

    /// Select the whole buffer.
    pub fn select_all(&mut self) {
        let last = self.buffer.line_count().saturating_sub(1);
        self.anchor = Some((0, 0));
        self.cursor_line = last;
        self.cursor_col = self.buffer.line_len(last);
        self.group_open = false;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Editing with grouped undo
    // ---------------------------------------------------------------

    /// Record an applied operation on the undo stack.
    fn record(&mut self, op: EditOperation, kind: EditKind, before: Pos) {
        self.redo_stack.clear();
        self.modified = true;
        let after = self.cursor();
        if self.group_open
            && kind != EditKind::Other
            && let Some(group) = self.undo_stack.last_mut()
            && group.kind == kind
            && group.try_merge(&op)
        {
            group.after = after;
            return;
        }
        self.push_group(UndoGroup {
            ops: vec![op],
            kind,
            before,
            after,
        });
    }

    fn push_group(&mut self, group: UndoGroup) {
        self.group_open = group.kind != EditKind::Other;
        self.undo_stack.push(group);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
    }

    /// Replace `start..end` with `text` as one operation (or two, delete +
    /// insert, grouped together). Leaves the cursor after the new text.
    fn replace_range(&mut self, start: Pos, end: Pos, text: &str, kind: EditKind) {
        let before = self.cursor();
        let mut ops = Vec::new();
        if start < end {
            let removed = self.buffer.remove_range(start, end);
            ops.push(EditOperation::Delete {
                line: start.0,
                col: start.1,
                text: removed,
            });
        }
        let mut cur = start;
        if !text.is_empty() {
            cur = self.buffer.insert_text(start, text);
            ops.push(EditOperation::Insert {
                line: start.0,
                col: start.1,
                text: text.to_string(),
            });
        }
        self.anchor = None;
        self.cursor_line = cur.0;
        self.cursor_col = cur.1;
        match ops.len() {
            0 => return,
            1 => {
                if let Some(op) = ops.pop() {
                    self.record(op, kind, before);
                }
            },
            _ => {
                self.redo_stack.clear();
                self.modified = true;
                self.push_group(UndoGroup {
                    ops,
                    kind: EditKind::Other,
                    before,
                    after: cur,
                });
            },
        }
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    /// Insert `text` at the cursor, replacing the selection if any.
    pub fn insert_text(&mut self, text: &str) {
        let kind = if text.chars().count() == 1 && !text.contains('\n') {
            EditKind::Typing
        } else {
            EditKind::Other
        };
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (self.cursor(), self.cursor()));
        self.replace_range(start, end, text, kind);
    }

    /// Insert a character at the cursor (replacing any selection).
    pub fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        self.insert_text(ch.encode_utf8(&mut buf));
    }

    /// Insert spaces for the Tab key.
    pub(crate) fn insert_tab(&mut self) {
        self.insert_text(TAB_TEXT);
    }

    /// Delete the selection. Returns `false` when nothing was selected.
    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((start, end)) => {
                self.replace_range(start, end, "", EditKind::Other);
                true
            },
            None => false,
        }
    }

    /// Delete `start..end` as a (coalescing) deletion.
    fn delete_span(&mut self, start: Pos, end: Pos) {
        if start < end {
            self.replace_range(start, end, "", EditKind::Deleting);
        }
    }

    /// Backspace: delete the selection or the character before the cursor
    /// (joining with the previous line at column 0).
    pub fn delete_char(&mut self) {
        if !self.delete_selection() {
            let cur = self.cursor();
            self.delete_span(self.pos_left(), cur);
        }
    }

    /// Delete: remove the selection or the character at the cursor
    /// (joining with the next line at the end of a line).
    pub fn delete_forward(&mut self) {
        if !self.delete_selection() {
            let cur = self.cursor();
            self.delete_span(cur, self.pos_right());
        }
    }

    /// Ctrl+Backspace: delete back to the start of the previous word.
    pub fn delete_word_back(&mut self) {
        if !self.delete_selection() {
            let cur = self.cursor();
            self.delete_span(self.pos_word_left(), cur);
        }
    }

    /// Ctrl+Delete: delete forward to the end of the next word.
    pub fn delete_word_forward(&mut self) {
        if !self.delete_selection() {
            let cur = self.cursor();
            self.delete_span(cur, self.pos_word_right());
        }
    }

    /// Insert a newline at the cursor (Enter key), carrying the current
    /// line's indentation onto the new line.
    pub fn new_line(&mut self) {
        let line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        let indent_len = line.len() - line.trim_start().len();
        let indent = &line[..indent_len.min(self.cursor_col)];
        let mut text = String::with_capacity(1 + indent.len());
        text.push('\n');
        text.push_str(indent);
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (self.cursor(), self.cursor()));
        self.replace_range(start, end, &text, EditKind::Other);
    }

    // ---------------------------------------------------------------
    // Undo / Redo
    // ---------------------------------------------------------------

    /// Undo the last edit group (a word / typing burst, a deletion run, a
    /// paste, a replace-all, ...).
    pub fn undo(&mut self) {
        let Some(group) = self.undo_stack.pop() else {
            return;
        };
        for op in group.ops.iter().rev() {
            self.buffer.apply(&op.inverse());
        }
        self.restore_cursor(group.before);
        self.redo_stack.push(group);
    }

    /// Redo the last undone edit group.
    pub fn redo(&mut self) {
        let Some(group) = self.redo_stack.pop() else {
            return;
        };
        for op in &group.ops {
            self.buffer.apply(op);
        }
        self.restore_cursor(group.after);
        self.group_open = false;
        self.undo_stack.push(group);
    }

    fn restore_cursor(&mut self, pos: Pos) {
        let (line, col) = self.buffer.clamp_pos(pos);
        self.cursor_line = line;
        self.cursor_col = col;
        self.anchor = None;
        self.group_open = false;
        self.modified = true;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Clipboard
    // ---------------------------------------------------------------

    /// Copy the selection to the clipboard. Returns `false` if nothing is
    /// selected.
    pub fn copy(&mut self) -> bool {
        match self.selected_text() {
            Some(text) => {
                self.clipboard.copy(&text);
                self.status_message = Some("Copied.".into());
                self.rebuild_display_lines();
                true
            },
            None => false,
        }
    }

    /// Cut the selection to the clipboard.
    pub fn cut(&mut self) -> bool {
        if self.copy() {
            self.delete_selection();
            self.status_message = Some("Cut.".into());
            true
        } else {
            false
        }
    }

    /// Paste the clipboard at the cursor (replacing any selection).
    pub fn paste(&mut self) -> bool {
        let Some(text) = self.clipboard.paste() else {
            return false;
        };
        // Pasted text is one undo step, never merged with typing.
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (self.cursor(), self.cursor()));
        let text = text.replace("\r\n", "\n");
        self.replace_range(start, end, &text, EditKind::Other);
        true
    }

    // ---------------------------------------------------------------
    // Find / Replace
    // ---------------------------------------------------------------

    /// Search for the first occurrence of `query` from the start.
    ///
    /// Returns `true` if found (cursor is moved to the match, which is
    /// selected).
    pub fn find(&mut self, query: &str) -> bool {
        if query.is_empty() {
            return false;
        }
        self.find_query = query.to_string();
        self.find_active = true;
        self.select_match_from((0, 0))
    }

    /// Find the next occurrence after the cursor, wrapping around to the
    /// beginning of the buffer.
    pub fn find_next(&mut self) -> bool {
        if self.find_query.is_empty() {
            return false;
        }
        let (line, col) = self.cursor();
        let from = if col < self.buffer.line_len(line) {
            (line, self.buffer.next_col(line, col))
        } else if line + 1 < self.buffer.line_count() {
            (line + 1, 0)
        } else {
            (0, 0)
        };
        self.select_match_from(from)
    }

    /// Locate the first match at or after `from` (wrapping once).
    fn locate(&self, from: Pos) -> Option<Pos> {
        let query = self.find_query.as_str();
        let count = self.buffer.line_count();
        let from = self.buffer.clamp_pos(from);
        for line_idx in from.0..count {
            let line = self.buffer.get_line(line_idx).unwrap_or("");
            let start = if line_idx == from.0 { from.1 } else { 0 };
            if let Some(pos) = line[start..].find(query) {
                return Some((line_idx, start + pos));
            }
        }
        for line_idx in 0..=from.0 {
            let line = self.buffer.get_line(line_idx).unwrap_or("");
            let end = if line_idx == from.0 {
                (from.1 + query.len()).min(line.len())
            } else {
                line.len()
            };
            let end = line.floor_char_boundary(end);
            if let Some(pos) = line[..end].find(query) {
                return Some((line_idx, pos));
            }
        }
        None
    }

    /// Select the match found from `from`: the cursor sits at the match
    /// start and the anchor at its end.
    fn select_match_from(&mut self, from: Pos) -> bool {
        let Some(start) = self.locate(from) else {
            return false;
        };
        self.cursor_line = start.0;
        self.cursor_col = start.1;
        self.anchor = Some((start.0, start.1 + self.find_query.len()));
        self.group_open = false;
        self.ensure_cursor_visible();
        self.rebuild_display_lines();
        true
    }

    /// Replace the current match (if the selection is one) and move to the
    /// next match. Returns `true` when a replacement was made.
    pub fn replace_next(&mut self) -> bool {
        if self.find_query.is_empty() {
            return false;
        }
        let replaced = match self.selection() {
            Some((start, end)) if self.buffer.text_range(start, end) == self.find_query => {
                let replacement = self.replace_text.clone();
                self.replace_range(start, end, &replacement, EditKind::Other);
                true
            },
            _ => false,
        };
        let from = self.cursor();
        self.select_match_from(from);
        replaced
    }

    /// Replace every occurrence of the find query as a single undo step.
    /// Returns the number of replacements.
    pub fn replace_all(&mut self) -> usize {
        if self.find_query.is_empty() {
            return 0;
        }
        let before = self.cursor();
        let mut ops = Vec::new();
        let mut total = 0;
        for idx in 0..self.buffer.line_count() {
            let line = self.buffer.get_line(idx).unwrap_or("");
            let hits = line.matches(self.find_query.as_str()).count();
            if hits == 0 {
                continue;
            }
            total += hits;
            let new_line = line.replace(self.find_query.as_str(), &self.replace_text);
            let old_line = line.to_string();
            self.buffer.set_line(idx, new_line.clone());
            ops.push(EditOperation::Delete {
                line: idx,
                col: 0,
                text: old_line,
            });
            ops.push(EditOperation::Insert {
                line: idx,
                col: 0,
                text: new_line,
            });
        }
        if total > 0 {
            self.anchor = None;
            let (line, col) = self.buffer.clamp_pos(before);
            self.cursor_line = line;
            self.cursor_col = col;
            self.redo_stack.clear();
            self.modified = true;
            self.push_group(UndoGroup {
                ops,
                kind: EditKind::Other,
                before,
                after: (line, col),
            });
            self.ensure_cursor_visible();
        }
        self.rebuild_display_lines();
        total
    }

    // ---------------------------------------------------------------
    // Go to line
    // ---------------------------------------------------------------

    /// Jump to 1-based `line` (clamped), column 0.
    pub fn go_to_line(&mut self, line: usize) {
        let target = line.saturating_sub(1);
        self.move_to((target, 0), false);
    }

    // ---------------------------------------------------------------
    // Mode entry helpers
    // ---------------------------------------------------------------

    pub(crate) fn enter_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        self.status_message = None;
        match mode {
            EditorMode::Find | EditorMode::Replace => {
                self.find_active = true;
                self.replace_focus = false;
                // Seed the query from a single-line selection.
                if let Some(sel) = self.selected_text()
                    && !sel.contains('\n')
                {
                    self.find_query = sel;
                }
            },
            EditorMode::GoToLine => self.prompt_input.clear(),
            EditorMode::SaveAs => {
                self.prompt_input = self
                    .file_path
                    .clone()
                    .unwrap_or_else(|| "/home/user/untitled.txt".to_string());
            },
            _ => {},
        }
        self.rebuild_display_lines();
    }

    /// Leave any prompt / search mode back to Normal.
    pub(crate) fn leave_mode(&mut self) {
        if self.mode == EditorMode::SaveAs || self.mode == EditorMode::ConfirmDiscard {
            self.pending_close = None;
        }
        self.mode = EditorMode::Normal;
        self.find_active = false;
        self.status_message = None;
        self.rebuild_display_lines();
    }

    /// Commit the Go-to-line prompt.
    pub(crate) fn commit_go_to_line(&mut self) {
        match self.prompt_input.trim().parse::<usize>() {
            Ok(n) if n > 0 => {
                self.mode = EditorMode::Normal;
                self.go_to_line(n);
                self.status_message = None;
            },
            _ => self.status_message = Some("Enter a line number.".into()),
        }
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Save / Save As / close
    // ---------------------------------------------------------------

    /// Save to the current path, or open the Save As prompt when the
    /// document is untitled. The write happens in `apply_vfs_ops`.
    pub fn save(&mut self) {
        match self.file_path.clone() {
            Some(path) => {
                self.queue_save(path);
                self.mode = EditorMode::Normal;
            },
            None => self.enter_mode(EditorMode::SaveAs),
        }
        self.rebuild_display_lines();
    }

    /// Save under `path` (made absolute relative to the current file's
    /// directory), switching the document to that path.
    pub fn save_as(&mut self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() || path.ends_with('/') {
            self.status_message = Some("Enter a file name.".into());
            self.rebuild_display_lines();
            return false;
        }
        let full = if path.starts_with('/') {
            path.to_string()
        } else {
            let dir = self
                .file_path
                .as_deref()
                .and_then(|p| p.rsplit_once('/'))
                .map_or("", |(d, _)| d);
            format!("{dir}/{path}")
        };
        self.file_type = detect_file_type(&full);
        self.file_path = Some(full.clone());
        self.content.title = self.build_title();
        self.mode = EditorMode::Normal;
        self.queue_save(full);
        self.rebuild_display_lines();
        true
    }

    fn queue_save(&mut self, path: String) {
        self.status_message = Some(format!("Saving {path}..."));
        self.pending_save = Some((path, self.save_content(), self.buffer.generation()));
    }

    /// Write a queued save. Called from `App::apply_vfs_ops`.
    pub(crate) fn flush_save(&mut self, vfs: &mut dyn oasis_vfs::Vfs) -> bool {
        let Some((path, data, generation)) = self.pending_save.take() else {
            return false;
        };
        match vfs.write(&path, data.as_bytes()) {
            Ok(()) => {
                if self.buffer.generation() == generation {
                    self.modified = false;
                }
                self.status_message = Some(format!("Saved {path}"));
                match self.pending_close.take() {
                    Some(PendingClose::Exit) => {
                        self.close_requested = true;
                        self.status_message = Some("Saved. Closing...".into());
                    },
                    Some(PendingClose::New) => self.reset_document(),
                    None => {},
                }
            },
            Err(e) => {
                self.pending_close = None;
                self.status_message = Some(format!("Save failed: {e}"));
            },
        }
        self.rebuild_display_lines();
        true
    }

    /// Ask to close (or start a new document). With unsaved changes this
    /// opens the "Save / Discard / Cancel" prompt instead.
    pub(crate) fn request_close(&mut self, what: PendingClose) -> AppAction {
        if self.modified {
            self.pending_close = Some(what);
            self.mode = EditorMode::ConfirmDiscard;
            self.status_message = None;
            self.rebuild_display_lines();
            return AppAction::None;
        }
        self.finish_close(what)
    }

    fn finish_close(&mut self, what: PendingClose) -> AppAction {
        self.pending_close = None;
        match what {
            PendingClose::Exit => AppAction::Exit,
            PendingClose::New => {
                self.reset_document();
                AppAction::None
            },
        }
    }

    /// Unsaved-changes prompt: Save.
    pub(crate) fn confirm_save(&mut self) {
        self.mode = EditorMode::Normal;
        // `pending_close` stays set; `flush_save` completes it.
        self.save();
    }

    /// Unsaved-changes prompt: Discard.
    pub(crate) fn confirm_discard(&mut self) -> AppAction {
        self.mode = EditorMode::Normal;
        let what = self.pending_close.take().unwrap_or(PendingClose::Exit);
        self.modified = false;
        self.finish_close(what)
    }

    /// Replace the document with an empty, untitled one.
    pub(crate) fn reset_document(&mut self) {
        self.buffer = EditorBuffer::new();
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.anchor = None;
        self.content.scroll = 0;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.group_open = false;
        self.modified = false;
        self.file_path = None;
        self.file_type = crate::FileType::Plain;
        self.content.title = self.build_title();
        self.mode = EditorMode::Normal;
        self.status_message = Some("New document".into());
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Accessors
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

    /// Current cursor position (line, byte col).
    pub fn cursor_position(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }

    /// `true` once a "Save & close" finished writing: the host should close
    /// the editor. Hosts learn this through `App::take_close_request`
    /// (polled every frame after `apply_vfs_ops`); as a fallback for hosts
    /// that do not poll, every input hook also returns `AppAction::Exit`
    /// from then on.
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }
}

/// Cursor motions reachable from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Motion {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordLeft,
    WordRight,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}
