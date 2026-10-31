//! File-browsing app for Music Player and Photo Viewer.
//!
//! Both apps share the same behavior: start in a directory, allow navigation
//! into subdirectories, and open files for viewing. `BrowsingApp` implements
//! the `App` trait with this shared logic.

use std::cell::{Cell, RefCell};

use oasis_app_core::file_viewer::{
    ListingEntry, ListingKind, join_path, list_directory_entries, parent_dir, view_audio_file,
    view_generic_file, view_image_file,
};
use oasis_app_core::render::draw_content_windowed;
use oasis_app_core::{App, AppAction, AppRequest, ContentState};
use oasis_skin::SimpleRng;
use oasis_types::backend::TextureId;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

use metadata::parse_audio_metadata;

pub mod image;
mod metadata;
mod music_ui;
pub mod palette;
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
    /// PRNG picking the next shuffled track. Also stepped once per
    /// `tick`, so the sequence depends on when the user skips.
    rng: SimpleRng,
    /// Playlist indices played before the current one in shuffle mode,
    /// so "previous" retraces the shuffled order.
    shuffle_history: Vec<usize>,
    /// Zoom level for photo viewer (1 = fit, 2 = 2x, etc.).
    zoom_level: u32,
    /// Image rotation in degrees (0, 90, 180, 270).
    rotation: u16,
    /// Whether slideshow mode is active.
    slideshow: bool,
    /// Milliseconds the current slide has been shown (slideshow mode).
    slideshow_timer: u32,
    /// Decoded pixel buffer for the currently viewed image (photo mode).
    /// Populated by `open_file`; the backend owns the GPU texture, keyed
    /// by the viewed file path.
    decoded_image: Option<image::DecodedImage>,
    /// Cached backend texture handle for the currently viewed image.
    /// Uploaded lazily in `photo_ui::draw` on first frame after `open_file`
    /// and reused across subsequent frames so we don't churn a GPU upload
    /// per redraw.
    pub(crate) cached_photo_texture: Cell<Option<TextureId>>,
    /// Textures from previous images that need to be destroyed on the
    /// next `photo_ui::draw` call. A queue — not a single slot — so
    /// that `BrowsingApp::inherit_textures_from` can hand off multiple
    /// pending textures when a runner is swapped out by
    /// `launch_app_window_for_file`.
    pub(crate) stale_photo_textures: RefCell<Vec<TextureId>>,
    /// Raw audio bytes for the currently viewed track (music mode).
    /// The app loads the bytes during `open_file` so the host audio
    /// controller can feed them to the backend without re-reading VFS.
    /// Metadata (title, duration) displayed in the UI is derived from
    /// the same bytes.
    track_title: Option<String>,
    track_duration_str: Option<String>,
    track_size_bytes: Option<usize>,
    /// Host-reported `(position_ms, duration_ms)` of the current track,
    /// read from [`MEDIA_POSITION_PATH`] in `tick`. `None` until the
    /// host reports a position for *this* file (the progress bar is
    /// hidden meanwhile).
    playback: Option<(u64, u64)>,
    /// Listing line hit by the previous click: clicking the same line
    /// again opens it (double-click without timing info, as in the File
    /// Manager).
    last_click_line: Option<usize>,
    /// The directory entry behind each listing row (`content.lines`), so
    /// a selected row is never parsed back out of its display text.
    entries: Vec<Option<ListingEntry>>,
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
        let mut entries = Vec::new();
        if vfs.exists(start_dir) {
            let listing = list_directory_entries(vfs, start_dir);
            content.lines = listing.lines;
            entries = listing.entries;
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
            rng: SimpleRng::new(0x05A1_5EED),
            shuffle_history: Vec::new(),
            zoom_level: 1,
            rotation: 0,
            slideshow: false,
            slideshow_timer: 0,
            decoded_image: None,
            cached_photo_texture: Cell::new(None),
            stale_photo_textures: RefCell::new(Vec::new()),
            track_title: None,
            track_duration_str: None,
            track_size_bytes: None,
            playback: None,
            last_click_line: None,
            entries,
        }
    }

    /// Host-reported `(position_ms, duration_ms)` for the open track, or
    /// `None` when no real position is known.
    pub fn playback(&self) -> Option<(u64, u64)> {
        self.playback
    }

    /// Record the host's playback position for the open track.
    ///
    /// A zero duration means "unknown" and clears the position. Hosts
    /// that hold the app directly can call this instead of the
    /// [`MEDIA_POSITION_PATH`] IPC file.
    pub fn set_playback(&mut self, position_ms: u64, duration_ms: u64) {
        self.playback = (duration_ms > 0).then_some((position_ms.min(duration_ms), duration_ms));
    }

    /// Poll [`MEDIA_POSITION_PATH`] (`<position_ms> <duration_ms> <path>`)
    /// and adopt it when it refers to the open track. A missing report
    /// leaves the position untouched (a host may use
    /// [`set_playback`](Self::set_playback) instead); a report for a
    /// different file clears it. Returns `true` when the displayed
    /// position changed.
    fn poll_playback(&mut self, vfs: &dyn Vfs) -> bool {
        let before = self.playback;
        let Some(viewing) = self.content.viewing_file.as_deref() else {
            self.playback = None;
            return self.playback != before;
        };
        let Ok(data) = vfs.read(MEDIA_POSITION_PATH) else {
            return false;
        };
        match parse_position_report(&String::from_utf8_lossy(&data), viewing) {
            Some((pos, dur)) => self.set_playback(pos, dur),
            None => self.playback = None,
        }
        self.playback != before
    }

    /// Decoded image for the currently viewed photo, if any.
    pub fn decoded_image(&self) -> Option<&image::DecodedImage> {
        self.decoded_image.as_ref()
    }

    /// Move any pending GPU textures from `other` into this app's
    /// stale queue so they get destroyed on the next render frame.
    /// Used by `launch_app_window_for_file` when it swaps in a new
    /// `BrowsingApp` for the same window slot — without this the old
    /// app's cached texture would leak in the backend's texture map.
    pub fn inherit_textures_from(&self, other: &BrowsingApp) {
        if let Some(t) = other.cached_photo_texture.take() {
            self.stale_photo_textures.borrow_mut().push(t);
        }
        let mut drained: Vec<TextureId> =
            other.stale_photo_textures.borrow_mut().drain(..).collect();
        self.stale_photo_textures.borrow_mut().append(&mut drained);
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

    /// List `dir` into the browser (cursor on the first row).
    fn show_dir(&mut self, vfs: &dyn Vfs, dir: &str) {
        let listing = list_directory_entries(vfs, dir);
        self.content.browse_dir = Some(dir.to_string());
        self.content.lines = listing.lines;
        self.entries = listing.entries;
        self.content.scroll = 0;
        self.content.cursor = 0;
    }

    /// The selected listing row's entry and the browse directory.
    fn selected_entry(&self) -> Option<(&ListingEntry, &str)> {
        let abs_idx = self.content.scroll + self.content.cursor;
        let entry = self.entries.get(abs_idx)?.as_ref()?;
        Some((entry, self.content.browse_dir.as_deref()?))
    }

    /// Enter the selected directory or open the selected file.
    fn enter_selected(&mut self, vfs: &dyn Vfs) {
        let Some((entry, dir)) = self.selected_entry() else {
            return;
        };
        let is_file = entry.kind == ListingKind::File;
        let target = match entry.kind {
            ListingKind::ParentDir => parent_dir(dir),
            _ => join_path(dir, &entry.name),
        };
        if is_file {
            self.open_file(vfs, &target);
        } else {
            self.show_dir(vfs, &target);
        }
    }

    /// Add the selected file to the playlist (music mode).
    fn add_to_playlist(&mut self, vfs: &dyn Vfs) {
        if !matches!(self.viewer_mode, ViewerMode::Audio) {
            return;
        }
        let Some((entry, dir)) = self.selected_entry() else {
            return;
        };
        if entry.kind != ListingKind::File {
            return;
        }
        let file_path = join_path(dir, &entry.name);
        if vfs.exists(&file_path) && !self.playlist.contains(&file_path) {
            self.playlist.push(file_path);
        }
    }

    /// Play the next track in the playlist (a random other track when
    /// shuffle is on).
    fn playlist_next(&mut self, vfs: &dyn Vfs) {
        let len = self.playlist.len();
        if len == 0 {
            return;
        }
        self.playlist_index = if self.shuffle && len > 1 {
            // Uniform over the other len-1 tracks: never repeats the
            // current one. High LCG bits; the low ones cycle quickly.
            let pick = (self.rng.next_u32() >> 16) as usize % (len - 1);
            let next = if pick >= self.playlist_index {
                pick + 1
            } else {
                pick
            };
            self.shuffle_history.push(self.playlist_index);
            next
        } else {
            (self.playlist_index + 1) % len
        };
        let path = self.playlist[self.playlist_index].clone();
        self.open_file(vfs, &path);
    }

    /// Play the previous track in the playlist (retracing the shuffled
    /// order when shuffle is on).
    fn playlist_prev(&mut self, vfs: &dyn Vfs) {
        if self.playlist.is_empty() {
            return;
        }
        if self.shuffle
            && let Some(prev) = self.shuffle_history.pop()
            && prev < self.playlist.len()
        {
            self.playlist_index = prev;
        } else if self.playlist_index == 0 {
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

    /// Rotate image 90 degrees clockwise: the decoded pixels are turned
    /// and the uploaded texture is retired, so the next frame shows it.
    fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
        if let Some(img) = self.decoded_image.as_mut() {
            *img = image::rotate_cw(img);
            if let Some(t) = self.cached_photo_texture.take() {
                self.stale_photo_textures.borrow_mut().push(t);
            }
        }
    }

    /// Navigate to next/previous image in slideshow or manual browsing.
    fn navigate_image(&mut self, vfs: &dyn Vfs, forward: bool) {
        if !matches!(self.viewer_mode, ViewerMode::Image) {
            return;
        }
        let Some(dir) = self.content.browse_dir.clone() else {
            return;
        };
        let image_exts = [".png", ".jpg", ".jpeg", ".bmp", ".gif"];
        // Files only, matched on the real name: `notes.png.txt` and a
        // `.jpgs/` folder are not images.
        let images: Vec<String> = list_directory_entries(vfs, &dir)
            .entries
            .into_iter()
            .flatten()
            .filter(|e| {
                let name = e.name.to_lowercase();
                e.kind == ListingKind::File && image_exts.iter().any(|ext| name.ends_with(ext))
            })
            .map(|e| e.name)
            .collect();
        if images.is_empty() {
            return;
        }
        let current_file = self.content.viewing_file.as_deref().unwrap_or("");
        let current_name = current_file.rsplit('/').next().unwrap_or("");
        let cur_idx = images
            .iter()
            .position(|name| name == current_name)
            .unwrap_or(0);
        let next_idx = if forward {
            (cur_idx + 1) % images.len()
        } else if cur_idx == 0 {
            images.len() - 1
        } else {
            cur_idx - 1
        };
        let next_path = join_path(&dir, &images[next_idx]);
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
        // The viewer's rows replace the listing (relisted on Cancel).
        self.entries.clear();
        // A track opened by hand becomes the playlist position, so
        // Next / Previous step from it.
        if let Some(i) = self.playlist.iter().position(|p| p == path) {
            self.playlist_index = i;
        }
        // Clear any previous view-specific state. A new image is decoded
        // upright, so its rotation starts over.
        self.decoded_image = None;
        self.rotation = 0;
        // Hand the previous texture to the render path so it can be
        // destroyed on the next frame; clear the active slot so the new
        // image is re-uploaded.
        if let Some(prev) = self.cached_photo_texture.take() {
            self.stale_photo_textures.borrow_mut().push(prev);
        }
        self.track_title = None;
        self.track_duration_str = None;
        self.track_size_bytes = None;
        self.playback = None;

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
                self.content
                    .post_request(MEDIA_REQUEST_PATH, format!("play_file {path}"));
            },
            ViewerMode::Generic => {},
        }
    }
}

