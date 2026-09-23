#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! User journeys through the real desktop shell, spanning several apps.
//!
//! Per-app e2e tests (`oasis-app-core`'s testing feature) drive one app in
//! isolation. These scenarios boot the whole shell through
//! `oasis_app::harness::Harness` and act only like a user would: clicking
//! on things located in the rendered frame, pressing keys, typing. They
//! assert on what reached the screen, the VFS, the recording audio output
//! and (for persistence) a real settings file in a temp directory.

use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use oasis_app::Mode;
use oasis_app::harness::{Harness, HarnessOptions};
use oasis_app::headless::DrawnText;
use oasis_core::input::{Key, Modifiers};
use oasis_core::vfs::Vfs;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The UI locale is process-global. Scenarios that switch it hold the
/// write lock; everything else holds a read lock so no English-text
/// assertion runs while another scenario has the UI in German.
static LOCALE_LOCK: RwLock<()> = RwLock::new(());

fn shared() -> RwLockReadGuard<'static, ()> {
    LOCALE_LOCK.read().unwrap_or_else(|e| e.into_inner())
}

fn exclusive() -> RwLockWriteGuard<'static, ()> {
    LOCALE_LOCK.write().unwrap_or_else(|e| e.into_inner())
}

/// Put the UI back to English when a locale scenario ends (even on panic).
struct EnglishOnDrop;

impl Drop for EnglishOnDrop {
    fn drop(&mut self) {
        oasis_core::i18n::set_ui_locale(oasis_core::i18n::Locale::English);
    }
}

/// Save the framebuffer when a scenario fails.
fn dump(h: &Harness, name: &str) -> String {
    let dir = std::env::temp_dir().join("oasis-e2e");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.png"));
    match h.save_png(&path) {
        Ok(()) => format!("(frame saved to {})", path.display()),
        Err(e) => format!("(frame dump failed: {e})"),
    }
}

/// A fresh settings-file path in a per-scenario temp directory.
fn temp_prefs(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oasis-e2e-prefs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("settings.toml")
}

fn boot_with_prefs(skin: &str, prefs: &Path) -> Harness {
    let mut opts = HarnessOptions::new(skin);
    opts.prefs_path = Some(prefs.to_path_buf());
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    h
}

/// Boot `skin` and let the entrance finish.
fn boot(skin: &str) -> Harness {
    let mut h = Harness::new(skin);
    h.settle();
    h
}

/// Whether painted `shown` is `text` cut short with an ellipsis
/// (`"oasis_sa…"` for `"oasis_sample.png"`).
fn is_ellipsized(shown: &str, text: &str) -> bool {
    let shown = shown.trim();
    shown
        .strip_suffix('\u{2026}')
        .or_else(|| shown.strip_suffix("..."))
        .or_else(|| shown.strip_suffix(".."))
        .is_some_and(|stem| stem.chars().count() >= 3 && text.starts_with(stem))
}

/// The topmost (last drawn) text call exactly equal to `text`, else the
/// last one containing it, else one showing it ellipsized, inside `area`
/// (`x, y, w, h`) when given.
fn find_drawn(h: &Harness, text: &str, area: Option<(i32, i32, u32, u32)>) -> Option<DrawnText> {
    let inside = |t: &DrawnText| {
        area.is_none_or(|(x, y, w, hh)| {
            t.x >= x && t.y >= y && t.x < x + w as i32 && t.y < y + hh as i32
        })
    };
    let calls = h.frame_text_calls();
    let pick = |f: &dyn Fn(&DrawnText) -> bool| calls.iter().rev().find(|t| inside(t) && f(t));
    pick(&|t| t.text.trim() == text)
        .or_else(|| pick(&|t| t.text.contains(text)))
        .or_else(|| pick(&|t| is_ellipsized(&t.text, text)))
        .cloned()
}

