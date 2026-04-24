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
use oasis_types::input::Button;
use oasis_vfs::Vfs;

pub mod image;
mod music_ui;
mod photo_ui;

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
    /// Decoded pixel buffer for the currently viewed image (photo mode).
    /// Populated by `open_file`; the backend owns the GPU texture, keyed
    /// by the viewed file path.
    decoded_image: Option<image::DecodedImage>,
    /// Raw audio bytes for the currently viewed track (music mode).
    /// The app loads the bytes during `open_file` so the host audio
    /// controller can feed them to the backend without re-reading VFS.
    /// Metadata (title, duration) displayed in the UI is derived from
    /// the same bytes.
    track_title: Option<String>,
    track_duration_str: Option<String>,
    track_size_bytes: Option<usize>,
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
            decoded_image: None,
            track_title: None,
            track_duration_str: None,
            track_size_bytes: None,
        }
    }

    /// Decoded image for the currently viewed photo, if any.
    pub fn decoded_image(&self) -> Option<&image::DecodedImage> {
        self.decoded_image.as_ref()
    }

    /// Track display info for the currently open music track.
    pub fn track_info(&self) -> (Option<&str>, Option<&str>, Option<usize>) {
        (
            self.track_title.as_deref(),
            self.track_duration_str.as_deref(),
            self.track_size_bytes,
        )
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

    /// Create the Music Player app, pre-opening `file_path` so the viewer
    /// starts in track-view mode. The app's browse directory is set to the
    /// parent of the file so Cancel returns the user to a useful listing.
    pub fn music_player_at(path: &str, file_path: &str, vfs: &dyn Vfs) -> Self {
        let mut app = Self::new(
            "Music Player",
            path,
            parent_dir(file_path).as_str(),
            "Music directory not found",
            ViewerMode::Audio,
            vfs,
        );
        app.open_file(vfs, file_path);
        app
    }

    /// Create the Photo Viewer app, pre-opening `file_path`.
    pub fn photo_viewer_at(path: &str, file_path: &str, vfs: &dyn Vfs) -> Self {
        let mut app = Self::new(
            "Photo Viewer",
            path,
            parent_dir(file_path).as_str(),
            "Photos directory not found",
            ViewerMode::Image,
            vfs,
        );
        app.open_file(vfs, file_path);
        app
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

    /// Whether shuffle mode is on (music mode).
    pub fn shuffle(&self) -> bool {
        self.shuffle
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
        // Clear any previous view-specific state.
        self.decoded_image = None;
        self.track_title = None;
        self.track_duration_str = None;
        self.track_size_bytes = None;

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

        match self.viewer_mode {
            ViewerMode::Image => {
                self.decoded_image = image::decode(&data);
                if self.decoded_image.is_none() {
                    log::warn!(
                        "photo viewer: unsupported or unreadable image at {path} ({} bytes)",
                        data.len(),
                    );
                }
            },
            ViewerMode::Audio => {
                let (title, duration) = parse_audio_metadata(path, &data);
                self.track_title = Some(title);
                self.track_duration_str = duration;
                self.track_size_bytes = Some(data.len());
                // Ask the host audio subsystem to load + play the
                // track. Consumed by `oasis-app::media_controller`.
                self.content.pending_vfs_request =
                    Some((MEDIA_REQUEST_PATH.to_string(), format!("play_file {path}")));
            },
            ViewerMode::Generic => {},
        }
    }
}

/// Extract a display title and duration string from an audio file.
/// Falls back to the filename stem if ID3 tags are not available.
fn parse_audio_metadata(path: &str, data: &[u8]) -> (String, Option<String>) {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let fallback_title = filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .replace(['_', '-'], " ");
    let title = id3v2_title(data).unwrap_or(fallback_title);
    let duration = mp3_duration(data).map(format_duration);
    (title, duration)
}

/// Small ID3v2 TIT2 (title) reader. Returns `None` if not present —
/// just enough to show a nice track title when one is available.
fn id3v2_title(data: &[u8]) -> Option<String> {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return None;
    }
    let size = ((data[6] as usize) << 21)
        | ((data[7] as usize) << 14)
        | ((data[8] as usize) << 7)
        | (data[9] as usize);
    let tag_end = 10 + size;
    if tag_end > data.len() {
        return None;
    }
    let tag = &data[10..tag_end];
    let mut i = 0;
    while i + 10 <= tag.len() {
        let frame_id = &tag[i..i + 4];
        if frame_id[0] == 0 {
            break;
        }
        let frame_size =
            u32::from_be_bytes([tag[i + 4], tag[i + 5], tag[i + 6], tag[i + 7]]) as usize;
        let body_start = i + 10;
        let body_end = body_start + frame_size;
        if body_end > tag.len() {
            break;
        }
        if frame_id == b"TIT2" && frame_size >= 1 {
            let encoding = tag[body_start];
            let payload = &tag[body_start + 1..body_end];
            if encoding == 0 || encoding == 3 {
                let s = String::from_utf8_lossy(payload).into_owned();
                let trimmed = s.trim_end_matches('\0').trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        i = body_end;
    }
    None
}

/// Estimate MP3 duration from first-frame bitrate + file size. Close
/// enough for a UI readout; we're not a metadata library.
fn mp3_duration(data: &[u8]) -> Option<u32> {
    let audio_start = if data.len() >= 10 && &data[..3] == b"ID3" {
        let size = ((data[6] as usize) << 21)
            | ((data[7] as usize) << 14)
            | ((data[8] as usize) << 7)
            | (data[9] as usize);
        10 + size
    } else {
        0
    };
    if audio_start >= data.len() {
        return None;
    }
    let audio = &data[audio_start..];
    let header_pos = audio
        .windows(2)
        .position(|w| w[0] == 0xFF && (w[1] & 0xE0) == 0xE0)?;
    let header = audio.get(header_pos..header_pos + 4)?;
    let bitrate_index = (header[2] >> 4) & 0x0F;
    const BITRATES: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    let bitrate_kbps = BITRATES[bitrate_index as usize];
    if bitrate_kbps == 0 {
        return None;
    }
    let bits = (audio.len() as u64) * 8;
    Some((bits / (bitrate_kbps as u64 * 1000)) as u32)
}

fn format_duration(secs: u32) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m:02}:{s:02}")
}

