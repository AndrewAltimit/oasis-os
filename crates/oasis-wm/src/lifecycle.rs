//! Window lifecycle: creation, destruction, and state transitions.
//!
//! Handles `create_window`, `close_window`, `close_all`, `minimize_window`,
//! `maximize_window`, `restore_window`, `enter_fullscreen`, and
//! `exit_fullscreen`.

use oasis_sdi::SdiRegistry;
use oasis_types::error::{OasisError, Result, WmError};

use super::manager::{CASCADE_OFFSET, WindowManager};
use super::sdi_objects::set_window_visible;
use super::window::{Window, WindowConfig, WindowId, WindowState};

impl WindowManager {
    /// Change a window's title (titlebar text and taskbar label).
    /// Returns `false` if there is no such window; setting the current
    /// title again is a no-op.
    pub fn set_window_title(&mut self, id: &str, title: &str, sdi: &mut SdiRegistry) -> bool {
        let Some(window) = self.windows.iter_mut().find(|w| w.id == id) else {
            return false;
        };
        if window.title != title {
            window.title = title.to_string();
            // A running open/minimize animation lays the window out from
            // a copy every tick (which carries the new title); don't cut
            // it short.
            if !self.anim.is_animating(id)
                && let Some(window) = self.windows.iter().find(|w| w.id == id)
            {
                self.layout_window_sdi(window, sdi);
            }
        }
        true
    }

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
        let (x, y, cascaded) = match (config.x, config.y) {
            (Some(x), Some(y)) => (x, y, false),
            _ => {
                let pos = (self.next_cascade_x, self.next_cascade_y);
                self.advance_cascade();
                (pos.0, pos.1, true)
            },
        };

        // A window with this id may still be fading out; drop its ghost
        // so the two don't share SDI objects.
        self.flush_closing(Some(&config.id), sdi);

        let mut window = Window::new(config, x, y, &self.theme);
        if cascaded {
            window = self.place_cascaded(window, config);
        }
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
        self.animate_open(&id, sdi);

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
        for window in &windows {
            self.anim.cancel(&window.id);
        }
        self.anim_visual.clear();
        self.flush_closing(None, sdi);
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
        let window = self.windows.remove(idx);
        let drawn = self.animating_geometry(id);
        self.anim_visual.retain(|(wid, _)| *wid != id);
        // With motion on, the SDI objects outlive the window until the
        // close animation ends (see `motion.rs`).
        if !self.animate_close(&window, drawn) {
            self.anim.cancel(id);
            self.destroy_sdi_objects(&window, sdi);
        }

        self.cancel_drag_of(id);

        // Focus passes to the topmost *visible* window: a minimized one
        // must not become the (invisible) keyboard target.
        if self.active_window.as_deref() == Some(id) {
            self.active_window = None;
            if let Some(next) = self
                .windows
                .iter()
                .rev()
                .find(|w| w.state != WindowState::Minimized)
                .map(|w| w.id.clone())
            {
                self.focus_window_internal(&next, sdi);
            }
        }

        // Hide modal overlay if no more modal windows remain.
        if was_modal && !self.has_modal() {
            self.hide_modal_overlay(sdi);
        }

