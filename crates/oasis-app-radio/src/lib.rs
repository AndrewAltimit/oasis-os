//! Internet Radio app for OASIS_OS.
//!
//! Displays station list from VFS config, shows playback status,
//! and sends tune/favorite requests via VFS IPC.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_audio::radio::station::StationRegistry;
use oasis_audio::{RADIO_REQUEST_PATH, RADIO_STATUS_PATH};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// Number of header lines before the station list begins.
const STATION_HEADER_LINES: usize = 7;

/// Internet Radio app implementing the `App` trait.
#[derive(Debug)]
pub struct RadioApp {
    content: ContentState,
}

impl RadioApp {
    /// Create a new Internet Radio app, loading initial content from VFS.
    pub fn new(path: &str, vfs: &dyn Vfs) -> Self {
        let mut content = ContentState::new("Internet Radio", path);
        content.lines = radio_content(vfs);
        Self { content }
    }
}

impl App for RadioApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        let station_count = self
            .content
            .lines
            .len()
            .saturating_sub(STATION_HEADER_LINES + 2);

        match button {
            Button::Cancel => AppAction::Exit,
            Button::Up => {
                self.content.navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.content.navigate_down();
                AppAction::None
            },
            Button::Confirm => {
                let abs_idx = self.content.scroll + self.content.cursor;
                if abs_idx >= STATION_HEADER_LINES && abs_idx < STATION_HEADER_LINES + station_count
                {
                    let station_idx = abs_idx - STATION_HEADER_LINES;
                    self.content.pending_vfs_request = Some((
                        RADIO_REQUEST_PATH.to_string(),
                        format!("tune {station_idx}"),
                    ));
                }
                AppAction::None
            },
            Button::Triangle => {
                let abs_idx = self.content.scroll + self.content.cursor;
                if abs_idx >= STATION_HEADER_LINES && abs_idx < STATION_HEADER_LINES + station_count
                {
                    let station_idx = abs_idx - STATION_HEADER_LINES;
                    self.content.pending_vfs_request =
                        Some((RADIO_REQUEST_PATH.to_string(), format!("fav {station_idx}")));
                }
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
        let old_cursor = self.content.cursor;
        let old_scroll = self.content.scroll;
        self.content.lines = radio_content(vfs);
        self.content.cursor = old_cursor;
        self.content.scroll = old_scroll;
    }
}

/// Generate content lines for the Internet Radio display.
fn radio_content(vfs: &dyn Vfs) -> Vec<String> {
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
        let _ = i;
    }

    lines.push(String::new());
    lines.push("Confirm=Tune  Triangle=Fav  Cancel=Exit".to_string());

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
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
        assert!(app.lines().iter().any(|l| l.contains("Internet Radio")));
        assert!(app.lines().iter().any(|l| l.contains("Stations")));
        // Default stations include at least one entry.
        assert!(app.lines().len() > STATION_HEADER_LINES + 2);
    }

    #[test]
    fn radio_cancel_exits() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn radio_navigate_up_down() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.content.cached_max_visible = 20;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.cursor, 1);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.content.cursor, 0);
    }

    #[test]
    fn radio_tune_creates_request() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.content.cached_max_visible = 30;
        // Navigate to first station (after header lines).
        for _ in 0..STATION_HEADER_LINES {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);
        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.unwrap();
        assert_eq!(path, RADIO_REQUEST_PATH);
        assert!(data.starts_with("tune "));
    }

    #[test]
    fn radio_favorite_creates_request() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.content.cached_max_visible = 30;
        for _ in 0..STATION_HEADER_LINES {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Triangle, &vfs);
        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.unwrap();
        assert_eq!(path, RADIO_REQUEST_PATH);
        assert!(data.starts_with("fav "));
    }

    #[test]
    fn radio_confirm_outside_station_list_noop() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        // Cursor at line 0 (header area).
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn radio_refresh_preserves_cursor() {
        let vfs = make_vfs();
        let mut app = RadioApp::new("/apps/radio", &vfs);
        app.content.cached_max_visible = 30;
        app.content.cursor = 3;
        app.content.scroll = 1;
        app.refresh(&vfs);
        assert_eq!(app.content.cursor, 3);
        assert_eq!(app.content.scroll, 1);
    }

    #[test]
    fn radio_reads_status_from_vfs() {
        use oasis_vfs::Vfs;
        let mut vfs = make_vfs();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/radio").unwrap();
        vfs.write(
            RADIO_STATUS_PATH,
            b"State: Playing\nStation: Jazz FM\nNow Playing: Blue Note",
        )
        .unwrap();
        let app = RadioApp::new("/apps/radio", &vfs);
        assert!(app.lines().iter().any(|l| l.contains("Playing")));
        assert!(app.lines().iter().any(|l| l.contains("Jazz FM")));
        assert!(app.lines().iter().any(|l| l.contains("Blue Note")));
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
