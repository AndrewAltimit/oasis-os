//! Keyboard focus navigation for widget groups.
//!
//! `FocusRing` tracks which widget index has focus and handles
//! directional navigation (next/prev/wrapping). Widgets query the
//! ring to determine their visual focus state.

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
}
