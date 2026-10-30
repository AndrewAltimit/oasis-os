//! Cursor movement, deletion, kill ring and insertion for [`LineEditor`].

use super::LineEditor;
use super::text::{next_char_boundary, prev_char_boundary, word_end, word_start};

impl LineEditor {
    // -- Cursor movement (private) ----------------------------------------

    pub(super) fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    pub(super) fn move_to_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    pub(super) fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = prev_char_boundary(&self.buffer, self.cursor);
            self.cursor = prev;
        }
    }

    pub(super) fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            let next = next_char_boundary(&self.buffer, self.cursor);
            self.cursor = next;
        }
    }

    pub(super) fn move_word_left(&mut self) {
        self.cursor = word_start(&self.buffer, self.cursor);
    }

    pub(super) fn move_word_right(&mut self) {
        self.cursor = word_end(&self.buffer, self.cursor);
    }

    // -- Deletion (private) -----------------------------------------------

    pub(super) fn delete_char_back(&mut self) {
        if self.cursor > 0 {
            let prev = prev_char_boundary(&self.buffer, self.cursor);
            self.buffer.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    pub(super) fn delete_char_forward(&mut self) {
        if self.cursor < self.buffer.len() {
            let next = next_char_boundary(&self.buffer, self.cursor);
            self.buffer.drain(self.cursor..next);
        }
    }

    pub(super) fn delete_word_back(&mut self) {
        let start = word_start(&self.buffer, self.cursor);
        if start < self.cursor {
            self.kill_ring = self.buffer[start..self.cursor].to_string();
            self.buffer.drain(start..self.cursor);
            self.cursor = start;
        }
    }

    pub(super) fn kill_to_end(&mut self) {
        if self.cursor < self.buffer.len() {
            self.kill_ring = self.buffer[self.cursor..].to_string();
            self.buffer.truncate(self.cursor);
        }
    }

    pub(super) fn kill_to_start(&mut self) {
        if self.cursor > 0 {
            self.kill_ring = self.buffer[..self.cursor].to_string();
            self.buffer.drain(..self.cursor);
            self.cursor = 0;
        }
    }

    // -- Text insertion ---------------------------------------------------

    pub(super) fn insert_char(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    // -- Swap chars -------------------------------------------------------

    pub(super) fn swap_chars(&mut self) {
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
}
