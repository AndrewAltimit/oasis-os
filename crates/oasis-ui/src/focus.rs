//! Keyboard focus navigation for widget groups.
//!
//! `FocusRing` tracks which widget index has focus and handles
//! directional navigation (next/prev/wrapping). Widgets query the
//! ring to determine their visual focus state.
//!
//! `FocusManager` builds on `FocusRing` to provide Tab/Shift-Tab
//! keyboard cycling, skip-disabled item logic, and visual focus
//! indicator drawing through `FocusStyle`.

/// Direction of focus movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    /// Move focus to the next widget.
    Next,
    /// Move focus to the previous widget.
    Prev,
}

/// Tracks keyboard focus across a group of widgets.
#[derive(Debug, Clone)]
pub struct FocusRing {
    /// Total number of focusable items.
    count: usize,
    /// Index of the currently focused item.
    focused: usize,
    /// Whether wrap-around is enabled.
    pub wrap: bool,
}

impl FocusRing {
    /// Create a new focus ring with the given number of items.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            focused: 0,
            wrap: true,
        }
    }

    /// Get the current focused index.
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Return whether the given index has focus.
    pub fn is_focused(&self, index: usize) -> bool {
        self.focused == index
    }

    /// Set focus to a specific index (clamped to bounds).
    pub fn set_focused(&mut self, index: usize) {
        if self.count > 0 {
            self.focused = index.min(self.count - 1);
        }
    }

    /// Update the total number of focusable items.
    ///
    /// If the current focus index exceeds the new count, it is clamped.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        if count > 0 {
            self.focused = self.focused.min(count - 1);
        } else {
            self.focused = 0;
        }
    }

    /// Total number of focusable items.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Move focus in the given direction.
    ///
    /// Returns the new focused index.
    pub fn move_focus(&mut self, dir: FocusDir) -> usize {
        if self.count == 0 {
            return 0;
        }
        match dir {
            FocusDir::Next => {
                if self.focused + 1 < self.count {
                    self.focused += 1;
                } else if self.wrap {
                    self.focused = 0;
                }
            },
            FocusDir::Prev => {
                if self.focused > 0 {
                    self.focused -= 1;
                } else if self.wrap {
                    self.focused = self.count - 1;
                }
            },
        }
        self.focused
    }

    /// Move focus to the first item.
    pub fn focus_first(&mut self) {
        self.focused = 0;
    }

    /// Move focus to the last item.
    pub fn focus_last(&mut self) {
        if self.count > 0 {
            self.focused = self.count - 1;
        }
    }
}

use oasis_types::backend::Color;

/// Visual style for focus indicators drawn around focused widgets.
#[derive(Debug, Clone, Copy)]
pub struct FocusStyle {
    /// Border color for the focus indicator.
    pub color: Color,
    /// Border width in pixels.
    pub width: u16,
    /// Corner radius for the focus border.
    pub radius: u16,
    /// Gap between the widget edge and the focus border.
    pub offset: i32,
}

impl FocusStyle {
    /// Create a focus style from a theme's accent color.
    pub fn from_accent(accent: Color) -> Self {
        Self {
            color: accent,
            width: 1,
            radius: 2,
            offset: 1,
        }
    }

    /// Compute the focus indicator rectangle given a widget rect.
    pub fn indicator_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        let off = self.offset;
        let delta = off.saturating_mul(2);
        (
            x.saturating_sub(off),
            y.saturating_sub(off),
            w.saturating_add_signed(delta),
            h.saturating_add_signed(delta),
        )
    }

    /// Draw the focus indicator around a widget rectangle.
    pub fn draw(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        let (fx, fy, fw, fh) = self.indicator_rect(x, y, w, h);
        backend.stroke_rounded_rect(fx, fy, fw, fh, self.radius, self.width, self.color)
    }
}

/// Keyboard navigation action derived from input events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAction {
    /// Tab key: move to the next focusable widget.
    TabNext,
    /// Shift+Tab: move to the previous focusable widget.
    TabPrev,
    /// Activate/confirm the focused widget (Enter/Cross).
    Activate,
    /// Home key: focus the first widget.
    Home,
    /// End key: focus the last widget.
    End,
}

/// Manages keyboard focus across a set of widgets, with support
/// for Tab/Shift-Tab cycling, disabled-item skipping, and visual
/// focus indicators.
#[derive(Debug, Clone)]
pub struct FocusManager {
    /// The underlying focus ring.
    ring: FocusRing,
    /// Per-item enabled state. Items marked `false` are skipped
    /// during Tab navigation.
    enabled: Vec<bool>,
    /// Whether keyboard focus is currently active. When false,
    /// no widget shows a focus indicator.
    pub active: bool,
}