/// VFS IPC path used by the Music Player to talk to the audio host.
/// The host writes back status here (track loaded, errors) and reads
/// `play_file <path>` / `stop` requests from the same path.
pub const MEDIA_REQUEST_PATH: &str = "/var/audio/request";

impl App for BrowsingApp {
    fn title(&self) -> &str {
        &self.content.title
    }
    fn path(&self) -> &str {
        &self.content.app_path
    }
    fn update_sdi(&mut self, sdi: &mut oasis_sdi::SdiRegistry, at: &oasis_skin::ActiveTheme) {
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
        backend: &mut dyn oasis_types::backend::SdiBackend,
        at: &oasis_skin::ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        // File listing: default content renderer.
        if self.content.viewing_file.is_none() {
            return draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at);
        }
        match self.viewer_mode {
            ViewerMode::Image => photo_ui::draw(self, cx, cy, cw, ch, backend, at),
            ViewerMode::Audio => music_ui::draw(self, cx, cy, cw, ch, backend, at),
            ViewerMode::Generic => {
                draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
            },
        }
    }
    fn hide_sdi(&self, sdi: &mut oasis_sdi::SdiRegistry) {
        hide_app_sdi(sdi);
    }
    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.content.pending_vfs_request.take()
    }
    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.content.pending_vfs_request.as_ref()
    }
    fn lines(&self) -> &[String] {
        &self.content.lines
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => {
                if self.content.viewing_file.is_some() {
                    // Stop audio if we were playing a track.
                    if matches!(self.viewer_mode, ViewerMode::Audio) {
                        self.content.pending_vfs_request =
                            Some((MEDIA_REQUEST_PATH.to_string(), "stop".to_string()));
                    }
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

    fn browse_dir(&self) -> Option<&str> {
        self.content.browse_dir.as_deref()
    }

    fn viewing_file(&self) -> Option<&str> {
        self.content.viewing_file.as_deref()
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
    fn music_open_emits_play_ipc() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
        let req = app.content.pending_vfs_request.clone().expect("ipc");
        assert_eq!(req.0, MEDIA_REQUEST_PATH);
        assert_eq!(req.1, "play_file /home/user/music/ambient_dawn.mp3");
    }

    #[test]
    fn music_cancel_emits_stop_ipc() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::music_player("/apps/music", &vfs);
        app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
        // Drain the play IPC.
        let _ = app.content.pending_vfs_request.take();
        app.handle_input(&Button::Cancel, &vfs);
        let req = app.content.pending_vfs_request.clone().expect("ipc");
        assert_eq!(req.0, MEDIA_REQUEST_PATH);
        assert_eq!(req.1, "stop");
    }

    #[test]
    fn photo_open_does_not_emit_media_ipc() {
        let vfs = setup_vfs();
        let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
        app.open_file(&vfs, "/home/user/photos/sunset.png");
        assert!(
            app.content.pending_vfs_request.is_none(),
            "photo viewer must not emit media IPC"
        );
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
