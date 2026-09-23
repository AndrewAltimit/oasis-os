//! End-to-end sessions: the calculator driven through the `App` trait the
//! way the desktop host drives it (clicks on drawn keys, keyboard, gamepad).
#![allow(clippy::unwrap_used)]

use oasis_app_calculator::CalculatorApp;
use oasis_app_core::testing::{AppHarness, SIZES, THEMES, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

fn calc() -> AppHarness {
    AppHarness::new(Box::new(CalculatorApp::new("/apps/Calculator")))
}

/// First content line (the "Result: ..." / "Input: ..." line).
fn readout(h: &AppHarness) -> String {
    h.app().lines().first().cloned().unwrap_or_default()
}

#[test]
fn clicking_drawn_keys_computes_at_every_size_and_theme() {
    for &(w, hgt) in &SIZES[1..] {
        for theme in THEMES {
            let mut h = calc();
            h.set_size(w, hgt).set_theme(theme);
            for label in ["1", "2", "+", "3", "x", "2", "="] {
                h.click_text_last(label);
            }
            // 12 + 3 * 2 = 18 (operator precedence).
            assert_eq!(readout(&h), "  Result: 18", "{w}x{hgt} {theme}");
            assert!(h.screen_text().contains("18"), "{w}x{hgt} {theme}");
        }
    }
}

#[test]
fn typed_expressions_evaluate() {
    let cases = [
        ("(2+3)*4", "20"),
        ("2^10", "1024"),
        ("7%3", "1"),
        ("1.5*4", "6"),
        ("-3+5", "2"),
        ("10/4", "2.5"),
        ("2^3^2", "512"),
    ];
    for (expr, want) in cases {
        let mut h = calc();
        h.type_text(expr);
        h.key(Key::Enter);
        assert_eq!(readout(&h), format!("  Result: {want}"), "{expr}");
    }
}

#[test]
fn divide_by_zero_reports_error_and_recovers() {
    let mut h = calc();
    h.type_text("7/0");
    h.key(Key::Enter);
    let lines = h.app().lines().join("\n");
    assert!(lines.contains("Division by zero"), "{lines}");
    assert!(h.screen_text().contains("Division by zero"));
    // The broken entry is kept for editing; Escape clears it instead of
    // closing the app.
    assert_eq!(h.key(Key::Escape), AppAction::None);
    assert!(!h.closed());
    h.type_text("8/2=");
    assert_eq!(readout(&h), "  Result: 4");
    // Escape on a clear display falls through to Cancel and closes.
    h.key(Key::Escape);
    h.key(Key::Escape);
    assert!(h.closed());
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    for expr in ["(((", "1+", ")", "..", "1/(3-3)"] {
        let mut h = calc();
        h.type_text(expr);
        h.key(Key::Enter);
        let lines = h.app().lines().join("\n");
        assert!(
            lines.contains("Error") || lines.contains("Division"),
            "{expr}: {lines}"
        );
        let _ = h.draw();
    }
}

#[test]
fn gamepad_only_session_reaches_every_feature() {
    let mut h = calc();
    // Cursor starts on 5. Press 5, move right to 6, press, then = via d-pad.
    h.press(Button::Confirm);
    h.press(Button::Right);
    h.press(Button::Confirm);
    // Walk down to "." then right onto "=" (bottom-right) and press it.
    for _ in 0..2 {
        h.press(Button::Down);
    }
    h.press(Button::Right);
    h.press(Button::Confirm);
    assert_eq!(readout(&h), "  Result: 56");
    // Select recalls the last expression, Square deletes a char.
    h.press(Button::Select);
    assert!(readout(&h).contains("56"), "{}", readout(&h));
    h.press(Button::Square);
    assert_eq!(readout(&h), "  Input: 5");
    // Start = AC clears everything including history.
    h.press(Button::Start);
    assert!(h.app().lines().join("\n").contains("History: (empty)"));
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn memory_keys_by_click() {
    let mut h = calc();
    for label in ["4", "2", "MS", "C", "1", "+", "MR", "="] {
        h.click_text_last(label);
    }
    assert_eq!(readout(&h), "  Result: 43");
    for label in ["M+", "MC", "MR"] {
        h.click_text_last(label);
    }
    assert_eq!(readout(&h), "  Input: 0");
}

#[test]
fn history_persists_across_reopen() {
    let mut h = calc();
    h.type_text("6*7=");
    h.type_text("1+1=");
    h.frame(16);
    assert!(h.vfs().exists(oasis_app_calculator::history::HISTORY_PATH));

    h.replace_app(Box::new(CalculatorApp::new("/apps/Calculator")));
    h.frame(16);
    let lines = h.app().lines().join("\n");
    assert!(lines.contains("6*7 = 42"), "{lines}");
    assert!(lines.contains("1+1 = 2"), "{lines}");
    // Up recalls the newest entry first.
    h.key(Key::Up);
    assert_eq!(readout(&h), "  Input: 1+1");
    h.key(Key::Up);
    assert_eq!(readout(&h), "  Input: 6*7");
}

#[test]
fn corrupt_history_file_is_ignored() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.write(
        oasis_app_calculator::history::HISTORY_PATH,
        b"\xff\xfe garbage\n=\n\t=nan\n",
    )
    .unwrap();
    let mut h = AppHarness::with_vfs(Box::new(CalculatorApp::new("/apps/c")), vfs);
    h.frame(16);
    h.type_text("2+2=");
    assert_eq!(readout(&h), "  Result: 4");
    let _ = h.draw();
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = calc();
    h.type_text("123456789*987654321");
    h.draw_all_sizes_and_themes();
    h.key(Key::Enter);
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |_: &dyn Vfs| -> Box<dyn App> { Box::new(CalculatorApp::new("/apps/c")) };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}
