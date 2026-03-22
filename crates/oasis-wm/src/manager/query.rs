//! Query and lookup methods for the window manager.

use super::WindowManager;
use crate::window::{Window, WmTheme};

impl WindowManager {
    /// Get the number of open windows.
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Iterate over all windows in z-order (last = topmost).
    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    /// Returns `true` if any open window has `modal == true`.
    pub fn has_modal(&self) -> bool {
        self.windows.iter().any(|w| w.modal)
    }

    /// Returns the id of the topmost modal window, if any.
    pub fn topmost_modal(&self) -> Option<&str> {
        self.windows
            .iter()
            .rev()
            .find(|w| w.modal)
            .map(|w| w.id.as_str())
    }

    /// Get the active (focused) window id.
    pub fn active_window(&self) -> Option<&str> {
        self.active_window.as_deref()
    }

    /// Get a reference to a window by id.
    pub fn get_window(&self, id: &str) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Returns `true` if any window is currently in fullscreen kiosk mode.
    pub fn has_fullscreen_kiosk(&self) -> bool {
        self.windows.iter().any(|w| w.fullscreen_kiosk)
    }

    /// Find the topmost window whose outer rect contains `(px, py)`.
    ///
    /// Iterates in reverse z-order so the topmost window wins.
    pub fn window_at(&self, px: i32, py: i32) -> Option<&str> {
        self.windows
            .iter()
            .rev()
            .filter(|w| w.state != crate::window::WindowState::Minimized)
            .find(|w| {
                px >= w.x && py >= w.y && px < w.x + w.outer_w as i32 && py < w.y + w.outer_h as i32
            })
            .map(|w| w.id.as_str())
    }

    /// Return the topmost non-minimized window id.
    pub fn topmost_visible(&self) -> Option<&str> {
        self.windows
            .iter()
            .rev()
            .find(|w| w.state != crate::window::WindowState::Minimized)
            .map(|w| w.id.as_str())
    }

    /// Get a reference to the current theme.
    pub fn theme(&self) -> &WmTheme {
        &self.theme
    }

    /// Replace the visual theme at runtime.
    pub fn set_theme(&mut self, theme: WmTheme) {
        self.theme = theme;
    }
}
