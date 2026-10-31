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

/// Requests for the window manager, one [`WmRequest`] per line (producers
/// append, the host applies every line it finds): `close [--force] <id>`,
/// `focus <id>`, `minimize <id>`, `maximize <id>`, `open <app>[\t<file>]`.
pub const WM_REQUEST_PATH: &str = "/var/wm/request";
/// Window list published by the host, one window per line.
pub const WM_STATUS_PATH: &str = "/var/wm/status";

/// Requests for the host's plugin manager, one [`PluginRequest`] per line
/// (producers append, the host applies every line it finds).
pub const PLUGIN_REQUEST_PATH: &str = "/var/plugin/request";
/// Plugin list published by the host, one plugin per line:
/// `<name> (<version>) [registered|active|stopped] <description>`.
pub const PLUGIN_STATUS_PATH: &str = "/var/plugin/status";

// -- UI requests (terminal `notify` / `screenshot` / `theme` / `browse`) --

/// Toast request (a [`Notification`]: `level:message`) from the `notify`
/// command.
pub const NOTIFY_PATH: &str = "/var/notify/message";
/// Screenshot request (a VFS destination path) from the `screenshot`
/// command; the host captures the next presented frame there.
pub const SCREENSHOT_REQUEST_PATH: &str = "/var/screenshot/request";
/// Current theme summary published by the host for the `theme` command.
pub const THEME_STATUS_PATH: &str = "/var/theme/current";
/// Browser request (a [`BrowserRequest`]: `op [arg]`) from the `browse`
/// command.
pub const BROWSER_REQUEST_PATH: &str = "/var/browser/request";

// -- Settings (the Settings app and the host talk through these) --

/// Skin change request. Payload: a skin name.
pub const SKIN_CHANGE_REQUEST_PATH: &str = "/system/ipc/skin-change";
/// In-memory theme preview ("Apply" in the Appearance editor). The payload
/// is a serialized `SkinTheme` TOML document; the shell keeps the current
/// skin's layout/features and swaps only the theme, without writing
/// anything to disk.
pub const SKIN_APPLY_THEME_REQUEST_PATH: &str = "/system/ipc/skin-apply-theme";
/// Save the edited theme as a custom skin. The payload is
/// `<name>\n<theme toml>`; the shell writes `skins/<name>/` in the standard
/// directory format and then swaps to it by name.
pub const SKIN_SAVE_CUSTOM_REQUEST_PATH: &str = "/system/ipc/skin-save-custom";
/// Resolution change request. Payload: `"WIDTHxHEIGHT"` (e.g.
/// `"1280x720"`, see [`parse_resolution`]).
pub const RESOLUTION_CHANGE_REQUEST_PATH: &str = "/system/ipc/resolution-change";
/// Master volume request. Payload: `0`-`100`.
pub const VOLUME_CHANGE_REQUEST_PATH: &str = "/system/ipc/volume";
/// UI locale request. Payload: a locale code (`"en"`, `"de"`, ...).
pub const LOCALE_CHANGE_REQUEST_PATH: &str = "/system/ipc/locale";
/// Font scale request. Payload: a float such as `"1.25"`.
pub const FONT_SCALE_REQUEST_PATH: &str = "/system/ipc/font-scale";
/// Reduced-motion request. Payload: `"1"` or `"0"`.
pub const REDUCED_MOTION_REQUEST_PATH: &str = "/system/ipc/reduced-motion";
/// High-contrast shortcut. Payload: `"on"` (swap to the high-contrast
/// skin, remembering the current one) or `"off"` (swap back).
pub const HIGH_CONTRAST_REQUEST_PATH: &str = "/system/ipc/high-contrast";

/// Active skin name, published by the host.
pub const SKIN_STATE_PATH: &str = "/system/state/skin";
/// Current resolution (`"WxH"`), published by the host.
pub const RESOLUTION_STATE_PATH: &str = "/system/state/resolution";
/// Rendering backend name, published by the host.
pub const BACKEND_STATE_PATH: &str = "/system/state/backend";
/// Master volume (`0`-`100`), published by the host.
pub const VOLUME_STATE_PATH: &str = "/system/state/volume";
/// Selected locale code, published by the host.
pub const LOCALE_STATE_PATH: &str = "/system/state/locale";
/// Font scale, published by the host.
pub const FONT_SCALE_STATE_PATH: &str = "/system/state/font-scale";
/// Reduced-motion switch (`1`/`0`), published by the host.
pub const REDUCED_MOTION_STATE_PATH: &str = "/system/state/reduced-motion";
/// Network service state, published by the host ([`NetState`] lines).
pub const NET_STATE_PATH: &str = "/system/state/net";

