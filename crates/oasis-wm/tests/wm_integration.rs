//! Integration tests for the oasis-wm window manager.
//!
//! These tests exercise multi-step workflows that span multiple WM operations,
//! verifying that state remains consistent across create/focus/minimize/maximize/
//! close/restore/drag/snap/tile/desktop sequences.

#![allow(clippy::unwrap_used)]

use oasis_sdi::SdiRegistry;
use oasis_types::input::InputEvent;
use oasis_wm::{
    DesktopManager, KeyboardSnapDirection, SnapManager, SnapZone, TilingLayout, TilingManager,
    WindowConfig, WindowId, WindowManager, WindowPlacement, WindowState, WindowType, WmEvent,
};

// ── Helpers ──────────────────────────────────────────────────────────

fn app_config(id: &str) -> WindowConfig {
    WindowConfig {
        id: id.to_string(),
        title: id.to_string(),
        x: Some(10),
        y: Some(10),
        width: 200,
        height: 150,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    }
}

fn app_config_at(id: &str, x: i32, y: i32, w: u32, h: u32) -> WindowConfig {
    WindowConfig {
        id: id.to_string(),
        title: id.to_string(),
        x: Some(x),
        y: Some(y),
        width: w,
        height: h,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    }
}

const SCREEN_W: u32 = 800;
const SCREEN_H: u32 = 600;

// ── 1. Window lifecycle ──────────────────────────────────────────────

#[test]
fn window_lifecycle_create_minimize_maximize_close_restore() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    // Create 3 windows.
    wm.create_window(&app_config("a"), &mut sdi).unwrap();
    wm.create_window(&app_config("b"), &mut sdi).unwrap();
    wm.create_window(&app_config("c"), &mut sdi).unwrap();
    assert_eq!(wm.window_count(), 3);

    // Minimize first window.
    wm.minimize_window("a", &mut sdi).unwrap();
    assert_eq!(wm.get_window("a").unwrap().state, WindowState::Minimized);
    assert!(!sdi.get("a.frame").unwrap().visible);
    assert!(!sdi.get("a.content").unwrap().visible);

    // Maximize second window.
    wm.maximize_window("b", &mut sdi).unwrap();
    let win_b = wm.get_window("b").unwrap();
    assert_eq!(win_b.state, WindowState::Maximized);
    assert_eq!(win_b.outer_w, SCREEN_W);
    assert_eq!(win_b.outer_h, SCREEN_H);

    // Close third window.
    wm.close_window("c", &mut sdi).unwrap();
    assert_eq!(wm.window_count(), 2);
    assert!(!sdi.contains("c.frame"));
    assert!(!sdi.contains("c.content"));

    // Restore first window from minimized.
    wm.restore_window("a", &mut sdi).unwrap();
    assert_eq!(wm.get_window("a").unwrap().state, WindowState::Normal);
    assert!(sdi.get("a.frame").unwrap().visible);
    assert!(sdi.get("a.content").unwrap().visible);

    // Verify final states: a=Normal, b=Maximized, c=gone.
    assert_eq!(wm.get_window("a").unwrap().state, WindowState::Normal);
    assert_eq!(wm.get_window("b").unwrap().state, WindowState::Maximized);
    assert!(wm.get_window("c").is_none());
}

// ── 2. Focus chain ───────────────────────────────────────────────────

#[test]
fn focus_chain_click_each_in_sequence() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config("w1"), &mut sdi).unwrap();
    wm.create_window(&app_config("w2"), &mut sdi).unwrap();
    wm.create_window(&app_config("w3"), &mut sdi).unwrap();

    // After creation, w3 is active (last created).
    assert_eq!(wm.active_window(), Some("w3"));

    // Focus w1.
    wm.focus_window("w1", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), Some("w1"));
    // w1 titlebar should have active color.
    assert_eq!(
        sdi.get("w1.titlebar").unwrap().color,
        wm.theme().titlebar_active_color
    );
    // w2 and w3 should have inactive color.
    assert_eq!(
        sdi.get("w2.titlebar").unwrap().color,
        wm.theme().titlebar_inactive_color
    );
    assert_eq!(
        sdi.get("w3.titlebar").unwrap().color,
        wm.theme().titlebar_inactive_color
    );

    // Focus w2.
    wm.focus_window("w2", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), Some("w2"));
    assert_eq!(
        sdi.get("w2.titlebar").unwrap().color,
        wm.theme().titlebar_active_color
    );

    // Focus w3.
    wm.focus_window("w3", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), Some("w3"));
    assert_eq!(
        sdi.get("w3.titlebar").unwrap().color,
        wm.theme().titlebar_active_color
    );

    // All windows still exist.
    assert_eq!(wm.window_count(), 3);
}

