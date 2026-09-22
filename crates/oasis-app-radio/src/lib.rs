//! Internet Radio app for OASIS_OS.
//!
//! Displays the station list from VFS config and the playback status the
//! radio subsystem publishes to [`RADIO_STATUS_PATH`], and sends tune /
//! favorite / stop / volume requests via VFS IPC ([`RADIO_REQUEST_PATH`]).
//!
//! Controls: Up/Down select a station, Confirm tunes, Triangle toggles
//! favorite, Start stops, Left/Right change the volume. Keyboard hosts
//! add Escape = stop (closes the app when already stopped), `+` / `-`
//! and Q / E (the shoulder-button keys) = volume. Windowed mode also has
//! clickable station rows, a click-to-set volume bar and Vol-/Vol+/Stop
//! buttons.

use std::cell::Cell;

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_audio::radio::station::StationRegistry;
use oasis_audio::{RADIO_APP_TITLE, RADIO_REQUEST_PATH, RADIO_STATUS_PATH};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

mod layout;
mod render;

use layout::{PanelButton, RadioLayout};
pub use render::RadioColors;

/// Volume change per key press, in percent.
pub const VOLUME_STEP: u8 = 10;

/// Volume assumed until the radio subsystem publishes one.
const DEFAULT_VOLUME: u8 = 80;

/// Status refreshes an optimistic volume survives without the published
/// status confirming it (~1 s of per-frame refreshes).
const VOLUME_OVERRIDE_TTL: u8 = 60;

/// Number of header lines above the station list in the text view.
const STATION_HEADER_LINES: usize = 7;

/// Stations config file (falls back to the built-in defaults).
const STATIONS_PATH: &str = "/etc/radio/stations.toml";

/// Radio playback state as published in the status file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    /// Nothing playing.
    Stopped,
    /// Connecting / buffering / loading the next track.
    Loading,
    /// Audio is playing.
    Playing,
    /// Playback failed.
    Error,
}

impl PlayState {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "playing" => PlayState::Playing,
            "connecting" | "buffering" | "loading" => PlayState::Loading,
            "error" => PlayState::Error,
            _ => PlayState::Stopped,
        }
    }

    /// Short label for the status badge.
    pub fn label(self) -> &'static str {
        match self {
            PlayState::Stopped => "STOPPED",
            PlayState::Loading => "LOADING",
            PlayState::Playing => "PLAYING",
            PlayState::Error => "ERROR",
        }
    }
}

/// Parsed contents of [`RADIO_STATUS_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioStatus {
    /// Playback state.
    pub state: PlayState,
    /// Raw state word (e.g. "buffering"), for display.
    pub state_text: String,
    /// Tuned station, if any.
    pub station: Option<String>,
    /// Current track / stream title, if any.
    pub now_playing: Option<String>,
    /// Failure reason while in the error state.
    pub error: Option<String>,
    /// Published volume (0-100).
    pub volume: Option<u8>,
}

impl Default for RadioStatus {
    fn default() -> Self {
        Self {
            state: PlayState::Stopped,
            state_text: "stopped".to_string(),
            station: None,
            now_playing: None,
            error: None,
            volume: None,
        }
    }
}

impl RadioStatus {
    /// Parse the status file text (`Key: value` lines).
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        let known = |v: &str| {
            let v = v.trim();
            (!v.is_empty() && v != "--").then(|| v.to_string())
        };
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("State: ") {
                s.state = PlayState::parse(v);
                s.state_text = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Station: ") {
                s.station = known(v);
            } else if let Some(v) = line.strip_prefix("Now Playing: ") {
                s.now_playing = known(v);
            } else if let Some(v) = line.strip_prefix("Error: ") {
                s.error = known(v);
            } else if let Some(v) = line.strip_prefix("Volume: ") {
                s.volume = v.trim().trim_end_matches('%').trim().parse::<u8>().ok();
            }
        }
        s
    }

    /// Read the status from the VFS (default "stopped" when missing).
    pub fn load(vfs: &dyn Vfs) -> Self {
        if !vfs.exists(RADIO_STATUS_PATH) {
            return Self::default();
        }
        vfs.read(RADIO_STATUS_PATH)
            .map(|d| Self::parse(&String::from_utf8_lossy(&d)))
            .unwrap_or_default()
    }

    /// Whether Stop has anything to stop.
    pub fn is_active(&self) -> bool {
        self.state != PlayState::Stopped
    }
}

