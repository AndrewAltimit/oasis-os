//! End-to-end window-manager sessions driven purely through pointer input
//! (`PointerClick` / `CursorMove` / `PointerRelease`), the way a user
//! interacts with windows, plus rendering checks with a clip-auditing
//! backend. `wm_integration.rs` covers API-level workflows; this file
//! targets what a user can reach by mouse, invariants under random input,
//! and edge cases (off-screen windows, borders, animations).

#![allow(clippy::unwrap_used)]

use oasis_sdi::SdiRegistry;
use oasis_test_backend::{ClipAuditBackend, Color};
use oasis_types::input::InputEvent;
use oasis_wm::hit_test::hit_test;
use oasis_wm::{
    HitRegion, ResizeEdge, SnapZone, TilingLayout, WindowConfig, WindowManager, WindowState,
    WindowType, WmEvent, WmTheme,
};

const SW: u32 = 800;
const SH: u32 = 600;

fn cfg(id: &str, x: i32, y: i32, w: u32, h: u32) -> WindowConfig {
    WindowConfig {
        id: id.to_string(),
        title: format!("Window {id}"),
        x: Some(x),
        y: Some(y),
        width: w,
        height: h,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    }
}

fn cascaded(id: &str) -> WindowConfig {
    WindowConfig {
        x: None,
        y: None,
        ..cfg(id, 0, 0, 240, 160)
    }
}

/// A user's mouse.
struct Mouse<'a> {
    wm: &'a mut WindowManager,
    sdi: &'a mut SdiRegistry,
}

impl Mouse<'_> {
    fn click(&mut self, x: i32, y: i32) -> WmEvent {
        let ev = self
            .wm
            .handle_input(&InputEvent::PointerClick { x, y }, self.sdi);
        self.wm
            .handle_input(&InputEvent::PointerRelease { x, y }, self.sdi);
        ev
    }

    fn drag(&mut self, from: (i32, i32), to: (i32, i32)) -> WmEvent {
        self.wm.handle_input(
            &InputEvent::PointerClick {
                x: from.0,
                y: from.1,
            },
            self.sdi,
        );
        // Move in a few steps like a real pointer.
        for i in 1..=4 {
            let x = from.0 + (to.0 - from.0) * i / 4;
            let y = from.1 + (to.1 - from.1) * i / 4;
            self.wm
                .handle_input(&InputEvent::CursorMove { x, y }, self.sdi);
        }
        self.wm
            .handle_input(&InputEvent::PointerRelease { x: to.0, y: to.1 }, self.sdi)
    }
}

fn titlebar_grab(wm: &WindowManager, id: &str) -> (i32, i32) {
    let w = wm.get_window(id).unwrap();
    // Middle of the titlebar: clear of the buttons on either side.
    let (tx, ty, tw, th) = w.titlebar_rect(wm.theme()).unwrap();
    (tx + tw as i32 / 2, ty + th as i32 / 2)
}

/// Every on-screen pixel in the window's titlebar row that a user could
/// press to start dragging it.
fn grabbable_titlebar_points(wm: &WindowManager, id: &str) -> Vec<(i32, i32)> {
    let w = wm.get_window(id).unwrap();
    let (tx, ty, tw, th) = w.titlebar_rect(wm.theme()).unwrap();
    let y = ty + th as i32 / 2;
    if !(0..SH as i32).contains(&y) {
        return Vec::new();
    }
    // Test against this window alone: another window covering it is fine
    // (the user can move that one), being off-screen is not.
    let alone = [w.clone()];
    (tx.max(0)..(tx + tw as i32).min(SW as i32))
        .filter(|&x| hit_test(&alone, x, y, wm.theme()) == HitRegion::Titlebar(id.into()))
        .map(|x| (x, y))
        .collect()
}

fn min_outer(theme: &WmTheme) -> (u32, u32) {
    (
        80 + theme.border_width * 2,
        60 + theme.titlebar_height + theme.border_width * 2,
    )
}

fn geom(wm: &WindowManager, id: &str) -> (i32, i32, u32, u32) {
    let w = wm.get_window(id).unwrap();
    (w.x, w.y, w.outer_w, w.outer_h)
}

fn assert_on_screen_and_reachable(wm: &WindowManager, id: &str) {
    let pts = grabbable_titlebar_points(wm, id);
    let w = wm.get_window(id).unwrap();
    assert!(
        !pts.is_empty(),
        "window {id} at ({}, {}) {}x{} has no grabbable titlebar pixel on screen",
        w.x,
        w.y,
        w.outer_w,
        w.outer_h
    );
}

