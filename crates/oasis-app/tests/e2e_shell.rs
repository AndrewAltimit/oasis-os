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
    for p in px.as_chunks::<4>().0 {
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
        // Terminal-only skins (no dashboard, no window manager) rest in
        // their fullscreen terminal.
        let idle = Mode::idle_for(&h.state().skin.features);
        assert_eq!(h.mode(), idle, "{name}: boots to its idle mode");
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
        assert_eq!(h.mode(), Mode::idle_for(&h.state().skin.features), "{name}");
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

/// A skin switch used to replace the status bar with a fresh one reading
/// "00:00" until the next periodic info update (about a second later).
/// The very first frame after the switch shows the real clock, and the
/// selected top tab is kept.
#[test]
fn status_bar_keeps_the_clock_and_tab_across_a_skin_switch() {
    let mut h = Harness::new("classic");
    h.settle();
    h.trigger(Trigger::Left);
    h.settle();
    let tab = h.state().ui.status_bar.active_tab;
    h.vfs_mut()
        .write(oasis_apps::settings::SKIN_CHANGE_REQUEST_PATH, b"modern")
        .unwrap();
    h.step(&[]);
    assert_eq!(h.state().skin.manifest.name, "modern");
    assert!(
        h.sdi_text_contains("12:00"),
        "first frame after the switch shows the frozen clock: {:?}",
        h.sdi_texts()
    );
    // (The clock-widget plugin's "12:00:00" overlay is not the status bar.)
    assert!(
        !h.sdi_texts()
            .iter()
            .any(|t| t.contains("00:00") && t != "12:00:00"),
        "{:?}",
        h.sdi_texts()
    );
    assert_eq!(h.state().ui.status_bar.active_tab, tab);
}

/// `OASIS_APP=<title>` (and `Harness::open_app`) launches every registered
/// app, including the ones without a dashboard tile (Network, Package
/// Manager, System Monitor used to be "not found").
#[test]
fn every_registered_app_launches_by_title() {
    let mut h = Harness::new("classic");
    h.settle();
    let titles: Vec<String> = h
        .state()
        .content
        .apps
        .titles()
        .map(str::to_string)
        .collect();
    for title in &titles {
        let title = title.as_str();
        assert!(h.open_app(title), "{title}: not launchable by title");
        h.settle();
        assert_eq!(h.mode(), Mode::Desktop, "{title}");
        let id = title.to_lowercase().replace(' ', "_");
        assert!(
            h.state().wm.get_window(&id).is_some(),
            "{title}: no window {id:?} in {:?}",
            h.windows()
        );
    }
    // Lower-case titles resolve too.
    assert!(h.open_app("system monitor"));
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
        let has_icons = h.state().skin.features.dashboard;
        drop(h);
        if !has_icons || !apps.iter().any(|a| a == "Calculator") {
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
        // Read the feature from the skin itself: booting a whole shell per
        // skin just to filter doubled this test's run time.
        // Terminal-only skins rest in the fullscreen terminal, which has
        // no bars.
        .filter(|n| {
            oasis_core::skin::builtin::load_builtin(n).is_ok_and(|s| {
                s.features.start_menu && Mode::idle_for(&s.features) == Mode::Dashboard
            })
        })
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
        if pages < 2 || !h.state().skin.features.dashboard {
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
        !h.state().skin_assets.pending_refresh,
        "wallpaper regenerated"
    );
    h.button(Button::Cancel); // back to the dashboard
    h.settle();
    h.render_now();
    assert_not_blank(&h, "skin_switch_terminal");
}

/// `skin xp; skin @variant:dark` typed in the terminal ends on the dark
/// variant of xp (it used to derive from the skin active before the line),
/// and the shell really runs it with its own palette.
#[test]
fn chained_skin_and_variant_from_the_terminal() {
    let mut h = Harness::new("classic");
    h.settle();
    h.terminal("skin xp; skin @variant:dark");
    h.settle();
    assert_eq!(
        h.state().skin.manifest.name,
        "xp-dark",
        "{:?}",
        h.state().terminal.output_lines
    );
    // xp's desktop background is already dark navy, so compare the whole
    // derived theme (window / app chrome colors) rather than one color.
    let dark_theme = format!("{:?}", h.state().active_theme);
    h.terminal("skin xp");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "xp");
    assert_ne!(
        format!("{:?}", h.state().active_theme),
        dark_theme,
        "dark variant has its own palette"
    );
    h.button(Button::Cancel);
    h.settle();
    h.render_now();
    assert_not_blank(&h, "skin_variant_chain");
}

#[test]
fn skin_switch_via_settings_ipc_round_trip() {
    let mut h = Harness::new("classic");
    h.settle();
    let classic_bg = h.state().bg_color;
    h.vfs_mut()
        .write(oasis_apps::settings::SKIN_CHANGE_REQUEST_PATH, b"vaporwave")
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "vaporwave");
    assert_ne!(h.state().bg_color, classic_bg, "theme re-derived");
    h.render_now();
    assert_not_blank(&h, "skin_switch_ipc");
    // The persisted preference follows the swap.
    let settings = String::from_utf8(
        h.vfs()
            .read(oasis_app::settings::DEFAULT_PATH)
            .unwrap_or_default(),
    )
    .unwrap();
    assert!(settings.contains("vaporwave"), "saved: {settings}");

    // And back.
    h.vfs_mut()
        .write(oasis_apps::settings::SKIN_CHANGE_REQUEST_PATH, b"classic")
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
            oasis_apps::settings::RESOLUTION_CHANGE_REQUEST_PATH,
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

/// The dashboard's static vector background layer is baked into one
/// offscreen render target and replayed with the premultiplied composite,
/// the path the SDL shell takes. The harness used to fail that composite
/// (the headless backend did not forward it and the UE5 framebuffer did
/// not implement it), so every run drew the layer immediately instead.
#[test]
fn static_background_layer_is_baked_and_reused() {
    let mut h = Harness::new("paper");
    h.settle();
    assert_eq!(h.state().mode, Mode::Dashboard);
    assert!(
        !h.state().active_theme.background_layers.is_empty(),
        "paper has background layers"
    );
    h.render_now();
    let baked = h.resource_counts().render_targets;
    assert!(
        baked >= 1,
        "static layer baked {}",
        dump(&h, "bake_missing")
    );
    for _ in 0..5 {
        h.render_now();
    }
    assert_eq!(
        h.resource_counts().render_targets,
        baked,
        "the bake is reused, not redone per frame"
    );
    assert_not_blank(&h, "baked_background");
}

/// Writes a shell profile and a startup script during plugin init, which
/// runs before the shell's boot scripts (a stand-in for a host whose VFS
/// persists them across boots).
mod boot_script_plugin {
    use oasis_core::error::Result;
    use oasis_core::plugin::{Plugin, PluginHost, PluginInfo};

    struct WritesBootScripts;

    impl Plugin for WritesBootScripts {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("boot-scripts", "1.0.0")
        }
        fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            host.vfs.write("/home/.profile", b"set GREETING=hello")?;
            host.vfs
                .write("/etc/startup.sh", b"echo $GREETING from startup.sh")?;
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    pub fn make() -> Box<dyn Plugin> {
        Box::new(WritesBootScripts)
    }
}

/// `/home/.profile` and `/etc/startup.sh` run once at boot, in that order:
/// the profile's variables are visible to the startup script and to the
/// interactive terminal, and the script's output is in the scrollback.
#[test]
fn boot_runs_profile_then_startup_script() {
    let mut opts = HarnessOptions::new("classic");
    opts.plugins = vec![boot_script_plugin::make];
    let mut h = Harness::with_options(opts).unwrap();
    let lines = &h.state().terminal.output_lines;
    assert!(
        lines.iter().any(|l| l == "hello from startup.sh"),
        "startup output missing: {lines:?}"
    );
    h.terminal("echo $GREETING");
    assert_eq!(
        h.state().terminal.output_lines.last().map(String::as_str),
        Some("hello")
    );
}

/// The default seeded profile is a no-op: it restates the shell defaults
/// and prints nothing.
#[test]
fn default_profile_keeps_shell_identity() {
    let mut h = Harness::new("classic");
    assert!(h.vfs().exists("/home/.profile"));
    h.terminal("echo $USER $SHELL");
    assert_eq!(
        h.state().terminal.output_lines.last().map(String::as_str),
        Some("user oasis")
    );
}

/// Snapshot of every SDI object's distortable properties, sorted by name.
fn scene_props(h: &Harness) -> Vec<(String, i32, i32, u8, Option<String>)> {
    let sdi = &h.shell.sdi;
    let mut names: Vec<&str> = sdi.names().collect();
    names.sort_unstable();
    names
        .into_iter()
        .map(|n| {
            let o = sdi.get(n).unwrap();
            (n.to_string(), o.x, o.y, o.alpha, o.text.clone())
        })
        .collect()
}

/// The corrupted skin distorts what is drawn (jitter / garble / flicker
/// vary frame to frame) but never the scene itself: every frame starts
/// from the clean layout, so nothing drifts.
#[test]
fn corrupted_skin_glitches_frames_without_drifting_the_scene() {
    let mut h = Harness::new("corrupted");
    assert!(h.state().skin.corrupted_modifiers.is_some());
    // The skin rests in its terminal: pin the prompt cursor so its blink
    // does not count as drift.
    h.state_mut().active_theme.terminal_cursor_blink_rate = 0;
    h.settle();
    let clean = scene_props(&h);
    let mut drawn = std::collections::HashSet::new();
    for _ in 0..20 {
        let out = h.step(&[]);
        assert!(out.redraw, "the glitch keeps the corrupted skin animating");
        let frame: Vec<_> = h
            .frame_text_calls()
            .iter()
            .map(|t| (t.text.clone(), t.x, t.y))
            .collect();
        drawn.insert(format!("{frame:?}"));
    }
    assert!(drawn.len() > 1, "drawn output never varied");
    let now = scene_props(&h);
    let changed: Vec<_> = now.iter().filter(|p| !clean.contains(p)).collect();
    assert!(
        changed.is_empty(),
        "corruption leaked into the scene: {changed:?}"
    );
    assert_eq!(now, clean, "corruption leaked into the scene");
}

/// Skins with `dashboard = false` (desktop, corrupted) show no icon grid:
/// no icon objects are drawn, before or after a window opens, and a click
/// where an icon would sit launches nothing. The terminal-only corrupted
/// skin rests in its terminal instead of an empty dashboard.
#[test]
fn dashboardless_skins_draw_and_launch_no_icons() {
    for (skin, idle) in [("desktop", Mode::Dashboard), ("corrupted", Mode::Terminal)] {
        let mut h = Harness::new(skin);
        assert!(!h.state().skin.features.dashboard, "{skin}");
        h.settle();
        assert_eq!(h.mode(), idle, "{skin}");
        let icon_objects = |h: &Harness| {
            (0..4)
                .flat_map(|i| [format!("icon_{i}"), format!("icon_label_{i}")])
                .filter(|n| h.sdi_rect(n).is_some())
                .collect::<Vec<_>>()
        };
        assert!(
            icon_objects(&h).is_empty(),
            "{skin}: icons drawn: {:?} {}",
            icon_objects(&h),
            dump(&h, &format!("no_icons_{skin}"))
        );
        let rect = h
            .app_icon_rect("Settings")
            .expect("layout still has a slot");
        let (x, y) = oasis_app::harness::rect_center(rect);
        h.click(x, y);
        h.settle();
        assert!(
            h.find_window("Settings").is_none(),
            "{skin}: hidden icon launched"
        );
        if h.state().skin.features.window_manager {
            assert!(h.open_app("Terminal"), "{skin}");
            h.settle();
            assert!(icon_objects(&h).is_empty(), "{skin}: icons behind windows");
        }
    }
}

/// New windows open below psix-hifi's top tab row (the status bar + tab
/// row the skin reserves), not over it.
#[test]
fn new_windows_open_below_the_tab_row() {
    let mut h = Harness::new("psix-hifi");
    h.settle();
    assert!(h.state().skin.features.show_tabs);
    let at = &h.state().active_theme;
    let tab_bottom = (at.statusbar_height + at.tab_row_height) as i32;
    for title in ["Settings", "Text Editor", "Calculator"] {
        assert!(h.open_app(title), "{title}");
        h.settle();
        let win = h.find_window(title).expect("window opened");
        assert!(
            win.frame.1 >= tab_bottom,
            "{title} frame {:?} covers the tab row (ends at {tab_bottom}) {}",
            win.frame,
            dump(&h, &format!("tab_row_{title}"))
        );
        let (_, sh) = h.size();
        assert!(
            win.frame.1 + win.frame.3 as i32 <= sh as i32,
            "{title} off screen"
        );
    }
}
