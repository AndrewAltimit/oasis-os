use crate::colors::EditorColors;
use crate::{EditorMode, FileType, TextEditorApp};
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;

/// Gap between the top of the text area and the first text row.
const NP_PAD_TOP: i32 = 6;
/// Menu bar and status strip heights of the windowed Notepad layout.
const NP_MENU_H: u32 = 18;
const NP_STATUS_H: u32 = 18;

/// Left inset of the text from the text area's edge.
pub(crate) const NP_PAD_LEFT: i32 = 8;
/// Font size of the text area.
pub(crate) const NP_FONT_SIZE: u16 = 12;
/// Room the caret keeps from the text area's right edge.
const H_SCROLL_MARGIN: u32 = 16;

/// Horizontal scroll (px) of a text area `cw` wide whose caret sits
/// `caret_px` from the text origin: 0 while the caret fits, otherwise just
/// enough to keep it [`H_SCROLL_MARGIN`] from the right edge. Stateless, so
/// drawing and click hit-testing always agree.
pub(crate) fn h_scroll(caret_px: u32, cw: u32) -> u32 {
    let room = cw.saturating_sub(NP_PAD_LEFT as u32 + H_SCROLL_MARGIN);
    caret_px.saturating_sub(room)
}

/// `(text rows, row height)` of the windowed editor viewport in a content
/// area `ch` tall: what `draw_notepad` draws and `App::layout` records.
pub(crate) fn notepad_viewport(ch: u32, at: &ActiveTheme) -> (usize, i32) {
    let area_h = ch.saturating_sub(NP_MENU_H + NP_STATUS_H) as i32;
    let line_h = at.terminal_line_height.max(14) as i32;
    (((area_h - NP_PAD_TOP) / line_h).max(0) as usize, line_h)
}

impl TextEditorApp {
    /// Format the buffer lines with line numbers for display.
    pub fn format_display_lines(&self) -> Vec<String> {
        (0..self.buffer.line_count())
            .map(|i| self.format_display_line(i))
            .collect()
    }

    /// Format one buffer line as `">   12 | text"` (marker on the cursor
    /// line).
    fn format_display_line(&self, i: usize) -> String {
        let text = self.buffer.get_line(i).unwrap_or("");
        let num = i + 1;
        let marker = if i == self.cursor_line { ">" } else { " " };
        format!("{marker}{num:>4} | {text}")
    }

    /// Text rows visible in the editor viewport: the last rendered
    /// viewport, or the fullscreen layout estimate before the first draw
    /// (minus the status line).
    pub(crate) fn page_lines(&self) -> usize {
        match self.viewport_lines {
            0 => self.content.cached_max_visible.max(2) - 1,
            n => n,
        }
    }

    /// Ensure the cursor line is within the visible scroll window.
    pub(crate) fn ensure_cursor_visible(&mut self) {
        let page = self.page_lines().max(1);
        if self.cursor_line < self.content.scroll {
            self.content.scroll = self.cursor_line;
        } else if self.cursor_line >= self.content.scroll + page {
            self.content.scroll = self.cursor_line + 1 - page;
        }
    }

