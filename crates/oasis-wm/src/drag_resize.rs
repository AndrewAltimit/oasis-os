//! Drag/resize state machine for the window manager.
//!
//! Handles pointer-driven window dragging (via titlebar) and edge/corner
//! resizing. Also provides position clamping and edge-snapping helpers
//! that keep windows visible on screen.

use oasis_sdi::SdiRegistry;

use super::hit_test::{ButtonKind, HitRegion, ResizeEdge, hit_test};
use super::manager::{WindowManager, WmEvent};
use super::window::{Geometry, WindowId, WindowState};

/// Active drag/resize operation.
#[derive(Debug, Clone)]
pub(crate) enum DragState {
    /// Dragging a window by its titlebar.
    Moving {
        window_id: WindowId,
        /// Cursor position at drag start.
        start_cursor_x: i32,
        start_cursor_y: i32,
        /// Window position at drag start.
        start_win_x: i32,
        start_win_y: i32,
    },
    /// Resizing a window by a handle.
    Resizing {
        window_id: WindowId,
        edge: ResizeEdge,
        start_cursor_x: i32,
        start_cursor_y: i32,
        start_geometry: Geometry,
    },
}

/// Minimum window content width during resize.
pub(crate) const MIN_RESIZE_W: u32 = 80;

/// Minimum window content height during resize.
pub(crate) const MIN_RESIZE_H: u32 = 60;

/// Distance in pixels for edge snapping during drag.
pub(crate) const SNAP_DISTANCE: i32 = 8;

/// Minimum visible pixels of a window titlebar at screen edges.
/// At least 20px of the titlebar must remain on-screen so the user
/// can always grab and drag the window back.
pub(crate) const MIN_VISIBLE: i32 = 20;

