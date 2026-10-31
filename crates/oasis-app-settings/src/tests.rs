//! Settings app unit tests (navigation, IPC requests, state sync,
//! click hit-testing, Appearance editor).

use super::*;
use oasis_app_core::testing::click_at;
use oasis_vfs::MemoryVfs;

/// Take the oldest queued IPC request as `(path, data)`.
fn take_req(app: &mut SettingsApp) -> Option<(String, String)> {
    if app.content.requests.is_empty() {
        return None;
    }
    let req = app.content.requests.remove(0);
    Some((req.path, req.data))
}

fn make_app() -> SettingsApp {
    SettingsApp::new("/apps/settings", "classic", 480, 272, "SDL3")
}

fn make_vfs() -> MemoryVfs {
    MemoryVfs::new()
}

#[test]
fn title_and_path() {
    let app = make_app();
    assert_eq!(app.title(), "Settings");
    assert_eq!(app.path(), "/apps/settings");
}

#[test]
fn initial_category_is_display() {
    let app = make_app();
    assert_eq!(app.category, Category::Display);
}

#[test]
fn lines_not_empty() {
    let app = make_app();
    assert!(!app.lines().is_empty());
}

#[test]
fn lines_contain_skin_names() {
    let app = make_app();
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("classic")));
    assert!(lines.iter().any(|l| l.contains("balatro")));
}

#[test]
fn current_skin_marked() {
    let app = make_app();
    let lines = app.lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("classic") && l.contains('*')),
        "current skin should be marked with *"
    );
}

#[test]
fn cancel_exits() {
    let vfs = make_vfs();
    let mut app = make_app();
    assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
}

#[test]
fn right_switches_category() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.category, Category::Appearance);
}

#[test]
fn left_wraps_category() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.category, Category::About);
}

#[test]
fn category_cycle_wraps_right() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..Category::ALL.len() {
        app.handle_input(&Button::Right, &vfs);
    }
    assert_eq!(app.category, Category::Display);
}

#[test]
fn navigate_skins_down() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.item_cursor, 1);
}

#[test]
fn navigate_skins_up_at_top_stays() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.item_cursor, 0);
}

#[test]
fn confirm_selects_skin() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Move to second skin and confirm.
    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Confirm, &vfs);

    let names = builtin_names();
    // Should have a pending VFS IPC request.
    let req = take_req(&mut app);
    assert!(req.is_some());
    let (path, data) = req.as_ref().unwrap();
    assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
    assert_eq!(data, names[1]);
}

#[test]
fn confirm_same_skin_no_request() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Confirm on current skin (classic, cursor=0).
    app.handle_input(&Button::Confirm, &vfs);
    assert!(take_req(&mut app).is_none());
}

#[test]
fn audio_volume_up() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..3 {
        // Display -> Appearance -> Resolution -> Audio
        app.handle_input(&Button::Right, &vfs);
    }
    assert_eq!(app.category, Category::Audio);
    let before = app.volume;
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.volume, (before + 5).min(100));
}

#[test]
fn audio_volume_down() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..3 {
        app.handle_input(&Button::Right, &vfs);
    }
    let before = app.volume;
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.volume, before.saturating_sub(5));
}

#[test]
fn audio_volume_clamped_at_100() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..3 {
        app.handle_input(&Button::Right, &vfs);
    }
    app.volume = 100;
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.volume, 100);
}

#[test]
fn audio_volume_clamped_at_0() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..3 {
        app.handle_input(&Button::Right, &vfs);
    }
    app.volume = 0;
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.volume, 0);
}

#[test]
fn system_category_lines() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Navigate to System.
    for _ in 0..6 {
        app.handle_input(&Button::Right, &vfs);
    }
    assert_eq!(app.category, Category::System);
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("SDL3")));
    assert!(lines.iter().any(|l| l.contains("480")));
}

#[test]
fn about_category_lines() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Navigate to About.
    for _ in 0..7 {
        app.handle_input(&Button::Right, &vfs);
    }
    assert_eq!(app.category, Category::About);
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("MIT")));
    assert!(lines.iter().any(|l| l.contains("PSP")));
}

