#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! End-to-end scenarios driving the real desktop shell headlessly.
//!
//! Every test boots the full shell (`oasis_app::Shell`) through
//! `oasis_app::harness::Harness`: real boot sequence, real input dispatch,
//! real per-frame ticking and real rendering into a software framebuffer.
//! See `docs/testing.md` for the harness API.

use oasis_app::Mode;
use oasis_app::harness::{Harness, HarnessOptions};
use oasis_core::input::{Button, Key, Trigger};
use oasis_core::skin::builtin::builtin_names;
use oasis_core::vfs::Vfs;

/// Save the framebuffer next to the test binary's target dir when a
/// scenario fails, so the failure can be inspected.
fn dump(h: &Harness, name: &str) -> String {
    let dir = std::env::temp_dir().join("oasis-e2e");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.png"));
    match h.save_png(&path) {
        Ok(()) => format!("(frame saved to {})", path.display()),
        Err(e) => format!("(frame dump failed: {e})"),
    }
}

/// External skin directories shipped in `skins/`.
fn external_skin_dirs() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins");
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.join("skin.toml").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// Assert the last presented frame is a real picture: many colors and
/// no single color covering (almost) the whole screen.
fn assert_not_blank(h: &Harness, what: &str) {
    let colors = h.distinct_colors();
    assert!(
        colors >= 8,
        "{what}: frame looks blank ({colors} distinct colors) {}",
        dump(h, what)
    );
    let px = h.screenshot();
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    for p in px.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    let total = px.len() / 4;
    let max = counts.values().copied().max().unwrap_or(0);
    assert!(
        max * 100 < total * 98,
        "{what}: one color covers {}% of the frame {}",
        max * 100 / total,
        dump(h, what)
    );
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

#[test]
fn every_builtin_skin_boots_to_a_rendered_dashboard() {
    assert_eq!(builtin_names().len(), 17, "update this test's expectations");
    for name in builtin_names() {
        let mut h = Harness::new(name);
        h.settle();
        h.render_now();
        assert_eq!(h.mode(), Mode::Dashboard, "{name}: boots to the dashboard");
        assert_eq!(h.state().skin.manifest.name, *name, "{name}: skin applied");
        let (w, hgt) = h.size();
        assert_eq!(
            (w, hgt),
            (
                h.state().active_theme.screen_w,
                h.state().active_theme.screen_h
            ),
            "{name}: framebuffer matches theme size"
        );
        assert!(
            h.state().active_transition.is_none(),
            "{name}: entrance finished"
        );
        assert_not_blank(&h, &format!("boot_{name}"));
        assert!(
            !h.frame_text().is_empty(),
            "{name}: some text was drawn {}",
            dump(&h, &format!("boot_{name}"))
        );
    }
}

#[test]
fn every_external_skin_directory_boots() {
    let dirs = external_skin_dirs();
    assert!(
        dirs.len() >= 10,
        "expected the skins/ directory, got {dirs:?}"
    );
    for dir in dirs {
        let name = dir.to_string_lossy().into_owned();
        let mut h = Harness::new(&name);
        h.settle();
        h.render_now();
        assert_eq!(h.mode(), Mode::Dashboard, "{name}");
        let label = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_not_blank(&h, &format!("external_{label}"));
    }
}

#[test]
fn shader_wallpaper_skins_render_through_the_bridge() {
    for name in builtin_names() {
        let mut opts = HarnessOptions::new(name);
        opts.shader_wallpaper = true;
        let mut h = Harness::with_options(opts).unwrap();
        let theme = &h.state().active_theme;
        if oasis_core::vector_overlay::get_shader_layer(theme).is_none() {
            continue;
        }
        h.run_frames(3);
        h.render_now();
        assert!(h.shell.shader_bridge.is_some(), "{name}: bridge created");
        assert_not_blank(&h, &format!("shader_{name}"));
    }
}

#[test]
fn boot_status_bar_uses_the_frozen_clock() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(
        h.sdi_text_contains("2025") || h.sdi_text_contains("12:00"),
        "status bar shows the fixed time: {:?}",
        h.sdi_texts()
    );
}

// ---------------------------------------------------------------------------
// Dashboard -> app -> dashboard
// ---------------------------------------------------------------------------