impl WindowManager {
    pub(crate) fn handle_click(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) -> WmEvent {
        let region = hit_test(&self.windows, x, y, &self.theme);

        // If a modal window exists, only allow clicks on the topmost modal.
        if let Some(modal_id) = self.topmost_modal() {
            let hit_id = match &region {
                HitRegion::TitlebarButton(id, _)
                | HitRegion::Titlebar(id)
                | HitRegion::ResizeHandle(id, _)
                | HitRegion::Content(id, _, _) => Some(id.as_str()),
                HitRegion::Desktop => None,
            };
            if hit_id != Some(modal_id) {
                return WmEvent::None;
            }
        }

        match region {
            HitRegion::TitlebarButton(id, ButtonKind::Close) => {
                if let Err(e) = self.close_window(&id, sdi) {
                    log::debug!("close_window({id}): {e}");
                }
                WmEvent::WindowClosed(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Minimize) => {
                if let Err(e) = self.minimize_window(&id, sdi) {
                    log::debug!("minimize_window({id}): {e}");
                }
                WmEvent::WindowMinimized(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Maximize) => {
                // Toggle: if maximized, restore; otherwise maximize.
                let is_maximized = self
                    .windows
                    .iter()
                    .find(|w| w.id == id)
                    .map(|w| w.state == WindowState::Maximized)
                    .unwrap_or(false);

                if is_maximized {
                    if let Err(e) = self.restore_window(&id, sdi) {
                        log::debug!("restore_window({id}): {e}");
                    }
                    WmEvent::WindowRestored(id)
                } else {
                    self.focus_window_internal(&id, sdi);
                    if let Err(e) = self.maximize_window(&id, sdi) {
                        log::debug!("maximize_window({id}): {e}");
                    }
                    WmEvent::WindowMaximized(id)
                }
            },
            HitRegion::Titlebar(id) => {
                self.focus_window_internal(&id, sdi);
                // Start drag if the window is draggable.
                if let Some(window) = self.windows.iter().find(|w| w.id == id)
                    && window.is_draggable()
                {
                    self.drag = Some(DragState::Moving {
                        window_id: id.clone(),
                        start_cursor_x: x,
                        start_cursor_y: y,
                        start_win_x: window.x,
                        start_win_y: window.y,
                    });
                }
                WmEvent::WindowFocused(id)
            },
            HitRegion::ResizeHandle(id, edge) => {
                self.focus_window_internal(&id, sdi);
                if let Some(window) = self.windows.iter().find(|w| w.id == id) {
                    self.drag = Some(DragState::Resizing {
                        window_id: id.clone(),
                        edge,
                        start_cursor_x: x,
                        start_cursor_y: y,
                        start_geometry: Geometry {
                            x: window.x,
                            y: window.y,
                            w: window.outer_w,
                            h: window.outer_h,
                        },
                    });
                }
                WmEvent::WindowFocused(id)
            },
            HitRegion::Content(id, lx, ly) => {
                self.focus_window_internal(&id, sdi);
                WmEvent::ContentClick(id, lx, ly)
            },
            HitRegion::Desktop => {
                self.active_window = None;
                WmEvent::DesktopClick(x, y)
            },
        }
    }

    pub(crate) fn handle_cursor_move(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) -> WmEvent {
        let drag = match self.drag.clone() {
            Some(d) => d,
            None => {
                // No drag active -- check for button hover.
                self.update_button_hover(x, y, sdi);
                return WmEvent::None;
            },
        };

        match drag {
            DragState::Moving {
                ref window_id,
                start_cursor_x,
                start_cursor_y,
                start_win_x,
                start_win_y,
            } => {
                let raw_x = start_win_x + (x - start_cursor_x);
                let raw_y = start_win_y + (y - start_cursor_y);

                // Get dimensions for snap/clamp before mutable borrow.
                let dims = self
                    .windows
                    .iter()
                    .find(|w| w.id == *window_id)
                    .map(|w| (w.outer_w, w.outer_h));

                if let Some((outer_w, outer_h)) = dims {
                    let (sx, sy) =
                        snap_to_edges(raw_x, raw_y, outer_w, outer_h, self.screen_w, self.screen_h);
                    let (cx, cy) = clamp_position(
                        sx,
                        sy,
                        outer_w,
                        self.screen_w,
                        self.screen_h,
                        self.theme.titlebar_height,
                    );

                    if let Some(window) = self.windows.iter_mut().find(|w| w.id == *window_id) {
                        let dx = cx - window.x;
                        let dy = cy - window.y;
                        window.x = cx;
                        window.y = cy;

                        for suffix in window.sdi_suffixes() {
                            let name = window.sdi_name(suffix);
                            if let Ok(obj) = sdi.get_mut(&name) {
                                obj.x += dx;
                                obj.y += dy;
                            }
                        }
                    }
                }
                WmEvent::WindowMoved(window_id.clone())
            },
            DragState::Resizing {
                ref window_id,
                edge,
                start_cursor_x,
                start_cursor_y,
                start_geometry,
            } => {
                let dx = x - start_cursor_x;
                let dy = y - start_cursor_y;

                let (mut new_x, mut new_y, mut new_w, mut new_h) =
                    compute_resize(start_geometry, edge, dx, dy, &self.theme);

                // Clamp resize to screen bounds.
                let sw = self.screen_w as i32;
                let sh = self.screen_h as i32;
                if new_x < 0 {
                    new_w = new_w.saturating_sub((-new_x) as u32);
                    new_x = 0;
                }
                if new_y < 0 {
                    new_h = new_h.saturating_sub((-new_y) as u32);
                    new_y = 0;
                }
                if new_x + new_w as i32 > sw {
                    new_w = (sw - new_x).max(0) as u32;
                }
                if new_y + new_h as i32 > sh {
                    new_h = (sh - new_y).max(0) as u32;
                }

                if let Some(window) = self.windows.iter_mut().find(|w| w.id == *window_id) {
                    window.x = new_x;
                    window.y = new_y;
                    window.outer_w = new_w;
                    window.outer_h = new_h;
                }

                self.update_sdi_positions(window_id.as_str(), sdi);
                WmEvent::WindowResized(window_id.clone())
            },
        }
    }

    pub(crate) fn handle_release(&mut self) -> WmEvent {
        self.hover_button = None;
        if let Some(drag) = self.drag.take() {
            let id = match drag {
                DragState::Moving { window_id, .. } => window_id,
                DragState::Resizing { window_id, .. } => window_id,
            };
            return WmEvent::WindowMoved(id);
        }
        WmEvent::None
    }
}

/// Compute new geometry after a resize drag.
pub(crate) fn compute_resize(
    start: Geometry,
    edge: ResizeEdge,
    dx: i32,
    dy: i32,
    theme: &super::window::WmTheme,
) -> (i32, i32, u32, u32) {
    let min_w = MIN_RESIZE_W + theme.border_width * 2;
    let min_h = MIN_RESIZE_H + theme.titlebar_height + theme.border_width * 2;

    let mut x = start.x;
    let mut y = start.y;
    let mut w = start.w;
    let mut h = start.h;

    match edge {
        ResizeEdge::East => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
        },
        ResizeEdge::West => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
        },
        ResizeEdge::South => {
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::North => {
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
        ResizeEdge::SouthEast => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::SouthWest => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::NorthEast => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
        ResizeEdge::NorthWest => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
    }

    (x, y, w, h)
}

/// Clamp a window position to keep the titlebar partially visible on screen.
pub(crate) fn clamp_position(
    x: i32,
    y: i32,
    outer_w: u32,
    screen_w: u32,
    screen_h: u32,
    titlebar_height: u32,
) -> (i32, i32) {
    let sw = screen_w as i32;
    let sh = screen_h as i32;
    let tb_h = titlebar_height as i32;
    let cx = x.max(MIN_VISIBLE - outer_w as i32).min(sw - MIN_VISIBLE);
    let cy = y.max(0).min(sh - tb_h);
    (cx, cy)
}

/// Snap position to screen edges if within [`SNAP_DISTANCE`].
pub(crate) fn snap_to_edges(
    x: i32,
    y: i32,
    outer_w: u32,
    outer_h: u32,
    screen_w: u32,
    screen_h: u32,
) -> (i32, i32) {
    let sw = screen_w as i32;
    let sh = screen_h as i32;
    let mut sx = x;
    let mut sy = y;
    if sx.abs() < SNAP_DISTANCE {
        sx = 0;
    } else if (sx + outer_w as i32 - sw).abs() < SNAP_DISTANCE {
        sx = sw - outer_w as i32;
    }
    if sy.abs() < SNAP_DISTANCE {
        sy = 0;
    } else if (sy + outer_h as i32 - sh).abs() < SNAP_DISTANCE {
        sy = sh - outer_h as i32;
    }
    (sx, sy)
}

#[cfg(test)]
mod tests {
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
}
