//! Snapping, keyboard window management and tiling for the window manager.
//!
//! Wires the standalone [`SnapManager`](crate::snap::SnapManager),
//! [`keyboard_snap`](crate::snap::keyboard_snap) directions and
//! [`TilingManager`](crate::tiling::TilingManager) into live windows:
//!
//! - Dragging a window's titlebar to a screen edge shows a
//!   [`SnapPreview`] (see [`WindowManager::snap_preview`]); releasing the
//!   drag snaps the window (left/right half, quarters in the corners, top
//!   edge = maximize).
//! - [`WindowManager::keyboard_snap_window`] implements Super+Arrow style
//!   snapping / restoring.
//! - [`WindowManager::cycle_tiling`] steps through the tiling layouts and
//!   back to free-floating windows.
//!
//! All geometry is computed inside the *work area*: the screen minus the
//! theme's `maximize_top_inset` / `maximize_bottom_inset` (status bar and
//! taskbar), so snapped windows line up with maximized ones.

use oasis_sdi::SdiRegistry;

use super::manager::{WindowManager, WmEvent};
use super::snap::{KeyboardSnapDirection, SnapPreview, SnapZone};
use super::tiling::{TilingLayout, cycle_layout};
use super::window::{Geometry, Window, WindowState, WindowType};

/// Current geometry of a window as a [`Geometry`].
fn geometry_of(w: &Window) -> Geometry {
    Geometry {
        x: w.x,
        y: w.y,
        w: w.outer_w,
        h: w.outer_h,
    }
}

/// Whether a window can be snapped or tiled at all.
fn snappable(w: &Window) -> bool {
    w.is_resizable() && w.state != WindowState::Minimized
}

impl WindowManager {
    // -- Snapping ---------------------------------------------------------

