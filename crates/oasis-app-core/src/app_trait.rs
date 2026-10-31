//! The `App` trait: extensible application interface.
//!
//! Each application (File Manager, Settings, Music Player, etc.) implements
//! this trait. `AppRunner` delegates to the active `App` implementation.

use std::any::Any;

use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

use crate::AppAction;

/// Downcasting support for [`App`] trait objects.
///
/// Implemented automatically for every [`App`] (blanket impl), so apps never
/// write `as_any` themselves. The blanket impl is deliberately restricted to
/// `T: App`: a `Box<dyn App>` is itself `Any`, and an unrestricted impl
/// would let `boxed.as_any()` resolve to the *box*, so every downcast would
/// silently fail.
pub trait AsAny: Any {
    /// Downcast to `&dyn Any` for app-specific access.
    fn as_any(&self) -> &dyn Any;
    /// Downcast to `&mut dyn Any` for mutable app-specific access.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: App> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A VFS IPC request an app asks its host to perform: write `data` to
/// `path` (hosts act directly on paths they recognise, such as a TV Guide
/// `tune_url` or a Video Embed search).
///
/// Apps queue requests from their input / tick handlers and hand them over
/// in [`App::drain_requests`]; hosts drain every open app once per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRequest {
    /// VFS path the request targets (e.g. `/var/radio/request`).
    pub path: String,
    /// Request payload.
    pub data: String,
}

impl AppRequest {
    /// A request to write `data` to `path`.
    pub fn new(path: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            data: data.into(),
        }
    }
}

/// Upper bound on the rows one wheel event moves in the default
/// [`App::handle_scroll`] (a flung trackpad can report large deltas).
pub const MAX_WHEEL_STEPS: u32 = 16;

/// Trait for applications that can be launched from the dashboard.
///
/// Each app owns its state and implements input handling, drawing into
/// its window ([`Self::draw_windowed`]) and VFS IPC. Apps are created via
/// `AppRunner::launch()` and dispatched through the runner.
///
/// # Input routing
///
/// One physical input reaches an app through these hooks, in this order:
///
/// | Input | Hooks |
/// |---|---|
/// | Keyboard key | [`Self::handle_key`]; if that returns `None`, the key's gamepad twin ([`Self::handle_input`] / [`Self::handle_backspace`]), skipped for typing keys while [`Self::accepts_text`]; then [`Self::handle_text_input`] for printable keys |
/// | Gamepad button (PSP, OSK) | [`Self::handle_input`] |
/// | Typed text (IME, OSK) | [`Self::handle_text_input`] / [`Self::handle_backspace`] |
/// | Pointer click | [`Self::layout`], then [`Self::handle_click`] |
/// | Mouse wheel | [`Self::handle_scroll`] |
///
/// # Per-frame order
///
/// Every frame, for every open app: [`Self::drain_requests`],
/// [`Self::apply_vfs_ops`], [`Self::tick`], [`Self::take_close_request`].
/// Then, for each window that is drawn: [`Self::layout`] and
/// [`Self::draw_windowed`]. A closed app gets [`Self::release_resources`]
/// once before it is dropped.
pub trait App: AsAny + std::fmt::Debug + Send {
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

    /// Mouse wheel over the app's window: `delta` notches, negative =
    /// toward the top (earlier content), positive = toward the bottom.
    /// Return `true` when anything the app draws changed.
    ///
    /// The default moves the selection one row per notch by repeating
    /// [`Button::Up`] / [`Button::Down`] through [`Self::handle_input`]
    /// (at most [`MAX_WHEEL_STEPS`] per event), which suits list-style
    /// apps. Apps where Up / Down means something else (steering a game,
    /// moving a canvas cursor) override this.
    fn handle_scroll(&mut self, delta: i32, vfs: &dyn Vfs) -> bool {
        let button = if delta < 0 { Button::Up } else { Button::Down };
        let steps = delta.unsigned_abs().min(MAX_WHEEL_STEPS);
        for _ in 0..steps {
            self.handle_input(&button, vfs);
        }
        steps > 0
    }

    /// The app's content area is `cw` x `ch` under theme `at`.
    ///
    /// Hosts call this right before every [`Self::draw_windowed`] and
    /// [`Self::handle_click`], with the same size, so apps can record
    /// layout metrics (content size, visible row counts, hit-test
    /// geometry) in plain fields instead of smuggling them out of the
    /// `&self` draw call through `Cell`s. This is the only place an app
    /// learns its size outside drawing: `handle_click` hit-tests against
    /// what `layout` recorded. Hooks that run without a preceding `layout`
    /// (keys, buttons, wheel) see the metrics of the last drawn frame.
    /// Default is a no-op.
    fn layout(&mut self, _cw: u32, _ch: u32, _at: &ActiveTheme) {}