/// VFS IPC path the Music Player sends `play_file <path>` / `stop`
/// requests to (the shared audio request file; the grammar is
/// [`oasis_types::ipc::MediaRequest`]).
pub const MEDIA_REQUEST_PATH: &str = oasis_types::ipc::AUDIO_REQUEST_PATH;

/// VFS path where the audio host reports the Music Player track's real
/// playback position, as `<position_ms> <duration_ms> <path>` (the
/// values of `AudioBackend::position_ms` / `duration_ms`). The path
/// field lets the app ignore a stale report for a previous track.
pub const MEDIA_POSITION_PATH: &str = oasis_types::ipc::AUDIO_POSITION_PATH;

/// Parse a [`MEDIA_POSITION_PATH`] report, returning
/// `(position_ms, duration_ms)` when it refers to `viewing` and the
/// duration is known (non-zero).
fn parse_position_report(report: &str, viewing: &str) -> Option<(u64, u64)> {
    let mut parts = report.trim().splitn(3, ' ');
    let pos = parts.next()?.parse::<u64>().ok()?;
    let dur = parts.next()?.parse::<u64>().ok()?;
    let path = parts.next()?;
    (path == viewing && dur > 0).then_some((pos, dur))
}

/// How long each photo stays on screen in slideshow mode.
pub const SLIDESHOW_INTERVAL_MS: u32 = 5_000;

