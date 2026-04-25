//! File Manager application with dual-panel browsing.
//!
//! Implements the `App` trait for a dual-panel file manager with
//! file viewing capabilities (text, audio metadata, image metadata).

use std::any::Any;
use std::cell::Cell;

use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::input::Button;
use oasis_ui::flex;
use oasis_vfs::Vfs;

pub use oasis_app_core::file_viewer::{
    app_for_file, join_path, list_directory, parent_dir, view_audio_file, view_generic_file,
    view_image_file,
};

// ---------------------------------------------------------------
// FilePanel: per-panel state for dual-panel browsing
// ---------------------------------------------------------------

/// Per-panel state for dual-panel file browsing.
#[derive(Debug, Clone)]
pub struct FilePanel {
    /// Current directory being browsed.
    pub browse_dir: String,
    /// Display lines for the current directory.
    pub lines: Vec<String>,
    /// Scroll offset.
    pub scroll: usize,
    /// Cursor position (relative to visible area).
    pub cursor: usize,
}

impl FilePanel {
    /// Create a new panel rooted at the given directory.
    pub fn new(dir: &str, vfs: &dyn Vfs) -> Self {
        let lines = list_directory(vfs, dir);
        Self {
            browse_dir: dir.to_string(),
            lines,
            scroll: 0,
            cursor: 0,
        }
    }

    fn visible_count(&self, max_visible: usize) -> usize {
        let remaining = self.lines.len().saturating_sub(self.scroll);
        remaining.min(max_visible)
    }

    fn navigate_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        } else if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    fn navigate_down(&mut self, max_visible: usize) {
        let visible = self.visible_count(max_visible);
        if self.cursor + 1 < visible {
            self.cursor += 1;
        } else if self.scroll + max_visible < self.lines.len() {
            self.scroll += 1;
        }
    }

    fn enter_selected(&mut self, vfs: &dyn Vfs) {
        let abs_idx = self.scroll + self.cursor;
        let Some(line) = self.lines.get(abs_idx) else {
            return;
        };
        let line = line.trim().to_string();

        if line == ".." {
            let parent = parent_dir(&self.browse_dir);
            self.browse_dir = parent.clone();
            self.lines = list_directory(vfs, &parent);
            self.scroll = 0;
            self.cursor = 0;
        } else if line.ends_with('/') {
            let name = &line[..line.len() - 1];
            let new_dir = join_path(&self.browse_dir, name);
            self.browse_dir = new_dir.clone();
            self.lines = list_directory(vfs, &new_dir);
            self.scroll = 0;
            self.cursor = 0;
        }
    }

    fn enter_selected_parent(&mut self, vfs: &dyn Vfs) {
        let parent = parent_dir(&self.browse_dir);
        self.browse_dir = parent.clone();
        self.lines = list_directory(vfs, &parent);
        self.scroll = 0;
        self.cursor = 0;
    }

    /// Return the full path of the currently selected entry (if any).
    fn selected_path(&self) -> Option<String> {
        let abs_idx = self.scroll + self.cursor;
        let line = self.lines.get(abs_idx)?;
        let name = line.trim();
        if name == ".." {
            return None;
        }
        // Strip trailing '/' for directories, and strip size suffix for files.
        let name = name
            .strip_suffix('/')
            .unwrap_or_else(|| name.split("  (").next().unwrap_or(name));
        Some(join_path(&self.browse_dir, name))
    }

    /// Refresh the panel listing from VFS.
    pub fn refresh(&mut self, vfs: &dyn Vfs) {
        self.lines = list_directory(vfs, &self.browse_dir);
        // Clamp cursor to new list size.
        let max = self.lines.len().saturating_sub(1);
        self.cursor = self.cursor.min(max);
    }
}

// ---------------------------------------------------------------
// FileManagerApp
// ---------------------------------------------------------------

/// A pending VFS operation for the file manager.
#[derive(Debug, Clone)]
pub enum FileOp {
    /// Delete the file or directory at this path.
    Delete(String),
    /// Create a directory at this path.
    Mkdir(String),
}

/// Which presentation the file manager is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Twin text panels (TUI feel) -- the original layout.
    Dual,
    /// Single-pane Win2K Explorer-style icon grid with folder tree.
    Explorer,
}

/// File Manager application with dual-panel browsing.
#[derive(Debug)]
pub struct FileManagerApp {
    /// Shared content state (title, lines, scroll, cursor, etc.).
    pub content: ContentState,
    /// Dual panels.
    pub panels: [FilePanel; 2],
    /// Which panel is active (0 = left, 1 = right).
    pub active_panel: usize,
    /// Pending file operation to be applied by the runner.
    pub pending_op: Option<FileOp>,
    /// Active view mode (toggled via Button::Select).
    pub view_mode: ViewMode,
    /// Cached column count for the Explorer icon grid (written by the
    /// renderer each frame so the next input tick can navigate by tile
    /// coordinates). `Cell` so the `&self` windowed renderer can refresh it.
    explorer_cols: Cell<usize>,
    /// Cached visible row count for the Explorer icon grid.
    explorer_visible_rows: Cell<usize>,
}

impl FileManagerApp {
    /// Create a new File Manager app.
    pub fn new(path: &str, vfs: &dyn Vfs) -> Self {
        let mut content = ContentState::new("File Manager", path);
        content.browse_dir = Some("/".to_string());
        content.lines = list_directory(vfs, "/");
        Self {
            content,
            panels: [FilePanel::new("/", vfs), FilePanel::new("/", vfs)],
            active_panel: 0,
            pending_op: None,
            view_mode: ViewMode::Dual,
            explorer_cols: Cell::new(1),
            explorer_visible_rows: Cell::new(1),
        }
    }