// ── Open many / focus / z-order ─────────────────────────────────────

#[test]
fn opening_many_windows_keeps_every_one_reachable_and_clickable() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    for i in 0..40 {
        wm.create_window(&cascaded(&format!("w{i}")), &mut sdi)
            .unwrap();
    }
    assert_eq!(wm.window_count(), 40);
    assert_eq!(wm.active_window(), Some("w39"));
    // The newest window is on top everywhere it covers.
    let top = wm.get_window("w39").unwrap();
    let (cx, cy, _, _) = top.content_rect(wm.theme());
    assert_eq!(
        hit_test(wm.windows(), cx + 5, cy + 5, wm.theme()),
        HitRegion::Content("w39".into(), 5, 5)
    );
    for i in 0..40 {
        let w = wm.get_window(&format!("w{i}")).unwrap();
        assert!(
            w.x >= 0 && w.y >= 0,
            "w{i} placed off-screen at {},{}",
            w.x,
            w.y
        );
        assert!(
            w.x + (w.outer_w as i32) <= SW as i32 && w.y + (w.outer_h as i32) <= SH as i32,
            "cascade pushed w{i} past the screen edge: ({}, {}) {}x{}",
            w.x,
            w.y,
            w.outer_w,
            w.outer_h
        );
    }
}

#[test]
fn clicking_a_background_window_raises_and_focuses_it() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("a", 50, 50, 300, 200), &mut sdi)
        .unwrap();
    wm.create_window(&cfg("b", 150, 100, 300, 200), &mut sdi)
        .unwrap();
    wm.create_window(&cfg("c", 250, 150, 300, 200), &mut sdi)
        .unwrap();
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    // A visible sliver of "a" (its content, left of b).
    let ev = m.click(60, 150);
    assert!(matches!(ev, WmEvent::ContentClick(ref id, _, _) if id.as_str() == "a"));
    assert_eq!(wm.active_window(), Some("a"));
    assert_eq!(wm.windows().last().unwrap().id.as_str(), "a");
    // Point inside the overlap of all three now resolves to "a".
    assert!(matches!(
        hit_test(wm.windows(), 270, 200, wm.theme()),
        HitRegion::Content(ref id, _, _) if id.as_str() == "a"
    ));
    // Clicking the desktop clears focus without changing z-order.
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    assert_eq!(m.click(790, 590), WmEvent::DesktopClick(790, 590));
    assert_eq!(wm.active_window(), None);
    assert_eq!(wm.windows().last().unwrap().id.as_str(), "a");
}

// ── Drag ───────────────────────────────────────────────────────────

#[test]
fn drag_by_titlebar_moves_window_and_its_chrome() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(grab, (grab.0 + 123, grab.1 + 77));
    let w = wm.get_window("w").unwrap();
    assert_eq!((w.x, w.y), (223, 177));
    assert_eq!(sdi.get("w.frame").unwrap().x, 223);
    assert_eq!(sdi.get("w.titlebar").unwrap().y, 177 + 1);
    // Content clicks are reported relative to the moved content area.
    let (cx, cy, _, _) = w.content_rect(wm.theme());
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    assert_eq!(
        m.click(cx + 7, cy + 9),
        WmEvent::ContentClick("w".into(), 7, 9)
    );
}

#[test]
fn window_dragged_off_every_edge_stays_reachable_and_can_be_dragged_back() {
    let targets = [
        (-5000, 200),
        (5000, 200),
        (300, -5000),
        (300, 5000),
        (-5000, -5000),
        (5000, 5000),
    ];
    for side in ["right", "left"] {
        for target in targets {
            let mut sdi = SdiRegistry::new();
            let theme = WmTheme {
                button_side: side.to_string(),
                ..WmTheme::default()
            };
            let mut wm = WindowManager::with_theme(SW, SH, theme);
            wm.create_window(&cfg("w", 200, 200, 300, 200), &mut sdi)
                .unwrap();
            let grab = titlebar_grab(&wm, "w");
            let mut m = Mouse {
                wm: &mut wm,
                sdi: &mut sdi,
            };
            m.drag(grab, target);
            assert_on_screen_and_reachable(&wm, "w");

            // Grab it by any reachable titlebar pixel and bring it home.
            let pts = grabbable_titlebar_points(&wm, "w");
            let from = pts[pts.len() / 2];
            let mut m = Mouse {
                wm: &mut wm,
                sdi: &mut sdi,
            };
            m.drag(from, (400, 300));
            let w = wm.get_window("w").unwrap();
            assert!(
                w.x >= 0 && w.x + w.outer_w as i32 <= SW as i32,
                "{side} {target:?}: not fully back on screen: x={}",
                w.x
            );
        }
    }
}

