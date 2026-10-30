//! Tab completion cycling for [`LineEditor`].

use super::text::common_prefix;
use super::{EditResult, LineEditor};

impl LineEditor {
    // -- Tab completion ---------------------------------------------------

    pub(super) fn complete(&mut self, _history: &[String]) -> EditResult {
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

    pub(super) fn reset_completion(&mut self) {
        self.completion_candidates.clear();
        self.completion_index = 0;
        self.completion_base.clear();
    }
}
