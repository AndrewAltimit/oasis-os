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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_variants_exist() {
        // Ensure all Mode variants can be constructed.
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
        // Both should still be usable after copy.
        assert_eq!(mode, Mode::Terminal);
        assert_eq!(copied, Mode::Terminal);
    }

    #[test]
    fn test_mode_debug() {
        // Ensure Debug formatting works for all variants.
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
}