    /// Handle a click/tap in the app content area.
    ///
    /// `lx`/`ly` are local coordinates within the content area;
    /// [`Self::layout`] has just been called with its size, so hit-test
    /// against the geometry recorded there -- the same geometry
    /// [`Self::draw_windowed`] draws (a kiosk-fullscreen window is still
    /// drawn through it, just bigger). Returns `AppAction::None` by
    /// default.
    fn handle_click(&mut self, _lx: i32, _ly: i32, _vfs: &dyn Vfs) -> AppAction {
        AppAction::None
    }

    /// Draw the app into its `cw` x `ch` content area at `(cx, cy)`, inside
    /// the window manager's clip rect. Every host draws apps this way (a
    /// kiosk-fullscreen app is a window filling the screen). [`Self::layout`]
    /// has just been called with `cw` x `ch`.
    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()>;

    /// The app is closing: destroy every backend resource it created
    /// (textures, render targets). Hosts call this exactly once, after
    /// the window is gone and before the app is dropped -- the only point
    /// where a closing app still has backend access. Default: no-op.
    fn release_resources(&mut self, _backend: &mut dyn SdiBackend) {}

    /// Move every queued VFS IPC request into `out`, oldest first.
    ///
    /// Hosts call this once per frame for every open app and carry out
    /// all of them, including the last requests of an app that closed
    /// during the frame. Default: the app makes no requests.
    fn drain_requests(&mut self, _out: &mut Vec<AppRequest>) {}

    /// Apply VFS mutations the app has queued (deletes, renames, copies,
    /// binary saves, ...).
    ///
    /// Input hooks only see a shared `&dyn Vfs`, so apps that need to
    /// change the file system queue the work from input handlers and the
    /// host drains it here with mutable access, once per frame for every
    /// open app. Returns `true` when anything was applied so the host can
    /// redraw. Default is a no-op returning `false`.
    fn apply_vfs_ops(&mut self, _vfs: &mut dyn Vfs) -> bool {
        false
    }

    /// Advance time-driven state by `dt_ms` milliseconds of wall time, and
    /// poll any VFS state the app mirrors (radio status, search results).
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

    /// The user or an agent asked to close the app from outside the app's
    /// own input (a titlebar close button, `wm close`, MCP `close_window`).
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

    /// Content lines, for apps built on the shared line-list renderers
    /// (hosts and tests read an app's text through this). Default: none.
    fn lines(&self) -> &[String] {
        &[]
    }

    /// Current browse directory, if the app browses files.
    fn browse_dir(&self) -> Option<&str> {
        None
    }

    /// Path of the file currently being viewed, if any.
    fn viewing_file(&self) -> Option<&str> {
        None
    }
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
    /// Cached max visible lines (updated each frame).
    pub cached_max_visible: usize,
    /// Cached title bar height in pixels (updated each frame from the active
    /// theme). Used by click handlers to map click Y back to a line index
    /// without hardcoding renderer metrics.
    pub cached_title_bar_height: u32,
    /// Cached content line height in pixels (updated each frame from the
    /// active theme). Mirrors the value used by `draw_content_windowed`.
    pub cached_line_h: u32,
    /// Content-area height of the last `update_layout_windowed` (click
    /// hit-testing with [`Self::line_at`]).
    pub cached_content_h: u32,
    /// Current browse directory.
    pub browse_dir: Option<String>,
    /// File currently being viewed.
    pub viewing_file: Option<String>,
    /// Queued VFS IPC requests, handed to the host by
    /// [`App::drain_requests`] (see [`Self::post_request`]).
    pub requests: Vec<AppRequest>,
}

