use std::sync::mpsc;

use oasis_audio::RadioManager;
use oasis_audio::radio::archive::ArchiveCatalog;
use oasis_audio::radio::source::RadioSource;
use oasis_backend_sdl::SdlAudioBackend;
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
use oasis_core::terminal::CommandRegistry;
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

/// The UI modes the app supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dashboard,
    Terminal,
    #[allow(dead_code)]
    App,
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
    pub input_buf: String,
    pub output_lines: Vec<String>,
    pub scroll_offset: usize,
    /// Set when output_lines or input_buf changes; cleared after sync.
    pub dirty: bool,
    /// Signature of the content last synced to the windowed terminal
    /// runner: (lines len, scroll offset, input buffer, first line, last
    /// line). `dirty` is set on *any* input event as a catch-all, so
    /// without this a mouse drag over the desktop would deep-clone the
    /// whole scrollback every frame. First+last+len covers append,
    /// cap-trim (`remove(0)`), and `clear`.
    pub sync_signature: Option<(usize, usize, String, String, String)>,
}

/// Networking: TCP backend, remote listener/client, FTP, TLS.
pub struct NetworkLayer {
    pub backend: StdNetworkBackend,
    pub listener: Option<RemoteListener>,
    pub ftp_server: Option<FtpServer>,
    pub remote_client: Option<RemoteClient>,
    pub tls_provider: RustlsTlsProvider,
}

/// Running applications: app runners, browser, fullscreen state.
pub struct ContentLayer {
    pub app_runner: Option<AppRunner>,
    pub open_runners: Vec<(String, AppRunner)>,
    pub browser: Option<BrowserWidget>,
    pub fullscreen_app: Option<String>,
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
    pub radio_manager: RadioManager,
    pub radio_source: Option<Box<dyn RadioSource>>,
    pub archive_catalog: Option<ArchiveCatalog>,
    pub pending_catalog_fetch: Option<mpsc::Receiver<Result<CatalogFetchResult, String>>>,
    pub pending_source_fetch: Option<mpsc::Receiver<Result<TrackFetchResult, String>>>,
    pub audio_backend: SdlAudioBackend,
    pub toasts: ToastManager,
    /// UI sound events queued by input/toast chokepoints this frame,
    /// drained once per frame by `ui_sfx::tick`.
    pub ui_sounds: oasis_core::ui_sound::UiSoundQueue,
    /// Skin-defined one-shot UI samples (loaded on skin swap; empty for
    /// skins without a `[sounds]` table).
    pub sfx: oasis_audio::sfx::SfxPlayer,
    pub pending_tv_catalog_fetch: Option<
        mpsc::Receiver<Result<Vec<Option<oasis_core::apps::tv_guide::ChannelCatalog>>, String>>,
    >,
    /// When the TV catalog fetch thread was spawned (for timeout detection).
    pub tv_fetch_start: Option<std::time::Instant>,
    /// In-app video player (ffmpeg subprocess or software decode) for TV Guide preview.
    pub video_player: crate::video_player::VideoPlayer,
    /// Audio track for TV Guide video playback.
    pub tv_audio_track: Option<AudioTrackId>,
    /// Audio track for the Music Player app.
    pub media_track: Option<AudioTrackId>,
    /// Diagnostic: total audio chunks fed to the backend since last tune.
    pub tv_audio_chunks_fed: u64,
    /// Diagnostic: total audio samples fed to the backend since last tune.
    pub tv_audio_samples_fed: u64,
    /// Pending video file download (software decode path).
    #[cfg(feature = "_video")]
    pub pending_video_download: Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// Cached video file path for cleanup on untune.
    #[cfg(feature = "_video")]
    pub tv_video_cache_path: Option<std::path::PathBuf>,
    /// Video file cache: (URL, file path) pairs, FIFO eviction at 3 entries.
    #[cfg(feature = "_video")]
    pub tv_video_cache: Vec<(String, std::path::PathBuf)>,
    /// Parameters saved from tune request, needed when download completes.
    #[cfg(feature = "_video")]
    pub pending_video_params: Option<PendingVideoParams>,
    /// Download progress: (bytes downloaded, total bytes). Updated atomically
    /// from the download thread.
    #[cfg(feature = "_video")]
    pub tv_download_progress:
        Option<std::sync::Arc<(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64)>>,
    /// Current streaming session — cancelled on re-tune to abort orphaned
    /// download + decoder threads.
    #[cfg(feature = "_video")]
    pub tv_stream_session: Option<std::sync::Arc<crate::tv_controller::StreamingInner>>,
    /// URL currently being played — used to deduplicate tune requests.
    #[cfg(feature = "_video")]
    pub tv_current_url: Option<String>,
}

