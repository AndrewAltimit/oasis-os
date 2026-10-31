//! File Manager application with dual-panel browsing.
//!
//! Implements the `App` trait for a dual-panel file manager with
//! file viewing capabilities (text, audio metadata, image metadata).
//!
//! The crate is organised into focused (crate-private) submodules:
//! - `model` — pure data types ([`FilePanel`], [`FileOp`], [`NavTarget`],
//!   [`ViewMode`], [`TreeEntry`]) and side-effect-free helpers.
//! - `state` — the [`FileManagerApp`] struct definition + constructors
//!   + simple accessors.
//! - `commands` — input handling and menu-action dispatch.
//! - `view` — windowed rendering for both view modes.
//!
//! This file re-exports the public API and contains the `App` trait impl.

use oasis_app_core::render::draw_content_windowed;
use oasis_app_core::{App, AppAction};
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_ui::menu_bar::MenuHit;
use oasis_vfs::Vfs;

pub mod colors;
pub(crate) mod commands;
pub(crate) mod dialog;
pub(crate) mod model;
pub(crate) mod ops;
pub(crate) mod state;
pub(crate) mod view;

pub use colors::FileManagerColors;
pub use model::{
    Clipboard, Dialog, FileOp, FilePanel, NamePurpose, NavTarget, TreeEntry, ViewMode,
};
pub use state::FileManagerApp;

pub use oasis_app_core::file_viewer::{
    app_for_file, join_path, list_directory, parent_dir, view_audio_file, view_generic_file,
    view_image_file,
};

use dialog::DialogClick;
use view::{FM_MENU_H, compute_explorer_geom, grid_hit_test, tree_hit_test};

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
        if self.dialog.is_some() {
            return self.handle_dialog_input(button);
        }
        // Operation feedback lasts until the next press.
        self.status = None;
        if matches!(button, Button::Select) {
            self.toggle_view_mode();
            return AppAction::None;
        }
        match self.view_mode {
            ViewMode::Dual => self.handle_dual_panel_input(button, vfs),
            ViewMode::Explorer => self.handle_explorer_input(button, vfs),
        }
    }

    fn handle_text_input(&mut self, ch: char) {
        self.dialog_type_char(ch);
    }

    fn handle_backspace(&mut self) {
        self.dialog_backspace();
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        self.handle_fm_key(key, mods)
    }

    fn accepts_text(&self) -> bool {
        matches!(self.dialog, Some(Dialog::NameEntry { .. }))
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        self.apply_pending_ops(vfs)
    }

    fn tick(&mut self, dt_ms: u32, vfs: &dyn Vfs) -> bool {
        self.rescan_elapsed_ms = self.rescan_elapsed_ms.saturating_add(dt_ms);
        if self.rescan_elapsed_ms < state::RESCAN_INTERVAL_MS {
            return false;
        }
        self.rescan_elapsed_ms = 0;
        self.rescan(vfs)
    }

    fn layout(&mut self, cw: u32, ch: u32, at: &ActiveTheme) {
        self.content_size = (cw, ch);
        self.content.update_layout_windowed(ch, at);
        // Same geometry `draw_windowed_explorer` draws with (no inner title
        // bar; the grid metrics don't depend on the window origin).
        let g = compute_explorer_geom(0, 0, cw, ch);
        self.explorer_cols = g.cols;
        self.explorer_visible_rows = g.rows;
        self.dual_visible_rows = view::dual_max_lines(ch, at);
        self.cached_font_hint = at.font_hint;
    }

    fn handle_click(&mut self, lx: i32, ly: i32, vfs: &dyn Vfs) -> AppAction {
        let (cw, ch) = self.content_size;
        let action = self.click(lx, ly, cw, ch);
        self.apply_pending_navigation(vfs);
        action
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
        backend.fill_rect(cx, cy, cw, ch, FileManagerColors::from_theme(at).bg)?;

        if self.content.viewing_file.is_some() {
            return draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at);
        }
        match self.view_mode {
            ViewMode::Dual => self.draw_windowed_dual(cx, cy, cw, ch, backend, at)?,
            ViewMode::Explorer => self.draw_windowed_explorer(cx, cy, cw, ch, backend, at)?,
        }
        self.draw_dialog_windowed(cx, cy, cw, ch, backend, at)
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
}