/// Maximum visible lines fallback for 480x272.
const DEFAULT_MAX_VISIBLE: usize = 13;
/// Default title bar height fallback used until the first
/// `update_layout_windowed` call populates the real value from the active
/// theme. Matches the common desktop skin metric.
const DEFAULT_TITLE_BAR_HEIGHT: u32 = 20;
/// Default content line height fallback used until the first
/// `update_layout_windowed` call.
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
            cached_max_visible: DEFAULT_MAX_VISIBLE,
            cached_title_bar_height: DEFAULT_TITLE_BAR_HEIGHT,
            cached_line_h: DEFAULT_LINE_H,
            cached_content_h: 0,
            browse_dir: None,
            viewing_file: None,
            requests: Vec::new(),
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

    /// Select line `idx` (clamped to the last line), scrolling as little as
    /// possible to bring it on screen.
    pub fn select(&mut self, idx: usize) {
        let idx = idx.min(self.lines.len().saturating_sub(1));
        let page = self.cached_max_visible.max(1);
        if idx < self.scroll {
            self.scroll = idx;
        } else if idx >= self.scroll + page {
            self.scroll = idx + 1 - page;
        }
        self.cursor = idx - self.scroll;
    }

    /// Absolute index of the selected line.
    pub fn selected(&self) -> usize {
        self.scroll + self.cursor
    }

    /// Update cached layout for a windowed content area `ch` tall: the
    /// row count, line height and header inset
    /// [`draw_content_windowed`](crate::render::draw_content_windowed)
    /// draws with, so scrolling and click-to-line mapping match the screen.
    /// [`impl_content_app_methods!`](crate::impl_content_app_methods)
    /// calls this from [`App::layout`].
    pub fn update_layout_windowed(&mut self, ch: u32, at: &ActiveTheme) {
        let metrics = crate::render::WindowedMetrics::from_theme(at);
        self.cached_max_visible = crate::render::windowed_max_lines(self, ch, metrics).max(1);
        self.cached_title_bar_height = metrics.title_bar_height;
        self.cached_line_h = metrics.line_h;
        self.cached_content_h = ch;
        // A shrunk window keeps the selected line selected and on screen.
        let max_cursor = self.visible_count().saturating_sub(1);
        if self.cursor > max_cursor {
            self.scroll += self.cursor - max_cursor;
            self.cursor = max_cursor;
        }
    }

    /// The windowed metrics recorded by the last
    /// [`Self::update_layout_windowed`], for
    /// [`windowed_line_at`](crate::render::windowed_line_at) in click
    /// handlers.
    pub fn windowed_metrics(&self) -> crate::render::WindowedMetrics {
        crate::render::WindowedMetrics {
            title_bar_height: self.cached_title_bar_height,
            line_h: self.cached_line_h,
        }
    }

    /// Index of the content line drawn at content-local `ly` by
    /// [`draw_content_windowed`](crate::render::draw_content_windowed),
    /// using the layout of the last [`Self::update_layout_windowed`]
    /// (`None` above the list or below its last visible row).
    pub fn line_at(&self, ly: i32) -> Option<usize> {
        crate::render::windowed_line_at(self, self.cached_content_h, self.windowed_metrics(), ly)
    }

    /// Queue a VFS IPC request for the host (see [`App::drain_requests`]).
    pub fn post_request(&mut self, path: impl Into<String>, data: impl Into<String>) {
        self.requests.push(AppRequest::new(path, data));
    }

    /// Standard list navigation shared by the simple line-list apps:
    /// Cancel exits, Up / Down move the selection. Returns `None` for any
    /// other button so the caller can handle it.
    pub fn handle_nav(&mut self, button: &Button) -> Option<AppAction> {
        match button {
            Button::Cancel => Some(AppAction::Exit),
            Button::Up => {
                self.navigate_up();
                Some(AppAction::None)
            },
            Button::Down => {
                self.navigate_down();
                Some(AppAction::None)
            },
            _ => None,
        }
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
    fn select_scrolls_minimally_and_clamps() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = (0..20).map(|i| format!("line {i}")).collect();
        cs.cached_max_visible = 5;
        cs.select(3);
        assert_eq!((cs.scroll, cs.cursor), (0, 3));
        cs.select(12);
        assert_eq!((cs.scroll, cs.cursor), (8, 4));
        cs.select(9);
        assert_eq!((cs.scroll, cs.cursor), (8, 1), "already visible: no scroll");
        cs.select(2);
        assert_eq!((cs.scroll, cs.cursor), (2, 0));
        cs.select(99);
        assert_eq!(cs.selected(), 19);
    }

    #[test]
    fn requests_queue_in_order() {
        let mut cs = ContentState::new("Test", "/apps/test");
        assert!(cs.requests.is_empty());
        cs.post_request("/a", "1");
        cs.post_request("/b", "2");
        assert_eq!(
            cs.requests,
            vec![AppRequest::new("/a", "1"), AppRequest::new("/b", "2")]
        );
    }

    #[test]
    fn handle_nav_maps_cancel_and_arrows() {
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = vec!["a".into(), "b".into()];
        assert_eq!(cs.handle_nav(&Button::Down), Some(AppAction::None));
        assert_eq!(cs.cursor, 1);
        assert_eq!(cs.handle_nav(&Button::Up), Some(AppAction::None));
        assert_eq!(cs.cursor, 0);
        assert_eq!(cs.handle_nav(&Button::Cancel), Some(AppAction::Exit));
        assert_eq!(cs.handle_nav(&Button::Confirm), None);
    }

    #[test]
    fn windowed_layout_matches_drawn_rows() {
        let at = ActiveTheme::default();
        let line_h = at.terminal_line_height.max(12);
        let mut cs = ContentState::new("Test", "/apps/test");
        cs.lines = (0..40).map(|i| format!("line {i}")).collect();
        // Room for exactly 5 rows (top pad + 16 px footer).
        let ch = crate::render::WINDOWED_TOP_PAD + 16 + 5 * line_h;
        cs.cursor = 9;
        cs.update_layout_windowed(ch, &at);
        assert_eq!(cs.cached_max_visible, 5);
        assert_eq!(cs.cursor, 4, "cursor clamped onto a visible row");
        assert_eq!(cs.scroll, 5, "the selected line stays selected");
        // Navigation now scrolls against the real row count.
        cs.navigate_down();
        assert_eq!((cs.scroll, cs.cursor), (6, 4));
    }
}