/// One row of the station list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationRow {
    /// Station name.
    pub name: String,
    /// Genre tag.
    pub genre: String,
    /// Bitrate (`128k`) or archive collection.
    pub info: String,
    /// Favorite flag.
    pub favorite: bool,
}

/// Load the station list from [`STATIONS_PATH`] (or the defaults).
fn load_stations(vfs: &dyn Vfs) -> Vec<StationRow> {
    let registry = if vfs.exists(STATIONS_PATH) {
        let data = vfs.read(STATIONS_PATH).unwrap_or_default();
        let text = String::from_utf8_lossy(&data);
        StationRegistry::from_toml(&text).unwrap_or_else(|_| StationRegistry::defaults())
    } else {
        StationRegistry::defaults()
    };
    registry
        .stations
        .into_iter()
        .map(|s| {
            let info = if s.source_type == "icecast" {
                if s.bitrate > 0 {
                    format!("{}k", s.bitrate)
                } else {
                    "?".to_string()
                }
            } else if !s.collection.is_empty() {
                s.collection
            } else {
                "archive".to_string()
            };
            StationRow {
                name: s.name,
                genre: s.genre,
                info,
                favorite: s.favorite,
            }
        })
        .collect()
}

/// Internet Radio app implementing the `App` trait.
#[derive(Debug)]
pub struct RadioApp {
    content: ContentState,
    /// Last published playback status.
    status: RadioStatus,
    /// Station list.
    stations: Vec<StationRow>,
    /// Selected station index.
    selected: usize,
    /// First station row shown in the windowed list.
    list_scroll: usize,
    /// Station rows the windowed list showed last frame (keeps the
    /// selection scrolled into view).
    visible_rows: Cell<usize>,
    /// Volume requested but not yet confirmed by the published status,
    /// with the refreshes it has left.
    volume_override: Option<(u8, u8)>,
}

impl RadioApp {
    /// Create a new Internet Radio app, loading initial content from VFS.
    pub fn new(path: &str, vfs: &dyn Vfs) -> Self {
        let mut app = Self {
            content: ContentState::new(RADIO_APP_TITLE, path),
            status: RadioStatus::load(vfs),
            stations: load_stations(vfs),
            selected: 0,
            list_scroll: 0,
            visible_rows: Cell::new(8),
            volume_override: None,
        };
        app.rebuild_lines();
        app
    }

    /// Last published playback status.
    pub fn status(&self) -> &RadioStatus {
        &self.status
    }

    /// Volume shown to the user: the pending request, else the published
    /// value, else the subsystem default.
    pub fn volume(&self) -> u8 {
        self.volume_override
            .map(|(v, _)| v)
            .or(self.status.volume)
            .unwrap_or(DEFAULT_VOLUME)
    }

    fn request(&mut self, data: String) {
        self.content.pending_vfs_request = Some((RADIO_REQUEST_PATH.to_string(), data));
    }

    /// Request a volume (clamped to 0-100).
    pub fn set_volume(&mut self, volume: u8) {
        let volume = volume.min(100);
        self.volume_override = Some((volume, VOLUME_OVERRIDE_TTL));
        self.request(format!("vol {volume}"));
        self.rebuild_lines();
    }

    /// Step the volume up or down by [`VOLUME_STEP`].
    pub fn change_volume(&mut self, up: bool) {
        let v = self.volume();
        let next = if up {
            v.saturating_add(VOLUME_STEP).min(100)
        } else {
            v.saturating_sub(VOLUME_STEP)
        };
        self.set_volume(next);
    }

    /// Request playback stop.
    pub fn stop(&mut self) {
        self.request("stop".to_string());
    }

    /// Tune to the selected station.
    fn tune_selected(&mut self) {
        if self.selected < self.stations.len() {
            self.request(format!("tune {}", self.selected));
        }
    }

    /// Toggle the selected station's favorite flag.
    fn toggle_favorite(&mut self) {
        if let Some(s) = self.stations.get_mut(self.selected) {
            s.favorite = !s.favorite;
            let idx = self.selected;
            self.request(format!("fav {idx}"));
            self.rebuild_lines();
        }
    }

    /// Move the station selection by `delta` rows (clamped).
    fn move_selection(&mut self, delta: isize) {
        if self.stations.is_empty() {
            return;
        }
        let last = self.stations.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.scroll_into_view();
        self.rebuild_lines();
    }