/// Render, then click on the painted string `text` inside window `win`
/// (or anywhere when `win` is `None`). Panics when it isn't on screen.
fn click_text_in(h: &mut Harness, win: Option<&str>, text: &str) {
    h.render_now();
    let area = win.map(|w| {
        h.find_window(w)
            .unwrap_or_else(|| panic!("window {w} open: {:?}", h.windows()))
            .content
    });
    let t = find_drawn(h, text, area).unwrap_or_else(|| {
        panic!(
            "'{text}' painted in {win:?}: {:?} {}",
            h.frame_text(),
            dump(
                h,
                &format!("missing_{}", text.replace(['/', ' ', '.'], "_"))
            )
        )
    });
    h.click(t.x + 3, t.y + i32::from(t.font_size) / 2);
}

/// Double-click painted text (two clicks a frame apart).
fn double_click_text_in(h: &mut Harness, win: Option<&str>, text: &str) {
    click_text_in(h, win, text);
    click_text_in(h, win, text);
}

fn drawn_in_window(h: &mut Harness, win: &str) -> Vec<String> {
    h.render_now();
    let (x, y, w, hh) = h.find_window(win).unwrap().content;
    h.frame_text_calls()
        .iter()
        .filter(|t| t.x >= x && t.y >= y && t.x < x + w as i32 && t.y < y + hh as i32)
        .map(|t| t.text.clone())
        .collect()
}

fn window_shows(h: &mut Harness, win: &str, needle: &str) -> bool {
    drawn_in_window(h, win)
        .iter()
        .any(|t| t.contains(needle) || is_ellipsized(t, needle))
}

/// Press Backspace `n` times.
fn backspace(h: &mut Harness, n: usize) {
    for _ in 0..n {
        h.key(Key::Backspace);
    }
}

fn ctrl(h: &mut Harness, c: char) {
    h.key_with(Key::Char(c), Modifiers::CTRL);
}

/// Click the taskbar button labelled `title` (bottom strip of the screen).
fn click_taskbar(h: &mut Harness, title: &str) {
    h.render_now();
    let (sw, sh) = h.size();
    let bar = (0, sh as i32 - 40, sw, 40);
    let t = find_drawn(h, title, Some(bar))
        .unwrap_or_else(|| panic!("taskbar button '{title}': {:?}", h.frame_text()));
    h.click(t.x + 3, t.y + i32::from(t.font_size) / 2);
    h.settle();
}

