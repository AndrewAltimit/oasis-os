//! App screen runner with title bar and scrollable content.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::dashboard::AppEntry;
use crate::input::Button;
use crate::vfs::Vfs;

use super::file_viewer::list_directory;
use super::tv_guide::guide::TvGuideState;

/// Maximum lines visible in the app content area (fallback for 480x272).
const MAX_VISIBLE_LINES: usize = 13;

/// Action returned by the app after handling input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// App consumed the input, no mode change needed.
    None,
    /// User wants to exit this app and return to dashboard.
    Exit,
    /// App wants to switch to terminal mode.
    SwitchToTerminal,
    /// App requests entering fullscreen kiosk mode.
    RequestFullscreen,
}

/// Runtime state for a launched application screen.
///
/// Apps that have been extracted to the `App` trait (e.g. File Manager)
/// are stored in the `delegate` field. Legacy apps still use the
/// inline fields. This allows incremental migration.
#[derive(Debug)]
pub struct AppRunner {
    /// App display title.
    pub title: String,
    /// App path in VFS.
    pub path: String,
    /// Content lines displayed in the app area.
    pub lines: Vec<String>,
    /// Scroll offset (first visible line index).
    pub scroll: usize,
    /// Current directory for file-manager navigation.
    pub browse_dir: Option<String>,
    /// Path of the file currently being viewed (file viewer mode).
    pub viewing_file: Option<String>,
    /// Selected line index (relative to visible area).
    pub cursor: usize,
    /// Pending VFS IPC request from radio app (path, data).
    pub(crate) pending_vfs_request: Option<(String, String)>,
    /// Smooth selection position (lerps toward cursor index).
    pub(crate) visual_selected: f32,
    /// Cached max visible lines (updated each frame by `update_sdi`).
    pub(crate) cached_max_visible: usize,
    /// TV Guide state (only for "TV Guide" app).
    pub(crate) tv_guide: Option<TvGuideState>,
    /// Extracted app implementation (Some for migrated apps).
    pub(crate) delegate: Option<Box<dyn super::app_trait::App>>,
}

impl AppRunner {
    /// Launch an app from its dashboard entry.
    pub fn launch(app: &AppEntry, vfs: &dyn Vfs) -> Self {
        let title = app.title.clone();
        let path = app.path.clone();

        // Try to create a delegate for extracted App trait implementations.
        let delegate: Option<Box<dyn super::app_trait::App>> = match title.as_str() {
            "File Manager" => Some(Box::new(super::file_manager::FileManagerApp::new(
                &path, vfs,
            ))),
            "Settings" => Some(Box::new(super::simple_app::SimpleApp::settings(
                &path, "Classic", 480, 272,
            ))),
            "Network" => Some(Box::new(super::simple_app::SimpleApp::network(
                &path, false, 9000, false,
            ))),
            "Package Manager" => Some(Box::new(super::simple_app::SimpleApp::package_manager(
                &path,
            ))),
            "Browser" => Some(Box::new(super::simple_app::SimpleApp::browser(&path))),
            "System Monitor" => Some(Box::new(super::simple_app::SimpleApp::system_monitor(
                &path,
                "Desktop (SDL2)",
                "SDL2",
                0,
            ))),
            "Terminal" => Some(Box::new(super::simple_app::SimpleApp::terminal(&path))),
            "Music Player" => Some(Box::new(super::browsing_app::BrowsingApp::music_player(
                &path, vfs,
            ))),
            "Photo Viewer" => Some(Box::new(super::browsing_app::BrowsingApp::photo_viewer(
                &path, vfs,
            ))),
            "Text Editor" => Some(Box::new(super::text_editor::TextEditorApp::new(&path))),
            "Calculator" => Some(Box::new(super::calculator::CalculatorApp::new(&path))),
            "Clock" => Some(Box::new(super::clock::ClockApp::new(&path))),
            "Paint" => Some(Box::new(super::paint::PaintApp::new(&path))),
            "Games" => Some(Box::new(super::games::GamesApp::new(&path))),
            // Internet Radio and TV Guide have special rendering in AppRunner.
            "Internet Radio" | "TV Guide" => None,
            // All other apps get a generic placeholder.
            _ => Some(Box::new(super::simple_app::SimpleApp::new(
                &title,
                &path,
                vec![
                    title.clone(),
                    String::new(),
                    "(No content available for this app)".to_string(),
                ],
            ))),
        };

        if let Some(app_impl) = delegate {
            return Self {
                title: app_impl.title().to_string(),
                path: app_impl.path().to_string(),
                lines: app_impl.lines().to_vec(),
                scroll: 0,
                browse_dir: app_impl.browse_dir().map(String::from),
                viewing_file: None,
                cursor: 0,
                pending_vfs_request: None,
                visual_selected: 0.0,
                cached_max_visible: MAX_VISIBLE_LINES,
                tv_guide: None,
                delegate: Some(app_impl),
            };
        }

        let mut runner = Self {
            title: title.clone(),
            path,
            lines: Vec::new(),
            scroll: 0,
            browse_dir: None,
            viewing_file: None,
            cursor: 0,
            pending_vfs_request: None,
            visual_selected: 0.0,
            cached_max_visible: MAX_VISIBLE_LINES,
            tv_guide: None,
            delegate: None,
        };
        runner.init_content(&title, vfs);
        runner
    }

    /// Generate initial content based on the app title.
    ///
    /// Generate initial content for non-delegate apps.
    ///
    /// Only Internet Radio and TV Guide remain here; all other apps
    /// are handled via the `delegate` field.
    fn init_content(&mut self, title: &str, vfs: &dyn Vfs) {
        match title {
            "Internet Radio" => {
                self.lines = Self::radio_content(vfs);
                self.cursor = 0;
            },
            "TV Guide" => {
                self.init_tv_guide(vfs, &ActiveTheme::default());
            },
            _ => {},
        }
    }

    /// Handle input while the app is active.
    pub fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        // Delegate to extracted app if present.
        if let Some(ref mut app) = self.delegate {
            let action = app.handle_input(button, vfs);
            self.sync_from_delegate();
            return action;
        }

        // Internet Radio mode.
        if self.title == "Internet Radio" {
            return self.handle_radio_input(button, vfs);
        }

        // TV Guide mode.
        if self.title == "TV Guide" {
            return self.handle_tv_guide_input(button);
        }

