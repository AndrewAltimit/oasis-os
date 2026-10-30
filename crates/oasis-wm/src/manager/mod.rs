//! Window manager: core struct, accessors, geometry, and input dispatch.
//!
//! The WM creates and manipulates groups of SDI objects to simulate windowed
//! interfaces. It is a consumer of the SDI API -- SDI remains a flat scene
//! graph with no concept of grouping or hierarchy.
//!
//! Method implementations are split across several files:
//! - `lifecycle.rs` — Window creation, destruction, state transitions
//! - `focus.rs` — Focus management and z-order operations
//! - `render.rs` — Drawing with clip rects
//! - `sdi_objects.rs` — SDI object create/destroy/update
//! - `drag_resize/` — Drag/resize state machine
//! - `manager/input.rs` — Input dispatch
//! - `window_ops.rs` — Snapping, keyboard window management, tiling
//! - `motion.rs` — Open/close/minimize/restore animations
//! - `manager/query.rs` — Query/lookup methods and accessors

mod input;
mod query;

use oasis_sdi::SdiRegistry;
use oasis_types::error::{OasisError, Result, WmError};

use super::animation::AnimationManager;
use super::drag_resize::{DragState, clamp_position};
use super::hit_test::ButtonKind;
use super::snap::SnapManager;
use super::tiling::{TilingLayout, TilingManager};
use super::window::{Geometry, Window, WindowId, WmTheme};

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
pub(crate) const CASCADE_OFFSET: i32 = 24;

/// SDI object name for the semi-transparent modal backdrop.
pub(crate) const MODAL_OVERLAY_ID: &str = "__wm_modal_overlay";

/// Last titlebar click record used to detect double-clicks.
///
/// Stored as `(window_id, click_time, x, y)`. A follow-up titlebar click
/// within [`DOUBLE_CLICK_WINDOW`] and [`DOUBLE_CLICK_RADIUS`] on the same
/// window toggles maximize/restore.
///
/// `web_time::Instant` delegates to `std::time::Instant` on native and to
/// `performance.now()` in the browser — `std::time::Instant::now()` aborts
/// on `wasm32-unknown-unknown`.
pub(crate) type TitlebarClickStamp = (WindowId, web_time::Instant, i32, i32);

/// Maximum interval between two clicks to register as a double-click.
pub(crate) const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

/// Maximum movement between two clicks to register as a double-click (px).
pub(crate) const DOUBLE_CLICK_RADIUS: i32 = 6;

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
    /// Most recent titlebar click, used to detect double-clicks that toggle
    /// maximize/restore.
    pub(crate) last_titlebar_click: Option<TitlebarClickStamp>,
    /// Drag-to-edge snap zone detection and preview state.
    pub(crate) snap: SnapManager,
    /// Whether drag-to-edge snapping is enabled.
    pub(crate) snap_enabled: bool,
    /// Tiling layout engine used by [`WindowManager::cycle_tiling`].
    pub(crate) tiling: TilingManager,
    /// Active tiling layout (`None` = windows float freely).
    pub(crate) tiling_layout: Option<TilingLayout>,
    /// Open/close/minimize/restore animations (see `motion.rs`).
    pub(crate) anim: AnimationManager,
    /// Whether window animations are enabled (off by default).
    pub(crate) motion_enabled: bool,
    /// Closed windows whose SDI objects stay on screen until their close
    /// animation finishes. Not interactive.
    pub(crate) closing: Vec<Window>,
    /// Interpolated geometry of windows mid-animation, used for drawing.
    pub(crate) anim_visual: Vec<(WindowId, Geometry)>,
    /// Time of the previous `tick_animations` call.
    pub(crate) anim_last_tick: Option<web_time::Instant>,
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
            last_titlebar_click: None,
            snap: SnapManager::new(),
            snap_enabled: true,
            tiling: TilingManager::new(),
            tiling_layout: None,
            anim: {
                let mut anim = AnimationManager::new();
                anim.set_reduced_motion(true);
                anim
            },
            motion_enabled: false,
            closing: Vec::new(),
            anim_visual: Vec::new(),
            anim_last_tick: None,
        }
    }

    /// Create a new window manager with a custom theme.
    pub fn with_theme(screen_w: u32, screen_h: u32, theme: WmTheme) -> Self {
        Self {
            theme,
            ..Self::new(screen_w, screen_h)
        }
    }

    // -- Geometry methods --

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
        self.cancel_drag_of(id);
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
}

#[cfg(test)]
mod tests;