    /// Keep the selection inside the windowed list viewport.
    fn scroll_into_view(&mut self) {
        let rows = self.visible_rows.get().max(1);
        if self.selected < self.list_scroll {
            self.list_scroll = self.selected;
        } else if self.selected >= self.list_scroll + rows {
            self.list_scroll = self.selected + 1 - rows;
        }
    }

    /// Station name + status line for the panel / text view.
    fn status_line(&self) -> String {
        let mut s = self.status.state_text.clone();
        if let Some(err) = &self.status.error {
            // Surface the failure reason inline so it can be diagnosed
            // without dev tools.
            s = format!("{s} - {err}");
        }
        s
    }

    /// Rebuild the text lines (full-screen SDI view) and point the
    /// generic line cursor at the selected station.
    fn rebuild_lines(&mut self) {
        let na = |v: &Option<String>| v.clone().unwrap_or_else(|| "--".to_string());
        let vol = self.volume();
        let filled = (vol as usize).div_ceil(10);
        let mut lines = vec![
            format!("Status: {}", self.status_line()),
            format!("Station: {}", na(&self.status.station)),
            format!("Now Playing: {}", na(&self.status.now_playing)),
            format!(
                "Volume: [{}{}] {vol}%",
                "#".repeat(filled),
                "-".repeat(10 - filled)
            ),
            String::new(),
            "--- Stations ---".to_string(),
            String::new(),
        ];
        debug_assert_eq!(lines.len(), STATION_HEADER_LINES);
        for s in &self.stations {
            let fav = if s.favorite { "*" } else { " " };
            lines.push(format!(
                "  [{fav}] {:<26} {:<12} {}",
                s.name, s.genre, s.info
            ));
        }
        lines.push(String::new());
        lines.push("Confirm=Tune  Tri=Fav  Start=Stop  L/R=Volume".to_string());
        self.content.lines = lines;

        // Highlight the selected station line, scrolling it into view.
        let target = STATION_HEADER_LINES + self.selected;
        let max = self.content.cached_max_visible.max(1);
        if target < self.content.scroll {
            self.content.scroll = target;
        } else if target >= self.content.scroll + max {
            self.content.scroll = target + 1 - max;
        }
        self.content.cursor = target - self.content.scroll;
    }

    /// Re-read the published status. Returns `true` when it changed.
    fn reload_status(&mut self, vfs: &dyn Vfs) -> bool {
        let status = RadioStatus::load(vfs);
        let mut changed = status != self.status;
        if let Some((v, ttl)) = self.volume_override {
            if status.volume == Some(v) || ttl <= 1 {
                self.volume_override = None;
                changed = true;
            } else {
                self.volume_override = Some((v, ttl - 1));
            }
        }
        self.status = status;
        if changed {
            self.rebuild_lines();
        }
        changed
    }
}