impl FileManagerApp {
    /// Content-area click: hit-test against the geometry recorded by
    /// `App::layout`. Folder activations land in `pending_navigation`.
    fn click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32) -> AppAction {
        if self.content.viewing_file.is_some() {
            return AppAction::None;
        }
        // An open dialog is modal: only its buttons react.
        if self.dialog.is_some() {
            match self.dialog_hit(lx, ly, cw, ch) {
                DialogClick::Ok => self.commit_dialog(),
                DialogClick::Cancel => self.cancel_dialog(),
                DialogClick::None => {},
            }
            return AppAction::None;
        }
        let title_h = self.content.cached_title_bar_height.max(16) as i32;
        // Only `draw_windowed_dual` paints an inner header row (a
        // panel-paths context strip at `title_bar_height`);
        // `draw_windowed_explorer` reuses the WM titlebar and starts the
        // menu at the top of the content rect. Mirror that here so
        // hit-tests align with what's drawn.
        let inner_title_bar = matches!(self.view_mode, ViewMode::Dual);
        let menu_y = if inner_title_bar { title_h } else { 0 };

        // 1. Menu bar.
        match self.menu.hit_test(lx, ly, 0, menu_y, cw, FM_MENU_H) {
            MenuHit::Label(i) => {
                if self.menu.open == Some(i) {
                    self.menu.close();
                } else {
                    self.menu.open = Some(i);
                    self.menu.hovered_item = None;
                }
                self.last_click_tile = None;
                return AppAction::None;
            },
            MenuHit::Item { id } => {
                self.menu.close();
                self.last_click_tile = None;
                return self.run_menu_action(&id);
            },
            MenuHit::NoOp => return AppAction::None,
            MenuHit::Outside => {
                if self.menu.is_open() {
                    self.menu.close();
                    return AppAction::None;
                }
            },
        }

        // 2. Explorer view: tree pane + icon grid.
        if !matches!(self.view_mode, ViewMode::Explorer) {
            return AppAction::None;
        }
        // `compute_explorer_geom` carves out the menu strip internally
        // (`addr_y = cy + menu_h`), so `cy` here is the top of the menu.
        // There is no inner title bar (`draw_windowed_explorer` skips it)
        // and the WM-relative click coords already start past the WM
        // titlebar.
        let body_top = menu_y;
        let body_h_local = (ch as i32 - body_top).max(20) as u32;
        let g = compute_explorer_geom(0, body_top, cw, body_h_local);

        // Tree row click: single-click navigates immediately.
        let font_hint = self.cached_font_hint;
        if let Some(target) = tree_hit_test(&g, lx, ly, &self.active().tree_entries, font_hint) {
            self.pending_navigation = Some(NavTarget::Folder(target));
            self.last_click_tile = None;
            return AppAction::None;
        }

        // Icon grid click: select on first click, activate on second.
        // `grid_hit_test` only returns indices for tiles already visible
        // within the current scroll window, so don't shift `scroll` —
        // re-aligning the row would jump the view under the user's cursor.
        if let Some(abs) = grid_hit_test(&g, lx, ly, &self.active().lines, self.active().scroll) {
            let panel = self.active_mut();
            panel.cursor = abs.saturating_sub(panel.scroll);
            if self.last_click_tile == Some(abs) {
                self.last_click_tile = None;
                return self.activate_index(abs);
            }
            self.last_click_tile = Some(abs);
            return AppAction::None;
        }

        self.last_click_tile = None;
        AppAction::None
    }

    /// Apply a click's queued navigation / file open with the vfs.
    pub(crate) fn apply_pending_navigation(&mut self, vfs: &dyn Vfs) {
        let Some(target) = self.pending_navigation.take() else {
            return;
        };
        match target {
            NavTarget::Folder(path) => {
                self.active_mut().navigate_to(&path, vfs);
                self.content.browse_dir = Some(path);
            },
            NavTarget::File(path) => {
                self.open_file(vfs, &path);
            },
        }
    }
}

#[cfg(test)]
mod tests;
