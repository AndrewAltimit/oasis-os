//! Window manager: lifecycle, drag/resize, focus, and input dispatch.
//!
//! The WM creates and manipulates groups of SDI objects to simulate windowed
//! interfaces. It is a consumer of the SDI API -- SDI remains a flat scene
//! graph with no concept of grouping or hierarchy.

use oasis_sdi::SdiRegistry;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::{OasisError, Result};
use oasis_types::input::InputEvent;

use super::hit_test::{ButtonKind, HitRegion, ResizeEdge, hit_test};
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

/// Active drag/resize operation.
#[derive(Debug, Clone)]
enum DragState {
    /// Dragging a window by its titlebar.
    Moving {
        window_id: WindowId,
        /// Cursor position at drag start.
        start_cursor_x: i32,
        start_cursor_y: i32,
        /// Window position at drag start.
        start_win_x: i32,
        start_win_y: i32,
    },
    /// Resizing a window by a handle.
    Resizing {
        window_id: WindowId,
        edge: ResizeEdge,
        start_cursor_x: i32,
        start_cursor_y: i32,
        start_geometry: Geometry,
    },
}

/// Minimum window content width during resize.
const MIN_RESIZE_W: u32 = 80;

/// Minimum window content height during resize.
const MIN_RESIZE_H: u32 = 60;

/// Cascade offset between newly created windows.
const CASCADE_OFFSET: i32 = 24;

/// Distance in pixels for edge snapping during drag.
const SNAP_DISTANCE: i32 = 8;

/// Minimum visible pixels of a window titlebar at screen edges.
/// At least 20px of the titlebar must remain on-screen so the user
/// can always grab and drag the window back.
const MIN_VISIBLE: i32 = 20;

/// SDI object name for the semi-transparent modal backdrop.
const MODAL_OVERLAY_ID: &str = "__wm_modal_overlay";

/// The window manager.
///
/// Manages a list of windows ordered by z-depth (last = topmost).
/// Processes input events through hit testing and a drag/resize state machine.
pub struct WindowManager {
    /// Windows in z-order (last = topmost).
    windows: Vec<Window>,
    /// Visual theme.
    theme: WmTheme,
    /// Cascade position for the next window.
    next_cascade_x: i32,
    next_cascade_y: i32,
    /// Screen dimensions (for maximize and cascade wrapping).
    screen_w: u32,
    screen_h: u32,
    /// Active window id (receives keyboard input).
    active_window: Option<WindowId>,
    /// Current drag/resize operation.
    drag: Option<DragState>,
    /// Currently hovered window button (for hover color feedback).
    hover_button: Option<(WindowId, ButtonKind)>,
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
            return Err(OasisError::Wm(format!(
                "window already exists: {}",
                config.id
            )));
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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

        window.outer_w = new_outer_w;
        window.outer_h = new_outer_h;

        // Reposition all SDI objects based on new geometry.
        self.update_sdi_positions(id, sdi);

