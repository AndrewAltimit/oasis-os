//! Window manager: lifecycle, drag/resize, focus, and input dispatch.
//!
//! The WM creates and manipulates groups of SDI objects to simulate windowed
//! interfaces. It is a consumer of the SDI API -- SDI remains a flat scene
//! graph with no concept of grouping or hierarchy.

use oasis_sdi::SdiRegistry;
use oasis_types::backend::SdiBackend;
use oasis_types::error::{OasisError, Result, WmError};
use oasis_types::input::InputEvent;

use super::drag_resize::{DragState, clamp_position};
use super::hit_test::ButtonKind;
use super::window::{Geometry, Window, WindowConfig, WindowId, WindowState, WmTheme};

/// Events produced by the WM in response to input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WmEvent {
    /// A window was brought to front.
    WindowFocused(WindowId),
    /// A window was moved.
    WindowMoved(WindowId),
    /// A window was resized.
    WindowResized(WindowId),
    /// A window was closed.
    WindowClosed(WindowId),
    /// A window was minimized.
    WindowMinimized(WindowId),
    /// A window was maximized.
    WindowMaximized(WindowId),
    /// A window was restored from minimized or maximized.
    WindowRestored(WindowId),
    /// Content area was clicked (coordinates are content-local).
    ContentClick(WindowId, i32, i32),
    /// Desktop background was clicked.
    DesktopClick(i32, i32),
    /// Nothing happened.
    None,
}

/// Cascade offset between newly created windows.
const CASCADE_OFFSET: i32 = 24;

/// SDI object name for the semi-transparent modal backdrop.
pub(crate) const MODAL_OVERLAY_ID: &str = "__wm_modal_overlay";

/// The window manager.
///
/// Manages a list of windows ordered by z-depth (last = topmost).
/// Processes input events through hit testing and a drag/resize state machine.
pub struct WindowManager {
    /// Windows in z-order (last = topmost).
    pub(crate) windows: Vec<Window>,
    /// Visual theme.
    pub(crate) theme: WmTheme,
    /// Cascade position for the next window.
    pub(crate) next_cascade_x: i32,
    pub(crate) next_cascade_y: i32,
    /// Screen dimensions (for maximize and cascade wrapping).
    pub(crate) screen_w: u32,
    pub(crate) screen_h: u32,
    /// Active window id (receives keyboard input).
    pub(crate) active_window: Option<WindowId>,
    /// Current drag/resize operation.
    pub(crate) drag: Option<DragState>,
    /// Currently hovered window button (for hover color feedback).
    pub(crate) hover_button: Option<(WindowId, ButtonKind)>,
}

impl WindowManager {
    /// Create a new window manager for the given screen size.
    pub fn new(screen_w: u32, screen_h: u32) -> Self {
        Self {
            windows: Vec::new(),
            theme: WmTheme::default(),
            next_cascade_x: CASCADE_OFFSET,
            next_cascade_y: CASCADE_OFFSET,
            screen_w,
            screen_h,
            active_window: None,
            drag: None,
            hover_button: None,
        }
    }

    /// Create a new window manager with a custom theme.
    pub fn with_theme(screen_w: u32, screen_h: u32, theme: WmTheme) -> Self {
        Self {
            theme,
            ..Self::new(screen_w, screen_h)
        }
    }

    /// Get a reference to the current theme.
    pub fn theme(&self) -> &WmTheme {
        &self.theme
    }

    /// Replace the visual theme at runtime.
    pub fn set_theme(&mut self, theme: WmTheme) {
        self.theme = theme;
    }

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
                DragState::Moving { window_id, .. } => window_id.clone(),
                DragState::Resizing { window_id, .. } => window_id.clone(),
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

    /// Move a window by a delta. Updates all SDI object positions.
    /// The final position is clamped to keep the titlebar visible on screen.
    pub fn move_window(&mut self, id: &str, dx: i32, dy: i32, sdi: &mut SdiRegistry) -> Result<()> {
        let sw = self.screen_w;
        let sh = self.screen_h;
        let tb_h = self.theme.titlebar_height;

        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        let raw_x = window.x + dx;
        let raw_y = window.y + dy;
        let (cx, cy) = clamp_position(raw_x, raw_y, window.outer_w, sw, sh, tb_h);
        let actual_dx = cx - window.x;
        let actual_dy = cy - window.y;
        window.x = cx;
        window.y = cy;

        // Update all SDI objects.
        for suffix in window.sdi_suffixes() {
            let name = window.sdi_name(suffix);
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.x += actual_dx;
                obj.y += actual_dy;
            }
        }

