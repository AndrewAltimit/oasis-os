//! The `App` trait: extensible application interface.
//!
//! Each application (File Manager, Settings, Music Player, etc.) implements
//! this trait. `AppRunner` delegates to the active `App` implementation.

use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

use crate::AppAction;
use crate::layout::AppLayout;

/// Trait for applications that can be launched from the dashboard.
///
/// Each app owns its state and implements input handling, rendering (both
/// SDI fullscreen and windowed), and VFS IPC. Apps are created via
/// `AppRunner::launch()` and dispatched through the runner.
pub trait App: std::fmt::Debug + Send {
    /// Display title shown in the title bar.
    fn title(&self) -> &str;

    /// VFS path of the app.
    fn path(&self) -> &str;

    /// Handle a button press. Returns the resulting action.
    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction;

    /// Handle a text character input (typing). Default is no-op.
    fn handle_text_input(&mut self, _ch: char) {}

    /// Handle backspace (delete last character). Default is no-op.
    fn handle_backspace(&mut self) {}

    /// Handle a raw keyboard key press (shortcuts and navigation keys such
    /// as Ctrl+S, Delete, Home/End, PageUp/PageDown, F-keys).
    ///
    /// Called by keyboard hosts (desktop SDL, WASM, FFI) for every key-down
    /// while this app has focus, *before* the gamepad-style event the key
    /// maps to (e.g. Enter -> [`Button::Confirm`] via `handle_input`,
    /// Backspace -> `handle_backspace`). Never called on PSP / gamepad-only
    /// hosts, so every feature must stay reachable through `handle_input`.
    ///
    /// Return `Some(action)` when the key was consumed: the host applies
    /// `action` and drops the key's gamepad-style twin so the press is not
    /// handled twice. Return `None` (the default) to let the key fall
    /// through to the legacy handlers unchanged.
    ///
    /// Plain printable keys also arrive as [`Self::handle_text_input`];
    /// text-entry apps should type from there and only claim `Key::Char`
    /// here for modifier combinations (`mods.has_command()`).
    fn handle_key(&mut self, _key: &Key, _mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        None
    }

    /// Whether this app is currently a text-entry target.
    ///
    /// While a text-accepting app has focus, keys that type a character
    /// (letters, digits, Space) are treated purely as typing: the host
    /// suppresses the gamepad-style shortcuts they would otherwise trigger
    /// (Space -> Triangle, Q/E -> shoulder triggers / desktop switching).
    /// May depend on internal state (e.g. only while a search box is
    /// active). Default is `false`.
    fn accepts_text(&self) -> bool {
        false
    }

    /// Handle a click/tap in the app content area.
    ///
    /// `lx`/`ly` are local coordinates within the content area.
    /// Returns `AppAction::None` by default.
    fn handle_click(
        &mut self,
        _lx: i32,
        _ly: i32,
        _cw: u32,
        _ch: u32,
        _fullscreen: bool,
    ) -> AppAction {
        AppAction::None
    }