    /// Toggle between dual-panel and Explorer view modes.
    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Dual => ViewMode::Explorer,
            ViewMode::Explorer => ViewMode::Dual,
        };
    }

    /// Currently active panel (the one driving Explorer view too).
    fn active(&self) -> &FilePanel {
        &self.panels[self.active_panel]
    }

    /// Mutable accessor for the currently active panel.
    fn active_mut(&mut self) -> &mut FilePanel {
        &mut self.panels[self.active_panel]
    }

    /// Handle input in dual-panel mode (no file viewer open).
    fn handle_dual_panel_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Left | Button::Right => {
                self.active_panel = 1 - self.active_panel;
                self.content.browse_dir = Some(self.panels[self.active_panel].browse_dir.clone());
                AppAction::None
            },
            Button::Up => {
                self.panels[self.active_panel].navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.panels[self.active_panel].navigate_down(self.content.cached_max_visible);
                AppAction::None
            },
            Button::Confirm => {
                let p = &mut self.panels[self.active_panel];
                let abs_idx = p.scroll + p.cursor;
                let is_file = p.lines.get(abs_idx).is_some_and(|line| {
                    let l = line.trim();
                    l != ".." && !l.ends_with('/')
                });
                if is_file {
                    let line = p.lines[abs_idx].trim().to_string();
                    let file_name = line.split("  (").next().unwrap_or(&line);
                    let dir = &p.browse_dir;
                    let file_path = join_path(dir, file_name);
                    if let Some(app_title) = app_for_file(&file_path) {
                        return AppAction::LaunchAppWithFile {
                            app_title: app_title.to_string(),
                            file_path,
                        };
                    }
                    self.open_file(vfs, &file_path);
                } else {
                    p.enter_selected(vfs);
                    self.content.browse_dir = Some(p.browse_dir.clone());
                }
                AppAction::None
            },
            Button::Cancel => {
                let p = &self.panels[self.active_panel];
                if p.browse_dir == "/" {
                    AppAction::Exit
                } else {
                    self.panels[self.active_panel].enter_selected_parent(vfs);
                    self.content.browse_dir =
                        Some(self.panels[self.active_panel].browse_dir.clone());
                    AppAction::None
                }
            },
            Button::Triangle => {
                // Delete selected file/directory.
                if let Some(path) = self.panels[self.active_panel].selected_path() {
                    self.pending_op = Some(FileOp::Delete(path));
                }
                AppAction::None
            },
            Button::Square => {
                // Create new directory in active panel.
                let dir = &self.panels[self.active_panel].browse_dir;
                let new_dir = join_path(dir, "new_folder");
                self.pending_op = Some(FileOp::Mkdir(new_dir));
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Take and clear the pending file operation.
    pub fn take_file_op(&mut self) -> Option<FileOp> {
        self.pending_op.take()
    }

    /// Move the active panel's cursor by `delta` tiles in the Explorer grid,
    /// keeping the scrolled-into-view invariant intact.
    fn explorer_move_cursor(&mut self, delta: isize) {
        let cols = self.explorer_cols.get().max(1);
        let rows = self.explorer_visible_rows.get().max(1);
        let p = self.active_mut();
        let total = p.lines.len();
        if total == 0 {
            return;
        }
        let abs = (p.scroll + p.cursor) as isize + delta;
        if abs < 0 || abs >= total as isize {
            return;
        }
        let abs = abs as usize;

        let row = abs / cols;
        let first_visible_row = p.scroll / cols;
        if row < first_visible_row {
            p.scroll = row * cols;
        } else if row >= first_visible_row + rows {
            p.scroll = (row + 1 - rows) * cols;
        }
        p.cursor = abs - p.scroll;
    }

    /// Handle input in Explorer (single-pane icon grid) mode.
    fn handle_explorer_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        let cols = self.explorer_cols.get().max(1) as isize;
        match button {
            Button::Left => {
                self.explorer_move_cursor(-1);
                AppAction::None
            },
            Button::Right => {
                self.explorer_move_cursor(1);
                AppAction::None
            },
            Button::Up => {
                self.explorer_move_cursor(-cols);
                AppAction::None
            },
            Button::Down => {
                self.explorer_move_cursor(cols);
                AppAction::None
            },
            Button::Confirm => {
                let p = self.active_mut();
                let abs_idx = p.scroll + p.cursor;
                let is_file = p.lines.get(abs_idx).is_some_and(|line| {
                    let l = line.trim();
                    l != ".." && !l.ends_with('/')
                });
                if is_file {
                    let line = p.lines[abs_idx].trim().to_string();
                    let file_name = line.split("  (").next().unwrap_or(&line);
                    let dir = p.browse_dir.clone();
                    let file_path = join_path(&dir, file_name);
                    if let Some(app_title) = app_for_file(&file_path) {
                        return AppAction::LaunchAppWithFile {
                            app_title: app_title.to_string(),
                            file_path,
                        };
                    }
                    self.open_file(vfs, &file_path);
                } else {
                    p.enter_selected(vfs);
                    self.content.browse_dir = Some(p.browse_dir.clone());
                }
                AppAction::None
            },
            Button::Cancel => {
                let p = self.active();
                if p.browse_dir == "/" {
                    AppAction::Exit
                } else {
                    self.active_mut().enter_selected_parent(vfs);
                    self.content.browse_dir = Some(self.active().browse_dir.clone());
                    AppAction::None
                }
            },
            Button::Triangle => {
                if let Some(path) = self.active().selected_path() {
                    self.pending_op = Some(FileOp::Delete(path));
                }
                AppAction::None
            },
            Button::Square => {
                let dir = self.active().browse_dir.clone();
                let new_dir = join_path(&dir, "new_folder");
                self.pending_op = Some(FileOp::Mkdir(new_dir));
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle input when viewing a file.
    fn handle_file_viewer_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => {
                self.content.viewing_file = None;
                self.content.scroll = 0;
                self.content.cursor = 0;
                let p = &self.panels[self.active_panel];
                self.content.browse_dir = Some(p.browse_dir.clone());
                self.content.lines = p.lines.clone();
                AppAction::None
            },
            Button::Up => {
                self.content.navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.content.navigate_down();
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Open a file in the viewer.
    pub fn open_file(&mut self, vfs: &dyn Vfs, path: &str) {
        if !vfs.exists(path) {
            return;
        }
        self.content.viewing_file = Some(path.to_string());
        self.content.scroll = 0;
        self.content.cursor = 0;

        let data = match vfs.read(path) {
            Ok(d) => d,
            Err(e) => {
                self.content.lines = vec![
                    format!("Error reading file: {e}"),
                    "Cancel=back".to_string(),
                ];
                return;
            },
        };

        self.content.lines = view_generic_file(path, &data);
    }

    /// Draw dual-panel layout to backend (windowed mode).
    #[allow(clippy::too_many_arguments)]
    fn draw_windowed_dual(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let half_w = (cw / 2).saturating_sub(1);
        let divider_x = cx + half_w as i32;

        // Title bar.
        let title = format!(
            "File Manager  [L: {}]  [R: {}]",
            self.panels[0].browse_dir, self.panels[1].browse_dir,
        );
        backend.draw_text(&title, cx + 4, cy + 2, 12, at.app.title_bar_text)?;
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            at.app.divider,
        )?;

        // Vertical divider.
        let title_h = at.app.title_bar_height as i32;
        let content_y = cy + title_h;
        let content_h = ch.saturating_sub(title_h as u32 + 14);
        backend.fill_rect(divider_x, content_y, 1, content_h, at.app.divider)?;

        // Draw each panel.
        let line_h = at.terminal_line_height.max(12) as i32;
        let max_lines = ((content_h as i32) / line_h).max(0) as usize;
        for (pi, panel) in self.panels.iter().enumerate() {
            let px = if pi == 0 { cx } else { divider_x + 1 };
            let pw = if pi == 0 { half_w } else { cw - half_w - 1 };
            let is_active = pi == self.active_panel;

            if is_active {
                backend.fill_rect(px, content_y, pw, 1, at.app.selected_text)?;
            }

            let visible = panel
                .lines
                .len()
                .saturating_sub(panel.scroll)
                .min(max_lines);
            for i in 0..visible {
                let line_idx = panel.scroll + i;
                let line = &panel.lines[line_idx];
                let prefix = if is_active && i == panel.cursor {
                    "> "
                } else {
                    "  "
                };
                let max_chars = (pw as usize / 8).saturating_sub(2);
                let display = if line.len() > max_chars {
                    &line[..line.floor_char_boundary(max_chars)]
                } else {
                    line.as_str()
                };
                let text = format!("{prefix}{display}");
                let text_color = if is_active && i == panel.cursor {
                    at.app.selected_text
                } else {
                    at.app.text
                };
                let y = content_y + 2 + i as i32 * line_h;
                backend.draw_text(&text, px + 2, y, 12, text_color)?;
            }
        }

        let scroll_y = cy + ch as i32 - 14;
        backend.draw_text(
            "L/R=panel  \u{25b3}=delete  \u{25a1}=mkdir  Cancel=back",
            cx + 4,
            scroll_y,
            10,
            at.app.dim_text,
        )?;

        Ok(())
    }

    /// Render dual-panel to SDI objects.
    fn update_sdi_dual(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Title with both panel paths.
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            obj.text = Some(format!(
                "File Manager  [L: {}]  [R: {}]",
                self.panels[0].browse_dir, self.panels[1].browse_dir,
            ));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = at.app.title_bar_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        // Responsive dual-panel geometry.
        let title_h = at.app.title_bar_height;
        let content_y = (title_h + 4) as i32;
        let half_w = at.screen_w / 2;
        let panel_pad = 8u32;
        let divider_x = half_w as i32;
        let left_x = 8i32;
        let left_w = half_w - panel_pad - left_x as u32;
        let right_x = divider_x + panel_pad as i32;
        let right_w = at.screen_w - right_x as u32 - panel_pad;
        let divider_h = at.screen_h - title_h - at.statusbar_height - at.bottombar_height;
        let usable_h = at.screen_h - title_h - at.statusbar_height - at.bottombar_height - 14;
        let panel_visible = (usable_h / at.terminal_line_height.max(1)).max(1) as usize;

        // Vertical divider.
        if !sdi.contains("app_divider") {
            sdi.create("app_divider");
        }
        if let Ok(obj) = sdi.get_mut("app_divider") {
            obj.x = divider_x;
            obj.y = content_y - 2;
            obj.w = 1;
            obj.h = divider_h;
            obj.color = at.app.divider;
            obj.visible = true;
            obj.z = 102;
        }

        // Left panel lines.
        let lp_rects = flex::vertical_list(
            left_x,
            content_y,
            left_w,
            at.terminal_line_height,
            0,
            panel_visible,
        );
        for (i, rect) in lp_rects.iter().enumerate() {
            let name = format!("app_lp_line_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let p = &self.panels[0];
                let line_idx = p.scroll + i;
                let is_active = self.active_panel == 0;
                if line_idx < p.lines.len() {
                    obj.text = Some(p.lines[line_idx].clone());
                    obj.visible = true;
                } else {
                    obj.text = None;
                    obj.visible = false;
                }
                obj.x = rect.x + 6;
                obj.y = rect.y;
                obj.font_size = at.font_body;
                obj.text_color = if is_active && i == p.cursor {
                    at.app.selected_text
                } else {
                    at.app.text
                };
                obj.w = 0;
                obj.h = 0;
                obj.z = 102;
            }
        }

        // Right panel lines.
        let rp_rects = flex::vertical_list(
            right_x,
            content_y,
            right_w,
            at.terminal_line_height,
            0,
            panel_visible,
        );
        for (i, rect) in rp_rects.iter().enumerate() {
            let name = format!("app_rp_line_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let p = &self.panels[1];
                let line_idx = p.scroll + i;
                let is_active = self.active_panel == 1;
                if line_idx < p.lines.len() {
                    obj.text = Some(p.lines[line_idx].clone());
                    obj.visible = true;
                } else {
                    obj.text = None;
                    obj.visible = false;
                }
                obj.x = rect.x + 6;
                obj.y = rect.y;
                obj.font_size = at.font_body;
                obj.text_color = if is_active && i == p.cursor {
                    at.app.selected_text
                } else {
                    at.app.text
                };
                obj.w = 0;
                obj.h = 0;
                obj.z = 102;
            }
        }

        // Scroll indicator.
        if !sdi.contains("app_scroll") {
            sdi.create("app_scroll");
        }
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            obj.text = Some("L/R=panel  \u{25b3}=delete  \u{25a1}=mkdir  Cancel=back".to_string());
            obj.x = 8;
            obj.y = at.screen_h as i32 - 14;
            obj.font_size = at.font_hint;
            obj.text_color = at.app.dim_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        // Hide single-panel lines.
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }

    /// Render Explorer view to SDI. Updates `explorer_cols`/`rows` cache.
    fn update_sdi_explorer(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let panel = self.panels[self.active_panel].clone();
        // Title text reused from chrome.
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            obj.text = Some(format!("File Manager  -  {}", panel.browse_dir));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = at.app.title_bar_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        let title_h = at.app.title_bar_height as i32;
        let body_top = title_h;
        let body_h = at
            .screen_h
            .saturating_sub(at.app.title_bar_height + at.statusbar_height + at.bottombar_height);
        let g = compute_explorer_geom(0, body_top, at.screen_w, body_h, at);
        self.explorer_cols.set(g.cols);
        self.explorer_visible_rows.set(g.rows);

        let dark = at.app.divider;

        // Menu bar.
        ensure_rect(
            sdi,
            "app_xp_menu_bg",
            Box2d {
                x: g.menu_x,
                y: g.menu_y,
                w: g.menu_w,
                h: g.menu_h,
            },
            at.app.title_bar_bg,
            104,
        );
        ensure_text(
            sdi,
            "app_xp_menu_text",
            "File  Edit  View  Favorites  Tools  Help",
            g.menu_x + 4,
            g.menu_y + 2,
            TextStyle {
                font_size: at.font_hint,
                color: at.app.title_bar_text,
                z: 105,
            },
        );
        ensure_rect(
            sdi,
            "app_xp_menu_sep",
            Box2d {
                x: g.menu_x,
                y: g.menu_y + g.menu_h as i32 - 1,
                w: g.menu_w,
                h: 1,
            },
            dark,
            105,
        );

        // Address bar.
        ensure_rect(
            sdi,
            "app_xp_addr_bg",
            Box2d {
                x: g.menu_x,
                y: g.addr_y,
                w: g.menu_w,
                h: g.addr_h,
            },
            at.app.bg,
            104,
        );
        ensure_text(
            sdi,
            "app_xp_addr_label",
            "Address:",
            g.menu_x + 4,
            g.addr_y + 2,
            TextStyle {
                font_size: at.font_hint,
                color: at.app.dim_text,
                z: 105,
            },
        );
        let addr_field_x = g.menu_x + 56;
        let addr_field_w = g.menu_w.saturating_sub(60);
        let addr_field = Box2d {
            x: addr_field_x,
            y: g.addr_y + 1,
            w: addr_field_w,
            h: g.addr_h - 2,
        };
        ensure_rect(sdi, "app_xp_addr_field", addr_field, Color::WHITE, 105);
        outline_rect(sdi, "app_xp_addr_o", addr_field, dark, 106);
        ensure_text(
            sdi,
            "app_xp_addr_text",
            &panel.browse_dir,
            addr_field_x + 4,
            g.addr_y + 3,
            TextStyle {
                font_size: at.font_hint,
                color: Color::BLACK,
                z: 107,
            },
        );

        // Tree pane (sunken white pane).
        let tree_box = Box2d {
            x: g.tree_x,
            y: g.body_y,
            w: g.tree_w,
            h: g.body_h,
        };
        ensure_rect(sdi, "app_xp_tree_bg", tree_box, Color::WHITE, 104);
        outline_rect(sdi, "app_xp_tree_o", tree_box, dark, 105);

        // Tree contents.
        let tree_entries = build_tree_entries(&panel.browse_dir);
        let tree_line_h = (at.font_hint as i32 + 2).max(11);
        for i in 0..MAX_TREE_LINES {
            let name = format!("app_xp_tree_l_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                if let Some(entry) = tree_entries.get(i) {
                    let y = g.body_y + 4 + i as i32 * tree_line_h;
                    if y + tree_line_h > g.body_y + g.body_h as i32 - 4 {
                        obj.visible = false;
                        continue;
                    }
                    let indent = entry.depth as i32 * 8;
                    obj.text = Some(entry.label.clone());
                    obj.x = g.tree_x + 4 + indent;
                    obj.y = y + 1;
                    obj.font_size = at.font_hint;
                    obj.text_color = if entry.is_current {
                        at.app.selected_text
                    } else {
                        Color::BLACK
                    };
                    obj.w = 0;
                    obj.h = 0;
                    obj.visible = true;
                    obj.z = 106;
                } else {
                    obj.visible = false;
                }
            }
        }
        // Tree current-row highlight.
        if let Some(idx) = tree_entries.iter().position(|e| e.is_current) {
            let y = g.body_y + 4 + idx as i32 * tree_line_h;
            if y + tree_line_h <= g.body_y + g.body_h as i32 - 4 {
                ensure_rect(
                    sdi,
                    "app_xp_tree_sel",
                    Box2d {
                        x: g.tree_x + 2,
                        y,
                        w: g.tree_w.saturating_sub(4),
                        h: tree_line_h as u32,
                    },
                    at.app.selected_bg,
                    105,
                );
            } else if let Ok(obj) = sdi.get_mut("app_xp_tree_sel") {
                obj.visible = false;
            }
        } else if let Ok(obj) = sdi.get_mut("app_xp_tree_sel") {
            obj.visible = false;
        }

        // Grid pane (sunken white).
        let grid_box = Box2d {
            x: g.grid_x,
            y: g.body_y,
            w: g.grid_w,
            h: g.body_h,
        };
        ensure_rect(sdi, "app_xp_grid_bg", grid_box, Color::WHITE, 104);
        outline_rect(sdi, "app_xp_grid_o", grid_box, dark, 105);

        // Tiles.
        let count_visible = (g.cols * g.rows).min(MAX_TILES);
        for i in 0..MAX_TILES {
            let abs_idx = panel.scroll + i;
            let line = if i < count_visible {
                panel.lines.get(abs_idx)
            } else {
                None
            };
            let row = i.checked_div(g.cols).unwrap_or(0);
            let col = i.checked_rem(g.cols).unwrap_or(0);
            let tx = g.grid_x + 4 + col as i32 * g.tile_w as i32;
            let ty = g.body_y + 4 + row as i32 * g.tile_h as i32;
            let tile_visible = line.is_some()
                && tx + g.tile_w as i32 <= g.grid_x + g.grid_w as i32 - 2
                && ty + g.tile_h as i32 <= g.body_y + g.body_h as i32 - 2;
            let is_selected = abs_idx == panel.scroll + panel.cursor;

            // Selection background.
            let sel_name = format!("app_xp_t_sel_{i}");
            if !sdi.contains(&sel_name) {
                sdi.create(&sel_name);
            }
            if let Ok(obj) = sdi.get_mut(&sel_name) {
                obj.x = tx;
                obj.y = ty;
                obj.w = g.tile_w;
                obj.h = g.tile_h;
                obj.color = at.app.selected_bg;
                obj.visible = tile_visible && is_selected;
                obj.z = 106;
            }

            let (name, kind) = if let Some(l) = line {
                parse_entry(l)
            } else {
                (String::new(), EntryKind::File)
            };

            // Icon body.
            let icon_x = tx + (g.tile_w as i32 - g.icon_w as i32) / 2;
            let icon_y = ty + 4;
            let body_color = match kind {
                EntryKind::Dir | EntryKind::ParentDir => Color::rgb(255, 207, 87),
                EntryKind::File => Color::WHITE,
            };
            let body_name = format!("app_xp_t_body_{i}");
            if !sdi.contains(&body_name) {
                sdi.create(&body_name);
            }
            if let Ok(obj) = sdi.get_mut(&body_name) {
                obj.x = icon_x;
                obj.y = icon_y;
                obj.w = g.icon_w;
                obj.h = g.icon_h;
                obj.color = body_color;
                obj.stroke_width = Some(1);
                obj.stroke_color = Some(dark);
                obj.visible = tile_visible;
                obj.z = 107;
            }

            // Icon accent (folder tab or page fold).
            let accent_name = format!("app_xp_t_accent_{i}");
            if !sdi.contains(&accent_name) {
                sdi.create(&accent_name);
            }
            if let Ok(obj) = sdi.get_mut(&accent_name) {
                match kind {
                    EntryKind::Dir | EntryKind::ParentDir => {
                        obj.x = icon_x;
                        obj.y = icon_y;
                        obj.w = g.icon_w / 2;
                        obj.h = 4;
                        obj.color = Color::rgb(220, 170, 50);
                    },
                    EntryKind::File => {
                        let fold = 6u32;
                        obj.x = icon_x + g.icon_w as i32 - fold as i32;
                        obj.y = icon_y;
                        obj.w = fold;
                        obj.h = fold;
                        obj.color = Color::rgb(220, 220, 220);
                    },
                }
                obj.stroke_width = None;
                obj.stroke_color = None;
                obj.visible = tile_visible;
                obj.z = 108;
            }

            // Label.
            let label_name = format!("app_xp_t_lbl_{i}");
            if !sdi.contains(&label_name) {
                sdi.create(&label_name);
            }
            if let Ok(obj) = sdi.get_mut(&label_name) {
                let max_chars = (g.tile_w as usize / 7).max(4);
                obj.text = Some(truncate_label(&name, max_chars));
                obj.x = tx + 2;
                obj.y = icon_y + g.icon_h as i32 + 2;
                obj.font_size = at.font_hint;
                obj.text_color = if is_selected {
                    at.app.selected_text
                } else {
                    Color::BLACK
                };
                obj.w = 0;
                obj.h = 0;
                obj.visible = tile_visible;
                obj.z = 108;
            }
        }

        // Status bar.
        let count = panel.lines.iter().filter(|l| l.trim() != "..").count();
        ensure_rect(
            sdi,
            "app_xp_status_bg",
            Box2d {
                x: g.menu_x,
                y: g.status_y,
                w: g.menu_w,
                h: g.status_h,
            },
            at.app.title_bar_bg,
            104,
        );
        let status_style = TextStyle {
            font_size: at.font_hint,
            color: at.app.title_bar_text,
            z: 105,
        };
        ensure_text(
            sdi,
            "app_xp_status_text",
            &format!("{count} object(s)"),
            g.menu_x + 4,
            g.status_y + 1,
            status_style,
        );
        ensure_text(
            sdi,
            "app_xp_status_hint",
            "Select=toggle  Cancel=back",
            g.menu_x + g.menu_w as i32 - 180,
            g.status_y + 1,
            status_style,
        );

        // Hide the scroll/divider used by other modes.
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut("app_divider") {
            obj.visible = false;
        }
    }

    /// Direct-draw Explorer view in windowed mode.
    fn draw_windowed_explorer(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let panel = self.active();
        let title = format!("File Manager  -  {}", panel.browse_dir);
        backend.draw_text(&title, cx + 4, cy + 2, 12, at.app.title_bar_text)?;
        let title_h = at.app.title_bar_height as i32;
        backend.fill_rect(cx, cy + title_h - 4, cw, 1, at.app.divider)?;

        let body_top = cy + title_h;
        let body_h = ch.saturating_sub(title_h as u32);
        let g = compute_explorer_geom(cx, body_top, cw, body_h, at);
        self.explorer_cols.set(g.cols);
        self.explorer_visible_rows.set(g.rows);

        let dark = at.app.divider;

        // Menu bar.
        backend.fill_rect(g.menu_x, g.menu_y, g.menu_w, g.menu_h, at.app.title_bar_bg)?;
        backend.draw_text(
            "File  Edit  View  Favorites  Tools  Help",
            g.menu_x + 4,
            g.menu_y + 2,
            at.font_hint,
            at.app.title_bar_text,
        )?;
        backend.fill_rect(g.menu_x, g.menu_y + g.menu_h as i32 - 1, g.menu_w, 1, dark)?;

        // Address bar.
        backend.fill_rect(g.menu_x, g.addr_y, g.menu_w, g.addr_h, at.app.bg)?;
        backend.draw_text(
            "Address:",
            g.menu_x + 4,
            g.addr_y + 2,
            at.font_hint,
            at.app.dim_text,
        )?;
        let addr_field_x = g.menu_x + 56;
        let addr_field_w = g.menu_w.saturating_sub(60);
        backend.fill_rect(
            addr_field_x,
            g.addr_y + 1,
            addr_field_w,
            g.addr_h - 2,
            Color::WHITE,
        )?;
        draw_outline(
            backend,
            addr_field_x,
            g.addr_y + 1,
            addr_field_w,
            g.addr_h - 2,
            dark,
        )?;
        backend.draw_text(
            &panel.browse_dir,
            addr_field_x + 4,
            g.addr_y + 3,
            at.font_hint,
            Color::BLACK,
        )?;

        // Tree pane.
        backend.fill_rect(g.tree_x, g.body_y, g.tree_w, g.body_h, Color::WHITE)?;
        draw_outline(backend, g.tree_x, g.body_y, g.tree_w, g.body_h, dark)?;
        let tree_entries = build_tree_entries(&panel.browse_dir);
        let tree_line_h = (at.font_hint as i32 + 2).max(11);
        for (i, entry) in tree_entries.iter().enumerate() {
            let y = g.body_y + 4 + i as i32 * tree_line_h;
            if y + tree_line_h > g.body_y + g.body_h as i32 - 4 {
                break;
            }
            if entry.is_current {
                backend.fill_rect(
                    g.tree_x + 2,
                    y,
                    g.tree_w.saturating_sub(4),
                    tree_line_h as u32,
                    at.app.selected_bg,
                )?;
            }
            let indent = entry.depth as i32 * 8;
            let color = if entry.is_current {
                at.app.selected_text
            } else {
                Color::BLACK
            };
            backend.draw_text(
                &entry.label,
                g.tree_x + 4 + indent,
                y + 1,
                at.font_hint,
                color,
            )?;
        }

        // Grid pane.
        backend.fill_rect(g.grid_x, g.body_y, g.grid_w, g.body_h, Color::WHITE)?;
        draw_outline(backend, g.grid_x, g.body_y, g.grid_w, g.body_h, dark)?;
        let count_visible = g.cols * g.rows;
        for i in 0..count_visible {
            let abs_idx = panel.scroll + i;
            let Some(line) = panel.lines.get(abs_idx) else {
                break;
            };
            let row = i / g.cols.max(1);
            let col = i % g.cols.max(1);
            let tx = g.grid_x + 4 + col as i32 * g.tile_w as i32;
            let ty = g.body_y + 4 + row as i32 * g.tile_h as i32;
            if tx + g.tile_w as i32 > g.grid_x + g.grid_w as i32 - 2
                || ty + g.tile_h as i32 > g.body_y + g.body_h as i32 - 2
            {
                break;
            }
            let is_selected = abs_idx == panel.scroll + panel.cursor;
            if is_selected {
                backend.fill_rect(tx, ty, g.tile_w, g.tile_h, at.app.selected_bg)?;
            }
            let (name, kind) = parse_entry(line);
            let icon_x = tx + (g.tile_w as i32 - g.icon_w as i32) / 2;
            let icon_y = ty + 4;
            draw_icon(backend, icon_x, icon_y, g.icon_w, g.icon_h, kind, dark)?;
            let max_chars = (g.tile_w as usize / 7).max(4);
            let label = truncate_label(&name, max_chars);
            let label_color = if is_selected {
                at.app.selected_text
            } else {
                Color::BLACK
            };
            backend.draw_text(
                &label,
                tx + 2,
                icon_y + g.icon_h as i32 + 2,
                at.font_hint,
                label_color,
            )?;
        }

        // Status.
        let count = panel.lines.iter().filter(|l| l.trim() != "..").count();
        backend.fill_rect(
            g.menu_x,
            g.status_y,
            g.menu_w,
            g.status_h,
            at.app.title_bar_bg,
        )?;
        backend.draw_text(
            &format!("{count} object(s)"),
            g.menu_x + 4,
            g.status_y + 1,
            at.font_hint,
            at.app.title_bar_text,
        )?;
        backend.draw_text(
            "Select=toggle  Cancel=back",
            g.menu_x + g.menu_w as i32 - 180,
            g.status_y + 1,
            at.font_hint,
            at.app.title_bar_text,
        )?;

        Ok(())
    }
}

// ---------------------------------------------------------------
// Explorer-view helpers
// ---------------------------------------------------------------

/// Maximum number of icon tiles allocated as SDI objects in Explorer view.
const MAX_TILES: usize = 48;
/// Maximum number of folder-tree lines.
const MAX_TREE_LINES: usize = 16;

/// Kind of entry shown in the Explorer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
    ParentDir,
}