    /// Short mode label for the status bar.
    fn mode_label(&self) -> &'static str {
        match self.mode {
            EditorMode::Normal => "Normal",
            EditorMode::Insert => "Insert",
            EditorMode::Find => "Find",
            EditorMode::Replace => "Replace",
            EditorMode::GoToLine => "Go to",
            EditorMode::SaveAs => "Save as",
            EditorMode::Saving => "Save?",
            EditorMode::ConfirmDiscard => "Unsaved",
        }
    }

    /// The find query as the prompt shows it: a primed (carried-over,
    /// about to be replaced) query is bracketed like a selection.
    fn query_field(&self) -> String {
        if self.query_primed {
            format!("[{}]", self.find_query)
        } else {
            self.find_query.clone()
        }
    }

    /// Left status-bar text: the active prompt, a status message, or the
    /// file summary.
    pub(crate) fn status_left(&self, file_summary: bool) -> String {
        let mode = self.mode_label();
        match self.mode {
            EditorMode::Find => {
                let caret = if self.query_primed { "" } else { "_" };
                return format!("{mode}  |  Find: {}{caret}", self.query_field());
            },
            EditorMode::Replace => {
                let (f, r) = match (self.replace_focus, self.query_primed) {
                    (true, _) => ("", "_"),
                    (false, true) => ("", ""),
                    (false, false) => ("_", ""),
                };
                return format!(
                    "{mode}  |  Find: {}{f}  Replace: {}{r}  (Enter / Ctrl+Enter all)",
                    self.query_field(),
                    self.replace_text
                );
            },
            EditorMode::GoToLine => return format!("Go to line: {}_", self.prompt_input),
            EditorMode::SaveAs => return format!("Save as: {}_", self.prompt_input),
            EditorMode::ConfirmDiscard => {
                return "Unsaved changes: [S]ave / [D]iscard / [Esc] Cancel".to_string();
            },
            _ => {},
        }
        if let Some(ref msg) = self.status_message {
            return format!("{mode}  |  {msg}");
        }
        let lines = self.buffer.line_count();
        if file_summary {
            let file_label = match &self.file_path {
                Some(fp) => fp.rsplit('/').next().unwrap_or(fp),
                None => "(untitled)",
            };
            let mod_marker = if self.modified { "*" } else { "" };
            format!("{mode}  |  {file_label}{mod_marker}  |  {lines} lines")
        } else {
            format!("{mode}  |  {lines} lines")
        }
    }

    /// Right status-bar text: 1-based line and character column.
    pub(crate) fn status_position(&self) -> String {
        let col = self.cursor_char_col();
        match self.selection() {
            Some((s, e)) => {
                let n = self.buffer.text_range(s, e).chars().count();
                format!(
                    "Ln {}, Col {}  ({n} selected)",
                    self.cursor_line + 1,
                    col + 1
                )
            },
            None => format!("Ln {}, Col {}", self.cursor_line + 1, col + 1),
        }
    }

    /// Byte range of `line_idx` covered by the selection, if any. The end
    /// is `None` when the selection continues past the end of the line.
    pub(crate) fn selection_on_line(&self, line_idx: usize) -> Option<(usize, Option<usize>)> {
        let (s, e) = self.selection()?;
        if line_idx < s.0 || line_idx > e.0 {
            return None;
        }
        let from = if line_idx == s.0 { s.1 } else { 0 };
        let to = if line_idx == e.0 { Some(e.1) } else { None };
        Some((from, to))
    }

    /// Rebuild the listing-style display lines (`App::lines`) and update
    /// ContentState.
    ///
    /// Only the visible window is formatted (plus the status line): this
    /// runs on every key press, and formatting the whole buffer made each
    /// keystroke O(file size).
    pub(crate) fn rebuild_display_lines(&mut self) {
        let count = self.buffer.line_count();
        let first = self.content.scroll.min(count);
        let end = first.saturating_add(self.page_lines()).min(count);
        let mut lines = std::mem::take(&mut self.content.lines);
        lines.clear();
        lines.extend((first..end).map(|i| self.format_display_line(i)));

        // Status bar line at the end.
        let mode_str = self.mode_label().to_uppercase();
        let mod_str = if self.modified { " [Modified]" } else { "" };
        let pos_str = format!(
            "Ln {}, Col {}",
            self.cursor_line + 1,
            self.cursor_char_col() + 1
        );
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

    /// Draw the full Windows-Notepad-style editor window: menu bar,
    /// text area (syntax-highlighted when applicable), and status bar.
    /// This is the primary windowed renderer for the Text Editor app.
    ///
    /// No inner title bar — the WM titlebar already shows the app title.
    /// The open file's name (and modified marker) live in the status bar.
    pub(crate) fn draw_notepad(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = EditorColors::from_theme(at);
        let body_bg = colors.bg;
        let body_fg = colors.text;
        let selection_bg = colors.selection_bg;
        let current_line_bg = colors.current_line_bg;

        // Menu bar at the very top: real widget with live drop-downs.
        let menu_h: u32 = 18;
        let menu_y = cy;
        let menu_style = colors.menu;
        self.menu
            .draw_bar(backend, cx, menu_y, cw, menu_h, &menu_style)?;

        // Text area.
        let area_y = menu_y + menu_h as i32;
        let status_h: u32 = 18;
        let area_h = (cy + ch as i32 - area_y - status_h as i32).max(0) as u32;
        backend.fill_rect(cx, area_y, cw, area_h, body_bg)?;

        let font_size = NP_FONT_SIZE;
        let pad_top = NP_PAD_TOP;
        // Scroll horizontally so a caret past the right edge stays visible;
        // the text area is clipped so scrolled text can't spill out.
        let caret_line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        let caret_px = backend.measure_text(
            &caret_line[..caret_line.floor_char_boundary(self.cursor_col)],
            font_size,
        );
        let pad_left = NP_PAD_LEFT - h_scroll(caret_px, cw) as i32;
        backend.push_clip_rect(cx, area_y, cw, area_h)?;
        // Same viewport `App::layout` recorded for scrolling and clicks.
        let (max_lines, line_h) = notepad_viewport(ch, at);
        let first = self.content.scroll;
        let visible = self
            .buffer
            .line_count()
            .saturating_sub(first)
            .min(max_lines);

        // Syntax spans for the visible window come from the buffer's cache:
        // block-comment state above the viewport is remembered per line and
        // the window's spans are re-used until an edit or scroll.
        let theme = &colors.syntax;
        let highlighted = self.file_type != FileType::Plain;
        let window = if highlighted { visible } else { 0 };
        let drawn = self
            .buffer
            .with_visible_spans(self.file_type, first, window, |spans| {
                for i in 0..visible {
                    let line_idx = first + i;
                    let y = area_y + pad_top + i as i32 * line_h;

                    // Current-line highlight on the active line.
                    if line_idx == self.cursor_line {
                        backend.fill_rect(cx, y - 1, cw, line_h as u32, current_line_bg)?;
                    }

                    let Some(line_text) = self.buffer.get_line(line_idx) else {
                        continue;
                    };

                    // Selection band behind the text.
                    if let Some((from, to)) = self.selection_on_line(line_idx) {
                        let from = line_text.floor_char_boundary(from);
                        let x0 = backend.measure_text(&line_text[..from], font_size) as i32;
                        let x1 = match to {
                            Some(to) => {
                                let to = line_text.floor_char_boundary(to);
                                backend.measure_text(&line_text[..to], font_size) as i32
                            },
                            // Selected newline: extend a little past the text.
                            None => backend.measure_text(line_text, font_size) as i32 + 6,
                        };
                        if x1 > x0 {
                            backend.fill_rect(
                                cx + pad_left + x0,
                                y - 1,
                                (x1 - x0) as u32,
                                line_h as u32,
                                selection_bg,
                            )?;
                        }
                    }

                    match spans.get(i) {
                        Some(line_spans) => {
                            let mut text_x = cx + pad_left;
                            for span in line_spans {
                                let segment = &line_text[span.start..span.end];
                                if segment.is_empty() {
                                    continue;
                                }
                                let color = theme.color_for(span.kind);
                                backend.draw_text(segment, text_x, y, font_size, color)?;
                                text_x += backend.measure_text(segment, font_size) as i32;
                            }
                        },
                        None => {
                            backend.draw_text(line_text, cx + pad_left, y, font_size, body_fg)?;
                        },
                    }

                    // Cursor caret on the active line — a solid 2px bar,
                    // accent-colored in Insert mode and dimmer in Normal
                    // mode. Kept opaque rather than blinking to avoid
                    // burning frame time on a redraw just for the caret.
                    if line_idx == self.cursor_line {
                        // Byte columns: the prefix is a zero-copy slice.
                        let prefix = &line_text[..line_text.floor_char_boundary(self.cursor_col)];
                        let caret_x =
                            cx + pad_left + backend.measure_text(prefix, font_size) as i32;
                        let caret_color = if self.mode == EditorMode::Insert {
                            colors.caret
                        } else {
                            colors.caret_normal
                        };
                        backend.fill_rect(caret_x, y - 1, 2, line_h as u32, caret_color)?;
                    }
                }
                Ok::<(), oasis_types::error::OasisError>(())
            });
        backend.pop_clip_rect()?;
        drawn?;

        // Status bar.
        let status_y = cy + ch as i32 - status_h as i32;
        let status_fg = colors.status_text;
        backend.fill_rect(cx, status_y, cw, status_h, colors.status_bg)?;
        backend.fill_rect(cx, status_y, cw, 1, colors.border)?;

        // The file name lives here now that there's no inner title bar.
        let position = self.status_position();
        let status_left = self.status_left(true);
        backend.draw_text_ellipsis(
            &status_left,
            cx + 6,
            status_y + 4,
            11,
            status_fg,
            cw.saturating_sub(12),
        )?;

        let left_w = backend.measure_text(&status_left, 11);
        let pos_w = backend.measure_text(&position, 11);
        let pos_x = cx + cw as i32 - pos_w as i32 - 8;
        if status_position_fits(cx + 6, left_w, pos_x) {
            backend.draw_text(&position, pos_x, status_y + 4, 11, status_fg)?;
        }

        // Drop-down must render ABOVE the text area and status bar.
        // Draw it last so it naturally layers on top without needing
        // any z-ordering from the backend.
        if self.menu.is_open() {
            self.menu
                .draw_dropdown(backend, (cx, menu_y, cw, menu_h), &menu_style)?;
        }

        Ok(())
    }
}

