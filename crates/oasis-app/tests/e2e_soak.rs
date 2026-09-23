#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! Long-running stability of the real desktop shell: resource leaks
//! across open/close, skin and resolution cycles, per-frame state growth,
//! seeded random-input fuzzing, and idle-frame elision with windows open.
//!
//! The default runs stay within a few tens of seconds in debug builds.
//! Heavier variants are `#[ignore]`d:
//!
//! ```bash
//! cargo test -p oasis-app --release --test e2e_soak -- --ignored
//! ```

use std::time::{Duration, Instant};

use oasis_app::Mode;
use oasis_app::harness::{Harness, HarnessOptions, ResourceCounts};
use oasis_core::bottombar::MediaTab;
use oasis_core::input::{Button, InputEvent, Key, Modifiers, Trigger};
use oasis_core::skin::builtin::builtin_names;
use oasis_core::vfs::Vfs;

fn dump(h: &Harness, name: &str) -> String {
    let dir = std::env::temp_dir().join("oasis-e2e");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.png"));
    match h.save_png(&path) {
        Ok(()) => format!("(frame saved to {})", path.display()),
        Err(e) => format!("(frame dump failed: {e})"),
    }
}

/// Boot `skin` at a small resolution (soak runs render thousands of
/// frames; the software rasterizer's cost scales with the pixel count).
fn boot_small(skin: &str) -> Harness {
    let mut opts = HarnessOptions::new(skin);
    opts.resolution = Some((640, 480));
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    h
}

/// Open and close every dashboard app once, alternating the close path
/// (titlebar button / Escape). Returns to the dashboard.
fn cycle_every_app(h: &mut Harness, round: usize) {
    let apps: Vec<String> = h
        .state()
        .ui
        .dashboard
        .apps
        .iter()
        .map(|a| a.title.clone())
        .collect();
    for (i, app) in apps.iter().enumerate() {
        assert!(h.open_app(app), "{app}");
        h.run_frames(20);
        h.render_now();
        if (i + round).is_multiple_of(2) && h.find_window(app).is_some() {
            assert!(h.close_window(app), "{app}: close button");
        } else {
            for _ in 0..4 {
                if h.mode() != Mode::Desktop && h.mode() != Mode::Terminal {
                    break;
                }
                h.key(Key::Escape);
                h.run_frames(3);
            }
        }
        h.settle();
        assert!(
            h.find_window(app).is_none(),
            "round {round}: {app} did not close: {:?}",
            h.windows()
        );
        assert!(
            matches!(h.mode(), Mode::Dashboard),
            "round {round}: {app}: mode {:?}",
            h.mode()
        );
    }
}

/// Backend resources, windows, runners and audio tracks must match the
/// baseline exactly. The SDI object count may shrink (e.g. the fullscreen
/// terminal's line objects follow its scrollback) but never grow.
fn assert_counts(h: &Harness, base: ResourceCounts, what: &str) {
    let now = h.resource_counts();
    assert_eq!(
        ResourceCounts {
            sdi_objects: 0,
            ..now
        },
        ResourceCounts {
            sdi_objects: 0,
            ..base
        },
        "{what}: resources drifted from the baseline"
    );
    assert!(
        now.sdi_objects <= base.sdi_objects,
        "{what}: SDI objects grew {} -> {}",
        base.sdi_objects,
        now.sdi_objects
    );
}

/// Warm up with two rounds and return the baseline (the larger SDI
/// count of the two).
fn warm_up(h: &mut Harness) -> ResourceCounts {
    cycle_every_app(h, 0);
    let a = h.resource_counts();
    cycle_every_app(h, 1);
    let b = h.resource_counts();
    ResourceCounts {
        sdi_objects: a.sdi_objects.max(b.sdi_objects),
        ..b
    }
}

// ---------------------------------------------------------------------------
// Leaks
// ---------------------------------------------------------------------------

#[test]
fn open_close_every_app_returns_to_baseline() {
    for skin in ["classic", "psix-tribute"] {
        let mut h = boot_small(skin);
        // Warm-up round: first launches create lazily-built SDI objects
        // (hidden, not destroyed, on close) and caches.
        let base = warm_up(&mut h);
        assert_eq!(base.windows, 0, "{skin}");
        assert_eq!(base.runners, 0, "{skin}");
        assert_eq!(base.audio_tracks, 0, "{skin}");
        for round in 2..=3 {
            cycle_every_app(&mut h, round);
            assert_counts(&h, base, &format!("{skin} round {round}"));
        }
    }
}

