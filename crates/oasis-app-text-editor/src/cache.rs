//! Syntax-highlight cache owned by [`EditorBuffer`](crate::EditorBuffer).
//!
//! Two layers:
//!
//! - **Block-comment state per line** (`starts[i]` = line `i` begins inside
//!   a `/* ... */` comment). Valid for a prefix of the buffer; an edit on
//!   line `n` truncates it to `n + 1` entries (lines above the edit are
//!   unaffected), and it is extended lazily up to the first visible line.
//! - **Spans of the last drawn window**, keyed on the buffer generation,
//!   file type and window. A steady frame (no edit, no scroll) re-uses them
//!   without calling the highlighter at all.

use crate::highlight::{ColorSpan, FileType, highlight_line};

/// Whether a line's highlighting can depend on the lines above it.
fn has_block_comments(file_type: FileType) -> bool {
    matches!(
        file_type,
        FileType::Rust | FileType::C | FileType::JavaScript | FileType::Css
    )
}

/// Highlighted spans for a window of lines.
#[derive(Debug, Clone, Default)]
struct SpanWindow {
    valid: bool,
    generation: u64,
    first: usize,
    count: usize,
    spans: Vec<Vec<ColorSpan>>,
}

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub(crate) struct HighlightCache {
    file_type: Option<FileType>,
    starts: Vec<bool>,
    window: SpanWindow,
    /// Number of `highlight_line` calls made (for tests / diagnostics).
    calls: u64,
}

impl HighlightCache {
    /// Drop cached state for `line` and everything below it.
    pub(crate) fn invalidate_from(&mut self, line: usize) {
        self.starts.truncate(line + 1);
        self.window.valid = false;
    }

    /// Reset everything when the file type changes (e.g. after Save As).
    fn sync_file_type(&mut self, file_type: FileType) {
        if self.file_type != Some(file_type) {
            self.file_type = Some(file_type);
            self.starts.clear();
            self.window.valid = false;
        }
    }

    /// Whether `line` starts inside a block comment.
    fn start_state(&mut self, lines: &[String], file_type: FileType, line: usize) -> bool {
        self.sync_file_type(file_type);
        if !has_block_comments(file_type) {
            return false;
        }
        if self.starts.is_empty() {
            self.starts.push(false);
        }
        let target = line.min(lines.len());
        while self.starts.len() <= target {
            let i = self.starts.len() - 1;
            let state = self.starts[i];
            let (_, end) = highlight_line(&lines[i], file_type, state);
            self.calls += 1;
            self.starts.push(end);
        }
        self.starts.get(target).copied().unwrap_or(false)
    }

    /// Spans for lines `first..first + count` (clamped to the buffer),
    /// recomputed only when the key changed.
    pub(crate) fn visible(
        &mut self,
        lines: &[String],
        file_type: FileType,
        generation: u64,
        first: usize,
        count: usize,
    ) -> &[Vec<ColorSpan>] {
        self.sync_file_type(file_type);
        let w = &self.window;
        if w.valid && w.generation == generation && w.first == first && w.count == count {
            return &self.window.spans;
        }
        let end = first.saturating_add(count).min(lines.len());
        let mut state = self.start_state(lines, file_type, first);
        let mut spans = std::mem::take(&mut self.window.spans);
        spans.clear();
        let track = has_block_comments(file_type);
        for (i, line) in lines.iter().enumerate().take(end).skip(first) {
            let (line_spans, next) = highlight_line(line, file_type, state);
            self.calls += 1;
            spans.push(line_spans);
            state = next;
            // Extend the per-line state for free while we are here.
            if track && self.starts.len() == i + 1 {
                self.starts.push(next);
            }
        }
        self.window = SpanWindow {
            valid: true,
            generation,
            first,
            count,
            spans,
        };
        &self.window.spans
    }

    /// Number of `highlight_line` calls made so far.
    pub(crate) fn calls(&self) -> u64 {
        self.calls
    }
}

#[cfg(test)]
mod tests {
    use oasis_app_core::App;
    use oasis_skin::ActiveTheme;
    use oasis_test_backend::RecordingBackend;
    use oasis_types::input::{Key, Modifiers};
    use oasis_vfs::MemoryVfs;