// -- TV Guide --

/// TV channel configuration (TOML).
pub const TV_CHANNELS_PATH: &str = "/etc/tv/channels.toml";
/// TV playback requests (`tune_url <url> <seek_secs>`, ...).
pub const TV_REQUEST_PATH: &str = "/var/tv/request";
/// TV playback status.
pub const TV_STATUS_PATH: &str = "/var/tv/status";

/// Parse a `"WIDTHxHEIGHT"` resolution (whitespace around either number is
/// allowed).
pub fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// One request line on [`WM_REQUEST_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WmRequest {
    /// Close window `id`. Without `force` the app may decline (unsaved
    /// changes); with `force` it is closed regardless.
    Close { id: String, force: bool },
    /// Focus window `id`.
    Focus(String),
    /// Minimize window `id`.
    Minimize(String),
    /// Maximize window `id`.
    Maximize(String),
    /// Open the app titled `app` (case-insensitive), optionally with
    /// `file` pre-opened. Wire format: `open <app>` or `open <app>\t<file>`
    /// (app titles contain spaces, so the file is tab-separated).
    Open { app: String, file: Option<String> },
}

impl WmRequest {
    /// Parse one request line. `None` for unknown ops or a missing
    /// argument.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ');
        let (op, rest) = line.split_once(' ')?;
        if op == "open" {
            let (app, file) = match rest.split_once('\t') {
                Some((a, f)) => (a.trim(), Some(f.trim()).filter(|f| !f.is_empty())),
                None => (rest.trim(), None),
            };
            return (!app.is_empty()).then(|| Self::Open {
                app: app.to_string(),
                file: file.map(str::to_string),
            });
        }
        let rest = rest.trim();
        let (force, id) = match rest.strip_prefix("--force") {
            Some(id) if op == "close" && (id.is_empty() || id.starts_with(' ')) => {
                (true, id.trim())
            },
            _ => (false, rest),
        };
        if id.is_empty() {
            return None;
        }
        let id = id.to_string();
        Some(match op {
            "close" => Self::Close { id, force },
            "focus" => Self::Focus(id),
            "minimize" => Self::Minimize(id),
            "maximize" => Self::Maximize(id),
            _ => return None,
        })
    }

    /// Serialize to the request-line wire format ([`WmRequest::parse`]
    /// round-trips it).
    pub fn to_line(&self) -> String {
        match self {
            Self::Close { id, force: true } => format!("close --force {id}"),
            Self::Close { id, force: false } => format!("close {id}"),
            Self::Focus(id) => format!("focus {id}"),
            Self::Minimize(id) => format!("minimize {id}"),
            Self::Maximize(id) => format!("maximize {id}"),
            Self::Open { app, file: None } => format!("open {app}"),
            Self::Open {
                app,
                file: Some(file),
            } => format!("open {app}\t{file}"),
        }
    }
}

/// One request line on [`PLUGIN_REQUEST_PATH`]: `load <name>` /
/// `unload <name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRequest {
    /// Initialize (or re-initialize after an unload) the statically linked
    /// plugin `name`.
    Load(String),
    /// Shut the plugin `name` down (its commands, apps and SDI objects go
    /// with it); it stays known, so `load` brings it back.
    Unload(String),
}

impl PluginRequest {
    /// Parse one request line. `None` for unknown ops or a missing name.
    pub fn parse(line: &str) -> Option<Self> {
        let (op, name) = line.trim().split_once(' ')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        match op {
            "load" => Some(Self::Load(name.to_string())),
            "unload" => Some(Self::Unload(name.to_string())),
            _ => None,
        }
    }

    /// Serialize to the request-line wire format ([`PluginRequest::parse`]
    /// round-trips it).
    pub fn to_line(&self) -> String {
        match self {
            Self::Load(name) => format!("load {name}"),
            Self::Unload(name) => format!("unload {name}"),
        }
    }
}

/// Severity of a [`Notification`] toast.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NotifyLevel {
    /// Informational (the default).
    #[default]
    Info,
    /// An operation succeeded.
    Success,
    /// Something needs attention.
    Warning,
    /// An operation failed.
    Error,
}

