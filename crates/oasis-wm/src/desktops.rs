//! Virtual desktop management for the window manager.
//!
//! Provides `DesktopManager` which tracks multiple virtual desktops and
//! controls which windows are visible based on their desktop assignment.
//! Windows can be pinned to a single desktop or made sticky (visible on
//! all desktops).

use std::collections::HashMap;

/// Zero-based desktop index.
pub type DesktopId = usize;

/// How a window is assigned to desktops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowPlacement {
    /// Window appears on a specific desktop.
    Desktop(DesktopId),
    /// Window appears on all desktops (sticky).
    AllDesktops,
}

/// Manages virtual desktops and window-to-desktop assignments.
///
/// Each window can be assigned to a single desktop or marked as sticky
/// (visible on all desktops). Windows that have not been explicitly
/// assigned default to the currently active desktop.
pub struct DesktopManager {
    /// Total number of desktops.
    count: usize,
    /// Currently active desktop.
    active: DesktopId,
    /// Window placement mapping.
    placements: HashMap<String, WindowPlacement>,
    /// Desktop names (optional, for display).
    names: Vec<String>,
}

impl DesktopManager {
    /// Create a new desktop manager with `count` desktops.
    ///
    /// The count is clamped to a minimum of 1. Desktops are named
    /// "Desktop 1", "Desktop 2", etc. by default. The first desktop
    /// (index 0) is initially active.
    pub fn new(count: usize) -> Self {
        let count = count.max(1);
        let names = (1..=count).map(|i| format!("Desktop {i}")).collect();
        Self {
            count,
            active: 0,
            placements: HashMap::new(),
            names,
        }
    }

    /// Return the total number of desktops.
    pub fn desktop_count(&self) -> usize {
        self.count
    }

    /// Return the currently active desktop index.
    pub fn active_desktop(&self) -> DesktopId {
        self.active
    }

    /// Switch to the specified desktop.
    ///
    /// Returns `true` if the switch was successful, `false` if the
    /// desktop index is out of range.
    pub fn switch_to(&mut self, desktop: DesktopId) -> bool {
        if desktop < self.count {
            self.active = desktop;
            true
        } else {
            false
        }
    }

    /// Cycle to the next desktop, wrapping around to 0 at the end.
    ///
    /// Returns the new active desktop index.
    pub fn switch_next(&mut self) -> DesktopId {
        self.active = (self.active + 1) % self.count;
        self.active
    }

    /// Cycle to the previous desktop, wrapping around to the last
    /// desktop when at 0.
    ///
    /// Returns the new active desktop index.
    pub fn switch_prev(&mut self) -> DesktopId {
        if self.active == 0 {
            self.active = self.count - 1;
        } else {
            self.active -= 1;
        }
        self.active
    }

    /// Assign a window to a specific desktop or make it sticky.
    ///
    /// If the window was previously tracked, its placement is updated.
    pub fn assign_window(&mut self, window_id: &str, placement: WindowPlacement) {
        self.placements.insert(window_id.to_string(), placement);
    }

    /// Remove a window from tracking (e.g. when the window is closed).
    pub fn remove_window(&mut self, window_id: &str) {
        self.placements.remove(window_id);
    }

    /// Get the placement of a window.
    ///
    /// Windows that have not been explicitly assigned are treated as
    /// belonging to the currently active desktop.
    pub fn window_placement(&self, window_id: &str) -> WindowPlacement {
        self.placements
            .get(window_id)
            .cloned()
            .unwrap_or(WindowPlacement::Desktop(self.active))
    }

    /// Check whether a window is visible on the currently active desktop.
    ///
    /// A window is visible if it is sticky (`AllDesktops`) or assigned
    /// to the active desktop. Untracked windows are treated as belonging
    /// to the active desktop and are therefore visible.
    pub fn is_visible(&self, window_id: &str) -> bool {
        match self.placements.get(window_id) {
            Some(WindowPlacement::AllDesktops) => true,
            Some(WindowPlacement::Desktop(d)) => *d == self.active,
            None => true,
        }
    }

