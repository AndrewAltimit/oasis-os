//! File-browsing app for Music Player and Photo Viewer.
//!
//! Both apps share the same behavior: start in a directory, allow navigation
//! into subdirectories, and open files for viewing. `BrowsingApp` implements
//! the `App` trait with this shared logic.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::ContentState;
use super::app_trait::App;
use super::file_manager::{
    draw_content_windowed, hide_app_sdi, join_path, list_directory, parent_dir, render_app_chrome,
    render_content_sdi, view_audio_file, view_generic_file, view_image_file,
};
use super::runner::AppAction;

/// File-browsing app implementing the `App` trait.
///
/// Used for Music Player, Photo Viewer, and any app that browses a
/// directory tree and views files.
#[derive(Debug)]
pub struct BrowsingApp {
    pub content: ContentState,
    /// Which viewer to use when opening files.
    viewer_mode: ViewerMode,
}

/// How files should be viewed when opened.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum ViewerMode {
    Audio,
    Image,
    Generic,
}

impl BrowsingApp {
    /// Create a new browsing app starting at the given directory.
    fn new(
        title: &str,
        path: &str,
        start_dir: &str,
        not_found_msg: &str,
        viewer: ViewerMode,
        vfs: &dyn Vfs,
    ) -> Self {
        let mut content = ContentState::new(title, path);
        content.browse_dir = Some(start_dir.to_string());
        if vfs.exists(start_dir) {
            content.lines = list_directory(vfs, start_dir);
        } else {
            content.lines = vec![
                format!("({not_found_msg})"),
                String::new(),
                format!("Create {start_dir}/ and add files."),
            ];
        }
        Self {
            content,
            viewer_mode: viewer,
        }
    }

    /// Create the Music Player app.
    pub fn music_player(path: &str, vfs: &dyn Vfs) -> Self {
        Self::new(
            "Music Player",
            path,
            "/home/user/music",
            "Music directory not found",
            ViewerMode::Audio,
            vfs,
        )
    }

    /// Create the Photo Viewer app.
    pub fn photo_viewer(path: &str, vfs: &dyn Vfs) -> Self {
        Self::new(
            "Photo Viewer",
            path,
            "/home/user/photos",
            "Photos directory not found",
            ViewerMode::Image,
            vfs,
        )
    }

    /// Enter the selected directory or open the selected file.
    fn enter_selected(&mut self, vfs: &dyn Vfs) {
        let abs_idx = self.content.scroll + self.content.cursor;
        let Some(line) = self.content.lines.get(abs_idx) else {
            return;
        };
        let line = line.trim().to_string();
        let Some(ref dir) = self.content.browse_dir else {
            return;
        };

        if line == ".." {
            let parent = parent_dir(dir);
            self.content.browse_dir = Some(parent.clone());
            self.content.lines = list_directory(vfs, &parent);
            self.content.scroll = 0;
            self.content.cursor = 0;
        } else if line.ends_with('/') {
            let name = &line[..line.len() - 1];
            let new_dir = join_path(dir, name);
            self.content.browse_dir = Some(new_dir.clone());
            self.content.lines = list_directory(vfs, &new_dir);
            self.content.scroll = 0;
            self.content.cursor = 0;
        } else {
            // It's a file -- extract the filename (strip size suffix).
            let file_name = line.split("  (").next().unwrap_or(&line);
            let file_path = join_path(dir, file_name);
            self.open_file(vfs, &file_path);
        }
    }

    /// Open a file for viewing.
    fn open_file(&mut self, vfs: &dyn Vfs, path: &str) {
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

        self.content.lines = match self.viewer_mode {
            ViewerMode::Audio => view_audio_file(path, &data),
            ViewerMode::Image => view_image_file(path, &data),
            ViewerMode::Generic => view_generic_file(path, &data),
        };
    }
}

impl App for BrowsingApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => {
                if self.content.viewing_file.is_some() {
                    // Return from file viewer to directory listing.
                    self.content.viewing_file = None;
                    self.content.scroll = 0;
                    self.content.cursor = 0;
                    if let Some(ref dir) = self.content.browse_dir {
                        self.content.lines = list_directory(vfs, dir);
                    }
                    AppAction::None
                } else {
                    AppAction::Exit
                }
            },
            Button::Up => {
                self.content.navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.content.navigate_down();
                AppAction::None
            },
            Button::Confirm => {
                if self.content.browse_dir.is_some() && self.content.viewing_file.is_none() {
                    self.enter_selected(vfs);
                }
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
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
    ) -> crate::error::Result<()> {
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    fn setup_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home/user/music").unwrap();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home/user/photos").unwrap();
        crate::vfs::Vfs::write(&mut vfs, "/home/user/music/ambient_dawn.mp3", b"fake mp3").unwrap();
        crate::vfs::Vfs::write(
            &mut vfs,
            "/home/user/music/nightfall_theme.mp3",
            b"fake mp3 2",
        )
        .unwrap();
        crate::vfs::Vfs::write(&mut vfs, "/home/user/photos/sunset.png", b"fake png").unwrap();
        vfs
    }

    #[test]
    fn music_player_title_and_path() {
        let vfs = setup_vfs();
        let app = BrowsingApp::music_player("/apps/music", &vfs);
        assert_eq!(app.title(), "Music Player");
        assert_eq!(app.path(), "/apps/music");
    }

    #[test]
    fn music_player_lists_tracks() {
        let vfs = setup_vfs();
        let app = BrowsingApp::music_player("/apps/music", &vfs);
        assert!(app.browse_dir().is_some());
        assert!(app.lines().iter().any(|l| l.contains("ambient_dawn")));
        assert!(app.lines().iter().any(|l| l.contains("nightfall_theme")));
    }

    #[test]
    fn photo_viewer_lists_photos() {
        let vfs = setup_vfs();
        let app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        assert!(app.browse_dir().is_some());
        assert!(app.lines().iter().any(|l| l.contains("sunset.png")));
    }

    #[test]
    fn music_player_missing_dir() {
        let mut vfs = MemoryVfs::new();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
        let app = BrowsingApp::music_player("/apps/music", &vfs);
        assert!(
            app.lines()
                .iter()
                .any(|l| l.contains("Music directory not found"))
        );
    }

    #[test]
    fn cancel_exits_from_listing() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn cancel_returns_from_viewer() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        // Open a file.
        app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
        assert!(app.viewing_file().is_some());
        // Cancel returns to listing.
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn navigate_up_down() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.content.cached_max_visible = 20;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.cursor, 1);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.content.cursor, 0);
    }

    #[test]
    fn open_nonexistent_file_noop() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.open_file(&vfs, "/does/not/exist.mp3");
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn photo_viewer_open_file() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.content.cached_max_visible = 20;
        // Find sunset.png and open it.
        let idx = app
            .content
            .lines
            .iter()
            .position(|l| l.contains("sunset.png"))
            .unwrap();
        app.content.cursor = idx;
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.viewing_file().is_some());
    }

    #[test]
    fn downcast_works() {
        let vfs = setup_vfs();
        let app = BrowsingApp::music_player("/apps/music", &vfs);
        let any = app.as_any();
        assert!(any.downcast_ref::<BrowsingApp>().is_some());
    }
}