        match button {
            Button::Cancel => {
                // If viewing a file, go back to directory listing.
                if self.viewing_file.is_some() {
                    self.viewing_file = None;
                    self.scroll = 0;
                    self.cursor = 0;
                    // Refresh directory listing.
                    if let Some(ref dir) = self.browse_dir {
                        self.lines = list_directory(vfs, dir);
                    }
                    return AppAction::None;
                }
                AppAction::Exit
            },
            Button::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                } else if self.scroll > 0 {
                    self.scroll -= 1;
                }
                AppAction::None
            },
            Button::Down => {
                let visible = self.visible_count();
                if self.cursor + 1 < visible {
                    self.cursor += 1;
                } else if self.scroll + self.cached_max_visible < self.lines.len() {
                    self.scroll += 1;
                }
                AppAction::None
            },
            Button::Confirm => AppAction::None,
            _ => AppAction::None,
        }
    }

    /// Render app content directly into a windowed content area.
    ///
    /// Unlike `update_sdi()` which creates named SDI objects for full-screen
    /// display, this method draws directly into the clip region provided by the
    /// window manager's `draw_with_clips` callback.
    pub fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        // Delegate to extracted app.
        if let Some(ref app) = self.delegate {
            return app.draw_windowed(cx, cy, cw, ch, backend, at);
        }

        // TV Guide gets its own EPG grid renderer.
        if let Some(ref guide) = self.tv_guide {
            return guide.draw_windowed(cx, cy, cw, ch, backend, at);
        }

        // Content background.
        backend.fill_rect(cx, cy, cw, ch, at.app.bg)?;

        // Title row with dir/file suffix.
        let dir_suffix = if let Some(ref file) = self.viewing_file {
            format!("  [{file}]")
        } else {
            self.browse_dir
                .as_deref()
                .map(|d| format!("  [{d}]"))
                .unwrap_or_default()
        };
        let title_text = format!("{}{dir_suffix}", self.title);
        backend.draw_text(&title_text, cx + 4, cy + 2, 12, at.app.title_bar_text)?;

        // Separator line.
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            at.app.divider,
        )?;

        // Content lines.
        let line_h = at.terminal_line_height.max(12) as i32;
        let max_lines = ((ch as i32 - line_h - 4) / line_h).max(0) as usize;
        let visible = self.lines.len().saturating_sub(self.scroll).min(max_lines);
        for i in 0..visible {
            let line_idx = self.scroll + i;
            let line = &self.lines[line_idx];
            let prefix = if i == self.cursor { "> " } else { "  " };
            let text = format!("{prefix}{line}");
            let text_color = if i == self.cursor {
                at.app.selected_text
            } else {
                at.app.text
            };
            let y = cy + at.app.title_bar_height as i32 + i as i32 * line_h;
            backend.draw_text(&text, cx + 4, y, 12, text_color)?;
        }

        // Scroll indicator at bottom-left.
        let scroll_text = if self.lines.len() > max_lines {
            format!(
                "[{}/{}]  Cancel=back",
                self.scroll + 1,
                self.lines.len().saturating_sub(max_lines) + 1,
            )
        } else {
            "Cancel=back".to_string()
        };
        let scroll_y = cy + ch as i32 - 14;
        backend.draw_text(&scroll_text, cx + 4, scroll_y, 10, at.app.dim_text)?;

        Ok(())
    }

    /// Number of currently visible content lines.
    fn visible_count(&self) -> usize {
        let remaining = self.lines.len().saturating_sub(self.scroll);
        remaining.min(self.cached_max_visible)
    }

    // ---------------------------------------------------------------
    // Internet Radio helpers
    // ---------------------------------------------------------------

    /// Generate content lines for the Internet Radio app.
    fn radio_content(vfs: &dyn Vfs) -> Vec<String> {
        use oasis_audio::radio::station::StationRegistry;
        use oasis_audio::{RADIO_REQUEST_PATH, RADIO_STATUS_PATH};

        let mut lines = Vec::new();
        lines.push("=== Internet Radio ===".to_string());
        lines.push(String::new());

        // Read status from VFS if available.
        let (state, station, now_playing) = if vfs.exists(RADIO_STATUS_PATH) {
            let data = vfs.read(RADIO_STATUS_PATH).unwrap_or_default();
            let text = String::from_utf8_lossy(&data);
            let mut st = "Stopped".to_string();
            let mut stn = "--".to_string();
            let mut np = "--".to_string();
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("State: ") {
                    st = v.to_string();
                } else if let Some(v) = line.strip_prefix("Station: ") {
                    stn = v.to_string();
                } else if let Some(v) = line.strip_prefix("Now Playing: ") {
                    np = v.to_string();
                }
            }
            (st, stn, np)
        } else {
            ("Stopped".to_string(), "--".to_string(), "--".to_string())
        };

        lines.push(format!("Status: {state}"));
        lines.push(format!("Station: {station}"));
        lines.push(format!("Now Playing: {now_playing}"));
        lines.push(String::new());
        lines.push("--- Stations ---".to_string());

        // Load stations from VFS.
        let registry = if vfs.exists("/etc/radio/stations.toml") {
            let data = vfs.read("/etc/radio/stations.toml").unwrap_or_default();
            let text = String::from_utf8_lossy(&data);
            StationRegistry::from_toml(&text).unwrap_or_else(|_| StationRegistry::defaults())
        } else {
            StationRegistry::defaults()
        };

        // Check for pending request (to avoid re-sending).
        let _ = RADIO_REQUEST_PATH;

        for (i, s) in registry.stations.iter().enumerate() {
            let fav = if s.favorite { "*" } else { " " };
            let source_info = if s.source_type == "icecast" {
                if s.bitrate > 0 {
                    format!("{}k", s.bitrate)
                } else {
                    "?".to_string()
                }
            } else if !s.collection.is_empty() {
                s.collection.clone()
            } else {
                "archive".to_string()
            };
            lines.push(format!(
                "  [{fav}] {:<26} {:<12} {source_info}",
                s.name, s.genre
            ));
            // Store index as hidden data (used by input handler).
            let _ = i;
        }

        lines.push(String::new());
        lines.push("Confirm=Tune  Triangle=Fav  Cancel=Exit".to_string());

        lines
    }

    /// Handle input for the Internet Radio app.
    fn handle_radio_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        use oasis_audio::RADIO_REQUEST_PATH;

        // The station list starts at line 7 (after header + status lines).
        let station_header_lines = 7;
        let station_count = self.lines.len().saturating_sub(station_header_lines + 2);
        // 2 = blank line + help line at bottom.

        match button {
            Button::Cancel => AppAction::Exit,
            Button::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                } else if self.scroll > 0 {
                    self.scroll -= 1;
                }
                AppAction::None
            },
            Button::Down => {
                let visible = self.visible_count();
                if self.cursor + 1 < visible {
                    self.cursor += 1;
                } else if self.scroll + self.cached_max_visible < self.lines.len() {
                    self.scroll += 1;
                }
                AppAction::None
            },
            Button::Confirm => {
                // Determine which station is selected.
                let abs_idx = self.scroll + self.cursor;
                if abs_idx >= station_header_lines && abs_idx < station_header_lines + station_count
                {
                    let station_idx = abs_idx - station_header_lines;
                    self.pending_vfs_request = Some((
                        RADIO_REQUEST_PATH.to_string(),
                        format!("tune {station_idx}"),
                    ));
                }
                AppAction::None
            },
            Button::Triangle => {
                // Triangle = toggle favorite.
                let abs_idx = self.scroll + self.cursor;
                if abs_idx >= station_header_lines && abs_idx < station_header_lines + station_count
                {
                    let station_idx = abs_idx - station_header_lines;
                    self.pending_vfs_request =
                        Some((RADIO_REQUEST_PATH.to_string(), format!("fav {station_idx}")));
                }
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Peek at a pending VFS IPC request without consuming it.
    pub fn peek_pending_request(&self) -> Option<&(String, String)> {
        if let Some(ref app) = self.delegate {
            return app.peek_pending_request();
        }
        self.pending_vfs_request.as_ref()
    }

    /// Take any pending VFS IPC request (returns path and data if present).
    pub fn take_pending_request(&mut self) -> Option<(String, String)> {
        if let Some(ref mut app) = self.delegate {
            return app.take_pending_request();
        }
        self.pending_vfs_request.take()
    }

    /// Refresh radio display from VFS status (called each frame when visible).
    pub fn refresh_radio(&mut self, vfs: &dyn Vfs) {
        if self.title != "Internet Radio" {
            return;
        }
        let old_cursor = self.cursor;
        let old_scroll = self.scroll;
        self.lines = Self::radio_content(vfs);
        self.cursor = old_cursor;
        self.scroll = old_scroll;
    }

    /// Refresh TV Guide text display after catalog changes.
    pub fn refresh_tv_text(&mut self) {
        if let Some(ref guide) = self.tv_guide {
            let old_cursor = self.cursor;
            let old_scroll = self.scroll;
            self.lines = guide.text_content();
            log::debug!("TV: refresh_tv_text -> {} lines", self.lines.len());
            self.cursor = old_cursor;
            self.scroll = old_scroll;
        }
    }

    // ---------------------------------------------------------------
    // TV Guide helpers
    // ---------------------------------------------------------------

    /// Initialize the TV Guide app from VFS channel config.
    fn init_tv_guide(&mut self, vfs: &dyn Vfs, at: &ActiveTheme) {
        use super::tv_guide::TV_CHANNELS_PATH;
        use super::tv_guide::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};

        let config = if vfs.exists(TV_CHANNELS_PATH) {
            log::debug!("TV: loading channel config from VFS");
            let data = vfs.read(TV_CHANNELS_PATH).unwrap_or_default();
            let text = String::from_utf8_lossy(&data);
            ChannelConfig::from_toml(&text).unwrap_or_else(|_| {
                ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML)
                    .expect("default channels TOML is valid")
            })
        } else {
            log::debug!("TV: using default channel config");
            ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).expect("default channels TOML is valid")
        };

        log::debug!("TV: init_tv_guide with {} channels", config.channel.len());
        let guide = TvGuideState::new(&config, at);
        self.lines = guide.text_content();
        self.tv_guide = Some(guide);
        self.cursor = 0;
    }

    /// Handle input for the TV Guide app.
    fn handle_tv_guide_input(&mut self, button: &Button) -> AppAction {
        use super::tv_guide::TV_REQUEST_PATH;

        let Some(ref mut guide) = self.tv_guide else {
            return AppAction::None;
        };

        match button {
            Button::Cancel => {
                if guide.tuned_channel.is_some() {
                    guide.untune();
                    AppAction::None
                } else {
                    AppAction::Exit
                }
            },
            Button::Up => {
                guide.select_up();
                self.lines = guide.text_content();
                AppAction::None
            },
            Button::Down => {
                guide.select_down();
                self.lines = guide.text_content();
                AppAction::None
            },
            Button::Left => {
                guide.scroll_left();
                AppAction::None
            },
            Button::Right => {
                guide.scroll_right();
                AppAction::None
            },
            Button::Confirm => {
                let tuned = if let Some(req) = guide.tune() {
                    // Build direct video URL and pass via VFS IPC.
                    let url = super::tv_guide::catalog::ChannelCatalog::download_url(&req.episode);
                    let data = format!("tune_url {url} {}", req.seek_secs);
                    log::info!("TV: tune CH{} -> {}", req.channel_index, req.episode.title,);
                    self.pending_vfs_request = Some((TV_REQUEST_PATH.to_string(), data));
                    true
                } else {
                    false
                };
                self.lines = guide.text_content();
                if tuned {
                    AppAction::RequestFullscreen
                } else {
                    AppAction::None
                }
            },
            Button::Select => {
                // Retry catalog fetch from scratch: clear existing
                // catalogs so the `all(|c| c.is_none())` guard passes.
                guide.reset_for_retry();
                self.lines = guide.text_content();
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle a content-area click for the current app.
    ///
    /// Delegates to the TV Guide click handler when applicable.
    pub fn handle_click(
        &mut self,
        lx: i32,
        ly: i32,
        cw: u32,
        ch: u32,
        fullscreen: bool,
    ) -> AppAction {
        if let Some(ref mut app) = self.delegate {
            let action = app.handle_click(lx, ly, cw, ch, fullscreen);
            self.sync_from_delegate();
            return action;
        }

        if self.title == "TV Guide"
            && let Some(ref mut guide) = self.tv_guide
        {
            if let Some(req) = guide.handle_click(lx, ly, cw, ch, fullscreen) {
                use super::tv_guide::TV_REQUEST_PATH;
                let url = super::tv_guide::catalog::ChannelCatalog::download_url(&req.episode);
                let data = format!("tune_url {url} {}", req.seek_secs);
                log::info!(
                    "TV: click-tune CH{} -> {}",
                    req.channel_index,
                    req.episode.title,
                );
                self.pending_vfs_request = Some((TV_REQUEST_PATH.to_string(), data));
                self.lines = guide.text_content();
                return AppAction::RequestFullscreen;
            }
            self.lines = guide.text_content();
        }
        AppAction::None
    }

    /// Get mutable reference to the TV guide state.
    pub fn tv_guide_state(&mut self) -> Option<&mut TvGuideState> {
        self.tv_guide.as_mut()
    }

    /// Sync AppRunner pub fields from the delegate app.
    ///
    /// This keeps the legacy `title`, `lines`, `browse_dir`, `viewing_file`
    /// fields in sync after delegate calls, for backward compatibility with
    /// external code that reads these fields directly.
    fn sync_from_delegate(&mut self) {
        if let Some(ref app) = self.delegate {
            self.lines = app.lines().to_vec();
            self.browse_dir = app.browse_dir().map(String::from);
            self.viewing_file = app.viewing_file().map(String::from);
        }
    }

    /// Get a reference to the delegate app, if present.
    /// Get a reference to the delegate app, downcasting with `as_any()`.
    pub fn delegate_as<T: 'static>(&self) -> Option<&T> {
        self.delegate
            .as_ref()
            .and_then(|app| app.as_any().downcast_ref::<T>())
    }

    /// Get a mutable reference to the delegate app, downcasting with `as_any_mut()`.
    pub fn delegate_as_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.delegate
            .as_mut()
            .and_then(|app| app.as_any_mut().downcast_mut::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::file_viewer::{view_audio_file, view_image_file};
    use crate::backend::Color;
    use crate::dashboard::AppEntry;
    use crate::sdi::SdiRegistry;
    use crate::vfs::MemoryVfs;

    fn make_app(title: &str) -> AppEntry {
        AppEntry {
            title: title.to_string(),
            path: format!("/apps/{title}"),
            icon_png: Vec::new(),
            color: Color::rgb(100, 100, 100),
        }
    }

    fn setup_vfs() -> MemoryVfs {
        use crate::vfs::Vfs;
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/home/user/music").unwrap();
        vfs.mkdir("/home/user/photos").unwrap();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/home/user/readme.txt", b"Hello!").unwrap();
        vfs.write("/etc/hostname", b"oasis").unwrap();
        // Sample music tracks.
        vfs.write(
            "/home/user/music/ambient_dawn.mp3",
            b"fake-mp3-data-ambient",
        )
        .unwrap();
        vfs.write(
            "/home/user/music/nightfall_theme.mp3",
            b"fake-mp3-data-nightfall",
        )
        .unwrap();
        // Sample photo.
        vfs.write("/home/user/photos/sunset.png", b"\x89PNG\r\n\x1a\nfake-png")
            .unwrap();
        vfs
    }

    #[test]
    fn launch_file_manager() {
        let vfs = setup_vfs();
        let runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        assert_eq!(runner.title, "File Manager");
        assert!(runner.browse_dir.is_some());
        assert!(!runner.lines.is_empty());
        // Root should list etc, home, tmp directories.
        assert!(runner.lines.iter().any(|l| l.contains("home")));
    }

    #[test]
    fn launch_settings() {
        let vfs = setup_vfs();
        let runner = AppRunner::launch(&make_app("Settings"), &vfs);
        assert!(runner.lines.iter().any(|l| l.contains("480")));
    }

    #[test]
    fn launch_generic_app() {
        let vfs = setup_vfs();
        let runner = AppRunner::launch(&make_app("Unknown App"), &vfs);
        assert!(runner.lines.iter().any(|l| l.contains("No content")));
    }

    #[test]
    fn file_manager_navigate_down() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.panels[0].cursor, 0);
        runner.handle_input(&Button::Down, &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.panels[0].cursor, 1);
    }

    #[test]
    fn file_manager_enter_directory() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let home_idx = runner.delegate_as::<FileManagerApp>().unwrap().panels[0]
            .lines
            .iter()
            .position(|l: &String| l.starts_with("home"))
            .expect("home/ should be in listing");
        for _ in 0..home_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        assert_eq!(runner.browse_dir.as_deref(), Some("/home"));
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert!(
            fm.panels[0]
                .lines
                .iter()
                .any(|l: &String| l.contains("user"))
        );
    }

    #[test]
    fn file_manager_go_up() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let home_idx = runner.delegate_as::<FileManagerApp>().unwrap().panels[0]
            .lines
            .iter()
            .position(|l: &String| l.starts_with("home"))
            .expect("home/ should be in listing");
        for _ in 0..home_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        assert_eq!(runner.browse_dir.as_deref(), Some("/home"));

        runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(runner.browse_dir.as_deref(), Some("/"));
    }

    #[test]
    fn cancel_exits_app() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Settings"), &vfs);
        let action = runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn terminal_app_switches_mode() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Terminal"), &vfs);
        let action = runner.handle_input(&Button::Confirm, &vfs);
        assert_eq!(action, AppAction::SwitchToTerminal);
    }

    #[test]
    fn scroll_down_when_content_exceeds_view() {
        use crate::apps::simple_app::SimpleApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Settings"), &vfs);
        // Settings uses delegate -- add extra lines via the delegate's content.
        let app = runner.delegate_as_mut::<SimpleApp>().unwrap();
        for i in 0..20 {
            app.content.lines.push(format!("Extra line {i}"));
        }
        app.content.cached_max_visible = MAX_VISIBLE_LINES;
        // Move cursor to bottom of visible area.
        for _ in 0..MAX_VISIBLE_LINES - 1 {
            runner.handle_input(&Button::Down, &vfs);
        }
        let app = runner.delegate_as::<SimpleApp>().unwrap();
        assert_eq!(app.content.cursor, MAX_VISIBLE_LINES - 1);
        // Next down should scroll.
        runner.handle_input(&Button::Down, &vfs);
        let app = runner.delegate_as::<SimpleApp>().unwrap();
        assert_eq!(app.content.scroll, 1);
    }

    #[test]
    fn update_sdi_creates_objects() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Settings"), &vfs);
        let mut sdi = SdiRegistry::new();
        runner.update_sdi(&mut sdi, &ActiveTheme::default());
        assert!(sdi.contains("app_bg"));
        assert!(sdi.contains("app_title_bg"));
        assert!(sdi.contains("app_title_text"));
        assert!(sdi.contains("app_line_0"));
        assert!(sdi.contains("app_scroll"));
    }

    #[test]
    fn hide_sdi_hides_objects() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Settings"), &vfs);
        let mut sdi = SdiRegistry::new();
        runner.update_sdi(&mut sdi, &ActiveTheme::default());
        AppRunner::hide_sdi(&mut sdi);
        assert!(!sdi.get("app_bg").unwrap().visible);
        assert!(!sdi.get("app_title_bg").unwrap().visible);
    }

    #[test]
    fn list_directory_root() {
        let vfs = setup_vfs();
        let lines = list_directory(&vfs, "/");
        // Root has no ".." entry.
        assert!(!lines.iter().any(|l| l == ".."));
        // Should have directories.
        assert!(lines.iter().any(|l| l.starts_with("home")));
    }

    #[test]
    fn list_directory_shows_sizes() {
        let vfs = setup_vfs();
        let lines = list_directory(&vfs, "/home/user");
        // readme.txt is 6 bytes.
        assert!(
            lines
                .iter()
                .any(|l| l.contains("readme.txt") && l.contains("6 B"))
        );
    }

    /// Helper: navigate active panel cursor to a specific entry index.
    fn navigate_panel_to(runner: &mut AppRunner, idx: usize, vfs: &dyn Vfs) {
        // Reset cursor to 0 first (go up until we can't).
        for _ in 0..20 {
            runner.handle_input(&Button::Up, vfs);
        }
        for _ in 0..idx {
            runner.handle_input(&Button::Down, vfs);
        }
    }

    /// Helper: find entry index in active panel lines (delegate-aware).
    fn find_panel_entry(runner: &AppRunner, needle: &str) -> usize {
        use crate::apps::file_manager::FileManagerApp;
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        let p = &fm.panels[fm.active_panel];
        p.lines
            .iter()
            .position(|l: &String| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not found in panel lines"))
    }

    #[test]
    fn file_manager_open_file() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let home_idx = find_panel_entry(&runner, "home");
        navigate_panel_to(&mut runner, home_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let user_idx = find_panel_entry(&runner, "user");
        navigate_panel_to(&mut runner, user_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let file_idx = find_panel_entry(&runner, "readme.txt");
        navigate_panel_to(&mut runner, file_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        assert!(runner.viewing_file.is_some());
        assert!(runner.lines.iter().any(|l| l.contains("Hello!")));
    }

    #[test]
    fn file_viewer_cancel_returns_to_dir() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let home_idx = find_panel_entry(&runner, "home");
        navigate_panel_to(&mut runner, home_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let user_idx = find_panel_entry(&runner, "user");
        navigate_panel_to(&mut runner, user_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let file_idx = find_panel_entry(&runner, "readme.txt");
        navigate_panel_to(&mut runner, file_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        assert!(runner.viewing_file.is_some());
        let action = runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert!(runner.viewing_file.is_none());
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert!(
            fm.panels[0]
                .lines
                .iter()
                .any(|l: &String| l.contains("readme.txt"))
        );
    }

    #[test]
    fn file_viewer_binary_file() {
        use crate::vfs::Vfs;
        let mut vfs = setup_vfs();
        vfs.write("/home/user/data.bin", &[0x00, 0x01, 0xFF, 0xFE, 0x80])
            .unwrap();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let home_idx = find_panel_entry(&runner, "home");
        navigate_panel_to(&mut runner, home_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let user_idx = find_panel_entry(&runner, "user");
        navigate_panel_to(&mut runner, user_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        let file_idx = find_panel_entry(&runner, "data.bin");
        navigate_panel_to(&mut runner, file_idx, &vfs);
        runner.handle_input(&Button::Confirm, &vfs);
        assert!(runner.viewing_file.is_some());
        assert!(runner.lines.iter().any(|l| l.contains("Binary file")));
        assert!(runner.lines.iter().any(|l| l.contains("00 01 ff fe")));
    }

    #[test]
    fn view_audio_wav_metadata() {
        // Minimal valid WAV header (44 bytes).
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&36u32.to_le_bytes()); // file size - 8
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&2u16.to_le_bytes()); // channels
        wav.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
        wav.extend_from_slice(&176400u32.to_le_bytes()); // byte rate
        wav.extend_from_slice(&4u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes()); // data size

        let lines = view_audio_file("/music/test.wav", &wav);
        assert!(lines.iter().any(|l| l.contains("WAV")));
        assert!(lines.iter().any(|l| l.contains("44100")));
        assert!(lines.iter().any(|l| l.contains("2")));
        assert!(lines.iter().any(|l| l.contains("16-bit")));
    }

    #[test]
    fn view_audio_mp3_metadata() {
        // Fake MP3 with sync bytes.
        let data = vec![0xFF, 0xFB, 0x90, 0x00, 0x00];
        let lines = view_audio_file("/music/song.mp3", &data);
        assert!(lines.iter().any(|l| l.contains("MP3")));
        assert!(lines.iter().any(|l| l.contains("music play")));
    }

    #[test]
    fn view_image_png_metadata() {
        // Minimal PNG: 8-byte signature + IHDR chunk.
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n"); // signature
        png.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&480u32.to_be_bytes()); // width
        png.extend_from_slice(&272u32.to_be_bytes()); // height
        png.push(8); // bit depth
        png.push(6); // color type (RGBA)
        png.extend_from_slice(&[0, 0, 0]); // compression, filter, interlace

        let lines = view_image_file("/photos/test.png", &png);
        assert!(lines.iter().any(|l| l.contains("PNG")));
        assert!(lines.iter().any(|l| l.contains("480 x 272")));
        assert!(lines.iter().any(|l| l.contains("RGBA")));
    }

    #[test]
    fn view_image_jpeg_metadata() {
        // Minimal JPEG with SOF0 marker.
        let data = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xC0, // SOF0
            0x00, 0x0B, // length
            0x08, // precision
            0x01, 0x10, // height = 272
            0x01, 0xE0, // width = 480
            0x03, // components
            0x01, 0x22, 0x00,
        ];
        let lines = view_image_file("/photos/pic.jpg", &data);
        assert!(lines.iter().any(|l| l.contains("JPEG")));
        assert!(lines.iter().any(|l| l.contains("480 x 272")));
    }

    #[test]
    fn music_player_lists_tracks() {
        let vfs = setup_vfs();
        let runner = AppRunner::launch(&make_app("Music Player"), &vfs);
        assert!(runner.browse_dir.is_some());
        // Uses list_directory, so ".." is first, then files.
        assert!(runner.lines.iter().any(|l| l.contains("ambient_dawn")));
        assert!(runner.lines.iter().any(|l| l.contains("nightfall_theme")));
    }

    #[test]
    fn music_player_open_track() {
        use crate::apps::browsing_app::BrowsingApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Music Player"), &vfs);
        let app = runner.delegate_as::<BrowsingApp>().unwrap();
        let track_idx = app
            .content
            .lines
            .iter()
            .position(|l: &String| l.contains("ambient_dawn"))
            .unwrap();
        for _ in 0..track_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        // Should open audio viewer with track info and playback hints.
        assert!(runner.viewing_file.is_some());
        assert!(runner.lines.iter().any(|l| l.contains("Now Viewing")));
        assert!(runner.lines.iter().any(|l| l.contains("music play")));
    }

    #[test]
    fn music_player_empty() {
        use crate::vfs::Vfs;
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        // Music dir doesn't exist.
        let runner = AppRunner::launch(&make_app("Music Player"), &vfs);
        assert!(
            runner
                .lines
                .iter()
                .any(|l| l.contains("Music directory not found"))
        );
    }

    #[test]
    fn photo_viewer_lists_photos() {
        let vfs = setup_vfs();
        let runner = AppRunner::launch(&make_app("Photo Viewer"), &vfs);
        assert!(runner.browse_dir.is_some());
        assert!(runner.lines.iter().any(|l| l.contains("sunset.png")));
    }

    #[test]
    fn photo_viewer_open_image() {
        use crate::apps::browsing_app::BrowsingApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Photo Viewer"), &vfs);
        let app = runner.delegate_as::<BrowsingApp>().unwrap();
        let photo_idx = app
            .content
            .lines
            .iter()
            .position(|l: &String| l.contains("sunset.png"))
            .unwrap();
        for _ in 0..photo_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        // Photo viewer shows image metadata.
        assert!(runner.viewing_file.is_some());
        assert!(runner.lines.iter().any(|l| l.contains("Photo:")));
    }

    #[test]
    fn photo_viewer_cancel_from_view() {
        use crate::apps::browsing_app::BrowsingApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("Photo Viewer"), &vfs);
        let app = runner.delegate_as::<BrowsingApp>().unwrap();
        let photo_idx = app
            .content
            .lines
            .iter()
            .position(|l: &String| l.contains("sunset.png"))
            .unwrap();
        for _ in 0..photo_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        assert!(runner.viewing_file.is_some());
        // Cancel returns to photo list.
        let action = runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert!(runner.viewing_file.is_none());
        assert!(runner.lines.iter().any(|l| l.contains("sunset.png")));
    }

    #[test]
    fn photo_viewer_empty_dir() {
        use crate::vfs::Vfs;
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/home/user/photos").unwrap();
        let runner = AppRunner::launch(&make_app("Photo Viewer"), &vfs);
        // Empty dir shows "(empty directory)" via list_directory.
        assert!(runner.lines.iter().any(|l| l.contains("empty directory")));
    }

    #[test]
    fn dual_panel_switch_active() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        assert!(runner.delegate_as::<FileManagerApp>().is_some());
        assert_eq!(
            runner.delegate_as::<FileManagerApp>().unwrap().active_panel,
            0
        );

        // Right switches to panel 1.
        runner.handle_input(&Button::Right, &vfs);
        assert_eq!(
            runner.delegate_as::<FileManagerApp>().unwrap().active_panel,
            1
        );

        // Left switches back to panel 0.
        runner.handle_input(&Button::Left, &vfs);
        assert_eq!(
            runner.delegate_as::<FileManagerApp>().unwrap().active_panel,
            0
        );
    }

    #[test]
    fn dual_panel_independent_navigation() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);

        // Navigate down in left panel (panel 0).
        runner.handle_input(&Button::Down, &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.panels[0].cursor, 1);

        // Switch to right panel.
        runner.handle_input(&Button::Right, &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.active_panel, 1);

        // Right panel cursor should still be at 0.
        assert_eq!(fm.panels[1].cursor, 0);

        // Navigate down in right panel.
        runner.handle_input(&Button::Down, &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.panels[1].cursor, 1);

        // Left panel cursor should still be at 1.
        assert_eq!(fm.panels[0].cursor, 1);
    }

    #[test]
    fn dual_panel_enter_directory() {
        use crate::apps::file_manager::FileManagerApp;
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);

        // Find "home/" and navigate into it on left panel.
        let home_idx = runner.delegate_as::<FileManagerApp>().unwrap().panels[0]
            .lines
            .iter()
            .position(|l: &String| l.starts_with("home"))
            .expect("home/ should be in listing");
        // Move cursor to home entry.
        for _ in 0..home_idx {
            runner.handle_input(&Button::Down, &vfs);
        }
        runner.handle_input(&Button::Confirm, &vfs);
        let fm = runner.delegate_as::<FileManagerApp>().unwrap();
        assert_eq!(fm.panels[0].browse_dir, "/home");
        // Right panel should still be at root.
        assert_eq!(fm.panels[1].browse_dir, "/");
    }

    #[test]
    fn dual_panel_sdi_objects() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let mut sdi = SdiRegistry::new();
        runner.update_sdi(&mut sdi, &ActiveTheme::default());
        assert!(sdi.contains("app_bg"));
        assert!(sdi.contains("app_divider"));
        assert!(sdi.contains("app_lp_line_0"));
        assert!(sdi.contains("app_rp_line_0"));
        assert!(sdi.contains("app_scroll"));
    }

    #[test]
    fn dual_panel_hide_sdi() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("File Manager"), &vfs);
        let mut sdi = SdiRegistry::new();
        runner.update_sdi(&mut sdi, &ActiveTheme::default());
        AppRunner::hide_sdi(&mut sdi);
        assert!(!sdi.get("app_bg").unwrap().visible);
        assert!(!sdi.get("app_divider").unwrap().visible);
        assert!(!sdi.get("app_lp_line_0").unwrap().visible);
        assert!(!sdi.get("app_rp_line_0").unwrap().visible);
    }

    // ---------------------------------------------------------------
    // TV Guide lifecycle tests
    // ---------------------------------------------------------------

    #[test]
    fn tv_guide_launch_and_catalog_inject() {
        use crate::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};

        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);
        assert!(runner.tv_guide_state().is_some());
        // Initially shows "Loading".
        assert!(runner.lines.iter().any(|l| l.contains("Loading")));

        // Inject a catalog for channel 0.
        let guide = runner.tv_guide_state().unwrap();
        let ch_num = guide.channels[0].number;
        let mut catalog = ChannelCatalog::new(ch_num);
        catalog.add_episodes(vec![VideoEpisode {
            item_id: "test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Space Adventures".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 5000,
            format: "MPEG4".into(),
            original: None,
        }]);
        guide.catalogs[0] = Some(catalog);
        guide.rebuild_cached_schedule(0);
        guide.fetch_attempted = true;

        // Refresh text lines.
        runner.refresh_tv_text();
        assert!(runner.lines.iter().any(|l| l.contains("Space Adventures")));
        assert!(!runner.lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn tv_guide_error_display() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        let guide = runner.tv_guide_state().unwrap();
        guide.fetch_attempted = true;
        guide.fetch_error = Some("connection refused".to_string());

        runner.refresh_tv_text();
        assert!(
            runner
                .lines
                .iter()
                .any(|l| l.contains("Error: connection refused"))
        );
        assert!(!runner.lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn tv_guide_tune_with_catalog() {
        use crate::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};

        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        // Inject catalog.
        let guide = runner.tv_guide_state().unwrap();
        let ch_num = guide.channels[0].number;
        let mut catalog = ChannelCatalog::new(ch_num);
        catalog.add_episodes(vec![VideoEpisode {
            item_id: "tune-test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Tune Test Episode".to_string(),
            duration_secs: 3600.0,
            width: 640,
            height: 480,
            size_bytes: 5000,
            format: "MPEG4".into(),
            original: None,
        }]);
        guide.catalogs[0] = Some(catalog);
        guide.rebuild_cached_schedule(0);

        // Press Confirm to tune -- TV Guide requests fullscreen on tune.
        let action = runner.handle_input(&Button::Confirm, &vfs);
        assert_eq!(action, AppAction::RequestFullscreen);

        // Should have a pending VFS request for the tune.
        let req = runner.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.unwrap();
        assert!(path.contains("tv"));
        assert!(data.starts_with("tune_url "));
        assert!(data.contains("tune-test"));
    }

    // ---------------------------------------------------------------
    // TV Guide video launch pipeline tests
    // ---------------------------------------------------------------

    /// Extract the URL from a `tune_url {url} {seek_secs}` IPC string.
    fn extract_tune_url(data: &str) -> &str {
        let rest = &data["tune_url ".len()..];
        rest.rsplit_once(' ').map_or(rest, |(url, _)| url)
    }

    /// Extract the seek_secs from a `tune_url {url} {seek_secs}` IPC string.
    fn extract_tune_seek(data: &str) -> u64 {
        let rest = &data["tune_url ".len()..];
        rest.rsplit_once(' ')
            .and_then(|(_, s)| s.parse().ok())
            .unwrap_or(0)
    }

    /// Helper: create a TV Guide runner with a catalog injected for channel 0.
    fn setup_tv_guide_with_catalog(
        item_id: &str,
        filename: &str,
        title: &str,
    ) -> (AppRunner, crate::vfs::MemoryVfs) {
        use crate::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};

        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        let guide = runner.tv_guide_state().unwrap();
        let ch_num = guide.channels[0].number;
        let mut catalog = ChannelCatalog::new(ch_num);
        catalog.add_episodes(vec![VideoEpisode {
            item_id: item_id.to_string(),
            filename: filename.to_string(),
            title: title.to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 50000,
            format: "MPEG4".into(),
            original: None,
        }]);
        guide.catalogs[0] = Some(catalog);
        guide.rebuild_cached_schedule(0);
        guide.fetch_attempted = true;

        (runner, vfs)
    }

    #[test]
    fn tv_tune_url_is_direct_download_not_embed() {
        let (mut runner, vfs) = setup_tv_guide_with_catalog("my-item", "video.mp4", "My Video");

        runner.handle_input(&Button::Confirm, &vfs);
        let (_, data) = runner.take_pending_request().unwrap();

        // Must use tune_url prefix (not old "tune " format).
        assert!(
            data.starts_with("tune_url "),
            "expected tune_url, got: {data}"
        );

        let url = extract_tune_url(&data);
        let seek = extract_tune_seek(&data);

        // Must include seek_secs in IPC data.
        assert!(
            seek > 0 || data.ends_with(" 0"),
            "missing seek_secs: {data}"
        );

        // Must be a direct download URL, not an embed URL.
        assert!(
            url.starts_with("https://archive.org/download/"),
            "expected download URL, got: {url}",
        );
        assert!(
            !url.contains("/embed/"),
            "URL must not use embed endpoint: {url}",
        );
    }

    #[test]
    fn tv_tune_url_contains_specific_filename() {
        let (mut runner, vfs) =
            setup_tv_guide_with_catalog("sonic-episodes", "Season1/ep01.mp4", "Episode 1");

        runner.handle_input(&Button::Confirm, &vfs);
        let (_, data) = runner.take_pending_request().unwrap();
        let url = extract_tune_url(&data);

        // URL must contain the item ID.
        assert!(url.contains("sonic-episodes"), "missing item_id in: {url}");

        // URL must contain the filename (possibly percent-encoded).
        assert!(
            url.contains("Season1") && url.contains("ep01.mp4"),
            "missing filename in: {url}",
        );
    }

    #[test]
    fn tv_tune_url_percent_encodes_special_chars() {
        let (mut runner, vfs) =
            setup_tv_guide_with_catalog("test-item", "My Video #1.mp4", "My Video");

        runner.handle_input(&Button::Confirm, &vfs);
        let (_, data) = runner.take_pending_request().unwrap();
        let url = extract_tune_url(&data);

        // '#' must be percent-encoded to '%23' (raw '#' breaks URLs).
        assert!(!url.contains('#'), "raw '#' in URL breaks fragment: {url}");
        assert!(url.contains("%23"), "expected percent-encoded '#': {url}");

        // Spaces should be percent-encoded too.
        assert!(!url.contains("My Video"), "raw spaces in URL: {url}",);
    }

    #[test]
    fn tv_tune_navigate_then_tune_second_channel() {
        use crate::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};

        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        // Inject catalogs for channels 0 and 1.
        let guide = runner.tv_guide_state().unwrap();
        for i in 0..2 {
            let ch_num = guide.channels[i].number;
            let mut catalog = ChannelCatalog::new(ch_num);
            catalog.add_episodes(vec![VideoEpisode {
                item_id: format!("item-ch{i}"),
                filename: format!("ch{i}_video.mp4"),
                title: format!("Channel {i} Show"),
                duration_secs: 1800.0,
                width: 640,
                height: 480,
                size_bytes: 5000,
                format: "MPEG4".into(),
                original: None,
            }]);
            guide.catalogs[i] = Some(catalog);
            guide.rebuild_cached_schedule(i);
        }

        // Navigate down to channel 1.
        runner.handle_input(&Button::Down, &vfs);
        let guide = runner.tv_guide_state().unwrap();
        assert_eq!(guide.selected_channel, 1);

        // Tune channel 1.
        runner.handle_input(&Button::Confirm, &vfs);
        let (_, data) = runner.take_pending_request().unwrap();
        let url = extract_tune_url(&data);

        // URL must reference channel 1's item, not channel 0's.
        assert!(
            url.contains("item-ch1"),
            "expected channel 1 item_id, got: {url}",
        );
        assert!(
            url.contains("ch1_video.mp4"),
            "expected channel 1 filename, got: {url}",
        );
    }

    #[test]
    fn tv_tune_without_catalog_produces_no_request() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        // Press Confirm with no catalogs loaded.
        runner.handle_input(&Button::Confirm, &vfs);
        assert!(
            runner.take_pending_request().is_none(),
            "should not produce tune request without catalog",
        );
    }

    #[test]
    fn tv_select_resets_fetch_for_retry() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        // Simulate a failed fetch.
        let guide = runner.tv_guide_state().unwrap();
        guide.fetch_attempted = true;
        guide.fetch_error = Some("network error".to_string());
        runner.refresh_tv_text();
        assert!(runner.lines.iter().any(|l| l.contains("Error")));

        // Press Select to retry.
        runner.handle_input(&Button::Select, &vfs);

        let guide = runner.tv_guide_state().unwrap();
        assert!(!guide.fetch_attempted, "fetch_attempted should be reset");
        assert!(guide.fetch_error.is_none(), "fetch_error should be cleared");

        // Text should now show loading again.
        assert!(runner.lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn tv_select_retry_clears_partial_catalogs() {
        let vfs = setup_vfs();
        let mut runner = AppRunner::launch(&make_app("TV Guide"), &vfs);

        // Simulate a partial fetch: first channel loaded, rest failed.
        let guide = runner.tv_guide_state().unwrap();
        guide.fetch_attempted = true;
        assert!(!guide.catalogs.is_empty(), "need channels for this test");
        guide.catalogs[0] = Some(crate::apps::tv_guide::catalog::ChannelCatalog::new(0));

        // Press Select to retry — should clear all catalogs.
        runner.handle_input(&Button::Select, &vfs);

        let guide = runner.tv_guide_state().unwrap();
        assert!(!guide.fetch_attempted, "fetch_attempted should be reset");
        assert!(
            guide.catalogs.iter().all(|c| c.is_none()),
            "catalogs should be cleared so fetch guard passes"
        );
    }

    #[test]
    fn tv_cancel_while_tuned_untunes_instead_of_exit() {
        let (mut runner, vfs) = setup_tv_guide_with_catalog("item-x", "video.mp4", "Test Show");

        // Tune to a channel.
        runner.handle_input(&Button::Confirm, &vfs);
        let guide = runner.tv_guide_state().unwrap();
        assert!(guide.tuned_channel.is_some());

        // Cancel should untune, not exit.
        let action = runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        let guide = runner.tv_guide_state().unwrap();
        assert!(guide.tuned_channel.is_none());

        // Second cancel should exit.
        let action = runner.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn tv_tune_request_path_matches_constant() {
        use crate::apps::tv_guide::TV_REQUEST_PATH;

        let (mut runner, vfs) = setup_tv_guide_with_catalog("path-test", "ep.mp4", "Path Test");

        runner.handle_input(&Button::Confirm, &vfs);
        let (path, _) = runner.take_pending_request().unwrap();
        assert_eq!(path, TV_REQUEST_PATH, "IPC path must match TV_REQUEST_PATH");
    }

    #[test]
    fn tv_tune_url_is_well_formed_https() {
        let (mut runner, vfs) = setup_tv_guide_with_catalog("https-test", "ep.mp4", "HTTPS Test");

        runner.handle_input(&Button::Confirm, &vfs);
        let (_, data) = runner.take_pending_request().unwrap();
        let url = extract_tune_url(&data);

        assert!(url.starts_with("https://"), "URL must be HTTPS: {url}");
        assert!(!url.contains(' '), "URL must not contain spaces: {url}");
        assert!(
            url.contains("archive.org"),
            "URL must target archive.org: {url}"
        );
    }

    // ---------------------------------------------------------------
    // TV Guide click handler tests
    // ---------------------------------------------------------------

    #[test]
    fn tv_click_selects_then_tunes() {
        let (mut runner, _vfs) = setup_tv_guide_with_catalog("click-test", "ep.mp4", "Click Tune");

        // Content dimensions matching a typical window.
        let (cw, ch) = (800u32, 600u32);

        // Compute layout to find a row 1 y position.
        let usable_h = ch;
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        let footer_h = (usable_h * 5 / 100).max(18);
        let grid_h = usable_h.saturating_sub(header_h + time_header_h + footer_h);
        let row_count = 5u32; // default channels
        let row_h = (grid_h / row_count).max(20);
        let grid_y = header_h + time_header_h;

        // Click row 1 — should select channel 1 (not tune).
        let ly = (grid_y + row_h + row_h / 2) as i32;
        let action = runner.handle_click(100, ly, cw, ch, true);
        assert_eq!(action, AppAction::None);
        assert_eq!(runner.tv_guide_state().unwrap().selected_channel, 1);
        assert!(runner.take_pending_request().is_none());

        // Click row 0 — selects channel 0 (catalog is on ch 0).
        let ly0 = (grid_y + row_h / 2) as i32;
        let action = runner.handle_click(100, ly0, cw, ch, true);
        assert_eq!(action, AppAction::None);
        assert_eq!(runner.tv_guide_state().unwrap().selected_channel, 0);

        // Click row 0 again — already selected, should tune.
        let action = runner.handle_click(100, ly0, cw, ch, true);
        assert_eq!(action, AppAction::RequestFullscreen);
        let (path, data) = runner.take_pending_request().unwrap();
        assert!(path.contains("tv"));
        assert!(data.starts_with("tune_url "));
    }

    #[test]
    fn tv_click_outside_grid_is_noop() {
        let (mut runner, _vfs) = setup_tv_guide_with_catalog("noop-test", "ep.mp4", "Noop");

        // Click in the header area.
        let action = runner.handle_click(100, 10, 800, 600, true);
        assert_eq!(action, AppAction::None);
        assert!(runner.take_pending_request().is_none());
    }
}