        Ok(())
    }

    /// Bring a window to the front (topmost z-order).
    pub fn focus_window(&mut self, id: &str, sdi: &mut SdiRegistry) -> Result<()> {
        if !self.windows.iter().any(|w| w.id == id) {
            return Err(OasisError::Wm(format!("window not found: {id}")));
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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

        if !window.has_minimize_button() {
            return Err(OasisError::Wm(format!(
                "window type does not support minimize: {id}"
            )));
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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

        if !window.has_maximize_button() {
            return Err(OasisError::Wm(format!(
                "window type does not support maximize: {id}"
            )));
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
            .ok_or_else(|| OasisError::Wm(format!("window not found: {id}")))?;

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
        // First draw all SDI objects (frames, titlebars, etc.).
        sdi.draw(backend)?;

        // Then draw clipped content for each visible window.
        for window in &self.windows {
            if window.state == WindowState::Minimized {
                continue;
            }
            let (cx, cy, cw, ch) = window.content_rect(&self.theme);
            if cw == 0 || ch == 0 {
                continue;
            }
            backend.set_clip_rect(cx, cy, cw, ch)?;
            draw_content(&window.id, cx, cy, cw, ch, backend)?;
            backend.reset_clip_rect()?;
        }

        Ok(())
    }

    // -- Internal methods --

    fn handle_click(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) -> WmEvent {
        let region = hit_test(&self.windows, x, y, &self.theme);

        // If a modal window exists, only allow clicks on the topmost modal.
        if let Some(modal_id) = self.topmost_modal() {
            let hit_id = match &region {
                HitRegion::TitlebarButton(id, _)
                | HitRegion::Titlebar(id)
                | HitRegion::ResizeHandle(id, _)
                | HitRegion::Content(id, _, _) => Some(id.as_str()),
                HitRegion::Desktop => None,
            };
            if hit_id != Some(modal_id) {
                return WmEvent::None;
            }
        }

        match region {
            HitRegion::TitlebarButton(id, ButtonKind::Close) => {
                let _ = self.close_window(&id, sdi);
                WmEvent::WindowClosed(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Minimize) => {
                let _ = self.minimize_window(&id, sdi);
                WmEvent::WindowMinimized(id)
            },
            HitRegion::TitlebarButton(id, ButtonKind::Maximize) => {
                // Toggle: if maximized, restore; otherwise maximize.
                let is_maximized = self
                    .windows
                    .iter()
                    .find(|w| w.id == id)
                    .map(|w| w.state == WindowState::Maximized)
                    .unwrap_or(false);

                if is_maximized {
                    let _ = self.restore_window(&id, sdi);
                    WmEvent::WindowRestored(id)
                } else {
                    self.focus_window_internal(&id, sdi);
                    let _ = self.maximize_window(&id, sdi);
                    WmEvent::WindowMaximized(id)
                }
            },
            HitRegion::Titlebar(id) => {
                self.focus_window_internal(&id, sdi);
                // Start drag if the window is draggable.
                if let Some(window) = self.windows.iter().find(|w| w.id == id)
                    && window.is_draggable()
                {
                    self.drag = Some(DragState::Moving {
                        window_id: id.clone(),
                        start_cursor_x: x,
                        start_cursor_y: y,
                        start_win_x: window.x,
                        start_win_y: window.y,
                    });
                }
                WmEvent::WindowFocused(id)
            },
            HitRegion::ResizeHandle(id, edge) => {
                self.focus_window_internal(&id, sdi);
                if let Some(window) = self.windows.iter().find(|w| w.id == id) {
                    self.drag = Some(DragState::Resizing {
                        window_id: id.clone(),
                        edge,
                        start_cursor_x: x,
                        start_cursor_y: y,
                        start_geometry: Geometry {
                            x: window.x,
                            y: window.y,
                            w: window.outer_w,
                            h: window.outer_h,
                        },
                    });
                }
                WmEvent::WindowFocused(id)
            },
            HitRegion::Content(id, lx, ly) => {
                self.focus_window_internal(&id, sdi);
                WmEvent::ContentClick(id, lx, ly)
            },
            HitRegion::Desktop => {
                self.active_window = None;
                WmEvent::DesktopClick(x, y)
            },
        }
    }

    fn handle_cursor_move(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) -> WmEvent {
        let drag = match self.drag.clone() {
            Some(d) => d,
            None => {
                // No drag active -- check for button hover.
                self.update_button_hover(x, y, sdi);
                return WmEvent::None;
            },
        };

        match drag {
            DragState::Moving {
                ref window_id,
                start_cursor_x,
                start_cursor_y,
                start_win_x,
                start_win_y,
            } => {
                let raw_x = start_win_x + (x - start_cursor_x);
                let raw_y = start_win_y + (y - start_cursor_y);

                // Get dimensions for snap/clamp before mutable borrow.
                let dims = self
                    .windows
                    .iter()
                    .find(|w| w.id == *window_id)
                    .map(|w| (w.outer_w, w.outer_h));

                if let Some((outer_w, outer_h)) = dims {
                    let (sx, sy) =
                        snap_to_edges(raw_x, raw_y, outer_w, outer_h, self.screen_w, self.screen_h);
                    let (cx, cy) = clamp_position(
                        sx,
                        sy,
                        outer_w,
                        self.screen_w,
                        self.screen_h,
                        self.theme.titlebar_height,
                    );

                    if let Some(window) = self.windows.iter_mut().find(|w| w.id == *window_id) {
                        let dx = cx - window.x;
                        let dy = cy - window.y;
                        window.x = cx;
                        window.y = cy;

                        for suffix in window.sdi_suffixes() {
                            let name = window.sdi_name(suffix);
                            if let Ok(obj) = sdi.get_mut(&name) {
                                obj.x += dx;
                                obj.y += dy;
                            }
                        }
                    }
                }
                WmEvent::WindowMoved(window_id.clone())
            },
            DragState::Resizing {
                ref window_id,
                edge,
                start_cursor_x,
                start_cursor_y,
                start_geometry,
            } => {
                let dx = x - start_cursor_x;
                let dy = y - start_cursor_y;

                let (mut new_x, mut new_y, mut new_w, mut new_h) =
                    compute_resize(start_geometry, edge, dx, dy, &self.theme);

                // Clamp resize to screen bounds.
                let sw = self.screen_w as i32;
                let sh = self.screen_h as i32;
                if new_x < 0 {
                    new_w = new_w.saturating_sub((-new_x) as u32);
                    new_x = 0;
                }
                if new_y < 0 {
                    new_h = new_h.saturating_sub((-new_y) as u32);
                    new_y = 0;
                }
                if new_x + new_w as i32 > sw {
                    new_w = (sw - new_x).max(0) as u32;
                }
                if new_y + new_h as i32 > sh {
                    new_h = (sh - new_y).max(0) as u32;
                }

                if let Some(window) = self.windows.iter_mut().find(|w| w.id == *window_id) {
                    window.x = new_x;
                    window.y = new_y;
                    window.outer_w = new_w;
                    window.outer_h = new_h;
                }

                self.update_sdi_positions(window_id.as_str(), sdi);
                WmEvent::WindowResized(window_id.clone())
            },
        }
    }

    fn handle_release(&mut self) -> WmEvent {
        self.hover_button = None;
        if let Some(drag) = self.drag.take() {
            let id = match drag {
                DragState::Moving { window_id, .. } => window_id,
                DragState::Resizing { window_id, .. } => window_id,
            };
            return WmEvent::WindowMoved(id);
        }
        WmEvent::None
    }

    /// Move a window to the top of its z-order group and update SDI z-ordering.
    /// Groups: normal < always_on_top < modal.
    fn focus_window_internal(&mut self, id: &str, sdi: &mut SdiRegistry) {
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

    /// Create all SDI objects for a window.
    fn create_sdi_objects(&self, window: &Window, sdi: &mut SdiRegistry) {
        let theme = &self.theme;
        let use_radius = theme.frame_border_radius > 0;
        let use_shadow = theme.frame_shadow_level > 0;

        // Frame (background).
        if window.sdi_suffixes().contains(&"frame") {
            let obj = sdi.create(window.sdi_name("frame"));
            obj.x = window.x;
            obj.y = window.y;
            obj.w = window.outer_w;
            obj.h = window.outer_h;
            obj.color = theme.frame_color;
            if use_radius {
                obj.border_radius = Some(theme.frame_border_radius);
            }
            if use_shadow {
                obj.shadow_level = Some(theme.frame_shadow_level);
            }
            if theme.border_width > 0 {
                obj.stroke_width = Some(theme.border_width as u16);
                obj.stroke_color = Some(theme.frame_color);
            }
        }

        // Titlebar.
        if let Some((tx, ty, tw, th)) = window.titlebar_rect(theme) {
            if window.sdi_suffixes().contains(&"titlebar") {
                let obj = sdi.create(window.sdi_name("titlebar"));
                obj.x = tx;
                obj.y = ty;
                obj.w = tw;
                obj.h = th;
                obj.color = theme.titlebar_active_color;
                if theme.titlebar_radius > 0 {
                    obj.border_radius = Some(theme.titlebar_radius);
                }
                if theme.titlebar_gradient {
                    use oasis_types::color::lighten;
                    obj.gradient_top = Some(
                        theme
                            .titlebar_gradient_top
                            .unwrap_or_else(|| lighten(theme.titlebar_active_color, 0.1)),
                    );
                    obj.gradient_bottom = Some(
                        theme
                            .titlebar_gradient_bottom
                            .unwrap_or(theme.titlebar_active_color),
                    );
                }
            }

            // Title text.
            if window.sdi_suffixes().contains(&"title_text") {
                let (text_x, avail_w) = window
                    .title_text_x(theme)
                    .unwrap_or((tx + 4, tw.saturating_sub(8)));
                let text_y = ty + (th as i32 - theme.titlebar_font_size as i32) / 2 - 1;
                let obj = sdi.create(window.sdi_name("title_text"));
                obj.x = text_x;
                obj.y = text_y;
                obj.w = avail_w;
                obj.h = th;
                obj.text = Some(window.title.clone());
                obj.font_size = theme.titlebar_font_size;
                obj.text_color = theme.titlebar_text_color;
                obj.color = Color::rgba(0, 0, 0, 0);

                // Title text shadow (Tier 3).
                if theme.title_text_shadow {
                    let sobj = sdi.create(window.sdi_name("title_shadow"));
                    sobj.x = text_x + 1;
                    sobj.y = text_y + 1;
                    sobj.w = avail_w;
                    sobj.h = th;
                    sobj.text = Some(window.title.clone());
                    sobj.font_size = theme.titlebar_font_size;
                    sobj.text_color = theme.title_text_shadow_color;
                    sobj.color = Color::rgba(0, 0, 0, 0);
                }
            }

            // Separator (Tier 2): 1px bar at titlebar bottom edge.
            if theme.separator_enabled {
                let obj = sdi.create(window.sdi_name("separator"));
                obj.x = tx;
                obj.y = ty + th as i32 - 1;
                obj.w = tw;
                obj.h = 1;
                obj.color = theme.separator_color;
            }
        }

        let glyph_font_size = (theme.button_size as u16).min(12);

        // Close button.
        if let Some((bx, by, bw, bh)) = window.close_btn_rect(theme) {
            let obj = sdi.create(window.sdi_name("btn_close"));
            obj.x = bx;
            obj.y = by;
            obj.w = bw;
            obj.h = bh;
            obj.color = theme.btn_close_color;
            if theme.button_radius > 0 {
                obj.border_radius = Some(theme.button_radius);
            }
            let gobj = sdi.create(window.sdi_name("btn_close_glyph"));
            gobj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
            gobj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            gobj.w = bw;
            gobj.h = bh;
            gobj.text = Some(theme.glyph_close.clone());
            gobj.font_size = glyph_font_size;
            gobj.text_color = theme.glyph_close_color;
            gobj.color = Color::rgba(0, 0, 0, 0);
        }

        // Minimize button.
        if let Some((bx, by, bw, bh)) = window.minimize_btn_rect(theme) {
            let obj = sdi.create(window.sdi_name("btn_minimize"));
            obj.x = bx;
            obj.y = by;
            obj.w = bw;
            obj.h = bh;
            obj.color = theme.btn_minimize_color;
            if theme.button_radius > 0 {
                obj.border_radius = Some(theme.button_radius);
            }
            let gobj = sdi.create(window.sdi_name("btn_minimize_glyph"));
            gobj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
            gobj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            gobj.w = bw;
            gobj.h = bh;
            gobj.text = Some(theme.glyph_minimize.clone());
            gobj.font_size = glyph_font_size;
            gobj.text_color = theme.glyph_minimize_color;
            gobj.color = Color::rgba(0, 0, 0, 0);
        }

        // Maximize button.
        if let Some((bx, by, bw, bh)) = window.maximize_btn_rect(theme) {
            let obj = sdi.create(window.sdi_name("btn_maximize"));
            obj.x = bx;
            obj.y = by;
            obj.w = bw;
            obj.h = bh;
            obj.color = theme.btn_maximize_color;
            if theme.button_radius > 0 {
                obj.border_radius = Some(theme.button_radius);
            }
            let gobj = sdi.create(window.sdi_name("btn_maximize_glyph"));
            gobj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
            gobj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            gobj.w = bw;
            gobj.h = bh;
            gobj.text = Some(theme.glyph_maximize.clone());
            gobj.font_size = glyph_font_size;
            gobj.text_color = theme.glyph_maximize_color;
            gobj.color = Color::rgba(0, 0, 0, 0);
        }

        // Content area (stays sharp for clip rect compatibility).
        {
            let (cx, cy, cw, ch) = window.content_rect(theme);
            let obj = sdi.create(window.sdi_name("content"));
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
            obj.color = theme.content_bg_color;

            // Content stroke overlay (Tier 3).
            if theme.content_stroke_width > 0 {
                let sobj = sdi.create(window.sdi_name("content_stroke"));
                sobj.x = cx;
                sobj.y = cy;
                sobj.w = cw;
                sobj.h = ch;
                sobj.color = Color::rgba(0, 0, 0, 0);
                sobj.stroke_width = Some(theme.content_stroke_width);
                sobj.stroke_color = Some(theme.content_stroke_color);
            }
        }
    }

    /// Destroy all SDI objects for a window.
    fn destroy_sdi_objects(&self, window: &Window, sdi: &mut SdiRegistry) {
        for suffix in window.sdi_suffixes() {
            let name = window.sdi_name(suffix);
            let _ = sdi.destroy(&name);
        }
    }

    /// Reposition all SDI objects based on window's current geometry.
    fn update_sdi_positions(&self, id: &str, sdi: &mut SdiRegistry) {
        let window = match self.windows.iter().find(|w| w.id == id) {
            Some(w) => w,
            None => return,
        };

        let theme = &self.theme;

        // Frame.
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("frame")) {
            obj.x = window.x;
            obj.y = window.y;
            obj.w = window.outer_w;
            obj.h = window.outer_h;
        }

        let glyph_font_size = (theme.button_size as u16).min(12);

        // Titlebar.
        if let Some((tx, ty, tw, th)) = window.titlebar_rect(theme) {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("titlebar")) {
                obj.x = tx;
                obj.y = ty;
                obj.w = tw;
                obj.h = th;
            }
            let (text_x, avail_w) = window
                .title_text_x(theme)
                .unwrap_or((tx + 4, tw.saturating_sub(8)));
            let text_y = ty + (th as i32 - theme.titlebar_font_size as i32) / 2 - 1;
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("title_text")) {
                obj.x = text_x;
                obj.y = text_y;
                obj.w = avail_w;
                obj.h = th;
            }
            // Title shadow.
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("title_shadow")) {
                obj.x = text_x + 1;
                obj.y = text_y + 1;
                obj.w = avail_w;
                obj.h = th;
            }
            // Separator.
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("separator")) {
                obj.x = tx;
                obj.y = ty + th as i32 - 1;
                obj.w = tw;
                obj.h = 1;
            }
        }

        // Buttons + glyphs.
        if let Some((bx, by, bw, bh)) = window.close_btn_rect(theme) {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_close")) {
                obj.x = bx;
                obj.y = by;
                obj.w = bw;
                obj.h = bh;
            }
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_close_glyph")) {
                obj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
                obj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            }
        }
        if let Some((bx, by, bw, bh)) = window.minimize_btn_rect(theme) {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_minimize")) {
                obj.x = bx;
                obj.y = by;
                obj.w = bw;
                obj.h = bh;
            }
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_minimize_glyph")) {
                obj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
                obj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            }
        }
        if let Some((bx, by, bw, bh)) = window.maximize_btn_rect(theme) {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_maximize")) {
                obj.x = bx;
                obj.y = by;
                obj.w = bw;
                obj.h = bh;
            }
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("btn_maximize_glyph")) {
                obj.x = bx + (bw as i32 - glyph_font_size as i32) / 2;
                obj.y = by + (bh as i32 - glyph_font_size as i32) / 2;
            }
        }

        // Content.
        let (cx, cy, cw, ch) = window.content_rect(theme);
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("content")) {
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
        }
        // Content stroke.
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("content_stroke")) {
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
        }
    }

    /// Update button hover state. Sets hover color on the hovered button and
    /// restores the base color on the previously hovered button.
    fn update_button_hover(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) {
        let region = hit_test(&self.windows, x, y, &self.theme);
        let new_hover = match &region {
            HitRegion::TitlebarButton(id, kind) => Some((id.clone(), *kind)),
            _ => None,
        };

        // If hover changed, restore old button color.
        if self.hover_button != new_hover {
            if let Some((ref old_id, ref old_kind)) = self.hover_button {
                let (suffix, base_color) = match old_kind {
                    ButtonKind::Close => ("btn_close", self.theme.btn_close_color),
                    ButtonKind::Minimize => ("btn_minimize", self.theme.btn_minimize_color),
                    ButtonKind::Maximize => ("btn_maximize", self.theme.btn_maximize_color),
                };
                let name = format!("{old_id}.{suffix}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.color = base_color;
                }
            }
            // Apply new hover color.
            if let Some((ref new_id, ref new_kind)) = new_hover {
                let (suffix, hover_color) = match new_kind {
                    ButtonKind::Close => ("btn_close", self.theme.btn_close_hover),
                    ButtonKind::Minimize => ("btn_minimize", self.theme.btn_minimize_hover),
                    ButtonKind::Maximize => ("btn_maximize", self.theme.btn_maximize_hover),
                };
                let name = format!("{new_id}.{suffix}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.color = hover_color;
                }
            }
            self.hover_button = new_hover;
        }
    }

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

    /// Show the semi-transparent overlay behind modal windows.
    fn show_modal_overlay(&self, sdi: &mut SdiRegistry) {
        if sdi.contains(MODAL_OVERLAY_ID) {
            return;
        }
        let obj = sdi.create(MODAL_OVERLAY_ID.to_string());
        obj.x = 0;
        obj.y = 0;
        obj.w = self.screen_w;
        obj.h = self.screen_h;
        obj.color = self.theme.modal_overlay_color;
    }

    /// Hide and destroy the modal overlay.
    fn hide_modal_overlay(&self, sdi: &mut SdiRegistry) {
        let _ = sdi.destroy(MODAL_OVERLAY_ID);
    }
}

