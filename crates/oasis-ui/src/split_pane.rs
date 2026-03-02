//! Split pane widget: resizable two-pane container with draggable divider.

use crate::context::DrawContext;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Orientation of the split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOrientation {
    /// Side-by-side panes (divider is vertical, panes are left/right).
    Horizontal,
    /// Stacked panes (divider is horizontal, panes are top/bottom).
    Vertical,
}

/// Identifies one of the two panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    /// The first (left or top) pane.
    First,
    /// The second (right or bottom) pane.
    Second,
}

/// A two-pane container with a draggable divider bar.
///
/// The split pane divides its area into two regions separated by a
/// divider bar. The caller is responsible for filling the pane areas;
/// the widget only draws the divider.
pub struct SplitPane {
    /// Split direction.
    pub orientation: SplitOrientation,
    /// Position of the divider as a fraction of the total size (0.0 to 1.0).
    ratio: f32,
    /// Minimum allowed ratio.
    min_ratio: f32,
    /// Maximum allowed ratio.
    max_ratio: f32,
    /// Divider bar width in pixels.
    pub divider_width: u16,
    /// Whether the divider is currently being dragged.
    dragging: bool,
    /// If a pane is collapsed, which one.
    collapsed_pane: Option<PaneId>,
    /// Whether the divider is hovered (for highlight).
    pub hovered: bool,
}

impl SplitPane {
    /// Create a new split pane with the given orientation.
    ///
    /// Defaults to a 50/50 split with a 4px divider bar.
    pub fn new(orientation: SplitOrientation) -> Self {
        Self {
            orientation,
            ratio: 0.5,
            min_ratio: 0.0,
            max_ratio: 1.0,
            divider_width: 4,
            dragging: false,
            collapsed_pane: None,
            hovered: false,
        }
    }

    /// Set the divider ratio (clamped to min/max bounds).
    pub fn with_ratio(mut self, r: f32) -> Self {
        self.ratio = r.clamp(self.min_ratio, self.max_ratio);
        self
    }

    /// Set the minimum and maximum allowed ratios.
    pub fn with_min_max(mut self, min: f32, max: f32) -> Self {
        self.min_ratio = min.clamp(0.0, 1.0);
        self.max_ratio = max.clamp(self.min_ratio, 1.0);
        // Re-clamp ratio to new bounds.
        self.ratio = self.ratio.clamp(self.min_ratio, self.max_ratio);
        self
    }

    /// Return the current divider ratio.
    pub fn ratio(&self) -> f32 {
        self.ratio
    }

    /// Set the divider ratio (clamped to min/max bounds).
    pub fn set_ratio(&mut self, r: f32) {
        self.ratio = r.clamp(self.min_ratio, self.max_ratio);
    }