impl App for RadioApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => return AppAction::Exit,
            Button::Up => self.move_selection(-1),
            Button::Down => self.move_selection(1),
            Button::Left => self.change_volume(false),
            Button::Right => self.change_volume(true),
            Button::Confirm => self.tune_selected(),
            Button::Triangle => self.toggle_favorite(),
            Button::Start => self.stop(),
            Button::Square | Button::Select => {},
        }
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        if mods.has_command() {
            return None;
        }
        match key {
            // `+` arrives as the unshifted `=` key; Q / E are the
            // keyboard's shoulder buttons.
            Key::Char('+' | '=' | 'e') => self.change_volume(true),
            Key::Char('-' | 'q') => self.change_volume(false),
            // Escape stops playback; when already stopped it falls
            // through to Cancel and closes the app.
            Key::Escape if self.status.is_active() => self.stop(),
            _ => return None,
        }
        Some(AppAction::None)
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
        let l = RadioLayout::compute(0, 0, cw, ch);
        if let Some(button) = l.button_at(lx, ly) {
            match button {
                PanelButton::VolumeDown => self.change_volume(false),
                PanelButton::VolumeUp => self.change_volume(true),
                PanelButton::Stop => self.stop(),
            }
        } else if let Some(v) = l.volume_at(lx, ly) {
            self.set_volume(v);
        } else if let Some(row) = l.row_at(lx, ly) {
            let idx = self.list_scroll + row;
            if idx < self.stations.len() {
                // Clicking a station selects and tunes it.
                self.selected = idx;
                self.rebuild_lines();
                self.tune_selected();
            }
        }
        AppAction::None
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
        self.reload_status(vfs);
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
        self.draw_radio(cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_test_backend::{DrawCommand, RecordingBackend};
    use oasis_vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    fn write_status(vfs: &mut MemoryVfs, text: &str) {
        if !vfs.exists("/var") {
            vfs.mkdir("/var").expect("mkdir");
        }
        if !vfs.exists("/var/radio") {
            vfs.mkdir("/var/radio").expect("mkdir");
        }
        vfs.write(RADIO_STATUS_PATH, text.as_bytes())
            .expect("write status");
    }

    fn take(app: &mut RadioApp) -> Option<String> {
        app.take_pending_request().map(|(path, data)| {
            assert_eq!(path, RADIO_REQUEST_PATH);
            data
        })
    }

    fn drawn_texts(app: &RadioApp, w: u32, h: u32) -> Vec<String> {
        let mut backend = RecordingBackend::new(w, h);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, w, h, &mut backend, &at)
            .expect("draw");
        backend
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn radio_app_title_and_path() {
        let vfs = make_vfs();
        let app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.title(), "Internet Radio");
        assert_eq!(app.path(), "/apps/radio");
    }

    #[test]
    fn radio_app_has_default_stations() {
        let vfs = make_vfs();
        let app = RadioApp::new("/apps/radio", &vfs);
        assert!(app.lines().iter().any(|l| l.contains("Stations")));
        assert!(!app.stations.is_empty());
        assert!(app.lines().len() > STATION_HEADER_LINES + 2);
    }

    #[test]
    fn radio_cancel_exits() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn up_down_select_stations_and_highlight_line() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        assert!(app.stations.len() >= 2);
        app.content.cached_max_visible = 30;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.selected, 1);
        let line = &app.lines()[app.content.scroll + app.content.cursor];
        assert!(line.contains(&app.stations[1].name));
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn radio_tune_creates_request() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("tune 1"));
    }

    #[test]
    fn radio_favorite_creates_request_and_marks_row() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        let before = app.stations[0].favorite;
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("fav 0"));
        assert_ne!(app.stations[0].favorite, before);
    }

    #[test]
    fn start_stops_playback() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("stop"));
    }

    #[test]
    fn escape_stops_when_playing_else_falls_through() {
        let mut vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        // Stopped: Escape is left to Cancel (exit).
        assert_eq!(app.handle_key(&Key::Escape, Modifiers::NONE, &vfs), None);
        write_status(&mut vfs, "State: playing\nStation: Jazz FM\n");
        app.refresh(&vfs);
        assert_eq!(
            app.handle_key(&Key::Escape, Modifiers::NONE, &vfs),
            Some(AppAction::None)
        );
        assert_eq!(take(&mut app).as_deref(), Some("stop"));
    }

    #[test]
    fn dpad_left_right_change_volume() {
        let mut vfs = make_vfs();
        write_status(&mut vfs, "State: playing\nVolume: 50%\n");
        let mut app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.volume(), 50);
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("vol 60"));
        // Repeated presses build on the pending value before the status
        // catches up.
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("vol 70"));
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(take(&mut app).as_deref(), Some("vol 60"));
    }

    #[test]
    fn keyboard_volume_keys() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.volume(), DEFAULT_VOLUME);
        for (key, want) in [
            (Key::Char('='), "vol 90"),
            (Key::Char('+'), "vol 100"),
            (Key::Char('e'), "vol 100"),
            (Key::Char('-'), "vol 90"),
            (Key::Char('q'), "vol 80"),
        ] {
            assert_eq!(
                app.handle_key(&key, Modifiers::NONE, &vfs),
                Some(AppAction::None)
            );
            assert_eq!(take(&mut app).as_deref(), Some(want), "{key:?}");
        }
        // Shortcuts with Ctrl belong to the host; other keys fall through.
        assert_eq!(app.handle_key(&Key::Char('q'), Modifiers::CTRL, &vfs), None);
        assert_eq!(app.handle_key(&Key::Enter, Modifiers::NONE, &vfs), None);
    }

    #[test]
    fn volume_clamps_at_bounds() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.set_volume(3);
        app.change_volume(false);
        assert_eq!(app.volume(), 0);
        app.set_volume(250);
        assert_eq!(app.volume(), 100);
    }

    #[test]
    fn volume_override_settles_on_published_value() {
        let mut vfs = make_vfs();
        write_status(&mut vfs, "State: playing\nVolume: 50%\n");
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.change_volume(true);
        // Status not updated yet: keep showing the requested value.
        app.refresh(&vfs);
        assert_eq!(app.volume(), 60);
        write_status(&mut vfs, "State: playing\nVolume: 60%\n");
        app.refresh(&vfs);
        assert!(app.volume_override.is_none());
        assert_eq!(app.volume(), 60);
        // A request the subsystem never confirms expires.
        app.change_volume(true);
        for _ in 0..VOLUME_OVERRIDE_TTL {
            app.refresh(&vfs);
        }
        assert_eq!(app.volume(), 60);
    }

    #[test]
    fn radio_reads_status_from_vfs() {
        let mut vfs = make_vfs();
        write_status(
            &mut vfs,
            "State: playing\nVolume: 40%\nStation: Jazz FM\nNow Playing: Blue Note",
        );
        let app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.status().state, PlayState::Playing);
        assert!(app.lines().iter().any(|l| l.contains("playing")));
        assert!(app.lines().iter().any(|l| l.contains("Jazz FM")));
        assert!(app.lines().iter().any(|l| l.contains("Blue Note")));
        assert!(app.lines().iter().any(|l| l.contains("[####------] 40%")));
    }

    #[test]
    fn status_parse_error_and_placeholders() {
        let s = RadioStatus::parse("State: error\nStation: --\nError: 404 not found\n");
        assert_eq!(s.state, PlayState::Error);
        assert_eq!(s.station, None);
        assert_eq!(s.error.as_deref(), Some("404 not found"));
        assert!(s.is_active());
        assert_eq!(
            RadioStatus::parse("State: buffering").state,
            PlayState::Loading
        );
        assert!(!RadioStatus::parse("").is_active());
    }

    #[test]
    fn windowed_view_shows_state_volume_and_stations() {
        let mut vfs = make_vfs();
        write_status(
            &mut vfs,
            "State: playing\nVolume: 70%\nStation: Jazz FM\nNow Playing: Blue Note",
        );
        let app = RadioApp::new("/apps/radio", &vfs);
        let texts = drawn_texts(&app, 400, 240);
        for want in [
            "PLAYING",
            "Jazz FM",
            "Blue Note",
            "70%",
            "Vol-",
            "Vol+",
            "Stop",
        ] {
            assert!(
                texts.iter().any(|t| t.contains(want)),
                "missing {want}: {texts:?}"
            );
        }
        assert!(texts.iter().any(|t| t.contains(&app.stations[0].name)));
    }

    #[test]
    fn clicks_hit_buttons_volume_bar_and_rows() {
        let mut vfs = make_vfs();
        write_status(&mut vfs, "State: playing\nVolume: 50%\n");
        let mut app = RadioApp::new("/apps/radio", &vfs);
        let (w, h) = (400, 240);
        let l = RadioLayout::compute(0, 0, w, h);
        let click = |app: &mut RadioApp, x: i32, y: i32| {
            app.handle_click(x, y, w, h, false);
        };

        let stop = l.buttons[2];
        click(&mut app, stop.x + 2, stop.y + 2);
        assert_eq!(take(&mut app).as_deref(), Some("stop"));

        let up = l.buttons[1];
        click(&mut app, up.x + 2, up.y + 2);
        assert_eq!(take(&mut app).as_deref(), Some("vol 60"));

        let bar = l.volume_bar;
        click(&mut app, bar.x, bar.y + 1);
        assert_eq!(take(&mut app).as_deref(), Some("vol 0"));

        let row = l.row_rect(1);
        click(&mut app, row.x + 4, row.y + 2);
        assert_eq!(app.selected, 1);
        assert_eq!(take(&mut app).as_deref(), Some("tune 1"));

        // The panel text area does nothing.
        click(&mut app, l.panel.x + 4, l.panel.y + 4);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn keyboard_selection_scrolls_windowed_list() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.visible_rows.set(2);
        let n = app.stations.len();
        for _ in 0..n {
            app.handle_input(&Button::Down, &vfs);
        }
        assert_eq!(app.selected, n - 1);
        assert!(app.list_scroll + 2 > app.selected);
        assert!(app.list_scroll <= app.selected);
    }

    #[test]
    fn radio_no_browse_dir() {
        let vfs = make_vfs();
        let app = RadioApp::new("/apps/radio", &vfs);
        assert!(app.browse_dir().is_none());
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn radio_downcast() {
        let vfs = make_vfs();
        let app = RadioApp::new("/apps/radio", &vfs);
        assert!(app.as_any().downcast_ref::<RadioApp>().is_some());
    }
}