/// Compute new geometry after a resize drag.
fn compute_resize(
    start: Geometry,
    edge: ResizeEdge,
    dx: i32,
    dy: i32,
    theme: &WmTheme,
) -> (i32, i32, u32, u32) {
    let min_w = MIN_RESIZE_W + theme.border_width * 2;
    let min_h = MIN_RESIZE_H + theme.titlebar_height + theme.border_width * 2;

    let mut x = start.x;
    let mut y = start.y;
    let mut w = start.w;
    let mut h = start.h;

    match edge {
        ResizeEdge::East => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
        },
        ResizeEdge::West => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
        },
        ResizeEdge::South => {
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::North => {
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
        ResizeEdge::SouthEast => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::SouthWest => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
            h = (start.h as i32 + dy).max(min_h as i32) as u32;
        },
        ResizeEdge::NorthEast => {
            w = (start.w as i32 + dx).max(min_w as i32) as u32;
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
        ResizeEdge::NorthWest => {
            let new_w = (start.w as i32 - dx).max(min_w as i32) as u32;
            x = start.x + (start.w as i32 - new_w as i32);
            w = new_w;
            let new_h = (start.h as i32 - dy).max(min_h as i32) as u32;
            y = start.y + (start.h as i32 - new_h as i32);
            h = new_h;
        },
    }

    (x, y, w, h)
}

