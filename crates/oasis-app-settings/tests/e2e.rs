//! End-to-end Settings sessions: the app driven through the `App` trait the
//! way the desktop host drives it, with a tiny "shell" that applies the
//! IPC requests Settings posts and publishes the resulting state back to
//! the VFS (exactly the round trip `oasis-app` performs).
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_settings::{
    FONT_SCALE_REQUEST_PATH, FONT_SCALE_STATE_PATH, HIGH_CONTRAST_REQUEST_PATH, HIGH_CONTRAST_SKIN,
    LOCALE_CHANGE_REQUEST_PATH, LOCALE_STATE_PATH, REDUCED_MOTION_REQUEST_PATH,
    REDUCED_MOTION_STATE_PATH, RESOLUTION_CHANGE_REQUEST_PATH, RESOLUTION_STATE_PATH,
    SKIN_APPLY_THEME_REQUEST_PATH, SKIN_CHANGE_REQUEST_PATH, SKIN_STATE_PATH, SettingsApp,
    VOLUME_CHANGE_REQUEST_PATH, VOLUME_STATE_PATH,
};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

/// Content origin of the harness window (see `AppHarness::new`).
const ORIGIN: (i32, i32) = (8, 20);

fn shell_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.mkdir("/system/ipc").unwrap();
    vfs.write(SKIN_STATE_PATH, b"classic").unwrap();
    vfs.write(RESOLUTION_STATE_PATH, b"480x272").unwrap();
    vfs
}

fn open(vfs: &dyn Vfs) -> Box<dyn App> {
    Box::new(SettingsApp::from_vfs(
        "/apps/Settings",
        vfs,
        "classic",
        480,
        272,
        "SDL3",
    ))
}

fn settings() -> AppHarness {
    let vfs = shell_vfs();
    let app = open(&vfs);
    let mut h = AppHarness::with_vfs(app, vfs);
    h.set_size(1024, 768);
    h
}

/// Take the pending IPC request (as the shell does each frame).
fn take_request(h: &mut AppHarness) -> Option<(String, String)> {
    h.app_mut().take_pending_request()
}

/// Apply one request the way the shell does and publish the new state.
fn shell_apply(h: &mut AppHarness) -> (String, String) {
    let (path, data) = take_request(h).expect("Settings posted no request");
    let state: Option<(&str, String)> = match path.as_str() {
        SKIN_CHANGE_REQUEST_PATH => Some((SKIN_STATE_PATH, data.clone())),
        RESOLUTION_CHANGE_REQUEST_PATH => Some((RESOLUTION_STATE_PATH, data.clone())),
        VOLUME_CHANGE_REQUEST_PATH => Some((VOLUME_STATE_PATH, data.clone())),
        LOCALE_CHANGE_REQUEST_PATH => Some((LOCALE_STATE_PATH, data.clone())),
        FONT_SCALE_REQUEST_PATH => Some((FONT_SCALE_STATE_PATH, data.clone())),
        REDUCED_MOTION_REQUEST_PATH => Some((REDUCED_MOTION_STATE_PATH, data.clone())),
        HIGH_CONTRAST_REQUEST_PATH => Some((
            SKIN_STATE_PATH,
            if data == "on" {
                HIGH_CONTRAST_SKIN
            } else {
                "classic"
            }
            .to_string(),
        )),
        _ => None,
    };
    if let Some((state_path, value)) = state {
        h.vfs_mut().write(state_path, value.as_bytes()).unwrap();
    }
    // The shell publishes a frame later; Settings picks it up on tick.
    h.frame(16);
    (path, data)
}

fn lines(h: &AppHarness) -> String {
    h.app().lines().join("\n")
}