/// Bring a window to the front by clicking its taskbar button (windows
/// cascade over each other's titlebars; the taskbar is always reachable).
fn focus_window(h: &mut Harness, title: &str) {
    let w = h
        .find_window(title)
        .unwrap_or_else(|| panic!("{title} open: {:?}", h.windows()));
    if h.state().wm.active_window() == Some(w.id.as_str()) {
        return;
    }
    click_taskbar(h, title);
    assert_eq!(
        h.state().wm.active_window(),
        Some(w.id.as_str()),
        "clicking the {title} taskbar button focuses it"
    );
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Open Settings from the dashboard and switch to `tab` by clicking it.
fn settings_tab(h: &mut Harness, tab: &str) {
    if h.find_window("Settings").is_none() {
        assert!(h.click_app_icon("Settings"), "{:?}", h.dashboard_apps());
        h.settle();
    }
    click_text_in(h, Some("Settings"), tab);
    h.settle();
}

#[test]
fn settings_changes_apply_live_and_persist_across_reboot() {
    let _g = shared();
    let prefs = temp_prefs("settings");
    let mut h = boot_with_prefs("classic", &prefs);
    assert_eq!(h.state().skin.manifest.name, "classic");

    // -- Audio: Up raises the master volume in 5% steps. --
    settings_tab(&mut h, "Audio");
    let v0 = h.audio().volume;
    h.key(Key::Up);
    h.key(Key::Up);
    h.settle();
    let v1 = v0 + 10;
    assert_eq!(h.audio().volume, v1, "volume reached the audio output");
    assert!(
        window_shows(&mut h, "Settings", &format!("{v1}%")),
        "slider shows {v1}%: {:?}",
        drawn_in_window(&mut h, "Settings")
    );

    // -- Accessibility: font scale up, reduced motion on. --
    settings_tab(&mut h, "Accessibil");
    let font_before = h.state().active_theme.font_body;
    // Select the font-scale slider row, Enter steps it up one preset.
    click_text_in(&mut h, Some("Settings"), "Font");
    h.key(Key::Enter);
    h.settle();
    assert!(
        h.state().active_theme.font_body > font_before,
        "font scale grew the body font ({font_before} -> {})",
        h.state().active_theme.font_body
    );
    assert!(window_shows(&mut h, "Settings", "125%"));
    assert!(!h.state().skin.features.reduced_motion);
    click_text_in(&mut h, Some("Settings"), "Reduce");
    h.settle();
    assert!(
        h.state().skin.features.reduced_motion,
        "reduced motion applied to the running skin"
    );

    // -- High contrast on and back off. --
    click_text_in(&mut h, Some("Settings"), "High");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "highcontrast");
    h.render_now();
    let hc_bg = h.state().bg_color;
    click_text_in(&mut h, Some("Settings"), "High");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "classic", "back to classic");
    assert_ne!(h.state().bg_color, hc_bg);

    // -- Resolution: 800x600. --
    settings_tab(&mut h, "Resolution");
    click_text_in(&mut h, Some("Settings"), "800x600");
    h.settle();
    assert_eq!(h.size(), (800, 600), "framebuffer resized");
    h.render_now();
    assert!(h.distinct_colors() > 8, "{}", dump(&h, "settings_800x600"));

    // -- Display: switch skin to xp (scroll the list down to it first). --
    settings_tab(&mut h, "Display");
    for _ in 0..20 {
        if drawn_in_window(&mut h, "Settings")
            .iter()
            .any(|t| t == "xp")
        {
            break;
        }
        h.key(Key::Down);
    }
    click_text_in(&mut h, Some("Settings"), "xp");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "xp", "skin applied live");
    assert_eq!(h.size(), (800, 600), "resolution kept across the skin swap");

    // Everything the user changed is in the settings file.
    h.shutdown().unwrap();
    let saved = std::fs::read_to_string(&prefs).expect("settings file written");
    for needle in ["xp", "800", "1.25"] {
        assert!(saved.contains(needle), "'{needle}' persisted: {saved}");
    }

    // -- Re-boot: the preferences come back. --
    let mut h = boot_with_prefs("classic", &prefs);
    assert_eq!(h.state().skin.manifest.name, "xp", "skin restored");
    assert_eq!(h.size(), (800, 600), "resolution restored");
    assert_eq!(h.audio().volume, v1, "volume restored to the output");
    assert!(
        h.state().skin.features.reduced_motion,
        "reduced motion restored"
    );
    let fresh_xp = Harness::new("xp").state().active_theme.font_body;
    assert!(
        h.state().active_theme.font_body > fresh_xp,
        "font scale restored"
    );
    // And Settings shows the restored values.
    settings_tab(&mut h, "Audio");
    assert!(
        window_shows(&mut h, "Settings", &format!("{v1}%")),
        "{:?}",
        drawn_in_window(&mut h, "Settings")
    );
    let _ = std::fs::remove_dir_all(prefs.parent().unwrap());
}

// ---------------------------------------------------------------------------
// Locale
// ---------------------------------------------------------------------------

/// Translation-key placeholders (`settings.cat_audio`) are what `tr!`
/// returns for a missing key. None may reach the screen.
fn assert_no_key_placeholders(h: &Harness, what: &str) {
    let en = include_str!("../../oasis-i18n/translations/en.toml");
    let sections: Vec<&str> = en
        .lines()
        .filter_map(|l| l.trim().strip_prefix('[')?.strip_suffix(']'))
        .collect();
    for t in h.frame_text() {
        for s in &sections {
            let pat = format!("{s}.");
            if let Some(i) = t.find(&pat) {
                let rest = &t[i + pat.len()..];
                assert!(
                    !rest.starts_with(|c: char| c.is_ascii_lowercase()),
                    "{what}: key placeholder painted: '{t}'"
                );
            }
        }
    }
}

