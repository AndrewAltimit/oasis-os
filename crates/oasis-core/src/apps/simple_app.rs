//! Generic simple app for static-content screens.
//!
//! Used as the delegate for the in-core Terminal app (which syncs its
//! display lines from the desktop terminal pipeline via [`SimpleApp::set_lines`])
//! and as the placeholder type the plugin system constructs for dynamically
//! registered apps. The four other static-content apps (Browser, Network,
//! Package Manager, System Monitor) live in their own `oasis-app-*` crates
//! and no longer go through this type.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::AppAction;
use super::ContentState;
use super::app_trait::App;
use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};

/// A simple static-content app that implements the `App` trait.
///
/// Used for apps that display informational text with basic navigation.
/// Content is set at creation time via a builder or direct lines.
#[derive(Debug)]
pub struct SimpleApp {
    pub content: ContentState,
    /// Action returned on Confirm press (default: None).
    confirm_action: AppAction,
}

impl SimpleApp {
    /// Create a simple app with pre-set content lines.
    pub fn new(title: &str, path: &str, lines: Vec<String>) -> Self {
        let mut content = ContentState::new(title, path);
        content.lines = lines;
        Self {
            content,
            confirm_action: AppAction::None,
        }
    }

    /// Create the Settings app.
    pub fn settings(path: &str, skin_name: &str, width: u32, height: u32) -> Self {
        Self::new(
            "Settings",
            path,
            vec![
                "OASIS_OS Settings".to_string(),
                String::new(),
                format!("  Screen:     {width} x {height}"),
                format!("  Skin:       {skin_name}"),
                "  Audio:      Enabled".to_string(),
                "  Network:    Enabled".to_string(),
                "  Terminal:   Enabled".to_string(),
                "  Plugins:    Enabled".to_string(),
                String::new(),
                "(Settings are read-only in this build)".to_string(),
            ],
        )
    }

    /// Create the Terminal app (windowed interactive terminal).
    ///
    /// Text input and command execution are handled by the desktop input
    /// dispatcher, which syncs output lines back into this app via
    /// `set_terminal_lines()`.
    pub fn terminal(path: &str) -> Self {
        Self::new(
            "Terminal",
            path,
            vec![
                "OASIS_OS Terminal".to_string(),
                String::new(),
                "Type a command and press Enter.".to_string(),
            ],
        )
    }

    /// Update the display lines (used by the desktop input handler to sync
    /// terminal output into this app's display).
    ///
    /// `scroll_offset` scrolls up from the bottom (0 = fully scrolled down).
    pub fn set_lines(&mut self, lines: Vec<String>, scroll_offset: usize) {
        let len = lines.len();
        self.content.lines = lines;
        if len > self.content.cached_max_visible {
            let max_scroll = len - self.content.cached_max_visible;
            self.content.scroll = max_scroll.saturating_sub(scroll_offset);
        } else {
            self.content.scroll = 0;
        }
    }
}

impl App for SimpleApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

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
            Button::Confirm => self.confirm_action.clone(),
            _ => AppAction::None,
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    #[test]
    fn settings_title_and_path() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.title(), "Settings");
        assert_eq!(app.path(), "/apps/settings");
    }

    #[test]
    fn settings_content_lines() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.lines().iter().any(|l| l.contains("OASIS_OS Settings")));
        assert!(app.lines().iter().any(|l| l.contains("read-only")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn navigate_up_down() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        app.content.cached_max_visible = 20;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.cursor, 1);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.content.cursor, 0);
    }

    #[test]
    fn custom_content() {
        let app = SimpleApp::new(
            "Custom",
            "/apps/custom",
            vec!["Line 1".into(), "Line 2".into()],
        );
        assert_eq!(app.title(), "Custom");
        assert_eq!(app.lines().len(), 2);
    }

    #[test]
    fn no_browse_dir_or_viewing_file() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.browse_dir().is_none());
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn no_pending_request() {
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.take_pending_request().is_none());
        assert!(app.peek_pending_request().is_none());
    }

    #[test]
    fn downcast_works() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        let any = app.as_any();
        assert!(any.downcast_ref::<SimpleApp>().is_some());
    }

    #[test]
    fn confirm_is_noop() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.handle_input(&Button::Confirm, &vfs), AppAction::None);
    }
}