#[test]
fn clicking_a_skin_requests_it_and_marks_it_active_once_applied() {
    let mut h = settings();
    h.click_text("paper");
    // Not active until the shell confirms.
    assert!(!lines(&h).contains("paper *"), "{}", lines(&h));
    let (path, data) = shell_apply(&mut h);
    assert_eq!(
        (path.as_str(), data.as_str()),
        (SKIN_CHANGE_REQUEST_PATH, "paper")
    );
    assert!(lines(&h).contains("paper *"), "{}", lines(&h));
    assert!(!lines(&h).contains("classic *"));
    // The active marker is drawn next to the row.
    let b = h.draw();
    let paper = b.find_text("paper").unwrap();
    let star = b
        .texts()
        .iter()
        .find(|t| t.visible && t.text == " *")
        .unwrap();
    assert!((star.y - paper.y).abs() <= 2, "marker not on the paper row");

    // Clicking the already-active skin posts nothing.
    h.click_text("paper");
    assert!(take_request(&mut h).is_none());
}

#[test]
fn settings_survive_reopening_from_the_published_state() {
    let mut h = settings();
    // Skin via click.
    h.click_text("xp");
    shell_apply(&mut h);
    // Resolution via keyboard: Right x2 -> Resolution, Down, Enter.
    h.key(Key::Right);
    h.key(Key::Right);
    assert!(lines(&h).contains("[Resolution]"), "{}", lines(&h));
    h.key(Key::Down);
    h.key(Key::Enter);
    let (_, res) = shell_apply(&mut h);
    assert_eq!(res, "800x600");
    // Volume via '+' / '-' keys on the Audio tab.
    h.key(Key::Right);
    h.type_text("+++");
    h.type_text("-");
    let (_, vol) = shell_apply(&mut h);
    assert_eq!(vol, "90");

    // Reopen: a fresh instance on the same VFS shows the applied values.
    let app = open(h.vfs());
    h.replace_app(app);
    let text = lines(&h);
    assert!(text.contains("xp *"), "{text}");
    for _ in 0..6 {
        h.press(Button::Right); // -> System
    }
    let text = lines(&h);
    assert!(text.contains("Active Skin:   xp"), "{text}");
    assert!(text.contains("Resolution:    800 x 600"), "{text}");
    h.press(Button::Left);
    h.press(Button::Left);
    h.press(Button::Left); // Audio
    assert!(lines(&h).contains("90%"), "{}", lines(&h));
    assert!(h.screen_text().contains("90%"));
}

#[test]
fn volume_slider_click_sets_value_and_optimistic_value_survives_stale_state() {
    let mut h = settings();
    h.vfs_mut().write(VOLUME_STATE_PATH, b"80").unwrap();
    h.frame(16);
    h.click_text("Audio");
    let b = h.draw();
    let value = b.find_text("80%").unwrap().clone();
    // The slider track starts right after the value text (value width +
    // 6px padding); click its left end. The harness window's content
    // origin is (8, 20), `click` takes content-local coordinates.
    let (ox, oy) = ORIGIN;
    let track_x = value.x + value.w as i32 + 6;
    h.click(track_x + 1 - ox, value.center().1 - oy);
    assert_eq!(take_request(&mut h).unwrap().1, "0");
    assert!(lines(&h).contains(" 0%"), "{}", lines(&h));
    // The shell has not published yet: ticking with the old "80" must not
    // snap the slider back.
    h.frame(16);
    h.frame(16);
    assert!(lines(&h).contains(" 0%"), "{}", lines(&h));
    // Keys step by 5 and clamp at 0 / 100.
    h.type_text("-");
    assert!(take_request(&mut h).is_none(), "no request below 0");
    for _ in 0..30 {
        h.type_text("+");
    }
    assert_eq!(take_request(&mut h).unwrap().1, "100");
}