impl NotifyLevel {
    /// The wire name (`info`, `success`, `warning`, `error`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Parse a level as a user types it: the wire names plus `warn` and
    /// `err`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "info" => Self::Info,
            "success" => Self::Success,
            "warning" | "warn" => Self::Warning,
            "error" | "err" => Self::Error,
            _ => return None,
        })
    }
}

/// The toast request on [`NOTIFY_PATH`]: `level:message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Toast severity.
    pub level: NotifyLevel,
    /// Toast text.
    pub message: String,
}

impl Notification {
    /// Parse a payload. A payload without a known `level:` prefix is an
    /// [`NotifyLevel::Info`] toast of the whole text (so `note: x` keeps
    /// its colon).
    pub fn parse(payload: &str) -> Self {
        // Only the wire names count as a prefix (not the `warn` / `err`
        // aliases a user may type to the `notify` command).
        let (level, message) = payload
            .split_once(':')
            .and_then(|(prefix, message)| {
                let level = NotifyLevel::parse(prefix).filter(|l| l.as_str() == prefix)?;
                Some((level, message))
            })
            .unwrap_or((NotifyLevel::Info, payload));
        Self {
            level,
            message: message.trim().to_string(),
        }
    }

    /// Serialize to the payload format ([`Notification::parse`]
    /// round-trips it).
    pub fn to_payload(&self) -> String {
        format!("{}:{}", self.level.as_str(), self.message)
    }
}

/// One request on [`BROWSER_REQUEST_PATH`]: `open <url>`,
/// `sandbox <url>`, `back`, `forward`, `reload`, `home`, `reader`,
/// `bookmarks`, `history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserRequest {
    /// Navigate to a URL.
    Open(String),
    /// Navigate to a URL with the page sandboxed (no scripts, no network
    /// beyond the page).
    Sandbox(String),
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload the page.
    Reload,
    /// Go to the home page.
    Home,
    /// Toggle reader mode.
    Reader,
    /// Show the bookmarks page.
    Bookmarks,
    /// Show the history page.
    History,
}

impl BrowserRequest {
    /// Parse one request. `None` for unknown ops and a missing URL.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (op, arg) = match line.split_once(' ') {
            Some((op, arg)) => (op, arg.trim()),
            None => (line, ""),
        };
        Some(match op {
            "open" if !arg.is_empty() => Self::Open(arg.to_string()),
            "sandbox" if !arg.is_empty() => Self::Sandbox(arg.to_string()),
            "back" => Self::Back,
            "forward" => Self::Forward,
            "reload" => Self::Reload,
            "home" => Self::Home,
            "reader" => Self::Reader,
            "bookmarks" => Self::Bookmarks,
            "history" => Self::History,
            _ => return None,
        })
    }

    /// Serialize to the request wire format ([`BrowserRequest::parse`]
    /// round-trips it).
    pub fn to_line(&self) -> String {
        match self {
            Self::Open(url) => format!("open {url}"),
            Self::Sandbox(url) => format!("sandbox {url}"),
            Self::Back => "back".to_string(),
            Self::Forward => "forward".to_string(),
            Self::Reload => "reload".to_string(),
            Self::Home => "home".to_string(),
            Self::Reader => "reader".to_string(),
            Self::Bookmarks => "bookmarks".to_string(),
            Self::History => "history".to_string(),
        }
    }
}

/// Network service state published at [`NET_STATE_PATH`] as `key: value`
/// lines: `listener: <port>|off`, `ftp: <port>|off`, `remote:
/// connected|off`, `mcp: on|off`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetState {
    /// Port of the running remote-terminal listener.
    pub listener_port: Option<u16>,
    /// Port of the running FTP server.
    pub ftp_port: Option<u16>,
    /// Whether an outbound remote-terminal connection is open.
    pub remote_connected: bool,
    /// Whether the MCP control server is running.
    pub mcp_running: bool,
}

impl NetState {
    /// Serialize as `key: value` lines.
    pub fn to_text(&self) -> String {
        fn port(p: Option<u16>) -> String {
            p.map_or_else(|| "off".to_string(), |p| p.to_string())
        }
        format!(
            "listener: {}\nftp: {}\nremote: {}\nmcp: {}\n",
            port(self.listener_port),
            port(self.ftp_port),
            if self.remote_connected {
                "connected"
            } else {
                "off"
            },
            if self.mcp_running { "on" } else { "off" },
        )
    }

