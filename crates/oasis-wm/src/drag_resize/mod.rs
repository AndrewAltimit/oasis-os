//! Drag/resize state machine for the window manager.
//!
//! Handles pointer-driven window dragging (via titlebar) and edge/corner
//! resizing. Also provides position clamping and edge-snapping helpers
//! that keep windows visible on screen.

use oasis_sdi::SdiRegistry;

use super::hit_test::{ButtonKind, HitRegion, ResizeEdge, hit_test};
use super::manager::{
    DOUBLE_CLICK_RADIUS, DOUBLE_CLICK_WINDOW, WindowManager, WmEvent, move_window_to,
};
use super::window::{Geometry, WindowId, WindowState, WmTheme};

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

impl DragState {
    /// The window being moved or resized.
    pub(crate) fn window_id(&self) -> &WindowId {
        match self {
            DragState::Moving { window_id, .. } | DragState::Resizing { window_id, .. } => {
                window_id
            },
        }
    }
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
                self.last_titlebar_click = None;
                if let Err(e) = self.close_window(&id, sdi) {
                    log::debug!("close_window({id}): {e}");
                }
                WmEvent::WindowClosed(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Minimize) => {
                self.last_titlebar_click = None;
                if let Err(e) = self.minimize_window(&id, sdi) {
                    log::debug!("minimize_window({id}): {e}");
                }
                WmEvent::WindowMinimized(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Maximize) => {
                self.last_titlebar_click = None;
                self.toggle_maximize(&id, sdi)
            },
            HitRegion::Titlebar(id) => {
                self.focus_window_internal(&id, sdi);

                // Double-click on the titlebar body toggles maximize/restore.
                let is_double = self.last_titlebar_click.as_ref().is_some_and(|last| {
                    last.0 == id
                        && last.1.elapsed() <= DOUBLE_CLICK_WINDOW
                        && (last.2 - x).abs() <= DOUBLE_CLICK_RADIUS
                        && (last.3 - y).abs() <= DOUBLE_CLICK_RADIUS
                });
                let can_maximize = self
                    .windows
                    .iter()
                    .find(|w| w.id == id)
                    .map(|w| w.has_maximize_button())
                    .unwrap_or(false);

                if is_double && can_maximize {
                    // Consume the stamp so a third click doesn't chain.
                    self.last_titlebar_click = None;
                    return self.toggle_maximize(&id, sdi);
                }

                self.last_titlebar_click = Some((id.clone(), web_time::Instant::now(), x, y));

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
                self.last_titlebar_click = None;
                self.focus_window_internal(&id, sdi);
                if let Some(window) = self.windows.iter().find(|w| w.id == id) {
                    self.drag = Some(DragState::Resizing {
                        window_id: id.clone(),
                        edge,
                        start_cursor_x: x,
                        start_cursor_y: y,
                        start_geometry: window.geometry(),
                    });
                }
                WmEvent::WindowFocused(id)
            },
            HitRegion::Content(id, lx, ly) => {
                self.last_titlebar_click = None;
                self.focus_window_internal(&id, sdi);
                WmEvent::ContentClick(id, lx, ly)
            },
            HitRegion::Desktop => {
                self.last_titlebar_click = None;
                self.active_window = None;
                WmEvent::DesktopClick(x, y)
            },
        }
    }

    /// Toggle maximize/restore state for a window that supports it.
    /// Returns the appropriate WmEvent. If the window doesn't support
    /// maximize, returns [`WmEvent::None`].
    pub(crate) fn toggle_maximize(&mut self, id: &WindowId, sdi: &mut SdiRegistry) -> WmEvent {
        let Some(window) = self.windows.iter().find(|w| w.id == *id) else {
            return WmEvent::None;
        };
        if !window.has_maximize_button() {
            return WmEvent::None;
        }
        let is_maximized = window.state == WindowState::Maximized;

        if is_maximized {
            if let Err(e) = self.restore_window(id.as_str(), sdi) {
                log::debug!("restore_window({id}): {e}");
            }
            WmEvent::WindowRestored(id.clone())
        } else {
            self.focus_window_internal(id, sdi);
            if let Err(e) = self.maximize_window(id.as_str(), sdi) {
                log::debug!("maximize_window({id}): {e}");
            }
            WmEvent::WindowMaximized(id.clone())
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
                // Actual drag movement invalidates any pending double-click.
                // Jitter within DOUBLE_CLICK_RADIUS still counts as a steady
                // click so release-then-reclick at the same spot toggles maximize.
                let mut start_win_x = start_win_x;
                let mut start_win_y = start_win_y;
                // Saturating: a pointer jumping across the whole i32 range
                // (multi-monitor coordinates) must not overflow.
                let (dx, dy) = (
                    x.saturating_sub(start_cursor_x),
                    y.saturating_sub(start_cursor_y),
                );
                if dx.unsigned_abs() > DOUBLE_CLICK_RADIUS.unsigned_abs()
                    || dy.unsigned_abs() > DOUBLE_CLICK_RADIUS.unsigned_abs()
                {
                    self.last_titlebar_click = None;
                    // Dragging a snapped window out restores its size.
                    if let Some((nx, ny)) = self.unsnap_for_drag(window_id, start_cursor_x, sdi) {
                        start_win_x = nx;
                        start_win_y = ny;
                    }
                }

                let raw_x = start_win_x.saturating_add(dx);
                let raw_y = start_win_y.saturating_add(dy);

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
                        move_window_to(window, cx, cy, sdi);
                    }
                }
                self.update_drag_snap_preview(window_id.as_str(), x, y);
                WmEvent::WindowMoved(window_id.clone())
            },
            DragState::Resizing {
                ref window_id,
                edge,
                start_cursor_x,
                start_cursor_y,
                start_geometry,
            } => {
                let dx = i64::from(x) - i64::from(start_cursor_x);
                let dy = i64::from(y) - i64::from(start_cursor_y);
                let g = resize_on_screen(
                    start_geometry,
                    edge,
                    (dx, dy),
                    (self.screen_w, self.screen_h),
                    &self.theme,
                );

                if let Some(window) = self.windows.iter_mut().find(|w| w.id == *window_id) {
                    window.set_geometry(g);
                    // A hand-resized window is no longer snapped.
                    window.snap_zone = None;
                    window.pre_snap_geometry = None;
                }

                self.update_sdi_positions(window_id.as_str(), sdi);
                WmEvent::WindowResized(window_id.clone())
            },
        }
    }

    pub(crate) fn handle_release(&mut self, sdi: &mut SdiRegistry) -> WmEvent {
        self.hover_button = None;
        if let Some(drag) = self.drag.take() {
            let id = match drag {
                DragState::Moving { window_id, .. } => {
                    // Released inside a snap zone: snap the window there.
                    if let Some(ev) = self.apply_drag_snap(window_id.as_str(), sdi) {
                        return ev;
                    }
                    window_id
                },
                DragState::Resizing { window_id, .. } => window_id,
            };
            self.snap.clear_preview();
            return WmEvent::WindowMoved(id);
        }
        self.snap.clear_preview();
        WmEvent::None
    }
}