/// Photo Viewer uploads image textures; closing the window (every close
/// path) must release them.
#[test]
fn photo_viewer_textures_are_released_on_close() {
    let mut h = boot_small("classic");
    let base = h.resource_counts();
    for close_with_button in [true, false, true] {
        assert!(h.open_app("Photo Viewer"));
        h.settle();
        // Open the first entry (image or folder) a few times to load
        // thumbnails / the full image.
        // The listing of /home/user/photos: select the sample PNG, open it.
        let lines = h.app_runner("Photo Viewer").unwrap().lines.clone();
        let idx = lines
            .iter()
            .position(|l| l.contains(".png"))
            .unwrap_or_else(|| panic!("no image in the listing: {lines:?}"));
        for _ in 0..idx {
            h.key(Key::Down);
        }
        let mut peak = 0;
        h.key(Key::Enter);
        for _ in 0..3 {
            h.run_frames(5);
            h.render_now();
            peak = peak.max(h.resource_counts().textures);
        }
        assert!(
            peak > base.textures,
            "setup: the viewer never uploaded an image {}",
            dump(&h, "photo_viewer_setup")
        );
        if close_with_button {
            assert!(h.close_window("Photo Viewer"));
        } else {
            for _ in 0..5 {
                if h.find_window("Photo Viewer").is_none() {
                    break;
                }
                h.key(Key::Escape);
                h.run_frames(2);
            }
        }
        h.settle();
        h.render_now();
        assert!(h.find_window("Photo Viewer").is_none());
        let now = h.resource_counts();
        assert_eq!(
            now.textures, base.textures,
            "Photo Viewer textures leaked (button close: {close_with_button})"
        );
    }
}

#[test]
fn skin_switch_cycles_do_not_leak() {
    let mut h = boot_small("classic");
    // A window stays open through every skin swap.
    assert!(h.open_app("Calculator"));
    h.settle();
    let skins: Vec<&str> = builtin_names().to_vec();
    let mut at_classic = Vec::new();
    for cycle in 0..2 {
        for skin in skins.iter().chain(std::iter::once(&"classic")) {
            h.vfs_mut()
                .write(
                    oasis_app_settings::SKIN_CHANGE_REQUEST_PATH,
                    skin.as_bytes(),
                )
                .unwrap();
            h.settle();
            assert_eq!(h.state().skin.manifest.name, *skin, "cycle {cycle}");
            h.render_now();
            assert!(
                h.find_window("Calculator").is_some(),
                "{skin}: window survives the swap"
            );
        }
        at_classic.push(h.resource_counts());
        let _ = cycle;
    }
    assert_eq!(
        at_classic[0], at_classic[1],
        "a full skin cycle leaked resources"
    );
    // The window still works after all that.
    assert!(h.close_window("Calculator"));
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
}

#[test]
fn resolution_cycles_do_not_leak() {
    let mut h = boot_small("xp");
    assert!(h.open_app("Paint"));
    h.settle();
    let sizes = [(800, 600), (1024, 768), (640, 480), (1280, 720), (640, 480)];
    let mut at_640 = Vec::new();
    for _ in 0..3 {
        for (w, hh) in sizes {
            h.vfs_mut()
                .write(
                    oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH,
                    format!("{w}x{hh}").as_bytes(),
                )
                .unwrap();
            h.settle();
            assert_eq!(h.size(), (w, hh));
            h.render_now();
        }
        at_640.push(h.resource_counts());
    }
    assert_eq!(at_640[0], at_640[1], "resolution cycle leaked");
    assert_eq!(at_640[1], at_640[2], "resolution cycle leaked");
}

/// Long idle runs with windows open: no per-frame growth of scene objects,
/// textures, terminal scrollback or toasts.
#[test]
fn no_per_frame_state_growth() {
    let mut h = boot_small("classic");
    for app in ["Calculator", "Text Editor", "Terminal", "Settings"] {
        assert!(h.open_app(app), "{app}");
        h.settle();
    }
    h.advance(Duration::from_secs(2));
    let base = h.resource_counts();
    let lines = h.state().terminal.output_lines.len();
    for i in 0..10 {
        // Mix in pointer motion so hover paths run too.
        h.move_to(50 + i * 30, 60 + i * 20);
        h.advance(Duration::from_secs(1));
    }
    assert_eq!(h.resource_counts(), base, "idle desktop grew");
    assert_eq!(h.state().terminal.output_lines.len(), lines);
}