/// Click `app`'s dashboard icon, check a window opens and renders, close
/// it via its titlebar button and check the dashboard is back.
fn launch_and_close(skin: &str, app: &str) {
    let mut h = Harness::new(skin);
    h.settle();
    assert!(
        h.click_app_icon(app),
        "{skin}: '{app}' icon on page 0 of {:?}",
        h.dashboard_apps()
    );
    h.settle();
    assert_eq!(
        h.mode(),
        Mode::Desktop,
        "{skin}: launching {app} enters Desktop mode"
    );
    let win = h
        .find_window(app)
        .unwrap_or_else(|| panic!("{skin}: {app} window open; windows: {:?}", h.windows()));
    assert!(!win.minimized);
    h.render_now();
    assert!(
        h.text_drawn_contains(app),
        "{skin}: window title '{app}' painted; drawn: {:?} {}",
        h.frame_text(),
        dump(&h, &format!("launch_{skin}_{app}"))
    );

    assert!(h.close_window(app), "{skin}: {app} has a close button");
    h.settle();
    assert!(h.find_window(app).is_none(), "{skin}: {app} window closed");
    assert_eq!(h.mode(), Mode::Dashboard, "{skin}: closing the last window");
    assert!(
        h.state().content.open_runners.is_empty(),
        "{skin}: runner dropped"
    );
    h.render_now();
    assert_not_blank(&h, &format!("closed_{skin}_{app}"));
}

#[test]
fn dashboard_icon_launch_then_close_returns_to_dashboard() {
    launch_and_close("classic", "Calculator");
}

#[test]
fn icon_launch_and_close_on_every_skin() {
    for name in builtin_names() {
        let h = Harness::new(name);
        let apps = h.dashboard_apps();
        drop(h);
        if !apps.iter().any(|a| a == "Calculator") {
            continue;
        }
        launch_and_close(name, "Calculator");
    }
}

#[test]
fn keyboard_confirm_launches_the_selected_icon() {
    let mut h = Harness::new("classic");
    h.settle();
    let selected = h
        .state()
        .ui
        .dashboard
        .selected_app()
        .map(|a| a.title.clone())
        .unwrap();
    h.key(Key::Enter);
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    assert!(
        h.find_window(&selected).is_some(),
        "Enter opens '{selected}': {:?}",
        h.windows()
    );
}

#[test]
fn dpad_moves_the_dashboard_selection() {
    let mut h = Harness::new("classic");
    h.settle();
    let before = h.state().ui.dashboard.selected;
    h.key(Key::Right);
    assert_ne!(
        h.state().ui.dashboard.selected,
        before,
        "Right moves selection"
    );
    h.key(Key::Left);
    assert_eq!(
        h.state().ui.dashboard.selected,
        before,
        "Left moves it back"
    );
}

// ---------------------------------------------------------------------------
// Start menu / pages / tabs
// ---------------------------------------------------------------------------

#[test]
fn start_menu_opens_and_launches_settings() {
    let skins: Vec<&str> = builtin_names()
        .iter()
        .copied()
        .filter(|n| Harness::new(n).state().skin.features.start_menu)
        .collect();
    assert!(!skins.is_empty(), "some skin has a start menu");
    for skin in skins {
        let mut h = Harness::new(skin);
        h.settle();
        let (x, y, w, bh) = h
            .sdi_rect("start_btn_bg")
            .unwrap_or_else(|| panic!("{skin}: start button visible"));
        h.click(x + w as i32 / 2, y + bh as i32 / 2);
        h.settle();
        assert!(h.state().ui.start_menu.open, "{skin}: start menu opened");
        assert!(
            h.click_sdi_text("Settings"),
            "{skin}: Settings item visible: {:?}",
            h.sdi_texts()
        );
        h.settle();
        assert!(
            !h.state().ui.start_menu.open,
            "{skin}: menu closed after pick"
        );
        assert!(
            h.find_window("Settings").is_some(),
            "{skin}: Settings window opened: {:?} {}",
            h.windows(),
            dump(&h, &format!("startmenu_{skin}"))
        );
    }
}

#[test]
fn dashboard_page_navigation() {
    let mut tested = 0;
    for name in builtin_names() {
        let mut h = Harness::new(name);
        h.settle();
        let pages = h.state().ui.dashboard.page_count();
        if pages < 2 {
            continue;
        }
        tested += 1;
        let first = h.dashboard_apps();
        h.button(Button::Triangle);
        h.settle();
        assert_eq!(h.state().ui.dashboard.page, 1, "{name}: Triangle -> page 1");
        assert_eq!(
            h.state().ui.bottom_bar.current_page,
            1,
            "{name}: bottom bar follows"
        );
        assert_ne!(
            h.dashboard_apps(),
            first,
            "{name}: different icons on page 1"
        );
        h.render_now();
        assert_not_blank(&h, &format!("page1_{name}"));
        h.button(Button::Square);
        h.settle();
        assert_eq!(h.state().ui.dashboard.page, 0, "{name}: Square -> page 0");
    }
    if tested == 0 {
        // Every skin fits all apps on one page: paging must be a no-op.
        let mut h = Harness::new("classic");
        h.settle();
        h.button(Button::Triangle);
        assert_eq!(h.state().ui.dashboard.page, 0);
    }
}