impl WindowManager {
    /// Drop an in-progress drag / resize of `id`.
    ///
    /// Every programmatic geometry or state change (close, minimize,
    /// maximize, restore, fullscreen, snap, tiling, `move_window` /
    /// `resize_window`) calls this: the drag computes each frame from the
    /// geometry it started with, so letting it continue would undo the
    /// change on the next cursor move (e.g. a tile resized from its stale
    /// pre-tiling size, or a maximized window dragged around).
    pub(crate) fn cancel_drag_of(&mut self, id: &str) {
        if self.drag.as_ref().is_some_and(|d| d.window_id() == id) {
            self.drag = None;
        }
    }

    /// If the window being dragged is snapped or maximized, restore its
    /// floating size so it detaches from the edge, keeping the grab point
    /// under the cursor proportionally. Updates the drag state's origin and
    /// returns the new drag origin `(start_win_x, start_win_y)`; `None` if
    /// the window was floating.
    fn unsnap_for_drag(
        &mut self,
        id: &WindowId,
        start_cursor_x: i32,
        sdi: &mut SdiRegistry,
    ) -> Option<(i32, i32)> {
        let window = self.windows.iter_mut().find(|w| w.id == *id)?;
        let orig = if window.state == WindowState::Maximized {
            // Dragging a maximized window by its titlebar un-maximizes it
            // (otherwise a screen-sized window would slide off-screen while
            // still claiming to be maximized). A window snapped before it
            // was maximized detaches from both, back to its floating size:
            // checking the snap first used to leave it marked maximized.
            let pre_snap = window.pre_snap_geometry.take();
            let saved = window.saved_geometry.take();
            let g = pre_snap.or(saved)?;
            window.snap_zone = None;
            window.state = WindowState::Normal;
            g
        } else if window.snap_zone.is_some() {
            let g = window.pre_snap_geometry.take()?;
            window.snap_zone = None;
            g
        } else {
            return None;
        };
        let old_w = window.outer_w.max(1) as i64;
        let grab = i64::from(start_cursor_x - window.x);
        let new_grab = (grab * i64::from(orig.w) / old_w) as i32;
        window.x = start_cursor_x - new_grab;
        window.outer_w = orig.w;
        window.outer_h = orig.h;
        let (nx, ny) = (window.x, window.y);
        if let Some(DragState::Moving {
            start_win_x,
            start_win_y,
            ..
        }) = self.drag.as_mut()
        {
            *start_win_x = nx;
            *start_win_y = ny;
        }
        self.update_sdi_positions(id.as_str(), sdi);
        Some((nx, ny))
    }
}

