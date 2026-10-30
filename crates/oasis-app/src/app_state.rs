use std::sync::mpsc;

use oasis_audio::RadioManager;
use oasis_audio::radio::archive::ArchiveCatalog;
use oasis_audio::radio::source::RadioSource;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::backend::AudioTrackId;
use oasis_core::backend::Color;
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::config::OasisConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::DashboardState;
use oasis_core::net::{RemoteClient, RemoteListener, RustlsTlsProvider, StdNetworkBackend};
use oasis_core::osk::OskState;
use oasis_core::platform::DesktopPlatform;
use oasis_core::plugin::PluginManager;
use oasis_core::skin::Skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{CommandRegistry, ShellSession};
use oasis_core::toast::ToastManager;
use oasis_core::transfer::FtpServer;
use oasis_core::transition;
use oasis_core::wm::DesktopManager;
use oasis_core::wm::manager::WindowManager;

/// Result of a background catalog fetch (catalog + first connected track).
pub struct CatalogFetchResult {
    pub catalog: ArchiveCatalog,
    pub source: Box<dyn RadioSource + Send>,
}

/// Result of a background single-track fetch.
pub struct TrackFetchResult {
    pub source: Box<dyn RadioSource + Send>,
}

/// The UI modes the desktop shell supports. Apps always run in windows
/// (a "fullscreen" app is a kiosk window, see
/// [`ContentLayer::fullscreen_app`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dashboard,
    Terminal,
    Osk,
    Desktop,
}

/// Dashboard, status/bottom bars, taskbar, start menu, and mouse cursor.
pub struct UiLayer {
    pub dashboard: DashboardState,
    pub status_bar: StatusBar,
    pub bottom_bar: BottomBar,
    pub taskbar: oasis_core::taskbar::Taskbar,
    pub start_menu: StartMenuState,
    pub mouse_cursor: CursorState,
    /// Virtual desktop manager for desktop switching.
    pub desktops: DesktopManager,
}

/// Terminal/shell state: command registry, CWD, I/O buffers.
pub struct TerminalLayer {
    pub cmd_reg: CommandRegistry,
    pub cwd: String,
    /// Input line editor, tab completion and persistent history.
    pub session: ShellSession,
    pub output_lines: Vec<String>,
    pub scroll_offset: usize,
    /// Set when output_lines or the input line changes; cleared after sync.
    pub dirty: bool,
    /// Signature of the content last synced to the windowed terminal
    /// runner: (lines len, scroll offset, input buffer, first line, last
    /// line). `dirty` is set on *any* input event as a catch-all, so
    /// without this a mouse drag over the desktop would deep-clone the
    /// whole scrollback every frame. First+last+len covers append,
    /// cap-trim (`remove(0)`), and `clear`.
    pub sync_signature: Option<(usize, usize, String, String, String)>,
    /// Hash of everything `setup_terminal_objects` renders from (visible
    /// lines, prompt, scroll, blink state, theme geometry/colors). The
    /// Terminal-mode SDI rebuild is skipped while it is unchanged; reset
    /// to `None` whenever the mode is not Terminal so re-entry (and any
    /// theme edit made in another mode) forces a rebuild.
    pub sdi_signature: Option<u64>,
}

impl TerminalLayer {
    /// A terminal at `/` with `output_lines` already in its scrollback.
    pub fn new(cmd_reg: CommandRegistry, output_lines: Vec<String>) -> Self {
        Self {
            cmd_reg,
            cwd: "/".to_string(),
            session: ShellSession::new(),
            output_lines,
            scroll_offset: 0,
            dirty: true,
            sync_signature: None,
            sdi_signature: None,
        }
    }

    /// Append one line to the scrollback.
    pub fn print(&mut self, line: impl Into<String>) {
        self.output_lines.push(line.into());
    }
}

