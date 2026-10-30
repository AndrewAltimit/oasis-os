//! [`FocusManager`]: Tab / Shift-Tab cycling with disabled-item skipping.

use super::{FocusDir, FocusRing, FocusStyle};

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

    /// Draw the focus indicator around a widget if it has focus,
    /// styled from the theme (honors skin `focus_ring_*` overrides).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_indicator_themed(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        index: usize,
        theme: &crate::theme::Theme,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        self.draw_indicator(backend, index, &FocusStyle::from_theme(theme), x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use crate::focus::*;
    #[allow(unused_imports)]
    use oasis_types::backend::Color;
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
        use crate::test_utils::RecordingBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = RecordingBackend::default();
        fm.draw_indicator(&mut backend, 0, &s, 10, 20, 80, 30).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn manager_draw_indicator_wrong_index() {
        use crate::test_utils::RecordingBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = RecordingBackend::default();
        fm.draw_indicator(&mut backend, 1, &s, 10, 20, 80, 30).ok();
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn manager_draw_indicator_inactive() {
        use crate::test_utils::RecordingBackend;
        let fm = FocusManager::new(3);
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = RecordingBackend::default();
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
