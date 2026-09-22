#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! Input routing and window/desktop behaviour of the real desktop shell.
//!
//! Keyboard focus, text entry vs. global shortcuts (the gamepad twins a
//! keyboard backend synthesizes), window-manager shortcuts, Escape/Cancel
//! semantics per app, the on-screen keyboard, the pointer (overlapping
//! windows, scroll wheel), the taskbar and start menu, and window
//! drag/resize/maximize/snap through real mouse input, across skins.
//! See `docs/testing.md` for the harness API.

use oasis_app::Mode;
use oasis_app::harness::{Harness, HarnessOptions, WindowInfo, rect_center};
use oasis_core::input::{Button, InputEvent, Key, Modifiers};
use oasis_core::skin::builtin::builtin_names;
use oasis_core::wm::window::WindowState;

fn dump(h: &Harness, name: &str) -> String {
    let dir = std::env::temp_dir().join("oasis-e2e");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.png"));
    match h.save_png(&path) {
        Ok(()) => format!("(frame saved to {})", path.display()),
        Err(e) => format!("(frame dump failed: {e})"),
    }
}

/// Boot `skin` and let the entrance transition finish.
fn boot(skin: &str) -> Harness {
    let mut h = Harness::new(skin);
    h.settle();
    h
}

/// Open `title` from the dashboard (launch path, no clicking) and settle.
fn open(h: &mut Harness, title: &str) -> WindowInfo {
    assert!(h.open_app(title), "no dashboard app '{title}'");
    h.settle();
    h.find_window(title)
        .unwrap_or_else(|| panic!("{title} window open: {:?}", h.windows()))
}

/// Strings drawn (last presented frame) inside the content rect of the
/// window titled `title`.
fn text_in_window(h: &mut Harness, title: &str) -> Vec<String> {
    h.render_now();
    let (x, y, w, hh) = h.find_window(title).unwrap().content;
    h.frame_text_calls()
        .iter()
        .filter(|t| t.x >= x && t.y >= y && t.x < x + w as i32 && t.y < y + hh as i32)
        .map(|t| t.text.clone())
        .collect()
}

fn joined(v: &[String]) -> String {
    v.join("|")
}

/// A titlebar point of `title` clear of its buttons (a drag handle).
fn grab_point(h: &Harness, title: &str) -> (i32, i32) {
    let c = h.window_chrome(title).unwrap();
    let (tx, ty, tw, th) = c.titlebar.unwrap();
    let y = ty + th as i32 / 2;
    let hit = |x: i32, r: Option<(i32, i32, u32, u32)>| {
        r.is_some_and(|(bx, _, bw, _)| x >= bx - 2 && x < bx + bw as i32 + 2)
    };
    let mut x = tx + tw as i32 / 2;
    while hit(x, c.close) || hit(x, c.maximize) || hit(x, c.minimize) {
        x -= 4;
    }
    (x, y)
}

/// The Text Editor's cursor column from its status bar ("Ln 1, Col N").
fn editor_col(h: &mut Harness) -> usize {
    h.render_now();
    h.frame_text()
        .iter()
        .find_map(|t| t.split("Col ").nth(1).and_then(|c| c.trim().parse().ok()))
        .unwrap_or_else(|| panic!("editor status not drawn: {:?}", h.frame_text()))
}

/// Id of the window titled `title`.
fn id_of(h: &Harness, title: &str) -> String {
    h.find_window(title).unwrap().id
}

/// A point inside `a`'s content rect that is not inside `b`'s frame.
fn point_in_a_not_b(a: &WindowInfo, b: &WindowInfo) -> Option<(i32, i32)> {
    let (ax, ay, aw, ah) = a.content;
    let (bx, by, bw, bh) = b.frame;
    for dy in (4..ah as i32 - 4).step_by(6) {
        for dx in (4..aw as i32 - 4).step_by(6) {
            let (px, py) = (ax + dx, ay + dy);
            let in_b = px >= bx && py >= by && px < bx + bw as i32 && py < by + bh as i32;
            if !in_b {
                return Some((px, py));
            }
        }
    }
    None
}

/// Skins that run windowed desktops (window manager + titlebars).
fn windowed_skins() -> Vec<&'static str> {
    vec![
        "classic", "xp", "modern", "win95", "macos", "gnome", "desktop",
    ]
}

