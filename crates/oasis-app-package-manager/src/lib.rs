//! Package Manager app for OASIS_OS.
//!
//! Read-only listing of installed crates, plugins, and skins. The pre-launch
//! build does not yet ship a real package fetch/install path — this screen
//! exists so the dashboard slot is reachable and the surface is in place
//! for a future install/update implementation.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// Package Manager app.
#[derive(Debug)]
pub struct PackageManagerApp {
    content: ContentState,
}

impl PackageManagerApp {
    /// Create a new Package Manager app.
    pub fn new(path: &str) -> Self {
        let mut content = ContentState::new("Package Manager", path);
        content.lines = vec![
            "Package Manager".to_string(),
            String::new(),
            "Installed packages:".to_string(),
            "  oasis-core      0.1.0  (system)".to_string(),
            "  oasis-sdl       0.1.0  (backend)".to_string(),
            "  classic-skin    1.0.0  (skin)".to_string(),
            String::new(),
            "No updates available.".to_string(),
        ];
        Self { content }
    }
}

impl App for PackageManagerApp {
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
        let app = PackageManagerApp::new("/apps/pkgmgr");
        assert_eq!(app.title(), "Package Manager");
        assert_eq!(app.path(), "/apps/pkgmgr");
    }

    #[test]
    fn lists_oasis_core() {
        let app = PackageManagerApp::new("/apps/pkgmgr");
        assert!(app.lines().iter().any(|l| l.contains("oasis-core")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = PackageManagerApp::new("/apps/pkgmgr");
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = PackageManagerApp::new("/apps/pkgmgr");
        let any = app.as_any();
        assert!(any.downcast_ref::<PackageManagerApp>().is_some());
    }
}