// ---------------------------------------------------------------------------
// Random-input fuzz
// ---------------------------------------------------------------------------

/// Tiny deterministic PRNG (xorshift64*), so the fuzz needs no extra
/// dependency and every failure reproduces from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn coord(&mut self, max: u32) -> i32 {
        self.below(u64::from(max)) as i32
    }
}

const FUZZ_KEYS: &[Key] = &[
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Backspace,
    Key::Space,
    Key::Home,
    Key::End,
    Key::PageDown,
    Key::F(1),
    Key::F(2),
    Key::F(11),
    Key::Char('q'),
    Key::Char('e'),
    Key::Char('a'),
    Key::Char('s'),
    Key::Char('1'),
];

const FUZZ_BUTTONS: &[Button] = &[
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::Confirm,
    Button::Cancel,
    Button::Triangle,
    Button::Square,
    Button::Start,
    Button::Select,
];

/// One random user gesture.
fn fuzz_gesture(h: &mut Harness, rng: &mut Rng) -> String {
    let (w, hh) = h.size();
    match rng.below(100) {
        0..=24 => {
            let (x, y) = (rng.coord(w), rng.coord(hh));
            h.click(x, y);
            format!("click {x},{y}")
        },
        25..=34 => {
            let from = (rng.coord(w), rng.coord(hh));
            let to = (rng.coord(w), rng.coord(hh));
            h.drag(from, to, 1 + rng.below(4) as u32);
            format!("drag {from:?} -> {to:?}")
        },
        35..=39 => {
            let (x, y) = (rng.coord(w), rng.coord(hh));
            let delta = if rng.below(2) == 0 { -1 } else { 1 };
            h.move_to(x, y);
            h.scroll(delta);
            format!("wheel {delta} at {x},{y}")
        },
        40..=69 => {
            let key = FUZZ_KEYS[rng.below(FUZZ_KEYS.len() as u64) as usize];
            let mods = match rng.below(12) {
                0 => Modifiers::ALT,
                1 => Modifiers::SUPER,
                2 => Modifiers::CTRL | Modifiers::ALT,
                3 => Modifiers::CTRL,
                4 => Modifiers::SHIFT,
                _ => Modifiers::NONE,
            };
            h.key_with(key, mods);
            format!("key {key:?} {mods:?}")
        },
        70..=79 => {
            let n = 1 + rng.below(4);
            let text: String = (0..n)
                .map(|_| (b'a' + rng.below(26) as u8) as char)
                .collect();
            h.type_text(&text);
            format!("type {text:?}")
        },
        80..=89 => {
            let b = FUZZ_BUTTONS[rng.below(FUZZ_BUTTONS.len() as u64) as usize];
            h.button(b);
            format!("button {b:?}")
        },
        90..=93 => {
            let t = if rng.below(2) == 0 {
                Trigger::Left
            } else {
                Trigger::Right
            };
            h.trigger(t);
            format!("trigger {t:?}")
        },
        94..=95 => {
            h.send(&[InputEvent::ToggleFullscreen]);
            "toggle fullscreen".to_string()
        },
        _ => {
            let n = 1 + rng.below(30) as u32;
            h.run_frames(n);
            format!("idle {n}")
        },
    }
}

/// Escape (plus "Discard" for an editor's unsaved-changes prompt) must
/// always lead back to the dashboard.
fn recover_to_dashboard(h: &mut Harness, what: &str) {
    let mut stuck = 0;
    for _ in 0..80 {
        if h.mode() == Mode::Dashboard && !h.state().ui.start_menu.open {
            return;
        }
        let before = (h.mode(), h.windows().len());
        h.key(Key::Escape);
        h.run_frames(2);
        check_focus_invariants(h, &format!("{what}: recovery after Escape from {before:?}"));
        if (h.mode(), h.windows().len()) == before {
            stuck += 1;
        } else {
            stuck = 0;
        }
        if stuck >= 3 {
            // A Text Editor with unsaved edits: Escape raises the prompt,
            // 'd' discards.
            h.key(Key::Char('d'));
            h.run_frames(2);
            check_focus_invariants(h, &format!("{what}: recovery after 'd'"));
            stuck = 0;
        }
    }
    panic!(
        "{what}: Escape could not get back to the dashboard: mode {:?}, windows {:?}, \
         start menu open {} {}",
        h.mode(),
        h.windows(),
        h.state().ui.start_menu.open,
        dump(h, what)
    );
}

