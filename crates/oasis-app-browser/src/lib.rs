//! Browser dashboard placeholder for OASIS_OS.
//!
//! The full embedded browser engine lives in `oasis-browser` and is launched
//! via the dashboard's browser widget. This crate is the static-content
//! placeholder app that explains how to reach it from the launcher screen.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// Browser launcher app — informational screen pointing users at the full
/// browser widget.
#[derive(Debug)]
pub struct BrowserApp {
    content: ContentState,
}

impl BrowserApp {
    /// Create a new Browser launcher app rooted at the given VFS path.
    pub fn new(path: &str) -> Self {
        let mut content = ContentState::new("Browser", path);
        content.lines = vec![
            "Browser".to_string(),
            String::new(),
            "Use the browser widget for web browsing.".to_string(),
            String::new(),
            "The browser supports HTML, CSS, and".to_string(),
            "Gemini protocol content.".to_string(),
            String::new(),
            "Launch from the dashboard to open the".to_string(),
            "full browser widget.".to_string(),
        ];
        Self { content }
    }
}

impl App for BrowserApp {
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
        let app = BrowserApp::new("/apps/browser");
        assert_eq!(app.title(), "Browser");
        assert_eq!(app.path(), "/apps/browser");
    }

    #[test]
    fn lines_mention_html() {
        let app = BrowserApp::new("/apps/browser");
        assert!(app.lines().iter().any(|l| l.contains("HTML")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = BrowserApp::new("/apps/browser");
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn navigate_down_moves_cursor() {
        let vfs = MemoryVfs::new();
        let mut app = BrowserApp::new("/apps/browser");
        app.content.cached_max_visible = 20;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.cursor, 1);
    }

    #[test]
    fn downcast_works() {
        let app = BrowserApp::new("/apps/browser");
        let any = app.as_any();
        assert!(any.downcast_ref::<BrowserApp>().is_some());
    }
}
