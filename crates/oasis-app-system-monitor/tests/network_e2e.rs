//! End-to-end Network status sessions through the `App` trait.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_system_monitor::NetworkApp;
use oasis_types::input::{Button, Key};
use oasis_types::ipc::{NET_STATE_PATH, NetState};
use oasis_vfs::{MemoryVfs, Vfs};

fn state_vfs(listening: bool, connected: bool) -> MemoryVfs {
    let st = NetState {
        listener_port: listening.then_some(9000),
        ftp_port: None,
        remote_connected: connected,
        mcp_running: false,
    };
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(NET_STATE_PATH, st.to_text().as_bytes()).unwrap();
    vfs
}

fn net(listening: bool, connected: bool) -> AppHarness {
    let vfs = state_vfs(listening, connected);
    let app = NetworkApp::new("/apps/Network", &vfs);
    AppHarness::with_vfs(Box::new(app), vfs)
}

#[test]
fn window_shows_listener_and_remote_state() {
    let mut h = net(true, true);
    let text = h.screen_text();
    assert!(text.contains("Running (port 9000)"), "{text}");
    assert!(text.contains("Remote:     Connected"), "{text}");
    let text = net(false, false).screen_text();
    assert!(text.contains("Not running"), "{text}");
    assert!(text.contains("Not connected"), "{text}");
}

#[test]
fn window_follows_the_published_state() {
    let mut h = net(false, false);
    assert!(h.screen_text().contains("Listener:   Not running"));
    let st = NetState {
        listener_port: Some(9123),
        ..NetState::default()
    };
    h.vfs_mut()
        .write(NET_STATE_PATH, st.to_text().as_bytes())
        .unwrap();
    h.frame(1000);
    let text = h.screen_text();
    assert!(text.contains("Listener:   Running (port 9123)"), "{text}");
}

#[test]
fn cursor_moves_with_keys_and_clicks_are_harmless() {
    let mut h = net(false, false);
    // The selected line is drawn with a "> " prefix.
    let first = h.draw().find_text_containing("> ").unwrap().text.clone();
    assert!(first.contains("Network Status"), "{first}");
    h.key(Key::Down);
    h.key(Key::Down);
    let sel = h.draw().find_text_containing("> ").unwrap().text.clone();
    assert!(sel.contains("Listener"), "{sel}");
    h.key(Key::Up);
    h.click(40, 40);
    h.click(-5, 9999);
    let sel = h.draw().find_text_containing("> ").unwrap().text.clone();
    // Back on the blank line under the heading; clicks do not move it.
    assert_eq!(sel, "> ");
    assert_eq!(h.press(Button::Confirm), AppAction::None);
    assert_eq!(h.key(Key::Escape), AppAction::Exit);
}

/// Every line must be reachable in a small window: scrolling with Down
/// has to bring the last line into view.
#[test]
fn small_window_can_scroll_to_the_last_line() {
    let mut h = net(false, false);
    h.set_size(160, 80);
    for _ in 0..20 {
        h.key(Key::Down);
    }
    let text = h.screen_text();
    // The last line, ellipsized to the narrow window.
    assert!(text.contains("> ") && text.contains("'remote'"), "{text}");
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = net(true, true);
    h.draw_all_sizes_and_themes();
    for _ in 0..20 {
        h.key(Key::Down);
    }
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |vfs: &dyn Vfs| -> Box<dyn App> { Box::new(NetworkApp::new("/apps/Network", vfs)) };
    for seed in 1..=3 {
        fuzz_app(&make, state_vfs(true, false), seed, 3000);
    }
}