// ---------------------------------------------------------------------------
// Keyboard focus and text entry
// ---------------------------------------------------------------------------

/// Letters with gamepad twins (q = L trigger, e = R trigger, Space =
/// Triangle) typed into a text-entry window reach the app exactly once
/// and never fire the twin's global action.
#[test]
fn typing_in_text_apps_does_not_fire_gamepad_twins() {
    for skin in ["classic", "xp", "paper"] {
        let mut h = boot(skin);
        open(&mut h, "Text Editor");
        let tab = h.state().ui.bottom_bar.active_tab;
        let desk = h.state().ui.desktops.active_desktop();
        h.type_text("Qe qE eq");
        h.settle();
        let drawn = joined(&text_in_window(&mut h, "Text Editor"));
        assert!(
            drawn.contains("Qe qE eq"),
            "{skin}: typed text reached the editor once: {drawn} {}",
            dump(&h, &format!("typing_{skin}"))
        );
        assert_eq!(h.mode(), Mode::Desktop, "{skin}");
        assert_eq!(h.state().ui.bottom_bar.active_tab, tab, "{skin}: R twin");
        assert_eq!(h.state().ui.desktops.active_desktop(), desk, "{skin}");
        assert!(h.find_window("Text Editor").is_some(), "{skin}");
        assert!(h.state().active_transition.is_none(), "{skin}");
    }
}

/// Keyboard input goes to the focused window only; clicking another
/// window's titlebar moves the focus (and the typing) there.
#[test]
fn keyboard_focus_follows_the_active_window() {
    let mut h = boot("classic");
    open(&mut h, "Text Editor");
    let calc = open(&mut h, "Calculator");
    assert_eq!(h.active_window().as_deref(), Some(calc.id.as_str()));

    h.type_text("7");
    h.settle();
    assert_eq!(editor_col(&mut h), 1, "editor got calculator input");

    // Focus the editor by clicking its titlebar where the calculator
    // (cascaded down-right) does not cover it.
    let (_, ty, _, th) = h.window_chrome("Text Editor").unwrap().titlebar.unwrap();
    let ed_frame = h.find_window("Text Editor").unwrap().frame;
    h.click(ed_frame.0 + 12, ty + th as i32 / 2);
    h.settle();
    assert_eq!(
        h.active_window().as_deref(),
        Some(id_of(&h, "Text Editor").as_str())
    );
    h.type_text("zz");
    h.settle();
    assert_eq!(editor_col(&mut h), 3, "editor received the typing");
    assert!(h.text_drawn_contains("zz"));
}

// ---------------------------------------------------------------------------
// Window-manager keyboard shortcuts
// ---------------------------------------------------------------------------

#[test]
fn wm_keyboard_shortcuts_through_the_shell() {
    let super_ = Modifiers::SUPER;
    let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
    for skin in ["classic", "xp"] {
        let mut h = boot(skin);
        let ed = open(&mut h, "Text Editor");
        let calc = open(&mut h, "Calculator");
        let work = h.state().wm.work_area();

        // Alt+Tab cycles focus.
        h.key_with(Key::Tab, Modifiers::ALT);
        assert_eq!(h.active_window().as_deref(), Some(ed.id.as_str()), "{skin}");
        h.key_with(Key::Tab, Modifiers::ALT);
        assert_eq!(
            h.active_window().as_deref(),
            Some(calc.id.as_str()),
            "{skin}"
        );

        // Super+Up maximizes into the work area; Super+Down restores.
        h.key_with(Key::Up, super_);
        h.settle();
        let w = h.find_window("Calculator").unwrap();
        assert_eq!(
            w.frame,
            (work.x, work.y, work.w, work.h),
            "{skin}: maximized"
        );
        h.key_with(Key::Down, super_);
        h.settle();
        assert_eq!(
            h.find_window("Calculator").unwrap().frame,
            calc.frame,
            "{skin}"
        );

        // Ctrl+Alt+Left snaps the focused text editor left without typing.
        h.key_with(Key::Tab, Modifiers::ALT);
        h.key_with(Key::Left, ctrl_alt);
        h.settle();
        let w = h.find_window("Text Editor").unwrap();
        assert_eq!(
            (w.frame.0, w.frame.1, w.frame.2),
            (work.x, work.y, work.w / 2),
            "{skin}: snapped left"
        );
        // Ctrl+Alt+T tiles; a second window keeps its own tile.
        h.key_with(Key::Char('t'), ctrl_alt);
        h.settle();
        assert!(h.state().wm.tiling_layout().is_some(), "{skin}: tiling on");
        h.key_with(Key::Tab, Modifiers::ALT);
        let focused = h.active_window();
        if focused.as_deref() == Some(ed.id.as_str()) {
            assert_eq!(editor_col(&mut h), 1, "{skin}: shortcut typed into editor");
        }
        // Back to floating.
        for _ in 0..8 {
            if h.state().wm.tiling_layout().is_none() {
                break;
            }
            h.key_with(Key::Char('t'), super_);
        }
        assert!(h.state().wm.tiling_layout().is_none(), "{skin}");
        h.settle();
        h.render_now();
        assert!(h.windows().iter().all(|w| !w.minimized), "{skin}");
    }
}