#[test]
fn accessibility_toggles_post_requests_and_reflect_state() {
    let mut h = settings();
    h.click_text_containing("Access");
    let text = lines(&h);
    assert!(text.contains("[OFF]"), "{text}");
    h.click_text("Reduced Motion");
    let (path, data) = shell_apply(&mut h);
    assert_eq!(
        (path.as_str(), data.as_str()),
        (REDUCED_MOTION_REQUEST_PATH, "1")
    );
    h.click_text("High Contrast");
    let (path, data) = shell_apply(&mut h);
    assert_eq!(
        (path.as_str(), data.as_str()),
        (HIGH_CONTRAST_REQUEST_PATH, "on")
    );
    let text = lines(&h);
    assert!(!text.contains("[OFF]"), "both toggles should be on: {text}");
    // Font scale: Confirm on the slider row cycles presets upward.
    h.press(Button::Up);
    h.press(Button::Up);
    h.press(Button::Confirm);
    let (path, data) = shell_apply(&mut h);
    assert_eq!(path, FONT_SCALE_REQUEST_PATH);
    assert_eq!(data, "1.25");
    // Turning high contrast back off restores a normal skin.
    h.click_text("High Contrast");
    let (_, data) = shell_apply(&mut h);
    assert_eq!(data, "off");
    assert!(lines(&h).contains("High Contrast: [OFF]"), "{}", lines(&h));
}

#[test]
fn appearance_editor_edits_reverts_and_applies_a_palette() {
    let mut h = settings();
    h.click_text_containing("Appear");
    let before = lines(&h);
    let bg_line = |s: &str| {
        s.lines()
            .find(|l| l.contains("Background"))
            .unwrap()
            .to_string()
    };
    // Enter edit mode on Background, raise R, Cancel reverts.
    h.press(Button::Confirm);
    h.press(Button::Up);
    assert_ne!(bg_line(&lines(&h)), bg_line(&before));
    assert_eq!(
        h.press(Button::Cancel),
        AppAction::None,
        "Cancel must not exit"
    );
    assert_eq!(bg_line(&lines(&h)), bg_line(&before));
    // Edit again, raise B twice, commit.
    h.press(Button::Confirm);
    h.press(Button::Left); // R -> B
    h.press(Button::Up);
    h.press(Button::Up);
    h.press(Button::Confirm);
    let edited = bg_line(&lines(&h));
    assert_ne!(edited, bg_line(&before));
    let hex = edited
        .split_whitespace()
        .find(|w| w.starts_with('#'))
        .unwrap()
        .to_string();
    // Apply by clicking the action row.
    h.click_text_containing("Apply");
    let (path, toml) = take_request(&mut h).unwrap();
    assert_eq!(path, SKIN_APPLY_THEME_REQUEST_PATH);
    assert!(toml.contains(&hex), "applied theme lacks {hex}: {toml}");
}

#[test]
fn language_choice_is_requested_and_applied() {
    let mut h = settings();
    h.click_text("Language");
    h.press(Button::Down);
    h.press(Button::Confirm);
    let (path, code) = shell_apply(&mut h);
    assert_eq!(path, LOCALE_CHANGE_REQUEST_PATH);
    assert_ne!(code, "en");
    assert!(lines(&h).contains(" *"));
}

#[test]
fn garbage_published_state_is_ignored() {
    let mut vfs = shell_vfs();
    vfs.write(RESOLUTION_STATE_PATH, b"wide").unwrap();
    vfs.write(VOLUME_STATE_PATH, b"\xff\xfe").unwrap();
    vfs.write(FONT_SCALE_STATE_PATH, b"NaN").unwrap();
    vfs.write(LOCALE_STATE_PATH, b"xx-YY").unwrap();
    let app = open(&vfs);
    let mut h = AppHarness::with_vfs(app, vfs);
    h.frame(16);
    for _ in 0..6 {
        h.press(Button::Right);
    }
    assert!(lines(&h).contains("480 x 272"), "{}", lines(&h));
    h.draw_all_sizes_and_themes();
}

#[test]
fn cancel_exits_and_every_tab_draws_at_all_sizes_and_themes() {
    let mut h = settings();
    for _ in 0..8 {
        h.draw_all_sizes_and_themes();
        h.press(Button::Right);
    }
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |vfs: &dyn Vfs| open(vfs);
    for seed in 1..=3 {
        fuzz_app(&make, shell_vfs(), seed, 3000);
    }
}