    /// Compute the rectangle of the first (left or top) pane.
    ///
    /// Returns `(x, y, w, h)` within the given container bounds.
    pub fn first_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        if self.collapsed_pane == Some(PaneId::First) {
            return (x, y, 0, 0);
        }
        let dw = self.divider_width as u32;
        match self.orientation {
            SplitOrientation::Horizontal => {
                let first_w = self.first_size(w, dw);
                (x, y, first_w, h)
            },
            SplitOrientation::Vertical => {
                let first_h = self.first_size(h, dw);
                (x, y, w, first_h)
            },
        }
    }

    /// Compute the rectangle of the second (right or bottom) pane.
    ///
    /// Returns `(x, y, w, h)` within the given container bounds.
    pub fn second_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        if self.collapsed_pane == Some(PaneId::Second) {
            return (x, y, 0, 0);
        }
        let dw = self.divider_width as u32;
        match self.orientation {
            SplitOrientation::Horizontal => {
                let first_w = if self.collapsed_pane == Some(PaneId::First) {
                    0
                } else {
                    self.first_size(w, dw)
                };
                let sx = x + first_w as i32 + dw as i32;
                let sw = w.saturating_sub(first_w + dw);
                (sx, y, sw, h)
            },
            SplitOrientation::Vertical => {
                let first_h = if self.collapsed_pane == Some(PaneId::First) {
                    0
                } else {
                    self.first_size(h, dw)
                };
                let sy = y + first_h as i32 + dw as i32;
                let sh = h.saturating_sub(first_h + dw);
                (x, sy, w, sh)
            },
        }
    }

    /// Compute the rectangle of the divider bar.
    ///
    /// Returns `(x, y, w, h)` within the given container bounds.
    pub fn divider_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        let dw = self.divider_width as u32;
        match self.orientation {
            SplitOrientation::Horizontal => {
                let first_w = if self.collapsed_pane == Some(PaneId::First) {
                    0
                } else if self.collapsed_pane == Some(PaneId::Second) {
                    w.saturating_sub(dw)
                } else {
                    self.first_size(w, dw)
                };
                (x + first_w as i32, y, dw, h)
            },
            SplitOrientation::Vertical => {
                let first_h = if self.collapsed_pane == Some(PaneId::First) {
                    0
                } else if self.collapsed_pane == Some(PaneId::Second) {
                    h.saturating_sub(dw)
                } else {
                    self.first_size(h, dw)
                };
                (x, y + first_h as i32, w, dw)
            },
        }
    }

    /// Begin dragging the divider.
    pub fn start_drag(&mut self) {
        self.dragging = true;
    }

    /// Stop dragging the divider.
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// Update the divider position during a drag.
    ///
    /// `mouse_pos` is the mouse coordinate along the split axis (x for
    /// horizontal, y for vertical) relative to the container origin.
    /// `total_size` is the container size along the split axis.
    pub fn update_drag(&mut self, mouse_pos: i32, total_size: u32) {
        if !self.dragging || total_size == 0 {
            return;
        }
        let new_ratio = mouse_pos as f32 / total_size as f32;
        self.ratio = new_ratio.clamp(self.min_ratio, self.max_ratio);
    }

    /// Collapse a pane, hiding it entirely.
    pub fn collapse(&mut self, pane: PaneId) {
        self.collapsed_pane = Some(pane);
    }

    /// Expand a previously collapsed pane, restoring the divider position.
    pub fn expand(&mut self) {
        self.collapsed_pane = None;
    }

    /// Return which pane is collapsed, if any.
    pub fn is_collapsed(&self) -> Option<PaneId> {
        self.collapsed_pane
    }

    /// Compute the first-pane pixel size along the split axis.
    fn first_size(&self, total: u32, divider: u32) -> u32 {
        let available = total.saturating_sub(divider);
        (available as f32 * self.ratio) as u32
    }
}