#[test]
fn about_up_scrolls_back_immediately_after_overscrolling_down() {
    // Regression: Down past the end kept growing `text_scroll` beyond
    // the last page, so the next Up presses had no visible effect.
    let vfs = make_vfs();
    let mut app = app_in(Category::About, &vfs);
    app.layout(CW, CH, &ActiveTheme::default());
    let rows = app.rows().len();
    let max = rows.saturating_sub(app.visible_rows);
    assert!(max > 0, "About must overflow the test window");
    for _ in 0..rows + 5 {
        app.handle_input(&Button::Down, &vfs);
    }
    assert_eq!(app.text_scroll, max);
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.text_scroll, max - 1);
}

#[test]
fn layout_shrink_clamps_text_scroll() {
    let vfs = make_vfs();
    let mut app = app_in(Category::About, &vfs);
    app.layout(CW, 60, &ActiveTheme::default());
    for _ in 0..50 {
        app.handle_input(&Button::Down, &vfs);
    }
    // A taller window shows more rows, so the old offset is past the end.
    app.layout(CW, 2000, &ActiveTheme::default());
    let max = app.rows().len().saturating_sub(app.visible_rows);
    assert!(app.text_scroll <= max);
}

#[test]
fn published_font_scale_is_clamped_to_range() {
    let mut vfs = make_vfs();
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(FONT_SCALE_STATE_PATH, b"5").unwrap();
    let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
    let (min, max) = font_scale_range();
    assert_eq!(app.font_scale, max);
    vfs.write(FONT_SCALE_STATE_PATH, b"-1").unwrap();
    let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
    assert_eq!(app.font_scale, min);
}

#[test]
fn ui_strings_resolve_through_catalog() {
    // A missing key would render as the raw key ("settings.cat_...").
    for c in Category::ALL {
        assert!(!c.label().starts_with("settings."), "{c:?}");
    }
    let vfs = make_vfs();
    for c in Category::ALL {
        let app = app_in(c, &vfs);
        for line in app.lines() {
            assert!(
                !line.contains("settings.") && !line.contains("system."),
                "untranslated key in {c:?}: {line}"
            );
        }
    }
    // German strings exist for the categories (Latin-1, drawable).
    assert_eq!(
        oasis_i18n::translate_for("settings.cat_language", Locale::German),
        "Sprache"
    );
}

#[test]
fn downcast_works() {
    let app: Box<dyn App> = Box::new(make_app());
    assert!(app.as_any().downcast_ref::<SettingsApp>().is_some());
}

#[test]
fn no_browse_dir_or_viewing_file() {
    let app = make_app();
    assert!(app.browse_dir().is_none());
    assert!(app.viewing_file().is_none());
}

#[test]
fn item_cursor_bounded() {
    let vfs = make_vfs();
    let mut app = make_app();
    let count = builtin_names().len();
    // Navigate down past all skins.
    for _ in 0..count + 5 {
        app.handle_input(&Button::Down, &vfs);
    }
    assert!(app.item_cursor < count);
}

#[test]
fn switching_category_resets_cursor_to_current_skin() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Move cursor off the active skin.
    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Down, &vfs);
    assert!(app.item_cursor > 0);
    // Leave and return -> cursor snaps back to active skin (classic, index 0).
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.item_cursor, 0);
}

#[test]
fn audio_lines_contain_volume() {
    let vfs = make_vfs();
    let mut app = make_app();
    for _ in 0..3 {
        app.handle_input(&Button::Right, &vfs);
    }
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("Volume")));
}

// -- Resolution category --

#[test]
fn resolution_category_lists_presets() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.category, Category::Resolution);
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("480x272")));
    assert!(lines.iter().any(|l| l.contains("1920x1080")));
}

#[test]
fn resolution_current_is_marked() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    let lines = app.lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("480x272") && l.contains('*')),
        "current resolution should be marked"
    );
}

#[test]
fn resolution_cursor_starts_on_current() {
    let vfs = make_vfs();
    // Start with a non-first resolution so we know the cursor is really
    // being aligned rather than just defaulting to 0.
    let mut app = SettingsApp::new("/apps/settings", "classic", 1280, 720, "SDL3");
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.category, Category::Resolution);
    // 1280x720 is index 3 in RESOLUTION_PRESETS.
    assert_eq!(app.item_cursor, 3);
}