// ---------------------------------------------------------------------------
// Escape / Cancel semantics
// ---------------------------------------------------------------------------

/// Every app closes on Escape at its top level (a fresh window), the
/// windowed terminal and browser included, and the shell lands back on
/// the dashboard. A second Escape on the dashboard is the quit gesture.
#[test]
fn escape_at_top_level_closes_every_app() {
    let mut h = boot("classic");
    let apps = h.dashboard_apps();
    for app in &apps {
        open(&mut h, app);
        h.key(Key::Escape);
        h.settle();
        assert!(
            h.find_window(app).is_none(),
            "Escape closes a fresh {app}: {:?} {}",
            h.windows(),
            dump(&h, &format!("escape_{app}"))
        );
        assert_eq!(h.mode(), Mode::Dashboard, "{app}");
        assert!(!h.quit_requested(), "{app}: Escape must not quit");
        assert!(h.state().content.open_runners.is_empty(), "{app}");
    }
}

/// Escape closes only the focused window; the one below keeps running and
/// takes the focus.
#[test]
fn escape_closes_only_the_focused_window() {
    let mut h = boot("xp");
    let ed = open(&mut h, "Text Editor");
    open(&mut h, "Paint");
    h.key(Key::Escape);
    h.settle();
    assert!(h.find_window("Paint").is_none());
    assert!(h.find_window("Text Editor").is_some());
    assert_eq!(h.active_window().as_deref(), Some(ed.id.as_str()));
    assert_eq!(h.mode(), Mode::Desktop);
}

/// Escape inside a sub-view backs out of it instead of closing the app.
#[test]
fn escape_in_file_manager_subfolder_backs_out_first() {
    let mut h = boot("classic");
    open(&mut h, "File Manager");
    // Enter the first folder of the listing.
    h.key(Key::Enter);
    h.settle();
    h.key(Key::Escape);
    h.settle();
    assert!(
        h.find_window("File Manager").is_some(),
        "first Escape stays in the app: {:?}",
        h.windows()
    );
}

/// The start menu owns the keyboard while it is open: Enter picks the
/// highlighted item and Escape closes the menu (it must neither launch the
/// dashboard's selected icon nor quit the shell).
#[test]
fn start_menu_keyboard_navigation_on_the_dashboard() {
    let mut h = boot("classic");
    let (x, y, w, bh) = h.sdi_rect("start_btn_bg").unwrap();
    let start = (x + w as i32 / 2, y + bh as i32 / 2);
    h.click(start.0, start.1);
    h.settle();
    assert!(h.state().ui.start_menu.open);
    h.key(Key::Escape);
    h.settle();
    assert!(!h.quit_requested(), "Escape with the start menu open quit");
    assert!(!h.state().ui.start_menu.open, "Escape closes the menu");
    assert_eq!(h.mode(), Mode::Dashboard);

    h.click(start.0, start.1);
    h.settle();
    h.key(Key::Down);
    h.settle();
    let sel = h.state().ui.start_menu.selected;
    assert!(sel > 0, "Down moves the menu selection");
    h.key(Key::Enter);
    h.settle();
    assert!(!h.state().ui.start_menu.open, "Enter picks an item");
    // Enter must not also launch the dashboard's selected icon.
    let dash_sel = h
        .state()
        .ui
        .dashboard
        .selected_app()
        .map(|a| a.title.clone())
        .unwrap();
    let wins: Vec<String> = h.windows().into_iter().map(|w| w.title).collect();
    assert!(
        wins.len() <= 1,
        "Enter launched the menu item and the dashboard icon: {wins:?}"
    );
    if wins.len() == 1 {
        assert_ne!(wins[0], dash_sel, "Enter went to the dashboard icon");
    }
}