        Ok(())
    }

    /// Resize a window to new outer dimensions. Repositions all SDI objects.
    pub fn resize_window(
        &mut self,
        id: &str,
        new_outer_w: u32,
        new_outer_h: u32,
        sdi: &mut SdiRegistry,
    ) -> Result<()> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| OasisError::Wm(WmError::WindowNotFound { id: id.to_string() }))?;

        window.outer_w = new_outer_w;
        window.outer_h = new_outer_h;

        // Reposition all SDI objects based on new geometry.
        self.update_sdi_positions(id, sdi);

        Ok(())
    }

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
        for suffix in window.sdi_suffixes() {
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

    /// Returns `true` if any window is currently in fullscreen kiosk mode.
    pub fn has_fullscreen_kiosk(&self) -> bool {
        self.windows.iter().any(|w| w.fullscreen_kiosk)
    }

    /// Process an input event through the WM. Returns what happened.
    pub fn handle_input(&mut self, event: &InputEvent, sdi: &mut SdiRegistry) -> WmEvent {
        match event {
            InputEvent::PointerClick { x, y } => self.handle_click(*x, *y, sdi),
            InputEvent::CursorMove { x, y } => self.handle_cursor_move(*x, *y, sdi),
            InputEvent::PointerRelease { .. } => self.handle_release(),
            _ => WmEvent::None,
        }
    }

    /// Draw window content with clipping. The caller provides a draw callback
    /// for each window's content. The WM sets up clip rects before each call
    /// and resets them after.
    pub fn draw_with_clips<F>(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
        mut draw_content: F,
    ) -> Result<()>
    where
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        // Collect window id prefixes so we can exclude them from the
        // global SDI draw pass (they'll be drawn per-window instead).
        let prefixes: Vec<String> = self.windows.iter().map(|w| format!("{}.", w.id)).collect();
        let prefix_refs: Vec<&str> = prefixes.iter().map(|s| s.as_str()).collect();

        // Draw non-window base SDI objects (wallpaper, dashboard, bars, etc.).
        sdi.draw_base_excluding_prefixes(backend, &prefix_refs)?;

        // Draw each window's SDI objects then content in z-order.
        // This ensures the active (topmost) window renders over all others.
        for window in &self.windows {
            if window.state == WindowState::Minimized {
                continue;
            }

            // Draw this window's SDI objects (frame, titlebar, buttons, etc.).
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                sdi.draw_named(&name, backend)?;
            }

            // Draw clipped content inside the window.
            let (cx, cy, cw, ch) = window.content_rect(&self.theme);
            if cw > 0 && ch > 0 {
                backend.set_clip_rect(cx, cy, cw, ch)?;
                draw_content(&window.id, cx, cy, cw, ch, backend)?;
                backend.reset_clip_rect()?;
            }
        }

        // Draw non-window overlay SDI objects (cursor, start menu, toasts)
        // AFTER windows so they render on top.
        sdi.draw_overlay_excluding_prefixes(backend, &prefix_refs)?;

        Ok(())
    }

    // -- Internal methods --
    // Drag/resize handlers are in drag_resize.rs (handle_click,
    // handle_cursor_move, handle_release).

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

    // SDI object methods (create/destroy/update/hover/modal) are in sdi_objects.rs.

    /// Advance the cascade position for the next window.
    fn advance_cascade(&mut self) {
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

    /// Compute the insertion index for a window based on z-order groups.
    /// Groups: normal < always_on_top < modal.
    fn z_insert_index(&self, window: &Window) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::WindowType;
    use oasis_types::backend::Color;

    fn app_config(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(10),
            y: Some(10),
            width: 200,
            height: 150,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        }
    }

    fn dialog_config(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(50),
            y: Some(50),
            width: 200,
            height: 100,
            window_type: WindowType::Dialog,
            always_on_top: false,
            modal: false,
        }
    }

    #[test]
    fn create_window_adds_sdi_objects() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        assert!(sdi.contains("w1.frame"));
        assert!(sdi.contains("w1.titlebar"));
        assert!(sdi.contains("w1.title_text"));
        assert!(sdi.contains("w1.btn_close"));
        assert!(sdi.contains("w1.btn_minimize"));
        assert!(sdi.contains("w1.btn_maximize"));
        assert!(sdi.contains("w1.content"));
        assert_eq!(wm.window_count(), 1);
    }

    #[test]
    fn create_duplicate_id_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        assert!(wm.create_window(&app_config("w1"), &mut sdi).is_err());
    }

    #[test]
    fn close_window_removes_sdi_objects() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.close_window("w1", &mut sdi).unwrap();

        assert!(!sdi.contains("w1.frame"));
        assert!(!sdi.contains("w1.titlebar"));
        assert!(!sdi.contains("w1.content"));
        assert_eq!(wm.window_count(), 0);
    }

    #[test]
    fn close_nonexistent_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.close_window("nope", &mut sdi).is_err());
    }

    #[test]
    fn move_window_updates_positions() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let orig_x = sdi.get("w1.frame").unwrap().x;
        let orig_y = sdi.get("w1.frame").unwrap().y;

        wm.move_window("w1", 50, 30, &mut sdi).unwrap();

        assert_eq!(sdi.get("w1.frame").unwrap().x, orig_x + 50);
        assert_eq!(sdi.get("w1.frame").unwrap().y, orig_y + 30);
        assert_eq!(
            sdi.get("w1.content").unwrap().x - 50,
            sdi.get("w1.content").unwrap().x - 50
        );
    }

    #[test]
    fn focus_reorders_windows() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // w2 is on top after creation.
        assert_eq!(wm.active_window(), Some("w2"));

        // Focus w1.
        wm.focus_window("w1", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w1"));
    }

    #[test]
    fn minimize_hides_objects() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        wm.minimize_window("w1", &mut sdi).unwrap();

        assert!(!sdi.get("w1.frame").unwrap().visible);
        assert!(!sdi.get("w1.content").unwrap().visible);
        assert_eq!(wm.get_window("w1").unwrap().state, WindowState::Minimized);
    }

    #[test]
    fn maximize_fills_screen() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        wm.maximize_window("w1", &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        assert_eq!(win.outer_w, 800);
        assert_eq!(win.outer_h, 600);
        assert_eq!(win.x, 0);
        assert_eq!(win.y, 0);
        assert_eq!(win.state, WindowState::Maximized);
    }

    #[test]
    fn restore_from_maximized() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let orig = wm.get_window("w1").unwrap();
        let orig_x = orig.x;
        let orig_w = orig.outer_w;

        wm.maximize_window("w1", &mut sdi).unwrap();
        wm.restore_window("w1", &mut sdi).unwrap();

        let restored = wm.get_window("w1").unwrap();
        assert_eq!(restored.x, orig_x);
        assert_eq!(restored.outer_w, orig_w);
        assert_eq!(restored.state, WindowState::Normal);
    }

    #[test]
    fn restore_from_minimized_shows_objects() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        wm.minimize_window("w1", &mut sdi).unwrap();
        assert!(!sdi.get("w1.frame").unwrap().visible);

        wm.restore_window("w1", &mut sdi).unwrap();
        assert!(sdi.get("w1.frame").unwrap().visible);
        assert!(sdi.get("w1.content").unwrap().visible);
    }

    #[test]
    fn cascade_positions_offset() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);

        let config1 = WindowConfig {
            id: "a".to_string(),
            title: "A".to_string(),
            x: None,
            y: None,
            width: 100,
            height: 80,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        let config2 = WindowConfig {
            id: "b".to_string(),
            title: "B".to_string(),
            x: None,
            y: None,
            width: 100,
            height: 80,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };

        wm.create_window(&config1, &mut sdi).unwrap();
        wm.create_window(&config2, &mut sdi).unwrap();

        let a = wm.get_window("a").unwrap();
        let b = wm.get_window("b").unwrap();
        assert_eq!(b.x - a.x, CASCADE_OFFSET);
        assert_eq!(b.y - a.y, CASCADE_OFFSET);
    }

    #[test]
    fn dialog_cannot_minimize() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&dialog_config("dlg"), &mut sdi).unwrap();
        assert!(wm.minimize_window("dlg", &mut sdi).is_err());
    }

    #[test]
    fn dialog_cannot_maximize() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&dialog_config("dlg"), &mut sdi).unwrap();
        assert!(wm.maximize_window("dlg", &mut sdi).is_err());
    }

    #[test]
    fn titlebar_active_inactive_colors() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // w2 is active, w1 is inactive.
        let w1_tb = sdi.get("w1.titlebar").unwrap().color;
        let w2_tb = sdi.get("w2.titlebar").unwrap().color;
        assert_eq!(w2_tb, wm.theme.titlebar_active_color);
        assert_eq!(w1_tb, wm.theme.titlebar_inactive_color);

        // Focus w1.
        wm.focus_window("w1", &mut sdi).unwrap();
        let w1_tb = sdi.get("w1.titlebar").unwrap().color;
        let w2_tb = sdi.get("w2.titlebar").unwrap().color;
        assert_eq!(w1_tb, wm.theme.titlebar_active_color);
        assert_eq!(w2_tb, wm.theme.titlebar_inactive_color);
    }

    #[test]
    fn close_updates_active_to_next_topmost() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // Close w2 (active).
        wm.close_window("w2", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w1"));
    }

    #[test]
    fn minimize_updates_active() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        wm.minimize_window("w2", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w1"));
    }

    #[test]
    fn fullscreen_window_creates_only_content() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let config = WindowConfig {
            id: "fs".to_string(),
            title: "Full".to_string(),
            x: Some(0),
            y: Some(0),
            width: 800,
            height: 600,
            window_type: WindowType::Fullscreen,
            always_on_top: false,
            modal: false,
        };
        wm.create_window(&config, &mut sdi).unwrap();

        assert!(sdi.contains("fs.content"));
        assert!(!sdi.contains("fs.frame"));
        assert!(!sdi.contains("fs.titlebar"));
    }

    #[test]
    fn with_theme_constructor() {
        let theme = WmTheme {
            titlebar_height: 32,
            ..WmTheme::default()
        };
        let wm = WindowManager::with_theme(800, 600, theme);
        assert_eq!(wm.theme().titlebar_height, 32);
    }

    // ---- Multi-window workflow integration tests ----

    #[test]
    fn workflow_open_three_close_middle_active_correct() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("a"), &mut sdi).unwrap();
        wm.create_window(&app_config("b"), &mut sdi).unwrap();
        wm.create_window(&app_config("c"), &mut sdi).unwrap();
        assert_eq!(wm.window_count(), 3);
        assert_eq!(wm.active_window(), Some("c"));

        // Focus middle window, then close it.
        wm.focus_window("b", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("b"));

        wm.close_window("b", &mut sdi).unwrap();
        assert_eq!(wm.window_count(), 2);
        // Active should fall to the remaining topmost.
        assert!(wm.active_window().is_some());
        let active = wm.active_window().unwrap();
        assert!(active == "a" || active == "c");
        // SDI objects for b should be gone.
        assert!(!sdi.contains("b.frame"));
        assert!(!sdi.contains("b.content"));
    }

    #[test]
    fn workflow_minimize_restore_preserves_geometry() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        // Move to a known position.
        wm.move_window("w", 100, 50, &mut sdi).unwrap();
        let frame_x = sdi.get("w.frame").unwrap().x;
        let frame_y = sdi.get("w.frame").unwrap().y;

        // Minimize then restore.
        wm.minimize_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Minimized);

        wm.restore_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);

        // Position should be the same after restore.
        assert_eq!(sdi.get("w.frame").unwrap().x, frame_x);
        assert_eq!(sdi.get("w.frame").unwrap().y, frame_y);
    }

    #[test]
    fn workflow_maximize_restore_preserves_geometry() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        let orig_w = sdi.get("w.frame").unwrap().w;
        let orig_h = sdi.get("w.frame").unwrap().h;

        wm.maximize_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Maximized);
        // Maximized frame should be larger than original.
        assert!(sdi.get("w.frame").unwrap().w >= orig_w);

        wm.restore_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);
        assert_eq!(sdi.get("w.frame").unwrap().w, orig_w);
        assert_eq!(sdi.get("w.frame").unwrap().h, orig_h);
    }

    #[test]
    fn workflow_focus_cycle_through_windows() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();
        wm.create_window(&app_config("w3"), &mut sdi).unwrap();

        // Cycle: w3 (top) -> focus w1 -> focus w2 -> focus w3.
        assert_eq!(wm.active_window(), Some("w3"));

        wm.focus_window("w1", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w1"));

        wm.focus_window("w2", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w2"));

        wm.focus_window("w3", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w3"));

        // All three still exist.
        assert_eq!(wm.window_count(), 3);
    }

    #[test]
    fn workflow_close_all_windows() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("a"), &mut sdi).unwrap();
        wm.create_window(&app_config("b"), &mut sdi).unwrap();
        wm.create_window(&app_config("c"), &mut sdi).unwrap();

        wm.close_window("c", &mut sdi).unwrap();
        wm.close_window("b", &mut sdi).unwrap();
        wm.close_window("a", &mut sdi).unwrap();

        assert_eq!(wm.window_count(), 0);
        assert!(wm.active_window().is_none());
        // All SDI objects should be cleaned up.
        assert!(!sdi.contains("a.frame"));
        assert!(!sdi.contains("b.frame"));
        assert!(!sdi.contains("c.frame"));
    }

    #[test]
    fn workflow_minimize_then_focus_other() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // Minimize w2 (the active one).
        wm.minimize_window("w2", &mut sdi).unwrap();
        // After minimizing active, w1 should become active.
        assert_eq!(wm.active_window(), Some("w1"));

        // Focus w1 explicitly (should be no-op since already active).
        wm.focus_window("w1", &mut sdi).unwrap();
        assert_eq!(wm.active_window(), Some("w1"));

        // Restore w2.
        wm.restore_window("w2", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w2").unwrap().state, WindowState::Normal);
    }

    #[test]
    fn workflow_dialog_with_app_windows() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("app1"), &mut sdi).unwrap();
        wm.create_window(&dialog_config("dlg1"), &mut sdi).unwrap();
        wm.create_window(&app_config("app2"), &mut sdi).unwrap();

        assert_eq!(wm.window_count(), 3);
        // Close the dialog.
        wm.close_window("dlg1", &mut sdi).unwrap();
        assert_eq!(wm.window_count(), 2);
        assert!(!sdi.contains("dlg1.frame"));
        // The remaining windows should be fine.
        assert!(sdi.contains("app1.frame"));
        assert!(sdi.contains("app2.frame"));
    }

    #[test]
    fn workflow_move_then_resize_then_maximize_restore() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        // Move.
        wm.move_window("w", 30, 20, &mut sdi).unwrap();
        let moved_x = sdi.get("w.frame").unwrap().x;
        let moved_y = sdi.get("w.frame").unwrap().y;

        // Resize.
        wm.resize_window("w", 300, 200, &mut sdi).unwrap();
        let resized_w = sdi.get("w.frame").unwrap().w;
        let resized_h = sdi.get("w.frame").unwrap().h;

        // Maximize and restore should return to the resized geometry.
        wm.maximize_window("w", &mut sdi).unwrap();
        wm.restore_window("w", &mut sdi).unwrap();

        assert_eq!(sdi.get("w.frame").unwrap().x, moved_x);
        assert_eq!(sdi.get("w.frame").unwrap().y, moved_y);
        assert_eq!(sdi.get("w.frame").unwrap().w, resized_w);
        assert_eq!(sdi.get("w.frame").unwrap().h, resized_h);
    }

    // ---- Titlebar gradient tests ----

    #[test]
    fn gradient_set_on_creation() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            titlebar_gradient: true,
            titlebar_gradient_top: Some(Color::rgb(100, 100, 200)),
            titlebar_gradient_bottom: Some(Color::rgb(50, 50, 100)),
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let tb = sdi.get("w1.titlebar").unwrap();
        assert_eq!(tb.gradient_top, Some(Color::rgb(100, 100, 200)));
        assert_eq!(tb.gradient_bottom, Some(Color::rgb(50, 50, 100)));
    }

    #[test]
    fn gradient_switches_on_focus_change() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            titlebar_gradient: true,
            titlebar_gradient_top: Some(Color::rgb(100, 100, 200)),
            titlebar_gradient_bottom: Some(Color::rgb(50, 50, 100)),
            titlebar_inactive_gradient_top: Some(Color::rgb(80, 80, 80)),
            titlebar_inactive_gradient_bottom: Some(Color::rgb(40, 40, 40)),
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // w2 is active, w1 is inactive.
        let w1_tb = sdi.get("w1.titlebar").unwrap();
        assert_eq!(w1_tb.gradient_top, Some(Color::rgb(80, 80, 80)));
        assert_eq!(w1_tb.gradient_bottom, Some(Color::rgb(40, 40, 40)));

        let w2_tb = sdi.get("w2.titlebar").unwrap();
        assert_eq!(w2_tb.gradient_top, Some(Color::rgb(100, 100, 200)));
        assert_eq!(w2_tb.gradient_bottom, Some(Color::rgb(50, 50, 100)));

        // Focus w1.
        wm.focus_window("w1", &mut sdi).unwrap();
        let w1_tb = sdi.get("w1.titlebar").unwrap();
        assert_eq!(w1_tb.gradient_top, Some(Color::rgb(100, 100, 200)));
        let w2_tb = sdi.get("w2.titlebar").unwrap();
        assert_eq!(w2_tb.gradient_top, Some(Color::rgb(80, 80, 80)));
    }

    #[test]
    fn no_gradient_when_disabled() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            titlebar_gradient: false,
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let tb = sdi.get("w1.titlebar").unwrap();
        assert_eq!(tb.gradient_top, None);
        assert_eq!(tb.gradient_bottom, None);
    }

    #[test]
    fn gradient_auto_derives_from_base_color() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            titlebar_gradient: true,
            titlebar_gradient_top: None, // Will auto-derive.
            titlebar_gradient_bottom: None,
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let tb = sdi.get("w1.titlebar").unwrap();
        // Auto-derived: top = lighten(active_color, 0.1), bottom = active_color.
        assert!(tb.gradient_top.is_some());
        assert!(tb.gradient_bottom.is_some());
        assert_eq!(tb.gradient_bottom, Some(wm.theme().titlebar_active_color));
    }

    #[test]
    fn gradient_cleared_on_focus_when_disabled() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            titlebar_gradient: false,
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();

        // Focus w1 -- gradients should remain None.
        wm.focus_window("w1", &mut sdi).unwrap();
        let w1_tb = sdi.get("w1.titlebar").unwrap();
        assert_eq!(w1_tb.gradient_top, None);
        let w2_tb = sdi.get("w2.titlebar").unwrap();
        assert_eq!(w2_tb.gradient_top, None);
    }

    // ---- Always-on-top and modal tests ----

    fn aot_config(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(10),
            y: Some(10),
            width: 200,
            height: 150,
            window_type: WindowType::AppWindow,
            always_on_top: true,
            modal: false,
        }
    }

    fn modal_config(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(50),
            y: Some(50),
            width: 200,
            height: 100,
            window_type: WindowType::Dialog,
            always_on_top: false,
            modal: true,
        }
    }

    #[test]
    fn always_on_top_stays_above_normal() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&aot_config("aot"), &mut sdi).unwrap();
        wm.create_window(&app_config("normal"), &mut sdi).unwrap();

        // Even though 'normal' was created after 'aot', 'aot' should be above.
        let aot_idx = wm.windows.iter().position(|w| w.id == "aot").unwrap();
        let normal_idx = wm.windows.iter().position(|w| w.id == "normal").unwrap();
        assert!(
            aot_idx > normal_idx,
            "always_on_top should be above normal: aot={aot_idx}, normal={normal_idx}"
        );
    }

    #[test]
    fn focus_normal_stays_below_always_on_top() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("n1"), &mut sdi).unwrap();
        wm.create_window(&aot_config("aot"), &mut sdi).unwrap();
        wm.create_window(&app_config("n2"), &mut sdi).unwrap();

        // Focus n1 -- it should move to the top of normal group, but below aot.
        wm.focus_window("n1", &mut sdi).unwrap();
        let aot_idx = wm.windows.iter().position(|w| w.id == "aot").unwrap();
        let n1_idx = wm.windows.iter().position(|w| w.id == "n1").unwrap();
        assert!(
            n1_idx < aot_idx,
            "normal window should stay below always_on_top: n1={n1_idx}, aot={aot_idx}"
        );
    }

    #[test]
    fn modal_stays_above_always_on_top() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("normal"), &mut sdi).unwrap();
        wm.create_window(&aot_config("aot"), &mut sdi).unwrap();
        wm.create_window(&modal_config("modal"), &mut sdi).unwrap();

        let modal_idx = wm.windows.iter().position(|w| w.id == "modal").unwrap();
        let aot_idx = wm.windows.iter().position(|w| w.id == "aot").unwrap();
        assert!(
            modal_idx > aot_idx,
            "modal should be above always_on_top: modal={modal_idx}, aot={aot_idx}"
        );
    }

    #[test]
    fn modal_creates_overlay() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("bg"), &mut sdi).unwrap();
        assert!(!sdi.contains(MODAL_OVERLAY_ID));

        wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();
        assert!(sdi.contains(MODAL_OVERLAY_ID));
    }

    #[test]
    fn modal_overlay_removed_on_close() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("bg"), &mut sdi).unwrap();
        wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();
        assert!(sdi.contains(MODAL_OVERLAY_ID));

        wm.close_window("dlg", &mut sdi).unwrap();
        assert!(!sdi.contains(MODAL_OVERLAY_ID));
    }

    #[test]
    fn has_modal_tracks_state() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(!wm.has_modal());

        wm.create_window(&modal_config("m1"), &mut sdi).unwrap();
        assert!(wm.has_modal());

        wm.close_window("m1", &mut sdi).unwrap();
        assert!(!wm.has_modal());
    }

    #[test]
    fn topmost_modal_returns_correct_window() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert_eq!(wm.topmost_modal(), None);

        wm.create_window(&modal_config("m1"), &mut sdi).unwrap();
        assert_eq!(wm.topmost_modal(), Some("m1"));

        wm.create_window(&modal_config("m2"), &mut sdi).unwrap();
        assert_eq!(wm.topmost_modal(), Some("m2"));

        wm.close_window("m2", &mut sdi).unwrap();
        assert_eq!(wm.topmost_modal(), Some("m1"));
    }

    #[test]
    fn modal_overlay_persists_with_multiple_modals() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&modal_config("m1"), &mut sdi).unwrap();
        wm.create_window(&modal_config("m2"), &mut sdi).unwrap();

        // Close one modal -- overlay should persist.
        wm.close_window("m1", &mut sdi).unwrap();
        assert!(sdi.contains(MODAL_OVERLAY_ID));

        // Close last modal -- overlay removed.
        wm.close_window("m2", &mut sdi).unwrap();
        assert!(!sdi.contains(MODAL_OVERLAY_ID));
    }

    #[test]
    fn close_all_removes_modal_overlay() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("bg"), &mut sdi).unwrap();
        wm.create_window(&modal_config("m"), &mut sdi).unwrap();
        assert!(sdi.contains(MODAL_OVERLAY_ID));

        wm.close_all(&mut sdi);
        assert!(!sdi.contains(MODAL_OVERLAY_ID));
    }

    // ---- Additional cascading / tiling tests ----

    #[test]
    fn cascade_wraps_when_near_edge() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(200, 200);
        // Create enough windows to trigger cascade wrapping.
        for i in 0..10 {
            let config = WindowConfig {
                id: format!("w{i}"),
                title: format!("W{i}"),
                x: None,
                y: None,
                width: 50,
                height: 30,
                window_type: WindowType::AppWindow,
                always_on_top: false,
                modal: false,
            };
            wm.create_window(&config, &mut sdi).unwrap();
        }
        // All windows should exist and be within screen bounds.
        assert_eq!(wm.window_count(), 10);
        for i in 0..10 {
            let win = wm.get_window(&format!("w{i}")).unwrap();
            assert!(win.x >= 0);
            assert!(win.y >= 0);
        }
    }

    #[test]
    fn explicit_position_overrides_cascade() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let config = WindowConfig {
            id: "fixed".to_string(),
            title: "Fixed".to_string(),
            x: Some(100),
            y: Some(200),
            width: 150,
            height: 100,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        wm.create_window(&config, &mut sdi).unwrap();
        let win = wm.get_window("fixed").unwrap();
        assert_eq!(win.x, 100);
        assert_eq!(win.y, 200);
    }

    #[test]
    fn close_all_empties_wm() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("a"), &mut sdi).unwrap();
        wm.create_window(&app_config("b"), &mut sdi).unwrap();
        wm.create_window(&app_config("c"), &mut sdi).unwrap();
        wm.close_all(&mut sdi);
        assert_eq!(wm.window_count(), 0);
        assert!(wm.active_window().is_none());
    }

    // ---- State transition tests ----

    #[test]
    fn maximize_then_minimize_then_restore() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        let orig_w = wm.get_window("w").unwrap().outer_w;

        wm.maximize_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Maximized);

        // Minimize from maximized.
        wm.minimize_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Minimized);

        // Restore should go back to pre-maximize geometry.
        wm.restore_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);
        assert_eq!(wm.get_window("w").unwrap().outer_w, orig_w);
    }

    #[test]
    fn double_maximize_is_idempotent() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        wm.maximize_window("w", &mut sdi).unwrap();
        let first_w = wm.get_window("w").unwrap().outer_w;
        let first_h = wm.get_window("w").unwrap().outer_h;

        wm.maximize_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().outer_w, first_w);
        assert_eq!(wm.get_window("w").unwrap().outer_h, first_h);
    }

    #[test]
    fn restore_normal_window_is_noop() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        let orig_x = wm.get_window("w").unwrap().x;
        let orig_y = wm.get_window("w").unwrap().y;

        // Restore on a normal window should not change anything.
        wm.restore_window("w", &mut sdi).unwrap();
        assert_eq!(wm.get_window("w").unwrap().x, orig_x);
        assert_eq!(wm.get_window("w").unwrap().y, orig_y);
        assert_eq!(wm.get_window("w").unwrap().state, WindowState::Normal);
    }

    #[test]
    fn move_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.move_window("nope", 10, 10, &mut sdi).is_err());
    }

    #[test]
    fn resize_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.resize_window("nope", 100, 100, &mut sdi).is_err());
    }

    #[test]
    fn focus_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.focus_window("nope", &mut sdi).is_err());
    }

    #[test]
    fn minimize_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.minimize_window("nope", &mut sdi).is_err());
    }

    #[test]
    fn maximize_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.maximize_window("nope", &mut sdi).is_err());
    }

    #[test]
    fn restore_nonexistent_window_fails() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        assert!(wm.restore_window("nope", &mut sdi).is_err());
    }

    #[test]
    fn set_theme_replaces_theme() {
        let mut wm = WindowManager::new(800, 600);
        let new_theme = WmTheme {
            titlebar_height: 40,
            ..WmTheme::default()
        };
        wm.set_theme(new_theme);
        assert_eq!(wm.theme().titlebar_height, 40);
    }

    #[test]
    fn cycle_focus_forward() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();
        wm.create_window(&app_config("w3"), &mut sdi).unwrap();
        // w3 is on top. Cycling forward brings w1 to top.
        let focused = wm.cycle_focus(true, &mut sdi);
        assert!(focused.is_some());
    }

    #[test]
    fn cycle_focus_backward() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        wm.create_window(&app_config("w2"), &mut sdi).unwrap();
        wm.create_window(&app_config("w3"), &mut sdi).unwrap();
        let focused = wm.cycle_focus(false, &mut sdi);
        assert!(focused.is_some());
    }

    #[test]
    fn cycle_focus_single_window() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        // With one window, cycle should return the same window.
        let focused = wm.cycle_focus(true, &mut sdi);
        assert_eq!(focused.as_deref(), Some("w1"));
    }

    #[test]
    fn cycle_focus_no_windows() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let focused = wm.cycle_focus(true, &mut sdi);
        assert!(focused.is_none());
    }

    #[test]
    fn maximize_with_insets() {
        let mut sdi = SdiRegistry::new();
        let theme = WmTheme {
            maximize_top_inset: 20,
            maximize_bottom_inset: 30,
            ..WmTheme::default()
        };
        let mut wm = WindowManager::with_theme(800, 600, theme);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();
        wm.maximize_window("w", &mut sdi).unwrap();

        let win = wm.get_window("w").unwrap();
        assert_eq!(win.x, 0);
        assert_eq!(win.y, 20);
        assert_eq!(win.outer_w, 800);
        assert_eq!(win.outer_h, 550); // 600 - 20 - 30
    }

    #[test]
    fn enter_fullscreen_expands_to_screen() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        wm.enter_fullscreen("w", &mut sdi).unwrap();

        let win = wm.get_window("w").unwrap();
        assert!(win.fullscreen_kiosk);
        assert_eq!(win.x, 0);
        assert_eq!(win.y, 0);
        assert_eq!(win.outer_w, 480);
        assert_eq!(win.outer_h, 272);
        assert!(win.kiosk_saved_geometry.is_some());
    }

    #[test]
    fn exit_fullscreen_restores_geometry() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        let orig = wm.get_window("w").unwrap();
        let orig_x = orig.x;
        let orig_y = orig.y;
        let orig_w = orig.outer_w;
        let orig_h = orig.outer_h;

        wm.enter_fullscreen("w", &mut sdi).unwrap();
        wm.exit_fullscreen("w", &mut sdi).unwrap();

        let win = wm.get_window("w").unwrap();
        assert!(!win.fullscreen_kiosk);
        assert_eq!(win.x, orig_x);
        assert_eq!(win.y, orig_y);
        assert_eq!(win.outer_w, orig_w);
        assert_eq!(win.outer_h, orig_h);
        assert!(win.kiosk_saved_geometry.is_none());
    }

    #[test]
    fn enter_fullscreen_hides_decorations() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        wm.enter_fullscreen("w", &mut sdi).unwrap();

        // Frame and titlebar should be hidden.
        assert!(!sdi.get("w.frame").unwrap().visible);
        assert!(!sdi.get("w.titlebar").unwrap().visible);
        // Content should remain visible.
        assert!(sdi.get("w.content").unwrap().visible);
    }

    #[test]
    fn exit_fullscreen_shows_decorations() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        wm.enter_fullscreen("w", &mut sdi).unwrap();
        wm.exit_fullscreen("w", &mut sdi).unwrap();

        assert!(sdi.get("w.frame").unwrap().visible);
        assert!(sdi.get("w.titlebar").unwrap().visible);
        assert!(sdi.get("w.content").unwrap().visible);
    }

    #[test]
    fn has_fullscreen_kiosk_tracks_state() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        assert!(!wm.has_fullscreen_kiosk());
        wm.enter_fullscreen("w", &mut sdi).unwrap();
        assert!(wm.has_fullscreen_kiosk());
        wm.exit_fullscreen("w", &mut sdi).unwrap();
        assert!(!wm.has_fullscreen_kiosk());
    }

    #[test]
    fn enter_fullscreen_content_rect_is_full_screen() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        wm.enter_fullscreen("w", &mut sdi).unwrap();

        let win = wm.get_window("w").unwrap();
        let (cx, cy, cw, ch) = win.content_rect(wm.theme());
        assert_eq!((cx, cy, cw, ch), (0, 0, 480, 272));
    }

    #[test]
    fn kiosk_from_maximized_preserves_normal_geometry() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();

        let orig = wm.get_window("w").unwrap();
        let orig_x = orig.x;
        let orig_y = orig.y;
        let orig_w = orig.outer_w;
        let orig_h = orig.outer_h;

        // Maximize → saved_geometry holds the Normal geometry.
        wm.maximize_window("w", &mut sdi).unwrap();
        assert!(wm.get_window("w").unwrap().saved_geometry.is_some());

        // Enter kiosk → should NOT clobber saved_geometry.
        wm.enter_fullscreen("w", &mut sdi).unwrap();
        assert!(wm.get_window("w").unwrap().saved_geometry.is_some());
        assert!(wm.get_window("w").unwrap().kiosk_saved_geometry.is_some());

        // Exit kiosk → back to maximized bounds, saved_geometry still intact.
        wm.exit_fullscreen("w", &mut sdi).unwrap();
        assert!(wm.get_window("w").unwrap().saved_geometry.is_some());

        // Restore from maximized → back to original Normal geometry.
        wm.restore_window("w", &mut sdi).unwrap();
        let win = wm.get_window("w").unwrap();
        assert_eq!(win.x, orig_x);
        assert_eq!(win.y, orig_y);
        assert_eq!(win.outer_w, orig_w);
        assert_eq!(win.outer_h, orig_h);
    }
}
