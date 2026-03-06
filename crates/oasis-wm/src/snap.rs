//! Aero Snap-style window snap zones for the window manager.
//!
//! When a window is dragged to a screen edge or corner, the snap manager
//! detects the target zone and provides a preview rectangle. Releasing the
//! drag applies the snap, resizing the window to fill the zone. Keyboard
//! shortcuts (Meta+Arrow) are also supported via [`keyboard_snap`].

use crate::window::Geometry;

/// Screen zone where a window can snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    /// Cursor is not near any snap zone.
    None,
    /// Left half of the screen.
    Left,
    /// Right half of the screen.
    Right,
    /// Full screen (maximize).
    Top,
    /// Top-left quarter of the screen.
    TopLeft,
    /// Top-right quarter of the screen.
    TopRight,
    /// Bottom-left quarter of the screen.
    BottomLeft,
    /// Bottom-right quarter of the screen.
    BottomRight,
}

/// Visual preview rectangle for an active snap zone.
///
/// Skins render this as a translucent overlay to show where the window
/// will land when the user releases the drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapPreview {
    /// The snap zone this preview represents.
    pub zone: SnapZone,
    /// X position of the preview rectangle.
    pub x: i32,
    /// Y position of the preview rectangle.
    pub y: i32,
    /// Width of the preview rectangle.
    pub width: u32,
    /// Height of the preview rectangle.
    pub height: u32,
}

/// Keyboard snap direction for Meta+Arrow shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardSnapDirection {
    /// Snap to left half (Meta+Left).
    Left,
    /// Snap to right half (Meta+Right).
    Right,
    /// Maximize (Meta+Up).
    Up,
    /// Restore or minimize (Meta+Down).
    Down,
}

/// Manages snap zone detection, preview state, and geometry computation.
///
/// During a window drag, call [`update_preview`](SnapManager::update_preview)
/// each frame with the cursor position. When the drag ends, call
/// [`apply_snap`](SnapManager::apply_snap) to get the target geometry and
/// clear the preview.
/// Default pixels from screen edge that trigger an edge snap zone.
const DEFAULT_EDGE_THRESHOLD: u32 = 16;
/// Default size of corner snap trigger zones (measured from each corner).
const DEFAULT_CORNER_SIZE: u32 = 64;

pub struct SnapManager {
    /// The currently active snap preview, if any.
    pub active_preview: Option<SnapPreview>,
    /// Pixels from screen edge that trigger an edge snap zone.
    edge_threshold: u32,
    /// Size of corner snap trigger zones (measured from each corner).
    corner_size: u32,
}

impl SnapManager {
    /// Create a new snap manager with default thresholds.
    ///
    /// Default edge threshold is 16px, corner size is 64px.
    pub fn new() -> Self {
        Self {
            active_preview: None,
            edge_threshold: DEFAULT_EDGE_THRESHOLD,
            corner_size: DEFAULT_CORNER_SIZE,
        }
    }

    /// Create a new snap manager with custom thresholds.
    pub fn with_threshold(edge_threshold: u32, corner_size: u32) -> Self {
        Self {
            active_preview: None,
            edge_threshold,
            corner_size,
        }
    }

    /// Return the current edge threshold in pixels.
    pub fn edge_threshold(&self) -> u32 {
        self.edge_threshold
    }

    /// Return the current corner zone size in pixels.
    pub fn corner_size(&self) -> u32 {
        self.corner_size
    }