#[test]
fn locale_switch_translates_the_shell_and_persists() {
    let _g = exclusive();
    let _english = EnglishOnDrop;
    let prefs = temp_prefs("locale");
    let mut h = boot_with_prefs("xp", &prefs);

    settings_tab(&mut h, "Language");
    click_text_in(&mut h, Some("Settings"), "Deutsch");
    h.settle();
    assert_eq!(
        oasis_core::i18n::get_locale(),
        oasis_core::i18n::Locale::German
    );
    // Settings itself re-renders in German.
    let drawn = drawn_in_window(&mut h, "Settings");
    for de in ["Anzeige", "Sprache", "Barriere"] {
        assert!(
            drawn.iter().any(|t| t.contains(de)),
            "Settings tab '{de}' painted: {drawn:?}"
        );
    }
    assert!(!drawn.iter().any(|t| t == "Display"), "{drawn:?}");
    h.render_now();
    assert_no_key_placeholders(&h, "settings (de)");
    // The taskbar entry (ellipsized to fit) and the dashboard icon label
    // behind the window.
    let (sw, sh) = h.size();
    assert!(
        find_drawn(&h, "Einstellungen", Some((0, sh as i32 - 40, sw, 40))).is_some(),
        "taskbar title translated: {:?}",
        h.frame_text()
    );
    assert!(
        h.frame_text().iter().any(|t| t == "Einstellungen"),
        "dashboard label translated: {:?}",
        h.frame_text()
    );
    // Known gap: the WM titlebar still paints the English window title
    // (`oasis-wm` draws `Window::title` untranslated).

    // Close Settings; the dashboard labels are German too.
    assert!(h.close_window("Settings"));
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
    h.render_now();
    assert!(
        h.sdi_text_contains("Einstellungen"),
        "dashboard icon label translated: {:?}",
        h.sdi_texts()
    );
    assert_no_key_placeholders(&h, "dashboard (de)");

    // Start menu labels.
    let (x, y, w, bh) = h.sdi_rect("start_btn_bg").expect("xp start button");
    h.click(x + w as i32 / 2, y + bh as i32 / 2);
    h.settle();
    assert!(h.state().ui.start_menu.open);
    h.render_now();
    assert_no_key_placeholders(&h, "start menu (de)");
    assert!(
        h.sdi_text_contains("Einstellungen"),
        "start menu Settings item translated: {:?}",
        h.sdi_texts()
    );
    h.shutdown().unwrap();

    // Re-boot restores German.
    oasis_core::i18n::set_ui_locale(oasis_core::i18n::Locale::English);
    let mut h = boot_with_prefs("xp", &prefs);
    assert_eq!(
        oasis_core::i18n::get_locale(),
        oasis_core::i18n::Locale::German,
        "locale restored at boot"
    );
    h.render_now();
    assert!(h.sdi_text_contains("Einstellungen"), "{:?}", h.sdi_texts());
    let _ = std::fs::remove_dir_all(prefs.parent().unwrap());
}

// ---------------------------------------------------------------------------
// Files: File Manager <-> Text Editor <-> Photo Viewer
// ---------------------------------------------------------------------------