impl Widget for SplitPane {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        (available_w, available_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (dx, dy, dw, dh) = self.divider_rect(x, y, w, h);
        let color = if self.hovered || self.dragging {
            ctx.theme.border_strong
        } else {
            ctx.theme.border
        };
        ctx.backend.fill_rect(dx, dy, dw, dh, color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DrawContext;
    use crate::test_utils::{MockBackend, test_draw_all_themes};
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn new_horizontal_defaults() {
        let sp = SplitPane::new(SplitOrientation::Horizontal);
        assert_eq!(sp.orientation, SplitOrientation::Horizontal);
        assert!((sp.ratio() - 0.5).abs() < f32::EPSILON);
        assert_eq!(sp.divider_width, 4);
        assert!(!sp.dragging);
        assert_eq!(sp.is_collapsed(), None);
    }

    #[test]
    fn new_vertical_defaults() {
        let sp = SplitPane::new(SplitOrientation::Vertical);
        assert_eq!(sp.orientation, SplitOrientation::Vertical);
        assert!((sp.ratio() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn with_ratio_sets_value() {
        let sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.3);
        assert!((sp.ratio() - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn ratio_clamped_to_min_max() {
        let sp = SplitPane::new(SplitOrientation::Horizontal)
            .with_min_max(0.2, 0.8)
            .with_ratio(0.1);
        assert!((sp.ratio() - 0.2).abs() < f32::EPSILON);

        let sp2 = SplitPane::new(SplitOrientation::Horizontal)
            .with_min_max(0.2, 0.8)
            .with_ratio(0.95);
        assert!((sp2.ratio() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn set_ratio_clamps() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_min_max(0.1, 0.9);
        sp.set_ratio(-0.5);
        assert!((sp.ratio() - 0.1).abs() < f32::EPSILON);
        sp.set_ratio(1.5);
        assert!((sp.ratio() - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn first_rect_horizontal() {
        let sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        // Container: 200x100 at (0,0). Divider = 4px.
        // Available for panes = 196, first = 98.
        let (fx, fy, fw, fh) = sp.first_rect(0, 0, 200, 100);
        assert_eq!(fx, 0);
        assert_eq!(fy, 0);
        assert_eq!(fw, 98); // (200 - 4) * 0.5 = 98
        assert_eq!(fh, 100);
    }

    #[test]
    fn second_rect_horizontal() {
        let sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        let (sx, sy, sw, sh) = sp.second_rect(0, 0, 200, 100);
        // first = 98, divider = 4, second starts at 102.
        assert_eq!(sx, 102);
        assert_eq!(sy, 0);
        assert_eq!(sw, 98); // 200 - 98 - 4 = 98
        assert_eq!(sh, 100);
    }

    #[test]
    fn first_rect_vertical() {
        let sp = SplitPane::new(SplitOrientation::Vertical).with_ratio(0.5);
        let (fx, fy, fw, fh) = sp.first_rect(0, 0, 200, 100);
        assert_eq!(fx, 0);
        assert_eq!(fy, 0);
        assert_eq!(fw, 200);
        assert_eq!(fh, 48); // (100 - 4) * 0.5 = 48
    }

    #[test]
    fn second_rect_vertical() {
        let sp = SplitPane::new(SplitOrientation::Vertical).with_ratio(0.5);
        let (sx, sy, sw, sh) = sp.second_rect(0, 0, 200, 100);
        assert_eq!(sx, 0);
        assert_eq!(sy, 52); // 48 + 4 = 52
        assert_eq!(sw, 200);
        assert_eq!(sh, 48); // 100 - 48 - 4 = 48
    }

    #[test]
    fn divider_rect_horizontal() {
        let sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        let (dx, dy, dw, dh) = sp.divider_rect(0, 0, 200, 100);
        assert_eq!(dx, 98); // first pane width
        assert_eq!(dy, 0);
        assert_eq!(dw, 4);
        assert_eq!(dh, 100);
    }

    #[test]
    fn divider_rect_vertical() {
        let sp = SplitPane::new(SplitOrientation::Vertical).with_ratio(0.5);
        let (dx, dy, dw, dh) = sp.divider_rect(0, 0, 200, 100);
        assert_eq!(dx, 0);
        assert_eq!(dy, 48);
        assert_eq!(dw, 200);
        assert_eq!(dh, 4);
    }

    #[test]
    fn min_max_enforcement() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_min_max(0.25, 0.75);
        sp.set_ratio(0.0);
        assert!((sp.ratio() - 0.25).abs() < f32::EPSILON);
        sp.set_ratio(1.0);
        assert!((sp.ratio() - 0.75).abs() < f32::EPSILON);
        sp.set_ratio(0.5);
        assert!((sp.ratio() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn collapse_first_pane() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        sp.collapse(PaneId::First);
        assert_eq!(sp.is_collapsed(), Some(PaneId::First));

        let (_, _, fw, fh) = sp.first_rect(0, 0, 200, 100);
        assert_eq!(fw, 0);
        assert_eq!(fh, 0);

        // Second pane should take the remaining space after divider.
        let (sx, _, sw, _) = sp.second_rect(0, 0, 200, 100);
        assert_eq!(sx, 4); // divider width
        assert_eq!(sw, 196); // 200 - 0 - 4
    }

    #[test]
    fn collapse_second_pane() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        sp.collapse(PaneId::Second);
        assert_eq!(sp.is_collapsed(), Some(PaneId::Second));

        let (_, _, sw, sh) = sp.second_rect(0, 0, 200, 100);
        assert_eq!(sw, 0);
        assert_eq!(sh, 0);
    }

    #[test]
    fn expand_restores_state() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.3);
        sp.collapse(PaneId::First);
        assert_eq!(sp.is_collapsed(), Some(PaneId::First));
        sp.expand();
        assert_eq!(sp.is_collapsed(), None);
        // Ratio should be preserved.
        assert!((sp.ratio() - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn drag_update() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_min_max(0.1, 0.9);
        sp.start_drag();
        sp.update_drag(150, 300);
        assert!((sp.ratio() - 0.5).abs() < f32::EPSILON);

        sp.update_drag(270, 300);
        assert!((sp.ratio() - 0.9).abs() < f32::EPSILON);

        // Below minimum.
        sp.update_drag(10, 300);
        assert!((sp.ratio() - 0.1).abs() < 0.05);

        sp.end_drag();
        // After ending, update_drag should be a no-op.
        sp.update_drag(150, 300);
        assert!(sp.ratio() < 0.15);
    }

    #[test]
    fn drag_zero_total_size_noop() {
        let mut sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        sp.start_drag();
        sp.update_drag(100, 0);
        assert!((sp.ratio() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn measure_returns_available_size() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let sp = SplitPane::new(SplitOrientation::Horizontal);
        let (w, h) = sp.measure(&ctx, 400, 300);
        assert_eq!(w, 400);
        assert_eq!(h, 300);
    }

    #[test]
    fn draw_emits_fill_rect() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let sp = SplitPane::new(SplitOrientation::Horizontal);
            sp.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_hovered_uses_border_strong() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut sp = SplitPane::new(SplitOrientation::Horizontal);
            sp.hovered = true;
            sp.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Verify the fill_rect used border_strong color.
        let fill = backend.calls.iter().find_map(|c| {
            if let crate::test_utils::DrawCall::FillRect { color, .. } = c {
                Some(*color)
            } else {
                None
            }
        });
        assert_eq!(fill, Some(theme.border_strong));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        test_draw_all_themes(|ctx| {
            let sp = SplitPane::new(SplitOrientation::Horizontal);
            sp.draw(ctx, 0, 0, 200, 100).unwrap();
        });
        test_draw_all_themes(|ctx| {
            let sp = SplitPane::new(SplitOrientation::Vertical);
            sp.draw(ctx, 0, 0, 200, 100).unwrap();
        });
    }

    #[test]
    fn orientation_debug_and_eq() {
        assert_eq!(SplitOrientation::Horizontal, SplitOrientation::Horizontal);
        assert_ne!(SplitOrientation::Horizontal, SplitOrientation::Vertical);
        let _ = format!("{:?}", SplitOrientation::Vertical);
    }

    #[test]
    fn pane_id_debug_and_eq() {
        assert_eq!(PaneId::First, PaneId::First);
        assert_ne!(PaneId::First, PaneId::Second);
        let _ = format!("{:?}", PaneId::Second);
    }

    #[test]
    fn rects_with_offset_origin() {
        let sp = SplitPane::new(SplitOrientation::Horizontal).with_ratio(0.5);
        let (fx, fy, fw, fh) = sp.first_rect(10, 20, 200, 100);
        assert_eq!(fx, 10);
        assert_eq!(fy, 20);
        assert_eq!(fw, 98);
        assert_eq!(fh, 100);

        let (sx, sy, sw, sh) = sp.second_rect(10, 20, 200, 100);
        assert_eq!(sx, 112); // 10 + 98 + 4
        assert_eq!(sy, 20);
        assert_eq!(sw, 98);
        assert_eq!(sh, 100);
    }
}