#[test]
fn confirm_resolution_writes_request() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Display -> Appearance -> Resolution.
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    // Move from 480x272 (idx 0) to 1280x720 (idx 3).
    for _ in 0..3 {
        app.handle_input(&Button::Down, &vfs);
    }
    app.handle_input(&Button::Confirm, &vfs);

    let req = take_req(&mut app);
    let (path, data) = req.expect("resolution confirm should post an IPC request");
    assert_eq!(path, RESOLUTION_CHANGE_REQUEST_PATH);
    assert_eq!(data, "1280x720");
}

#[test]
fn confirm_same_resolution_no_request() {
    let vfs = make_vfs();
    let mut app = make_app();
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    // Cursor starts on the active resolution, so Confirm should no-op.
    app.handle_input(&Button::Confirm, &vfs);
    assert!(take_req(&mut app).is_none());
}

// -- VFS-driven construction + sync --

#[test]
fn from_vfs_reads_state_paths() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(SKIN_STATE_PATH, b"balatro").unwrap();
    vfs.write(RESOLUTION_STATE_PATH, b"1280x720").unwrap();
    vfs.write(BACKEND_STATE_PATH, b"SDL3").unwrap();

    let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "UNKNOWN");
    assert_eq!(app.current_skin, "balatro");
    assert_eq!((app.width, app.height), (1280, 720));
    assert_eq!(app.backend_name, "SDL3");
}

#[test]
fn from_vfs_falls_back_to_defaults() {
    let vfs = MemoryVfs::new();
    let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
    assert_eq!(app.current_skin, "classic");
    assert_eq!((app.width, app.height), (480, 272));
}

#[test]
fn sync_picks_up_shell_state_changes() {
    let mut vfs = MemoryVfs::new();
    let mut app = make_app();
    assert_eq!(app.current_skin, "classic");

    // Shell applies a skin swap and publishes the new state.
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(SKIN_STATE_PATH, b"balatro").unwrap();
    vfs.write(RESOLUTION_STATE_PATH, b"1280x720").unwrap();

    // Any input tick refreshes from VFS.
    app.handle_input(&Button::Up, &vfs);

    assert_eq!(app.current_skin, "balatro");
    assert_eq!((app.width, app.height), (1280, 720));
}

// -- Click + alternate-confirm --

#[test]
fn space_triggers_confirm() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Move cursor off the active skin.
    app.handle_input(&Button::Down, &vfs);
    // Space maps to Button::Triangle in the SDL input layer — Settings
    // should accept it as an alternate Confirm so users who try Space
    // before discovering Enter still get feedback.
    app.handle_input(&Button::Triangle, &vfs);
    assert!(
        take_req(&mut app).is_some(),
        "Space/Triangle should confirm the selection"
    );
}

/// Windowed content size used by the click tests.
const CW: u32 = 480;
const CH: u32 = 260;

/// Content-local point in the middle of the row showing `item`, using
/// the same layout + scroll the renderer uses. `x_frac` positions the
/// point across the row (0.0 = left edge, 1.0 = right edge).
fn item_point(app: &SettingsApp, item: usize, x_frac: f32) -> (i32, i32) {
    let l = app.settings_layout(0, 0, CW, CH);
    let rows = app.rows();
    let scroll = app.body_scroll(&rows, l.visible_rows());
    let row = row_of_item(&rows, item).expect("item has a row");
    let r = l.row_rect(row - scroll);
    (r.x + (r.w as f32 * x_frac) as i32, r.y + r.h as i32 / 2)
}

#[test]
fn click_on_skin_row_applies() {
    let mut app = make_app();
    let (x, y) = item_point(&app, 1, 0.2);
    let _action = click_at(&mut app, x, y, CW, CH, &MemoryVfs::new());
    let req = take_req(&mut app);
    let (path, data) = req.expect("click on non-active skin should post IPC");
    assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
    // builtin_names()[1] is the second skin (not "classic").
    assert_eq!(data, builtin_names()[1]);
}

