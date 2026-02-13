use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::backend::Color;
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::config::OasisConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::DashboardState;
use oasis_core::net::{RemoteClient, RemoteListener, RustlsTlsProvider, StdNetworkBackend};
use oasis_core::osk::OskState;
use oasis_core::platform::DesktopPlatform;
use oasis_core::skin::Skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::CommandRegistry;
use oasis_core::transition;
use oasis_core::wm::manager::WindowManager;

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

/// All mutable application state except `backend`, `sdi`, and `vfs`
/// (which stay as separate local variables in main() for borrow-splitting).
pub struct AppState {
    pub config: OasisConfig,
    pub skin: Skin,
    pub active_theme: ActiveTheme,
    pub browser_config: BrowserConfig,
    pub platform: DesktopPlatform,
    pub dashboard: DashboardState,
    pub status_bar: StatusBar,
    pub bottom_bar: BottomBar,
    pub start_menu: StartMenuState,
    pub cmd_reg: CommandRegistry,
    pub cwd: String,
    pub input_buf: String,
    pub output_lines: Vec<String>,
    pub osk: Option<OskState>,
    pub app_runner: Option<AppRunner>,
    pub wm: WindowManager,
    pub open_runners: Vec<(String, AppRunner)>,
    pub browser: Option<BrowserWidget>,
    pub net_backend: StdNetworkBackend,
    pub listener: Option<RemoteListener>,
    pub remote_client: Option<RemoteClient>,
    pub tls_provider: RustlsTlsProvider,
    pub mouse_cursor: CursorState,
    pub mode: Mode,
    pub bg_color: Color,
    pub active_transition: Option<transition::TransitionState>,
    pub frame_counter: u64,
}
