//! History browsing and Ctrl+R reverse incremental search for
//! [`LineEditor`].

use super::{EditResult, LineEditor};

impl LineEditor {
    // -- History navigation -----------------------------------------------

    pub(super) fn history_prev(&mut self, history: &[String]) -> EditResult {
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

    pub(super) fn history_next(&mut self, history: &[String]) -> EditResult {
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

    pub(super) fn search_char(&mut self, ch: char, history: &[String]) -> EditResult {
        self.search_query.push(ch);
        self.find_search_match(history);
        EditResult::Continue
    }

    pub(super) fn search_next(&mut self, history: &[String]) -> EditResult {
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

    pub(super) fn accept_search(&mut self) -> EditResult {
        self.search_mode = false;
        self.search_query.clear();
        self.search_match_index = None;
        // Buffer already contains the matched line.
        self.cursor = self.buffer.len();
        EditResult::Continue
    }

    pub(super) fn cancel_search(&mut self) -> EditResult {
        self.search_mode = false;
        self.search_query.clear();
        self.search_match_index = None;
        self.buffer = self.saved_line.clone();
        self.cursor = self.buffer.len();
        EditResult::Continue
    }

    /// Search backwards through history starting at the current match
    /// position (or the end if no match yet).
    pub(super) fn find_search_match(&mut self, history: &[String]) {
        let start = self
            .search_match_index
            .unwrap_or(history.len().saturating_sub(1));
        self.find_search_match_from(history, start);
    }

    pub(super) fn find_search_match_from(&mut self, history: &[String], start: usize) {
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
    pub(super) fn check_match_at(&mut self, history: &[String]) -> bool {
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
}