/// One row in the folder tree pane.
struct TreeEntry {
    label: String,
    depth: usize,
    is_current: bool,
}

/// Geometry for the Explorer view, computed from a content rect.
struct ExplorerGeom {
    menu_x: i32,
    menu_y: i32,
    menu_w: u32,
    menu_h: u32,
    addr_y: i32,
    addr_h: u32,
    body_y: i32,
    body_h: u32,
    tree_x: i32,
    tree_w: u32,
    grid_x: i32,
    grid_w: u32,
    status_y: i32,
    status_h: u32,
    tile_w: u32,
    tile_h: u32,
    icon_w: u32,
    icon_h: u32,
    cols: usize,
    rows: usize,
}

fn compute_explorer_geom(cx: i32, cy: i32, cw: u32, ch: u32, at: &ActiveTheme) -> ExplorerGeom {
    let menu_h = ((at.font_hint as u32) + 6).max(14);
    let addr_h = ((at.font_hint as u32) + 8).max(16);
    let status_h = 14u32;
    let pad = 4i32;

    let menu_x = cx;
    let menu_y = cy;
    let menu_w = cw;
    let addr_y = menu_y + menu_h as i32;
    let body_y = addr_y + addr_h as i32 + pad;
    let status_y = cy + ch as i32 - status_h as i32;
    let body_h = (status_y - body_y).max(20) as u32;

    // Tree pane: ~28% width, clamped to keep both panes usable.
    let tree_w_target = ((cw as f32) * 0.28) as u32;
    let tree_w = tree_w_target.clamp(80, 200).min(cw.saturating_sub(120));
    let tree_x = cx + pad;

    let grid_x = cx + tree_w as i32 + pad;
    let grid_w = (cw as i32 - tree_w as i32 - pad * 2).max(60) as u32;

    let tile_w = 64u32.min(grid_w / 2).max(48);
    let tile_h = 56u32;
    let icon_w = 28u32;
    let icon_h = 24u32;

    let cols = ((grid_w.saturating_sub(8) / tile_w) as usize).max(1);
    let rows = ((body_h.saturating_sub(8) / tile_h) as usize).max(1);

    ExplorerGeom {
        menu_x,
        menu_y,
        menu_w,
        menu_h,
        addr_y,
        addr_h,
        body_y,
        body_h,
        tree_x,
        tree_w,
        grid_x,
        grid_w,
        status_y,
        status_h,
        tile_w,
        tile_h,
        icon_w,
        icon_h,
        cols,
        rows,
    }
}

