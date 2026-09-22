use crate::colors::EditorColors;
use crate::{EditorMode, FileType, TextEditorApp};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_ui::menu_bar::MenuStyle;

/// How many editor lines the SDI notepad renderer can show at once.
/// Used for both sizing (we create objects up front) and teardown.
const NP_MAX_VISIBLE_LINES: usize = 64;

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
        match self.viewport_lines.get() {
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

    /// Left status-bar text: the active prompt, a status message, or the
    /// file summary.
    pub(crate) fn status_left(&self, file_summary: bool) -> String {
        let mode = self.mode_label();
        match self.mode {
            EditorMode::Find => return format!("{mode}  |  Find: {}_", self.find_query),
            EditorMode::Replace => {
                let (f, r) = if self.replace_focus {
                    ("", "_")
                } else {
                    ("_", "")
                };
                return format!(
                    "{mode}  |  Find: {}{f}  Replace: {}{r}  (Enter / Ctrl+Enter all)",
                    self.find_query, self.replace_text
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
        let line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        let col = line[..line.floor_char_boundary(self.cursor_col)]
            .chars()
            .count();
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

        let font_size: u16 = 12;
        let line_h = at.terminal_line_height.max(14) as i32;
        let pad_left = 8i32;
        let pad_top = 6i32;

        let max_lines = ((area_h as i32 - pad_top) / line_h).max(0) as usize;
        self.viewport_lines.set(max_lines.max(1));
        self.viewport_line_h.set(line_h);
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
        self.buffer
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
            })?;

        // Status bar.
        let status_y = cy + ch as i32 - status_h as i32;
        let status_fg = colors.status_text;
        backend.fill_rect(cx, status_y, cw, status_h, colors.status_bg)?;
        backend.fill_rect(cx, status_y, cw, 1, colors.border)?;

        // The file name lives here now that there's no inner title bar.
        let position = self.status_position();
        let status_left = self.status_left(true);
        backend.draw_text(&status_left, cx + 6, status_y + 4, 11, status_fg)?;

        let pos_w = backend.measure_text(&position, 11);
        backend.draw_text(
            &position,
            cx + cw as i32 - pos_w as i32 - 8,
            status_y + 4,
            11,
            status_fg,
        )?;

        // Drop-down must render ABOVE the text area and status bar.
        // Draw it last so it naturally layers on top without needing
        // any z-ordering from the backend.
        if self.menu.is_open() {
            self.menu
                .draw_dropdown(backend, cx, menu_y, menu_h, &menu_style)?;
        }

        Ok(())
    }

    /// Render the full-screen Notepad GUI as SDI objects. Mirrors what
    /// `draw_notepad` produces for windowed mode, but emits named SDI
    /// objects instead of direct backend draw calls so it survives the
    /// classic-skin fullscreen render path.
    pub(crate) fn render_notepad_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Defensive: hide the generic content-listing SDI objects that
        // a previously-open app may have populated and left visible.
        // Without this, stale `app_line_*` text bleeds through our
        // white text area because we never repopulate them ourselves.
        for name in ["app_sel_bg", "app_sel_accent", "app_scroll", "app_divider"] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }

        let colors = EditorColors::from_theme(at);
        let body_bg = colors.bg;
        let body_fg = colors.text;
        let chrome_bg = colors.menu.bar_bg;
        let chrome_border = colors.menu.bar_border;
        let status_bg = colors.status_bg;
        let status_fg = colors.status_text;
        let selection_bg = colors.selection_bg;
        let current_line_bg = colors.current_line_bg;

        let sw = at.screen_w;
        let sh = at.screen_h;
        let title_h = at.app.title_bar_height.max(18);
        let menu_h: u32 = 18;
        let status_h: u32 = 18;
        let bottom_reserved = at.statusbar_height + at.bottombar_height;
        let menu_y = title_h as i32;
        let area_y = menu_y + menu_h as i32;
        let area_h = sh
            .saturating_sub(title_h + menu_h + status_h + bottom_reserved)
            .max(1);
        let status_y = area_y + area_h as i32;

        // Override the default title text (render_app_chrome's
        // `app_title_text` is already set by the `render_content_sdi`
        // path we bypass; keep it populated so the chrome shows the
        // editor's filename).
        if !sdi.contains("app_title_text") {
            sdi.create("app_title_text");
        }
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            let mod_marker = if self.modified { " *" } else { "" };
            obj.text = Some(format!("{}{mod_marker}", self.build_title()));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = at.app.title_bar_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        // Menu bar strip + labels with open-state highlight.
        rect(sdi, "np_menu_bg", 0, menu_y, sw, menu_h, chrome_bg, 103);
        rect(
            sdi,
            "np_menu_border",
            0,
            menu_y + menu_h as i32 - 1,
            sw,
            1,
            chrome_border,
            104,
        );
        let label_hot_bg = colors.menu.label_hot_bg;
        let label_hot_text = colors.menu.label_hot_text;
        let label_text = colors.menu.label_text;
        let mut mx = 6i32;
        for (i, m) in self.menu.menus.iter().enumerate() {
            let label_w = m.label.chars().count() as i32 * 7 + 16;
            let is_open = self.menu.open == Some(i);
            // Open-menu highlight strip — one SDI rect per slot,
            // toggled via `visible` so we reuse the object across
            // frames without churning the registry.
            let hot_name = format!("np_menu_hot_{i}");
            rect_visible(
                sdi,
                &hot_name,
                mx,
                menu_y + 2,
                label_w as u32,
                menu_h - 4,
                label_hot_bg,
                104,
                is_open,
            );
            let label_name = format!("np_menu_{i}");
            let text_color = if is_open { label_hot_text } else { label_text };
            text(
                sdi,
                &label_name,
                mx + 8,
                menu_y + 3,
                11,
                text_color,
                &m.label,
                105,
            );
            mx += label_w;
        }

        // Drop-down overlay: a bordered rect + one item row per
        // entry. Rendered with higher z than the text area so items
        // float above buffer content.
        self.render_dropdown_sdi(sdi, menu_y, menu_h, &colors.menu);

        // Text area background.
        rect(sdi, "np_area_bg", 0, area_y, sw, area_h, body_bg, 103);

        // Current-line selection highlight.
        let line_h = at.terminal_line_height.max(14);
        let pad_top = 6i32;
        let pad_left = 8i32;
        let max_lines = ((area_h as i32 - pad_top) / line_h as i32).max(0) as usize;
        let max_lines = max_lines.min(NP_MAX_VISIBLE_LINES);
        self.viewport_lines.set(max_lines.max(1));
        self.viewport_line_h.set(line_h as i32);
        let visible = self
            .buffer
            .line_count()
            .saturating_sub(self.content.scroll)
            .min(max_lines);

        // Selection bar on active line.
        let rel_line = self.cursor_line.saturating_sub(self.content.scroll);
        let sel_visible = self.cursor_line >= self.content.scroll && rel_line < visible;
        let sel_y = area_y + pad_top + rel_line as i32 * line_h as i32 - 1;
        rect_visible(
            sdi,
            "np_sel_bg",
            0,
            sel_y,
            sw,
            line_h,
            current_line_bg,
            104,
            sel_visible,
        );

        let theme = &colors.syntax;

        // First-span color per visible line, from the highlight cache.
        let mut first_colors = [None; NP_MAX_VISIBLE_LINES];
        if self.file_type != FileType::Plain {
            self.buffer
                .with_visible_spans(self.file_type, self.content.scroll, visible, |spans| {
                    for (slot, line_spans) in first_colors.iter_mut().zip(spans) {
                        *slot = line_spans.first().map(|s| theme.color_for(s.kind));
                    }
                });
        }

        // Visible text lines.
        for i in 0..NP_MAX_VISIBLE_LINES {
            let name = format!("np_line_{i}");
            let sel_name = format!("np_selrange_{i}");
            if i >= visible {
                hide(sdi, &name);
                hide(sdi, &sel_name);
                continue;
            }
            let line_idx = self.content.scroll + i;
            let y = area_y + pad_top + i as i32 * line_h as i32;
            let Some(line_text) = self.buffer.get_line(line_idx) else {
                hide(sdi, &name);
                hide(sdi, &sel_name);
                continue;
            };

            // Selection band (7px per char, like the SDI caret).
            match self.selection_on_line(line_idx) {
                Some((from, to)) => {
                    let chars_to = |col: usize| {
                        line_text[..line_text.floor_char_boundary(col)]
                            .chars()
                            .count() as i32
                    };
                    let x0 = chars_to(from) * 7;
                    let x1 = match to {
                        Some(to) => chars_to(to) * 7,
                        None => line_text.chars().count() as i32 * 7 + 6,
                    };
                    rect_visible(
                        sdi,
                        &sel_name,
                        pad_left + x0,
                        y - 1,
                        (x1 - x0).max(0) as u32,
                        line_h,
                        selection_bg,
                        104,
                        x1 > x0,
                    );
                },
                None => hide(sdi, &sel_name),
            }

            // Single-color display for plain files or simple fallback.
            // Syntax highlighting in SDI mode would need one object per
            // span — skip it here; windowed mode still renders colors.
            let display = if line_text.is_empty() {
                " ".to_string()
            } else {
                line_text.to_string()
            };
            // Rough: first-token color for highlighted files (cached
            // spans), body foreground for plain text. Cheap and visibly
            // distinct from plain text.
            let color = first_colors.get(i).copied().flatten().unwrap_or(body_fg);
            text(sdi, &name, pad_left, y, 12, color, &display, 105);
        }

        // Caret on active line (thin vertical bar).
        let caret_color = if self.mode == EditorMode::Insert {
            colors.caret
        } else {
            colors.caret_normal
        };
        let caret_visible = sel_visible;
        let caret_line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        // Approximate caret x: monospaced-ish width per char; the
        // windowed path uses `measure_text` but we don't have a backend
        // here. 7px/char at size 12 is close enough for the SDI path
        // and matches the bitmap font used by the backends at this size.
        let prefix_chars = caret_line[..caret_line.floor_char_boundary(self.cursor_col)]
            .chars()
            .count() as i32;
        let caret_x = pad_left + prefix_chars * 7;
        let caret_y = area_y + pad_top + rel_line as i32 * line_h as i32 - 1;
        rect_visible(
            sdi,
            "np_caret",
            caret_x,
            caret_y,
            2,
            line_h,
            caret_color,
            106,
            caret_visible,
        );

        // Status bar.
        rect(
            sdi,
            "np_status_bg",
            0,
            status_y,
            sw,
            status_h,
            status_bg,
            103,
        );
        rect(
            sdi,
            "np_status_border",
            0,
            status_y,
            sw,
            1,
            colors.border,
            104,
        );

        let status_left = self.status_left(false);
        text(
            sdi,
            "np_status_left",
            6,
            status_y + 4,
            11,
            status_fg,
            &status_left,
            105,
        );

        let position = self.status_position();
        let right_x = (sw as i32) - (position.chars().count() as i32 * 6) - 8;
        text(
            sdi,
            "np_status_right",
            right_x,
            status_y + 4,
            11,
            status_fg,
            &position,
            105,
        );
    }

    /// Render the active drop-down (if any) as SDI objects. Uses a
    /// fixed `NP_MAX_ENTRIES` pool of item names so the registry
    /// churn stays bounded across frames.
    fn render_dropdown_sdi(
        &self,
        sdi: &mut SdiRegistry,
        menu_y: i32,
        menu_h: u32,
        style: &MenuStyle,
    ) {
        // Hide every pooled item first, then repopulate only the
        // slots we actually need this frame.
        for i in 0..NP_MAX_DROPDOWN_ENTRIES {
            for kind in ["hot", "text", "shortcut", "sep"] {
                let name = format!("np_dd_{kind}_{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
        for name in ["np_dd_bg", "np_dd_border_l", "np_dd_border_d"] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }

        let Some(idx) = self.menu.open else {
            return;
        };
        let menu = &self.menu.menus[idx];

        // Compute label x to anchor the drop-down.
        let mut label_x = 6i32;
        for i in 0..idx {
            label_x += self.menu.menus[i].label.chars().count() as i32 * 7 + 16;
        }

        let dd_x = label_x;
        let dd_y = menu_y + menu_h as i32;
        let (dd_w, dd_h) = self.menu.dropdown_dimensions(menu);

        let bg = style.dropdown_bg;
        let light = style.dropdown_border_light;
        let dark = style.dropdown_border_dark;
        let item_text_color = style.item_text;
        let item_hot_bg = style.item_hot_bg;
        let item_hot_text = style.item_hot_text;
        let disabled = style.item_disabled_text;
        let sep_color = style.separator;

        // Background + bezel. Very high z so items float above the
        // text area (z=103) and line text (z=105).
        rect(sdi, "np_dd_bg", dd_x, dd_y, dd_w, dd_h, bg, 150);
        rect(sdi, "np_dd_border_l", dd_x, dd_y, dd_w, 1, light, 151);
        rect(
            sdi,
            "np_dd_border_d",
            dd_x,
            dd_y + dd_h as i32 - 1,
            dd_w,
            1,
            dark,
            151,
        );

        let mut item_y = dd_y + 4;
        for (i, entry) in menu
            .entries
            .iter()
            .enumerate()
            .take(NP_MAX_DROPDOWN_ENTRIES)
        {
            match entry {
                super::MenuEntry::Action {
                    label,
                    shortcut,
                    enabled,
                    ..
                } => {
                    let hot = self.menu.hovered_item == Some(i) && *enabled;
                    let hot_name = format!("np_dd_hot_{i}");
                    rect_visible(
                        sdi,
                        &hot_name,
                        dd_x + 2,
                        item_y,
                        dd_w - 4,
                        20,
                        item_hot_bg,
                        152,
                        hot,
                    );
                    let color = if !enabled {
                        disabled
                    } else if hot {
                        item_hot_text
                    } else {
                        item_text_color
                    };
                    let text_name = format!("np_dd_text_{i}");
                    text(
                        sdi,
                        &text_name,
                        dd_x + 22,
                        item_y + 4,
                        11,
                        color,
                        label,
                        153,
                    );
                    if let Some(sc) = shortcut {
                        let sc_w = sc.chars().count() as i32 * 7;
                        let sc_name = format!("np_dd_shortcut_{i}");
                        text(
                            sdi,
                            &sc_name,
                            dd_x + dd_w as i32 - sc_w - 22,
                            item_y + 4,
                            11,
                            color,
                            sc,
                            153,
                        );
                    }
                    item_y += 20;
                },
                super::MenuEntry::Separator => {
                    let sep_name = format!("np_dd_sep_{i}");
                    rect(
                        sdi,
                        &sep_name,
                        dd_x + 4,
                        item_y + 3,
                        dd_w - 8,
                        1,
                        sep_color,
                        152,
                    );
                    item_y += 6;
                },
            }
        }
    }

    /// Hide every SDI object the notepad renderer creates so a
    /// subsequent app launch doesn't leak stale chrome onto the screen.
    pub(crate) fn hide_notepad_sdi(&self, sdi: &mut SdiRegistry) {
        hide_notepad_sdi_objects(sdi);
    }
}

/// Hide every SDI object the notepad renderer creates. Exposed as a
/// standalone function so host code (e.g. the AppRunner cleanup path)
/// can call it without holding a `TextEditorApp` reference.
pub fn hide_notepad_sdi_objects(sdi: &mut SdiRegistry) {
    let fixed = [
        "np_menu_bg",
        "np_menu_border",
        "np_area_bg",
        "np_sel_bg",
        "np_caret",
        "np_status_bg",
        "np_status_border",
        "np_status_left",
        "np_status_right",
        "np_dd_bg",
        "np_dd_border_l",
        "np_dd_border_d",
    ];
    for name in fixed {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
    for i in 0..menu_label_count() {
        for name in [format!("np_menu_{i}"), format!("np_menu_hot_{i}")] {
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }
    for i in 0..NP_MAX_DROPDOWN_ENTRIES {
        for kind in ["hot", "text", "shortcut", "sep"] {
            let name = format!("np_dd_{kind}_{i}");
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }
    for i in 0..NP_MAX_VISIBLE_LINES {
        let name = format!("np_line_{i}");
        if !sdi.contains(&name) {
            break;
        }
        for name in [name, format!("np_selrange_{i}")] {
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }
}

/// Maximum drop-down items the SDI renderer preallocates slots for.
const NP_MAX_DROPDOWN_ENTRIES: usize = 16;

const fn menu_label_count() -> usize {
    4
}

#[allow(clippy::too_many_arguments)]
fn rect(sdi: &mut SdiRegistry, name: &str, x: i32, y: i32, w: u32, h: u32, color: Color, z: i32) {
    rect_visible(sdi, name, x, y, w, h, color, z, true);
}

#[allow(clippy::too_many_arguments)]
fn rect_visible(
    sdi: &mut SdiRegistry,
    name: &str,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: Color,
    z: i32,
    visible: bool,
) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = x;
        obj.y = y;
        obj.w = w;
        obj.h = h;
        obj.color = color;
        obj.z = z;
        obj.visible = visible;
        obj.text = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn text(
    sdi: &mut SdiRegistry,
    name: &str,
    x: i32,
    y: i32,
    size: u16,
    color: Color,
    content: &str,
    z: i32,
) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = x;
        obj.y = y;
        obj.w = 0;
        obj.h = 0;
        obj.font_size = size;
        obj.text_color = color;
        obj.text = Some(content.to_string());
        obj.z = z;
        obj.visible = true;
    }
}

fn hide(sdi: &mut SdiRegistry, name: &str) {
    if let Ok(obj) = sdi.get_mut(name) {
        obj.visible = false;
    }
}

#[cfg(test)]
mod tests {
    use oasis_app_core::App;
    use oasis_sdi::SdiRegistry;
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

    #[test]
    fn sdi_body_background_follows_dark_theme() {
        let at = dark_theme();
        let mut app = TextEditorApp::open_file("/a.txt", "x");
        let mut sdi = SdiRegistry::new();
        app.update_sdi(&mut sdi, &at);
        let area = sdi.get("np_area_bg").expect("area");
        assert_eq!(area.color, at.app.bg);
        let line = sdi.get("np_line_0").expect("line");
        assert_eq!(line.text_color, at.app.text);
    }
}
