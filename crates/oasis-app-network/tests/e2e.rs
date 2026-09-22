//! End-to-end Network status sessions through the `App` trait.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_network::NetworkApp;
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

fn net(listening: bool, connected: bool) -> AppHarness {
    AppHarness::new(Box::new(NetworkApp::new(
        "/apps/Network",
        listening,
        9000,
        connected,
    )))
}

#[test]
fn window_shows_listener_and_remote_state() {
    let h = net(true, true);
    let text = h.screen_text();
    assert!(text.contains("127.0.0.1"), "{text}");
    assert!(text.contains("Active (port 9000)"), "{text}");
    assert!(text.contains("Remote:     Connected"), "{text}");
    let text = net(false, false).screen_text();
    assert!(text.contains("Not running"), "{text}");
    assert!(text.contains("Not connected"), "{text}");
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
    assert!(sel.contains("Interface"), "{sel}");
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
#[ignore = "oasis-app-core bug: ContentState scrolls by cached_max_visible (13, only \
            updated by the fullscreen update_sdi path), so a windowed app smaller than \
            13 rows never scrolls and its bottom lines are unreachable"]
fn small_window_can_scroll_to_the_last_line() {
    let mut h = net(false, false);
    h.set_size(160, 80);
    for _ in 0..20 {
        h.key(Key::Down);
    }
    let text = h.screen_text();
    assert!(text.contains("commands for remote access."), "{text}");
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
    let make = |_: &dyn Vfs| -> Box<dyn App> {
        Box::new(NetworkApp::new("/apps/Network", true, 65535, false))
    };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}