/// Clamp a window position to keep the titlebar partially visible on screen.
fn clamp_position(
    x: i32,
    y: i32,
    outer_w: u32,
    screen_w: u32,
    screen_h: u32,
    titlebar_height: u32,
) -> (i32, i32) {
    let sw = screen_w as i32;
    let sh = screen_h as i32;
    let tb_h = titlebar_height as i32;
    let cx = x.max(MIN_VISIBLE - outer_w as i32).min(sw - MIN_VISIBLE);
    let cy = y.max(0).min(sh - tb_h);
    (cx, cy)
}

/// Snap position to screen edges if within [`SNAP_DISTANCE`].
fn snap_to_edges(
    x: i32,
    y: i32,
    outer_w: u32,
    outer_h: u32,
    screen_w: u32,
    screen_h: u32,
) -> (i32, i32) {
    let sw = screen_w as i32;
    let sh = screen_h as i32;
    let mut sx = x;
    let mut sy = y;
    if sx.abs() < SNAP_DISTANCE {
        sx = 0;
    } else if (sx + outer_w as i32 - sw).abs() < SNAP_DISTANCE {
        sx = sw - outer_w as i32;
    }
    if sy.abs() < SNAP_DISTANCE {
        sy = 0;
    } else if (sy + outer_h as i32 - sh).abs() < SNAP_DISTANCE {
        sy = sh - outer_h as i32;
    }
    (sx, sy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::WindowType;

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
    fn click_content_returns_local_coords() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let (cx, cy, _cw, _ch) = wm.get_window("w1").unwrap().content_rect(&wm.theme);
        let event = InputEvent::PointerClick {
            x: cx + 10,
            y: cy + 20,
        };
        let result = wm.handle_input(&event, &mut sdi);
        assert_eq!(result, WmEvent::ContentClick(WindowId::from("w1"), 10, 20));
    }

    #[test]
    fn click_desktop_returns_desktop_event() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let event = InputEvent::PointerClick { x: 700, y: 500 };
        let result = wm.handle_input(&event, &mut sdi);
        assert_eq!(result, WmEvent::DesktopClick(700, 500));
    }

    #[test]
    fn click_close_button_closes_window() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let (bx, by, bw, bh) = win.close_btn_rect(&wm.theme).unwrap();
        let event = InputEvent::PointerClick {
            x: bx + bw as i32 / 2,
            y: by + bh as i32 / 2,
        };
        let result = wm.handle_input(&event, &mut sdi);
        assert_eq!(result, WmEvent::WindowClosed(WindowId::from("w1")));
        assert_eq!(wm.window_count(), 0);
    }

    #[test]
    fn drag_moves_window() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();
        let orig_x = win.x;
        let orig_y = win.y;

        // Click on titlebar.
        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 5,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );

        // Drag.
        wm.handle_input(
            &InputEvent::CursorMove {
                x: tx + 55,
                y: ty + th as i32 / 2 + 30,
            },
            &mut sdi,
        );

        let win = wm.get_window("w1").unwrap();
        assert_eq!(win.x, orig_x + 50);
        assert_eq!(win.y, orig_y + 30);

        // Release.
        wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
        assert!(wm.drag.is_none());
    }

    #[test]
    fn resize_east() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let orig_w = win.outer_w;
        let right_edge = win.x + win.outer_w as i32 - 2;
        let mid_y = win.y + win.outer_h as i32 / 2;

        // Click on east resize handle.
        wm.handle_input(
            &InputEvent::PointerClick {
                x: right_edge,
                y: mid_y,
            },
            &mut sdi,
        );

        // Drag east by 40px.
        wm.handle_input(
            &InputEvent::CursorMove {
                x: right_edge + 40,
                y: mid_y,
            },
            &mut sdi,
        );

        let win = wm.get_window("w1").unwrap();
        assert_eq!(win.outer_w, orig_w + 40);

        wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
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
    fn resize_respects_minimum_size() {
        let theme = WmTheme::default();
        let start = Geometry {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        };
        // Try shrinking way past minimum.
        let (_, _, w, h) = compute_resize(start, ResizeEdge::SouthEast, -400, -400, &theme);
        let min_w = MIN_RESIZE_W + theme.border_width * 2;
        let min_h = MIN_RESIZE_H + theme.titlebar_height + theme.border_width * 2;
        assert_eq!(w, min_w);
        assert_eq!(h, min_h);
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

    // ---- Screen bounds enforcement tests ----

    #[test]
    fn clamp_position_keeps_titlebar_visible() {
        let tb_h = WmTheme::default().titlebar_height;
        // Far left: at least MIN_VISIBLE px visible.
        let (cx, _) = clamp_position(-500, 100, 200, 480, 272, tb_h);
        assert_eq!(cx, MIN_VISIBLE - 200);
        // Far right.
        let (cx, _) = clamp_position(500, 100, 200, 480, 272, tb_h);
        assert_eq!(cx, 480 - MIN_VISIBLE);
        // Above screen.
        let (_, cy) = clamp_position(100, -100, 200, 480, 272, tb_h);
        assert_eq!(cy, 0);
        // Below screen.
        let (_, cy) = clamp_position(100, 500, 200, 480, 272, tb_h);
        assert_eq!(cy, 272 - tb_h as i32);
    }

    #[test]
    fn snap_to_edges_near_left() {
        let (sx, _) = snap_to_edges(5, 100, 200, 150, 480, 272);
        assert_eq!(sx, 0);
    }

    #[test]
    fn snap_to_edges_near_top() {
        let (_, sy) = snap_to_edges(100, 5, 200, 150, 480, 272);
        assert_eq!(sy, 0);
    }

    #[test]
    fn snap_to_edges_near_right() {
        // Right edge = 275 + 200 = 475. Screen = 480. Diff = 5 < 8.
        let (sx, _) = snap_to_edges(275, 100, 200, 150, 480, 272);
        assert_eq!(sx, 280); // 480 - 200
    }

    #[test]
    fn snap_to_edges_near_bottom() {
        // Bottom edge = 119 + 150 = 269. Screen = 272. Diff = 3 < 8.
        let (_, sy) = snap_to_edges(100, 119, 200, 150, 480, 272);
        assert_eq!(sy, 122); // 272 - 150
    }

    #[test]
    fn snap_to_edges_no_snap_when_far() {
        let (sx, sy) = snap_to_edges(100, 50, 200, 150, 480, 272);
        assert_eq!(sx, 100);
        assert_eq!(sy, 50);
    }

    #[test]
    fn drag_clamps_to_screen_top() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 30,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );
        wm.handle_input(
            &InputEvent::CursorMove {
                x: tx + 30,
                y: -500,
            },
            &mut sdi,
        );

        let win = wm.get_window("w1").unwrap();
        assert!(win.y >= 0, "window y={} should be >= 0", win.y);
    }

    #[test]
    fn drag_clamps_to_screen_bottom() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 30,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );
        wm.handle_input(&InputEvent::CursorMove { x: tx + 30, y: 800 }, &mut sdi);

        let win = wm.get_window("w1").unwrap();
        let max_y = 272 - wm.theme().titlebar_height as i32;
        assert!(win.y <= max_y, "window y={} should be <= {}", win.y, max_y);
    }

    #[test]
    fn drag_clamps_to_screen_left() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let outer_w = win.outer_w;
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 30,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );
        wm.handle_input(
            &InputEvent::CursorMove {
                x: -1000,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );

        let win = wm.get_window("w1").unwrap();
        let min_x = MIN_VISIBLE - outer_w as i32;
        assert!(win.x >= min_x, "window x={} should be >= {}", win.x, min_x);
    }

    #[test]
    fn drag_clamps_to_screen_right() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();

        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 30,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );
        wm.handle_input(
            &InputEvent::CursorMove {
                x: 2000,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );

        let win = wm.get_window("w1").unwrap();
        let max_x = 480 - MIN_VISIBLE;
        assert!(win.x <= max_x, "window x={} should be <= {}", win.x, max_x);
    }

    #[test]
    fn move_window_clamps_to_bounds() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        wm.move_window("w1", 2000, 2000, &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        assert!(win.x <= 480 - MIN_VISIBLE);
        let max_y = 272 - wm.theme().titlebar_height as i32;
        assert!(win.y <= max_y);
    }

    #[test]
    fn move_window_clamps_negative() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        wm.move_window("w1", -2000, -2000, &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        assert!(win.y >= 0);
        assert!(win.x >= MIN_VISIBLE - win.outer_w as i32);
    }

    #[test]
    fn resize_clamps_east_to_screen() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let right_edge = win.x + win.outer_w as i32 - 2;
        let mid_y = win.y + win.outer_h as i32 / 2;

        wm.handle_input(
            &InputEvent::PointerClick {
                x: right_edge,
                y: mid_y,
            },
            &mut sdi,
        );
        wm.handle_input(&InputEvent::CursorMove { x: 1000, y: mid_y }, &mut sdi);

        let win = wm.get_window("w1").unwrap();
        assert!(
            win.x + win.outer_w as i32 <= 480,
            "right edge {} should be <= 480",
            win.x + win.outer_w as i32,
        );
    }

    #[test]
    fn resize_clamps_south_to_screen() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(480, 272);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();

        let win = wm.get_window("w1").unwrap();
        let bottom_edge = win.y + win.outer_h as i32 - 2;
        let mid_x = win.x + win.outer_w as i32 / 2;

        wm.handle_input(
            &InputEvent::PointerClick {
                x: mid_x,
                y: bottom_edge,
            },
            &mut sdi,
        );
        wm.handle_input(&InputEvent::CursorMove { x: mid_x, y: 1000 }, &mut sdi);

        let win = wm.get_window("w1").unwrap();
        assert!(
            win.y + win.outer_h as i32 <= 272,
            "bottom edge {} should be <= 272",
            win.y + win.outer_h as i32,
        );
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
    fn modal_blocks_click_on_normal_window() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("bg"), &mut sdi).unwrap();
        wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

        // Click on the background window's content area.
        let bg = wm.get_window("bg").unwrap();
        let (cx, cy, _cw, _ch) = bg.content_rect(wm.theme());
        let event = InputEvent::PointerClick {
            x: cx + 10,
            y: cy + 10,
        };
        let result = wm.handle_input(&event, &mut sdi);
        assert_eq!(
            result,
            WmEvent::None,
            "click on non-modal should be blocked"
        );
    }

    #[test]
    fn modal_allows_click_on_itself() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("bg"), &mut sdi).unwrap();
        wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

        // Click on the modal window's content area.
        let dlg = wm.get_window("dlg").unwrap();
        let (cx, cy, _cw, _ch) = dlg.content_rect(wm.theme());
        let event = InputEvent::PointerClick {
            x: cx + 10,
            y: cy + 10,
        };
        let result = wm.handle_input(&event, &mut sdi);
        assert!(
            matches!(result, WmEvent::ContentClick(ref id, _, _) if id == "dlg"),
            "click on modal should go through, got {result:?}"
        );
    }

    #[test]
    fn modal_blocks_desktop_click() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&modal_config("dlg"), &mut sdi).unwrap();

        let event = InputEvent::PointerClick { x: 700, y: 500 };
        let result = wm.handle_input(&event, &mut sdi);
        assert_eq!(result, WmEvent::None, "desktop click should be blocked");
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

    #[test]
    fn close_all_clears_drag_state() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w"), &mut sdi).unwrap();
        // Start a drag.
        let win = wm.get_window("w").unwrap();
        let (tx, ty, _tw, th) = win.titlebar_rect(&wm.theme).unwrap();
        wm.handle_input(
            &InputEvent::PointerClick {
                x: tx + 5,
                y: ty + th as i32 / 2,
            },
            &mut sdi,
        );
        wm.close_all(&mut sdi);
        assert!(wm.drag.is_none());
    }

    // ---- Edge snapping tests ----

    #[test]
    fn snap_exact_edge() {
        let (sx, sy) = snap_to_edges(0, 0, 200, 150, 480, 272);
        assert_eq!(sx, 0);
        assert_eq!(sy, 0);
    }

    #[test]
    fn snap_within_threshold() {
        // 7px from left edge.
        let (sx, _) = snap_to_edges(7, 50, 200, 150, 480, 272);
        assert_eq!(sx, 0);
    }

    #[test]
    fn no_snap_just_outside_threshold() {
        // 9px from left edge -- just outside 8px threshold.
        let (sx, _) = snap_to_edges(9, 50, 200, 150, 480, 272);
        assert_eq!(sx, 9);
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
    fn release_without_drag_returns_none() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let event = wm.handle_input(&InputEvent::PointerRelease { x: 0, y: 0 }, &mut sdi);
        assert_eq!(event, WmEvent::None);
    }

    #[test]
    fn cursor_move_without_drag_returns_none() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let event = wm.handle_input(&InputEvent::CursorMove { x: 50, y: 50 }, &mut sdi);
        assert_eq!(event, WmEvent::None);
    }

    #[test]
    fn button_press_returns_none() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        let event = wm.handle_input(
            &InputEvent::ButtonPress(oasis_types::input::Button::Confirm),
            &mut sdi,
        );
        assert_eq!(event, WmEvent::None);
    }

    #[test]
    fn resize_all_edges() {
        let theme = WmTheme::default();
        let start = Geometry {
            x: 50,
            y: 50,
            w: 200,
            h: 200,
        };
        // East
        let (_, _, w, _) = compute_resize(start, ResizeEdge::East, 30, 0, &theme);
        assert_eq!(w, 230);
        // West: x moves left, w grows
        let (x, _, w, _) = compute_resize(start, ResizeEdge::West, -20, 0, &theme);
        assert_eq!(w, 220);
        assert_eq!(x, 30);
        // South
        let (_, _, _, h) = compute_resize(start, ResizeEdge::South, 0, 40, &theme);
        assert_eq!(h, 240);
        // North: y moves up, h grows
        let (_, y, _, h) = compute_resize(start, ResizeEdge::North, 0, -10, &theme);
        assert_eq!(h, 210);
        assert_eq!(y, 40);
        // NorthEast
        let (_, y, w, h) = compute_resize(start, ResizeEdge::NorthEast, 20, -10, &theme);
        assert_eq!(w, 220);
        assert_eq!(h, 210);
        assert_eq!(y, 40);
        // NorthWest
        let (x, y, w, h) = compute_resize(start, ResizeEdge::NorthWest, -15, -10, &theme);
        assert_eq!(w, 215);
        assert_eq!(h, 210);
        assert_eq!(x, 35);
        assert_eq!(y, 40);
        // SouthWest
        let (x, _, w, h) = compute_resize(start, ResizeEdge::SouthWest, -20, 30, &theme);
        assert_eq!(w, 220);
        assert_eq!(h, 230);
        assert_eq!(x, 30);
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
}