    use crate::highlight::{ColorSpan, FileType, highlight_line};
    use crate::{EditorBuffer, TextEditorApp};

    /// Uncached reference: walk block-comment state from line 0.
    fn reference(buf: &EditorBuffer, ft: FileType, first: usize, n: usize) -> Vec<Vec<ColorSpan>> {
        let mut state = false;
        let mut out = Vec::new();
        for i in 0..buf.line_count().min(first + n) {
            let (spans, next) = highlight_line(buf.get_line(i).unwrap_or(""), ft, state);
            if i >= first {
                out.push(spans);
            }
            state = next;
        }
        out
    }

    fn cached(buf: &EditorBuffer, ft: FileType, first: usize, n: usize) -> Vec<Vec<ColorSpan>> {
        buf.with_visible_spans(ft, first, n, <[Vec<ColorSpan>]>::to_vec)
    }

    fn c_file(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("int v{i} = {i}; /* c */"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn edits_above_viewport_invalidate_block_state() {
        let ft = FileType::C;
        let mut buf = EditorBuffer::from_text(&c_file(40));
        assert_eq!(cached(&buf, ft, 20, 10), reference(&buf, ft, 20, 10));
        // Open a block comment above the viewport: everything below turns
        // into comment text.
        buf.insert_text((2, 0), "/* open ");
        assert_eq!(cached(&buf, ft, 20, 10), reference(&buf, ft, 20, 10));
        // Close it again inside the viewport.
        buf.insert_text((25, 0), "*/");
        assert_eq!(cached(&buf, ft, 20, 10), reference(&buf, ft, 20, 10));
        // Delete lines spanning the opener.
        buf.remove_range((1, 0), (3, 0));
        assert_eq!(cached(&buf, ft, 20, 10), reference(&buf, ft, 20, 10));
        // Undo-style re-insert of multiple lines.
        buf.insert_text((1, 0), "a\n/* b\nc */\n");
        for first in [0, 5, 18, 30] {
            assert_eq!(cached(&buf, ft, first, 10), reference(&buf, ft, first, 10));
        }
    }

    #[test]
    fn file_type_change_resets_cache() {
        let mut buf = EditorBuffer::from_text("/* a\nb */\nc");
        let _ = cached(&buf, FileType::C, 1, 2);
        assert_eq!(
            cached(&buf, FileType::Toml, 1, 2),
            reference(&buf, FileType::Toml, 1, 2)
        );
        buf.set_line(0, "x".into());
        assert_eq!(
            cached(&buf, FileType::C, 1, 2),
            reference(&buf, FileType::C, 1, 2)
        );
    }

    #[test]
    fn steady_frames_do_not_rehighlight() {
        let at = ActiveTheme::default();
        let mut app = TextEditorApp::open_file("/big.c", &c_file(5_000));
        let mut rec = RecordingBackend::new(800, 600);
        app.draw_windowed(0, 0, 800, 600, &mut rec, &at)
            .expect("draw");
        let _ = app.handle_key(&Key::End, Modifiers::CTRL, &MemoryVfs::new());
        app.draw_windowed(0, 0, 800, 600, &mut rec, &at)
            .expect("draw");
        let after_scroll = app.buffer.highlight_calls();
        for _ in 0..5 {
            app.draw_windowed(0, 0, 800, 600, &mut rec, &at)
                .expect("draw");
        }
        assert_eq!(app.buffer.highlight_calls(), after_scroll);
        // Typing on the last line only re-highlights the visible window.
        app.handle_text_input('x');
        app.draw_windowed(0, 0, 800, 600, &mut rec, &at)
            .expect("draw");
        let delta = app.buffer.highlight_calls() - after_scroll;
        assert!(delta <= 64, "re-highlighted {delta} lines for one edit");
    }

    #[test]
    fn display_lines_only_cover_the_viewport() {
        let mut app = TextEditorApp::open_file("/big.txt", &c_file(5_000));
        app.content.cached_max_visible = 21;
        app.rebuild_display_lines();
        // 20 visible rows + the status line, not 5000.
        assert_eq!(app.content.lines.len(), 21);
        assert!(app.content.lines[0].contains("int v0"));
    }
}