fn parse_entry(line: &str) -> (String, EntryKind) {
    let trimmed = line.trim();
    if trimmed == ".." {
        ("..".to_string(), EntryKind::ParentDir)
    } else if let Some(name) = trimmed.strip_suffix('/') {
        (name.to_string(), EntryKind::Dir)
    } else {
        let name = trimmed.split("  (").next().unwrap_or(trimmed).to_string();
        (name, EntryKind::File)
    }
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars || max_chars < 2 {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Build the folder-tree-pane entries from the current path. The top entry is
/// always "Desktop", followed by each ancestor of `current` (root, then each
/// directory component down to and including `current`). The last component
/// is flagged `is_current`.
fn build_tree_entries(current: &str) -> Vec<TreeEntry> {
    let mut out = vec![TreeEntry {
        label: "Desktop".to_string(),
        depth: 0,
        is_current: false,
    }];
    let trimmed = current.trim_start_matches('/');
    out.push(TreeEntry {
        label: "/".to_string(),
        depth: 1,
        is_current: trimmed.is_empty(),
    });
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    let last = parts.len();
    for (i, part) in parts.iter().enumerate() {
        out.push(TreeEntry {
            label: (*part).to_string(),
            depth: i + 2,
            is_current: i + 1 == last,
        });
    }
    out
}

/// Rectangle in screen coordinates, used by Explorer-view SDI helpers.
#[derive(Clone, Copy)]
struct Box2d {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

fn ensure_rect(sdi: &mut SdiRegistry, name: &str, b: Box2d, color: Color, z: i32) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = b.x;
        obj.y = b.y;
        obj.w = b.w;
        obj.h = b.h;
        obj.color = color;
        obj.text = None;
        obj.stroke_width = None;
        obj.stroke_color = None;
        obj.visible = true;
        obj.z = z;
    }
}

/// Compact text style used by `ensure_text` to keep the helper's argument
/// count under clippy's `too_many_arguments` limit.
#[derive(Clone, Copy)]
struct TextStyle {
    font_size: u16,
    color: Color,
    z: i32,
}

fn ensure_text(sdi: &mut SdiRegistry, name: &str, text: &str, x: i32, y: i32, style: TextStyle) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
    if let Ok(obj) = sdi.get_mut(name) {
        obj.text = Some(text.to_string());
        obj.x = x;
        obj.y = y;
        obj.font_size = style.font_size;
        obj.text_color = style.color;
        obj.w = 0;
        obj.h = 0;
        obj.visible = true;
        obj.z = style.z;
    }
}

