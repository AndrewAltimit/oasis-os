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
// App modes (Classic = full-screen, Desktop = windowed WM)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AppMode {
    /// Classic PSIX full-screen dashboard (existing behavior, default).
    Classic,
    /// Windowed desktop mode with floating windows managed by WM.
    Desktop,
}

// Classic sub-modes (within AppMode::Classic).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ClassicView {
    Dashboard,
    Terminal,
    FileManager,
    PhotoViewer,
    MusicPlayer,
    Browser,
    Radio,
    TvGuide,
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