    /// Return all window IDs that are visible on the active desktop.
    ///
    /// This includes windows assigned to the active desktop and all
    /// sticky windows.
    pub fn visible_windows(&self) -> Vec<&str> {
        self.placements
            .iter()
            .filter(|(_, p)| match p {
                WindowPlacement::AllDesktops => true,
                WindowPlacement::Desktop(d) => *d == self.active,
            })
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Return all window IDs assigned to a specific desktop.
    ///
    /// This includes windows explicitly assigned to that desktop and
    /// all sticky windows. Returns an empty vec if the desktop index
    /// is out of range.
    pub fn windows_on_desktop(&self, desktop: DesktopId) -> Vec<&str> {
        if desktop >= self.count {
            return Vec::new();
        }
        self.placements
            .iter()
            .filter(|(_, p)| match p {
                WindowPlacement::AllDesktops => true,
                WindowPlacement::Desktop(d) => *d == desktop,
            })
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Move a window to a different desktop.
    ///
    /// If the window is currently sticky, it becomes pinned to the
    /// target desktop. If the desktop index is out of range, the
    /// window is not moved.
    pub fn move_window_to_desktop(&mut self, window_id: &str, desktop: DesktopId) {
        if desktop < self.count {
            self.placements
                .insert(window_id.to_string(), WindowPlacement::Desktop(desktop));
        }
    }

    /// Toggle a window between sticky (`AllDesktops`) and pinned to
    /// the active desktop.
    ///
    /// If the window is currently sticky, it is pinned to the active
    /// desktop. Otherwise, it becomes sticky.
    pub fn toggle_sticky(&mut self, window_id: &str) {
        let new_placement = match self.placements.get(window_id) {
            Some(WindowPlacement::AllDesktops) => WindowPlacement::Desktop(self.active),
            _ => WindowPlacement::AllDesktops,
        };
        self.placements.insert(window_id.to_string(), new_placement);
    }

    /// Set the display name for a desktop.
    ///
    /// Returns `false` if the desktop index is out of range.
    pub fn set_desktop_name(&mut self, desktop: DesktopId, name: String) -> bool {
        if desktop < self.count {
            self.names[desktop] = name;
            true
        } else {
            false
        }
    }

    /// Get the display name of a desktop.
    ///
    /// Returns `None` if the desktop index is out of range.
    pub fn desktop_name(&self, desktop: DesktopId) -> Option<&str> {
        self.names.get(desktop).map(String::as_str)
    }

    /// Add a new desktop at the end.
    ///
    /// Returns the index of the newly created desktop.
    pub fn add_desktop(&mut self) -> DesktopId {
        let id = self.count;
        self.count += 1;
        self.names.push(format!("Desktop {}", self.count));
        id
    }

    /// Remove a desktop by index.
    ///
    /// Returns `false` if the index is out of range or if there is
    /// only one desktop remaining (cannot go below 1). All windows
    /// assigned to the removed desktop are reassigned to the currently
    /// active desktop (after adjustment).
    pub fn remove_desktop(&mut self, desktop: DesktopId) -> bool {
        if self.count <= 1 || desktop >= self.count {
            return false;
        }

        self.names.remove(desktop);
        self.count -= 1;

        // Adjust active desktop if needed.
        if self.active == desktop {
            // Snap to the previous desktop, or 0 if we removed desktop 0.
            if self.active >= self.count {
                self.active = self.count - 1;
            }
        } else if self.active > desktop {
            self.active -= 1;
        }

        // Reassign or adjust window placements.
        let active = self.active;
        for placement in self.placements.values_mut() {
            if let WindowPlacement::Desktop(d) = placement {
                if *d == desktop {
                    // Window was on the removed desktop: reassign to
                    // active.
                    *d = active;
                } else if *d > desktop {
                    // Shift indices down to close the gap.
                    *d -= 1;
                }
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Construction
    // ---------------------------------------------------------------

    #[test]
    fn new_with_multiple_desktops() {
        let dm = DesktopManager::new(4);
        assert_eq!(dm.desktop_count(), 4);
        assert_eq!(dm.active_desktop(), 0);
    }

    #[test]
    fn new_clamps_zero_to_one() {
        let dm = DesktopManager::new(0);
        assert_eq!(dm.desktop_count(), 1);
    }

    #[test]
    fn new_single_desktop() {
        let dm = DesktopManager::new(1);
        assert_eq!(dm.desktop_count(), 1);
        assert_eq!(dm.active_desktop(), 0);
    }

    #[test]
    fn new_default_names() {
        let dm = DesktopManager::new(3);
        assert_eq!(dm.desktop_name(0), Some("Desktop 1"));
        assert_eq!(dm.desktop_name(1), Some("Desktop 2"));
        assert_eq!(dm.desktop_name(2), Some("Desktop 3"));
    }

    // ---------------------------------------------------------------
    // Switching
    // ---------------------------------------------------------------

    #[test]
    fn switch_to_valid() {
        let mut dm = DesktopManager::new(3);
        assert!(dm.switch_to(2));
        assert_eq!(dm.active_desktop(), 2);
    }

    #[test]
    fn switch_to_invalid() {
        let mut dm = DesktopManager::new(3);
        assert!(!dm.switch_to(5));
        assert_eq!(dm.active_desktop(), 0);
    }

    #[test]
    fn switch_to_boundary() {
        let mut dm = DesktopManager::new(3);
        assert!(dm.switch_to(2));
        assert!(!dm.switch_to(3));
        assert_eq!(dm.active_desktop(), 2);
    }

    #[test]
    fn switch_next_wraps() {
        let mut dm = DesktopManager::new(3);
        assert_eq!(dm.switch_next(), 1);
        assert_eq!(dm.switch_next(), 2);
        assert_eq!(dm.switch_next(), 0); // wrap
    }

    #[test]
    fn switch_prev_wraps() {
        let mut dm = DesktopManager::new(3);
        assert_eq!(dm.switch_prev(), 2); // wrap from 0
        assert_eq!(dm.switch_prev(), 1);
        assert_eq!(dm.switch_prev(), 0);
    }

    #[test]
    fn switch_next_single_desktop() {
        let mut dm = DesktopManager::new(1);
        assert_eq!(dm.switch_next(), 0);
        assert_eq!(dm.switch_next(), 0);
    }

    #[test]
    fn switch_prev_single_desktop() {
        let mut dm = DesktopManager::new(1);
        assert_eq!(dm.switch_prev(), 0);
    }

    // ---------------------------------------------------------------
    // Window assignment and visibility
    // ---------------------------------------------------------------

    #[test]
    fn assign_window_and_check_visibility() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("browser", WindowPlacement::Desktop(0));
        dm.assign_window("terminal", WindowPlacement::Desktop(1));

        assert!(dm.is_visible("browser"));
        assert!(!dm.is_visible("terminal"));

        dm.switch_to(1);
        assert!(!dm.is_visible("browser"));
        assert!(dm.is_visible("terminal"));
    }

    #[test]
    fn all_desktops_always_visible() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("taskbar", WindowPlacement::AllDesktops);

        for i in 0..3 {
            dm.switch_to(i);
            assert!(
                dm.is_visible("taskbar"),
                "sticky window should be visible on desktop {i}"
            );
        }
    }

    #[test]
    fn untracked_window_visible_on_active() {
        let dm = DesktopManager::new(3);
        // Window not assigned -- treated as active desktop.
        assert!(dm.is_visible("unknown_window"));
    }

    #[test]
    fn default_placement_is_active_desktop() {
        let mut dm = DesktopManager::new(3);
        dm.switch_to(2);
        let p = dm.window_placement("untracked");
        assert_eq!(p, WindowPlacement::Desktop(2));
    }

    // ---------------------------------------------------------------
    // Move and toggle
    // ---------------------------------------------------------------

    #[test]
    fn move_window_to_desktop() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("editor", WindowPlacement::Desktop(0));
        assert!(dm.is_visible("editor"));

        dm.move_window_to_desktop("editor", 2);
        assert!(!dm.is_visible("editor"));

        dm.switch_to(2);
        assert!(dm.is_visible("editor"));
        assert_eq!(dm.window_placement("editor"), WindowPlacement::Desktop(2));
    }

    #[test]
    fn move_window_to_invalid_desktop_is_noop() {
        let mut dm = DesktopManager::new(2);
        dm.assign_window("win", WindowPlacement::Desktop(0));
        dm.move_window_to_desktop("win", 99);
        assert_eq!(dm.window_placement("win"), WindowPlacement::Desktop(0));
    }

    #[test]
    fn toggle_sticky_on() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("chat", WindowPlacement::Desktop(1));
        dm.toggle_sticky("chat");
        assert_eq!(dm.window_placement("chat"), WindowPlacement::AllDesktops);
    }

    #[test]
    fn toggle_sticky_off() {
        let mut dm = DesktopManager::new(3);
        dm.switch_to(2);
        dm.assign_window("chat", WindowPlacement::AllDesktops);
        dm.toggle_sticky("chat");
        assert_eq!(dm.window_placement("chat"), WindowPlacement::Desktop(2));
    }

    #[test]
    fn toggle_sticky_untracked_becomes_sticky() {
        let mut dm = DesktopManager::new(2);
        dm.toggle_sticky("new_win");
        assert_eq!(dm.window_placement("new_win"), WindowPlacement::AllDesktops);
    }

    // ---------------------------------------------------------------
    // Remove window
    // ---------------------------------------------------------------

    #[test]
    fn remove_window_cleanup() {
        let mut dm = DesktopManager::new(2);
        dm.assign_window("temp", WindowPlacement::Desktop(1));
        assert_eq!(dm.window_placement("temp"), WindowPlacement::Desktop(1));
        dm.remove_window("temp");
        // After removal, defaults to active desktop (0).
        assert_eq!(dm.window_placement("temp"), WindowPlacement::Desktop(0));
    }

    #[test]
    fn remove_nonexistent_window_is_noop() {
        let mut dm = DesktopManager::new(2);
        dm.remove_window("does_not_exist"); // should not panic
    }

    // ---------------------------------------------------------------
    // Listing windows
    // ---------------------------------------------------------------

    #[test]
    fn visible_windows_on_active_desktop() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("a", WindowPlacement::Desktop(0));
        dm.assign_window("b", WindowPlacement::Desktop(1));
        dm.assign_window("c", WindowPlacement::Desktop(0));
        dm.assign_window("s", WindowPlacement::AllDesktops);

        let mut visible = dm.visible_windows();
        visible.sort();
        assert_eq!(visible, vec!["a", "c", "s"]);
    }