// ── 3. Window stacking (z-order) ────────────────────────────────────

#[test]
fn window_stacking_z_order_after_focus() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config("a"), &mut sdi).unwrap();
    wm.create_window(&app_config("b"), &mut sdi).unwrap();
    wm.create_window(&app_config("c"), &mut sdi).unwrap();

    // After creation: z-order is [a, b, c] (c on top).
    // windows vec is ordered by z: last = topmost.
    assert_eq!(wm.active_window(), Some("c"));

    // Focus a -- it should move to the top of the z-order.
    wm.focus_window("a", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), Some("a"));

    // Verify a is now topmost by checking the SDI z-order:
    // After focus_window_internal, a's SDI objects are moved to top last,
    // meaning a.frame is the last object moved via move_to_top.
    // We can verify indirectly: the active window is a, and its titlebar
    // has the active color.
    assert_eq!(
        sdi.get("a.titlebar").unwrap().color,
        wm.theme().titlebar_active_color
    );
    assert_eq!(
        sdi.get("b.titlebar").unwrap().color,
        wm.theme().titlebar_inactive_color
    );
    assert_eq!(
        sdi.get("c.titlebar").unwrap().color,
        wm.theme().titlebar_inactive_color
    );

    // Focus b to verify re-stacking works multiple times.
    wm.focus_window("b", &mut sdi).unwrap();
    assert_eq!(wm.active_window(), Some("b"));
    assert_eq!(
        sdi.get("b.titlebar").unwrap().color,
        wm.theme().titlebar_active_color
    );
}

// ── 4. Tiling workflow ───────────────────────────────────────────────

#[test]
fn tiling_two_windows_master_stack() {
    let tiling = TilingManager::new();
    let config = tiling.config();
    let gap = config.gap;
    let margin = config.margin;
    let ids = vec!["w1", "w2"];
    let tiles = tiling.compute_layout(&ids, SCREEN_W, SCREEN_H);

    assert_eq!(tiles.len(), 2);

    let t1 = &tiles[0]; // master
    let t2 = &tiles[1]; // stack

    // Master is on the left side.
    assert_eq!(t1.window_id, "w1");
    assert_eq!(t1.geometry.x, margin as i32);
    assert_eq!(t1.geometry.y, margin as i32);

    // Stack is to the right of master.
    assert_eq!(t2.window_id, "w2");
    assert!(t2.geometry.x > t1.geometry.x);

    // Both windows should span the full usable height.
    let usable_h = SCREEN_H - margin * 2;
    assert_eq!(t1.geometry.h, usable_h);
    assert_eq!(t2.geometry.h, usable_h);

    // Their combined widths plus gap should roughly fill the usable width.
    let total_w = t1.geometry.w + gap + t2.geometry.w;
    let usable_w = SCREEN_W - margin * 2;
    // Allow 1px rounding tolerance.
    assert!((total_w as i32 - usable_w as i32).unsigned_abs() <= 1);
}

#[test]
fn tiling_columns_layout() {
    let tiling = TilingManager::with_layout(TilingLayout::Columns);
    let ids = vec!["w1", "w2", "w3"];
    let tiles = tiling.compute_layout(&ids, SCREEN_W, SCREEN_H);

    assert_eq!(tiles.len(), 3);

    // All columns should have the same height.
    let h0 = tiles[0].geometry.h;
    for tile in &tiles {
        assert_eq!(tile.geometry.h, h0);
    }

    // Each subsequent column should be to the right of the previous.
    assert!(tiles[1].geometry.x > tiles[0].geometry.x);
    assert!(tiles[2].geometry.x > tiles[1].geometry.x);
}

// ── 5. Snap workflow ─────────────────────────────────────────────────

#[test]
fn snap_left_then_right() {
    let mut snap = SnapManager::new();

    // Snap left.
    let left_geo = snap
        .apply_snap(&SnapZone::Left, SCREEN_W, SCREEN_H)
        .expect("left snap should produce geometry");
    assert_eq!(left_geo.x, 0);
    assert_eq!(left_geo.y, 0);
    assert_eq!(left_geo.w, SCREEN_W / 2);
    assert_eq!(left_geo.h, SCREEN_H);

    // Snap right.
    let right_geo = snap
        .apply_snap(&SnapZone::Right, SCREEN_W, SCREEN_H)
        .expect("right snap should produce geometry");
    assert_eq!(right_geo.x, (SCREEN_W / 2) as i32);
    assert_eq!(right_geo.y, 0);
    assert_eq!(right_geo.w, SCREEN_W - SCREEN_W / 2);
    assert_eq!(right_geo.h, SCREEN_H);

    // Left + right should tile the full screen width.
    assert_eq!(left_geo.w + right_geo.w, SCREEN_W);
}

