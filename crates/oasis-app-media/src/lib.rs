//! File-browsing app for Music Player and Photo Viewer.
//!
//! Both apps share the same behavior: start in a directory, allow navigation
//! into subdirectories, and open files for viewing. `BrowsingApp` implements
//! the `App` trait with this shared logic.

use oasis_app_core::file_viewer::{
    join_path, list_directory, parent_dir, view_audio_file, view_generic_file, view_image_file,
};
use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// File-browsing app implementing the `App` trait.
///
/// Used for Music Player, Photo Viewer, and any app that browses a
/// directory tree and views files.
#[derive(Debug)]
pub struct BrowsingApp {
    pub content: ContentState,
    /// Which viewer to use when opening files.
    viewer_mode: ViewerMode,
    /// Playlist queue (music mode): list of file paths.
    playlist: Vec<String>,
    /// Current index in the playlist.
    playlist_index: usize,
    /// Whether playlist plays in shuffle order.
    shuffle: bool,
    /// Zoom level for photo viewer (1 = fit, 2 = 2x, etc.).
    zoom_level: u32,
    /// Image rotation in degrees (0, 90, 180, 270).
    rotation: u16,
    /// Whether slideshow mode is active.
    slideshow: bool,
    /// Frame counter for slideshow timing.
    slideshow_timer: u32,
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
            playlist: Vec::new(),
            playlist_index: 0,
            shuffle: false,
            zoom_level: 1,
            rotation: 0,
            slideshow: false,
            slideshow_timer: 0,
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

    /// Add the selected file to the playlist (music mode).
    fn add_to_playlist(&mut self, vfs: &dyn Vfs) {
        if !matches!(self.viewer_mode, ViewerMode::Audio) {
            return;
        }
        let abs_idx = self.content.scroll + self.content.cursor;
        let Some(line) = self.content.lines.get(abs_idx) else {
            return;
        };
        let line = line.trim().to_string();
        if line == ".." || line.ends_with('/') {
            return;
        }
        let Some(ref dir) = self.content.browse_dir else {
            return;
        };
        let file_name = line.split("  (").next().unwrap_or(&line);
        let file_path = join_path(dir, file_name);
        if vfs.exists(&file_path) && !self.playlist.contains(&file_path) {
            self.playlist.push(file_path);
        }
    }

    /// Play the next track in the playlist.
    fn playlist_next(&mut self, vfs: &dyn Vfs) {
        if self.playlist.is_empty() {
            return;
        }
        self.playlist_index = (self.playlist_index + 1) % self.playlist.len();
        let path = self.playlist[self.playlist_index].clone();
        self.open_file(vfs, &path);
    }

    /// Play the previous track in the playlist.
    fn playlist_prev(&mut self, vfs: &dyn Vfs) {
        if self.playlist.is_empty() {
            return;
        }
        if self.playlist_index == 0 {
            self.playlist_index = self.playlist.len() - 1;
        } else {
            self.playlist_index -= 1;
        }
        let path = self.playlist[self.playlist_index].clone();
        self.open_file(vfs, &path);
    }

    /// Get the current playlist.
    pub fn playlist(&self) -> &[String] {
        &self.playlist
    }

    /// Get the current zoom level (photo viewer).
    pub fn zoom_level(&self) -> u32 {
        self.zoom_level
    }

    /// Get the current rotation in degrees (photo viewer).
    pub fn rotation(&self) -> u16 {
        self.rotation
    }

    /// Whether slideshow is active (photo viewer).
    pub fn slideshow_active(&self) -> bool {
        self.slideshow
    }

    /// Cycle zoom level: 1 -> 2 -> 4 -> 1.
    fn cycle_zoom(&mut self) {
        self.zoom_level = match self.zoom_level {
            1 => 2,
            2 => 4,
            _ => 1,
        };
    }