#[test]
fn file_manager_and_text_editor_round_trip() {
    let _g = shared();
    let mut h = boot("classic");
    assert!(h.click_app_icon("File Manager"));
    h.settle();
    assert!(window_shows(&mut h, "File Manager", "home"));

    // New folder (Ctrl+Shift+N): the name prompt is prefilled, replace it.
    h.key_with(Key::Char('n'), Modifiers::CTRL | Modifiers::SHIFT);
    h.settle();
    assert!(
        window_shows(&mut h, "File Manager", "new_folder"),
        "name prompt: {:?}",
        drawn_in_window(&mut h, "File Manager")
    );
    backspace(&mut h, "new_folder".len());
    h.type_text("journey");
    h.key(Key::Enter);
    h.settle();
    assert!(h.vfs().exists("/journey"), "folder created in the VFS");
    assert!(
        window_shows(&mut h, "File Manager", "journey"),
        "new folder listed: {:?}",
        drawn_in_window(&mut h, "File Manager")
    );
    // Go into it (double-click the tile).
    double_click_text_in(&mut h, Some("File Manager"), "journey");
    h.settle();
    assert!(
        window_shows(&mut h, "File Manager", "/journey"),
        "address bar shows the folder: {:?}",
        drawn_in_window(&mut h, "File Manager")
    );

    // Text Editor from the dashboard (icon not covered by the window).
    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    assert!(h.find_window("Text Editor").is_some(), "{:?}", h.windows());
    h.type_text("Hello from the journey");
    ctrl(&mut h, 's');
    h.settle();
    // Untitled: Save As prompt prefilled with a default path.
    backspace(&mut h, "/home/user/untitled.txt".len());
    h.type_text("/journey/note.txt");
    h.key(Key::Enter);
    h.settle();
    assert_eq!(
        h.vfs().read("/journey/note.txt").unwrap(),
        b"Hello from the journey",
        "saved into the new folder"
    );
    h.render_now();
    assert!(
        h.text_drawn_contains("note.txt"),
        "editor title shows the file name: {:?}",
        h.frame_text()
    );
    // Close the editor (saved, so no prompt).
    assert!(h.close_window("Text Editor") || h.close_window("Text Editor - note.txt"));
    h.settle();

    // Back in the File Manager, the file is listed in the open folder
    // (open panels re-scan the VFS twice a second).
    focus_window(&mut h, "File Manager");
    h.advance(std::time::Duration::from_secs(1));
    assert!(
        window_shows(&mut h, "File Manager", "note.txt"),
        "saved file listed without re-navigating: {:?} {}",
        drawn_in_window(&mut h, "File Manager"),
        dump(&h, "fm_after_save")
    );

    // Double-click opens it in the Text Editor with its contents.
    double_click_text_in(&mut h, Some("File Manager"), "note.txt");
    h.settle();
    let editor = h
        .windows()
        .into_iter()
        .find(|w| w.title.starts_with("Text Editor"))
        .unwrap_or_else(|| panic!("editor opened: {:?}", h.windows()));
    assert!(
        window_shows(&mut h, &editor.title, "Hello from the journey"),
        "contents shown: {:?}",
        drawn_in_window(&mut h, &editor.title)
    );
    assert!(h.close_window(&editor.title));
    h.settle();

    // Rename (F2) with the tile selected.
    focus_window(&mut h, "File Manager");
    click_text_in(&mut h, Some("File Manager"), "note.txt");
    h.key(Key::F(2));
    h.settle();
    backspace(&mut h, "note.txt".len());
    h.type_text("renamed.txt");
    h.key(Key::Enter);
    h.settle();
    assert!(!h.vfs().exists("/journey/note.txt"));
    assert_eq!(
        h.vfs().read("/journey/renamed.txt").unwrap(),
        b"Hello from the journey"
    );
    assert!(window_shows(&mut h, "File Manager", "renamed.txt"));

    // Delete (Del, confirm with y).
    click_text_in(&mut h, Some("File Manager"), "renamed.txt");
    h.key(Key::Delete);
    h.settle();
    h.key(Key::Char('y'));
    h.settle();
    assert!(!h.vfs().exists("/journey/renamed.txt"), "file deleted");
    let drawn = drawn_in_window(&mut h, "File Manager");
    assert!(
        !drawn.iter().any(|t| t.trim() == "renamed.txt"),
        "tile gone: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t.contains("Deleted renamed.txt")),
        "status reports the delete: {drawn:?}"
    );
}

#[test]
fn photo_viewer_from_file_manager_renders_the_image() {
    let _g = shared();
    let mut h = boot("classic");
    assert!(h.click_app_icon("File Manager"));
    h.settle();
    for dir in ["home", "user", "photos"] {
        double_click_text_in(&mut h, Some("File Manager"), dir);
        h.settle();
    }
    double_click_text_in(&mut h, Some("File Manager"), "oasis_sample.png");
    h.settle();
    let viewer = h
        .find_window("Photo Viewer")
        .unwrap_or_else(|| panic!("Photo Viewer opened: {:?}", h.windows()));
    h.render_now();

    // Decode the sample ourselves and look for its colors in the window.
    let png = h.vfs().read("/home/user/photos/oasis_sample.png").unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let bpp = info.color_type.samples();
    let img_colors: std::collections::HashSet<[u8; 3]> = buf[..info.buffer_size()]
        .chunks_exact(bpp)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    let (x, y, w, hh) = viewer.content;
    let mut hits = 0usize;
    let mut total = 0usize;
    for py in y.max(0)..(y + hh as i32) {
        for px in x.max(0)..(x + w as i32) {
            let p = h.pixel(px as u32, py as u32);
            total += 1;
            if img_colors.contains(&[p[0], p[1], p[2]]) {
                hits += 1;
            }
        }
    }
    // The sample fits the window, so (nearly) every image pixel lands.
    let img_px = (info.width * info.height) as usize;
    assert!(img_px < total, "sample smaller than the window");
    assert!(
        hits * 10 >= img_px * 9,
        "image pixels painted in the viewer: {hits} of {img_px} {}",
        dump(&h, "photo_viewer")
    );
}

