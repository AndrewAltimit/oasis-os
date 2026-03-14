//! Window stacking and z-order management.
//!
//! Determines insertion indices for windows based on z-order groups:
//! normal windows < always-on-top < modal (topmost).

use super::manager::WindowManager;
use super::window::Window;

impl WindowManager {
    /// Compute the insertion index for a window based on z-order groups.
    /// Groups: normal < always_on_top < modal.
    pub(crate) fn z_insert_index(&self, window: &Window) -> usize {
        if window.modal {
            // Modal windows go to the absolute top.
            self.windows.len()
        } else if window.always_on_top {
            // Above normal windows, below modal windows.
            self.windows
                .iter()
                .position(|w| w.modal)
                .unwrap_or(self.windows.len())
        } else {
            // Normal windows: below always_on_top and modal.
            self.windows
                .iter()
                .position(|w| w.always_on_top || w.modal)
                .unwrap_or(self.windows.len())
        }
    }
}
