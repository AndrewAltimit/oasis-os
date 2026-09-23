//! End-to-end Package Manager sessions through the `App` trait.
//!
//! The package manager is read-only in this build (no fetch / install
//! path exists yet), so these tests cover the listing and navigation.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_package_manager::PackageManagerApp;
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

fn pm() -> AppHarness {
    AppHarness::new(Box::new(PackageManagerApp::new("/apps/Package Manager")))
}

#[test]
fn lists_installed_packages_with_the_workspace_version() {
    let h = pm();
    let text = h.screen_text();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        text.contains(&format!("oasis-core      {version}")),
        "{text}"
    );
    assert!(text.contains("classic-skin"), "{text}");
    assert!(text.contains("No updates available."), "{text}");
}

#[test]
fn navigation_moves_the_selection_and_cancel_exits() {
    let mut h = pm();
    for _ in 0..3 {
        h.key(Key::Down);
    }
    let sel = h.draw().find_text_containing("> ").unwrap().text.clone();
    assert!(sel.contains("oasis-core"), "{sel}");
    // Past the end the selection stays on the last line.
    for _ in 0..20 {
        h.press(Button::Down);
    }
    let sel = h.draw().find_text_containing("> ").unwrap().text.clone();
    assert!(sel.contains("No updates"), "{sel}");
    assert_eq!(h.press(Button::Confirm), AppAction::None);
    assert_eq!(h.click(10, 10), AppAction::None);
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = pm();
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make =
        |_: &dyn Vfs| -> Box<dyn App> { Box::new(PackageManagerApp::new("/apps/Package Manager")) };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}
