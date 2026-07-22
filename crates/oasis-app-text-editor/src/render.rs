use crate::highlight::{SyntaxTheme, highlight_line};
use crate::{EditorMode, FileType, TextEditorApp};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};

/// How many editor lines the SDI notepad renderer can show at once.
/// Used for both sizing (we create objects up front) and teardown.
const NP_MAX_VISIBLE_LINES: usize = 64;

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
        let body_bg = Color::rgb(255, 255, 255);
        let body_fg = Color::rgb(16, 16, 16);
        let chrome_bg = Color::rgb(240, 240, 240);
        let chrome_border = Color::rgb(180, 180, 180);
        let status_bg = Color::rgb(224, 224, 230);
        let status_fg = Color::rgb(32, 32, 32);
        let selection_bg = Color::rgb(173, 214, 255);

        // Menu bar at the very top: real widget with live drop-downs.
        let menu_h: u32 = 18;
        let menu_y = cy;
        let menu_style = oasis_ui::menu_bar::MenuStyle::from_theme(&at.ui_theme);
        self.menu
            .draw_bar(backend, cx, menu_y, cw, menu_h, &menu_style)?;
        // Suppress unused warnings — still using these palette
        // entries for text-area chrome below.
        let _ = chrome_bg;
        let _ = chrome_border;

        // Text area.
        let area_y = menu_y + menu_h as i32;
        let status_h: u32 = 18;
        let area_h = (cy + ch as i32 - area_y - status_h as i32).max(0) as u32;
        backend.fill_rect(cx, area_y, cw, area_h, body_bg)?;

        let font_size: u16 = 12;
        let line_h = at.terminal_line_height.max(14) as i32;
        let pad_left = 8i32;
        let pad_top = 6i32;

        // If the file type has syntax highlighting, walk block-comment
        // state from the top of the buffer so multi-line comments
        // render correctly after scrolling.
        let theme = SyntaxTheme::default();
        let mut in_block_comment = false;
        for i in 0..self.content.scroll.min(self.buffer.line_count()) {
            if self.file_type != FileType::Plain
                && let Some(line_text) = self.buffer.get_line(i)
            {
                let (_, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;
            }
        }

        let max_lines = ((area_h as i32 - pad_top) / line_h).max(0) as usize;
        let visible = self
            .buffer
            .line_count()
            .saturating_sub(self.content.scroll)
            .min(max_lines);

        for i in 0..visible {
            let line_idx = self.content.scroll + i;
            let y = area_y + pad_top + i as i32 * line_h;

            // Selection/current-line highlight on the active line.
            if line_idx == self.cursor_line {
                backend.fill_rect(cx, y - 1, cw, line_h as u32, selection_bg)?;
            }

            let Some(line_text) = self.buffer.get_line(line_idx) else {
                continue;
            };

            if self.file_type == FileType::Plain {
                backend.draw_text(line_text, cx + pad_left, y, font_size, body_fg)?;
            } else {
                let (spans, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;
                let mut text_x = cx + pad_left;
                for span in &spans {
                    let segment = &line_text[span.start..span.end];
                    if segment.is_empty() {
                        continue;
                    }
                    let color = theme.color_for(span.kind);
                    backend.draw_text(segment, text_x, y, font_size, color)?;
                    text_x += backend.measure_text(segment, font_size) as i32;
                }
            }

            // Cursor caret on the active line — a solid 2px bar in
            // Insert mode (blue, always visible), a dimmer grey bar
            // in Normal mode. Kept opaque rather than blinking to
            // avoid burning frame time on a redraw just for the
            // caret.
            if line_idx == self.cursor_line {
                let prefix: String = line_text.chars().take(self.cursor_col).collect();
                let caret_x = cx + pad_left + backend.measure_text(&prefix, font_size) as i32;
                let (caret_color, caret_w) = if self.mode == EditorMode::Insert {
                    (Color::rgb(0, 100, 220), 2u32)
                } else {
                    (Color::rgb(60, 60, 60), 2u32)
                };
                backend.fill_rect(caret_x, y - 1, caret_w, line_h as u32, caret_color)?;
            }
        }

        // Status bar.
        let status_y = cy + ch as i32 - status_h as i32;
        backend.fill_rect(cx, status_y, cw, status_h, status_bg)?;
        backend.fill_rect(cx, status_y, cw, 1, chrome_border)?;

        let mode_str = match self.mode {
            EditorMode::Normal => "Normal",
            EditorMode::Insert => "Insert",
            EditorMode::Find => "Find",
            EditorMode::Saving => "Save?",
        };
        let position = format!("Ln {}, Col {}", self.cursor_line + 1, self.cursor_col + 1);
        let lines_total = format!("{} lines", self.buffer.line_count());
        // The file name lives here now that there's no inner title bar.
        let file_label = match &self.file_path {
            Some(fp) => fp.rsplit('/').next().unwrap_or(fp),
            None => "(untitled)",
        };
        let mod_marker = if self.modified { "*" } else { "" };
        let status_left = if let Some(ref msg) = self.status_message {
            format!("{mode_str}  |  {msg}")
        } else if self.mode == EditorMode::Find {
            format!("{mode_str}  |  Find: {}_", self.find_query)
        } else {
            format!("{mode_str}  |  {file_label}{mod_marker}  |  {lines_total}")
        };
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

    /// Draw the editor content with syntax highlighting into a
    /// windowed region using per-span colored `draw_text` calls.
    #[allow(dead_code)]
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

        let body_bg = Color::rgb(255, 255, 255);
        let body_fg = Color::rgb(16, 16, 16);
        let chrome_bg = Color::rgb(240, 240, 240);
        let chrome_border = Color::rgb(180, 180, 180);
        let status_bg = Color::rgb(224, 224, 230);
        let status_fg = Color::rgb(32, 32, 32);
        let selection_bg = Color::rgb(173, 214, 255);

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
        let label_hot_bg = Color::rgb(49, 106, 197);
        let label_hot_text = Color::rgb(255, 255, 255);
        let label_text = Color::rgb(40, 40, 40);
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
        self.render_dropdown_sdi(sdi, menu_y, menu_h);

        // Text area background.
        rect(sdi, "np_area_bg", 0, area_y, sw, area_h, body_bg, 103);

        // Current-line selection highlight.
        let line_h = at.terminal_line_height.max(14);
        let pad_top = 6i32;
        let pad_left = 8i32;
        let max_lines = ((area_h as i32 - pad_top) / line_h as i32).max(0) as usize;
        let max_lines = max_lines.min(NP_MAX_VISIBLE_LINES);
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
            selection_bg,
            104,
            sel_visible,
        );

        // Syntax state across the scrolled-over lines.
        let theme = SyntaxTheme::default();
        let mut in_block_comment = false;
        for i in 0..self.content.scroll.min(self.buffer.line_count()) {
            if self.file_type != FileType::Plain
                && let Some(line_text) = self.buffer.get_line(i)
            {
                let (_, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;
            }
        }

        // Visible text lines.
        for i in 0..NP_MAX_VISIBLE_LINES {
            let name = format!("np_line_{i}");
            if i >= visible {
                hide(sdi, &name);
                continue;
            }
            let line_idx = self.content.scroll + i;
            let y = area_y + pad_top + i as i32 * line_h as i32;
            let Some(line_text) = self.buffer.get_line(line_idx) else {
                hide(sdi, &name);
                continue;
            };

            // Single-color display for plain files or simple fallback.
            // Syntax highlighting in SDI mode would need one object per
            // span — skip it here; windowed mode still renders colors.
            let display = if line_text.is_empty() {
                " ".to_string()
            } else {
                line_text.to_string()
            };
            let color = if self.file_type == FileType::Plain {
                body_fg
            } else {
                // Rough: keyword-first color if the line starts with a
                // recognized token, else body foreground. Cheap and
                // visibly distinct from plain text.
                let (spans, _still) = highlight_line(line_text, self.file_type, in_block_comment);
                spans
                    .first()
                    .map(|s| theme.color_for(s.kind))
                    .unwrap_or(body_fg)
            };
            text(sdi, &name, pad_left, y, 12, color, &display, 105);

            if self.file_type != FileType::Plain {
                let (_, still_in) = highlight_line(line_text, self.file_type, in_block_comment);
                in_block_comment = still_in;
            }
        }

        // Caret on active line (thin vertical bar).
        let caret_color = if self.mode == EditorMode::Insert {
            Color::rgb(0, 100, 200)
        } else {
            Color::rgb(60, 60, 60)
        };
        let caret_visible = sel_visible;
        let caret_line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        // Approximate caret x: monospaced-ish width per char; the
        // windowed path uses `measure_text` but we don't have a backend
        // here. 7px/char at size 12 is close enough for the SDI path
        // and matches the bitmap font used by the backends at this size.
        let prefix_chars = caret_line.chars().take(self.cursor_col).count() as i32;
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
            chrome_border,
            104,
        );

        let mode_str = match self.mode {
            EditorMode::Normal => "Normal",
            EditorMode::Insert => "Insert",
            EditorMode::Find => "Find",
            EditorMode::Saving => "Save?",
        };
        let status_left = if let Some(ref msg) = self.status_message {
            format!("{mode_str}  |  {msg}")
        } else if self.mode == EditorMode::Find {
            format!("{mode_str}  |  Find: {}_", self.find_query)
        } else {
            format!("{mode_str}  |  {} lines", self.buffer.line_count())
        };
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

        let position = format!("Ln {}, Col {}", self.cursor_line + 1, self.cursor_col + 1);
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
    fn render_dropdown_sdi(&self, sdi: &mut SdiRegistry, menu_y: i32, menu_h: u32) {
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

        let bg = Color::rgb(236, 236, 236);
        let light = Color::rgb(255, 255, 255);
        let dark = Color::rgb(105, 105, 105);
        let item_text_color = Color::rgb(20, 20, 20);
        let item_hot_bg = Color::rgb(49, 106, 197);
        let item_hot_text = Color::rgb(255, 255, 255);
        let disabled = Color::rgb(150, 150, 150);
        let sep_color = Color::rgb(170, 170, 170);

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
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.visible = false;
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
