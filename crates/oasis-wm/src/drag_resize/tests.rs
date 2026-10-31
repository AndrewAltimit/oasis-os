use super::*;
use crate::window::{WindowConfig, WindowType, WmTheme};
use oasis_sdi::SdiRegistry;
use oasis_types::input::InputEvent;

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

fn modal_config(id: &str) -> WindowConfig {
    WindowConfig {
        id: id.to_string(),
        title: id.to_string(),
        x: Some(50),
        y: Some(50),
        width: 200,
        height: 100,
        window_type: WindowType::Dialog,
        always_on_top: false,
        modal: true,
    }
}

// ---- Drag tests ----

#[test]
fn drag_moves_window() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();
    let orig_x = win.x;
    let orig_y = win.y;

    // Click on titlebar.
    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 5,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );

    // Drag.
    wm.handle_input(
        &InputEvent::CursorMove {
            x: tx + 55,
            y: ty + th as i32 / 2 + 30,
        },
        &mut sdi,
    );

    let win = wm.get_window("w1").unwrap();
    assert_eq!(win.x, orig_x + 50);
    assert_eq!(win.y, orig_y + 30);

    // Release.
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    assert!(wm.drag.is_none());
}

#[test]
fn resize_east() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let orig_w = win.outer_w;
    let right_edge = win.x + win.outer_w as i32 - 2;
    let mid_y = win.y + win.outer_h as i32 / 2;

    // Click on east resize handle.
    wm.handle_input(
        &InputEvent::PointerClick {
            x: right_edge,
            y: mid_y,
        },
        &mut sdi,
    );

    // Drag east by 40px.
    wm.handle_input(
        &InputEvent::CursorMove {
            x: right_edge + 40,
            y: mid_y,
        },
        &mut sdi,
    );

    let win = wm.get_window("w1").unwrap();
    assert_eq!(win.outer_w, orig_w + 40);

    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
}

#[test]
fn close_all_clears_drag_state() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w"), &mut sdi).unwrap();
    // Start a drag.
    let win = wm.get_window("w").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();
    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 5,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );
    wm.close_all(&mut sdi);
    assert!(wm.drag.is_none());
}

// ---- Screen bounds enforcement (drag) ----

#[test]
fn drag_clamps_to_screen_top() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 30,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );
    wm.handle_input(
        &InputEvent::CursorMove {
            x: tx + 30,
            y: -500,
        },
        &mut sdi,
    );

    let win = wm.get_window("w1").unwrap();
    assert!(win.y >= 0, "window y={} should be >= 0", win.y);
}

#[test]
fn drag_clamps_to_screen_bottom() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 30,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );
    wm.handle_input(&InputEvent::CursorMove { x: tx + 30, y: 800 }, &mut sdi);

    let win = wm.get_window("w1").unwrap();
    let max_y = 272 - wm.theme().titlebar_height as i32;
    assert!(win.y <= max_y, "window y={} should be <= {}", win.y, max_y);
}

#[test]
fn drag_clamps_to_screen_left() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let outer_w = win.outer_w;
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 30,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );
    wm.handle_input(
        &InputEvent::CursorMove {
            x: -1000,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );

    let win = wm.get_window("w1").unwrap();
    let min_x = MIN_VISIBLE - outer_w as i32;
    assert!(win.x >= min_x, "window x={} should be >= {}", win.x, min_x);
}

#[test]
fn drag_clamps_to_screen_right() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 30,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );
    wm.handle_input(
        &InputEvent::CursorMove {
            x: 2000,
            y: ty + th as i32 / 2,
        },
        &mut sdi,
    );

    let win = wm.get_window("w1").unwrap();
    let max_x = 480 - MIN_VISIBLE;
    assert!(win.x <= max_x, "window x={} should be <= {}", win.x, max_x);
}

// ---- Resize clamping ----

#[test]
fn resize_clamps_east_to_screen() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let right_edge = win.x + win.outer_w as i32 - 2;
    let mid_y = win.y + win.outer_h as i32 / 2;

    wm.handle_input(
        &InputEvent::PointerClick {
            x: right_edge,
            y: mid_y,
        },
        &mut sdi,
    );
    wm.handle_input(&InputEvent::CursorMove { x: 1000, y: mid_y }, &mut sdi);

    let win = wm.get_window("w1").unwrap();
    assert!(
        win.x + win.outer_w as i32 <= 480,
        "right edge {} should be <= 480",
        win.x + win.outer_w as i32,
    );
}

