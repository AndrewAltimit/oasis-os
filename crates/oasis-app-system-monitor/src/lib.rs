//! System Monitor app for OASIS_OS.
//!
//! Read-only screen showing platform identification, active backend, VFS
//! type, and uptime. CPU / memory / battery slots are placeholders until
//! the platform service traits in `oasis-platform` provide live values.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// System Monitor app.
#[derive(Debug)]
pub struct SystemMonitorApp {
    content: ContentState,
}

impl SystemMonitorApp {
    /// Create a new System Monitor app for `platform` running on `backend`,
    /// using `vfs_type` (e.g. `"MemoryVfs"`, `"RealVfs"`, `"GameAssetVfs"`)
    /// and the given uptime in seconds.
    pub fn new(
        path: &str,
        platform: &str,
        backend: &str,
        vfs_type: &str,
        uptime_secs: u64,
    ) -> Self {
        let h = uptime_secs / 3600;
        let m = (uptime_secs % 3600) / 60;
        let s = uptime_secs % 60;
        let mut content = ContentState::new("System Monitor", path);
        content.lines = vec![
            "System Monitor".to_string(),
            String::new(),
            format!("  Platform:   {platform}"),
            format!("  Backend:    {backend}"),
            format!("  VFS:        {vfs_type}"),
            format!("  Uptime:     {h}:{m:02}:{s:02}"),
            String::new(),
            "  CPU:        --".to_string(),
            "  Memory:     --".to_string(),
            "  Battery:    N/A (desktop)".to_string(),
        ];
        Self { content }
    }
}

impl App for SystemMonitorApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
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
            _ => AppAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn title_and_path() {
        let app = SystemMonitorApp::new("/apps/sysmon", "Desktop (SDL3)", "SDL3", "MemoryVfs", 0);
        assert_eq!(app.title(), "System Monitor");
        assert_eq!(app.path(), "/apps/sysmon");
    }

    #[test]
    fn lines_contain_platform_and_backend() {
        let app = SystemMonitorApp::new("/apps/sysmon", "Desktop (SDL3)", "SDL3", "MemoryVfs", 0);
        assert!(app.lines().iter().any(|l| l.contains("Desktop (SDL3)")));
        assert!(app.lines().iter().any(|l| l.contains("SDL3")));
    }

    #[test]
    fn lines_show_passed_vfs_type() {
        let app = SystemMonitorApp::new("/apps/sysmon", "UE5", "UE5", "GameAssetVfs", 0);
        assert!(app.lines().iter().any(|l| l.contains("GameAssetVfs")));
    }

    #[test]
    fn uptime_formatted_as_hms() {
        let app = SystemMonitorApp::new("/apps/sysmon", "Desktop", "SDL3", "MemoryVfs", 3661);
        // 3661s = 1h 1m 1s
        assert!(app.lines().iter().any(|l| l.contains("1:01:01")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = SystemMonitorApp::new("/apps/sysmon", "Desktop", "SDL3", "MemoryVfs", 0);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = SystemMonitorApp::new("/apps/sysmon", "Desktop", "SDL3", "MemoryVfs", 0);
        let any = app.as_any();
        assert!(any.downcast_ref::<SystemMonitorApp>().is_some());
    }
}