    /// Render to SDI registry (fullscreen mode).
    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme);

    /// Render directly to backend (windowed mode).
    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()>;

    /// Hide all SDI objects created by this app.
    fn hide_sdi(&self, sdi: &mut SdiRegistry);

    /// Take any pending VFS IPC request (path, data).
    fn take_pending_request(&mut self) -> Option<(String, String)> {
        None
    }

    /// Peek at pending VFS IPC request without consuming it.
    fn peek_pending_request(&self) -> Option<&(String, String)> {
        None
    }

    /// Refresh display from VFS state (called each frame when visible).
    ///
    /// Default implementation is a no-op.
    fn refresh(&mut self, _vfs: &dyn Vfs) {}

    /// Apply VFS mutations the app has queued (deletes, renames, copies,
    /// binary saves, ...).
    ///
    /// Every other hook only sees a shared `&dyn Vfs`, so apps that need to
    /// change the file system queue the work from input handlers and the
    /// host drains it here with mutable access, once per frame for every
    /// open app. Returns `true` when anything was applied so the host can
    /// re-sync cached state. Default is a no-op returning `false`.
    fn apply_vfs_ops(&mut self, _vfs: &mut dyn Vfs) -> bool {
        false
    }

    /// Advance time-driven state by `dt_ms` milliseconds of wall time.
    ///
    /// Hosts call this once per frame for every open app (windowed or
    /// fullscreen, focused or not), *including frames whose redraw is
    /// elided*, so game loops, slideshows and timers run at a fixed
    /// wall-clock rate independent of the frame rate and of input.
    /// Apps should accumulate `dt_ms` rather than count calls.
    ///
    /// Return `true` when the tick changed anything this app draws; the
    /// host then schedules a redraw of the app's window. Default is a
    /// no-op returning `false`.
    fn tick(&mut self, _dt_ms: u32, _vfs: &dyn Vfs) -> bool {
        false
    }

    /// Take a pending request to close this app.
    ///
    /// For closes decided *outside* an input handler -- e.g. "Save &
    /// close", whose write only happens later in [`Self::apply_vfs_ops`].
    /// Hosts poll this once per frame for every open app, after
    /// `apply_vfs_ops` and [`Self::tick`], and close the app exactly as
    /// if an input hook had returned [`AppAction::Exit`]. Returning `true`
    /// consumes the request. Default is `false` (never self-closes).
    fn take_close_request(&mut self) -> bool {
        false
    }

    /// The user asked to close the app through the host's window chrome
    /// (a titlebar close button) rather than through the app's own input.
    ///
    /// Return [`AppAction::Exit`] to close now; anything else keeps the
    /// app open -- e.g. an editor with unsaved changes raises its
    /// "Save / Discard / Cancel" prompt and closes later through its own
    /// input or [`Self::take_close_request`]. Default: close immediately.
    fn on_close_requested(&mut self, _vfs: &dyn Vfs) -> AppAction {
        AppAction::Exit
    }

    /// Whether this app needs every frame drawn (continuous animation,
    /// video, embedded content that repaints outside the app's control).
    ///
    /// Hosts with idle-frame elision skip redraws while nothing visible
    /// changed; state changes caused by input, [`Self::tick`] returning
    /// `true` or VFS operations are tracked by the host, so only content
    /// that changes on its own every frame needs to return `true` here.
    /// Default is `false`.
    fn wants_frame(&self) -> bool {
        false
    }

    /// Content lines for the app (used by generic scroll/render logic).
    fn lines(&self) -> &[String];

    /// Current browse directory, if the app browses files.
    fn browse_dir(&self) -> Option<&str> {
        None
    }

    /// Path of the file currently being viewed, if any.
    fn viewing_file(&self) -> Option<&str> {
        None
    }

    /// Downcast to `&dyn Any` for app-specific access.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Downcast to `&mut dyn Any` for mutable app-specific access.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Shared scrollable content state used by many apps.
///
/// Encapsulates the common pattern of a title, scrollable line list,
/// cursor position, and optional file browsing/viewing. Apps embed
/// this struct and delegate navigation to its methods.
#[derive(Debug, Clone)]
pub struct ContentState {
    /// App display title.
    pub title: String,
    /// App VFS path.
    pub app_path: String,
    /// Content lines.
    pub lines: Vec<String>,
    /// Scroll offset (first visible line index).
    pub scroll: usize,
    /// Cursor position (relative to visible area).
    pub cursor: usize,
    /// Smooth selection for lerp animation.
    pub visual_selected: f32,
    /// Cached max visible lines (updated each frame).
    pub cached_max_visible: usize,
    /// Cached title bar height in pixels (updated each frame from the active
    /// theme). Used by click handlers to map click Y back to a line index
    /// without hardcoding renderer metrics.
    pub cached_title_bar_height: u32,
    /// Cached content line height in pixels (updated each frame from the
    /// active theme). Mirrors the value used by `draw_content_windowed`.
    pub cached_line_h: u32,
    /// Current browse directory.
    pub browse_dir: Option<String>,
    /// File currently being viewed.
    pub viewing_file: Option<String>,
    /// Pending VFS IPC request (path, data).
    pub pending_vfs_request: Option<(String, String)>,
}

/// Maximum visible lines fallback for 480x272.
const DEFAULT_MAX_VISIBLE: usize = 13;
/// Default title bar height fallback used until the first `update_layout`
/// call populates the real value from the active theme. Matches the common
/// desktop skin metric.
const DEFAULT_TITLE_BAR_HEIGHT: u32 = 20;
/// Default content line height fallback used until the first `update_layout`
/// call. Matches `AppLayout::compute` for the default desktop skin.
const DEFAULT_LINE_H: u32 = 14;

impl ContentState {
    /// Create new content state for an app.
    pub fn new(title: &str, path: &str) -> Self {
        Self {
            title: title.to_string(),
            app_path: path.to_string(),
            lines: Vec::new(),
            scroll: 0,
            cursor: 0,
            visual_selected: 0.0,
            cached_max_visible: DEFAULT_MAX_VISIBLE,
            cached_title_bar_height: DEFAULT_TITLE_BAR_HEIGHT,
            cached_line_h: DEFAULT_LINE_H,
            browse_dir: None,
            viewing_file: None,
            pending_vfs_request: None,
        }
    }

    /// Number of currently visible content lines.
    pub fn visible_count(&self) -> usize {
        let remaining = self.lines.len().saturating_sub(self.scroll);
        remaining.min(self.cached_max_visible)
    }