    /// Rotate image 90 degrees clockwise.
    fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
    }

    /// Navigate to next/previous image in slideshow or manual browsing.
    fn navigate_image(&mut self, vfs: &dyn Vfs, forward: bool) {
        if !matches!(self.viewer_mode, ViewerMode::Image) {
            return;
        }
        let Some(dir) = self.content.browse_dir.clone() else {
            return;
        };
        let files = list_directory(vfs, &dir);
        let image_exts = [".png", ".jpg", ".jpeg", ".bmp", ".gif"];
        let images: Vec<String> = files
            .iter()
            .filter(|f| {
                let lower = f.to_lowercase();
                image_exts.iter().any(|ext| lower.contains(ext)) && !f.ends_with('/')
            })
            .cloned()
            .collect();
        if images.is_empty() {
            return;
        }
        let current_file = self.content.viewing_file.as_deref().unwrap_or("");
        let current_name = current_file.rsplit('/').next().unwrap_or("");
        let cur_idx = images
            .iter()
            .position(|f| f.split("  (").next().unwrap_or(f) == current_name)
            .unwrap_or(0);
        let next_idx = if forward {
            (cur_idx + 1) % images.len()
        } else if cur_idx == 0 {
            images.len() - 1
        } else {
            cur_idx - 1
        };
        let next_name = images[next_idx]
            .split("  (")
            .next()
            .unwrap_or(&images[next_idx]);
        let next_path = join_path(&dir, next_name);
        self.open_file(vfs, &next_path);
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
                    self.content.viewing_file = None;
                    self.content.scroll = 0;
                    self.content.cursor = 0;
                    self.slideshow = false;
                    self.zoom_level = 1;
                    self.rotation = 0;
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
            // Music mode: Triangle adds to playlist from listing.
            Button::Triangle if matches!(self.viewer_mode, ViewerMode::Audio) => {
                if self.content.viewing_file.is_none() {
                    self.add_to_playlist(vfs);
                }
                AppAction::None
            },
            // Music mode: L/R for prev/next track in playlist.
            Button::Left
                if matches!(self.viewer_mode, ViewerMode::Audio)
                    && self.content.viewing_file.is_some() =>
            {
                self.playlist_prev(vfs);
                AppAction::None
            },
            Button::Right
                if matches!(self.viewer_mode, ViewerMode::Audio)
                    && self.content.viewing_file.is_some() =>
            {
                self.playlist_next(vfs);
                AppAction::None
            },
            // Music mode: Select toggles shuffle.
            Button::Select if matches!(self.viewer_mode, ViewerMode::Audio) => {
                self.shuffle = !self.shuffle;
                AppAction::None
            },
            // Photo mode: Square rotates, L/R cycle zoom, Start toggles slideshow.
            Button::Square if matches!(self.viewer_mode, ViewerMode::Image) => {
                if self.content.viewing_file.is_some() {
                    self.rotate_cw();
                }
                AppAction::None
            },
            Button::Left
                if matches!(self.viewer_mode, ViewerMode::Image)
                    && self.content.viewing_file.is_some() =>
            {
                self.navigate_image(vfs, false);
                AppAction::None
            },
            Button::Right
                if matches!(self.viewer_mode, ViewerMode::Image)
                    && self.content.viewing_file.is_some() =>
            {
                self.navigate_image(vfs, true);
                AppAction::None
            },
            Button::Triangle
                if matches!(self.viewer_mode, ViewerMode::Image)
                    && self.content.viewing_file.is_some() =>
            {
                self.cycle_zoom();
                AppAction::None
            },
            Button::Start if matches!(self.viewer_mode, ViewerMode::Image) => {
                if self.content.viewing_file.is_some() {
                    self.slideshow = !self.slideshow;
                    self.slideshow_timer = 0;
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
    ) -> oasis_types::error::Result<()> {
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
    use oasis_vfs::MemoryVfs;

    fn setup_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user/music").unwrap();
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user/photos").unwrap();
        oasis_vfs::Vfs::write(&mut vfs, "/home/user/music/ambient_dawn.mp3", b"fake mp3").unwrap();
        oasis_vfs::Vfs::write(
            &mut vfs,
            "/home/user/music/nightfall_theme.mp3",
            b"fake mp3 2",
        )
        .unwrap();
        oasis_vfs::Vfs::write(&mut vfs, "/home/user/photos/sunset.png", b"fake png").unwrap();
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
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
        oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
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

    #[test]
    fn music_add_to_playlist() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.content.cached_max_visible = 20;
        // Navigate to a track and add it.
        let idx = app
            .content
            .lines
            .iter()
            .position(|l| l.contains("ambient_dawn"))
            .unwrap();
        app.content.cursor = idx;
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.playlist().len(), 1);
        assert!(app.playlist()[0].contains("ambient_dawn"));
    }

    #[test]
    fn music_playlist_no_duplicates() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.content.cached_max_visible = 20;
        let idx = app
            .content
            .lines
            .iter()
            .position(|l| l.contains("ambient_dawn"))
            .unwrap();
        app.content.cursor = idx;
        app.handle_input(&Button::Triangle, &vfs);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.playlist().len(), 1);
    }

    #[test]
    fn music_shuffle_toggle() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        assert!(!app.shuffle);
        app.handle_input(&Button::Select, &vfs);
        assert!(app.shuffle);
        app.handle_input(&Button::Select, &vfs);
        assert!(!app.shuffle);
    }

    #[test]
    fn photo_rotate() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.open_file(&vfs, "/home/user/photos/sunset.png");
        assert_eq!(app.rotation(), 0);
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.rotation(), 90);
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.rotation(), 180);
    }

    #[test]
    fn photo_zoom_cycle() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.open_file(&vfs, "/home/user/photos/sunset.png");
        assert_eq!(app.zoom_level(), 1);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.zoom_level(), 2);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.zoom_level(), 4);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.zoom_level(), 1);
    }

    #[test]
    fn photo_slideshow_toggle() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.open_file(&vfs, "/home/user/photos/sunset.png");
        assert!(!app.slideshow_active());
        app.handle_input(&Button::Start, &vfs);
        assert!(app.slideshow_active());
        app.handle_input(&Button::Start, &vfs);
        assert!(!app.slideshow_active());
    }

    #[test]
    fn photo_cancel_resets_state() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.open_file(&vfs, "/home/user/photos/sunset.png");
        app.handle_input(&Button::Square, &vfs); // rotate
        app.handle_input(&Button::Triangle, &vfs); // zoom
        app.handle_input(&Button::Cancel, &vfs); // back to listing
        assert_eq!(app.rotation(), 0);
        assert_eq!(app.zoom_level(), 1);
        assert!(!app.slideshow_active());
    }
}