#[test]
fn resize_clamps_south_to_screen() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let bottom_edge = win.y + win.outer_h as i32 - 2;
    let mid_x = win.x + win.outer_w as i32 / 2;

    wm.handle_input(
        &InputEvent::PointerClick {
            x: mid_x,
            y: bottom_edge,
        },
        &mut sdi,
    );
    wm.handle_input(&InputEvent::CursorMove { x: mid_x, y: 1000 }, &mut sdi);

    let win = wm.get_window("w1").unwrap();
    assert!(
        win.y + win.outer_h as i32 <= 272,
        "bottom edge {} should be <= 272",
        win.y + win.outer_h as i32,
    );
}

// ---- compute_resize tests ----

#[test]
fn resize_respects_minimum_size() {
    let theme = WmTheme::default();
    let start = Geometry {
        x: 0,
        y: 0,
        w: 200,
        h: 200,
    };
    // Try shrinking way past minimum.
    let (_, _, w, h) = compute_resize(start, ResizeEdge::SouthEast, -400, -400, &theme);
    let min_w = MIN_RESIZE_W + theme.border_width * 2;
    let min_h = MIN_RESIZE_H + theme.titlebar_height + theme.border_width * 2;
    assert_eq!(w, min_w);
    assert_eq!(h, min_h);
}

#[test]
fn grabbing_handle_of_undersized_window_does_not_jump() {
    // A window already below the minimum (e.g. sized via resize_window)
    // used to snap to the minimum the moment its handle was grabbed.
    let theme = WmTheme::default();
    let start = Geometry {
        x: 100,
        y: 100,
        w: 40,
        h: 50,
    };
    assert_eq!(
        compute_resize(start, ResizeEdge::SouthEast, 0, 0, &theme),
        (100, 100, 40, 50)
    );
    assert_eq!(
        compute_resize(start, ResizeEdge::NorthWest, 0, 0, &theme),
        (100, 100, 40, 50)
    );
    // Growing still works; shrinking stops at the current size.
    assert_eq!(compute_resize(start, ResizeEdge::East, 30, 0, &theme).2, 70);
    assert_eq!(
        compute_resize(start, ResizeEdge::West, 10, 0, &theme),
        (100, 100, 40, 50)
    );
}

#[test]
fn clamp_position_lets_narrow_window_reach_left_edge() {
    // outer_w < MIN_VISIBLE used to force x >= MIN_VISIBLE - outer_w (> 0).
    let (cx, _) = clamp_position(0, 50, 12, 480, 272, 24);
    assert_eq!(cx, 0);
    let (cx, _) = clamp_position(-50, 50, 12, 480, 272, 24);
    assert_eq!(cx, 0, "a narrow frame stays fully on screen");
    let (cx, _) = clamp_position(1000, 50, 12, 480, 272, 24);
    assert_eq!(cx, 480 - 12);
}

#[test]
fn clamp_position_keeps_titlebar_on_tiny_screen() {
    // Screen shorter than the titlebar: the titlebar top stays at y = 0
    // instead of being pushed above the screen.
    let (_, cy) = clamp_position(0, 5, 200, 100, 10, 24);
    assert_eq!(cy, 0);
    // Screen narrower than MIN_VISIBLE: no inverted clamp range.
    let (cx, _) = clamp_position(50, 0, 200, 10, 10, 24);
    assert!((-190..=0).contains(&cx), "{cx}");
}

#[test]
fn resize_all_edges() {
    let theme = WmTheme::default();
    let start = Geometry {
        x: 50,
        y: 50,
        w: 200,
        h: 200,
    };
    // East
    let (_, _, w, _) = compute_resize(start, ResizeEdge::East, 30, 0, &theme);
    assert_eq!(w, 230);
    // West: x moves left, w grows
    let (x, _, w, _) = compute_resize(start, ResizeEdge::West, -20, 0, &theme);
    assert_eq!(w, 220);
    assert_eq!(x, 30);
    // South
    let (_, _, _, h) = compute_resize(start, ResizeEdge::South, 0, 40, &theme);
    assert_eq!(h, 240);
    // North: y moves up, h grows
    let (_, y, _, h) = compute_resize(start, ResizeEdge::North, 0, -10, &theme);
    assert_eq!(h, 210);
    assert_eq!(y, 40);
    // NorthEast
    let (_, y, w, h) = compute_resize(start, ResizeEdge::NorthEast, 20, -10, &theme);
    assert_eq!(w, 220);
    assert_eq!(h, 210);
    assert_eq!(y, 40);
    // NorthWest
    let (x, y, w, h) = compute_resize(start, ResizeEdge::NorthWest, -15, -10, &theme);
    assert_eq!(w, 215);
    assert_eq!(h, 210);
    assert_eq!(x, 35);
    assert_eq!(y, 40);
    // SouthWest
    let (x, _, w, h) = compute_resize(start, ResizeEdge::SouthWest, -20, 30, &theme);
    assert_eq!(w, 220);
    assert_eq!(h, 230);
    assert_eq!(x, 30);
}

