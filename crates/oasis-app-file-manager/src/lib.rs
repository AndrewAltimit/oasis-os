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
//! - `view` — direct-draw + SDI rendering for both view modes.
//!
//! This file re-exports the public API and contains the `App` trait impl.

use std::any::Any;

use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_ui::menu_bar::MenuHit;
use oasis_vfs::Vfs;

pub(crate) mod commands;
pub(crate) mod model;
pub(crate) mod state;
pub(crate) mod view;

pub use model::{FileOp, FilePanel, NavTarget, TreeEntry, ViewMode};
pub use state::FileManagerApp;

pub use oasis_app_core::file_viewer::{
    app_for_file, join_path, list_directory, parent_dir, view_audio_file, view_generic_file,
    view_image_file,
};

use view::{
    FM_MENU_H, compute_explorer_geom, grid_hit_test, hide_dual_panel_sdi, hide_explorer_sdi,
    hide_menu_sdi, tree_hit_test,
};

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

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, fullscreen: bool) -> AppAction {
        if self.content.viewing_file.is_some() {
            return AppAction::None;
        }
        let title_h = self.content.cached_title_bar_height.max(16) as i32;
        let menu_y = title_h;

        // 1. Menu bar.
        match self.menu.hit_test(lx, ly, 0, menu_y, cw, FM_MENU_H) {
            MenuHit::Label(i) => {
                if self.menu.open == Some(i) {
                    self.menu.close();
                } else {
                    self.menu.open = Some(i);
                    self.menu.hovered_item = None;
                }
                self.last_click_tile.set(None);
                return AppAction::None;
            },
            MenuHit::Item { id } => {
                self.menu.close();
                self.last_click_tile.set(None);
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
        // (`addr_y = cy + menu_h`), so `cy` here is the top of the menu —
        // i.e. just below the title bar. This matches `update_sdi_explorer`
        // (`body_top = title_h`) and `draw_windowed_explorer`
        // (`body_top = cy + title_h`).
        let body_top = title_h;
        // In SDI/fullscreen mode `ch == screen_h` and the renderer subtracts
        // `statusbar_height + bottombar_height` before computing geometry —
        // mirror that here so hit-tests don't extend over the system bars.
        // In windowed mode `ch` already excludes the system bars.
        let system_bars = if fullscreen {
            self.cached_system_bars.get() as i32
        } else {
            0
        };
        let body_h_local = (ch as i32 - body_top - system_bars).max(20) as u32;
        let g = compute_explorer_geom(0, body_top, cw, body_h_local);

        // Tree row click: single-click navigates immediately.
        let font_hint = self.cached_font_hint.get();
        if let Some(target) = tree_hit_test(&g, lx, ly, &self.active().tree_entries, font_hint) {
            self.pending_navigation = Some(NavTarget::Folder(target));
            self.last_click_tile.set(None);
            return AppAction::None;
        }

        // Icon grid click: select on first click, activate on second.
        // `grid_hit_test` only returns indices for tiles already visible
        // within the current scroll window, so don't shift `scroll` —
        // re-aligning the row would jump the view under the user's cursor.
        if let Some(abs) = grid_hit_test(&g, lx, ly, &self.active().lines, self.active().scroll) {
            let panel = self.active_mut();
            panel.cursor = abs.saturating_sub(panel.scroll);
            if self.last_click_tile.get() == Some(abs) {
                self.last_click_tile.set(None);
                return self.activate_index(abs);
            }
            self.last_click_tile.set(Some(abs));
            return AppAction::None;
        }

        self.last_click_tile.set(None);
        AppAction::None
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
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

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);

        if self.content.viewing_file.is_some() {
            // File viewer mode: use generic content rendering. Hide
            // explorer/dual/menu artefacts so they don't bleed through.
            hide_explorer_sdi(sdi);
            hide_menu_sdi(sdi);
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
        hide_menu_sdi(sdi);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EntryKind, build_tree_entries, parse_entry};
    use crate::view::compute_explorer_geom;
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
        fm.view_mode = ViewMode::Dual;
        assert_eq!(fm.panels[0].cursor, 0);
        fm.handle_input(&Button::Down, &vfs);
        assert_eq!(fm.panels[0].cursor, 1);
    }

    #[test]
    fn file_manager_switch_panel() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.view_mode = ViewMode::Dual;
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
    fn view_mode_defaults_to_explorer() {
        let vfs = setup_vfs();
        let fm = FileManagerApp::new("/apps/fm", &vfs);
        assert_eq!(fm.view_mode, ViewMode::Explorer);
    }

    #[test]
    fn select_toggles_view_mode() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Dual);
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Explorer);
    }

    #[test]
    fn select_does_not_toggle_in_file_viewer() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.open_file(&vfs, "/home/user/readme.txt");
        fm.handle_input(&Button::Select, &vfs);
        assert_eq!(fm.view_mode, ViewMode::Explorer);
    }

    #[test]
    fn explorer_right_advances_cursor() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
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
        let action = fm.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn explorer_enter_directory() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
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
    fn view_menu_grid_action_sets_explorer() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.view_mode = ViewMode::Dual;
        let action = fm.run_menu_action("view.grid");
        assert_eq!(action, AppAction::None);
        assert_eq!(fm.view_mode, ViewMode::Explorer);
    }

    #[test]
    fn view_menu_list_action_sets_dual() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        let action = fm.run_menu_action("view.list");
        assert_eq!(action, AppAction::None);
        assert_eq!(fm.view_mode, ViewMode::Dual);
    }

    #[test]
    fn file_close_action_exits() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        let action = fm.run_menu_action("file.close");
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn double_click_on_folder_navigates_after_refresh() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        fm.explorer_cols.set(4);
        fm.explorer_visible_rows.set(4);
        // Pick whichever directory ends up first after sorting.
        let first = fm.panels[0].lines[0].clone();
        let expected_name = first.trim_end_matches('/');
        let expected_path = format!("/{expected_name}");
        // Mirror the handler: body_top = title_h (20), body_h = ch - 20.
        let g = compute_explorer_geom(0, 20, 600, 380);
        let tile_x = g.grid_x + 4 + g.tile_w as i32 / 2;
        let tile_y = g.body_y + 4 + g.tile_h as i32 / 2;
        // First click selects.
        fm.handle_click(tile_x, tile_y, 600, 400, false);
        assert_eq!(fm.last_click_tile.get(), Some(0));
        assert!(fm.pending_navigation.is_none());
        // Second click on same tile activates.
        fm.handle_click(tile_x, tile_y, 600, 400, false);
        match &fm.pending_navigation {
            Some(NavTarget::Folder(p)) => assert_eq!(p, &expected_path),
            other => panic!("expected Folder({expected_path}), got {other:?}"),
        }
        fm.refresh(&vfs);
        assert_eq!(fm.panels[0].browse_dir, expected_path);
        assert!(fm.pending_navigation.is_none());
    }

    #[test]
    fn click_on_tree_row_navigates_immediately() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        // First go into /home so the tree has multiple rows.
        fm.pending_navigation = Some(NavTarget::Folder("/home".to_string()));
        fm.refresh(&vfs);
        // Tree row 1 is "/" (root). Compute its position. Mirror the handler:
        // body_top = title_h (20), body_h = ch - 20.
        let g = compute_explorer_geom(0, 20, 600, 380);
        // cached_font_hint defaults to 11 → (11+2).max(11) = 13.
        let tree_line_h = 13;
        let row_idx = 1; // "/" entry.
        let tx = g.tree_x + 8;
        let ty = g.body_y + 4 + row_idx * tree_line_h + tree_line_h / 2;
        fm.handle_click(tx, ty, 600, 400, false);
        assert!(matches!(
            fm.pending_navigation,
            Some(NavTarget::Folder(ref p)) if p == "/"
        ));
        fm.refresh(&vfs);
        assert_eq!(fm.panels[0].browse_dir, "/");
    }

    #[test]
    fn click_on_view_label_opens_dropdown() {
        let vfs = setup_vfs();
        let mut fm = FileManagerApp::new("/apps/fm", &vfs);
        // Title bar is 20px tall by default. Menu bar starts at y=20,
        // 18px tall. The View label is the third entry (after File+Edit).
        let file_w = 4 * 7 + 16; // "File"
        let edit_w = 4 * 7 + 16; // "Edit"
        let view_x = 6 + file_w + edit_w + 4;
        let action = fm.handle_click(view_x, 28, 600, 400, false);
        assert_eq!(action, AppAction::None);
        assert_eq!(fm.menu.open, Some(2));
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
    fn build_tree_entries_marks_current_and_lists_siblings() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/home/guest").unwrap();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/var").unwrap();

        let entries = build_tree_entries("/home/user", &vfs);
        let current = entries
            .iter()
            .find(|e| e.is_current)
            .expect("must mark current");
        assert_eq!(current.label, "user");
        assert_eq!(current.path, "/home/user");

        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        // Path crumbs.
        assert!(labels.contains(&"Desktop"));
        assert!(labels.contains(&"/"));
        assert!(labels.contains(&"home"));
        // Siblings of `home` at root level.
        assert!(labels.contains(&"etc"));
        assert!(labels.contains(&"var"));
        // Siblings of `user` inside `home`.
        assert!(labels.contains(&"guest"));
    }

    #[test]
    fn build_tree_entries_root_is_current() {
        let vfs = MemoryVfs::new();
        let entries = build_tree_entries("/", &vfs);
        let current = entries
            .iter()
            .find(|e| e.is_current)
            .expect("root must be current");
        assert_eq!(current.label, "/");
        assert_eq!(current.path, "/");
    }

    #[test]
    fn build_tree_entries_lists_children_of_current_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/home/user/documents").unwrap();
        vfs.mkdir("/home/user/photos").unwrap();
        vfs.mkdir("/home/user/scripts").unwrap();

        let entries = build_tree_entries("/home/user", &vfs);
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        // Children of the current directory must be navigable from the tree.
        assert!(labels.contains(&"documents"), "got {labels:?}");
        assert!(labels.contains(&"photos"), "got {labels:?}");
        assert!(labels.contains(&"scripts"), "got {labels:?}");

        // And those children should carry their absolute paths so a click
        // on the row can navigate directly.
        let docs = entries
            .iter()
            .find(|e| e.label == "documents")
            .expect("documents row");
        assert_eq!(docs.path, "/home/user/documents");
    }
}
