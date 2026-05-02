//! Input handling and menu-action dispatch for the File Manager app.
//!
//! All [`FileManagerApp`] methods that translate a user gesture (button
//! press or menu pick) into state changes live here. Side-effect-free
//! data lookups are imported from [`crate::model`]; rendering is in
//! [`crate::view`].

use oasis_app_core::AppAction;
use oasis_app_core::file_viewer::{app_for_file, join_path, parent_dir, view_generic_file};
use oasis_types::input::Button;
use oasis_ui::menu_bar::{Menu, MenuBar, MenuEntry};
use oasis_vfs::Vfs;

use crate::model::{FileOp, NavTarget, ViewMode};
use crate::state::FileManagerApp;

/// Build the default file-manager menu bar.
pub(crate) fn default_menu_bar() -> MenuBar {
    MenuBar::new(vec![
        Menu::new(
            "File",
            vec![MenuEntry::action("Close", "file.close").with_shortcut("Esc")],
        ),
        Menu::new(
            "Edit",
            vec![
                MenuEntry::action("New Folder", "edit.mkdir").with_shortcut("\u{25a1}"),
                MenuEntry::action("Delete", "edit.delete").with_shortcut("\u{25b3}"),
            ],
        ),
        Menu::new(
            "View",
            vec![
                MenuEntry::action("Grid", "view.grid"),
                MenuEntry::action("List", "view.list"),
            ],
        ),
    ])
}

impl FileManagerApp {
    /// Queue navigation/open for the entry at `abs_idx` of the active panel.
    /// Used by click activation; the actual vfs work happens in `refresh`.
    pub(crate) fn activate_index(&mut self, abs_idx: usize) -> AppAction {
        let p = self.active();
        let Some(line) = p.lines.get(abs_idx).cloned() else {
            return AppAction::None;
        };
        let trimmed = line.trim();
        if trimmed == ".." {
            self.pending_navigation = Some(NavTarget::Folder(parent_dir(&p.browse_dir)));
            return AppAction::None;
        }
        if let Some(name) = trimmed.strip_suffix('/') {
            self.pending_navigation = Some(NavTarget::Folder(join_path(&p.browse_dir, name)));
            return AppAction::None;
        }
        let name = trimmed.split("  (").next().unwrap_or(trimmed);
        let file_path = join_path(&p.browse_dir, name);
        self.pending_navigation = Some(NavTarget::File(file_path));
        AppAction::None
    }

    /// Dispatch a menu-bar action by id.
    pub(crate) fn run_menu_action(&mut self, id: &str) -> AppAction {
        match id {
            "view.grid" => {
                self.view_mode = ViewMode::Explorer;
            },
            "view.list" => {
                self.view_mode = ViewMode::Dual;
            },
            "edit.mkdir" => {
                let dir = self.active().browse_dir.clone();
                self.pending_op = Some(FileOp::Mkdir(join_path(&dir, "new_folder")));
            },
            "edit.delete" => {
                if let Some(path) = self.active().selected_path() {
                    self.pending_op = Some(FileOp::Delete(path));
                }
            },
            "file.close" => return AppAction::Exit,
            _ => {},
        }
        AppAction::None
    }

    /// Handle input in dual-panel mode (no file viewer open).
    pub(crate) fn handle_dual_panel_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
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

    /// Move the active panel's cursor by `delta` tiles in the Explorer grid,
    /// keeping the scrolled-into-view invariant intact.
    pub(crate) fn explorer_move_cursor(&mut self, delta: isize) {
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
    pub(crate) fn handle_explorer_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
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
    pub(crate) fn handle_file_viewer_input(
        &mut self,
        button: &Button,
        _vfs: &dyn Vfs,
    ) -> AppAction {
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
}
