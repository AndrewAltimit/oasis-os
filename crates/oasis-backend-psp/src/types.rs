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
    pub(crate) url: &'static str,
    pub(crate) bitrate: u32,
}

pub(crate) static RADIO_STATIONS: &[RadioStation] = &[
    RadioStation {
        name: "Drone Zone",
        genre: "ambient",
        url: "http://ice2.somafm.com/dronezone-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "DEF CON Radio",
        genre: "hacker",
        url: "http://ice2.somafm.com/defcon-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Groove Salad",
        genre: "chill",
        url: "http://ice2.somafm.com/groovesalad-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Space Station",
        genre: "space",
        url: "http://ice2.somafm.com/spacestation-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Secret Agent",
        genre: "lounge",
        url: "http://ice2.somafm.com/secretagent-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Lush",
        genre: "female vocal",
        url: "http://ice2.somafm.com/lush-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Metal Detector",
        genre: "metal",
        url: "http://ice2.somafm.com/metal-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Boot Liquor",
        genre: "americana",
        url: "http://ice2.somafm.com/bootliquor-128-mp3",
        bitrate: 128,
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
