//! Network status app.
//!
//! Shows the state of the shell's network services (remote-terminal
//! listener, FTP server, outbound remote connection, MCP control server)
//! as the host publishes it at [`NET_STATE_PATH`]. The app re-reads the
//! file from [`App::tick`] (throttled like the System Monitor) and only
//! reports a change when its bytes differ. Services are started and
//! stopped with the `listen`, `ftp`, `remote` and `mcp-server` commands.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_types::ipc::{NET_STATE_PATH, NetState};
use oasis_vfs::Vfs;

use crate::POLL_INTERVAL_MS;

/// Network status app.
#[derive(Debug)]
pub struct NetworkApp {
    content: ContentState,
    /// Wall time accumulated since the state file was last read.
    since_poll_ms: u32,
    /// Raw bytes of the last state file read (`None`: not published).
    last_raw: Option<Vec<u8>>,
}

impl NetworkApp {
    /// Create the app, showing the state currently published in `vfs`.
    pub fn new(path: &str, vfs: &dyn Vfs) -> Self {
        let mut app = Self {
            content: ContentState::new("Network", path),
            since_poll_ms: 0,
            last_raw: None,
        };
        app.last_raw = read_state(vfs);
        app.rebuild_lines();
        app
    }

    /// The state currently displayed (`None` when the host publishes none).
    pub fn state(&self) -> Option<NetState> {
        self.last_raw
            .as_deref()
            .map(|raw| NetState::parse(&String::from_utf8_lossy(raw)))
    }

    /// Re-read [`NET_STATE_PATH`] now. Returns `true` when it changed.
    pub fn reload(&mut self, vfs: &dyn Vfs) -> bool {
        let raw = read_state(vfs);
        if raw == self.last_raw {
            return false;
        }
        self.last_raw = raw;
        self.rebuild_lines();
        true
    }

    fn rebuild_lines(&mut self) {
        fn service(port: Option<u16>) -> String {
            port.map_or_else(
                || "Not running".to_string(),
                |p| format!("Running (port {p})"),
            )
        }
        let mut lines = vec!["Network Status".to_string(), String::new()];
        match self.state() {
            Some(st) => {
                lines.push(format!("  Listener:   {}", service(st.listener_port)));
                lines.push(format!("  FTP:        {}", service(st.ftp_port)));
                let remote = if st.remote_connected {
                    "Connected"
                } else {
                    "Not connected"
                };
                lines.push(format!("  Remote:     {remote}"));
                let mcp = if st.mcp_running {
                    "Running"
                } else {
                    "Not running"
                };
                lines.push(format!("  MCP:        {mcp}"));
            },
            None => lines.push("  (this host publishes no network state)".to_string()),
        }
        lines.push(String::new());
        lines.push("Use terminal 'listen', 'ftp' and".to_string());
        lines.push("'remote' commands to manage them.".to_string());
        // Keep the selection on screen when the line count changes.
        self.content.cursor = self.content.cursor.min(lines.len().saturating_sub(1));
        self.content.lines = lines;
    }
}

fn read_state(vfs: &dyn Vfs) -> Option<Vec<u8>> {
    if vfs.exists(NET_STATE_PATH) {
        vfs.read(NET_STATE_PATH).ok()
    } else {
        None
    }
}

impl App for NetworkApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        self.content.handle_nav(button).unwrap_or(AppAction::None)
    }

    fn tick(&mut self, dt_ms: u32, vfs: &dyn Vfs) -> bool {
        self.since_poll_ms = self.since_poll_ms.saturating_add(dt_ms);
        if self.since_poll_ms < POLL_INTERVAL_MS {
            return false;
        }
        self.since_poll_ms = 0;
        self.reload(vfs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_app_core::AsAny;
    use oasis_vfs::MemoryVfs;

    fn vfs_with(st: &NetState) -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(NET_STATE_PATH, st.to_text().as_bytes()).unwrap();
        vfs
    }

    #[test]
    fn title_and_path() {
        let app = NetworkApp::new("/apps/network", &MemoryVfs::new());
        assert_eq!(app.title(), "Network");
        assert_eq!(app.path(), "/apps/network");
    }

    #[test]
    fn unpublished_state_says_so() {
        let app = NetworkApp::new("/apps/network", &MemoryVfs::new());
        assert!(app.state().is_none());
        assert!(app.lines().iter().any(|l| l.contains("publishes no")));
    }

    #[test]
    fn running_services_render_their_ports() {
        let vfs = vfs_with(&NetState {
            listener_port: Some(9293),
            ftp_port: Some(2121),
            remote_connected: true,
            mcp_running: false,
        });
        let app = NetworkApp::new("/apps/network", &vfs);
        let text = app.lines().join("\n");
        assert!(text.contains("Listener:   Running (port 9293)"), "{text}");
        assert!(text.contains("FTP:        Running (port 2121)"), "{text}");
        assert!(text.contains("Remote:     Connected"), "{text}");
        assert!(text.contains("MCP:        Not running"), "{text}");
    }

    #[test]
    fn tick_picks_up_state_changes() {
        let mut vfs = vfs_with(&NetState::default());
        let mut app = NetworkApp::new("/apps/network", &vfs);
        assert!(!app.tick(POLL_INTERVAL_MS, &vfs), "unchanged state");
        let st = NetState {
            listener_port: Some(9000),
            ..NetState::default()
        };
        vfs.write(NET_STATE_PATH, st.to_text().as_bytes()).unwrap();
        assert!(!app.tick(1, &vfs), "throttled");
        assert!(app.tick(POLL_INTERVAL_MS, &vfs));
        assert_eq!(app.state(), Some(st));
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = NetworkApp::new("/apps/network", &vfs);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = NetworkApp::new("/apps/network", &MemoryVfs::new());
        assert!(app.as_any().downcast_ref::<NetworkApp>().is_some());
    }
}
