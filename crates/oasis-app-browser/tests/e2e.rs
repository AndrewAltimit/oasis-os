//! End-to-end Browser launcher sessions through the `App` trait.
//!
//! The launcher is an informational placeholder: the real browser is a
//! dashboard widget, so no input here launches anything.
#![allow(clippy::unwrap_used)]

use oasis_app_browser::BrowserApp;
use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

fn browser() -> AppHarness {
    AppHarness::new(Box::new(BrowserApp::new("/apps/Browser")))
}

#[test]
fn window_explains_how_to_reach_the_browser() {
    let h = browser();
    let text = h.screen_text();
    assert!(text.contains("Use the browser widget"), "{text}");
    assert!(text.contains("Gemini"), "{text}");
    assert!(text.contains("Cancel=back"), "{text}");
}

#[test]
fn every_input_is_inert_except_cancel() {
    let mut h = browser();
    for b in [
        Button::Confirm,
        Button::Triangle,
        Button::Square,
        Button::Start,
        Button::Select,
        Button::Left,
        Button::Right,
    ] {
        assert_eq!(h.press(b), AppAction::None, "{b:?}");
    }
    h.type_text("https://example.com\n");
    assert_eq!(h.click(20, 20), AppAction::None);
    assert!(h.actions().is_empty(), "{:?}", h.actions());
    h.key(Key::Down);
    let sel = h.draw().find_text_containing("> ").unwrap().text.clone();
    assert_eq!(sel, "> ", "cursor on the blank line below the title");
    assert_eq!(h.key(Key::Escape), AppAction::Exit);
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = browser();
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |_: &dyn Vfs| -> Box<dyn App> { Box::new(BrowserApp::new("/apps/Browser")) };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}
