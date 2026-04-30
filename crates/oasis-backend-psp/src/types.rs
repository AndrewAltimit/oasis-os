//! Data types and static data for the PSP shell.

use oasis_backend_psp::Color;

pub(crate) struct AppEntry {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) color: Color,
}

pub(crate) static APPS: &[AppEntry] = &[
    AppEntry {
        id: "filemgr",
        title: "File Manager",
        color: Color::rgb(70, 130, 180),
    },
    AppEntry {
        id: "settings",
        title: "Settings",
        color: Color::rgb(60, 179, 113),
    },
    AppEntry {
        id: "network",
        title: "Network",
        color: Color::rgb(218, 165, 32),
    },
    AppEntry {
        id: "terminal",
        title: "Terminal",
        color: Color::rgb(178, 102, 178),
    },
    AppEntry {
        id: "music",
        title: "Music Player",
        color: Color::rgb(205, 92, 92),
    },
    AppEntry {
        id: "photos",
        title: "Photo Viewer",
        color: Color::rgb(100, 149, 237),
    },
    AppEntry {
        id: "packages",
        title: "Package Mgr",
        color: Color::rgb(70, 130, 180),
    },
    AppEntry {
        id: "sysmon",
        title: "Sys Monitor",
        color: Color::rgb(60, 179, 113),
    },
    AppEntry {
        id: "browser",
        title: "Browser",
        color: Color::rgb(50, 120, 200),
    },
    AppEntry {
        id: "radio",
        title: "Radio",
        color: Color::rgb(255, 140, 60),
    },
    AppEntry {
        id: "tvguide",
        title: "TV Guide",
        color: Color::rgb(0, 100, 200),
    },
];

// ---------------------------------------------------------------------------
// Kiosk app tracking -- which app (if any) is in full-screen kiosk mode.
// Dashboard is the default state (no kiosk app active).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum KioskApp {
    /// No kiosk app -- dashboard is visible.
    None,
    Terminal,
    FileManager,
    PhotoViewer,
    MusicPlayer,
    Browser,
    Radio,
    TvGuide,
    Settings,
}

impl KioskApp {
    /// The WM window ID for this kiosk app, if any.
    pub(crate) fn window_id(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Terminal => Some("terminal"),
            Self::FileManager => Some("filemgr"),
            Self::PhotoViewer => Some("photos"),
            Self::MusicPlayer => Some("music"),
            Self::Browser => Some("browser"),
            Self::Radio => Some("radio"),
            Self::TvGuide => Some("tvguide"),
            Self::Settings => Some("settings"),
        }
    }

    /// Map a WM window ID to a KioskApp variant.
    pub(crate) fn from_window_id(id: &str) -> Self {
        match id {
            "terminal" => Self::Terminal,
            "filemgr" => Self::FileManager,
            "photos" => Self::PhotoViewer,
            "music" => Self::MusicPlayer,
            "browser" => Self::Browser,
            "radio" => Self::Radio,
            "tvguide" => Self::TvGuide,
            "settings" => Self::Settings,
            _ => Self::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Radio station list and status
// ---------------------------------------------------------------------------

pub(crate) struct RadioStation {
    pub(crate) name: &'static str,
    pub(crate) genre: &'static str,
    /// Direct stream URL for `icecast` stations. Empty for `archive` stations,
    /// where the I/O thread resolves the stream via the Internet Archive APIs
    /// using `collection` instead.
    pub(crate) url: &'static str,
    pub(crate) bitrate: u32,
    /// `"icecast"` for direct MP3 streams, `"archive"` for IA collections.
    /// Mirrors `oasis_audio::Station::source_type` so this list stays in
    /// sync with the desktop / WASM canonical list.
    pub(crate) source_type: &'static str,
    /// Internet Archive collection identifier (only for `archive` stations).
    pub(crate) collection: &'static str,
}

/// Canonical station list shared with the desktop and WASM backends.
/// Mirrors `oasis_audio::StationRegistry::defaults()`. Keep these two in
/// sync — users expect the same playlist on every backend.
pub(crate) static RADIO_STATIONS: &[RadioStation] = &[
    RadioStation {
        name: "Old Time Radio",
        genre: "drama",
        url: "",
        bitrate: 0,
        source_type: "archive",
        collection: "oldtimeradio",
    },
    RadioStation {
        name: "LibriVox Audiobooks",
        genre: "audiobooks",
        url: "",
        bitrate: 0,
        source_type: "archive",
        collection: "librivoxaudio",
    },
    RadioStation {
        name: "Netlabel Music",
        genre: "music",
        url: "",
        bitrate: 0,
        source_type: "archive",
        collection: "netlabels",
    },
    RadioStation {
        name: "78rpm Records",
        genre: "vintage",
        url: "",
        bitrate: 0,
        source_type: "archive",
        collection: "78rpm",
    },
    RadioStation {
        name: "This Is Your FBI",
        genre: "true crime",
        url: "",
        bitrate: 0,
        source_type: "archive",
        collection: "OTRR_This_Is_Your_FBI_Singles",
    },
];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RadioStatus {
    Stopped,
    Connecting,
    Buffering,
    Playing,
    Error,
}