/// Focus / mode consistency after every gesture:
/// - the focused window (keyboard target) is never a minimized, invisible
///   one (a click on the bare desktop may leave nothing focused);
/// - the dashboard (no window manager input) never shows window chrome:
///   any window left open there is minimized.
fn check_focus_invariants(h: &Harness, what: &str) {
    let wins = h.windows();
    let visible: Vec<&str> = wins
        .iter()
        .filter(|w| !w.minimized)
        .map(|w| w.id.as_str())
        .collect();
    // No window chrome outlives its window (closing animations aside).
    if !h.state().wm.is_animating() {
        const CHROME: &[&str] = &[".frame", ".titlebar", ".title_text", ".btn_close"];
        let orphans: Vec<&str> = h
            .shell
            .sdi
            .names()
            .filter(|n| CHROME.iter().any(|c| n.ends_with(c)))
            .filter(|n| {
                let id = &n[..n.rfind('.').unwrap_or(0)];
                !wins.iter().any(|w| w.id == id)
            })
            .collect();
        assert!(
            orphans.is_empty(),
            "{what}: window objects without a window: {orphans:?}"
        );
    }
    match h.mode() {
        Mode::Desktop => {
            if let Some(active) = h.active_window() {
                assert!(
                    visible.contains(&active.as_str()),
                    "{what}: focus {active:?} is not a visible window {wins:?}"
                );
            }
        },
        Mode::Dashboard => {
            assert!(
                visible.is_empty(),
                "{what}: dashboard mode with visible windows {wins:?} {}",
                dump(h, "invariant_dashboard")
            );
        },
        _ => {},
    }
}

fn fuzz(skin: &str, seed: u64, gestures: u32) {
    let mut h = boot_small(skin);
    let mut rng = Rng(seed);
    let what = format!("fuzz_{skin}_{seed:#x}");
    eprintln!("{what}: {gestures} gestures");
    let mut recent = std::collections::VecDeque::new();
    for i in 0..gestures {
        let g = fuzz_gesture(&mut h, &mut rng);
        recent.push_back(format!(
            "{i}: {g} -> {:?} focus {:?}",
            h.mode(),
            h.active_window()
        ));
        if recent.len() > 8 {
            recent.pop_front();
        }
        check_focus_invariants(
            &h,
            &format!("{what} gesture {i}, last gestures {recent:#?}"),
        );
        if i % 100 == 99 {
            h.render_now();
            let c = h.resource_counts();
            if i % 500 == 499 {
                eprintln!(
                    "{what} @{i}: {c:?} pcm chunks {} scrollback {} frames {}",
                    h.audio().pcm_chunks.len(),
                    h.state().terminal.output_lines.len(),
                    h.frames()
                );
            }
            assert!(c.windows <= 20, "{what}: {} windows", c.windows);
            assert!(
                c.sdi_objects < 5000,
                "{what}: {} SDI objects",
                c.sdi_objects
            );
            assert!(
                h.state().terminal.output_lines.len() <= 5000,
                "{what}: unbounded scrollback"
            );
        }
    }
    h.settle();
    h.render_now();
    assert!(
        h.distinct_colors() > 1,
        "{what}: blank frame {}",
        dump(&h, &what)
    );
    recover_to_dashboard(&mut h, &what);
    h.settle();
    check_focus_invariants(&h, &format!("{what}: after recovery"));
    h.render_now();
    assert!(h.distinct_colors() > 1, "{what}: blank dashboard");
    // The recovered shell is fully usable: an app still launches. (The
    // fuzz can legitimately delete /apps entries through the File Manager
    // or the terminal; the dashboard then has fewer, or no, icons.)
    let installed = h.vfs().readdir("/apps").map(|e| e.len()).unwrap_or(0);
    match h.dashboard_apps().first().cloned() {
        Some(app) => {
            assert!(h.open_app(&app), "{what}");
            h.settle();
            assert_eq!(h.mode(), Mode::Desktop, "{what}: launch after fuzz");
        },
        None => assert!(
            installed == 0 || h.state().ui.bottom_bar.active_tab != MediaTab::None,
            "{what}: {installed} apps installed but no dashboard icons {}",
            dump(&h, &what)
        ),
    }
}

// One test per skin family so they run in parallel.

#[test]
fn random_input_fuzz_windowed_skin() {
    fuzz("classic", 0x0A5_1500, 500);
}

#[test]
fn random_input_fuzz_free_layout_dashboard_skin() {
    fuzz("psix-tribute", 0xBEEF, 500);
}