impl FocusManager {
    /// Create a new focus manager with `count` focusable items,
    /// all initially enabled.
    pub fn new(count: usize) -> Self {
        Self {
            ring: FocusRing::new(count),
            enabled: vec![true; count],
            active: false,
        }
    }

    /// Get the current focused index.
    pub fn focused(&self) -> usize {
        self.ring.focused()
    }

    /// Whether the given index currently has visible focus.
    pub fn has_focus(&self, index: usize) -> bool {
        self.active && self.ring.is_focused(index)
    }

    /// Total number of managed items.
    pub fn count(&self) -> usize {
        self.ring.count()
    }

    /// Access the underlying `FocusRing`.
    pub fn ring(&self) -> &FocusRing {
        &self.ring
    }

    /// Mutably access the underlying `FocusRing`.
    pub fn ring_mut(&mut self) -> &mut FocusRing {
        &mut self.ring
    }

    /// Set whether a specific item is enabled for focus.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) {
        if index < self.enabled.len() {
            self.enabled[index] = enabled;
        }
    }

    /// Whether a specific item is enabled.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Update the item count. Grows or shrinks the enabled list.
    pub fn set_count(&mut self, count: usize) {
        self.ring.set_count(count);
        self.enabled.resize(count, true);
    }

    /// Process a focus action and return the new focused index.
    ///
    /// Skips disabled items when navigating. If all items are
    /// disabled, focus does not move.
    pub fn handle_action(&mut self, action: FocusAction) -> usize {
        if self.ring.count() == 0 {
            return 0;
        }
        self.active = true;
        match action {
            FocusAction::TabNext => self.move_skip(FocusDir::Next),
            FocusAction::TabPrev => self.move_skip(FocusDir::Prev),
            FocusAction::Activate => self.ring.focused(),
            FocusAction::Home => {
                self.ring.focus_first();
                self.skip_to_enabled(FocusDir::Next)
            },
            FocusAction::End => {
                self.ring.focus_last();
                self.skip_to_enabled(FocusDir::Prev)
            },
        }
    }

    /// Move focus in the given direction, skipping disabled items.
    fn move_skip(&mut self, dir: FocusDir) -> usize {
        let start = self.ring.focused();
        self.ring.move_focus(dir);
        // Skip disabled items, but stop if we wrap all the way
        // around to avoid infinite loops.
        let mut attempts = 0;
        while !self.is_enabled(self.ring.focused()) && attempts < self.ring.count() {
            self.ring.move_focus(dir);
            attempts += 1;
        }
        // If everything is disabled, go back to start.
        if attempts >= self.ring.count() {
            self.ring.set_focused(start);
        }
        self.ring.focused()
    }

    /// From the current position, skip to the nearest enabled
    /// item in the given direction.
    fn skip_to_enabled(&mut self, dir: FocusDir) -> usize {
        if self.is_enabled(self.ring.focused()) {
            return self.ring.focused();
        }
        let start = self.ring.focused();
        let mut attempts = 0;
        while !self.is_enabled(self.ring.focused()) && attempts < self.ring.count() {
            self.ring.move_focus(dir);
            attempts += 1;
        }
        if attempts >= self.ring.count() {
            self.ring.set_focused(start);
        }
        self.ring.focused()
    }

    /// Set focus to a specific index (must be enabled).
    pub fn set_focused(&mut self, index: usize) {
        if index < self.enabled.len() && self.enabled[index] {
            self.ring.set_focused(index);
            self.active = true;
        }
    }

    /// Deactivate keyboard focus (no visible indicator).
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    /// Draw the focus indicator around a widget if it has focus.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_indicator(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        index: usize,
        style: &FocusStyle,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        if self.has_focus(index) {
            style.draw(backend, x, y, w, h)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let f = FocusRing::new(5);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 5);
        assert!(f.wrap);
    }

    #[test]
    fn new_empty() {
        let f = FocusRing::new(0);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 0);
    }

    #[test]
    fn is_focused() {
        let f = FocusRing::new(3);
        assert!(f.is_focused(0));
        assert!(!f.is_focused(1));
        assert!(!f.is_focused(2));
    }

    #[test]
    fn set_focused_clamps() {
        let mut f = FocusRing::new(3);
        f.set_focused(10);
        assert_eq!(f.focused(), 2); // clamped to count-1
    }

    #[test]
    fn set_focused_valid() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        assert_eq!(f.focused(), 2);
    }

    #[test]
    fn set_focused_empty_noop() {
        let mut f = FocusRing::new(0);
        f.set_focused(5);
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn move_next_wraps() {
        let mut f = FocusRing::new(3);
        assert_eq!(f.move_focus(FocusDir::Next), 1);
        assert_eq!(f.move_focus(FocusDir::Next), 2);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps
    }

    #[test]
    fn move_prev_wraps() {
        let mut f = FocusRing::new(3);
        assert_eq!(f.move_focus(FocusDir::Prev), 2); // wraps from 0
        assert_eq!(f.move_focus(FocusDir::Prev), 1);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn move_next_no_wrap() {
        let mut f = FocusRing::new(3);
        f.wrap = false;
        f.set_focused(2);
        assert_eq!(f.move_focus(FocusDir::Next), 2); // stays at end
    }

    #[test]
    fn move_prev_no_wrap() {
        let mut f = FocusRing::new(3);
        f.wrap = false;
        assert_eq!(f.move_focus(FocusDir::Prev), 0); // stays at start
    }

    #[test]
    fn move_focus_empty_returns_zero() {
        let mut f = FocusRing::new(0);
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn move_focus_single_item() {
        let mut f = FocusRing::new(1);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps to same
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn focus_first_and_last() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.focus_first();
        assert_eq!(f.focused(), 0);
        f.focus_last();
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn focus_last_empty() {
        let mut f = FocusRing::new(0);
        f.focus_last();
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn set_count_clamps_focused() {
        let mut f = FocusRing::new(5);
        f.set_focused(4);
        f.set_count(3);
        assert_eq!(f.focused(), 2); // clamped
        assert_eq!(f.count(), 3);
    }

    #[test]
    fn set_count_to_zero() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.set_count(0);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 0);
    }

    #[test]
    fn set_count_grow_preserves_focus() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        f.set_count(10);
        assert_eq!(f.focused(), 2); // unchanged
    }

    #[test]
    fn focus_dir_debug() {
        assert_eq!(format!("{:?}", FocusDir::Next), "Next");
        assert_eq!(format!("{:?}", FocusDir::Prev), "Prev");
    }

    #[test]
    fn focus_ring_clone() {
        let f = FocusRing::new(3);
        let f2 = f.clone();
        assert_eq!(f.focused(), f2.focused());
        assert_eq!(f.count(), f2.count());
    }

    #[test]
    fn sequential_navigation() {
        let mut f = FocusRing::new(4);
        // Forward through all items
        for expected in 1..=3 {
            assert_eq!(f.move_focus(FocusDir::Next), expected);
        }
        // Backward through all items
        for expected in (0..=2).rev() {
            assert_eq!(f.move_focus(FocusDir::Prev), expected);
        }
    }

    // -- Additional focus traversal tests --

    #[test]
    fn wrap_around_full_cycle_next() {
        let mut f = FocusRing::new(3);
        // 0 -> 1 -> 2 -> 0 -> 1 -> 2 -> 0
        let expected = [1, 2, 0, 1, 2, 0];
        for &e in &expected {
            assert_eq!(f.move_focus(FocusDir::Next), e);
        }
    }

    #[test]
    fn wrap_around_full_cycle_prev() {
        let mut f = FocusRing::new(3);
        // 0 -> 2 -> 1 -> 0 -> 2 -> 1 -> 0
        let expected = [2, 1, 0, 2, 1, 0];
        for &e in &expected {
            assert_eq!(f.move_focus(FocusDir::Prev), e);
        }
    }

    #[test]
    fn no_wrap_next_boundary_stays() {
        let mut f = FocusRing::new(5);
        f.wrap = false;
        f.set_focused(4);
        // At the end, Next should not move.
        assert_eq!(f.move_focus(FocusDir::Next), 4);
        assert_eq!(f.move_focus(FocusDir::Next), 4);
    }

    #[test]
    fn no_wrap_prev_boundary_stays() {
        let mut f = FocusRing::new(5);
        f.wrap = false;
        f.set_focused(0);
        // At the beginning, Prev should not move.
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn set_count_increase() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        f.set_count(10);
        assert_eq!(f.count(), 10);
        assert_eq!(f.focused(), 2); // unchanged
    }

    #[test]
    fn set_count_decrease_clamps_focus() {
        let mut f = FocusRing::new(10);
        f.set_focused(9);
        f.set_count(5);
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn set_count_to_one() {
        let mut f = FocusRing::new(10);
        f.set_focused(5);
        f.set_count(1);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 1);
    }

    #[test]
    fn focus_first_from_middle() {
        let mut f = FocusRing::new(10);
        f.set_focused(5);
        f.focus_first();
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn focus_last_from_first() {
        let mut f = FocusRing::new(10);
        f.focus_last();
        assert_eq!(f.focused(), 9);
    }

    #[test]
    fn is_focused_after_movement() {
        let mut f = FocusRing::new(5);
        assert!(f.is_focused(0));
        f.move_focus(FocusDir::Next);
        assert!(!f.is_focused(0));
        assert!(f.is_focused(1));
    }

    #[test]
    fn set_focused_to_zero() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.set_focused(0);
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn single_item_wrap_next_prev() {
        let mut f = FocusRing::new(1);
        f.wrap = true;
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn single_item_no_wrap_next_prev() {
        let mut f = FocusRing::new(1);
        f.wrap = false;
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn two_items_alternating() {
        let mut f = FocusRing::new(2);
        assert_eq!(f.move_focus(FocusDir::Next), 1);
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 1);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn focus_dir_equality() {
        assert_eq!(FocusDir::Next, FocusDir::Next);
        assert_eq!(FocusDir::Prev, FocusDir::Prev);
        assert_ne!(FocusDir::Next, FocusDir::Prev);
    }

    #[test]
    fn large_focus_ring() {
        let mut f = FocusRing::new(1000);
        f.set_focused(999);
        assert_eq!(f.focused(), 999);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps
    }

    #[test]
    fn set_focused_exact_max() {
        let mut f = FocusRing::new(5);
        f.set_focused(4); // max valid
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn set_focused_way_over_max() {
        let mut f = FocusRing::new(5);
        f.set_focused(1000);
        assert_eq!(f.focused(), 4);
    }

    // -- FocusStyle tests --

    #[test]
    fn focus_style_from_accent() {
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        assert_eq!(s.color, Color::rgb(80, 160, 255));
        assert_eq!(s.width, 1);
        assert_eq!(s.radius, 2);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn focus_style_indicator_rect() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 2,
            radius: 4,
            offset: 2,
        };
        let (fx, fy, fw, fh) = s.indicator_rect(10, 20, 100, 50);
        assert_eq!(fx, 8);
        assert_eq!(fy, 18);
        assert_eq!(fw, 104);
        assert_eq!(fh, 54);
    }

    #[test]
    fn focus_style_indicator_rect_zero_offset() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 1,
            radius: 0,
            offset: 0,
        };
        let r = s.indicator_rect(5, 10, 80, 30);
        assert_eq!(r, (5, 10, 80, 30));
    }

    #[test]
    fn focus_style_draw_calls_backend() {
        use crate::test_utils::MockBackend;
        let s = FocusStyle::from_accent(Color::rgb(0, 255, 0));
        let mut backend = MockBackend::new();
        s.draw(&mut backend, 10, 20, 100, 50).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    // -- FocusManager tests --

    #[test]
    fn manager_new_defaults() {
        let fm = FocusManager::new(5);
        assert_eq!(fm.count(), 5);
        assert_eq!(fm.focused(), 0);
        assert!(!fm.active);
    }

    #[test]
    fn manager_new_empty() {
        let fm = FocusManager::new(0);
        assert_eq!(fm.count(), 0);
        assert_eq!(fm.focused(), 0);
    }

    #[test]
    fn manager_has_focus_inactive() {
        let fm = FocusManager::new(3);
        assert!(!fm.has_focus(0));
    }

    #[test]
    fn manager_has_focus_active() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        assert!(fm.has_focus(0));
        assert!(!fm.has_focus(1));
    }

    #[test]
    fn manager_tab_next() {
        let mut fm = FocusManager::new(3);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert!(fm.active);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 2);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 0);
    }

    #[test]
    fn manager_tab_prev() {
        let mut fm = FocusManager::new(3);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 2);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 0);
    }

    #[test]
    fn manager_activate() {
        let mut fm = FocusManager::new(3);
        fm.handle_action(FocusAction::TabNext);
        let idx = fm.handle_action(FocusAction::Activate);
        assert_eq!(idx, 1);
    }

    #[test]
    fn manager_home() {
        let mut fm = FocusManager::new(5);
        fm.handle_action(FocusAction::TabNext);
        fm.handle_action(FocusAction::TabNext);
        assert_eq!(fm.handle_action(FocusAction::Home), 0);
    }

    #[test]
    fn manager_end() {
        let mut fm = FocusManager::new(5);
        assert_eq!(fm.handle_action(FocusAction::End), 4);
    }

    #[test]
    fn manager_skip_disabled_next() {
        let mut fm = FocusManager::new(4);
        fm.set_enabled(1, false);
        fm.set_enabled(2, false);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 3);
    }

    #[test]
    fn manager_skip_disabled_prev() {
        let mut fm = FocusManager::new(4);
        fm.set_enabled(2, false);
        fm.set_enabled(3, false);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
    }

    #[test]
    fn manager_all_disabled_stays() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(0, false);
        fm.set_enabled(1, false);
        fm.set_enabled(2, false);
        let before = fm.focused();
        fm.handle_action(FocusAction::TabNext);
        assert_eq!(fm.focused(), before);
    }

    #[test]
    fn manager_set_count_grows() {
        let mut fm = FocusManager::new(3);
        fm.set_count(5);
        assert_eq!(fm.count(), 5);
        assert!(fm.is_enabled(3));
        assert!(fm.is_enabled(4));
    }

    #[test]
    fn manager_set_count_shrinks() {
        let mut fm = FocusManager::new(5);
        fm.ring_mut().set_focused(4);
        fm.set_count(3);
        assert_eq!(fm.count(), 3);
        assert_eq!(fm.focused(), 2);
    }

    #[test]
    fn manager_set_focused_enabled() {
        let mut fm = FocusManager::new(5);
        fm.set_focused(3);
        assert_eq!(fm.focused(), 3);
        assert!(fm.active);
    }

    #[test]
    fn manager_set_focused_disabled_rejected() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(3, false);
        fm.set_focused(3);
        assert_eq!(fm.focused(), 0);
    }

    #[test]
    fn manager_deactivate() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        fm.deactivate();
        assert!(!fm.active);
        assert!(!fm.has_focus(0));
    }

    #[test]
    fn manager_is_enabled_out_of_bounds() {
        let fm = FocusManager::new(3);
        assert!(!fm.is_enabled(10));
    }

    #[test]
    fn manager_set_enabled_out_of_bounds_noop() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(10, false);
        assert_eq!(fm.count(), 3);
    }

    #[test]
    fn manager_ring_access() {
        let fm = FocusManager::new(5);
        assert_eq!(fm.ring().count(), 5);
    }

    #[test]
    fn manager_ring_mut_access() {
        let mut fm = FocusManager::new(5);
        fm.ring_mut().set_focused(3);
        assert_eq!(fm.focused(), 3);
    }

    #[test]
    fn manager_home_skips_disabled_first() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(0, false);
        assert_eq!(fm.handle_action(FocusAction::Home), 1);
    }

    #[test]
    fn manager_end_skips_disabled_last() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(4, false);
        assert_eq!(fm.handle_action(FocusAction::End), 3);
    }

    #[test]
    fn manager_draw_indicator_active() {
        use crate::test_utils::MockBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 0, &s, 10, 20, 80, 30).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn manager_draw_indicator_wrong_index() {
        use crate::test_utils::MockBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 1, &s, 10, 20, 80, 30).ok();
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn manager_draw_indicator_inactive() {
        use crate::test_utils::MockBackend;
        let fm = FocusManager::new(3);
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 0, &s, 10, 20, 80, 30).ok();
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn manager_empty_handle_action() {
        let mut fm = FocusManager::new(0);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 0);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 0);
        assert_eq!(fm.handle_action(FocusAction::Home), 0);
        assert_eq!(fm.handle_action(FocusAction::End), 0);
    }

    #[test]
    fn focus_action_debug() {
        for a in [
            FocusAction::TabNext,
            FocusAction::TabPrev,
            FocusAction::Activate,
            FocusAction::Home,
            FocusAction::End,
        ] {
            let _ = format!("{a:?}");
        }
    }

    #[test]
    fn focus_action_equality() {
        assert_eq!(FocusAction::TabNext, FocusAction::TabNext);
        assert_ne!(FocusAction::TabNext, FocusAction::TabPrev);
    }

    #[test]
    fn manager_single_enabled_item() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(0, false);
        fm.set_enabled(2, false);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
    }

    #[test]
    fn manager_tab_full_cycle() {
        let mut fm = FocusManager::new(4);
        let mut visited = Vec::new();
        for _ in 0..4 {
            let idx = fm.handle_action(FocusAction::TabNext);
            visited.push(idx);
        }
        assert_eq!(visited, vec![1, 2, 3, 0]);
    }

    #[test]
    fn manager_clone() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        fm.set_enabled(1, false);
        let fm2 = fm.clone();
        assert_eq!(fm.focused(), fm2.focused());
        assert_eq!(fm.active, fm2.active);
        assert_eq!(fm.is_enabled(1), fm2.is_enabled(1));
    }
}