// ---------------------------------------------------------------------------
// Music
// ---------------------------------------------------------------------------

#[test]
fn music_player_plays_switches_tracks_and_stops_on_close() {
    let _g = shared();
    let mut h = boot("classic");
    // A second track, made from the terminal like a user would.
    h.terminal("cp /home/user/music/ambient_dawn.mp3 /home/user/music/zz_second.mp3");
    h.settle();
    assert!(h.vfs().exists("/home/user/music/zz_second.mp3"));
    h.key(Key::Escape); // terminal -> dashboard
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);

    assert!(h.click_app_icon("Music Player"));
    h.settle();
    assert!(window_shows(&mut h, "Music Player", "ambient_dawn.mp3"));

    // Add both tracks to the playlist (click to select, Triangle = Space),
    // then double-click the first to play it.
    click_text_in(&mut h, Some("Music Player"), "zz_second.mp3");
    h.key(Key::Space);
    click_text_in(&mut h, Some("Music Player"), "ambient_dawn.mp3");
    h.key(Key::Space);
    double_click_text_in(&mut h, Some("Music Player"), "ambient_dawn.mp3");
    h.settle();
    let first = h.state().media_track.expect("track playing");
    {
        let a = h.audio();
        assert!(a.tracks_opened.contains(&first));
        assert_eq!(a.playing, Some(first), "track is playing");
    }
    assert!(
        window_shows(&mut h, "Music Player", "ambient"),
        "{:?}",
        drawn_in_window(&mut h, "Music Player")
    );

    // Next track (Right).
    h.key(Key::Right);
    h.settle();
    let second = h.state().media_track.expect("next track playing");
    assert_ne!(second, first);
    {
        let a = h.audio();
        assert!(a.tracks_unloaded.contains(&first), "previous track freed");
        assert_eq!(a.playing, Some(second));
    }

    // System volume from Settings reaches the output while music plays;
    // switching to another app keeps the music playing.
    settings_tab(&mut h, "Audio");
    let v = h.audio().volume;
    h.key(Key::Down);
    h.settle();
    assert_eq!(h.audio().volume, v - 5);
    assert_eq!(h.audio().playing, Some(second), "music kept playing");
    assert!(h.close_window("Settings"));
    h.settle();

    // Closing the Music Player window stops and frees the track.
    assert!(h.close_window("Music Player"));
    h.settle();
    assert!(h.state().media_track.is_none());
    let a = h.audio();
    assert_eq!(a.playing, None, "stopped on close");
    assert!(a.tracks_unloaded.contains(&second));
}

// ---------------------------------------------------------------------------
// Radio (offline)
// ---------------------------------------------------------------------------

#[test]
fn radio_tune_offline_reports_an_error_and_favorites_persist() {
    let _g = shared();
    let mut h = boot("classic");
    assert!(h.click_app_icon("Internet Radio"));
    h.settle();
    let t0 = std::time::Instant::now();
    h.key(Key::Enter); // tune the selected station
    h.run_frames(30);
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(5),
        "tuning offline must not block the frame loop"
    );
    h.render_now();
    let drawn = drawn_in_window(&mut h, "Internet Radio");
    assert!(
        drawn.iter().any(|t| t.to_lowercase().contains("offline")),
        "offline error shown: {drawn:?} {}",
        dump(&h, "radio_offline")
    );
    assert!(h.state().radio_source.is_none(), "no stream opened");

    // Favorite the station (Triangle = Space); the registry is saved.
    let before = h.vfs().read("/etc/radio/stations.toml").unwrap_or_default();
    h.key(Key::Space);
    h.settle();
    let after = h.vfs().read("/etc/radio/stations.toml").unwrap();
    assert_ne!(before, after, "favorite saved to the station registry");

    // Stop and close cleanly.
    assert!(h.close_window("Internet Radio"));
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
}