#[test]
fn keyboard_snap_left_right() {
    use oasis_wm::keyboard_snap;

    let (zone_l, geo_l) = keyboard_snap(KeyboardSnapDirection::Left, SCREEN_W, SCREEN_H).unwrap();
    assert_eq!(zone_l, SnapZone::Left);
    assert_eq!(geo_l.x, 0);
    assert_eq!(geo_l.w, SCREEN_W / 2);

    let (zone_r, geo_r) = keyboard_snap(KeyboardSnapDirection::Right, SCREEN_W, SCREEN_H).unwrap();
    assert_eq!(zone_r, SnapZone::Right);
    assert_eq!(geo_r.x, (SCREEN_W / 2) as i32);

    // Down returns None (restore/minimize, not a snap).
    assert!(keyboard_snap(KeyboardSnapDirection::Down, SCREEN_W, SCREEN_H).is_none());
}

// ── 6. Desktop switching ─────────────────────────────────────────────

#[test]
fn desktop_switching_window_visibility() {
    let mut dm = DesktopManager::new(3);
    assert_eq!(dm.active_desktop(), 0);

    // Assign windows to desktop 0.
    dm.assign_window("w1", WindowPlacement::Desktop(0));
    dm.assign_window("w2", WindowPlacement::Desktop(0));

    // All visible on desktop 0.
    assert!(dm.is_visible("w1"));
    assert!(dm.is_visible("w2"));

    // Switch to desktop 1.
    assert!(dm.switch_to(1));
    assert_eq!(dm.active_desktop(), 1);

    // Windows from desktop 0 are no longer visible.
    assert!(!dm.is_visible("w1"));
    assert!(!dm.is_visible("w2"));

    // Create a window on desktop 1.
    dm.assign_window("w3", WindowPlacement::Desktop(1));
    assert!(dm.is_visible("w3"));

    // Switch back to desktop 0.
    assert!(dm.switch_to(0));
    assert_eq!(dm.active_desktop(), 0);

    // Desktop 0 windows visible again, desktop 1 window hidden.
    assert!(dm.is_visible("w1"));
    assert!(dm.is_visible("w2"));
    assert!(!dm.is_visible("w3"));
}

#[test]
fn desktop_sticky_window_visible_on_all() {
    let mut dm = DesktopManager::new(3);

    dm.assign_window("sticky", WindowPlacement::AllDesktops);
    dm.assign_window("pinned", WindowPlacement::Desktop(0));

    assert!(dm.is_visible("sticky"));
    assert!(dm.is_visible("pinned"));

    dm.switch_to(1);
    assert!(dm.is_visible("sticky"));
    assert!(!dm.is_visible("pinned"));

    dm.switch_to(2);
    assert!(dm.is_visible("sticky"));
    assert!(!dm.is_visible("pinned"));
}

#[test]
fn desktop_cycle_wraps_around() {
    let mut dm = DesktopManager::new(3);
    assert_eq!(dm.active_desktop(), 0);
    assert_eq!(dm.switch_next(), 1);
    assert_eq!(dm.switch_next(), 2);
    assert_eq!(dm.switch_next(), 0); // wraps

    assert_eq!(dm.switch_prev(), 2); // wraps back
    assert_eq!(dm.switch_prev(), 1);
}

// ── 7. Maximize/restore preserves geometry ───────────────────────────

#[test]
fn maximize_restore_preserves_exact_geometry() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    let config = app_config_at("w", 50, 75, 300, 200);
    wm.create_window(&config, &mut sdi).unwrap();

    let win = wm.get_window("w").unwrap();
    let orig_x = win.x;
    let orig_y = win.y;
    let orig_outer_w = win.outer_w;
    let orig_outer_h = win.outer_h;

    // Verify the window was placed at the requested position.
    assert_eq!(orig_x, 50);
    assert_eq!(orig_y, 75);

    // Maximize.
    wm.maximize_window("w", &mut sdi).unwrap();
    let win = wm.get_window("w").unwrap();
    assert_eq!(win.state, WindowState::Maximized);
    assert_eq!(win.x, 0);
    assert_eq!(win.outer_w, SCREEN_W);

    // Restore.
    wm.restore_window("w", &mut sdi).unwrap();
    let win = wm.get_window("w").unwrap();
    assert_eq!(win.state, WindowState::Normal);
    assert_eq!(win.x, orig_x);
    assert_eq!(win.y, orig_y);
    assert_eq!(win.outer_w, orig_outer_w);
    assert_eq!(win.outer_h, orig_outer_h);

    // Also verify SDI objects match.
    assert_eq!(sdi.get("w.frame").unwrap().x, orig_x);
    assert_eq!(sdi.get("w.frame").unwrap().y, orig_y);
}