/// Same with windows open: arrows and Enter drive the menu, not the app.
#[test]
fn start_menu_keyboard_navigation_over_windows() {
    let mut h = boot("xp");
    open(&mut h, "Text Editor");
    let (x, y, w, bh) = h.sdi_rect("start_btn_bg").unwrap();
    h.click(x + w as i32 / 2, y + bh as i32 / 2);
    h.settle();
    assert!(h.state().ui.start_menu.open);
    h.key(Key::Down);
    assert!(
        h.state().ui.start_menu.selected > 0,
        "Down reaches the menu"
    );
    h.key(Key::Escape);
    h.settle();
    assert!(!h.state().ui.start_menu.open, "Escape closes the menu");
    assert!(
        h.find_window("Text Editor").is_some(),
        "Escape for the menu must not close the editor"
    );
    h.type_text("ok");
    let editor = joined(&text_in_window(&mut h, "Text Editor"));
    assert!(editor.contains("ok"), "{editor}");
}

// ---------------------------------------------------------------------------
// On-screen keyboard
// ---------------------------------------------------------------------------

#[test]
fn osk_opens_types_and_closes() {
    let mut h = boot("classic");
    h.key(Key::F(2));
    h.settle();
    assert_eq!(h.mode(), Mode::Osk);
    assert!(h.sdi_rect("osk_bg").is_some(), "OSK drawn");
    // Type two characters, cycle the mode, then confirm with Start.
    h.button(Button::Confirm);
    h.button(Button::Right);
    h.button(Button::Confirm);
    h.button(Button::Triangle);
    h.button(Button::Triangle);
    h.button(Button::Start);
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
    assert!(h.state().osk.is_none());
    assert!(
        h.state()
            .terminal
            .output_lines
            .iter()
            .any(|l| l.starts_with("[OSK] Input: ") && l.len() > "[OSK] Input: ".len()),
        "{:?}",
        h.state().terminal.output_lines
    );
    let leftovers: Vec<String> = h
        .shell
        .sdi
        .names()
        .filter(|n| n.starts_with("osk_"))
        .filter(|n| h.shell.sdi.get(n).is_ok_and(|o| o.visible))
        .map(str::to_string)
        .collect();
    assert!(
        leftovers.is_empty(),
        "OSK objects still visible: {leftovers:?}"
    );

    // Escape cancels.
    h.key(Key::F(2));
    h.key(Key::Escape);
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
    assert!(!h.quit_requested());
}

/// OSK opened from the terminal types into the terminal's input line and
/// returns to the terminal.
#[test]
fn osk_from_the_terminal_types_into_the_prompt() {
    let mut h = boot("classic");
    h.button(Button::Start);
    h.settle();
    assert_eq!(h.mode(), Mode::Terminal);
    h.button(Button::Select);
    assert_eq!(h.mode(), Mode::Osk);
    h.button(Button::Confirm); // first key of the grid
    h.button(Button::Start);
    h.settle();
    assert_eq!(h.mode(), Mode::Terminal, "back to the terminal");
    assert!(
        !h.state().terminal.session.buffer().is_empty(),
        "OSK text lands on the prompt"
    );
}

// ---------------------------------------------------------------------------
// Pointer: overlapping windows, scroll wheel, cursor
// ---------------------------------------------------------------------------