/// Networking: TCP backends, remote listener/client, FTP, TLS.
///
/// A `StdNetworkBackend` holds at most one listening socket, so every
/// server gets its own backend: sharing one made `ftp start` replace the
/// remote terminal's socket (both servers then accepted each other's
/// clients).
pub struct NetworkLayer {
    /// Outbound connections (`remote` client).
    pub backend: StdNetworkBackend,
    /// Listening socket of the remote terminal (`listen`).
    pub listener_backend: StdNetworkBackend,
    /// Listening socket of the FTP server (`ftp start`).
    pub ftp_backend: StdNetworkBackend,
    pub listener: Option<RemoteListener>,
    pub ftp_server: Option<FtpServer>,
    pub remote_client: Option<RemoteClient>,
    pub tls_provider: RustlsTlsProvider,
}

impl NetworkLayer {
    /// No servers running, no remote connection.
    pub fn new() -> Self {
        Self {
            backend: StdNetworkBackend::with_tls(RustlsTlsProvider::new()),
            listener_backend: StdNetworkBackend::new(),
            ftp_backend: StdNetworkBackend::new(),
            listener: None,
            ftp_server: None,
            remote_client: None,
            tls_provider: RustlsTlsProvider::new(),
        }
    }
}

impl Default for NetworkLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Running applications: windowed app runners, browser, kiosk state.
#[derive(Default)]
pub struct ContentLayer {
    /// App runners by window id (the browser window has none).
    pub open_runners: Vec<(String, AppRunner)>,
    pub browser: Option<BrowserWidget>,
    /// Window id of the app in kiosk fullscreen, if any.
    pub fullscreen_app: Option<String>,
    /// Runners of apps closed this frame, waiting for the shell to release
    /// their backend resources (`AppRunner::release_resources`) before
    /// they are dropped. Input handlers have no backend access.
    pub retired_runners: Vec<AppRunner>,
}

/// Internet Radio playback (driven by `radio_controller`).
pub struct RadioLayer {
    pub manager: RadioManager,
    pub source: Option<Box<dyn RadioSource>>,
    pub archive_catalog: Option<ArchiveCatalog>,
    pub pending_catalog_fetch: Option<mpsc::Receiver<Result<CatalogFetchResult, String>>>,
    pub pending_source_fetch: Option<mpsc::Receiver<Result<TrackFetchResult, String>>>,
    /// When the radio status file was last published (see
    /// `radio_controller::STATUS_INTERVAL`).
    pub last_status_publish: Option<std::time::Instant>,
}

impl RadioLayer {
    /// Nothing playing, no stations loaded.
    pub fn new() -> Self {
        Self {
            manager: RadioManager::new(),
            source: None,
            archive_catalog: None,
            pending_catalog_fetch: None,
            pending_source_fetch: None,
            last_status_publish: None,
        }
    }
}

impl Default for RadioLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pending TV catalog fetch: per-channel catalogs (or an error).
pub type TvCatalogFetch =
    mpsc::Receiver<Result<Vec<Option<oasis_core::apps::tv_guide::ChannelCatalog>>, String>>;