#[test]
fn click_on_active_skin_noop() {
    let mut app = make_app();
    // Item 0 = "classic", which is the active skin.
    let (x, y) = item_point(&app, 0, 0.2);
    let _action = click_at(&mut app, x, y, CW, CH, &MemoryVfs::new());
    assert!(take_req(&mut app).is_none());
}

#[test]
fn click_above_content_area_ignored() {
    let mut app = make_app();
    // Top-left padding above the tab strip: nothing to hit.
    let _action = click_at(&mut app, 1, 1, CW, CH, &MemoryVfs::new());
    assert!(take_req(&mut app).is_none());
    assert_eq!(app.category, Category::Display);
}

#[test]
fn click_on_tab_switches_category() {
    let mut app = make_app();
    let l = app.settings_layout(0, 0, CW, CH);
    let audio = Category::ALL
        .iter()
        .position(|c| *c == Category::Audio)
        .expect("audio tab");
    let t = l.tabs[audio];
    click_at(&mut app, t.x + 2, t.y + 2, CW, CH, &MemoryVfs::new());
    assert_eq!(app.category, Category::Audio);
}

#[test]
fn click_on_resolution_preset_applies() {
    let vfs = make_vfs();
    let mut app = make_app();
    // Navigate to Resolution category.
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.category, Category::Resolution);
    // Click the 4th preset (1280x720, index 3).
    let (x, y) = item_point(&app, 3, 0.2);
    let _action = click_at(&mut app, x, y, CW, CH, &vfs);
    let req = take_req(&mut app);
    let (path, data) = req.expect("click on non-active preset should post IPC");
    assert_eq!(path, RESOLUTION_CHANGE_REQUEST_PATH);
    assert_eq!(data, "1280x720");
}

// -- Audio / Language / Accessibility --

/// Navigate a fresh app to `category`.
fn app_in(category: Category, vfs: &MemoryVfs) -> SettingsApp {
    let mut app = make_app();
    while app.category != category {
        app.handle_input(&Button::Right, vfs);
    }
    app
}

#[test]
fn volume_change_emits_ipc_request() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Audio, &vfs);
    app.handle_input(&Button::Up, &vfs);
    let (path, data) = take_req(&mut app).expect("volume posts IPC");
    assert_eq!(path, VOLUME_CHANGE_REQUEST_PATH);
    assert_eq!(data, "85");
    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Down, &vfs);
    // Both steps are queued and delivered in order.
    let (_, data) = take_req(&mut app).expect("volume posts IPC");
    assert_eq!(data, "80");
    let (_, data) = take_req(&mut app).expect("volume posts IPC");
    assert_eq!(data, "75");
    assert!(take_req(&mut app).is_none());
}

#[test]
fn volume_at_limit_posts_nothing() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Audio, &vfs);
    app.volume = 100;
    app.handle_input(&Button::Up, &vfs);
    assert!(take_req(&mut app).is_none());
}

#[test]
fn volume_plus_minus_keys() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Audio, &vfs);
    let consumed = app.handle_key(&Key::Char('-'), Modifiers::default(), &vfs);
    assert_eq!(consumed, Some(AppAction::None));
    assert_eq!(app.volume, 75);
    // Outside slider categories the keys fall through.
    let mut other = make_app();
    assert!(
        other
            .handle_key(&Key::Char('+'), Modifiers::default(), &vfs)
            .is_none()
    );
}

#[test]
fn click_on_volume_slider_sets_value() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Audio, &vfs);
    let l = app.settings_layout(0, 0, CW, CH);
    let rows = app.rows();
    let row = row_of_item(&rows, 0).expect("volume row");
    let track = l.control_rect(l.row_rect(row));
    // Click near the left end of the track -> low volume.
    click_at(&mut app, track.x + 1, track.y + 2, CW, CH, &vfs);
    assert!(app.volume <= 5, "volume {}", app.volume);
    let (path, _) = take_req(&mut app).expect("slider click posts");
    assert_eq!(path, VOLUME_CHANGE_REQUEST_PATH);
}