#[test]
fn off_screen_window_close_button_stays_clickable() {
    // Buttons on the right, window pushed off the left edge: the visible
    // strip must still let the user close it.
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 200, 200, 300, 200), &mut sdi)
        .unwrap();
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(grab, (-5000, 300));
    let (bx, by, bw, bh) = wm
        .get_window("w")
        .unwrap()
        .close_btn_rect(wm.theme())
        .unwrap();
    let (px, py) = (bx + bw as i32 / 2, by + bh as i32 / 2);
    assert!(px >= 0, "close button centre is off-screen at x={px}");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    assert_eq!(m.click(px, py), WmEvent::WindowClosed("w".into()));
    assert_eq!(wm.window_count(), 0);
}

// ── Resize ─────────────────────────────────────────────────────────

fn edge_point(wm: &WindowManager, id: &str, edge: ResizeEdge) -> (i32, i32) {
    let w = wm.get_window(id).unwrap();
    let (l, t) = (w.x + 1, w.y + 1);
    let (r, b) = (w.x + w.outer_w as i32 - 2, w.y + w.outer_h as i32 - 2);
    let (mx, my) = (w.x + w.outer_w as i32 / 2, w.y + w.outer_h as i32 / 2);
    match edge {
        ResizeEdge::North => (mx, t),
        ResizeEdge::South => (mx, b),
        ResizeEdge::East => (r, my),
        ResizeEdge::West => (l, my),
        ResizeEdge::NorthEast => (r, t),
        ResizeEdge::NorthWest => (l, t),
        ResizeEdge::SouthEast => (r, b),
        ResizeEdge::SouthWest => (l, b),
    }
}

const EDGES: [ResizeEdge; 8] = [
    ResizeEdge::North,
    ResizeEdge::South,
    ResizeEdge::East,
    ResizeEdge::West,
    ResizeEdge::NorthEast,
    ResizeEdge::NorthWest,
    ResizeEdge::SouthEast,
    ResizeEdge::SouthWest,
];

#[test]
fn every_edge_and_corner_is_hit_tested_as_its_resize_handle() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    for edge in EDGES {
        let (x, y) = edge_point(&wm, "w", edge);
        let hit = hit_test(wm.windows(), x, y, wm.theme());
        // The top edge/corners overlap the titlebar, whose buttons and
        // body take priority; everything else must be the resize handle.
        match edge {
            ResizeEdge::North | ResizeEdge::NorthEast | ResizeEdge::NorthWest => {
                assert!(
                    matches!(
                        hit,
                        HitRegion::ResizeHandle(_, e) if e == edge
                    ) || matches!(hit, HitRegion::Titlebar(_) | HitRegion::TitlebarButton(..)),
                    "{edge:?}: {hit:?}"
                );
            },
            _ => assert_eq!(hit, HitRegion::ResizeHandle("w".into(), edge), "{edge:?}"),
        }
    }
    // Exactly one pixel outside the frame is the desktop.
    let w = wm.get_window("w").unwrap();
    for (x, y) in [
        (w.x - 1, w.y + 50),
        (w.x + w.outer_w as i32, w.y + 50),
        (w.x + 50, w.y - 1),
        (w.x + 50, w.y + w.outer_h as i32),
    ] {
        assert_eq!(hit_test(wm.windows(), x, y, wm.theme()), HitRegion::Desktop);
    }
    // No border pixel is ever reported as content.
    let (cx, cy, cw, ch) = w.content_rect(wm.theme());
    for x in w.x..w.x + w.outer_w as i32 {
        for y in [w.y, w.y + w.outer_h as i32 - 1] {
            if let HitRegion::Content(_, lx, ly) = hit_test(wm.windows(), x, y, wm.theme()) {
                assert!(lx >= 0 && ly >= 0 && lx < cw as i32 && ly < ch as i32);
                assert!(x >= cx && y >= cy);
            }
        }
    }
}