    #[test]
    fn windows_on_desktop_listing() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("x", WindowPlacement::Desktop(1));
        dm.assign_window("y", WindowPlacement::Desktop(1));
        dm.assign_window("z", WindowPlacement::Desktop(2));
        dm.assign_window("sticky", WindowPlacement::AllDesktops);

        let mut on_1 = dm.windows_on_desktop(1);
        on_1.sort();
        assert_eq!(on_1, vec!["sticky", "x", "y"]);

        let mut on_2 = dm.windows_on_desktop(2);
        on_2.sort();
        assert_eq!(on_2, vec!["sticky", "z"]);
    }

    #[test]
    fn windows_on_invalid_desktop_is_empty() {
        let dm = DesktopManager::new(2);
        assert!(dm.windows_on_desktop(10).is_empty());
    }

    #[test]
    fn visible_windows_empty_when_no_assignments() {
        let dm = DesktopManager::new(2);
        assert!(dm.visible_windows().is_empty());
    }

    // ---------------------------------------------------------------
    // Desktop names
    // ---------------------------------------------------------------

    #[test]
    fn set_and_get_desktop_name() {
        let mut dm = DesktopManager::new(3);
        assert!(dm.set_desktop_name(1, "Work".to_string()));
        assert_eq!(dm.desktop_name(1), Some("Work"));
    }

    #[test]
    fn set_desktop_name_invalid_index() {
        let mut dm = DesktopManager::new(2);
        assert!(!dm.set_desktop_name(5, "Nope".to_string()));
    }

    #[test]
    fn desktop_name_out_of_range() {
        let dm = DesktopManager::new(2);
        assert_eq!(dm.desktop_name(10), None);
    }

    // ---------------------------------------------------------------
    // Add / remove desktops
    // ---------------------------------------------------------------

    #[test]
    fn add_desktop() {
        let mut dm = DesktopManager::new(2);
        let new_id = dm.add_desktop();
        assert_eq!(new_id, 2);
        assert_eq!(dm.desktop_count(), 3);
        assert_eq!(dm.desktop_name(2), Some("Desktop 3"));
    }

    #[test]
    fn add_multiple_desktops() {
        let mut dm = DesktopManager::new(1);
        dm.add_desktop();
        dm.add_desktop();
        assert_eq!(dm.desktop_count(), 3);
        assert_eq!(dm.desktop_name(1), Some("Desktop 2"));
        assert_eq!(dm.desktop_name(2), Some("Desktop 3"));
    }

    #[test]
    fn remove_desktop_basic() {
        let mut dm = DesktopManager::new(3);
        assert!(dm.remove_desktop(1));
        assert_eq!(dm.desktop_count(), 2);
        assert_eq!(dm.desktop_name(0), Some("Desktop 1"));
        assert_eq!(dm.desktop_name(1), Some("Desktop 3"));
    }

    #[test]
    fn remove_desktop_reassigns_windows() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("win_a", WindowPlacement::Desktop(1));
        dm.assign_window("win_b", WindowPlacement::Desktop(2));

        // Remove desktop 1; active is 0.
        assert!(dm.remove_desktop(1));
        // win_a was on removed desktop 1 -> reassigned to active (0).
        assert_eq!(dm.window_placement("win_a"), WindowPlacement::Desktop(0));
        // win_b was on desktop 2 -> shifted down to 1.
        assert_eq!(dm.window_placement("win_b"), WindowPlacement::Desktop(1));
    }

    #[test]
    fn remove_desktop_cannot_go_below_one() {
        let mut dm = DesktopManager::new(1);
        assert!(!dm.remove_desktop(0));
        assert_eq!(dm.desktop_count(), 1);
    }

    #[test]
    fn remove_desktop_invalid_index() {
        let mut dm = DesktopManager::new(3);
        assert!(!dm.remove_desktop(10));
        assert_eq!(dm.desktop_count(), 3);
    }

    #[test]
    fn remove_active_desktop_adjusts_active() {
        let mut dm = DesktopManager::new(3);
        dm.switch_to(2);
        assert!(dm.remove_desktop(2));
        // Was on desktop 2 which no longer exists; should clamp.
        assert!(dm.active_desktop() < dm.desktop_count());
    }

    #[test]
    fn remove_desktop_before_active_adjusts_index() {
        let mut dm = DesktopManager::new(4);
        dm.switch_to(3);
        assert!(dm.remove_desktop(1));
        // Active was 3, removed 1, so active shifts to 2.
        assert_eq!(dm.active_desktop(), 2);
        assert_eq!(dm.desktop_count(), 3);
    }

    #[test]
    fn remove_desktop_sticky_windows_unaffected() {
        let mut dm = DesktopManager::new(3);
        dm.assign_window("panel", WindowPlacement::AllDesktops);
        assert!(dm.remove_desktop(1));
        assert_eq!(dm.window_placement("panel"), WindowPlacement::AllDesktops);
    }

    // ---------------------------------------------------------------
    // Combined scenarios
    // ---------------------------------------------------------------

    #[test]
    fn full_workflow() {
        let mut dm = DesktopManager::new(2);

        // Assign windows.
        dm.assign_window("browser", WindowPlacement::Desktop(0));
        dm.assign_window("editor", WindowPlacement::Desktop(1));
        dm.assign_window("dock", WindowPlacement::AllDesktops);

        // Desktop 0: browser + dock visible.
        let mut v = dm.visible_windows();
        v.sort();
        assert_eq!(v, vec!["browser", "dock"]);

        // Switch to desktop 1: editor + dock visible.
        dm.switch_to(1);
        let mut v = dm.visible_windows();
        v.sort();
        assert_eq!(v, vec!["dock", "editor"]);

        // Move editor to desktop 0.
        dm.move_window_to_desktop("editor", 0);
        assert!(!dm.is_visible("editor"));

        // Add a new desktop and move browser there.
        let d2 = dm.add_desktop();
        dm.move_window_to_desktop("browser", d2);

        dm.switch_to(d2);
        let mut v = dm.visible_windows();
        v.sort();
        assert_eq!(v, vec!["browser", "dock"]);

        // Toggle dock off sticky.
        dm.toggle_sticky("dock");
        assert_eq!(dm.window_placement("dock"), WindowPlacement::Desktop(d2));

        // Close browser.
        dm.remove_window("browser");
        let mut v = dm.visible_windows();
        v.sort();
        assert_eq!(v, vec!["dock"]);
    }
}