#[test]
fn r_trigger_cycles_media_tabs() {
    let mut h = Harness::new("classic");
    h.settle();
    let before = h.state().ui.bottom_bar.active_tab;
    h.trigger(Trigger::Right);
    h.settle();
    assert_ne!(
        h.state().ui.bottom_bar.active_tab,
        before,
        "R cycles the media tab"
    );
    h.render_now();
    assert_not_blank(&h, "media_tab");
}

// ---------------------------------------------------------------------------
// Terminal + runtime skin switch
// ---------------------------------------------------------------------------

#[test]
fn terminal_runs_a_command() {
    let mut h = Harness::new("classic");
    h.settle();
    h.terminal("echo hello-e2e");
    h.settle();
    assert_eq!(h.mode(), Mode::Terminal);
    assert!(
        h.state()
            .terminal
            .output_lines
            .iter()
            .any(|l| l.contains("hello-e2e")),
        "echo output: {:?}",
        h.state().terminal.output_lines
    );
    h.render_now();
    assert!(
        h.sdi_text_contains("hello-e2e") || h.text_drawn_contains("hello-e2e"),
        "terminal output painted {}",
        dump(&h, "terminal_echo")
    );
}

#[test]
fn skin_switch_from_terminal_at_runtime() {
    let mut h = Harness::new("classic");
    h.settle();
    h.terminal("skin xp");
    h.settle();
    assert_eq!(
        h.state().skin.manifest.name,
        "xp",
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(
        !h.state().pending_wallpaper_refresh,
        "wallpaper regenerated"
    );
    h.button(Button::Cancel); // back to the dashboard
    h.settle();
    h.render_now();
    assert_not_blank(&h, "skin_switch_terminal");
}

#[test]
fn skin_switch_via_settings_ipc_round_trip() {
    let mut h = Harness::new("classic");
    h.settle();
    let classic_bg = h.state().bg_color;
    h.vfs_mut()
        .write(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH, b"vaporwave")
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "vaporwave");
    assert_ne!(h.state().bg_color, classic_bg, "theme re-derived");
    h.render_now();
    assert_not_blank(&h, "skin_switch_ipc");
    // The persisted preference follows the swap.
    let settings = String::from_utf8(
        h.vfs()
            .read(oasis_core::settings::DEFAULT_PATH)
            .unwrap_or_default(),
    )
    .unwrap();
    assert!(settings.contains("vaporwave"), "saved: {settings}");

    // And back.
    h.vfs_mut()
        .write(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH, b"classic")
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "classic");
    assert_eq!(h.state().bg_color, classic_bg);
}

// ---------------------------------------------------------------------------
// Idle frame elision
// ---------------------------------------------------------------------------

#[test]
fn idle_static_dashboard_stops_presenting() {
    let mut h = Harness::new("classic");
    h.settle();
    // Let the welcome toast expire.
    h.advance(std::time::Duration::from_secs(5));
    let before = h.rendered_frames();
    h.run_frames(120);
    let drawn = h.rendered_frames() - before;
    // Only the 1 s safety heartbeat should present in 2 s of idle time.
    assert!(
        drawn <= 3,
        "idle classic dashboard presented {drawn} of 120 frames"
    );
    // Input wakes it up again.
    h.move_to(10, 10);
    assert!(h.last_outcome().redraw, "input forces a redraw");
}

#[test]
fn resolution_change_via_settings_ipc() {
    let mut h = Harness::new("classic");
    h.settle();
    h.vfs_mut()
        .write(
            oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH,
            b"800x600",
        )
        .unwrap();
    h.settle();
    assert_eq!(
        (
            h.state().active_theme.screen_w,
            h.state().active_theme.screen_h
        ),
        (800, 600)
    );
    assert_eq!(h.size(), (800, 600), "framebuffer resized");
    h.render_now();
    assert_not_blank(&h, "resolution_800x600");
}