/// A click where two windows overlap goes to the top one only; a click on
/// the uncovered part of the lower window raises it.
#[test]
fn clicks_on_overlapping_windows_hit_only_the_top_one() {
    for skin in windowed_skins() {
        let mut h = boot(skin);
        let ed = open(&mut h, "Text Editor");
        let calc = open(&mut h, "Calculator");
        // Stack the calculator right over the editor's content.
        let (ex, ey, _, _) = ed.content;
        let grab = grab_point(&h, "Calculator");
        let off = grab.0 - calc.frame.0;
        h.drag(grab, (ex + 20 + off, ey + 10), 6);
        h.settle();
        let calc = h.find_window("Calculator").unwrap();
        let ed = h.find_window("Text Editor").unwrap();
        let (cx, cy, cw, ch) = calc.content;
        let both = (cx + cw as i32 / 2, cy + ch as i32 / 2);
        let (ex, ey, ew, eh) = ed.content;
        assert!(
            both.0 >= ex && both.1 >= ey && both.0 < ex + ew as i32 && both.1 < ey + eh as i32,
            "{skin}: test setup: point in both windows {ed:?} {calc:?}"
        );
        let order: Vec<String> = h.windows().into_iter().map(|w| w.id).collect();
        h.click(both.0, both.1);
        h.settle();
        assert_eq!(
            h.active_window().as_deref(),
            Some(calc.id.as_str()),
            "{skin}: top window keeps focus"
        );
        let after: Vec<String> = h.windows().into_iter().map(|w| w.id).collect();
        assert_eq!(order, after, "{skin}: z-order unchanged");

        let p = point_in_a_not_b(&ed, &calc).expect("uncovered editor area");
        h.click(p.0, p.1);
        h.settle();
        assert_eq!(
            h.active_window().as_deref(),
            Some(ed.id.as_str()),
            "{skin}: clicking the lower window raises it"
        );
        assert_eq!(
            h.windows().last().map(|w| w.id.clone()),
            Some(ed.id.clone()),
            "{skin}: raised to the top"
        );
    }
}

/// The wheel scrolls the window under the pointer, not merely the
/// focused one.
#[test]
fn scroll_wheel_goes_to_the_window_under_the_pointer() {
    let mut h = boot("classic");
    // A long terminal scrollback.
    let term = open(&mut h, "Terminal");
    for i in 0..60 {
        h.state_mut()
            .terminal
            .output_lines
            .push(format!("line {i}"));
    }
    h.state_mut().terminal.dirty = true;
    h.settle();
    // Focus another window that does not overlap the terminal.
    let calc = open(&mut h, "Calculator");
    let grab = grab_point(&h, "Calculator");
    let (fx, _, fw, _) = term.frame;
    let off = grab.0 - calc.frame.0;
    h.drag(grab, (fx + fw as i32 + 20 + off, grab.1 + 200), 6);
    h.settle();
    assert_eq!(h.active_window().as_deref(), Some(calc.id.as_str()));
    let term = h.find_window("Terminal").unwrap();
    let (x, y, w, hh) = term.content;
    h.move_to(x + w as i32 / 2, y + hh as i32 / 2);
    assert_eq!(h.state().terminal.scroll_offset, 0);
    h.scroll(-2);
    assert!(
        h.state().terminal.scroll_offset > 0,
        "wheel over the unfocused terminal scrolls it"
    );
    assert_eq!(
        h.active_window().as_deref(),
        Some(calc.id.as_str()),
        "scrolling does not steal focus"
    );
}

#[test]
fn cursor_tracks_pointer_and_hover_follows_it() {
    let mut h = boot("classic");
    let (x, y, w, hh) = h.app_icon_rect("Calculator").unwrap();
    let (cx, cy) = (x + w as i32 / 2, y + hh as i32 / 2);
    h.move_to(cx, cy);
    assert_eq!(
        (h.state().ui.mouse_cursor.x, h.state().ui.mouse_cursor.y),
        (cx, cy)
    );
    let idx = h
        .dashboard_apps()
        .iter()
        .position(|a| a == "Calculator")
        .unwrap();
    assert_eq!(h.state().ui.dashboard.hover_index, Some(idx));
    // Behind a window the icon is not hoverable.
    open(&mut h, "Text Editor");
    let ed = h.find_window("Text Editor").unwrap();
    let (fx, fy, fw, fh) = ed.frame;
    if let Some(i) = h
        .state()
        .ui
        .dashboard
        .icon_at(fx + fw as i32 / 2, fy + fh as i32 / 2)
    {
        h.move_to(fx + fw as i32 / 2, fy + fh as i32 / 2);
        assert_ne!(h.state().ui.dashboard.hover_index, Some(i));
    }
}

