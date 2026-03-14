//! Focus management and z-order operations for the window manager.
//!
//! Handles `focus_window`, `cycle_focus`, and internal z-order maintenance
//! including titlebar color updates for active/inactive state.

use oasis_sdi::SdiRegistry;
use oasis_types::error::{OasisError, Result, WmError};

use super::manager::{MODAL_OVERLAY_ID, WindowManager};
use super::window::{WindowId, WindowState};

impl WindowManager {
    /// Bring a window to the front (topmost z-order).
    pub fn focus_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        if !self.windows.iter().any(|w| w.id == id) {
            return Err(OasisError::Wm(WmError::WindowNotFound {
                id: id.to_string(),
            }));
        }
        self.focus_window_internal(id, sdi);
        Ok(())
    }

    /// Cycle focus to the next or previous window in z-order.
    /// `forward=true` brings the bottom-most visible window to the top.
    /// `forward=false` sends the top-most visible window to the bottom.
    /// Skips minimized windows. Returns the newly focused window id, if any.
    pub fn cycle_focus(&mut self, forward: bool, sdi: &mut SdiRegistry) -> Option<WindowId> {
        let visible_count = self
            .windows
            .iter()
            .filter(|w| w.state != WindowState::Minimized)
            .count();
        if visible_count < 2 {
            return self.active_window.clone();
        }
        if forward {
            // Bring the bottom-most visible window to the top.
            let idx = self
                .windows
                .iter()
                .position(|w| w.state != WindowState::Minimized)?;
            let id = self.windows[idx].id.clone();
            self.focus_window_internal(&id, sdi);
            Some(id)
        } else {
            // Send top-most visible to bottom of its z-group, then focus
            // the new top. Use group-aware insertion to preserve
            // normal < always_on_top < modal ordering.
            let top_idx = self
                .windows
                .iter()
                .rposition(|w| w.state != WindowState::Minimized)?;
            let window = self.windows.remove(top_idx);
            let group_start = if window.modal {
                // Bottom of modal group.
                self.windows
                    .iter()
                    .position(|w| w.modal)
                    .unwrap_or(self.windows.len())
            } else if window.always_on_top {
                // Bottom of always_on_top group.
                self.windows
                    .iter()
                    .position(|w| w.always_on_top || w.modal)
                    .unwrap_or(self.windows.len())
            } else {
                // Bottom of normal windows (index 0 or first visible).
                self.windows
                    .iter()
                    .position(|w| w.state != WindowState::Minimized)
                    .unwrap_or(0)
            };
            self.windows.insert(group_start, window);
            // Focus the new top-most visible window.
            let new_top = self
                .windows
                .iter()
                .rposition(|w| w.state != WindowState::Minimized)?;
            let id = self.windows[new_top].id.clone();
            self.focus_window_internal(&id, sdi);
            Some(id)
        }
    }

    /// Move a window to the top of its z-order group and update SDI z-ordering.
    /// Groups: normal < always_on_top < modal.
    pub(crate) fn focus_window_internal(&mut self, id: &str, sdi: &mut SdiRegistry) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            let window = self.windows.remove(idx);
            let insert_at = self.z_insert_index(&window);
            self.windows.insert(insert_at, window);
        }

        // Re-establish SDI z-ordering for the entire window stack.
        // This ensures groups (normal < always_on_top < modal) are correct
        // and the modal overlay sits between non-modal and modal windows.
        let mut overlay_moved = false;
        for window in &self.windows {
            // Place modal overlay once, just before the first modal window.
            if window.modal && !overlay_moved && sdi.contains(MODAL_OVERLAY_ID) {
                let _ = sdi.move_to_top(MODAL_OVERLAY_ID);
                overlay_moved = true;
            }
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                let _ = sdi.move_to_top(&name);
            }
        }

        // Update titlebar colors and frame distinction for all windows.
        for window in &self.windows {
            let is_active = window.id == id;
            let color = if is_active {
                self.theme.titlebar_active_color
            } else {
                self.theme.titlebar_inactive_color
            };
            let tb_name = window.sdi_name("titlebar");
            if let Ok(obj) = sdi.get_mut(&tb_name) {
                obj.color = color;
                // Update gradient colors on focus change.
                if self.theme.titlebar_gradient {
                    use oasis_types::color::lighten;
                    if is_active {
                        obj.gradient_top = Some(
                            self.theme
                                .titlebar_gradient_top
                                .unwrap_or_else(|| lighten(color, 0.1)),
                        );
                        obj.gradient_bottom =
                            Some(self.theme.titlebar_gradient_bottom.unwrap_or(color));
                    } else {
                        obj.gradient_top = Some(
                            self.theme
                                .titlebar_inactive_gradient_top
                                .unwrap_or_else(|| lighten(color, 0.1)),
                        );
                        obj.gradient_bottom = Some(
                            self.theme
                                .titlebar_inactive_gradient_bottom
                                .unwrap_or(color),
                        );
                    }
                } else {
                    obj.gradient_top = None;
                    obj.gradient_bottom = None;
                }
            }
            // Dim inactive window frames.
            let frame_name = window.sdi_name("frame");
            if let Ok(obj) = sdi.get_mut(&frame_name) {
                if is_active {
                    let fc = self.theme.frame_color;
                    obj.color = fc;
                    if self.theme.frame_shadow_level > 0 {
                        obj.shadow_level = Some(self.theme.frame_shadow_level);
                    }
                } else {
                    obj.color = oasis_types::color::with_alpha(
                        self.theme.frame_color,
                        self.theme.inactive_frame_alpha,
                    );
                    let reduced = self.theme.frame_shadow_level.saturating_sub(1);
                    obj.shadow_level = if reduced > 0 { Some(reduced) } else { None };
                }
            }
        }

        self.active_window = Some(WindowId::from(id));
    }
}
