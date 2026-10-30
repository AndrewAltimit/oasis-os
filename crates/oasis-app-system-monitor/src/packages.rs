//! Package Manager app.
//!
//! Read-only listing of what this build ships: the OASIS_OS core and the
//! host's rendering backend, both at the workspace version. There is no
//! package source or install path yet; the screen says so instead of
//! listing made-up packages.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// Package Manager app.
#[derive(Debug)]
pub struct PackageManagerApp {
    content: ContentState,
}

impl PackageManagerApp {
    /// Create the app for a host rendering through `backend` (e.g.
    /// `"SDL3"`, `"WASM Canvas 2D"`).
    pub fn new(path: &str, backend: &str) -> Self {
        let version = env!("CARGO_PKG_VERSION");
        let mut content = ContentState::new("Package Manager", path);
        content.lines = vec![
            "Package Manager".to_string(),
            String::new(),
            "Installed packages:".to_string(),
            format!("  oasis-core      {version}  (system)"),
            format!("  {backend:<15} {version}  (backend)"),
            String::new(),
            "No package sources configured.".to_string(),
        ];
        Self { content }
    }
}

impl App for PackageManagerApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        self.content.handle_nav(button).unwrap_or(AppAction::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_app_core::AsAny;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn title_and_path() {
        let app = PackageManagerApp::new("/apps/pkgmgr", "SDL3");
        assert_eq!(app.title(), "Package Manager");
        assert_eq!(app.path(), "/apps/pkgmgr");
    }

    #[test]
    fn lists_core_and_the_host_backend() {
        let app = PackageManagerApp::new("/apps/pkgmgr", "WASM");
        assert!(app.lines().iter().any(|l| l.contains("oasis-core")));
        assert!(app.lines().iter().any(|l| l.starts_with("  WASM ")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = PackageManagerApp::new("/apps/pkgmgr", "SDL3");
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = PackageManagerApp::new("/apps/pkgmgr", "SDL3");
        assert!(app.as_any().downcast_ref::<PackageManagerApp>().is_some());
    }
}