    /// Parse [`NetState::to_text`] output. Unknown keys and malformed
    /// values are ignored (the service reads as stopped).
    pub fn parse(text: &str) -> Self {
        let mut st = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "listener" => st.listener_port = value.parse().ok(),
                "ftp" => st.ftp_port = value.parse().ok(),
                "remote" => st.remote_connected = value == "connected",
                "mcp" => st.mcp_running = value == "on",
                _ => {},
            }
        }
        st
    }
}

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
    /// Skip to the next playlist entry (while paused: select it and stay
    /// paused).
    Next,
    /// Go back to the previous playlist entry (while paused: select it and
    /// stay paused).
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
    fn wm_request_round_trips() {
        let all = [
            WmRequest::Close {
                id: "text_editor".into(),
                force: false,
            },
            WmRequest::Close {
                id: "text_editor".into(),
                force: true,
            },
            WmRequest::Focus("terminal".into()),
            WmRequest::Minimize("paint".into()),
            WmRequest::Maximize("paint".into()),
            WmRequest::Open {
                app: "Text Editor".into(),
                file: None,
            },
            WmRequest::Open {
                app: "Photo Viewer".into(),
                file: Some("/home/user/my photo.png".into()),
            },
        ];
        for req in all {
            assert_eq!(WmRequest::parse(&req.to_line()), Some(req.clone()));
        }
    }

    #[test]
    fn wm_request_rejects_malformed() {
        for line in ["", "close", "close ", "open ", "fly away", "close --force "] {
            assert_eq!(WmRequest::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn plugin_request_round_trips_and_rejects_malformed() {
        for req in [
            PluginRequest::Load("clock-widget".into()),
            PluginRequest::Unload("hello".into()),
        ] {
            assert_eq!(PluginRequest::parse(&req.to_line()), Some(req.clone()));
        }
        assert_eq!(
            PluginRequest::parse("  load  notepad \r"),
            Some(PluginRequest::Load("notepad".into()))
        );
        for line in ["", "load", "load ", "unload  ", "reload hello", "hello"] {
            assert_eq!(PluginRequest::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn notification_round_trips_and_defaults_to_info() {
        for level in [
            NotifyLevel::Info,
            NotifyLevel::Success,
            NotifyLevel::Warning,
            NotifyLevel::Error,
        ] {
            let n = Notification {
                level,
                message: "disk: 90% full".into(),
            };
            assert_eq!(Notification::parse(&n.to_payload()), n);
        }
        let plain = Notification::parse("note: no level here");
        assert_eq!(plain.level, NotifyLevel::Info);
        assert_eq!(plain.message, "note: no level here");
        // Aliases are for typing, not a wire prefix.
        assert_eq!(Notification::parse("warn:x").message, "warn:x");
        assert_eq!(NotifyLevel::parse("err"), Some(NotifyLevel::Error));
    }

    #[test]
    fn browser_request_round_trips_and_rejects_malformed() {
        for req in [
            BrowserRequest::Open("https://example.com/a b".into()),
            BrowserRequest::Sandbox("vfs://x".into()),
            BrowserRequest::Back,
            BrowserRequest::Forward,
            BrowserRequest::Reload,
            BrowserRequest::Home,
            BrowserRequest::Reader,
            BrowserRequest::Bookmarks,
            BrowserRequest::History,
        ] {
            assert_eq!(BrowserRequest::parse(&req.to_line()), Some(req.clone()));
        }
        assert_eq!(
            BrowserRequest::parse(" reload\n"),
            Some(BrowserRequest::Reload)
        );
        for line in [
            "",
            "open",
            "open  ",
            "sandbox",
            "fly",
            "https://example.com",
        ] {
            assert_eq!(BrowserRequest::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn net_state_round_trips() {
        let st = NetState {
            listener_port: Some(9000),
            ftp_port: None,
            remote_connected: true,
            mcp_running: true,
        };
        assert_eq!(NetState::parse(&st.to_text()), st);
        assert_eq!(NetState::parse(""), NetState::default());
        assert_eq!(NetState::parse("listener: bogus"), NetState::default());
    }

    #[test]
    fn parse_resolution_accepts_spacing() {
        assert_eq!(parse_resolution("1280x720"), Some((1280, 720)));
        assert_eq!(parse_resolution(" 800 x 600 "), Some((800, 600)));
        assert_eq!(parse_resolution("800"), None);
        assert_eq!(parse_resolution("axb"), None);
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
