//! Window lifecycle: creation, destruction, and state transitions.
//!
//! Handles `create_window`, `close_window`, `close_all`, `minimize_window`,
//! `maximize_window`, `restore_window`, `enter_fullscreen`, and
//! `exit_fullscreen`.

use oasis_sdi::SdiRegistry;
use oasis_types::error::{OasisError, Result, WmError};

use super::manager::{CASCADE_OFFSET, WindowManager};
use super::window::{Geometry, Window, WindowConfig, WindowId, WindowState};

impl WindowManager {
    /// Create a new window and register its SDI objects.
    pub fn create_window(
        &mut self,
        config: &WindowConfig,
        sdi: &mut SdiRegistry,
    ) -> Result<WindowId> {
        // Check for duplicate id.
        if self.windows.iter().any(|w| w.id == config.id) {
            return Err(OasisError::Wm(WmError::WindowAlreadyExists {
                id: config.id.clone(),
            }));
        }

        // Determine initial position.
        let (x, y) = match (config.x, config.y) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                let pos = (self.next_cascade_x, self.next_cascade_y);
                self.advance_cascade();
                pos
            },
        };

        let window = Window::new(config, x, y, &self.theme);
        let is_modal = window.modal;

        // Create modal overlay before the window if this is a modal window.
        if is_modal {
            self.show_modal_overlay(sdi);
        }

        // Create SDI objects for each component.
        self.create_sdi_objects(&window, sdi);

        let id = window.id.clone();
        self.windows.push(window);

        // Focus the new window (will respect z-order groups).
        self.focus_window_internal(&id, sdi);

        Ok(id)
    }

    /// Close all open windows.
    pub fn close_all(&mut self, sdi: &mut SdiRegistry) {
        let windows = std::mem::take(&mut self.windows);
        for window in &windows {
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                let _ = sdi.destroy(&name);
            }
        }
        self.drag = None;
        self.active_window = None;
        self.hide_modal_overlay(sdi);
    }

    /// Close a window, destroying all its SDI objects.
    pub fn close_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let idx = self
            .windows
            .iter()
            .position(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        let was_modal = self.windows[idx].modal;
        let window = &self.windows[idx];
        self.destroy_sdi_objects(window, sdi);
        self.windows.remove(idx);

        // Cancel any drag on this window.
        if let Some(ref drag) = self.drag {
            let drag_id = match drag {
                super::drag_resize::DragState::Moving { window_id, .. } => window_id.clone(),
                super::drag_resize::DragState::Resizing { window_id, .. } => window_id.clone(),
            };
            if drag_id == id {
                self.drag = None;
            }
        }

        // Update active window.
        if self.active_window.as_deref() == Some(id) {
            self.active_window = self.windows.last().map(|w| w.id.clone());
        }

        // Hide modal overlay if no more modal windows remain.
        if was_modal && !self.has_modal() {
            self.hide_modal_overlay(sdi);
        }

        Ok(())
    }

    /// Minimize a window (hide all SDI objects).
    pub fn minimize_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        if !window.has_minimize_button() {
            return Err(OasisError::Wm(
                format!("window type does not support minimize: {id}").into(),
            ));
        }

        if window.state == WindowState::Normal {
            window.saved_geometry = Some(Geometry {
                x: window.x,
                y: window.y,
                w: window.outer_w,
                h: window.outer_h,
            });
        }
        window.state = WindowState::Minimized;

        // Hide all SDI objects.
        for suffix in window.sdi_suffixes() {
            let name = window.sdi_name(suffix);
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }

        // Move focus to next topmost visible window.
        let new_active = self
            .windows
            .iter()
            .rev()
            .find(|w| w.state != WindowState::Minimized && w.id != id)
            .map(|w| w.id.clone());
        self.active_window = new_active;

        Ok(())
    }

    /// Maximize a window to fill the screen.
    pub fn maximize_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        if !window.has_maximize_button() {
            return Err(OasisError::Wm(
                format!("window type does not support maximize: {id}").into(),
            ));
        }

        // Save geometry for restore.
        window.saved_geometry = Some(Geometry {
            x: window.x,
            y: window.y,
            w: window.outer_w,
            h: window.outer_h,
        });

        window.x = 0;
        window.y = self.theme.maximize_top_inset as i32;
        window.outer_w = self.screen_w;
        window.outer_h =
            self.screen_h - self.theme.maximize_top_inset - self.theme.maximize_bottom_inset;
        window.state = WindowState::Maximized;

        self.update_sdi_positions(id, sdi);

        Ok(())
    }

    /// Restore a window from minimized or maximized state.
    pub fn restore_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        let was_minimized = window.state == WindowState::Minimized;

        if let Some(geom) = window.saved_geometry.take() {
            window.x = geom.x;
            window.y = geom.y;
            window.outer_w = geom.w;
            window.outer_h = geom.h;
        }
        window.state = WindowState::Normal;

        if was_minimized {
            // Show all SDI objects.
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = true;
                }
            }
        }

        self.update_sdi_positions(id, sdi);

        Ok(())
    }

    /// Enter fullscreen kiosk mode for the given window.
    ///
    /// Saves the current geometry, expands the window to fill the screen,
    /// sets the `fullscreen_kiosk` flag, and hides all decoration SDI objects.
    pub fn enter_fullscreen(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let (sw, sh) = (self.screen_w, self.screen_h);
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        // Save current geometry for restore (separate from maximize/minimize
        // saved_geometry so we don't clobber it).
        window.kiosk_saved_geometry = Some(Geometry {
            x: window.x,
            y: window.y,
            w: window.outer_w,
            h: window.outer_h,
        });

        window.x = 0;
        window.y = 0;
        window.outer_w = sw;
        window.outer_h = sh;
        window.fullscreen_kiosk = true;

        // Hide all decoration SDI objects (everything except "content").
        for &suffix in window.sdi_suffixes() {
            if suffix != "content" {
                let name = window.sdi_name(suffix);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }

        self.update_sdi_positions(id, sdi);
        Ok(())
    }

    /// Exit fullscreen kiosk mode for the given window.
    ///
    /// Restores the saved geometry, clears the `fullscreen_kiosk` flag,
    /// and re-shows all decoration SDI objects.
    pub fn exit_fullscreen(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        if let Some(geom) = window.kiosk_saved_geometry.take() {
            window.x = geom.x;
            window.y = geom.y;
            window.outer_w = geom.w;
            window.outer_h = geom.h;
        }

        window.fullscreen_kiosk = false;

        // Re-show all decoration SDI objects.
        for suffix in window.sdi_suffixes() {
            let name = window.sdi_name(suffix);
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = true;
            }
        }

        self.update_sdi_positions(id, sdi);
        Ok(())
    }

    /// Hide all SDI objects for every window (decorations + content).
    ///
    /// Used when a kiosk/fullscreen app is active so floating window
    /// chrome doesn't bleed through.
    pub fn hide_all_window_sdi(&self, sdi: &mut SdiRegistry) {
        for window in &self.windows {
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Show SDI objects for all non-minimized windows.
    ///
    /// Call when returning from kiosk mode to restore window decorations.
    pub fn show_all_window_sdi(&self, sdi: &mut SdiRegistry) {
        for window in &self.windows {
            if window.state == crate::window::WindowState::Minimized {
                continue;
            }
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = true;
                }
            }
        }
    }

    /// Advance the cascade position for the next window.
    pub(crate) fn advance_cascade(&mut self) {
        self.next_cascade_x += CASCADE_OFFSET;
        self.next_cascade_y += CASCADE_OFFSET;

        // Wrap when we get close to the screen edge.
        if self.next_cascade_x > self.screen_w as i32 / 2 {
            self.next_cascade_x = CASCADE_OFFSET;
        }
        if self.next_cascade_y > self.screen_h as i32 / 2 {
            self.next_cascade_y = CASCADE_OFFSET;
        }
    }
}