/// Gap kept between the left status text and the right-aligned
/// "Ln / Col" readout.
const STATUS_GAP: i32 = 8;

/// Whether the "Ln / Col" readout at `pos_x` clears the left status text
/// (`left_w` px wide from `left_x`). A long prompt ("Unsaved changes:
/// [S]ave / [D]iscard / [Esc] Cancel", a Save-as path) wins: the readout
/// is dropped instead of being drawn on top of it.
fn status_position_fits(left_x: i32, left_w: u32, pos_x: i32) -> bool {
    left_x
        .saturating_add(left_w as i32)
        .saturating_add(STATUS_GAP)
        <= pos_x
}

#[cfg(test)]
mod tests {
    use oasis_app_core::App;
    use oasis_skin::ActiveTheme;
    use oasis_test_backend::{DrawCommand, RecordingBackend};
    use oasis_types::backend::Color;

    use crate::TextEditorApp;

    fn dark_theme() -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.app.bg = Color::rgb(20, 22, 30);
        at.app.text = Color::rgb(210, 210, 220);
        at
    }

    fn fills(rec: &RecordingBackend) -> Vec<(i32, i32, u32, Color)> {
        rec.commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::FillRect { x, y, w, color, .. } => Some((*x, *y, *w, *color)),
                _ => None,
            })
            .collect()
    }

    /// The windowed text area must paint the skin's app background, not a
    /// hardcoded white box.
    #[test]
    fn windowed_body_background_follows_dark_theme() {
        let at = dark_theme();
        let app = TextEditorApp::open_file("/a.rs", "fn main() {}\n// hi");
        let mut rec = RecordingBackend::new(400, 300);
        app.draw_windowed(0, 0, 400, 300, &mut rec, &at)
            .expect("draw");
        let fills = fills(&rec);
        // Text area starts right under the 18px menu bar and spans the width.
        let area = fills
            .iter()
            .find(|(x, y, w, _)| *x == 0 && *y == 18 && *w == 400)
            .expect("text area fill");
        assert_eq!(area.3, at.app.bg);
        let white = Color::rgb(255, 255, 255);
        assert!(
            fills.iter().all(|f| f.3 != white || f.1 < 18),
            "no white fills below the menu bar"
        );
    }

    /// Texts drawn on the status strip row (`y`), as `(text, x)`.
    fn status_texts(rec: &RecordingBackend, y: i32) -> Vec<(String, i32)> {
        rec.commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText { text, x, y: ty, .. } if *ty == y => {
                    Some((text.clone(), *x))
                },
                _ => None,
            })
            .collect()
    }

    /// The unsaved-changes prompt used to be drawn under the right-aligned
    /// "Ln / Col" readout in a narrow window. The prompt wins: the readout
    /// is dropped while it would overlap, and comes back with room.
    #[test]
    fn unsaved_prompt_does_not_overlap_the_position_readout() {
        let at = ActiveTheme::default();
        let mut app = TextEditorApp::open_file("/a.txt", "words");
        app.mode = crate::EditorMode::ConfirmDiscard;
        let (w, h) = (300, 200);
        let status_y = h as i32 - 18 + 4;

        let mut rec = RecordingBackend::new(w, h);
        app.draw_windowed(0, 0, w, h, &mut rec, &at).expect("draw");
        let row = status_texts(&rec, status_y);
        assert!(
            row.iter().any(|(t, _)| t.starts_with("Unsaved changes")),
            "{row:?}"
        );
        assert!(
            row.iter().all(|(t, _)| !t.starts_with("Ln ")),
            "the readout overlapped the prompt: {row:?}"
        );

        // A wide window has room for both, side by side.
        let mut rec = RecordingBackend::new(900, h);
        app.draw_windowed(0, 0, 900, h, &mut rec, &at)
            .expect("draw");
        let row = status_texts(&rec, status_y);
        let prompt = row.iter().find(|(t, _)| t.starts_with("Unsaved")).unwrap();
        let pos = row.iter().find(|(t, _)| t.starts_with("Ln ")).unwrap();
        let prompt_end =
            prompt.1 + oasis_types::backend::SdiCore::measure_text(&rec, &prompt.0, 11) as i32;
        assert!(prompt_end < pos.1, "{row:?}");
    }

    #[test]
    fn windowed_text_uses_theme_text_color() {
        let at = dark_theme();
        let app = TextEditorApp::open_file("/a.txt", "plain words");
        let mut rec = RecordingBackend::new(400, 300);
        app.draw_windowed(0, 0, 400, 300, &mut rec, &at)
            .expect("draw");
        let text_color = rec.commands().iter().find_map(|c| match c {
            DrawCommand::DrawText { text, color, .. } if text == "plain words" => Some(*color),
            _ => None,
        });
        assert_eq!(text_color, Some(at.app.text));
    }
}