// ---- clamp_position tests ----

#[test]
fn clamp_position_keeps_titlebar_visible() {
    let tb_h = WmTheme::default().titlebar_height;
    // Far left: at least MIN_VISIBLE px visible.
    let (cx, _) = clamp_position(-500, 100, 200, 480, 272, tb_h);
    assert_eq!(cx, MIN_VISIBLE - 200);
    // Far right.
    let (cx, _) = clamp_position(500, 100, 200, 480, 272, tb_h);
    assert_eq!(cx, 480 - MIN_VISIBLE);
    // Above screen.
    let (_, cy) = clamp_position(100, -100, 200, 480, 272, tb_h);
    assert_eq!(cy, 0);
    // Below screen.
    let (_, cy) = clamp_position(100, 500, 200, 480, 272, tb_h);
    assert_eq!(cy, 272 - tb_h as i32);
}

#[test]
fn move_window_clamps_to_bounds() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    wm.move_window("w1", 2000, 2000, &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    assert!(win.x <= 480 - MIN_VISIBLE);
    let max_y = 272 - wm.theme().titlebar_height as i32;
    assert!(win.y <= max_y);
}

#[test]
fn move_window_clamps_negative() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(480, 272);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    wm.move_window("w1", -2000, -2000, &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    assert!(win.y >= 0);
    assert!(win.x >= MIN_VISIBLE - win.outer_w as i32);
}

// ---- snap_to_edges tests ----

#[test]
fn snap_to_edges_near_left() {
    let (sx, _) = snap_to_edges(5, 100, 200, 150, 480, 272);
    assert_eq!(sx, 0);
}

#[test]
fn snap_to_edges_near_top() {
    let (_, sy) = snap_to_edges(100, 5, 200, 150, 480, 272);
    assert_eq!(sy, 0);
}

#[test]
fn snap_to_edges_near_right() {
    // Right edge = 275 + 200 = 475. Screen = 480. Diff = 5 < 8.
    let (sx, _) = snap_to_edges(275, 100, 200, 150, 480, 272);
    assert_eq!(sx, 280); // 480 - 200
}

#[test]
fn snap_to_edges_near_bottom() {
    // Bottom edge = 119 + 150 = 269. Screen = 272. Diff = 3 < 8.
    let (_, sy) = snap_to_edges(100, 119, 200, 150, 480, 272);
    assert_eq!(sy, 122); // 272 - 150
}

#[test]
fn snap_to_edges_no_snap_when_far() {
    let (sx, sy) = snap_to_edges(100, 50, 200, 150, 480, 272);
    assert_eq!(sx, 100);
    assert_eq!(sy, 50);
}

#[test]
fn snap_exact_edge() {
    let (sx, sy) = snap_to_edges(0, 0, 200, 150, 480, 272);
    assert_eq!(sx, 0);
    assert_eq!(sy, 0);
}

#[test]
fn snap_within_threshold() {
    // 7px from left edge.
    let (sx, _) = snap_to_edges(7, 50, 200, 150, 480, 272);
    assert_eq!(sx, 0);
}

#[test]
fn no_snap_just_outside_threshold() {
    // 9px from left edge -- just outside 8px threshold.
    let (sx, _) = snap_to_edges(9, 50, 200, 150, 480, 272);
    assert_eq!(sx, 9);
}

// ---- Modal click blocking ----

#[test]
fn modal_blocks_click_on_normal_window() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("bg"), &mut sdi).unwrap();
    wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

    // Click on the background window's content area.
    let bg = wm.get_window("bg").unwrap();
    let (cx, cy, _cw, _ch) = bg.content_rect(wm.theme());
    let event = InputEvent::PointerClick {
        x: cx + 10,
        y: cy + 10,
    };
    let result = wm.handle_input(&event, &mut sdi);
    assert_eq!(
        result,
        WmEvent::None,
        "click on non-modal should be blocked"
    );
}