/// Which sides of the frame a resize handle moves:
/// `(left, right, top, bottom)`.
fn edge_sides(edge: ResizeEdge) -> (bool, bool, bool, bool) {
    match edge {
        ResizeEdge::North => (false, false, true, false),
        ResizeEdge::South => (false, false, false, true),
        ResizeEdge::East => (false, true, false, false),
        ResizeEdge::West => (true, false, false, false),
        ResizeEdge::NorthEast => (false, true, true, false),
        ResizeEdge::NorthWest => (true, false, true, false),
        ResizeEdge::SouthEast => (false, true, false, true),
        ResizeEdge::SouthWest => (true, false, false, true),
    }
}

/// Compute new geometry after a resize drag (no screen clamping).
#[cfg(test)]
pub(crate) fn compute_resize(
    start: Geometry,
    edge: ResizeEdge,
    dx: i32,
    dy: i32,
    theme: &WmTheme,
) -> (i32, i32, u32, u32) {
    let g = to_geometry(resize_wide(
        start,
        edge,
        i64::from(dx),
        i64::from(dy),
        theme,
    ));
    (g.x, g.y, g.w, g.h)
}

/// A frame in `i64` (`x, y, w, h`), so resize maths cannot overflow.
type WideRect = (i64, i64, i64, i64);