    /// Navigate cursor up.
    pub fn navigate_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        } else if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    /// Navigate cursor down.
    pub fn navigate_down(&mut self) {
        let visible = self.visible_count();
        if self.cursor + 1 < visible {
            self.cursor += 1;
        } else if self.scroll + self.cached_max_visible < self.lines.len() {
            self.scroll += 1;
        }
    }

    /// Update cached layout from theme.
    pub fn update_layout(&mut self, at: &ActiveTheme) {
        // Match the floor `draw_content_windowed` applies so click-to-line
        // mapping uses the same row height the renderer drew.
        let line_h = at.terminal_line_height.max(12);
        let layout = AppLayout::compute(at, 14);
        // Re-derive `max_visible` using the clamped `line_h` so the scroll
        // viewport bound and the click-to-row mapping agree. `AppLayout`
        // internally uses `max(1)` which diverges from the windowed
        // renderer's `max(12)` whenever the skin has a tiny line height.
        self.cached_max_visible = (layout.usable_h / line_h).max(1) as usize;
        self.cached_title_bar_height = at.app.title_bar_height;
        self.cached_line_h = line_h;
    }

    /// Advance the smooth visual selection animation.
    pub fn animate_selection(&mut self, lerp_speed: f32) {
        self.visual_selected += (self.cursor as f32 - self.visual_selected) * lerp_speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_state_new() {
        let cs = ContentState::new("Test App", "/apps/test");
        assert_eq!(cs.title, "Test App");
        assert_eq!(cs.app_path, "/apps/test");
        assert!(cs.lines.is_empty());
        assert_eq!(cs.scroll, 0);
        assert_eq!(cs.cursor, 0);
    }

    #[test]
    fn visible_count_empty() {
        let cs = ContentState::new("Test", "/apps/test");
        assert_eq!(cs.visible_count(), 0);
    }

    #[test]
    fn visible_count_partial() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into(), "c".into()];
        cs.cached_max_visible = 10;
        assert_eq!(cs.visible_count(), 3);
    }

    #[test]
    fn visible_count_overflow() {
        let mut cs = ContentState::new("Test", "/apps/test");
        for i in 0..20 {
            cs.lines.push(format!("line {i}"));
        }
        cs.cached_max_visible = 10;
        assert_eq!(cs.visible_count(), 10);
    }

    #[test]
    fn visible_count_with_scroll() {
        let mut cs = ContentState::new("Test", "/apps/test");
        for i in 0..20 {
            cs.lines.push(format!("line {i}"));
        }
        cs.cached_max_visible = 10;
        cs.scroll = 15;
        assert_eq!(cs.visible_count(), 5);
    }

    #[test]
    fn navigate_up_from_top() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into()];
        cs.navigate_up();
        assert_eq!(cs.cursor, 0);
        assert_eq!(cs.scroll, 0);
    }

    #[test]
    fn navigate_up_moves_cursor() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into(), "c".into()];
        cs.cached_max_visible = 10;
        cs.cursor = 2;
        cs.navigate_up();
        assert_eq!(cs.cursor, 1);
    }

    #[test]
    fn navigate_up_scrolls() {
        let mut cs = ContentState::new("Test", "/apps/test");
        for i in 0..20 {
            cs.lines.push(format!("line {i}"));
        }
        cs.cached_max_visible = 10;
        cs.scroll = 5;
        cs.cursor = 0;
        cs.navigate_up();
        assert_eq!(cs.scroll, 4);
        assert_eq!(cs.cursor, 0);
    }

    #[test]
    fn navigate_down_moves_cursor() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into(), "c".into()];
        cs.cached_max_visible = 10;
        cs.navigate_down();
        assert_eq!(cs.cursor, 1);
    }

    #[test]
    fn navigate_down_scrolls() {
        let mut cs = ContentState::new("Test", "/apps/test");
        for i in 0..20 {
            cs.lines.push(format!("line {i}"));
        }
        cs.cached_max_visible = 10;
        cs.cursor = 9;
        cs.navigate_down();
        assert_eq!(cs.scroll, 1);
        assert_eq!(cs.cursor, 9);
    }

    #[test]
    fn navigate_down_at_end_noop() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into()];
        cs.cached_max_visible = 10;
        cs.cursor = 1;
        cs.navigate_down();
        assert_eq!(cs.cursor, 1);
        assert_eq!(cs.scroll, 0);
    }

    #[test]
    fn animate_selection_moves_toward_cursor() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.cursor = 5;
        cs.visual_selected = 0.0;
        cs.animate_selection(0.5);
        assert!(cs.visual_selected > 0.0);
        assert!(cs.visual_selected < 5.0);
    }

    #[test]
    fn pending_request_default_none() {
        let cs = ContentState::new("Test", "/apps/test");
        assert!(cs.pending_vfs_request.is_none());
    }
}
