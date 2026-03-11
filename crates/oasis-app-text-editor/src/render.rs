use crate::{EditorMode, TextEditorApp};

impl TextEditorApp {
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

    /// Clamp cursor column to the current line length.
    pub(crate) fn clamp_cursor_col(&mut self) {
        let len = self.buffer.line_len(self.cursor_line);
        if self.cursor_col > len {
            self.cursor_col = len;
        }
    }

    /// Ensure the cursor line is within the visible scroll window.
    pub(crate) fn ensure_cursor_visible(&mut self) {
        let max_vis = self.content.cached_max_visible.max(1).saturating_sub(1); // reserve 1 line for status
        if self.cursor_line < self.content.scroll {
            self.content.scroll = self.cursor_line;
        } else if self.cursor_line >= self.content.scroll + max_vis {
            self.content.scroll = self.cursor_line.saturating_sub(max_vis - 1);
        }
    }

    /// Rebuild the display lines from the buffer and update
    /// ContentState for rendering.
    pub(crate) fn rebuild_display_lines(&mut self) {
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
    pub(crate) fn build_title(&self) -> String {
        match &self.file_path {
            Some(fp) => {
                let name = fp.rsplit('/').next().unwrap_or(fp);
                format!("Text Editor - {name}")
            },
            None => "Text Editor".to_string(),
        }
    }
}
