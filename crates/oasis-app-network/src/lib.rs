//! Network status app for OASIS_OS.
//!
//! Static-content screen showing the current loopback interface and the
//! state of the remote-terminal listener and any active outbound connection.
//! Live remote-control happens via the `listen` and `connect` terminal
//! commands; this app is a read-only summary.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// Network status app.
#[derive(Debug)]
pub struct NetworkApp {
    content: ContentState,
}

impl NetworkApp {
    /// Create a new Network status app.
    ///
    /// `listener_active` and `listener_port` describe the local
    /// remote-terminal listener; `remote_connected` reflects whether the
    /// shell currently holds an outbound connection.
    pub fn new(
        path: &str,
        listener_active: bool,
        listener_port: u16,
        remote_connected: bool,
    ) -> Self {
        let listener_status = if listener_active {
            format!("Active (port {listener_port})")
        } else {
            "Not running".to_string()
        };
        let remote_status = if remote_connected {
            "Connected".to_string()
        } else {
            "Not connected".to_string()
        };
        let mut content = ContentState::new("Network", path);
        content.lines = vec![
            "Network Status".to_string(),
            String::new(),
            "  Interface:  lo (loopback)".to_string(),
            "  Status:     Active".to_string(),
            "  Address:    127.0.0.1".to_string(),
            String::new(),
            format!("  Remote:     {remote_status}"),
            format!("  Listener:   {listener_status}"),
            String::new(),
            "Use terminal 'listen' and 'connect'".to_string(),
            "commands for remote access.".to_string(),
        ];
        Self { content }
    }
}

impl App for NetworkApp {
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
        let app = NetworkApp::new("/apps/network", false, 9000, false);
        assert_eq!(app.title(), "Network");
        assert_eq!(app.path(), "/apps/network");
    }

    #[test]
    fn lines_show_loopback_address() {
        let app = NetworkApp::new("/apps/network", false, 9000, false);
        assert!(app.lines().iter().any(|l| l.contains("127.0.0.1")));
    }

    #[test]
    fn listener_active_renders_port() {
        let app = NetworkApp::new("/apps/network", true, 9293, false);
        assert!(app.lines().iter().any(|l| l.contains("port 9293")));
    }

    #[test]
    fn listener_inactive_renders_not_running() {
        let app = NetworkApp::new("/apps/network", false, 0, false);
        assert!(app.lines().iter().any(|l| l.contains("Not running")));
    }

    #[test]
    fn remote_connected_renders_connected() {
        let app = NetworkApp::new("/apps/network", false, 9000, true);
        assert!(
            app.lines()
                .iter()
                .any(|l| l.contains("Remote") && l.contains("Connected"))
        );
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = NetworkApp::new("/apps/network", false, 9000, false);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = NetworkApp::new("/apps/network", false, 9000, false);
        let any = app.as_any();
        assert!(any.downcast_ref::<NetworkApp>().is_some());
    }
}