        Ok(())
    }

    /// Minimize a window (hide all SDI objects).
    pub fn minimize_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        self.cancel_drag_of(id);
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

        match window.state {
            WindowState::Normal => {
                window.saved_geometry = Some(window.geometry());
                window.minimized_from_maximized = false;
            },
            // `saved_geometry` already holds the pre-maximize geometry.
            WindowState::Maximized => window.minimized_from_maximized = true,
            WindowState::Minimized => {},
        }
        window.state = WindowState::Minimized;

        // Hide all SDI objects (after the minimize animation, if any).
        if !self.animate_minimize(id, sdi)
            && let Some(window) = self.windows.iter().find(|w| w.id == id)
        {
            set_window_visible(window, false, sdi);
        }

        // Move focus to the next topmost visible window, unless another
        // (visible) window already has it. Going through
        // `focus_window_internal` repaints the active/inactive titlebars.
        let focus_elsewhere = self.active_window.as_deref().is_some_and(|a| a != id);
        if !focus_elsewhere {
            match self
                .windows
                .iter()
                .rev()
                .find(|w| w.state != WindowState::Minimized && w.id != id)
                .map(|w| w.id.clone())
            {
                Some(next) => self.focus_window_internal(&next, sdi),
                None => self.active_window = None,
            }
        }

        Ok(())
    }

    /// Maximize a window to fill the screen.
    pub fn maximize_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        self.cancel_drag_of(id);
        // The screen minus the status/task bar insets (saturating, so
        // insets taller than a tiny screen give a 0-height window).
        let area = self.work_area();
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

        // Save geometry for restore -- unless already maximized (or
        // minimized from maximized), where the current geometry *is* the
        // maximized one and `saved_geometry` already holds the geometry to
        // restore to. A window minimized from the normal state saved its
        // geometry on minimize, which is the one to keep.
        let was_minimized = window.state == WindowState::Minimized;
        if window.state == WindowState::Normal || window.saved_geometry.is_none() {
            window.saved_geometry = Some(window.geometry());
        }

        window.set_geometry(area);
        window.state = WindowState::Maximized;
        window.minimized_from_maximized = false;

        // Maximizing a minimized window shows it again (it used to stay
        // hidden while claiming to be maximized, so the taskbar could no
        // longer bring it back).
        if was_minimized {
            set_window_visible(window, true, sdi);
        }

        self.update_sdi_positions(id, sdi);

        Ok(())
    }

    /// Restore a window from minimized or maximized state.
    ///
    /// A window minimized while maximized comes back maximized (re-fitted
    /// to the current work area); restoring it again returns it to its
    /// pre-maximize geometry.
    pub fn restore_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        self.cancel_drag_of(id);
        let area = self.work_area();
        // Where the window is drawn right now, if a minimize animation is
        // still running: the restore animation continues from there.
        let visual = self.animating_geometry(id);
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        let was_minimized = window.state == WindowState::Minimized;

        if was_minimized && window.minimized_from_maximized {
            window.set_geometry(area);
            window.state = WindowState::Maximized;
        } else {
            if let Some(geom) = window.saved_geometry.take() {
                window.set_geometry(geom);
            }
            window.state = WindowState::Normal;
        }
        window.minimized_from_maximized = false;

        if was_minimized {
            set_window_visible(window, true, sdi);
        }

        self.update_sdi_positions(id, sdi);
        if was_minimized {
            self.animate_unminimize(id, visual, sdi);
        }

        Ok(())
    }

    /// Enter fullscreen kiosk mode for the given window.
    ///
    /// Saves the current geometry, expands the window to fill the screen,
    /// sets the `fullscreen_kiosk` flag, and hides all decoration SDI objects.
    pub fn enter_fullscreen(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        self.cancel_drag_of(id);
        let (sw, sh) = (self.screen_w, self.screen_h);
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        // Save current geometry for restore (separate from maximize/minimize
        // saved_geometry so we don't clobber it). Re-entering kiosk mode
        // keeps the original: the current geometry is the fullscreen one.
        if !window.fullscreen_kiosk || window.kiosk_saved_geometry.is_none() {
            window.kiosk_saved_geometry = Some(window.geometry());
        }

        window.x = 0;
        window.y = 0;
        window.outer_w = sw;
        window.outer_h = sh;
        window.fullscreen_kiosk = true;

        // Hide all decoration SDI objects (everything except "content").
        // (`set_window_visible` keeps a kiosk window's chrome hidden; a
        // minimized window stays hidden entirely.)
        set_window_visible(window, window.state != WindowState::Minimized, sdi);

        self.update_sdi_positions(id, sdi);
        Ok(())
    }

    /// Exit fullscreen kiosk mode for the given window.
    ///
    /// Restores the saved geometry, clears the `fullscreen_kiosk` flag,
    /// and re-shows all decoration SDI objects.
    pub fn exit_fullscreen(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        self.cancel_drag_of(id);
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        if let Some(geom) = window.kiosk_saved_geometry.take() {
            window.set_geometry(geom);
        }

        window.fullscreen_kiosk = false;

        // Re-show all decoration SDI objects (a minimized window stays
        // hidden until it is restored).
        if window.state != WindowState::Minimized {
            set_window_visible(window, true, sdi);
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
            set_window_visible(window, false, sdi);
        }
    }

    /// Show SDI objects for all non-minimized windows.
    ///
    /// Call when returning from kiosk mode to restore window decorations.
    /// A window still in fullscreen kiosk mode gets only its content back
    /// (its decorations stay hidden, as [`Self::enter_fullscreen`] left
    /// them).
    pub fn show_all_window_sdi(&self, sdi: &mut SdiRegistry) {
        for window in &self.windows {
            if window.state != WindowState::Minimized {
                set_window_visible(window, true, sdi);
            }
        }
    }

    /// Fit a cascaded window on screen. Horizontally a cascade step must
    /// not push the frame past the right edge: pull it back, or to the
    /// left edge when it is wider than the screen. Vertically the window
    /// opens below the host's top chrome ([`Self::placement_insets`]: the
    /// status bar and a skin's tab row) and, when it fits, above the bottom
    /// bar; one taller than the screen below the top chrome is shortened
    /// to fit (at 480x272 a default-size window is nearly as tall as the
    /// screen). Without room for even [`MIN_PLACED_CONTENT_H`] it falls
    /// back to the whole screen.
    fn place_cascaded(&self, mut window: Window, config: &WindowConfig) -> Window {
        let (top, bottom) = self.placement_insets();
        let screen_h = self.screen_h;
        let room = screen_h.saturating_sub(top);
        if window.outer_h > room {
            let chrome = window.outer_h.saturating_sub(config.height);
            let fitted_h = room.saturating_sub(chrome);
            if fitted_h >= MIN_PLACED_CONTENT_H.min(config.height) {
                let fitted = WindowConfig {
                    height: fitted_h,
                    ..config.clone()
                };
                window = Window::new(&fitted, window.x, window.y, &self.theme);
            }
        }
        window.x = fit_on_screen(window.x, window.outer_w, self.screen_w);
        window.y = if window.outer_h <= room {
            // Above the bottom bar when the band holds the window, else
            // overlapping it (the bars draw over windows).
            let band_end = if window.outer_h <= room.saturating_sub(bottom) {
                screen_h - bottom
            } else {
                screen_h
            };
            fit_span(
                window.y.saturating_add_unsigned(top),
                window.outer_h,
                top,
                band_end,
            )
        } else {
            fit_on_screen(window.y, window.outer_h, screen_h)
        };
        window
    }

    /// Advance the cascade position for the next window (see also
    /// [`fit_on_screen`], applied to each cascaded window).
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

/// Smallest content height [`WindowManager::place_cascaded`] shortens a
/// window to (it never grows one).
const MIN_PLACED_CONTENT_H: u32 = 120;

/// `pos` moved back so a `size`-long span starting there ends within a
/// `screen`-long axis, but never before 0.
fn fit_on_screen(pos: i32, size: u32, screen: u32) -> i32 {
    fit_span(pos, size, 0, screen)
}

/// `pos` moved back so a `size`-long span starting there ends by `end`,
/// but never before `start`.
fn fit_span(pos: i32, size: u32, start: u32, end: u32) -> i32 {
    let max = i64::from(end) - i64::from(size);
    // In i32 range: the result lies between `start` and `pos`.
    i64::from(pos).min(max).max(i64::from(start)) as i32
}