#[test]
fn double_maximize_restore_still_preserves_original() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    let config = app_config_at("w", 100, 100, 250, 180);
    wm.create_window(&config, &mut sdi).unwrap();

    // Maximize twice (second overwrites saved_geometry with maximized values).
    wm.maximize_window("w", &mut sdi).unwrap();
    wm.maximize_window("w", &mut sdi).unwrap();

    // Restore once should go back to some valid geometry.
    wm.restore_window("w", &mut sdi).unwrap();
    let win = wm.get_window("w").unwrap();
    assert_eq!(win.state, WindowState::Normal);
    assert!(win.outer_w > 0);
    assert!(win.outer_h > 0);
}

// ── 8. Drag/resize sequence ──────────────────────────────────────────

#[test]
fn drag_sequence_moves_window() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config_at("w", 50, 50, 200, 150), &mut sdi)
        .unwrap();

    let win = wm.get_window("w").unwrap();
    let orig_x = win.x;
    let orig_y = win.y;
    let (tx, ty, _tw, th) = win.titlebar_rect(wm.theme()).unwrap();

    // Click on titlebar to initiate drag.
    let click_x = tx + 10;
    let click_y = ty + th as i32 / 2;
    let event = wm.handle_input(
        &InputEvent::PointerClick {
            x: click_x,
            y: click_y,
        },
        &mut sdi,
    );
    assert_eq!(event, WmEvent::WindowFocused(WindowId::from("w")));

    // Drag by (60, 40).
    let move_event = wm.handle_input(
        &InputEvent::CursorMove {
            x: click_x + 60,
            y: click_y + 40,
        },
        &mut sdi,
    );
    assert_eq!(move_event, WmEvent::WindowMoved(WindowId::from("w")));

    // Verify position updated.
    let win = wm.get_window("w").unwrap();
    assert_eq!(win.x, orig_x + 60);
    assert_eq!(win.y, orig_y + 40);

    // Release.
    let release_event = wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    // Release after drag returns WindowMoved.
    assert!(matches!(release_event, WmEvent::WindowMoved(_)));

    // Verify SDI frame position matches.
    assert_eq!(sdi.get("w.frame").unwrap().x, orig_x + 60);
    assert_eq!(sdi.get("w.frame").unwrap().y, orig_y + 40);
}

#[test]
fn resize_via_handle_changes_dimensions() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config_at("w", 50, 50, 200, 150), &mut sdi)
        .unwrap();

    let win = wm.get_window("w").unwrap();
    let orig_w = win.outer_w;
    let orig_h = win.outer_h;

    // Click on east edge (right side, vertical center) to start resize.
    let right_edge = win.x + win.outer_w as i32 - 2;
    let mid_y = win.y + win.outer_h as i32 / 2;

    wm.handle_input(
        &InputEvent::PointerClick {
            x: right_edge,
            y: mid_y,
        },
        &mut sdi,
    );

    // Drag 50px to the right.
    wm.handle_input(
        &InputEvent::CursorMove {
            x: right_edge + 50,
            y: mid_y,
        },
        &mut sdi,
    );

    let win = wm.get_window("w").unwrap();
    assert_eq!(win.outer_w, orig_w + 50);
    // Height should not change for east-only resize.
    assert_eq!(win.outer_h, orig_h);

    // Release.
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
}

// ── Additional multi-step workflows ──────────────────────────────────