// ---------------------------------------------------------------------------
// Taskbar
// ---------------------------------------------------------------------------

fn taskbar_button_for(h: &Harness, title: &str) -> Option<(i32, i32)> {
    // The taskbar hit test is authoritative; scan the bar's row.
    let (w, hh) = h.size();
    let id = h.find_window(title)?.id;
    for y in (hh as i32 - 60..hh as i32).rev().step_by(3) {
        for x in (0..w as i32).step_by(4) {
            if h.state().ui.taskbar.hit_test(x, y) == Some(id.as_str()) {
                return Some((x + 6, y));
            }
        }
    }
    None
}

#[test]
fn taskbar_buttons_focus_minimize_and_restore() {
    for skin in ["classic", "xp", "win95"] {
        let mut h = boot(skin);
        let ed = open(&mut h, "Text Editor");
        let calc = open(&mut h, "Calculator");
        h.render_now();
        let ed_btn = taskbar_button_for(&h, "Text Editor")
            .unwrap_or_else(|| panic!("{skin}: taskbar button {}", dump(&h, "taskbar")));
        // Inactive -> focus.
        h.click(ed_btn.0, ed_btn.1);
        h.settle();
        assert_eq!(h.active_window().as_deref(), Some(ed.id.as_str()), "{skin}");
        // Active -> minimize; focus falls to the calculator.
        h.click(ed_btn.0, ed_btn.1);
        h.settle();
        assert!(h.find_window("Text Editor").unwrap().minimized, "{skin}");
        assert_eq!(
            h.active_window().as_deref(),
            Some(calc.id.as_str()),
            "{skin}"
        );
        // Typing goes to the calculator, not the hidden editor.
        h.type_text("x");
        // Minimized -> restore + focus.
        h.click(ed_btn.0, ed_btn.1);
        h.settle();
        let w = h.find_window("Text Editor").unwrap();
        assert!(!w.minimized, "{skin}");
        assert_eq!(w.frame, ed.frame, "{skin}: geometry restored");
        assert_eq!(h.active_window().as_deref(), Some(ed.id.as_str()), "{skin}");
    }
}

/// Launching an app whose window is minimized brings the window back
/// instead of focusing an invisible window.
#[test]
fn relaunching_a_minimized_app_restores_its_window() {
    let mut h = boot("classic");
    let ed = open(&mut h, "Text Editor");
    h.state_mut();
    {
        let shell = &mut h.shell;
        shell
            .state
            .wm
            .minimize_window(&ed.id, &mut shell.sdi)
            .unwrap();
    }
    h.settle();
    assert!(h.find_window("Text Editor").unwrap().minimized);
    h.open_app("Text Editor");
    h.settle();
    let w = h.find_window("Text Editor").unwrap();
    assert!(!w.minimized, "relaunch restores the minimized window");
    assert_eq!(h.active_window().as_deref(), Some(ed.id.as_str()));
    h.type_text("hi");
    let editor = joined(&text_in_window(&mut h, "Text Editor"));
    assert!(editor.contains("hi"), "{editor}");
}

/// With every window minimized, Escape still gets back to the dashboard.
#[test]
fn all_windows_minimized_escape_returns_to_dashboard() {
    let mut h = boot("classic");
    let ed = open(&mut h, "Text Editor");
    {
        let shell = &mut h.shell;
        shell
            .state
            .wm
            .minimize_window(&ed.id, &mut shell.sdi)
            .unwrap();
    }
    h.settle();
    assert_eq!(h.active_window(), None);
    h.key(Key::Escape);
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
    assert!(!h.quit_requested());
}

// ---------------------------------------------------------------------------
// Windows through real mouse input
// ---------------------------------------------------------------------------