fn outline_rect(sdi: &mut SdiRegistry, base: &str, b: Box2d, color: Color, z: i32) {
    let Box2d { x, y, w, h } = b;
    ensure_rect(sdi, &format!("{base}_t"), Box2d { x, y, w, h: 1 }, color, z);
    ensure_rect(
        sdi,
        &format!("{base}_b"),
        Box2d {
            x,
            y: y + h as i32 - 1,
            w,
            h: 1,
        },
        color,
        z,
    );
    ensure_rect(sdi, &format!("{base}_l"), Box2d { x, y, w: 1, h }, color, z);
    ensure_rect(
        sdi,
        &format!("{base}_r"),
        Box2d {
            x: x + w as i32 - 1,
            y,
            w: 1,
            h,
        },
        color,
        z,
    );
}

fn draw_outline(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: Color,
) -> oasis_types::error::Result<()> {
    backend.fill_rect(x, y, w, 1, color)?;
    backend.fill_rect(x, y + h as i32 - 1, w, 1, color)?;
    backend.fill_rect(x, y, 1, h, color)?;
    backend.fill_rect(x + w as i32 - 1, y, 1, h, color)?;
    Ok(())
}

fn draw_icon(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    kind: EntryKind,
    dark: Color,
) -> oasis_types::error::Result<()> {
    match kind {
        EntryKind::Dir | EntryKind::ParentDir => {
            backend.fill_rect(x, y, w, h, Color::rgb(255, 207, 87))?;
            backend.fill_rect(x, y, w / 2, 4, Color::rgb(220, 170, 50))?;
        },
        EntryKind::File => {
            backend.fill_rect(x, y, w, h, Color::WHITE)?;
            let fold = 6u32;
            backend.fill_rect(
                x + w as i32 - fold as i32,
                y,
                fold,
                fold,
                Color::rgb(220, 220, 220),
            )?;
        },
    }
    draw_outline(backend, x, y, w, h, dark)
}