// ---------------------------------------------------------------------------
// Terminal <-> apps
// ---------------------------------------------------------------------------

#[test]
fn files_made_in_the_terminal_show_up_in_the_file_manager() {
    let _g = shared();
    let mut h = boot("classic");
    h.terminal("mkdir /fromterm");
    h.terminal("write /fromterm/hello.txt typed in the terminal");
    h.settle();
    assert_eq!(
        h.vfs().read("/fromterm/hello.txt").unwrap(),
        b"typed in the terminal",
        "{:?}",
        h.state().terminal.output_lines
    );
    h.key(Key::Escape);
    h.settle();

    assert!(h.click_app_icon("File Manager"));
    h.settle();
    double_click_text_in(&mut h, Some("File Manager"), "fromterm");
    h.settle();
    assert!(
        window_shows(&mut h, "File Manager", "hello.txt"),
        "{:?}",
        drawn_in_window(&mut h, "File Manager")
    );
    double_click_text_in(&mut h, Some("File Manager"), "hello.txt");
    h.settle();
    let editor = h
        .windows()
        .into_iter()
        .find(|w| w.title.starts_with("Text Editor"))
        .unwrap_or_else(|| panic!("{:?}", h.windows()));
    assert!(window_shows(&mut h, &editor.title, "typed in the terminal"));
}

// ---------------------------------------------------------------------------
// Several apps at once
// ---------------------------------------------------------------------------

#[test]
fn switching_between_open_windows_keeps_each_apps_state() {
    let _g = shared();
    let mut h = boot("classic");

    // Launch order keeps each next dashboard icon clear of the cascaded
    // windows (the icons behind a window aren't clickable).
    assert!(h.click_app_icon("Calculator"));
    h.settle();
    h.type_text("12+30=");
    h.settle();
    assert!(
        window_shows(&mut h, "Calculator", "42"),
        "{:?}",
        drawn_in_window(&mut h, "Calculator")
    );

    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    h.type_text("draft one");

    assert!(h.click_app_icon("File Manager"));
    h.settle();
    double_click_text_in(&mut h, Some("File Manager"), "home");
    h.settle();
    assert_eq!(h.windows().len(), 3, "{:?}", h.windows());

    // Round-robin focus: each window still shows its own state.
    focus_window(&mut h, "Text Editor");
    h.type_text(" + more");
    assert!(
        window_shows(&mut h, "Text Editor", "draft one + more"),
        "{:?}",
        drawn_in_window(&mut h, "Text Editor")
    );
    focus_window(&mut h, "Calculator");
    assert!(window_shows(&mut h, "Calculator", "42"));
    focus_window(&mut h, "File Manager");
    assert!(
        window_shows(&mut h, "File Manager", "/home"),
        "{:?}",
        drawn_in_window(&mut h, "File Manager")
    );

    // Minimize (taskbar click on the active window) and restore (click
    // again) keeps the Calculator's state.
    focus_window(&mut h, "Calculator");
    click_taskbar(&mut h, "Calculator");
    assert!(h.find_window("Calculator").unwrap().minimized, "minimized");
    click_taskbar(&mut h, "Calculator");
    assert!(!h.find_window("Calculator").unwrap().minimized, "restored");
    assert!(window_shows(&mut h, "Calculator", "42"));

    // Alt+Tab cycles focus; typing then goes to the newly focused app.
    let before = h.state().wm.active_window().unwrap().to_string();
    h.key_with(Key::Tab, Modifiers::ALT);
    h.settle();
    let after = h.state().wm.active_window().unwrap().to_string();
    assert_ne!(after, before, "Alt+Tab moved focus");
    focus_window(&mut h, "Text Editor");
    assert!(window_shows(&mut h, "Text Editor", "draft one + more"));
}