/// TV Guide playback (driven by `tv_controller`).
pub struct TvLayer {
    pub pending_catalog_fetch: Option<TvCatalogFetch>,
    /// When the TV catalog fetch thread was spawned (for timeout detection).
    pub fetch_start: Option<std::time::Instant>,
    /// In-app video player (ffmpeg subprocess or software decode) for TV Guide preview.
    pub video_player: crate::video_player::VideoPlayer,
    /// Audio track for TV Guide video playback.
    pub audio_track: Option<AudioTrackId>,
    /// Diagnostic: total audio chunks fed to the backend since last tune.
    pub audio_chunks_fed: u64,
    /// Diagnostic: total audio samples fed to the backend since last tune.
    pub audio_samples_fed: u64,
    /// Pending video file download (software decode path).
    #[cfg(feature = "_video")]
    pub pending_video_download: Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// Cached video file path for cleanup on untune.
    #[cfg(feature = "_video")]
    pub video_cache_path: Option<std::path::PathBuf>,
    /// Video file cache: (URL, file path) pairs, FIFO eviction at 3 entries.
    #[cfg(feature = "_video")]
    pub video_cache: Vec<(String, std::path::PathBuf)>,
    /// Download progress: (bytes downloaded, total bytes). Updated atomically
    /// from the download thread.
    #[cfg(feature = "_video")]
    pub download_progress:
        Option<std::sync::Arc<(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>>,
    /// Current streaming session — cancelled on re-tune to abort orphaned
    /// download + decoder threads.
    #[cfg(feature = "_video")]
    pub stream_session: Option<std::sync::Arc<crate::tv_controller::StreamingInner>>,
    /// URL currently being played — used to deduplicate tune requests.
    #[cfg(feature = "_video")]
    pub current_url: Option<String>,
}

impl TvLayer {
    /// Nothing tuned, no fetch in flight.
    pub fn new() -> Self {
        Self {
            pending_catalog_fetch: None,
            fetch_start: None,
            video_player: crate::video_player::VideoPlayer::new(),
            audio_track: None,
            audio_chunks_fed: 0,
            audio_samples_fed: 0,
            #[cfg(feature = "_video")]
            pending_video_download: None,
            #[cfg(feature = "_video")]
            video_cache_path: None,
            #[cfg(feature = "_video")]
            video_cache: Vec::new(),
            #[cfg(feature = "_video")]
            download_progress: None,
            #[cfg(feature = "_video")]
            stream_session: None,
            #[cfg(feature = "_video")]
            current_url: None,
        }
    }
}

impl Default for TvLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// All mutable application state except `backend`, `sdi`, and `vfs`
/// (which stay as separate local variables in main() for borrow-splitting).
pub struct AppState {
    pub config: OasisConfig,
    pub skin: Skin,
    pub active_theme: ActiveTheme,
    pub browser_config: BrowserConfig,
    pub platform: DesktopPlatform,
    pub ui: UiLayer,
    pub terminal: TerminalLayer,
    pub net: NetworkLayer,
    pub content: ContentLayer,
    pub osk: Option<OskState>,
    pub plugin_manager: PluginManager,
    pub wm: WindowManager,
    pub mode: Mode,
    pub bg_color: Color,
    pub active_transition: Option<transition::TransitionState>,
    pub frame_counter: u64,
    /// Set by `apply_skin_swap` when the wallpaper texture needs to be
    /// regenerated against the new skin's theme. Consumed by the main loop
    /// (which holds the backend needed to upload textures) and cleared.
    /// Also triggers re-upload of skin layout textures and image layers.
    pub pending_wallpaper_refresh: bool,
    /// Backend textures uploaded for the current skin's layout
    /// `texture =` references. Destroyed and re-uploaded on skin swap.
    pub skin_layout_textures: Vec<oasis_core::backend::TextureId>,
    /// Image background layers (watermark decals) for the current skin.
    pub image_layers: Vec<oasis_core::image_layers::ImageLayerObject>,
    /// Cached ops for static `background_layers` (perf item D4).
    /// Invalidated on skin swap / resolution change.
    pub background_layer_cache: oasis_core::vector_overlay::LayerOpsCache,
    /// Cached ops for static `chrome_layers` (perf item D4).
    pub chrome_layer_cache: oasis_core::vector_overlay::LayerOpsCache,
    /// Armed or active desktop-icon drag (free icon layout only).
    pub icon_drag: Option<crate::icon_drag::IconDrag>,
    /// Software cursor texture (only when `features.software_cursor`).
    /// Destroyed and re-uploaded on skin swap.
    pub cursor_texture: Option<oasis_core::backend::TextureId>,
    /// Persistent key-value settings (icon positions, ...), stored in the
    /// VFS at `/system/settings.toml`.
    pub settings: oasis_core::settings::SettingsStore,
    pub radio: RadioLayer,
    pub tv: TvLayer,
    /// Audio output: the SDL device in the desktop binary, a recording
    /// fake in the headless e2e harness.
    pub audio_backend: Box<dyn crate::audio_out::ShellAudio>,
    /// Never start network fetches (TV catalogs / video downloads, radio
    /// streams). Set by the headless harness so scenarios are hermetic
    /// and deterministic; always `false` in the desktop binary.
    pub offline: bool,
    /// Directory "Save as custom skin" writes skins into (`skins/` next
    /// to the binary's working directory; a temp dir in the headless
    /// harness so tests never write into the repository).
    pub custom_skin_root: std::path::PathBuf,
    pub toasts: ToastManager,
    /// UI sound events queued by input/toast chokepoints this frame,
    /// drained once per frame by `ui_sfx::tick`.
    pub ui_sounds: oasis_core::ui_sound::UiSoundQueue,
    /// Skin-defined one-shot UI samples (loaded on skin swap; empty for
    /// skins without a `[sounds]` table).
    pub sfx: oasis_audio::sfx::SfxPlayer,
    /// Audio track for the Music Player app.
    pub media_track: Option<AudioTrackId>,
    /// Music playlist + playback state behind `/var/audio/*` (driven by
    /// `media_controller`).
    pub media: oasis_audio::AudioManager,
    /// Optional MCP control server bound to loopback. `None` unless started
    /// via `OASIS_MCP=1` or the `mcp-server start` command.
    #[cfg(feature = "mcp")]
    pub mcp: Option<oasis_mcp::McpServer>,
    /// Rolling agent-activity record for the in-UI assistant overlay.
    #[cfg(feature = "mcp")]
    pub agent_activity: crate::mcp_tools::AgentActivity,
}

#[cfg(test)]
impl AppState {
    /// A dashboard-mode state on the `classic` skin at 480x272 with no
    /// apps, an empty command registry, a silent audio backend and
    /// networking offline. Unit-test fixture.
    pub(crate) fn new_for_test() -> Self {
        use oasis_core::dashboard::DashboardConfig;
        use oasis_core::skin::SkinFeatures;
        use oasis_core::skin::builtin::load_builtin;

        let skin = load_builtin("classic").expect("built-in classic skin");
        let active_theme = ActiveTheme::from_skin(&skin.theme);
        let dash_cfg = DashboardConfig::from_features(&SkinFeatures::default(), &active_theme);
        Self {
            config: OasisConfig::default(),
            skin,
            active_theme: active_theme.clone(),
            browser_config: BrowserConfig::default(),
            platform: DesktopPlatform::new(),
            ui: UiLayer {
                dashboard: DashboardState::new(dash_cfg, vec![]),
                status_bar: StatusBar::new(),
                bottom_bar: BottomBar::new(),
                taskbar: oasis_core::taskbar::Taskbar::new(),
                start_menu: StartMenuState::new(StartMenuState::default_items(&active_theme)),
                mouse_cursor: CursorState::default(),
                desktops: DesktopManager::new(1),
            },
            terminal: TerminalLayer::new(CommandRegistry::new(), Vec::new()),
            net: NetworkLayer::new(),
            content: ContentLayer::default(),
            osk: None,
            plugin_manager: PluginManager::new(),
            wm: WindowManager::new(480, 272),
            mode: Mode::Dashboard,
            bg_color: Color::rgb(0, 0, 0),
            active_transition: None,
            frame_counter: 0,
            pending_wallpaper_refresh: false,
            skin_layout_textures: Vec::new(),
            image_layers: Vec::new(),
            background_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            chrome_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            icon_drag: None,
            cursor_texture: None,
            settings: oasis_core::settings::SettingsStore::new(),
            radio: RadioLayer::new(),
            tv: TvLayer::new(),
            audio_backend: Box::new(oasis_backend_sdl::SdlAudioBackend::new()),
            offline: true,
            custom_skin_root: std::env::temp_dir().join("oasis-unit-skins"),
            toasts: ToastManager::new(),
            ui_sounds: oasis_core::ui_sound::UiSoundQueue::new(),
            sfx: oasis_audio::sfx::SfxPlayer::new(),
            media_track: None,
            media: oasis_audio::AudioManager::new(),
            #[cfg(feature = "mcp")]
            mcp: None,
            #[cfg(feature = "mcp")]
            agent_activity: crate::mcp_tools::AgentActivity::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_print_appends_lines() {
        let mut term = TerminalLayer::new(CommandRegistry::new(), vec!["boot".into()]);
        term.print("one");
        term.print(format!("two {}", 2));
        assert_eq!(term.output_lines, ["boot", "one", "two 2"]);
        assert_eq!(term.cwd, "/");
    }

    #[test]
    fn test_fixture_starts_on_the_idle_dashboard() {
        let state = AppState::new_for_test();
        assert_eq!(state.mode, Mode::Dashboard);
        assert!(state.content.open_runners.is_empty());
        assert!(state.offline);
        assert!(state.radio.source.is_none());
        assert!(!state.tv.video_player.is_active());
    }
}