#[test]
fn south_east_resize_grows_and_shrinks_to_minimum() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    let p = edge_point(&wm, "w", ResizeEdge::SouthEast);
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(p, (p.0 + 100, p.1 + 50));
    let w = wm.get_window("w").unwrap();
    assert_eq!((w.x, w.y), (100, 100));
    let grown = (w.outer_w, w.outer_h);
    let p = edge_point(&wm, "w", ResizeEdge::SouthEast);
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(p, (-400, -400));
    let w = wm.get_window("w").unwrap();
    assert_eq!((w.x, w.y), (100, 100), "SE resize must not move the window");
    assert_eq!((w.outer_w, w.outer_h), min_outer(wm.theme()));
    assert!(grown.0 > 300 && grown.1 > 200);
    // Frame SDI follows the resize.
    assert_eq!(sdi.get("w.frame").unwrap().w, w.outer_w);
}

#[test]
fn shrinking_from_any_edge_respects_min_size_and_pins_opposite_edge() {
    for edge in EDGES {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(SW, SH);
        wm.create_window(&cfg("w", 200, 150, 300, 250), &mut sdi)
            .unwrap();
        let before = wm.get_window("w").unwrap().clone();
        let p = edge_point(&wm, "w", edge);
        // Resize handles only; skip top points claimed by the titlebar.
        if !matches!(
            hit_test(wm.windows(), p.0, p.1, wm.theme()),
            HitRegion::ResizeHandle(..)
        ) {
            continue;
        }
        let centre = (
            before.x + before.outer_w as i32 / 2,
            before.y + before.outer_h as i32 / 2,
        );
        // Drag well past the opposite edge.
        let to = (p.0 + (centre.0 - p.0) * 4, p.1 + (centre.1 - p.1) * 4);
        let mut m = Mouse {
            wm: &mut wm,
            sdi: &mut sdi,
        };
        m.drag(p, to);
        let w = wm.get_window("w").unwrap();
        let (min_w, min_h) = min_outer(wm.theme());
        assert!(w.outer_w >= min_w && w.outer_h >= min_h, "{edge:?}: {w:?}");
        let right = |w: &oasis_wm::Window| w.x + w.outer_w as i32;
        let bottom = |w: &oasis_wm::Window| w.y + w.outer_h as i32;
        match edge {
            ResizeEdge::West | ResizeEdge::NorthWest | ResizeEdge::SouthWest => {
                assert_eq!(right(w), right(&before), "{edge:?} moved the right edge")
            },
            ResizeEdge::East | ResizeEdge::NorthEast | ResizeEdge::SouthEast => {
                assert_eq!(w.x, before.x, "{edge:?} moved the left edge")
            },
            _ => {},
        }
        match edge {
            ResizeEdge::North | ResizeEdge::NorthEast | ResizeEdge::NorthWest => {
                assert_eq!(bottom(w), bottom(&before), "{edge:?} moved the bottom edge")
            },
            ResizeEdge::South | ResizeEdge::SouthEast | ResizeEdge::SouthWest => {
                assert_eq!(w.y, before.y, "{edge:?} moved the top edge")
            },
            _ => {},
        }
    }
}

#[test]
fn resizing_a_partly_off_screen_window_keeps_its_left_edge_and_min_size() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.set_snap_enabled(false);
    wm.create_window(&cfg("w", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    // Push most of the window off the left edge.
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(grab, (grab.0 - 250, grab.1));
    let before = wm.get_window("w").unwrap().clone();
    assert!(before.x < 0, "precondition: window partly off-screen");
    // Nudge the east edge 2px wider.
    let p = edge_point(&wm, "w", ResizeEdge::East);
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(p, (p.0 + 2, p.1));
    let w = wm.get_window("w").unwrap();
    assert_eq!(w.x, before.x, "east resize moved the window's left edge");
    assert_eq!(w.outer_w, before.outer_w + 2);
    let (min_w, _) = min_outer(wm.theme());
    assert!(w.outer_w >= min_w);
}

// ── Maximize / minimize / restore ──────────────────────────────────

#[test]
fn maximize_button_and_titlebar_double_click_round_trip() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 120, 90, 300, 200), &mut sdi)
        .unwrap();
    let orig = geom(&wm, "w");
    let (bx, by, bw, bh) = wm
        .get_window("w")
        .unwrap()
        .maximize_btn_rect(wm.theme())
        .unwrap();
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    let ev = m.click(bx + bw as i32 / 2, by + bh as i32 / 2);
    assert_eq!(ev, WmEvent::WindowMaximized("w".into()));
    let w = wm.get_window("w").unwrap();
    assert_eq!((w.x, w.y, w.outer_w, w.outer_h), (0, 0, SW, SH));
    // Double-click the titlebar body restores.
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.click(grab.0, grab.1);
    assert_eq!(m.click(grab.0, grab.1), WmEvent::WindowRestored("w".into()));
    assert_eq!(geom(&wm, "w"), orig);
}