#[test]
fn shell_volume_state_is_adopted_once() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(VOLUME_STATE_PATH, b"40").unwrap();
    let mut app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
    assert_eq!(app.volume, 40);
    // Optimistic local change survives a sync while the shell still
    // publishes the old value ...
    app.enter_category(Category::Audio);
    app.handle_input(&Button::Up, &vfs);
    assert!(!app.tick(16, &vfs));
    assert_eq!(app.volume, 45);
    // ... and a new published value is adopted.
    vfs.write(VOLUME_STATE_PATH, b"60").unwrap();
    assert!(app.tick(16, &vfs));
    assert_eq!(app.volume, 60);
}

#[test]
fn language_lists_locales_and_requests_change() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Language, &vfs);
    let lines = app.lines();
    assert!(lines.iter().any(|l| l.contains("Deutsch")));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("English") && l.contains('*')),
        "active locale marked"
    );
    // English is first; move to the next locale and confirm.
    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    let (path, data) = take_req(&mut app).expect("locale posts IPC");
    assert_eq!(path, LOCALE_CHANGE_REQUEST_PATH);
    assert_eq!(data, Locale::all()[1].code());
}

#[test]
fn language_marks_published_locale() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(LOCALE_STATE_PATH, b"fr").unwrap();
    let mut app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
    assert_eq!(app.locale, Locale::French);
    app.enter_category(Category::Language);
    let fr = Locale::all()
        .iter()
        .position(|l| *l == Locale::French)
        .expect("fr");
    assert_eq!(app.item_cursor, fr);
}

#[test]
fn accessibility_font_scale_cycles_and_posts() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Accessibility, &vfs);
    assert_eq!(app.item_cursor, A11Y_FONT_SCALE);
    app.handle_input(&Button::Confirm, &vfs);
    let (path, data) = take_req(&mut app).expect("font scale posts");
    assert_eq!(path, FONT_SCALE_REQUEST_PATH);
    assert_eq!(data.parse::<f32>().ok(), Some(1.25));
    // Square steps back down.
    app.handle_input(&Button::Square, &vfs);
    assert_eq!(app.font_scale, 1.0);
    // Confirm at the largest preset wraps to the smallest.
    app.font_scale = 1.5;
    app.handle_input(&Button::Confirm, &vfs);
    assert_eq!(app.font_scale, 0.75);
}

#[test]
fn accessibility_toggles_post_requests() {
    let vfs = make_vfs();
    let mut app = app_in(Category::Accessibility, &vfs);
    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    let (path, data) = take_req(&mut app).expect("high contrast posts");
    assert_eq!(path, HIGH_CONTRAST_REQUEST_PATH);
    assert_eq!(data, "on");

    app.handle_input(&Button::Down, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    let (path, data) = take_req(&mut app).expect("reduced motion posts");
    assert_eq!(path, REDUCED_MOTION_REQUEST_PATH);
    assert_eq!(data, "1");
    assert!(app.reduced_motion);
    assert!(
        app.lines()
            .iter()
            .any(|l| l.contains("Reduced Motion: [ON]"))
    );
}

#[test]
fn high_contrast_toggle_reflects_active_skin() {
    let vfs = make_vfs();
    let mut app = SettingsApp::new("/apps/settings", HIGH_CONTRAST_SKIN, 480, 272, "SDL3");
    app.enter_category(Category::Accessibility);
    assert!(
        app.lines()
            .iter()
            .any(|l| l.contains("High Contrast: [ON]"))
    );
    app.item_cursor = A11Y_HIGH_CONTRAST;
    app.handle_input(&Button::Confirm, &vfs);
    let (_, data) = take_req(&mut app).expect("posts");
    assert_eq!(data, "off");
}

#[test]
fn windowed_draw_uses_widgets_and_theme_fonts() {
    use oasis_test_backend::{DrawCommand, RecordingBackend};

    let vfs = make_vfs();
    let mut app = app_in(Category::Accessibility, &vfs);
    let mut backend = RecordingBackend::new(CW, CH);
    let mut at = ActiveTheme::default();
    at.font_body = 15;
    app.draw_windowed(0, 0, CW, CH, &mut backend, &at)
        .expect("draw");
    let cmds = backend.commands();
    let texts: Vec<(&str, u16)> = cmds
        .iter()
        .filter_map(|c| match c {
            DrawCommand::DrawText {
                text, font_size, ..
            } => Some((text.as_str(), *font_size)),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|(t, size)| t.contains("Font Scale") && *size == 15),
        "rows use the theme's (scaled) body font: {texts:?}"
    );
    // Toggle thumbs (circles, rasterized to rects by the default
    // `fill_circle`) use the widget theme's thumb color.
    let thumb = at.ui_theme.toggle_thumb;
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            DrawCommand::FillRect { color, .. } if *color == thumb
        )),
        "toggle widgets drawn"
    );
    // `layout` records the metrics for click hit-testing.
    app.layout(CW, CH, &at);
    assert_eq!(app.metrics.font_body, 15);
}