#[test]
fn modal_allows_click_on_itself() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("bg"), &mut sdi).unwrap();
    wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

    // Click on the modal window's content area.
    let dlg = wm.get_window("dlg").unwrap();
    let (cx, cy, _cw, _ch) = dlg.content_rect(wm.theme());
    let event = InputEvent::PointerClick {
        x: cx + 10,
        y: cy + 10,
    };
    let result = wm.handle_input(&event, &mut sdi);
    assert!(
        matches!(result, WmEvent::ContentClick(ref id, _, _) if id == "dlg"),
        "click on modal should go through, got {result:?}"
    );
}

#[test]
fn modal_blocks_desktop_click() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

    let event = InputEvent::PointerClick { x: 700, y: 500 };
    let result = wm.handle_input(&event, &mut sdi);
    assert_eq!(result, WmEvent::None, "desktop click should be blocked");
}

// ---- Input dispatch ----

#[test]
fn click_content_returns_local_coords() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let (cx, cy, _cw, _ch) = wm.get_window("w1").unwrap().content_rect(&wm.theme);
    let event = InputEvent::PointerClick {
        x: cx + 10,
        y: cy + 20,
    };
    let result = wm.handle_input(&event, &mut sdi);
    assert_eq!(result, WmEvent::ContentClick(WindowId::from("w1"), 10, 20));
}

#[test]
fn click_desktop_returns_desktop_event() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let event = InputEvent::PointerClick { x: 700, y: 500 };
    let result = wm.handle_input(&event, &mut sdi);
    assert_eq!(result, WmEvent::DesktopClick(700, 500));
}

#[test]
fn click_close_button_closes_window() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (bx, by, bw, bh) = win.close_btn_rect(&wm.theme).unwrap();
    let event = InputEvent::PointerClick {
        x: bx + bw as i32 / 2,
        y: by + bh as i32 / 2,
    };
    let result = wm.handle_input(&event, &mut sdi);
    assert_eq!(result, WmEvent::WindowClosed(WindowId::from("w1")));
    assert_eq!(wm.window_count(), 0);
}

#[test]
fn release_without_drag_returns_none() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    let event = wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    assert_eq!(event, WmEvent::None);
}

#[test]
fn cursor_move_without_drag_returns_none() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    let event = wm.handle_input(&InputEvent::CursorMove { x: 50, y: 50 }, &mut sdi);
    assert_eq!(event, WmEvent::None);
}

#[test]
fn button_press_returns_none() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    let event = wm.handle_input(
        &InputEvent::ButtonPress(oasis_types::input::Button::Confirm),
        &mut sdi,
    );
    assert_eq!(event, WmEvent::None);
}

// ---- Double-click titlebar to toggle maximize ----

/// Click the titlebar drag region (left of any buttons) at a point guaranteed
/// to fall outside all window buttons.
fn click_titlebar(wm: &mut WindowManager, sdi: &mut SdiRegistry, id: &str) -> WmEvent {
    let win = wm.get_window(id).unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(wm.theme()).unwrap();
    wm.handle_input(
        &InputEvent::PointerClick {
            x: tx + 4,
            y: ty + th as i32 / 2,
        },
        sdi,
    )
}

#[test]
fn double_click_titlebar_maximizes() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Normal);

    click_titlebar(&mut wm, &mut sdi, "w1");
    // Release ends the drag started by the first click.
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    let result = click_titlebar(&mut wm, &mut sdi, "w1");

    assert_eq!(result, WmEvent::WindowMaximized(WindowId::from("w1")));
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Maximized);
}

#[test]
fn double_click_maximized_restores() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();
    wm.maximize_window("w1", &mut sdi).unwrap();

    click_titlebar(&mut wm, &mut sdi, "w1");
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    let result = click_titlebar(&mut wm, &mut sdi, "w1");

    assert_eq!(result, WmEvent::WindowRestored(WindowId::from("w1")));
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Normal);
}

#[test]
fn third_click_does_not_chain() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    click_titlebar(&mut wm, &mut sdi, "w1");
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    click_titlebar(&mut wm, &mut sdi, "w1"); // maximizes
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Maximized);

    // A third click shouldn't restore immediately — it starts a new
    // double-click cycle instead.
    let result = click_titlebar(&mut wm, &mut sdi, "w1");
    assert!(
        matches!(result, WmEvent::WindowFocused(_)),
        "third click should be a fresh focus, got {result:?}"
    );
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Maximized);
}

#[test]
fn dialog_double_click_titlebar_ignored() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

    // Dialog cannot maximize, so double-clicks on its titlebar must not
    // attempt to toggle state.
    click_titlebar(&mut wm, &mut sdi, "dlg");
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
    let result = click_titlebar(&mut wm, &mut sdi, "dlg");
    assert!(matches!(result, WmEvent::WindowFocused(_)));
}