impl App for BrowsingApp {
    fn title(&self) -> &str {
        &self.content.title
    }
    fn path(&self) -> &str {
        &self.content.app_path
    }
    fn layout(&mut self, _cw: u32, ch: u32, at: &oasis_skin::ActiveTheme) {
        // The file listing uses the shared line renderer (and its click
        // hit-testing); the viewers lay themselves out while drawing.
        if self.content.viewing_file.is_none() {
            self.content.update_layout_windowed(ch, at);
        }
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
    fn release_resources(&mut self, backend: &mut dyn oasis_types::backend::SdiBackend) {
        // The photo texture (and any parked stale ones) would otherwise
        // stay in the backend's texture table after the window closes.
        if let Some(t) = self.cached_photo_texture.take() {
            let _ = backend.destroy_texture(t);
        }
        for t in self.stale_photo_textures.get_mut().drain(..) {
            let _ = backend.destroy_texture(t);
        }
    }
    fn drain_requests(&mut self, out: &mut Vec<AppRequest>) {
        out.append(&mut self.content.requests);
    }
    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    /// Windowed listing: a click selects an entry, a second click on it
    /// (a double-click) opens it like Confirm.
    fn handle_click(&mut self, _lx: i32, ly: i32, vfs: &dyn Vfs) -> AppAction {
        if self.content.viewing_file.is_some() || self.content.browse_dir.is_none() {
            return AppAction::None;
        }
        let Some(idx) = self.content.line_at(ly) else {
            return AppAction::None;
        };
        if self.last_click_line == Some(idx) && idx == self.content.scroll + self.content.cursor {
            self.last_click_line = None;
            self.enter_selected(vfs);
            return AppAction::None;
        }
        self.last_click_line = Some(idx);
        if idx >= self.content.scroll {
            self.content.cursor = idx - self.content.scroll;
        } else {
            self.content.scroll = idx;
            self.content.cursor = 0;
        }
        AppAction::None
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => {
                if self.content.viewing_file.is_some() {
                    // Stop audio if we were playing a track.
                    if matches!(self.viewer_mode, ViewerMode::Audio) {
                        self.content.post_request(MEDIA_REQUEST_PATH, "stop");
                    }
                    let viewed = self.content.viewing_file.take().unwrap_or_default();
                    self.slideshow = false;
                    self.zoom_level = 1;
                    self.rotation = 0;
                    if let Some(dir) = self.content.browse_dir.clone() {
                        self.show_dir(vfs, &dir);
                        // Back on the row of the file just viewed (which a
                        // slideshow / Left / Right may have moved on).
                        let name = viewed.rsplit('/').next().unwrap_or("");
                        let row = self.entries.iter().position(|e| {
                            e.as_ref()
                                .is_some_and(|e| e.kind == ListingKind::File && e.name == name)
                        });
                        if let Some(row) = row {
                            self.content.select(row);
                        }
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
                self.shuffle_history.clear();
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

    fn tick(&mut self, dt_ms: u32, vfs: &dyn Vfs) -> bool {
        // Mix frame timing into the shuffle sequence.
        self.rng.next_u32();

        if matches!(self.viewer_mode, ViewerMode::Audio) {
            return self.poll_playback(vfs);
        }

        if !self.slideshow
            || !matches!(self.viewer_mode, ViewerMode::Image)
            || self.content.viewing_file.is_none()
        {
            return false;
        }
        self.slideshow_timer = self.slideshow_timer.saturating_add(dt_ms);
        if self.slideshow_timer < SLIDESHOW_INTERVAL_MS {
            return false;
        }
        // Restart the interval rather than carrying the remainder: after
        // a long stall this advances one slide, not a burst of them.
        self.slideshow_timer = 0;
        self.navigate_image(vfs, true);
        true
    }

    fn browse_dir(&self) -> Option<&str> {
        self.content.browse_dir.as_deref()
    }

    fn viewing_file(&self) -> Option<&str> {
        self.content.viewing_file.as_deref()
    }
}

#[cfg(test)]
mod tests;