    /// Enable or disable drag-to-edge snapping (enabled by default).
    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.snap_enabled = enabled;
        if !enabled {
            self.snap.clear_preview();
        }
    }

    /// Whether drag-to-edge snapping is enabled.
    pub fn snap_enabled(&self) -> bool {
        self.snap_enabled
    }

    /// The snap preview to render while a window is being dragged into a
    /// snap zone (skins draw it as a translucent rectangle).
    pub fn snap_preview(&self) -> Option<SnapPreview> {
        self.snap.active_preview
    }

    /// The area windows snap, maximize and tile into: the screen minus the
    /// theme's top/bottom maximize insets.
    pub fn work_area(&self) -> Geometry {
        let top = self.theme.maximize_top_inset;
        let bottom = self.theme.maximize_bottom_inset;
        Geometry {
            x: 0,
            y: top as i32,
            w: self.screen_w,
            h: self.screen_h.saturating_sub(top + bottom),
        }
    }

    /// Target geometry (in screen coordinates) for a snap zone.
    pub fn snap_zone_geometry(&self, zone: SnapZone) -> Option<Geometry> {
        let area = self.work_area();
        let g = super::snap::SnapManager::snap_geometry(&zone, area.w, area.h)?;
        Some(Geometry {
            x: area.x + g.x,
            y: area.y + g.y,
            w: g.w,
            h: g.h,
        })
    }

    /// Update the drag snap preview for a cursor position while `id` is
    /// being moved.
    pub(crate) fn update_drag_snap_preview(&mut self, id: &str, x: i32, y: i32) {
        let can_snap = self.snap_enabled && self.windows.iter().any(|w| w.id == id && snappable(w));
        if !can_snap {
            self.snap.clear_preview();
            return;
        }
        let zone = self.snap.detect_zone(x, y, self.screen_w, self.screen_h);
        self.snap.active_preview = self.snap_zone_geometry(zone).map(|g| SnapPreview {
            zone,
            x: g.x,
            y: g.y,
            width: g.w,
            height: g.h,
        });
    }

    /// Apply (and clear) the pending drag snap preview, if any.
    pub(crate) fn apply_drag_snap(&mut self, id: &str, sdi: &mut SdiRegistry) -> Option<WmEvent> {
        let preview = self.snap.active_preview.take()?;
        match self.snap_window(id, preview.zone, sdi) {
            WmEvent::None => None,
            ev => Some(ev),
        }
    }

    /// Snap a window to `zone`. [`SnapZone::Top`] maximizes; the other
    /// zones resize the window to a half / quarter of the work area and
    /// remember its previous geometry for [`Self::unsnap_window`].
    ///
    /// Returns [`WmEvent::None`] for windows that cannot be snapped
    /// (dialogs, widgets, panels, kiosk or minimized windows).
    pub fn snap_window(&mut self, id: &str, zone: SnapZone, sdi: &mut SdiRegistry) -> WmEvent {
        let Some(target) = self.snap_zone_geometry(zone) else {
            return WmEvent::None;
        };
        let Some(window) = self.windows.iter_mut().find(|w| w.id == id) else {
            return WmEvent::None;
        };
        if !snappable(window) {
            return WmEvent::None;
        }
        let wid = window.id.clone();

        // The geometry to eventually return to: the pre-snap geometry if
        // already snapped, the pre-maximize geometry if maximized, else the
        // current free-floating geometry.
        let original = if let Some(g) = window.pre_snap_geometry.take() {
            g
        } else if window.state == WindowState::Maximized {
            window
                .saved_geometry
                .take()
                .unwrap_or_else(|| geometry_of(window))
        } else {
            geometry_of(window)
        };
        window.snap_zone = None;

        if zone == SnapZone::Top {
            window.state = WindowState::Normal;
            if let Err(e) = self.maximize_window(id, sdi) {
                log::debug!("snap maximize({id}): {e}");
                return WmEvent::None;
            }
            if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                w.saved_geometry = Some(original);
            }
            self.focus_window_internal(id, sdi);
            return WmEvent::WindowMaximized(wid);
        }

        window.state = WindowState::Normal;
        window.saved_geometry = None;
        window.x = target.x;
        window.y = target.y;
        window.outer_w = target.w;
        window.outer_h = target.h;
        window.snap_zone = Some(zone);
        window.pre_snap_geometry = Some(original);
        self.update_sdi_positions(id, sdi);
        self.focus_window_internal(id, sdi);
        WmEvent::WindowResized(wid)
    }

    /// Return a snapped window to the geometry it had before snapping.
    /// Returns [`WmEvent::None`] if the window is not snapped.
    pub fn unsnap_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> WmEvent {
        let Some(window) = self.windows.iter_mut().find(|w| w.id == id) else {
            return WmEvent::None;
        };
        let Some(g) = window.pre_snap_geometry.take() else {
            return WmEvent::None;
        };
        window.snap_zone = None;
        window.x = g.x;
        window.y = g.y;
        window.outer_w = g.w;
        window.outer_h = g.h;
        let wid = window.id.clone();
        self.update_sdi_positions(id, sdi);
        WmEvent::WindowRestored(wid)
    }

    /// Keyboard window management (Super+Arrow / Ctrl+Alt+Arrow):
    ///
    /// - Left / Right: snap to that half. Pressing the opposite direction
    ///   of the current half unsnaps back to the original geometry.
    /// - Up: maximize.
    /// - Down: restore a maximized or snapped window; minimize a normal one.
    pub fn keyboard_snap_window(
        &mut self,
        id: &str,
        direction: KeyboardSnapDirection,
        sdi: &mut SdiRegistry,
    ) -> WmEvent {
        let Some(window) = self.windows.iter().find(|w| w.id == id) else {
            return WmEvent::None;
        };
        if !snappable(window) {
            return WmEvent::None;
        }
        let state = window.state;
        let snapped = window.snap_zone;
        let can_minimize = window.has_minimize_button();

        match direction {
            KeyboardSnapDirection::Left | KeyboardSnapDirection::Right => {
                let (zone, opposite) = if direction == KeyboardSnapDirection::Left {
                    (SnapZone::Left, SnapZone::Right)
                } else {
                    (SnapZone::Right, SnapZone::Left)
                };
                match snapped {
                    Some(z) if z == zone => WmEvent::None,
                    Some(z) if z == opposite => self.unsnap_window(id, sdi),
                    _ => self.snap_window(id, zone, sdi),
                }
            },
            KeyboardSnapDirection::Up => {
                if state == WindowState::Maximized {
                    WmEvent::None
                } else {
                    self.snap_window(id, SnapZone::Top, sdi)
                }
            },
            KeyboardSnapDirection::Down => {
                if state == WindowState::Maximized {
                    match self.restore_window(id, sdi) {
                        Ok(()) => WmEvent::WindowRestored(id.into()),
                        Err(_) => WmEvent::None,
                    }
                } else if snapped.is_some() {
                    self.unsnap_window(id, sdi)
                } else if can_minimize {
                    match self.minimize_window(id, sdi) {
                        Ok(()) => WmEvent::WindowMinimized(id.into()),
                        Err(_) => WmEvent::None,
                    }
                } else {
                    WmEvent::None
                }
            },
        }
    }

    // -- Tiling -----------------------------------------------------------

    /// The active tiling layout, or `None` when windows float freely.
    pub fn tiling_layout(&self) -> Option<TilingLayout> {
        self.tiling_layout
    }

    /// Step through the tiling layouts: floating -> master/stack -> grid ->
    /// columns -> rows -> monocle -> floating. Each step re-arranges the
    /// visible app windows (the active window becomes the master); the
    /// final step restores every window's pre-tiling geometry.
    ///
    /// Returns the new layout (`None` = back to floating).
    pub fn cycle_tiling(&mut self, sdi: &mut SdiRegistry) -> Option<TilingLayout> {
        let next = match self.tiling_layout {
            None => Some(TilingLayout::MasterStack),
            Some(cur) => {
                let n = cycle_layout(cur);
                (n != TilingLayout::MasterStack).then_some(n)
            },
        };
        match next {
            Some(layout) => self.apply_tiling(layout, sdi),
            None => self.stop_tiling(sdi),
        }
        self.tiling_layout = next;
        next
    }

    /// Arrange visible app windows with `layout`.
    fn apply_tiling(&mut self, layout: TilingLayout, sdi: &mut SdiRegistry) {
        self.tiling.set_layout(layout);
        // Topmost (active) window first so it becomes the master tile.
        let ids: Vec<_> = self
            .windows
            .iter()
            .rev()
            .filter(|w| snappable(w) && w.window_type == WindowType::AppWindow)
            .map(|w| w.id.clone())
            .collect();
        let id_strs: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
        let area = self.work_area();
        let tiles = self.tiling.compute_layout(&id_strs, area.w, area.h);
        for tile in tiles {
            let Some(w) = self.windows.iter_mut().find(|w| w.id == tile.window_id) else {
                continue;
            };
            if w.pre_tile_geometry.is_none() {
                w.pre_tile_geometry = Some(if w.state == WindowState::Maximized {
                    w.saved_geometry.take().unwrap_or_else(|| geometry_of(w))
                } else {
                    w.pre_snap_geometry.take().unwrap_or_else(|| geometry_of(w))
                });
            }
            w.state = WindowState::Normal;
            w.snap_zone = None;
            w.pre_snap_geometry = None;
            // With more windows than fit, the tiler's minimum tile size
            // pushes the last tiles past the work area (even fully off
            // screen, out of the user's reach). Pull such tiles back inside;
            // they overlap their neighbours, which beats being unreachable.
            let g = tile.geometry;
            let max_x = area.x + area.w.saturating_sub(g.w) as i32;
            let max_y = area.y + area.h.saturating_sub(g.h) as i32;
            w.x = (area.x + g.x).min(max_x).max(area.x);
            w.y = (area.y + g.y).min(max_y).max(area.y);
            w.outer_w = g.w.min(area.w);
            w.outer_h = g.h.min(area.h);
            self.update_sdi_positions(&tile.window_id, sdi);
        }
        // Keep the active window on top (monocle stacks all tiles).
        if let Some(active) = self.active_window.clone() {
            self.focus_window_internal(&active, sdi);
        }
    }

    /// Restore every tiled window's pre-tiling geometry.
    fn stop_tiling(&mut self, sdi: &mut SdiRegistry) {
        let restored: Vec<_> = self
            .windows
            .iter_mut()
            .filter_map(|w| {
                let g = w.pre_tile_geometry.take()?;
                if w.state == WindowState::Normal {
                    w.x = g.x;
                    w.y = g.y;
                    w.outer_w = g.w;
                    w.outer_h = g.h;
                } else {
                    // Minimized / maximized since tiling: restore later.
                    w.saved_geometry = Some(g);
                }
                Some(w.id.clone())
            })
            .collect();
        for id in restored {
            self.update_sdi_positions(&id, sdi);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::manager::{WindowManager, WmEvent};
    use crate::snap::{KeyboardSnapDirection, SnapZone};
    use crate::tiling::TilingLayout;
    use crate::window::{WindowConfig, WindowState, WindowType, WmTheme};
    use oasis_sdi::SdiRegistry;
    use oasis_types::input::InputEvent;

    const SW: u32 = 800;
    const SH: u32 = 600;

    fn cfg(id: &str, kind: WindowType) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(200),
            y: Some(150),
            width: 240,
            height: 160,
            window_type: kind,
            always_on_top: false,
            modal: false,
        }
    }

    fn setup(ids: &[&str]) -> (WindowManager, SdiRegistry) {
        let mut wm = WindowManager::new(SW, SH);
        let mut sdi = SdiRegistry::new();
        for id in ids {
            wm.create_window(&cfg(id, WindowType::AppWindow), &mut sdi)
                .expect("create");
        }
        (wm, sdi)
    }

    fn geom(wm: &WindowManager, id: &str) -> (i32, i32, u32, u32) {
        let w = wm.get_window(id).expect("window");
        (w.x, w.y, w.outer_w, w.outer_h)
    }

    /// Press on the titlebar, move to `(x, y)`, release there.
    fn drag_to(wm: &mut WindowManager, sdi: &mut SdiRegistry, id: &str, x: i32, y: i32) -> WmEvent {
        let (tx, ty, _, th) = wm
            .get_window(id)
            .and_then(|w| w.titlebar_rect(wm.theme()))
            .expect("titlebar");
        let (px, py) = (tx + 30, ty + th as i32 / 2);
        wm.handle_input(&InputEvent::PointerClick { x: px, y: py }, sdi);
        wm.handle_input(&InputEvent::CursorMove { x, y }, sdi);
        wm.handle_input(&InputEvent::PointerRelease { x, y }, sdi)
    }

    #[test]
    fn drag_release_at_left_edge_snaps_left_half() {
        let (mut wm, mut sdi) = setup(&["a"]);
        let before = geom(&wm, "a");
        let ev = drag_to(&mut wm, &mut sdi, "a", 5, 300);
        assert_eq!(ev, WmEvent::WindowResized("a".into()));
        assert_eq!(geom(&wm, "a"), (0, 0, SW / 2, SH));
        let w = wm.get_window("a").expect("a");
        assert_eq!(w.snap_zone, Some(SnapZone::Left));
        assert_eq!(sdi.get("a.frame").expect("frame").w, SW / 2);
        assert!(wm.snap_preview().is_none(), "preview cleared on release");
        // Unsnap returns to the pre-drag size.
        wm.unsnap_window("a", &mut sdi);
        let after = geom(&wm, "a");
        assert_eq!((after.2, after.3), (before.2, before.3));
    }

    #[test]
    fn drag_shows_preview_before_release() {
        let (mut wm, mut sdi) = setup(&["a"]);
        let (tx, ty, _, _) = wm
            .get_window("a")
            .and_then(|w| w.titlebar_rect(wm.theme()))
            .expect("titlebar");
        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 30,
                y: ty + 5,
            },
            &mut sdi,
        );
        wm.handle_input(&InputEvent::CursorMove { x: 795, y: 300 }, &mut sdi);
        let p = wm.snap_preview().expect("preview");
        assert_eq!(p.zone, SnapZone::Right);
        assert_eq!((p.x, p.width), (400, 400));
        // Moving back to the middle clears it; releasing does not snap.
        wm.handle_input(&InputEvent::CursorMove { x: 400, y: 300 }, &mut sdi);
        assert!(wm.snap_preview().is_none());
        let ev = wm.handle_input(&InputEvent::PointerRelease { x: 400, y: 300 }, &mut sdi);
        assert!(matches!(ev, WmEvent::WindowMoved(_)));
        assert!(wm.get_window("a").expect("a").snap_zone.is_none());
    }

    #[test]
    fn dragging_snapped_window_restores_its_size() {
        let (mut wm, mut sdi) = setup(&["a"]);
        let before = geom(&wm, "a");
        drag_to(&mut wm, &mut sdi, "a", 3, 300);
        assert_eq!(geom(&wm, "a").2, SW / 2);
        // Drag it back out to the middle of the screen.
        drag_to(&mut wm, &mut sdi, "a", 400, 300);
        let after = geom(&wm, "a");
        assert_eq!((after.2, after.3), (before.2, before.3));
        let w = wm.get_window("a").expect("a");
        assert!(w.snap_zone.is_none() && w.pre_snap_geometry.is_none());
        // The grab point stays under the cursor.
        assert!(after.0 <= 400 && 400 < after.0 + after.2 as i32);
    }

    #[test]
    fn drag_release_at_right_edge_snaps_right_half() {
        let (mut wm, mut sdi) = setup(&["a"]);
        drag_to(&mut wm, &mut sdi, "a", SW as i32 - 3, 300);
        assert_eq!(geom(&wm, "a"), (SW as i32 / 2, 0, SW / 2, SH));
    }

    #[test]
    fn drag_release_at_top_maximizes_and_restores_original() {
        let (mut wm, mut sdi) = setup(&["a"]);
        let before = geom(&wm, "a");
        let ev = drag_to(&mut wm, &mut sdi, "a", 400, 2);
        assert_eq!(ev, WmEvent::WindowMaximized("a".into()));
        assert_eq!(wm.get_window("a").expect("a").state, WindowState::Maximized);
        wm.restore_window("a", &mut sdi).expect("restore");
        let after = geom(&wm, "a");
        assert_eq!((after.2, after.3), (before.2, before.3));
    }

    #[test]
    fn snap_respects_work_area_insets() {
        let theme = WmTheme {
            maximize_top_inset: 20,
            maximize_bottom_inset: 30,
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(SW, SH, theme);
        let mut sdi = SdiRegistry::new();
        wm.create_window(&cfg("a", WindowType::AppWindow), &mut sdi)
            .expect("create");
        drag_to(&mut wm, &mut sdi, "a", 3, 300);
        assert_eq!(geom(&wm, "a"), (0, 20, SW / 2, SH - 50));
    }

    #[test]
    fn dialogs_do_not_snap() {
        let mut wm = WindowManager::new(SW, SH);
        let mut sdi = SdiRegistry::new();
        wm.create_window(&cfg("d", WindowType::FloatingWidget), &mut sdi)
            .expect("create");
        let before = geom(&wm, "d");
        drag_to(&mut wm, &mut sdi, "d", 3, 300);
        assert!(wm.get_window("d").expect("d").snap_zone.is_none());
        assert_eq!(geom(&wm, "d").2, before.2);
    }

    #[test]
    fn snap_disabled_means_no_preview() {
        let (mut wm, mut sdi) = setup(&["a"]);
        wm.set_snap_enabled(false);
        drag_to(&mut wm, &mut sdi, "a", 3, 300);
        assert!(wm.get_window("a").expect("a").snap_zone.is_none());
    }

    #[test]
    fn keyboard_snap_left_right_up_down() {
        let (mut wm, mut sdi) = setup(&["a"]);
        let before = geom(&wm, "a");

        wm.keyboard_snap_window("a", KeyboardSnapDirection::Left, &mut sdi);
        assert_eq!(geom(&wm, "a"), (0, 0, SW / 2, SH));
        // Opposite direction unsnaps to the original geometry.
        wm.keyboard_snap_window("a", KeyboardSnapDirection::Right, &mut sdi);
        assert_eq!(geom(&wm, "a"), before);

        wm.keyboard_snap_window("a", KeyboardSnapDirection::Right, &mut sdi);
        assert_eq!(geom(&wm, "a"), (SW as i32 / 2, 0, SW / 2, SH));
        // Up maximizes; Down restores to the pre-snap geometry.
        wm.keyboard_snap_window("a", KeyboardSnapDirection::Up, &mut sdi);
        assert_eq!(wm.get_window("a").expect("a").state, WindowState::Maximized);
        wm.keyboard_snap_window("a", KeyboardSnapDirection::Down, &mut sdi);
        assert_eq!(geom(&wm, "a"), before);
        // Down on a normal window minimizes it.
        let ev = wm.keyboard_snap_window("a", KeyboardSnapDirection::Down, &mut sdi);
        assert_eq!(ev, WmEvent::WindowMinimized("a".into()));
    }

    #[test]
    fn cycle_focus_skips_minimized() {
        let (mut wm, mut sdi) = setup(&["a", "b", "c", "d"]);
        wm.minimize_window("b", &mut sdi).expect("minimize");
        let mut seen = Vec::new();
        for _ in 0..6 {
            let id = wm.cycle_focus(true, &mut sdi).expect("focus");
            assert_ne!(id.as_str(), "b");
            seen.push(id.to_string());
        }
        // Forward cycling visits every visible window in rotation.
        assert_eq!(seen[..3], ["a", "c", "d"]);
        // Backward cycling also skips the minimized window.
        for _ in 0..6 {
            let id = wm.cycle_focus(false, &mut sdi).expect("focus");
            assert_ne!(id.as_str(), "b");
        }
        assert_eq!(wm.get_window("b").expect("b").state, WindowState::Minimized);
    }

    #[test]
    fn cycle_tiling_steps_layouts_and_restores() {
        let (mut wm, mut sdi) = setup(&["a", "b"]);
        let before_a = geom(&wm, "a");
        let before_b = geom(&wm, "b");

        assert_eq!(wm.cycle_tiling(&mut sdi), Some(TilingLayout::MasterStack));
        // Active window (b, created last) is the master on the left.
        let (bx, _, bw, _) = geom(&wm, "b");
        let (ax, ..) = geom(&wm, "a");
        assert!(bx < ax && bw > 0);
        for expected in [
            TilingLayout::Grid,
            TilingLayout::Columns,
            TilingLayout::Rows,
            TilingLayout::Monocle,
        ] {
            assert_eq!(wm.cycle_tiling(&mut sdi), Some(expected));
        }
        assert_eq!(wm.cycle_tiling(&mut sdi), None);
        assert_eq!(geom(&wm, "a"), before_a);
        assert_eq!(geom(&wm, "b"), before_b);
    }

    #[test]
    fn tiling_skips_minimized_windows() {
        let (mut wm, mut sdi) = setup(&["a", "b"]);
        wm.minimize_window("a", &mut sdi).expect("minimize");
        let before_a = geom(&wm, "a");
        wm.cycle_tiling(&mut sdi);
        assert_eq!(geom(&wm, "a"), before_a);
        // b alone fills the usable area.
        assert!(geom(&wm, "b").2 > SW / 2);
    }
}