// -- Cursor sync (no double-`>` markers) --

#[test]
fn display_lines_have_no_embedded_cursor_marker() {
    let app = make_app();
    let lines = app.lines();
    // The only `>` in the rendered output should come from
    // `draw_content_windowed`, not from build_display_lines. Verify no
    // skin row contains a `>` character embedded in the text.
    for line in lines.iter() {
        let is_skin_row = builtin_names().iter().any(|n| line.contains(n));
        if is_skin_row {
            assert!(
                !line.contains('>'),
                "skin row should not embed a cursor marker: {line:?}"
            );
        }
    }
}

#[test]
fn parse_resolution_ok() {
    assert_eq!(parse_resolution("800x600"), Some((800, 600)));
    assert_eq!(parse_resolution("  1920x1080 "), Some((1920, 1080)));
}

#[test]
fn parse_resolution_bad() {
    assert!(parse_resolution("not a resolution").is_none());
    assert!(parse_resolution("800").is_none());
    assert!(parse_resolution("ax600").is_none());
}

// -- Appearance editor --

/// Navigate a fresh app to the Appearance category.
fn appearance_app(vfs: &MemoryVfs) -> SettingsApp {
    let mut app = make_app();
    app.handle_input(&Button::Right, vfs);
    assert_eq!(app.category, Category::Appearance);
    app
}

#[test]
fn appearance_lists_base_colors_and_actions() {
    let vfs = make_vfs();
    let app = appearance_app(&vfs);
    let lines = app.lines();
    for label in BASE_COLOR_LABELS {
        assert!(
            lines.iter().any(|l| l.contains(label)),
            "missing color row {label}"
        );
    }
    assert!(lines.iter().any(|l| l.contains("Apply")));
    assert!(lines.iter().any(|l| l.contains("custom-classic")));
    assert!(lines.iter().any(|l| l.contains("Variant: Dark")));
    assert!(lines.iter().any(|l| l.contains("Variant: High Contrast")));
}

#[test]
fn appearance_rows_show_contrast_readout() {
    let vfs = make_vfs();
    let app = appearance_app(&vfs);
    let lines = app.lines();
    // Every base-color row carries a "vs <partner> N.N:1 <verdict>"
    // readout; the Text role is judged against the background.
    let text_row = lines
        .iter()
        .find(|l| l.trim_start().starts_with("Text ") && l.contains(":1"))
        .expect("Text row has a contrast readout");
    assert!(
        text_row.contains("vs Bg"),
        "Text judged vs background: {text_row}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains(" AA") || l.contains(" low")),
        "no contrast verdicts rendered"
    );
}

#[test]
fn contrast_readout_flags_low_contrast() {
    let app = SettingsApp::new("/apps/settings", "classic", 480, 272, "SDL3");
    // Background (role 0) is judged against Text (role 3).
    let readout = app.contrast_readout(0);
    assert!(
        readout.starts_with("vs Text"),
        "unexpected partner: {readout}"
    );
    assert!(
        readout.contains("AA") || readout.contains("low"),
        "missing verdict: {readout}"
    );
}

#[test]
fn appearance_palette_matches_current_skin() {
    let vfs = make_vfs();
    let app = appearance_app(&vfs);
    let classic = resolve_skin("classic").expect("classic resolves");
    let expected = parse_hex_color(&classic.theme.background).expect("valid hex");
    assert_eq!(app.appearance.colors[0], expected);
}