/// Hide all Explorer-view SDI objects.
fn hide_explorer_sdi(sdi: &mut SdiRegistry) {
    let fixed = [
        "app_xp_menu_bg",
        "app_xp_menu_text",
        "app_xp_menu_sep",
        "app_xp_addr_bg",
        "app_xp_addr_label",
        "app_xp_addr_field",
        "app_xp_addr_o_t",
        "app_xp_addr_o_b",
        "app_xp_addr_o_l",
        "app_xp_addr_o_r",
        "app_xp_addr_text",
        "app_xp_tree_bg",
        "app_xp_tree_o_t",
        "app_xp_tree_o_b",
        "app_xp_tree_o_l",
        "app_xp_tree_o_r",
        "app_xp_tree_sel",
        "app_xp_grid_bg",
        "app_xp_grid_o_t",
        "app_xp_grid_o_b",
        "app_xp_grid_o_l",
        "app_xp_grid_o_r",
        "app_xp_status_bg",
        "app_xp_status_text",
        "app_xp_status_hint",
    ];
    for name in &fixed {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
    for i in 0..MAX_TREE_LINES {
        if let Ok(obj) = sdi.get_mut(&format!("app_xp_tree_l_{i}")) {
            obj.visible = false;
        }
    }
    for i in 0..MAX_TILES {
        for prefix in [
            "app_xp_t_sel_",
            "app_xp_t_body_",
            "app_xp_t_accent_",
            "app_xp_t_lbl_",
        ] {
            if let Ok(obj) = sdi.get_mut(&format!("{prefix}{i}")) {
                obj.visible = false;
            }
        }
    }
}

/// Hide dual-panel SDI objects (used when switching to Explorer view).
fn hide_dual_panel_sdi(sdi: &mut SdiRegistry) {
    if let Ok(obj) = sdi.get_mut("app_divider") {
        obj.visible = false;
    }
    for i in 0..100 {
        let lp = format!("app_lp_line_{i}");
        if !sdi.contains(&lp) {
            break;
        }
        if let Ok(obj) = sdi.get_mut(&lp) {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut(&format!("app_rp_line_{i}")) {
            obj.visible = false;
        }
    }
}

impl App for FileManagerApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        if self.content.viewing_file.is_some() {
            return self.handle_file_viewer_input(button, vfs);
        }
        if matches!(button, Button::Select) {
            self.toggle_view_mode();
            return AppAction::None;
        }
        match self.view_mode {
            ViewMode::Dual => self.handle_dual_panel_input(button, vfs),
            ViewMode::Explorer => self.handle_explorer_input(button, vfs),
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);

        if self.content.viewing_file.is_some() {
            // File viewer mode: use generic content rendering. Hide
            // explorer/dual artefacts so they don't bleed through.
            hide_explorer_sdi(sdi);
            render_app_chrome(sdi, at);
            if !sdi.contains("app_title_text") {
                sdi.create("app_title_text");
            }
            render_content_sdi(&self.content, sdi, at);
            return;
        }

        render_app_chrome(sdi, at);
        if !sdi.contains("app_title_text") {
            sdi.create("app_title_text");
        }
        match self.view_mode {
            ViewMode::Dual => {
                hide_explorer_sdi(sdi);
                self.update_sdi_dual(sdi, at);
            },
            ViewMode::Explorer => {
                hide_dual_panel_sdi(sdi);
                self.update_sdi_explorer(sdi, at);
            },
        }
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        backend.fill_rect(cx, cy, cw, ch, at.app.bg)?;

        if self.content.viewing_file.is_some() {
            return draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at);
        }
        match self.view_mode {
            ViewMode::Dual => self.draw_windowed_dual(cx, cy, cw, ch, backend, at),
            ViewMode::Explorer => self.draw_windowed_explorer(cx, cy, cw, ch, backend, at),
        }
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
        hide_explorer_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn browse_dir(&self) -> Option<&str> {
        self.content.browse_dir.as_deref()
    }

    fn viewing_file(&self) -> Option<&str> {
        self.content.viewing_file.as_deref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn setup_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/home/user/music").unwrap();
        vfs.mkdir("/home/user/photos").unwrap();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/home/user/readme.txt", b"Hello!").unwrap();
        vfs.write("/etc/hostname", b"oasis").unwrap();
        vfs
    }

    #[test]
    fn parent_dir_root() {
        assert_eq!(parent_dir("/"), "/");
    }

    #[test]
    fn parent_dir_one_level() {
        assert_eq!(parent_dir("/home"), "/");
    }

    #[test]
    fn parent_dir_two_levels() {
        assert_eq!(parent_dir("/home/user"), "/home");
    }

    #[test]
    fn join_path_root() {
        assert_eq!(join_path("/", "home"), "/home");
    }

    #[test]
    fn join_path_nested() {
        assert_eq!(join_path("/home", "user"), "/home/user");
    }

    #[test]
    fn list_directory_root_no_dotdot() {
        let vfs = setup_vfs();
        let lines = list_directory(&vfs, "/");
        assert!(!lines.iter().any(|l| l == ".."));
        assert!(lines.iter().any(|l| l.starts_with("home")));
    }

    #[test]
    fn list_directory_subdir_has_dotdot() {
        let vfs = setup_vfs();
        let lines = list_directory(&vfs, "/home");
        assert!(lines.iter().any(|l| l == ".."));
    }

    #[test]
    fn list_directory_shows_sizes() {
        let vfs = setup_vfs();
        let lines = list_directory(&vfs, "/home/user");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("readme.txt") && l.contains("6 B"))
        );
    }

    #[test]
    fn file_manager_launch() {
        let vfs = setup_vfs();
        let fm = FileManagerApp::new("/apps/fm", &vfs);
        assert_eq!(fm.title(), "File Manager");
        assert!(fm.browse_dir().is_some());
        assert!(!fm.lines().is_empty());
        assert!(fm.lines().iter().any(|l| l.contains("home")));
    }

    #[test]
    fn file_manager_navigate_down() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        assert_eq!(fm.panels[0].cursor, 0);
        fm.handle_input(&Button::Down, &vfs);
        assert_eq!(fm.panels[0].cursor, 1);
    }

    #[test]
    fn file_manager_switch_panel() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        assert_eq!(fm.active_panel, 0);
        fm.handle_input(&Button::Right, &vfs);
        assert_eq!(fm.active_panel, 1);
        fm.handle_input(&Button::Left, &vfs);
        assert_eq!(fm.active_panel, 0);
    }

    #[test]
    fn file_manager_enter_directory() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        let home_idx = fm.panels[0]
            .lines
            .iter()
            .position(|l| l.starts_with("home"))
            .expect("home/ should be in listing");
        for _ in 0..home_idx {
            fm.handle_input(&Button::Down, &vfs);
        }
        fm.handle_input(&Button::Confirm, &vfs);
        assert_eq!(fm.panels[0].browse_dir, "/home");
    }

    #[test]
    fn file_manager_cancel_goes_up() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        // Enter /home first.
        let home_idx = fm.panels[0]
            .lines
            .iter()
            .position(|l| l.starts_with("home"))
            .expect("home/ should be in listing");
        for _ in 0..home_idx {
            fm.handle_input(&Button::Down, &vfs);
        }
        fm.handle_input(&Button::Confirm, &vfs);
        assert_eq!(fm.panels[0].browse_dir, "/home");

        let action = fm.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert_eq!(fm.panels[0].browse_dir, "/");
    }

    #[test]
    fn file_manager_cancel_at_root_exits() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        let action = fm.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn file_manager_open_file() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.open_file(&vfs, "/home/user/readme.txt");
        assert!(fm.viewing_file().is_some());
        assert!(fm.lines().iter().any(|l| l.contains("Hello!")));
    }

    #[test]
    fn file_manager_cancel_from_viewer() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.open_file(&vfs, "/home/user/readme.txt");
        assert!(fm.viewing_file().is_some());
        let action = fm.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert!(fm.viewing_file().is_none());
    }

    #[test]
    fn file_manager_open_nonexistent_noop() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.open_file(&vfs, "/does/not/exist.txt");
        assert!(fm.viewing_file().is_none());
    }

    #[test]
    fn view_generic_text_file() {
        let lines = view_generic_file("/test.txt", b"Hello\nWorld");
        assert!(lines.iter().any(|l| l.contains("Hello")));
        assert!(lines.iter().any(|l| l.contains("World")));
    }

    #[test]
    fn view_generic_binary_file() {
        let lines = view_generic_file("/data.bin", &[0x00, 0x01, 0xFF, 0xFE, 0x80]);
        assert!(lines.iter().any(|l| l.contains("Binary file")));
    }

    #[test]
    fn view_audio_wav() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&176400u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes());

        let lines = view_audio_file("/music/test.wav", &wav);
        assert!(lines.iter().any(|l| l.contains("WAV")));
        assert!(lines.iter().any(|l| l.contains("44100")));
    }

    #[test]
    fn view_audio_mp3() {
        let data = vec![0xFF, 0xFB, 0x90, 0x00, 0x00];
        let lines = view_audio_file("/music/song.mp3", &data);
        assert!(lines.iter().any(|l| l.contains("MP3")));
    }

    #[test]
    fn view_image_png() {
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&480u32.to_be_bytes());
        png.extend_from_slice(&272u32.to_be_bytes());
        png.push(8);
        png.push(6);
        png.extend_from_slice(&[0, 0, 0]);

        let lines = view_image_file("/photos/test.png", &png);
        assert!(lines.iter().any(|l| l.contains("PNG")));
        assert!(lines.iter().any(|l| l.contains("480 x 272")));
        assert!(lines.iter().any(|l| l.contains("RGBA")));
    }

    #[test]
    fn view_image_jpeg() {
        let data = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x01, 0x10, 0x01, 0xE0, 0x03, 0x01, 0x22,
            0x00,
        ];
        let lines = view_image_file("/photos/pic.jpg", &data);
        assert!(lines.iter().any(|l| l.contains("JPEG")));
        assert!(lines.iter().any(|l| l.contains("480 x 272")));
    }

    #[test]
    fn view_mode_defaults_to_dual() {
        let vfs = setup_vfs();
        let fm = FileManagerApp::new("/apps/fm", &vfs);
        assert_eq!(fm.view_mode, ViewMode::Dual);
    }

    #[test]
    fn select_toggles_view_mode() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Explorer);
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Dual);
    }

    #[test]
    fn select_does_not_toggle_in_file_viewer() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.open_file(&vfs, "/home/user/readme.txt");
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Dual);
    }

    #[test]
    fn explorer_right_advances_cursor() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        // Force a 4-column grid so Right increments the cursor by 1.
        fm.explorer_cols.set(4);
        fm.explorer_visible_rows.set(4);
        let start = fm.panels[0].cursor;
        fm.handle_input(&Button::Right, &vfs);
        assert_eq!(fm.panels[0].cursor, start + 1);
    }

    #[test]
    fn explorer_down_advances_by_columns() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        // Root has 3 entries (home, etc, tmp). With cols=2, Down moves the
        // absolute cursor by 2 -> entry 2 which is in range.
        fm.explorer_cols.set(2);
        fm.explorer_visible_rows.set(4);
        let start_abs = fm.panels[0].scroll + fm.panels[0].cursor;
        fm.handle_input(&Button::Down, &vfs);
        let new_abs = fm.panels[0].scroll + fm.panels[0].cursor;
        assert_eq!(new_abs, start_abs + 2);
    }

    #[test]
    fn explorer_cancel_at_root_exits() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        let action = fm.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn explorer_enter_directory() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        fm.explorer_cols.set(4);
        fm.explorer_visible_rows.set(4);
        let home_idx = fm.panels[0]
            .lines
            .iter()
            .position(|l| l.starts_with("home"))
            .expect("home/ should be in listing");
        for _ in 0..home_idx {
            fm.handle_input(&Button::Right, &vfs);
        }
        fm.handle_input(&Button::Confirm, &vfs);
        assert_eq!(fm.panels[0].browse_dir, "/home");
    }

    #[test]
    fn parse_entry_classifies_correctly() {
        assert!(matches!(parse_entry("..").1, EntryKind::ParentDir));
        assert!(matches!(parse_entry("home/").1, EntryKind::Dir));
        let (name, kind) = parse_entry("readme.txt  (6 B)");
        assert_eq!(name, "readme.txt");
        assert!(matches!(kind, EntryKind::File));
    }

    #[test]
    fn build_tree_entries_marks_current() {
        let entries = build_tree_entries("/home/user");
        let current = entries
            .iter()
            .find(|e| e.is_current)
            .expect("must mark current");
        assert_eq!(current.label, "user");
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Desktop"));
        assert!(labels.contains(&"/"));
        assert!(labels.contains(&"home"));
    }

    #[test]
    fn build_tree_entries_root_is_current() {
        let entries = build_tree_entries("/");
        let current = entries
            .iter()
            .find(|e| e.is_current)
            .expect("root must be current");
        assert_eq!(current.label, "/");
    }
}