#[test]
fn drag_resize_maximize_and_snap_with_the_mouse() {
    for skin in windowed_skins() {
        let mut h = boot(skin);
        let w0 = open(&mut h, "Paint");
        let work = h.state().wm.work_area();
        let grab = grab_point(&h, "Paint");

        // Drag by the titlebar.
        h.drag(grab, (grab.0 + 40, grab.1 + 30), 5);
        h.settle();
        let w1 = h.find_window("Paint").unwrap();
        assert_eq!(
            (w1.frame.0, w1.frame.1),
            (w0.frame.0 + 40, w0.frame.1 + 30),
            "{skin}: moved"
        );

        // Resize from the bottom-right corner.
        let (fx, fy, fw, fh) = w1.frame;
        let corner = (fx + fw as i32 - 2, fy + fh as i32 - 2);
        h.drag(corner, (corner.0 + 50, corner.1 + 40), 5);
        h.settle();
        let w2 = h.find_window("Paint").unwrap();
        assert!(
            w2.frame.2 > fw && w2.frame.3 > fh,
            "{skin}: resized {:?} -> {:?}",
            w1.frame,
            w2.frame
        );

        // Maximize button fills the work area; again restores.
        let max = h.window_chrome("Paint").unwrap().maximize;
        if let Some(max) = max {
            let (mx, my) = rect_center(max);
            h.click(mx, my);
            h.settle();
            let w3 = h.find_window("Paint").unwrap();
            assert_eq!(
                w3.frame,
                (work.x, work.y, work.w, work.h),
                "{skin}: maximized"
            );
            let (mx, my) = rect_center(h.window_chrome("Paint").unwrap().maximize.unwrap());
            h.click(mx, my);
            h.settle();
            assert_eq!(h.find_window("Paint").unwrap().frame, w2.frame, "{skin}");
        }

        // Drag to the left edge snaps to the left half.
        let grab = grab_point(&h, "Paint");
        h.drag(grab, (1, grab.1.max(work.y + 40)), 8);
        h.settle();
        let w4 = h.find_window("Paint").unwrap();
        assert_eq!(
            (w4.frame.0, w4.frame.2),
            (work.x, work.w / 2),
            "{skin}: snapped left {}",
            dump(&h, &format!("snap_{skin}"))
        );

        // Minimize button hides it (taskbar keeps it reachable).
        if let Some(min) = h.window_chrome("Paint").unwrap().minimize {
            let (mx, my) = rect_center(min);
            h.click(mx, my);
            h.settle();
            assert!(h.find_window("Paint").unwrap().minimized, "{skin}");
            assert_eq!(h.active_window(), None, "{skin}");
        }
    }
}

/// Close via titlebar button, via Escape, and via the app's own Exit
/// (Calculator: Escape on an empty display = Cancel -> Exit).
#[test]
fn close_paths_all_release_the_runner() {
    for skin in windowed_skins() {
        let mut h = boot(skin);
        open(&mut h, "Calculator");
        assert!(h.close_window("Calculator"), "{skin}");
        h.settle();
        assert!(h.find_window("Calculator").is_none(), "{skin}");

        open(&mut h, "Calculator");
        h.type_text("12");
        h.key(Key::Escape); // clears the entry
        h.settle();
        assert!(h.find_window("Calculator").is_some(), "{skin}: clear first");
        h.key(Key::Escape); // now exits
        h.settle();
        assert!(h.find_window("Calculator").is_none(), "{skin}");
        assert!(h.state().content.open_runners.is_empty(), "{skin}");
        assert_eq!(h.mode(), Mode::Dashboard, "{skin}");
    }
}

// ---------------------------------------------------------------------------
// Live resolution change keeps windows reachable
// ---------------------------------------------------------------------------

fn request_resolution(h: &mut Harness, w: u32, hh: u32) {
    use oasis_core::vfs::Vfs;
    h.vfs_mut()
        .write(
            oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH,
            format!("{w}x{hh}").as_bytes(),
        )
        .unwrap();
    h.settle();
    assert_eq!(h.size(), (w, hh));
}

fn assert_window_reachable(h: &Harness, title: &str, what: &str) {
    let (sw, sh) = h.size();
    let w = h.find_window(title).unwrap();
    let chrome = h.window_chrome(title).unwrap();
    let inside = |(x, y, bw, bh): (i32, i32, u32, u32)| {
        x >= 0 && y >= 0 && x + bw as i32 <= sw as i32 && y + bh as i32 <= sh as i32
    };
    if !w.fullscreen {
        let close = chrome.close.expect("close button");
        assert!(
            inside(close),
            "{what}: {title} close button {close:?} off a {sw}x{sh} screen (frame {:?}) {}",
            w.frame,
            dump(h, what)
        );
    }
    let state = h.state().wm.get_window(&w.id).map(|w| w.state).unwrap();
    if state == WindowState::Maximized {
        let work = h.state().wm.work_area();
        assert_eq!(
            w.frame,
            (work.x, work.y, work.w, work.h),
            "{what}: maximized {title} fills the new work area"
        );
    }
}