#[test]
fn drag_then_click_does_not_double_click() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let (tx, ty, _tw, th) = win.titlebar_rect(wm.theme()).unwrap();
    let click_x = tx + 4;
    let click_y = ty + th as i32 / 2;

    // Click, drag well beyond the double-click radius, release.
    wm.handle_input(
        &InputEvent::PointerClick {
            x: click_x,
            y: click_y,
        },
        &mut sdi,
    );
    wm.handle_input(
        &InputEvent::CursorMove {
            x: click_x + 50,
            y: click_y + 30,
        },
        &mut sdi,
    );
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);

    // A click back at the original screen position must NOT trigger
    // maximize: the prior drag invalidated the titlebar stamp.
    let result = wm.handle_input(
        &InputEvent::PointerClick {
            x: click_x,
            y: click_y,
        },
        &mut sdi,
    );
    assert!(
        matches!(
            result,
            WmEvent::WindowFocused(_) | WmEvent::DesktopClick(_, _)
        ),
        "click after drag+release should not maximize, got {result:?}"
    );
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Normal);
}

#[test]
fn click_elsewhere_between_breaks_double_click() {
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::new(800, 600);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    click_titlebar(&mut wm, &mut sdi, "w1");
    wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);

    // Click into the content area. This should clear the titlebar stamp.
    let (cx, cy, _cw, _ch) = wm.get_window("w1").unwrap().content_rect(&wm.theme);
    wm.handle_input(
        &InputEvent::PointerClick {
            x: cx + 10,
            y: cy + 10,
        },
        &mut sdi,
    );

    let result = click_titlebar(&mut wm, &mut sdi, "w1");
    assert!(
        matches!(result, WmEvent::WindowFocused(_)),
        "titlebar click after content click must not double-click, got {result:?}"
    );
    assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Normal);
}

// ---- Button order (close at the corner on either side) ----

#[test]
fn button_order_right_side_is_minimize_maximize_close() {
    let theme = crate::window::WmTheme::default();
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::with_theme(800, 600, theme);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let min_x = win.minimize_btn_rect(&wm.theme).unwrap().0;
    let max_x = win.maximize_btn_rect(&wm.theme).unwrap().0;
    let close_x = win.close_btn_rect(&wm.theme).unwrap().0;

    assert!(
        min_x < max_x,
        "minimize ({min_x}) should be left of maximize ({max_x})"
    );
    assert!(
        max_x < close_x,
        "maximize ({max_x}) should be left of close ({close_x})"
    );
}

#[test]
fn button_order_left_side_is_close_minimize_maximize() {
    // macOS traffic-light order: close sits at the corner.
    let theme = crate::window::WmTheme {
        button_side: "left".to_string(),
        ..crate::window::WmTheme::default()
    };
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::with_theme(800, 600, theme);
    wm.create_window(&app_config("w1"), &mut sdi).unwrap();

    let win = wm.get_window("w1").unwrap();
    let min_x = win.minimize_btn_rect(&wm.theme).unwrap().0;
    let max_x = win.maximize_btn_rect(&wm.theme).unwrap().0;
    let close_x = win.close_btn_rect(&wm.theme).unwrap().0;

    assert!(
        close_x < min_x,
        "close ({close_x}) should be left of minimize ({min_x})"
    );
    assert!(
        min_x < max_x,
        "minimize ({min_x}) should be left of maximize ({max_x})"
    );
}

#[test]
fn dialog_close_button_at_edge_even_left_side() {
    // Dialog only has close. It should sit flush against the edge, not
    // offset by empty minimize/maximize slots.
    let theme = crate::window::WmTheme {
        button_side: "left".to_string(),
        ..crate::window::WmTheme::default()
    };
    let mut sdi = SdiRegistry::new();
    let mut wm = WindowManager::with_theme(800, 600, theme);
    wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

    let win = wm.get_window("dlg").unwrap();
    let (tx, _ty, _tw, _th) = win.titlebar_rect(&wm.theme).unwrap();
    let (bx, _, _, _) = win.close_btn_rect(&wm.theme).unwrap();
    // With left-side layout and only a close button, the button should be
    // near the titlebar's left edge — within one inset.
    let max_inset = wm.theme.titlebar_height as i32 / 2;
    assert!(
        bx - tx <= max_inset,
        "close button x={bx} too far from titlebar left tx={tx}"
    );
}