    /// Detect which snap zone the cursor is in.
    ///
    /// Corners are checked first (they overlap edges). If the cursor is
    /// not near any edge, returns [`SnapZone::None`].
    pub fn detect_zone(
        &self,
        cursor_x: i32,
        cursor_y: i32,
        screen_w: u32,
        screen_h: u32,
    ) -> SnapZone {
        let sw = screen_w as i32;
        let sh = screen_h as i32;
        let edge = self.edge_threshold as i32;
        let corner = self.corner_size as i32;

        let near_left = cursor_x < edge;
        let near_right = cursor_x >= sw - edge;
        let near_top = cursor_y < edge;
        let near_bottom = cursor_y >= sh - edge;

        let in_top_band = cursor_y < corner;
        let in_bottom_band = cursor_y >= sh - corner;
        let in_left_band = cursor_x < corner;
        let in_right_band = cursor_x >= sw - corner;

        // Corner zones: cursor is near an edge AND within a corner band
        // on the perpendicular axis.
        if near_left && in_top_band {
            return SnapZone::TopLeft;
        }
        if near_right && in_top_band {
            return SnapZone::TopRight;
        }
        if near_left && in_bottom_band {
            return SnapZone::BottomLeft;
        }
        if near_right && in_bottom_band {
            return SnapZone::BottomRight;
        }

        // Also detect corners when cursor is near top/bottom AND within
        // a corner band on the horizontal axis.
        if near_top && in_left_band {
            return SnapZone::TopLeft;
        }
        if near_top && in_right_band {
            return SnapZone::TopRight;
        }
        if near_bottom && in_left_band {
            return SnapZone::BottomLeft;
        }
        if near_bottom && in_right_band {
            return SnapZone::BottomRight;
        }

        // Edge zones (after corners so corners take priority).
        if near_left {
            return SnapZone::Left;
        }
        if near_right {
            return SnapZone::Right;
        }
        if near_top {
            return SnapZone::Top;
        }

        // Near-bottom without a corner does not have a dedicated zone
        // (no "bottom half" snap in Aero Snap).
        SnapZone::None
    }

    /// Update the active snap preview based on cursor position.
    ///
    /// If the cursor is in a snap zone, the preview is set to the
    /// corresponding geometry. If not, the preview is cleared.
    pub fn update_preview(&mut self, cursor_x: i32, cursor_y: i32, screen_w: u32, screen_h: u32) {
        let zone = self.detect_zone(cursor_x, cursor_y, screen_w, screen_h);
        match Self::snap_geometry(&zone, screen_w, screen_h) {
            Some(geo) => {
                self.active_preview = Some(SnapPreview {
                    zone,
                    x: geo.x,
                    y: geo.y,
                    width: geo.w,
                    height: geo.h,
                });
            },
            Option::None => {
                self.active_preview = None;
            },
        }
    }

    /// Clear the snap preview without applying any snap.
    pub fn clear_preview(&mut self) {
        self.active_preview = None;
    }

    /// Compute the target geometry for a snap zone.
    ///
    /// Returns `None` for [`SnapZone::None`] since there is no target
    /// geometry when the cursor is not in a snap zone.
    pub fn snap_geometry(zone: &SnapZone, screen_w: u32, screen_h: u32) -> Option<Geometry> {
        let sw = screen_w;
        let sh = screen_h;
        let half_w = sw / 2;
        let half_h = sh / 2;

        match zone {
            SnapZone::None => None,
            SnapZone::Left => Some(Geometry {
                x: 0,
                y: 0,
                w: half_w,
                h: sh,
            }),
            SnapZone::Right => Some(Geometry {
                x: half_w as i32,
                y: 0,
                w: sw - half_w,
                h: sh,
            }),
            SnapZone::Top => Some(Geometry {
                x: 0,
                y: 0,
                w: sw,
                h: sh,
            }),
            SnapZone::TopLeft => Some(Geometry {
                x: 0,
                y: 0,
                w: half_w,
                h: half_h,
            }),
            SnapZone::TopRight => Some(Geometry {
                x: half_w as i32,
                y: 0,
                w: sw - half_w,
                h: half_h,
            }),
            SnapZone::BottomLeft => Some(Geometry {
                x: 0,
                y: half_h as i32,
                w: half_w,
                h: sh - half_h,
            }),
            SnapZone::BottomRight => Some(Geometry {
                x: half_w as i32,
                y: half_h as i32,
                w: sw - half_w,
                h: sh - half_h,
            }),
        }
    }

    /// Apply a snap: compute the target geometry and clear the preview.
    ///
    /// Returns `None` for [`SnapZone::None`].
    pub fn apply_snap(
        &mut self,
        zone: &SnapZone,
        screen_w: u32,
        screen_h: u32,
    ) -> Option<Geometry> {
        self.clear_preview();
        Self::snap_geometry(zone, screen_w, screen_h)
    }
}