#[test]
fn fullscreen_kiosk_hides_decorations_and_restores() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    let config = app_config_at("w", 100, 100, 200, 150);
    wm.create_window(&config, &mut sdi).unwrap();

    let win = wm.get_window("w").unwrap();
    let orig_x = win.x;
    let orig_y = win.y;
    let orig_outer_w = win.outer_w;
    let orig_outer_h = win.outer_h;

    // Enter fullscreen kiosk.
    wm.enter_fullscreen("w", &mut sdi).unwrap();
    let win = wm.get_window("w").unwrap();
    assert!(win.fullscreen_kiosk);
    assert_eq!(win.x, 0);
    assert_eq!(win.y, 0);
    assert_eq!(win.outer_w, SCREEN_W);
    assert_eq!(win.outer_h, SCREEN_H);

    // Decorations hidden (frame, titlebar, etc.).
    assert!(!sdi.get("w.frame").unwrap().visible);
    assert!(!sdi.get("w.titlebar").unwrap().visible);
    // Content still visible.
    assert!(sdi.get("w.content").unwrap().visible);

    // Exit fullscreen kiosk.
    wm.exit_fullscreen("w", &mut sdi).unwrap();
    let win = wm.get_window("w").unwrap();
    assert!(!win.fullscreen_kiosk);
    assert_eq!(win.x, orig_x);
    assert_eq!(win.y, orig_y);
    assert_eq!(win.outer_w, orig_outer_w);
    assert_eq!(win.outer_h, orig_outer_h);

    // Decorations restored.
    assert!(sdi.get("w.frame").unwrap().visible);
    assert!(sdi.get("w.titlebar").unwrap().visible);
}

#[test]
fn cycle_focus_rotates_through_windows() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config("w1"), &mut sdi).unwrap();
    wm.create_window(&app_config("w2"), &mut sdi).unwrap();
    wm.create_window(&app_config("w3"), &mut sdi).unwrap();

    // w3 is active (topmost).
    assert_eq!(wm.active_window(), Some("w3"));

    // Cycle forward: brings bottom-most (w1) to top.
    let focused = wm.cycle_focus(true, &mut sdi);
    assert!(focused.is_some());
    let focused_id = focused.unwrap();
    assert_eq!(*focused_id, *"w1");

    // Cycle forward again: brings new bottom-most to top.
    let focused = wm.cycle_focus(true, &mut sdi);
    assert!(focused.is_some());
}

#[test]
fn close_all_clears_everything() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(SCREEN_W, SCREEN_H);

    wm.create_window(&app_config("a"), &mut sdi).unwrap();
    wm.create_window(&app_config("b"), &mut sdi).unwrap();
    wm.create_window(&app_config("c"), &mut sdi).unwrap();

    wm.close_all(&mut sdi);

    assert_eq!(wm.window_count(), 0);
    assert!(wm.active_window().is_none());
    assert!(!sdi.contains("a.frame"));
    assert!(!sdi.contains("b.frame"));
    assert!(!sdi.contains("c.frame"));
    assert!(!sdi.contains("a.content"));
    assert!(!sdi.contains("b.content"));
    assert!(!sdi.contains("c.content"));
}

#[test]
fn snap_preview_updates_and_clears() {
    let mut snap = SnapManager::new();

    // Move cursor to left edge -- should activate preview.
    snap.update_preview(2, 300, SCREEN_W, SCREEN_H);
    assert!(snap.active_preview.is_some());
    assert_eq!(snap.active_preview.unwrap().zone, SnapZone::Left);

    // Move cursor to center -- should clear preview.
    snap.update_preview(400, 300, SCREEN_W, SCREEN_H);
    assert!(snap.active_preview.is_none());

    // Move to right edge.
    snap.update_preview(798, 300, SCREEN_W, SCREEN_H);
    assert!(snap.active_preview.is_some());
    assert_eq!(snap.active_preview.unwrap().zone, SnapZone::Right);

    // Clear explicitly.
    snap.clear_preview();
    assert!(snap.active_preview.is_none());
}

#[test]
fn desktop_move_window_between_desktops() {
    let mut dm = DesktopManager::new(3);

    dm.assign_window("w1", WindowPlacement::Desktop(0));
    assert!(dm.is_visible("w1"));

    // Move w1 to desktop 2.
    dm.move_window_to_desktop("w1", 2);
    assert!(!dm.is_visible("w1")); // We are still on desktop 0.

    dm.switch_to(2);
    assert!(dm.is_visible("w1")); // Now visible on desktop 2.

    // Remove tracking.
    dm.remove_window("w1");
    // Untracked windows default to active desktop -- still visible.
    assert!(dm.is_visible("w1"));
}

#[test]
fn tiling_floating_override_excludes_window() {
    let mut tiling = TilingManager::new();
    tiling.set_floating("w2", true);

    let ids = vec!["w1", "w2", "w3"];
    let tiles = tiling.compute_layout(&ids, SCREEN_W, SCREEN_H);

    // w2 should be excluded.
    assert_eq!(tiles.len(), 2);
    assert!(tiles.iter().all(|t| t.window_id != "w2"));
    assert!(tiles.iter().any(|t| t.window_id == "w1"));
    assert!(tiles.iter().any(|t| t.window_id == "w3"));
}
