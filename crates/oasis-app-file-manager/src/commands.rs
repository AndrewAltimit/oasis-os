//! Input handling and menu-action dispatch for the File Manager app.
//!
//! All [`FileManagerApp`] methods that translate a user gesture (button
//! press or menu pick) into state changes live here. Side-effect-free
//! data lookups are imported from [`crate::model`]; rendering is in
//! [`crate::view`].

use oasis_app_core::AppAction;
use oasis_app_core::file_viewer::{app_for_file, join_path, parent_dir, view_generic_file};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_ui::menu_bar::{Menu, MenuBar, MenuEntry};
use oasis_vfs::Vfs;

use crate::model::{Clipboard, Dialog, FileOp, NamePurpose, NavTarget, ViewMode};
use crate::ops::{MAX_NAME_LEN, apply_file_op, file_name, validate_name};
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
                MenuEntry::action("Rename", "edit.rename").with_shortcut("F2"),
                MenuEntry::Separator,
                MenuEntry::action("Cut", "edit.cut").with_shortcut("Ctrl+X"),
                MenuEntry::action("Copy", "edit.copy").with_shortcut("Ctrl+C"),
                MenuEntry::action("Paste", "edit.paste").with_shortcut("Ctrl+V"),
                MenuEntry::Separator,
                MenuEntry::action("Delete", "edit.delete").with_shortcut("Del"),
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
            "edit.mkdir" => self.begin_new_folder(),
            "edit.rename" => self.begin_rename(),
            "edit.cut" => self.clipboard_set(true),
            "edit.copy" => self.clipboard_set(false),
            "edit.paste" => self.paste(),
            "edit.delete" => self.request_delete(),
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
                // Delete selected entry (after a Yes/No confirmation).
                self.request_delete();
                AppAction::None
            },
            Button::Square => {
                // New folder in the active directory (name prompt).
                self.begin_new_folder();
                AppAction::None
            },
            Button::Start => {
                self.copy_or_paste();
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
                // Delete selected entry (after a Yes/No confirmation).
                self.request_delete();
                AppAction::None
            },
            Button::Square => {
                // New folder in the active directory (name prompt).
                self.begin_new_folder();
                AppAction::None
            },
            Button::Start => {
                self.copy_or_paste();
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

// ---------------------------------------------------------------
// File operations: dialogs, clipboard, keyboard shortcuts
// ---------------------------------------------------------------

impl FileManagerApp {
    /// Open the "Delete X?" confirmation for the selected entry.
    pub(crate) fn request_delete(&mut self) {
        self.status = None;
        if let Some(path) = self.active().selected_path() {
            self.dialog = Some(Dialog::ConfirmDelete { path });
        }
    }

    /// Open the name prompt for a new folder in the active directory.
    pub(crate) fn begin_new_folder(&mut self) {
        self.status = None;
        let dir = self.active().browse_dir.clone();
        self.dialog = Some(Dialog::NameEntry {
            purpose: NamePurpose::NewFolder { dir },
            text: "new_folder".to_string(),
        });
    }

    /// Open the name prompt pre-filled with the selected entry's name.
    pub(crate) fn begin_rename(&mut self) {
        self.status = None;
        if let Some(from) = self.active().selected_path() {
            let text = file_name(&from).to_string();
            self.dialog = Some(Dialog::NameEntry {
                purpose: NamePurpose::Rename { from },
                text,
            });
        }
    }

    /// Put the selected entry on the clipboard (Cut when `cut`).
    pub(crate) fn clipboard_set(&mut self, cut: bool) {
        if let Some(path) = self.active().selected_path() {
            let verb = if cut { "Cut" } else { "Copied" };
            self.status = Some(format!("{verb} {} to clipboard", file_name(&path)));
            self.clipboard = Some(Clipboard { path, cut });
        }
    }

    /// Paste the clipboard entry into the active panel's directory.
    /// A Cut clipboard is consumed (the entry moves); Copy can be pasted
    /// repeatedly.
    pub(crate) fn paste(&mut self) {
        let Some(clip) = self.clipboard.clone() else {
            self.status = Some("Clipboard is empty".to_string());
            return;
        };
        let to_dir = self.active().browse_dir.clone();
        if clip.cut {
            self.clipboard = None;
            self.pending_ops.push(FileOp::Move {
                from: clip.path,
                to_dir,
            });
        } else {
            self.pending_ops.push(FileOp::Copy {
                from: clip.path,
                to_dir,
            });
        }
    }

    /// Gamepad shortcut (Start): copy the selection when the clipboard is
    /// empty, otherwise paste it. Keeps clipboard use reachable on hosts
    /// without a keyboard.
    pub(crate) fn copy_or_paste(&mut self) {
        if self.clipboard.is_some() {
            self.paste();
        } else {
            self.clipboard_set(false);
        }
    }

    /// Close the open dialog without doing anything.
    pub(crate) fn cancel_dialog(&mut self) {
        self.dialog = None;
    }

    /// Accept the open dialog: queue the delete / mkdir / rename. An
    /// invalid name keeps the prompt open and reports why in `status`.
    pub(crate) fn commit_dialog(&mut self) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match dialog {
            Dialog::ConfirmDelete { path } => {
                self.pending_ops.push(FileOp::Delete(path));
            },
            Dialog::NameEntry { purpose, text } => {
                let name = match validate_name(&text) {
                    Ok(name) => name.to_string(),
                    Err(msg) => {
                        self.status = Some(msg);
                        self.dialog = Some(Dialog::NameEntry { purpose, text });
                        return;
                    },
                };
                match purpose {
                    NamePurpose::NewFolder { dir } => {
                        self.pending_ops.push(FileOp::Mkdir(join_path(&dir, &name)));
                    },
                    NamePurpose::Rename { from } => {
                        let to = join_path(&parent_dir(&from), &name);
                        if to != from {
                            self.pending_ops.push(FileOp::Rename { from, to });
                        }
                    },
                }
            },
        }
    }

    /// Gamepad input while a dialog is open: Confirm accepts, Cancel
    /// dismisses, everything else is swallowed.
    pub(crate) fn handle_dialog_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Confirm => self.commit_dialog(),
            Button::Cancel => self.cancel_dialog(),
            _ => {},
        }
        AppAction::None
    }

    /// Keyboard shortcuts. See `App::handle_key`.
    pub(crate) fn handle_fm_key(&mut self, key: &Key, mods: Modifiers) -> Option<AppAction> {
        if self.content.viewing_file.is_some() {
            return None;
        }
        if let Some(dialog) = &self.dialog {
            let is_confirm = matches!(dialog, Dialog::ConfirmDelete { .. });
            match key {
                Key::Enter => self.commit_dialog(),
                Key::Escape => self.cancel_dialog(),
                Key::Char('y' | 'Y') if is_confirm => self.commit_dialog(),
                Key::Char('n' | 'N') if is_confirm => self.cancel_dialog(),
                // Plain typing reaches the name field via handle_text_input,
                // and Backspace via its handle_backspace twin -- swallowing
                // it here made the field impossible to edit from a keyboard.
                Key::Char(_) | Key::Space if !is_confirm && !mods.has_command() => return None,
                Key::Backspace if !is_confirm => return None,
                _ => {},
            }
            // Swallow everything else so the gamepad twin (e.g. an arrow's
            // d-pad press) can't act on the listing behind the dialog.
            return Some(AppAction::None);
        }
        if mods.only(Modifiers::CTRL) || mods.only(Modifiers::SUPER) {
            match key {
                Key::Char('c' | 'C') => self.clipboard_set(false),
                Key::Char('x' | 'X') => self.clipboard_set(true),
                Key::Char('v' | 'V') => self.paste(),
                _ => return None,
            }
            return Some(AppAction::None);
        }
        if mods.only(Modifiers::CTRL | Modifiers::SHIFT) && matches!(key, Key::Char('n' | 'N')) {
            self.begin_new_folder();
            return Some(AppAction::None);
        }
        match key {
            Key::Delete => self.request_delete(),
            Key::F(2) => self.begin_rename(),
            _ => return None,
        }
        Some(AppAction::None)
    }

    /// Append a typed character to the name field (name entry only).
    pub(crate) fn dialog_type_char(&mut self, ch: char) {
        if let Some(Dialog::NameEntry { text, .. }) = &mut self.dialog
            && !ch.is_control()
            && text.chars().count() < MAX_NAME_LEN
        {
            text.push(ch);
        }
    }

    /// Delete the last character of the name field (name entry only).
    pub(crate) fn dialog_backspace(&mut self) {
        if let Some(Dialog::NameEntry { text, .. }) = &mut self.dialog {
            text.pop();
        }
    }

    /// Apply every queued [`FileOp`], then re-list both panels so the
    /// result shows up immediately. Returns `true` if anything ran.
    pub(crate) fn apply_pending_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        if self.pending_ops.is_empty() {
            return false;
        }
        for op in std::mem::take(&mut self.pending_ops) {
            match apply_file_op(vfs, &op) {
                Ok(msg) if msg.is_empty() => {},
                Ok(msg) => self.status = Some(msg),
                Err(e) => self.status = Some(format!("Error: {e}")),
            }
        }
        for panel in &mut self.panels {
            // A panel whose directory vanished (deleted / moved) falls
            // back to the nearest surviving ancestor.
            let mut dir = panel.browse_dir.clone();
            while dir != "/" && !vfs.exists(&dir) {
                dir = parent_dir(&dir);
            }
            if dir == panel.browse_dir {
                panel.refresh(vfs);
            } else {
                panel.navigate_to(&dir, vfs);
            }
        }
        self.content.browse_dir = Some(self.active().browse_dir.clone());
        true
    }
}