#[test]
fn random_input_fuzz_psp_style_skin() {
    fuzz("vaporwave", 0x5EED_0003, 500);
}

/// Known issue: with the browser window focused after fullscreen /
/// terminal toggling, some frames transiently allocate several GB (seen
/// with `xp` seed 0x1234577c around gesture 2345; frames take seconds).
/// Watch process memory when running this.
#[test]
#[ignore = "long fuzz run (~30 min release); use --release --ignored"]
fn random_input_fuzz_long() {
    for (i, skin) in builtin_names().iter().enumerate() {
        for seed in 0..3u64 {
            fuzz(skin, 0x1234_5678 ^ (seed << 8) ^ i as u64, 3000);
        }
    }
}

#[test]
#[ignore = "long leak soak; use --release --ignored"]
fn open_close_soak_long() {
    for skin in builtin_names() {
        let mut h = boot_small(skin);
        let base = warm_up(&mut h);
        for round in 2..=10 {
            cycle_every_app(&mut h, round);
            assert_counts(&h, base, &format!("{skin} round {round}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Idle frames and frame cost
// ---------------------------------------------------------------------------

/// Frames presented during 4 s of idle time (after toasts and the input
/// grace period expire).
fn idle_presents(h: &mut Harness) -> u64 {
    h.advance(Duration::from_secs(5));
    let before = h.rendered_frames();
    h.run_frames(240);
    h.rendered_frames() - before
}

/// Static open windows stop redrawing: an idle desktop with windows open
/// presents no more than the same skin's idle dashboard (the 1 s
/// heartbeat, plus the skin's own ambient animation if it has one, e.g.
/// psix-tribute's drifting decal layers), with a little slack.
#[test]
fn idle_open_windows_stop_presenting() {
    for (skin, apps) in [
        ("classic", &["Calculator", "Text Editor", "Terminal"][..]),
        ("xp", &["File Manager", "Settings", "Paint"][..]),
        ("psix-tribute", &["Photo Viewer", "Music Player"][..]),
    ] {
        let mut h = boot_small(skin);
        let dashboard = idle_presents(&mut h);
        for app in apps {
            assert!(h.open_app(app), "{skin}: {app}");
            h.settle();
        }
        let drawn = idle_presents(&mut h);
        assert!(
            drawn <= dashboard + 8,
            "{skin}: idle {apps:?} presented {drawn} of 240 frames \
             (idle dashboard: {dashboard})"
        );
        if skin != "psix-tribute" {
            assert!(drawn <= 12, "{skin}: {drawn} of 240 frames");
        }
    }
}

/// A running game keeps presenting at its own step rate (not every frame,
/// not zero).
#[test]
fn running_game_window_presents_at_its_step_rate() {
    let mut h = boot_small("classic");
    assert!(h.open_app("Games"));
    h.settle();
    h.key(Key::Enter); // start the first game
    h.advance(Duration::from_secs(1));
    let before = h.rendered_frames();
    h.run_frames(120);
    let drawn = h.rendered_frames() - before;
    assert!(
        (4..=120).contains(&drawn),
        "running game presented {drawn} of 120 frames"
    );
}

/// Rough frame cost of a busy desktop. Generous bound: flags only
/// pathological regressions (e.g. per-frame texture uploads).
#[test]
fn busy_desktop_frame_cost_is_bounded() {
    let mut h = boot_small("xp");
    for app in [
        "Calculator",
        "Text Editor",
        "Terminal",
        "File Manager",
        "Paint",
    ] {
        assert!(h.open_app(app));
        h.settle();
    }
    // Time several short batches and keep the fastest: on a loaded CI
    // runner (parallel nextest pool) the mean absorbs every scheduling
    // stall and has read 15x the local cost, while the best batch still
    // tracks the real per-frame work.
    let per_frame = (0..6)
        .map(|batch| {
            let start = Instant::now();
            for i in 0..10 {
                let p = 100 + batch * 10 + i;
                h.move_to(p, p);
                h.render_now();
            }
            start.elapsed() / 10
        })
        .min()
        .expect("at least one batch");
    eprintln!("busy desktop: {per_frame:?} per step+render (debug build)");
    assert!(
        per_frame < Duration::from_secs(1),
        "step+render took {per_frame:?} per frame"
    );
    // No per-frame texture churn while hovering.
    let before = h.resource_counts();
    for i in 0..30 {
        h.move_to(200 + i, 150);
    }
    assert_eq!(h.resource_counts().textures, before.textures);
}
