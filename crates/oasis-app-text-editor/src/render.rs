use crate::highlight::{SyntaxTheme, highlight_line};
use crate::{EditorMode, TextEditorApp};
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;

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

    /// Draw the editor content with syntax highlighting into a
    /// windowed region using per-span colored `draw_text` calls.
    pub(crate) fn draw_highlighted(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let font_size: u16 = 12;

        // Title row.
        let dir_suffix = if let Some(ref file) = self.content.viewing_file {
            format!("  [{file}]")
        } else {
            self.content
                .browse_dir
                .as_deref()
                .map(|d| format!("  [{d}]"))
                .unwrap_or_default()
        };
        let title_text = format!("{}{dir_suffix}", self.content.title);
        backend.draw_text(
            &title_text,
            cx + 4,
            cy + 2,
            font_size,
            at.app.title_bar_text,
        )?;

        // Separator line.
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            at.app.divider,
        )?;

        // Content area.
        let line_h = at.terminal_line_height.max(12) as i32;
        let content_top = cy + at.app.title_bar_height as i32;
        let max_lines = ((ch as i32 - line_h - 4) / line_h).max(0) as usize;

        let theme = SyntaxTheme::default();

        // Track block-comment state across visible lines. We need to
        // start from the first buffer line and track through to the
        // scroll position so that multi-line comments render correctly.
        let mut in_block_comment = false;
        for i in 0..self.content.scroll.min(self.buffer.line_count()) {
            if let Some(line_text) = self.buffer.get_line(i) {
                let (_, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;
            }
        }

        let visible = self
            .buffer
            .line_count()
            .saturating_sub(self.content.scroll)
            .min(max_lines);

        for i in 0..visible {
            let line_idx = self.content.scroll + i;
            let y = content_top + i as i32 * line_h;

            // Line number gutter: `>  1 | ` or `   1 | `.
            let marker = if line_idx == self.cursor_line {
                ">"
            } else {
                " "
            };
            let gutter = format!("{marker}{:>4} | ", line_idx + 1);

            let gutter_color = if i == self.content.cursor {
                at.app.selected_text
            } else {
                at.app.dim_text
            };
            backend.draw_text(&gutter, cx + 4, y, font_size, gutter_color)?;

            let gutter_px = backend.measure_text(&gutter, font_size) as i32;

            // Syntax-highlighted content.
            if let Some(line_text) = self.buffer.get_line(line_idx) {
                let (spans, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;

                let mut text_x = cx + 4 + gutter_px;
                for span in &spans {
                    let segment = &line_text[span.start..span.end];
                    if segment.is_empty() {
                        continue;
                    }
                    let color = theme.color_for(span.kind);
                    backend.draw_text(segment, text_x, y, font_size, color)?;
                    text_x += backend.measure_text(segment, font_size) as i32;
                }
            } else {
                in_block_comment = false;
            }
        }

        // Status bar line (last display line).
        let status_lines = &self.content.lines;
        if let Some(status) = status_lines.last() {
            let status_y = content_top + visible as i32 * line_h;
            backend.draw_text(status, cx + 4, status_y, font_size, at.app.dim_text)?;
        }

        // Scroll indicator.
        let scroll_text = if self.buffer.line_count() > max_lines {
            format!(
                "[{}/{}]  Cancel=back",
                self.content.scroll + 1,
                self.buffer.line_count().saturating_sub(max_lines) + 1,
            )
        } else {
            "Cancel=back".to_string()
        };
        let scroll_y = cy + ch as i32 - 14;
        backend.draw_text(&scroll_text, cx + 4, scroll_y, 10, at.app.dim_text)?;

        Ok(())
    }
}