#[test]
fn appearance_edit_adjusts_channel() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    let before = app.appearance.colors[0];
    // Enter edit mode on Background, bump R by one step, commit.
    app.handle_input(&Button::Confirm, &vfs);
    assert!(app.appearance.editing_channel.is_some());
    app.handle_input(&Button::Up, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    assert!(app.appearance.editing_channel.is_none());
    assert_eq!(app.appearance.colors[0].r, before.r.saturating_add(8));
    assert_eq!(app.appearance.colors[0].g, before.g);
}

#[test]
fn appearance_edit_cancel_reverts() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    let before = app.appearance.colors[0];
    app.handle_input(&Button::Confirm, &vfs);
    app.handle_input(&Button::Up, &vfs);
    app.handle_input(&Button::Up, &vfs);
    // Cancel exits edit mode and restores the color (not the app).
    let action = app.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::None);
    assert!(app.appearance.editing_channel.is_none());
    assert_eq!(app.appearance.colors[0], before);
}

#[test]
fn appearance_edit_captures_left_right() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    app.handle_input(&Button::Confirm, &vfs);
    // Left/Right cycle channels instead of switching categories.
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.category, Category::Appearance);
    assert_eq!(app.appearance.editing_channel, Some(1));
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.appearance.editing_channel, Some(0));
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.appearance.editing_channel, Some(2));
}

#[test]
fn appearance_apply_posts_theme_toml() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    // Move to the "Apply" row (first action after the 9 colors).
    for _ in 0..BASE_COLOR_LABELS.len() {
        app.handle_input(&Button::Down, &vfs);
    }
    app.handle_input(&Button::Confirm, &vfs);
    let (path, payload) = take_req(&mut app).expect("apply posts IPC");
    assert_eq!(path, SKIN_APPLY_THEME_REQUEST_PATH);
    let theme = SkinTheme::from_toml_str(&payload).expect("payload is a valid theme");
    assert_eq!(
        parse_hex_color(&theme.background),
        Some(app.appearance.colors[0])
    );
}

#[test]
fn appearance_save_posts_named_payload() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    for _ in 0..BASE_COLOR_LABELS.len() + 1 {
        app.handle_input(&Button::Down, &vfs);
    }
    app.handle_input(&Button::Confirm, &vfs);
    let (path, payload) = take_req(&mut app).expect("save posts IPC");
    assert_eq!(path, SKIN_SAVE_CUSTOM_REQUEST_PATH);
    let (name, theme_toml) = payload.split_once('\n').expect("name line present");
    assert_eq!(name, "custom-classic");
    assert!(SkinTheme::from_toml_str(theme_toml).is_ok());
}

#[test]
fn appearance_variant_transforms_palette_and_previews() {
    let vfs = make_vfs();
    let mut app = appearance_app(&vfs);
    // classic is dark; the Light variant row should flip the background
    // into the light half and immediately post a preview.
    let light_row = BASE_COLOR_LABELS.len() + 2 + 1; // Apply, Save, Dark, Light
    for _ in 0..light_row {
        app.handle_input(&Button::Down, &vfs);
    }
    app.handle_input(&Button::Confirm, &vfs);
    let (path, _) = take_req(&mut app).expect("variant posts preview");
    assert_eq!(path, SKIN_APPLY_THEME_REQUEST_PATH);
    let bg = app.appearance.colors[0];
    let luma = 0.2126 * bg.r as f32 + 0.7152 * bg.g as f32 + 0.0722 * bg.b as f32;
    assert!(luma > 127.0, "light variant background not light: {bg:?}");
}

#[test]
fn custom_skin_name_does_not_stack_prefix() {
    let app = SettingsApp::new("/apps/settings", "custom-classic", 480, 272, "SDL3");
    assert_eq!(app.custom_skin_name(), "custom-classic");
}

#[test]
fn appearance_palette_reload_on_external_skin_change() {
    let mut vfs = MemoryVfs::new();
    let mut app = appearance_app(&vfs);
    let before = app.appearance.colors[0];
    // Shell swaps to paper (a light skin) behind our back.
    vfs.mkdir("/system").unwrap();
    vfs.mkdir("/system/state").unwrap();
    vfs.write(SKIN_STATE_PATH, b"paper").unwrap();
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.appearance.loaded_for, "paper");
    assert_ne!(app.appearance.colors[0], before);
}