impl Default for SnapManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the snap zone and geometry for a keyboard shortcut.
///
/// Returns `None` for [`KeyboardSnapDirection::Down`] since that
/// restores/minimizes rather than snapping to a zone.
pub fn keyboard_snap(
    direction: KeyboardSnapDirection,
    screen_w: u32,
    screen_h: u32,
) -> Option<(SnapZone, Geometry)> {
    let zone = match direction {
        KeyboardSnapDirection::Left => SnapZone::Left,
        KeyboardSnapDirection::Right => SnapZone::Right,
        KeyboardSnapDirection::Up => SnapZone::Top,
        KeyboardSnapDirection::Down => return None,
    };
    SnapManager::snap_geometry(&zone, screen_w, screen_h).map(|geo| (zone, geo))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN_W: u32 = 800;
    const SCREEN_H: u32 = 600;

    fn default_manager() -> SnapManager {
        SnapManager::new()
    }

    // ---- detect_zone: all 8 zones + center ----

    #[test]
    fn detect_zone_none_center() {
        let mgr = default_manager();
        let zone = mgr.detect_zone(400, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::None);
    }

    #[test]
    fn detect_zone_left_edge() {
        let mgr = default_manager();
        // cursor_x = 5 (< 16 threshold), cursor_y = 300 (middle)
        let zone = mgr.detect_zone(5, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Left);
    }

    #[test]
    fn detect_zone_right_edge() {
        let mgr = default_manager();
        // cursor_x = 795 (>= 800-16=784), cursor_y = 300 (middle)
        let zone = mgr.detect_zone(795, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Right);
    }

    #[test]
    fn detect_zone_top_edge() {
        let mgr = default_manager();
        // cursor_y = 5 (< 16), cursor_x = 400 (middle, not in corner band)
        let zone = mgr.detect_zone(400, 5, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Top);
    }

    #[test]
    fn detect_zone_top_left_corner_via_left_edge() {
        let mgr = default_manager();
        // near left edge (x=5 < 16) and in top corner band (y=30 < 64)
        let zone = mgr.detect_zone(5, 30, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::TopLeft);
    }

    #[test]
    fn detect_zone_top_left_corner_via_top_edge() {
        let mgr = default_manager();
        // near top edge (y=5 < 16) and in left corner band (x=30 < 64)
        let zone = mgr.detect_zone(30, 5, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::TopLeft);
    }

    #[test]
    fn detect_zone_top_right_corner() {
        let mgr = default_manager();
        // near right edge (x=795 >= 784) and in top band (y=10 < 64)
        let zone = mgr.detect_zone(795, 10, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::TopRight);
    }

    #[test]
    fn detect_zone_bottom_left_corner() {
        let mgr = default_manager();
        // near left edge (x=5 < 16) and in bottom band (y=580 >= 536)
        let zone = mgr.detect_zone(5, 580, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::BottomLeft);
    }

    #[test]
    fn detect_zone_bottom_right_corner() {
        let mgr = default_manager();
        // near right edge (x=795 >= 784) and in bottom band (y=590 >= 536)
        let zone = mgr.detect_zone(795, 590, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::BottomRight);
    }

    #[test]
    fn detect_zone_bottom_edge_no_zone() {
        let mgr = default_manager();
        // near bottom (y=595 >= 584) but not in a corner band (x=400)
        // Bottom edge alone has no snap zone.
        let zone = mgr.detect_zone(400, 595, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::None);
    }

    // ---- snap_geometry for all zones ----

    #[test]
    fn geometry_none_returns_none() {
        assert!(SnapManager::snap_geometry(&SnapZone::None, SCREEN_W, SCREEN_H).is_none());
    }

    #[test]
    fn geometry_left_half() {
        let geo = SnapManager::snap_geometry(&SnapZone::Left, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 0);
        assert_eq!(geo.y, 0);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 600);
    }

    #[test]
    fn geometry_right_half() {
        let geo = SnapManager::snap_geometry(&SnapZone::Right, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 400);
        assert_eq!(geo.y, 0);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 600);
    }

    #[test]
    fn geometry_top_maximize() {
        let geo = SnapManager::snap_geometry(&SnapZone::Top, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 0);
        assert_eq!(geo.y, 0);
        assert_eq!(geo.w, 800);
        assert_eq!(geo.h, 600);
    }

    #[test]
    fn geometry_top_left_quarter() {
        let geo = SnapManager::snap_geometry(&SnapZone::TopLeft, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 0);
        assert_eq!(geo.y, 0);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 300);
    }

    #[test]
    fn geometry_top_right_quarter() {
        let geo = SnapManager::snap_geometry(&SnapZone::TopRight, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 400);
        assert_eq!(geo.y, 0);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 300);
    }

    #[test]
    fn geometry_bottom_left_quarter() {
        let geo = SnapManager::snap_geometry(&SnapZone::BottomLeft, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 0);
        assert_eq!(geo.y, 300);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 300);
    }

    #[test]
    fn geometry_bottom_right_quarter() {
        let geo = SnapManager::snap_geometry(&SnapZone::BottomRight, SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(geo.x, 400);
        assert_eq!(geo.y, 300);
        assert_eq!(geo.w, 400);
        assert_eq!(geo.h, 300);
    }

    #[test]
    fn geometry_halves_cover_full_screen() {
        let left = SnapManager::snap_geometry(&SnapZone::Left, SCREEN_W, SCREEN_H).unwrap();
        let right = SnapManager::snap_geometry(&SnapZone::Right, SCREEN_W, SCREEN_H).unwrap();
        // Left and right halves tile without gap or overlap.
        assert_eq!(left.x + left.w as i32, right.x);
        assert_eq!(left.w + right.w, SCREEN_W);
    }

    #[test]
    fn geometry_quarters_cover_full_screen() {
        let tl = SnapManager::snap_geometry(&SnapZone::TopLeft, SCREEN_W, SCREEN_H).unwrap();
        let tr = SnapManager::snap_geometry(&SnapZone::TopRight, SCREEN_W, SCREEN_H).unwrap();
        let bl = SnapManager::snap_geometry(&SnapZone::BottomLeft, SCREEN_W, SCREEN_H).unwrap();
        let br = SnapManager::snap_geometry(&SnapZone::BottomRight, SCREEN_W, SCREEN_H).unwrap();
        // All four quarters tile the screen exactly.
        assert_eq!(tl.w + tr.w, SCREEN_W);
        assert_eq!(bl.w + br.w, SCREEN_W);
        assert_eq!(tl.h + bl.h, SCREEN_H);
        assert_eq!(tr.h + br.h, SCREEN_H);
    }

    // ---- Preview lifecycle ----

    #[test]
    fn preview_starts_none() {
        let mgr = default_manager();
        assert!(mgr.active_preview.is_none());
    }

    #[test]
    fn update_preview_sets_preview_in_zone() {
        let mut mgr = default_manager();
        mgr.update_preview(5, 300, SCREEN_W, SCREEN_H);
        let preview = mgr.active_preview.as_ref().unwrap();
        assert_eq!(preview.zone, SnapZone::Left);
        assert_eq!(preview.x, 0);
        assert_eq!(preview.y, 0);
        assert_eq!(preview.width, 400);
        assert_eq!(preview.height, 600);
    }

    #[test]
    fn update_preview_clears_when_leaving_zone() {
        let mut mgr = default_manager();
        mgr.update_preview(5, 300, SCREEN_W, SCREEN_H);
        assert!(mgr.active_preview.is_some());
        mgr.update_preview(400, 300, SCREEN_W, SCREEN_H);
        assert!(mgr.active_preview.is_none());
    }

    #[test]
    fn clear_preview_removes_active() {
        let mut mgr = default_manager();
        mgr.update_preview(5, 300, SCREEN_W, SCREEN_H);
        assert!(mgr.active_preview.is_some());
        mgr.clear_preview();
        assert!(mgr.active_preview.is_none());
    }

    #[test]
    fn apply_snap_returns_geometry_and_clears() {
        let mut mgr = default_manager();
        mgr.update_preview(5, 300, SCREEN_W, SCREEN_H);
        let geo = mgr.apply_snap(&SnapZone::Left, SCREEN_W, SCREEN_H);
        assert!(geo.is_some());
        assert!(mgr.active_preview.is_none());
    }

    #[test]
    fn apply_snap_none_returns_none() {
        let mut mgr = default_manager();
        let geo = mgr.apply_snap(&SnapZone::None, SCREEN_W, SCREEN_H);
        assert!(geo.is_none());
    }

    // ---- keyboard_snap ----

    #[test]
    fn keyboard_snap_left() {
        let result = keyboard_snap(KeyboardSnapDirection::Left, SCREEN_W, SCREEN_H);
        let (zone, geo) = result.unwrap();
        assert_eq!(zone, SnapZone::Left);
        assert_eq!(geo.w, 400);
    }

    #[test]
    fn keyboard_snap_right() {
        let result = keyboard_snap(KeyboardSnapDirection::Right, SCREEN_W, SCREEN_H);
        let (zone, geo) = result.unwrap();
        assert_eq!(zone, SnapZone::Right);
        assert_eq!(geo.x, 400);
    }

    #[test]
    fn keyboard_snap_up_maximizes() {
        let result = keyboard_snap(KeyboardSnapDirection::Up, SCREEN_W, SCREEN_H);
        let (zone, geo) = result.unwrap();
        assert_eq!(zone, SnapZone::Top);
        assert_eq!(geo.w, SCREEN_W);
        assert_eq!(geo.h, SCREEN_H);
    }

    #[test]
    fn keyboard_snap_down_returns_none() {
        let result = keyboard_snap(KeyboardSnapDirection::Down, SCREEN_W, SCREEN_H);
        assert!(result.is_none());
    }

    // ---- Edge cases ----

    #[test]
    fn detect_zone_exact_edge_zero() {
        let mgr = default_manager();
        // Cursor at (0, 0) -- top-left corner.
        let zone = mgr.detect_zone(0, 0, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::TopLeft);
    }

    #[test]
    fn detect_zone_exact_bottom_right_corner() {
        let mgr = default_manager();
        // Cursor at the very last pixel.
        let zone = mgr.detect_zone(SCREEN_W as i32 - 1, SCREEN_H as i32 - 1, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::BottomRight);
    }

    #[test]
    fn detect_zone_just_outside_edge_threshold() {
        let mgr = default_manager();
        // x = 16 is exactly at the threshold boundary (not less than 16).
        let zone = mgr.detect_zone(16, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::None);
    }

    #[test]
    fn detect_zone_just_inside_edge_threshold() {
        let mgr = default_manager();
        // x = 15 is within the threshold (< 16).
        let zone = mgr.detect_zone(15, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Left);
    }

    #[test]
    fn custom_threshold_smaller() {
        let mgr = SnapManager::with_threshold(4, 32);
        // x = 5 would be in zone with default threshold (16) but not with 4.
        let zone = mgr.detect_zone(5, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::None);
        // x = 3 is within the 4px threshold.
        let zone = mgr.detect_zone(3, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Left);
    }

    #[test]
    fn custom_threshold_larger() {
        let mgr = SnapManager::with_threshold(50, 100);
        // x = 30 is within the 50px threshold.
        let zone = mgr.detect_zone(30, 300, SCREEN_W, SCREEN_H);
        assert_eq!(zone, SnapZone::Left);
    }

    #[test]
    fn odd_screen_dimensions() {
        // 801x601: halves should still tile without gap.
        let left = SnapManager::snap_geometry(&SnapZone::Left, 801, 601).unwrap();
        let right = SnapManager::snap_geometry(&SnapZone::Right, 801, 601).unwrap();
        assert_eq!(left.w + right.w, 801);
        assert_eq!(left.x + left.w as i32, right.x);
    }

    #[test]
    fn psp_native_resolution() {
        // 480x272 (PSP native): verify snap zones work at small resolution.
        let mgr = default_manager();
        let zone = mgr.detect_zone(0, 136, 480, 272);
        assert_eq!(zone, SnapZone::Left);

        let geo = SnapManager::snap_geometry(&SnapZone::Left, 480, 272).unwrap();
        assert_eq!(geo.w, 240);
        assert_eq!(geo.h, 272);
    }

    #[test]
    fn default_impl_matches_new() {
        let a = SnapManager::new();
        let b = SnapManager::default();
        assert_eq!(a.edge_threshold, b.edge_threshold);
        assert_eq!(a.corner_size, b.corner_size);
        assert!(a.active_preview.is_none());
        assert!(b.active_preview.is_none());
    }

    #[test]
    fn preview_transitions_between_zones() {
        let mut mgr = default_manager();
        // Start in Left zone.
        mgr.update_preview(5, 300, SCREEN_W, SCREEN_H);
        assert_eq!(mgr.active_preview.as_ref().unwrap().zone, SnapZone::Left,);
        // Move to Right zone.
        mgr.update_preview(795, 300, SCREEN_W, SCREEN_H);
        assert_eq!(mgr.active_preview.as_ref().unwrap().zone, SnapZone::Right,);
        // Move to Top zone.
        mgr.update_preview(400, 5, SCREEN_W, SCREEN_H);
        assert_eq!(mgr.active_preview.as_ref().unwrap().zone, SnapZone::Top,);
    }
}