#[test]
fn windows_stay_reachable_across_resolution_changes() {
    let mut h = boot("classic");
    open(&mut h, "Calculator");
    let paint = open(&mut h, "Paint");
    // Maximize Paint, push the calculator to the bottom-right corner.
    let (mx, my) = rect_center(h.window_chrome("Paint").unwrap().maximize.unwrap());
    h.click(mx, my);
    h.settle();
    let _ = paint;
    let (sw, sh) = h.size();
    // Focus the calculator via Alt+Tab, then move it with the mouse.
    h.key_with(Key::Tab, Modifiers::ALT);
    let grab = grab_point(&h, "Calculator");
    h.drag(grab, (sw as i32 - 30, sh as i32 - 60), 8);
    h.settle();

    for (w, hh) in [(640, 480), (1920, 1080), (800, 600)] {
        request_resolution(&mut h, w, hh);
        let what = format!("res_{w}x{hh}");
        assert_window_reachable(&h, "Paint", &what);
        assert_window_reachable(&h, "Calculator", &what);
    }
    // The maximized window restores to a geometry that fits the screen.
    let (mx, my) = rect_center(h.window_chrome("Paint").unwrap().maximize.unwrap());
    h.click(mx, my);
    h.settle();
    assert_window_reachable(&h, "Paint", "restored_after_resize");
    // And it still closes with the mouse.
    assert!(h.close_window("Paint"));
    h.settle();
    assert!(h.find_window("Paint").is_none());
}

/// A kiosk-fullscreen window refits the new screen too.
#[test]
fn fullscreen_window_refits_on_resolution_change() {
    let mut h = boot("classic");
    open(&mut h, "Paint");
    h.send(&[InputEvent::ToggleFullscreen]);
    h.settle();
    assert!(h.find_window("Paint").unwrap().fullscreen);
    request_resolution(&mut h, 800, 600);
    let w = h.find_window("Paint").unwrap();
    assert_eq!(
        w.frame,
        (0, 0, 800, 600),
        "kiosk window fills the new screen"
    );
    h.send(&[InputEvent::ToggleFullscreen]);
    h.settle();
    assert_window_reachable(&h, "Paint", "kiosk_exit_after_resize");
}

// ---------------------------------------------------------------------------
// Skins: the key flows on dashboard-style and windowed skins
// ---------------------------------------------------------------------------

#[test]
fn key_flows_on_every_skin() {
    for name in builtin_names() {
        let mut h = Harness::with_options(HarnessOptions::new(name)).unwrap();
        h.settle();
        let apps = h.dashboard_apps();
        let app = if apps.iter().any(|a| a == "Calculator") {
            "Calculator"
        } else {
            apps[0].as_str()
        }
        .to_string();
        // Keyboard: select + Enter launches something.
        h.key(Key::Enter);
        h.settle();
        assert_eq!(h.mode(), Mode::Desktop, "{name}: Enter launches");
        // Escape closes it again.
        for _ in 0..3 {
            if h.mode() == Mode::Dashboard {
                break;
            }
            h.key(Key::Escape);
            h.settle();
        }
        assert_eq!(h.mode(), Mode::Dashboard, "{name}: back to the dashboard");
        // Mouse: click an icon, then close with the titlebar button.
        let found = h.click_app_icon(&app);
        if found {
            h.settle();
            assert!(h.find_window(&app).is_some(), "{name}: {app} opened");
            assert!(h.close_window(&app), "{name}: close button");
            h.settle();
            assert_eq!(h.mode(), Mode::Dashboard, "{name}");
        }
        assert!(!h.quit_requested(), "{name}");
        // Terminal round trip.
        h.key(Key::F(1));
        h.settle();
        if h.mode() == Mode::Terminal {
            h.key(Key::Escape);
            h.settle();
        } else if let Some(t) = h.find_window("Terminal") {
            let _ = t;
            h.key(Key::Escape);
            h.settle();
        }
        assert_eq!(h.mode(), Mode::Dashboard, "{name}: terminal round trip");
    }
}