#[test]
fn maximizing_twice_still_restores_the_original_geometry() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 250, 180), &mut sdi)
        .unwrap();
    let orig = geom(&wm, "w");
    wm.maximize_window("w", &mut sdi).unwrap();
    wm.maximize_window("w", &mut sdi).unwrap();
    wm.restore_window("w", &mut sdi).unwrap();
    assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);
    assert_eq!(geom(&wm, "w"), orig);
}

#[test]
fn minimize_then_restore_a_maximized_window_returns_pre_maximize_geometry() {
    // Documented behaviour (see `maximize_then_minimize_then_restore`):
    // restoring from the taskbar goes back to the floating geometry.
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 250, 180), &mut sdi)
        .unwrap();
    let orig = geom(&wm, "w");
    wm.maximize_window("w", &mut sdi).unwrap();
    wm.minimize_window("w", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), None);
    wm.restore_window("w", &mut sdi).unwrap();
    assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);
    assert_eq!(geom(&wm, "w"), orig);
    assert!(sdi.get("w.frame").unwrap().visible);
}

#[test]
fn minimize_button_hides_window_and_clicks_fall_through() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("under", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    wm.create_window(&cfg("top", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    let top_orig = geom(&wm, "top");
    let (bx, by, bw, bh) = wm
        .get_window("top")
        .unwrap()
        .minimize_btn_rect(wm.theme())
        .unwrap();
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    assert_eq!(
        m.click(bx + bw as i32 / 2, by + bh as i32 / 2),
        WmEvent::WindowMinimized("top".into())
    );
    assert_eq!(wm.active_window(), Some("under"));
    assert!(!sdi.get("top.frame").unwrap().visible);
    // (Past the 6px resize-handle band that overlaps the content edge.)
    let (cx, cy, _, _) = wm.get_window("under").unwrap().content_rect(wm.theme());
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    assert_eq!(
        m.click(cx + 30, cy + 30),
        WmEvent::ContentClick("under".into(), 30, 30)
    );
    wm.restore_window("top", &mut sdi).unwrap();
    assert!(sdi.get("top.frame").unwrap().visible);
    assert_eq!(geom(&wm, "top"), top_orig);
}

// ── Snapping / tiling ──────────────────────────────────────────────

#[test]
fn dragging_to_the_left_edge_snaps_and_dragging_out_restores_size() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 300, 200, 300, 200), &mut sdi)
        .unwrap();
    let orig = geom(&wm, "w");
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(grab, (0, 300));
    let w = wm.get_window("w").unwrap();
    assert_eq!(w.snap_zone, Some(SnapZone::Left));
    assert_eq!((w.x, w.y, w.outer_w, w.outer_h), (0, 0, SW / 2, SH));
    assert!(wm.snap_preview().is_none(), "preview must clear on release");
    // Dragging the snapped window away restores its floating size.
    let grab = titlebar_grab(&wm, "w");
    let mut m = Mouse {
        wm: &mut wm,
        sdi: &mut sdi,
    };
    m.drag(grab, (grab.0 + 200, grab.1 + 150));
    let w = wm.get_window("w").unwrap();
    assert_eq!(w.snap_zone, None);
    assert_eq!((w.outer_w, w.outer_h), (orig.2, orig.3));
}

