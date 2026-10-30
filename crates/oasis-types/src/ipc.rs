//! VFS IPC paths and request formats shared by producers and consumers.
//!
//! Terminal commands, apps and the host shell talk to each other by writing
//! line-based requests to well-known VFS files and reading status files
//! back. Keeping the paths and the request grammar in one place means a
//! producer (the `music` command, the Music Player app, an MCP tool) and the
//! consumer (the desktop shell's media controller) cannot drift apart.

use serde::{Deserialize, Serialize};

/// Requests for the host's music/audio controller ([`MediaRequest`] lines).
pub const AUDIO_REQUEST_PATH: &str = "/var/audio/request";
/// Status published by the host's music/audio controller (`Key: value`
/// lines: `State`, `Volume`, `Track`, `Position`, `Repeat`, `Shuffle`,
/// `Playlist`).
pub const AUDIO_STATUS_PATH: &str = "/var/audio/status";
/// Real playback position of the current music track, written by the host as
/// `<position_ms> <duration_ms> <path>`.
pub const AUDIO_POSITION_PATH: &str = "/var/audio/position";

/// Requests for the host's internet-radio controller.
pub const RADIO_REQUEST_PATH: &str = "/var/radio/request";
/// Status published by the host's internet-radio controller.
pub const RADIO_STATUS_PATH: &str = "/var/radio/status";

/// Requests for the window manager: `close [--force] <id>`, `focus <id>`,
/// `minimize <id>`, `maximize <id>`.
pub const WM_REQUEST_PATH: &str = "/var/wm/request";
/// Window list published by the host, one window per line.
pub const WM_STATUS_PATH: &str = "/var/wm/status";

/// Repeat mode for a playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    /// No repeat -- stop after last track.
    Off,
    /// Repeat the entire playlist.
    All,
    /// Repeat the current track.
    One,
}

impl std::fmt::Display for RepeatMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::All => write!(f, "all"),
            Self::One => write!(f, "one"),
        }
    }
}

impl RepeatMode {
    /// Parse from a string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" | "none" => Some(Self::Off),
            "all" | "playlist" => Some(Self::All),
            "one" | "track" | "single" => Some(Self::One),
            _ => None,
        }
    }
}

/// One request line on [`AUDIO_REQUEST_PATH`].
///
/// The wire format is one request per line: `play_file <path>`, `play`,
/// `pause`, `resume`, `stop`, `next`, `prev`, `vol <0-100>`,
/// `repeat <off|all|one>`, `shuffle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRequest {
    /// Load and play the file at this VFS path (adds it to the playlist).
    PlayFile(String),
    /// Start playback of the current playlist entry, or resume if paused.
    Play,
    /// Pause playback.
    Pause,
    /// Resume paused playback.
    Resume,
    /// Stop playback and release the track.
    Stop,
    /// Skip to the next playlist entry.
    Next,
    /// Go back to the previous playlist entry.
    Prev,
    /// Set the output volume (0-100).
    Volume(u8),
    /// Set the playlist repeat mode.
    Repeat(RepeatMode),
    /// Toggle shuffle.
    Shuffle,
}

impl MediaRequest {
    /// Parse one request line. Returns `None` for blank or unknown lines
    /// and for malformed arguments.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (cmd, arg) = match line.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        let req = match cmd {
            "play_file" if !arg.is_empty() => Self::PlayFile(arg.to_string()),
            "play" if arg.is_empty() => Self::Play,
            "pause" => Self::Pause,
            "resume" => Self::Resume,
            "stop" => Self::Stop,
            "next" => Self::Next,
            "prev" => Self::Prev,
            "vol" => Self::Volume(arg.parse::<u8>().ok().filter(|v| *v <= 100)?),
            "repeat" => Self::Repeat(RepeatMode::parse(arg)?),
            "shuffle" => Self::Shuffle,
            _ => return None,
        };
        Some(req)
    }

    /// Serialize to the request-line wire format ([`MediaRequest::parse`]
    /// round-trips it).
    pub fn to_line(&self) -> String {
        match self {
            Self::PlayFile(path) => format!("play_file {path}"),
            Self::Play => "play".to_string(),
            Self::Pause => "pause".to_string(),
            Self::Resume => "resume".to_string(),
            Self::Stop => "stop".to_string(),
            Self::Next => "next".to_string(),
            Self::Prev => "prev".to_string(),
            Self::Volume(v) => format!("vol {v}"),
            Self::Repeat(mode) => format!("repeat {mode}"),
            Self::Shuffle => "shuffle".to_string(),
        }
    }
}

impl std::fmt::Display for MediaRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_request_round_trips() {
        let all = [
            MediaRequest::PlayFile("/home/user/music/a b.mp3".into()),
            MediaRequest::Play,
            MediaRequest::Pause,
            MediaRequest::Resume,
            MediaRequest::Stop,
            MediaRequest::Next,
            MediaRequest::Prev,
            MediaRequest::Volume(0),
            MediaRequest::Volume(100),
            MediaRequest::Repeat(RepeatMode::Off),
            MediaRequest::Repeat(RepeatMode::All),
            MediaRequest::Repeat(RepeatMode::One),
            MediaRequest::Shuffle,
        ];
        for req in all {
            assert_eq!(MediaRequest::parse(&req.to_line()), Some(req.clone()));
        }
    }

    #[test]
    fn media_request_rejects_malformed() {
        for line in [
            "",
            "   ",
            "bogus",
            "play_file",
            "play_file   ",
            "play extra",
            "vol",
            "vol 101",
            "vol -1",
            "vol abc",
            "repeat",
            "repeat sideways",
        ] {
            assert_eq!(MediaRequest::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn media_request_parse_trims() {
        assert_eq!(
            MediaRequest::parse("  vol 42 \r"),
            Some(MediaRequest::Volume(42))
        );
        assert_eq!(
            MediaRequest::parse("repeat ALL"),
            Some(MediaRequest::Repeat(RepeatMode::All))
        );
    }

    #[test]
    fn repeat_mode_parse_and_display() {
        assert_eq!(RepeatMode::parse("off"), Some(RepeatMode::Off));
        assert_eq!(RepeatMode::parse("ALL"), Some(RepeatMode::All));
        assert_eq!(RepeatMode::parse("track"), Some(RepeatMode::One));
        assert_eq!(RepeatMode::parse("invalid"), None);
        assert_eq!(RepeatMode::One.to_string(), "one");
    }
}