/// [`compute_resize`] in `i64`: the dragged edges follow the pointer,
/// down to the minimum size.
fn resize_wide(start: Geometry, edge: ResizeEdge, dx: i64, dy: i64, theme: &WmTheme) -> WideRect {
    // A window that is already smaller than the minimum (created that way,
    // or sized by `resize_window`) keeps its size as the floor: grabbing a
    // handle must not make it jump to the minimum before the pointer moves.
    let border2 = u64::from(theme.border_width) * 2;
    let min_w = (u64::from(MIN_RESIZE_W) + border2).min(u64::from(start.w)) as i64;
    let min_h = (u64::from(MIN_RESIZE_H) + u64::from(theme.titlebar_height) + border2)
        .min(u64::from(start.h)) as i64;
    let (left, right, top, bottom) = edge_sides(edge);
    let (x0, y0) = (i64::from(start.x), i64::from(start.y));
    let (w0, h0) = (i64::from(start.w), i64::from(start.h));

    let (mut x, mut w) = (x0, w0);
    if right {
        w = (w0 + dx).max(min_w);
    } else if left {
        w = (w0 - dx).max(min_w);
        x = x0 + w0 - w;
    }
    let (mut y, mut h) = (y0, h0);
    if bottom {
        h = (h0 + dy).max(min_h);
    } else if top {
        h = (h0 - dy).max(min_h);
        y = y0 + h0 - h;
    }
    (x, y, w, h)
}

/// The geometry a resize drag of `edge` by `delta` gives, with the
/// dragged edges kept from growing past the `screen`.
///
/// Only the edges being dragged are clamped, and never further in than
/// where they started: a window that is already partly off-screen
/// (dragged there by its titlebar) must not jump or shrink below its
/// minimum size when the user grabs the opposite, on-screen edge.
fn resize_on_screen(
    start: Geometry,
    edge: ResizeEdge,
    delta: (i64, i64),
    screen: (u32, u32),
    theme: &WmTheme,
) -> Geometry {
    let (mut x, mut y, mut w, mut h) = resize_wide(start, edge, delta.0, delta.1, theme);
    let (sw, sh) = (i64::from(screen.0), i64::from(screen.1));
    let (sx, sy) = (i64::from(start.x), i64::from(start.y));
    let (left, right, top, bottom) = edge_sides(edge);
    let left_limit = sx.min(0);
    let top_limit = sy.min(0);
    let right_limit = (sx + i64::from(start.w)).max(sw);
    let bottom_limit = (sy + i64::from(start.h)).max(sh);
    if left && x < left_limit {
        w = (w - (left_limit - x)).max(0);
        x = left_limit;
    }
    if top && y < top_limit {
        h = (h - (top_limit - y)).max(0);
        y = top_limit;
    }
    if right && x + w > right_limit {
        w = (right_limit - x).max(0);
    }
    if bottom && y + h > bottom_limit {
        h = (bottom_limit - y).max(0);
    }
    to_geometry((x, y, w, h))
}

/// `r` saturated into a [`Geometry`].
fn to_geometry(r: WideRect) -> Geometry {
    let pos = |v: i64| v.clamp(i32::MIN.into(), i32::MAX.into()) as i32;
    let size = |v: i64| v.clamp(0, u32::MAX.into()) as u32;
    Geometry {
        x: pos(r.0),
        y: pos(r.1),
        w: size(r.2),
        h: size(r.3),
    }
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
    // A frame narrower than MIN_VISIBLE only has to keep its own width on
    // screen (otherwise it could never touch the left edge), and a screen
    // narrower than that keeps the frame's left edge on screen.
    let visible = MIN_VISIBLE.min(outer_w.min(i32::MAX as u32) as i32).min(sw);
    let min_x = visible - outer_w.min(i32::MAX as u32) as i32;
    let max_x = (sw - visible).max(min_x);
    let cx = x.clamp(min_x, max_x);
    let cy = y.min(sh - tb_h).max(0);
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
    // In i64: `pos + size` must not overflow for far-away frames.
    let snap = |pos: i32, size: u32, screen: u32| -> i32 {
        let far = i64::from(screen) - i64::from(size);
        if i64::from(pos).abs() < i64::from(SNAP_DISTANCE) {
            0
        } else if (i64::from(pos) - far).abs() < i64::from(SNAP_DISTANCE) {
            // Within SNAP_DISTANCE of an i32, so in range.
            far as i32
        } else {
            pos
        }
    };
    (snap(x, outer_w, screen_w), snap(y, outer_h, screen_h))
}

#[cfg(test)]
mod tests;