#[test]
fn every_tiling_layout_fits_the_work_area_and_restores_floating() {
    let mut sdi = SdiRegistry::new();
    let theme = WmTheme {
        maximize_top_inset: 20,
        maximize_bottom_inset: 30,
        ..WmTheme::default()
    };
    let mut wm = WindowManager::with_theme(SW, SH, theme);
    let originals: Vec<_> = (0..5)
        .map(|i| {
            (
                format!("t{i}"),
                30 * i,
                40 + 20 * i,
                200 + 10 * i as u32,
                150,
            )
        })
        .collect();
    for (id, x, y, w, h) in &originals {
        wm.create_window(&cfg(id, *x, *y, *w, h.to_owned()), &mut sdi)
            .unwrap();
    }
    let originals: Vec<_> = originals
        .iter()
        .map(|(id, ..)| (id.clone(), geom(&wm, id)))
        .collect();
    let area = wm.work_area();
    let mut seen = Vec::new();
    while let Some(layout) = wm.cycle_tiling(&mut sdi) {
        seen.push(layout);
        let wins: Vec<_> = wm.windows().to_vec();
        for w in &wins {
            assert!(
                w.x >= area.x
                    && w.y >= area.y
                    && w.x + w.outer_w as i32 <= area.x + area.w as i32
                    && w.y + w.outer_h as i32 <= area.y + area.h as i32,
                "{layout:?}: {} outside work area: ({}, {}) {}x{}",
                w.id,
                w.x,
                w.y,
                w.outer_w,
                w.outer_h
            );
            assert!(w.outer_w > 0 && w.outer_h > 0);
        }
        if layout != TilingLayout::Monocle {
            for (i, a) in wins.iter().enumerate() {
                for b in &wins[i + 1..] {
                    let overlap = a.x < b.x + b.outer_w as i32
                        && b.x < a.x + a.outer_w as i32
                        && a.y < b.y + b.outer_h as i32
                        && b.y < a.y + a.outer_h as i32;
                    assert!(!overlap, "{layout:?}: {} overlaps {}", a.id, b.id);
                }
            }
        }
        assert!(seen.len() < 20, "tiling cycle never returns to floating");
    }
    assert!(seen.len() >= 3);
    for (id, g) in &originals {
        assert_eq!(geom(&wm, id), *g, "{id} not restored after tiling");
    }
}

#[test]
fn tiling_more_windows_than_fit_keeps_every_window_on_screen() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    for i in 0..24 {
        wm.create_window(&cascaded(&format!("m{i}")), &mut sdi)
            .unwrap();
    }
    while let Some(layout) = wm.cycle_tiling(&mut sdi) {
        for w in wm.windows() {
            assert!(
                w.x >= 0
                    && w.y >= 0
                    && w.x + w.outer_w as i32 <= 480
                    && w.y + w.outer_h as i32 <= 272,
                "{layout:?}: {} tiled off-screen at ({}, {}) {}x{}",
                w.id,
                w.x,
                w.y,
                w.outer_w,
                w.outer_h
            );
        }
    }
}

// ── Animations ─────────────────────────────────────────────────────

#[test]
fn closing_windows_mid_animation_cleans_up_and_draws_safely() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.set_motion_enabled(true);
    wm.create_window(&cfg("a", 50, 50, 300, 200), &mut sdi)
        .unwrap();
    wm.create_window(&cfg("b", 80, 80, 300, 200), &mut sdi)
        .unwrap();
    assert!(wm.is_animating());
    wm.tick_animations_by(30, &mut sdi);
    // Close during the open animation, minimize the other mid-flight and
    // close it during its minimize animation.
    wm.close_window("b", &mut sdi).unwrap();
    wm.minimize_window("a", &mut sdi).unwrap();
    wm.tick_animations_by(20, &mut sdi);
    wm.close_window("a", &mut sdi).unwrap();
    // Re-create a window with a closing id while its ghost still fades.
    wm.create_window(&cfg("b", 10, 10, 200, 150), &mut sdi)
        .unwrap();

    let mut backend = ClipAuditBackend::new(SW, SH, (0, 0, SW, SH));
    let mut drawn = Vec::new();
    for _ in 0..60 {
        backend.clear_records();
        wm.draw_with_clips(&mut sdi, &mut backend, |id, _, _, _, _, _| {
            drawn.push(id.to_string());
            Ok(())
        })
        .unwrap();
        wm.tick_animations_by(16, &mut sdi);
    }
    assert!(!wm.is_animating(), "animations never settled");
    assert!(
        !drawn.iter().any(|id| id == "a"),
        "closed window drew content"
    );
    assert!(
        sdi.get("a.frame").is_err(),
        "closed window leaked SDI objects"
    );
    assert!(sdi.get("b.frame").is_ok());
    assert_eq!(wm.window_count(), 1);
    // The re-created window ends at its final geometry.
    let b = wm.get_window("b").unwrap();
    assert_eq!(sdi.get("b.frame").unwrap().x, b.x);
    assert_eq!(sdi.get("b.frame").unwrap().w, b.outer_w);
}

