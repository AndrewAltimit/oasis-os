//! File Manager application with dual-panel browsing.
//!
//! Implements the `App` trait for a dual-panel file manager with
//! file viewing capabilities (text, audio metadata, image metadata).

use std::any::Any;

use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
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
        }
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
        self.handle_dual_panel_input(button, vfs)
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);

        // Dual-panel rendering.
        if self.content.viewing_file.is_none() {
            // Common app chrome (bg, title bar).
            render_app_chrome(sdi, at);
            if !sdi.contains("app_title_text") {
                sdi.create("app_title_text");
            }
            self.update_sdi_dual(sdi, at);
            return;
        }

        // File viewer mode: use generic content rendering.
        render_app_chrome(sdi, at);
        if !sdi.contains("app_title_text") {
            sdi.create("app_title_text");
        }
        render_content_sdi(&self.content, sdi, at);
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

        if self.content.viewing_file.is_none() {
            return self.draw_windowed_dual(cx, cy, cw, ch, backend, at);
        }

        // File viewer: generic content rendering.
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
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
}