/// Parameters stashed from a tune request so they survive until download completes.
/// Retained for potential future cache-miss fallback paths.
#[cfg(feature = "_video")]
#[allow(dead_code)]
pub struct PendingVideoParams {
    pub url: String,
    pub seek_secs: u64,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_variants_exist() {
        let _dashboard = Mode::Dashboard;
        let _terminal = Mode::Terminal;
        let _app = Mode::App;
        let _osk = Mode::Osk;
        let _desktop = Mode::Desktop;
    }

    #[test]
    fn test_mode_equality() {
        assert_eq!(Mode::Dashboard, Mode::Dashboard);
        assert_eq!(Mode::Terminal, Mode::Terminal);
        assert_eq!(Mode::App, Mode::App);
        assert_eq!(Mode::Osk, Mode::Osk);
        assert_eq!(Mode::Desktop, Mode::Desktop);

        assert_ne!(Mode::Dashboard, Mode::Terminal);
        assert_ne!(Mode::Terminal, Mode::App);
        assert_ne!(Mode::App, Mode::Osk);
        assert_ne!(Mode::Osk, Mode::Desktop);
    }

    #[test]
    fn test_mode_clone() {
        let mode = Mode::Dashboard;
        let cloned = mode;
        assert_eq!(mode, cloned);
    }

    #[test]
    fn test_mode_copy() {
        let mode = Mode::Terminal;
        let copied = mode;
        assert_eq!(mode, Mode::Terminal);
        assert_eq!(copied, Mode::Terminal);
    }

    #[test]
    fn test_mode_debug() {
        assert_eq!(format!("{:?}", Mode::Dashboard), "Dashboard");
        assert_eq!(format!("{:?}", Mode::Terminal), "Terminal");
        assert_eq!(format!("{:?}", Mode::App), "App");
        assert_eq!(format!("{:?}", Mode::Osk), "Osk");
        assert_eq!(format!("{:?}", Mode::Desktop), "Desktop");
    }

    #[test]
    fn test_mode_pattern_matching() {
        let mode = Mode::Dashboard;
        match mode {
            Mode::Dashboard => {},
            _ => panic!("Expected Dashboard"),
        }

        let mode = Mode::Terminal;
        match mode {
            Mode::Terminal => {},
            _ => panic!("Expected Terminal"),
        }
    }

    #[test]
    fn test_layer_structs_constructible() {
        use oasis_core::dashboard::{DashboardConfig, DashboardState};
        use oasis_core::skin::SkinFeatures;

        let at = ActiveTheme::default();
        let dash_cfg = DashboardConfig::from_features(&SkinFeatures::default(), &at);

        let _ui = UiLayer {
            dashboard: DashboardState::new(dash_cfg, vec![]),
            status_bar: StatusBar::new(),
            bottom_bar: BottomBar::new(),
            taskbar: oasis_core::taskbar::Taskbar::new(),
            start_menu: StartMenuState::new(StartMenuState::default_items(&at)),
            mouse_cursor: CursorState::default(),
            desktops: DesktopManager::new(1),
        };

        let _terminal = TerminalLayer {
            cmd_reg: CommandRegistry::new(),
            cwd: "/".to_string(),
            input_buf: String::new(),
            output_lines: Vec::new(),
            scroll_offset: 0,
            dirty: true,
            sync_signature: None,
        };

        let _net = NetworkLayer {
            backend: StdNetworkBackend::new(),
            listener: None,
            ftp_server: None,
            remote_client: None,
            tls_provider: RustlsTlsProvider::new(),
        };

        let _content = ContentLayer {
            app_runner: None,
            open_runners: Vec::new(),
            browser: None,
            fullscreen_app: None,
        };
    }
}