// ── Rendering: content clip ────────────────────────────────────────

#[test]
fn app_content_stays_clipped_after_app_pushes_and_pops_its_own_clip() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SW, SH);
    wm.create_window(&cfg("w", 100, 100, 300, 200), &mut sdi)
        .unwrap();
    let content = wm.get_window("w").unwrap().content_rect(wm.theme());
    let marker = Color::rgb(1, 2, 3);
    let mut backend = ClipAuditBackend::new(SW, SH, content);
    backend.set_auditing(false);
    wm.draw_with_clips(&mut sdi, &mut backend, |_, cx, cy, cw, _, b| {
        // Typical app: a clipped scroll region, then more drawing that
        // overflows the window to the right (long text line, etc.).
        b.push_clip_rect(cx, cy, cw, 20)?;
        b.fill_rect(cx, cy, cw, 20, Color::WHITE)?;
        b.pop_clip_rect()?;
        b.fill_rect(cx, cy + 30, 2000, 10, marker)
    })
    .unwrap();
    let r = backend
        .rects()
        .iter()
        .find(|r| r.color == Some(marker))
        .unwrap();
    let (vx, vy, vw, vh) = r.visible.unwrap();
    assert!(
        vx >= content.0
            && vy >= content.1
            && vx + vw as i32 <= content.0 + content.2 as i32
            && vy + vh as i32 <= content.1 + content.3 as i32,
        "app draw leaked outside its window: visible {:?}, content {content:?}",
        r.visible
    );
}

// ── Property: random pointer sessions ──────────────────────────────

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n.max(1)
    }
}

#[test]
fn random_pointer_sessions_preserve_wm_invariants() {
    for seed in 1..=6u64 {
        let mut rng = Lcg(seed);
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(SW, SH);
        wm.set_motion_enabled(seed % 2 == 0);
        let mut next_id = 0;
        let mut pressed = false;
        for step in 0..3000 {
            let x = rng.next(SW as u64 + 200) as i32 - 100;
            let y = rng.next(SH as u64 + 200) as i32 - 100;
            let ev = match rng.next(20) {
                0 if wm.window_count() < 12 => {
                    let id = format!("r{next_id}");
                    next_id += 1;
                    let c = WindowConfig {
                        x: None,
                        y: None,
                        ..cfg(
                            &id,
                            0,
                            0,
                            100 + rng.next(400) as u32,
                            100 + rng.next(300) as u32,
                        )
                    };
                    wm.create_window(&c, &mut sdi).unwrap();
                    continue;
                },
                1 => {
                    wm.cycle_tiling(&mut sdi);
                    continue;
                },
                2..=6 => {
                    pressed = true;
                    InputEvent::PointerClick { x, y }
                },
                7..=15 => InputEvent::CursorMove { x, y },
                _ => {
                    pressed = false;
                    InputEvent::PointerRelease { x, y }
                },
            };
            wm.handle_input(&ev, &mut sdi);
            wm.tick_animations_by(16, &mut sdi);
            if pressed || step % 5 != 0 {
                continue;
            }
            let (min_w, min_h) = min_outer(wm.theme());
            for w in wm.windows().to_vec() {
                if w.state == WindowState::Minimized {
                    continue;
                }
                // Tiling layouts may legitimately tile below the manual
                // resize minimum; everything else must respect it.
                assert!(
                    wm.tiling_layout().is_some() || (w.outer_w >= min_w && w.outer_h >= min_h),
                    "seed {seed} step {step} after {ev:?}: {} shrank to {}x{} at ({}, {}) {:?}",
                    w.id,
                    w.outer_w,
                    w.outer_h,
                    w.x,
                    w.y,
                    w.snap_zone
                );
                assert_on_screen_and_reachable(&wm, w.id.as_str());
            }
            if let Some(active) = wm.active_window() {
                assert!(wm.get_window(active).is_some());
            }
        }
        // Everything still draws.
        let mut backend = ClipAuditBackend::new(SW, SH, (0, 0, SW, SH));
        wm.draw_with_clips(&mut sdi, &mut backend, |_, _, _, _, _, _| Ok(()))
            .unwrap();
    }
}
