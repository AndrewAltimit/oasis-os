//! App screen runner -- dispatches to extracted `App` trait implementations.
//!
//! All apps are fully delegated to their own crate via `Box<dyn App>`.
//!
//! App construction goes through the host's [`AppRegistry`]: adding an app
//! is one [`AppRegistration`](super::AppRegistration) (the built-in ones
//! live in the `oasis-apps` bundle); no match arms elsewhere change.
//!
//! Input handling lives in [`super::input`].

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::dashboard::AppEntry;
use crate::vfs::Vfs;

use super::registry::AppRegistry;
use super::{App, AppAction, AppRequest};

/// Runtime state for a launched application screen: the app itself plus the
/// host-facing bookkeeping (redraw tracking).
#[derive(Debug)]
pub struct AppRunner {
    /// App display title (the delegate's title at launch).
    pub title: String,
    /// App path in VFS (the delegate's path at launch).
    pub path: String,
    /// The app implementation.
    pub(crate) delegate: Box<dyn App>,
    /// Whether the app's visible content may have changed since the host
    /// last drew it (see [`Self::wants_frame`] / [`Self::mark_drawn`]).
    /// Set conservatively by every mutating entry point.
    pub(crate) redraw_pending: bool,
}

impl AppRunner {
    /// Launch the app of dashboard entry `app` from `apps`. A title the
    /// registry doesn't know opens a placeholder screen.
    pub fn launch(apps: &AppRegistry, app: &AppEntry, vfs: &dyn Vfs) -> Self {
        let delegate = apps.create(app, vfs).unwrap_or_else(|| {
            placeholder(
                &app.title,
                &app.path,
                "(No content available for this app)".to_string(),
            )
        });
        Self::from_delegate(delegate)
    }

    /// Launch an app with `file_path` pre-opened in its viewer.
    ///
    /// Used when File Manager dispatches to Photo Viewer / Music Player /
    /// Text Editor via `AppAction::LaunchAppWithFile`, and for
    /// `open <app> <file>`. An app that does not open files (or an unknown
    /// title) opens a placeholder saying so.
    pub fn launch_with_file(
        apps: &AppRegistry,
        app: &AppEntry,
        file_path: &str,
        vfs: &dyn Vfs,
    ) -> Self {
        let delegate = apps
            .create_for_file(app, file_path, vfs)
            .unwrap_or_else(|| {
                placeholder(
                    &app.title,
                    &app.path,
                    format!("(Cannot open {file_path} with this app)"),
                )
            });
        Self::from_delegate(delegate)
    }

    /// Create an `AppRunner` from a pre-built `App` delegate.
    ///
    /// Used by plugin apps that provide their own `App` implementation
    /// via the plugin-to-app bridge.
    pub fn from_delegate(delegate: Box<dyn App>) -> Self {
        Self {
            title: delegate.title().to_string(),
            path: delegate.path().to_string(),
            delegate,
            redraw_pending: true,
        }
    }

    /// The app's content lines (see [`App::lines`]).
    pub fn lines(&self) -> &[String] {
        self.delegate.lines()
    }

    /// The app's current browse directory (see [`App::browse_dir`]).
    pub fn browse_dir(&self) -> Option<&str> {
        self.delegate.browse_dir()
    }

    /// The file the app is viewing (see [`App::viewing_file`]).
    pub fn viewing_file(&self) -> Option<&str> {
        self.delegate.viewing_file()
    }

    /// Advance the app's time-driven state by `dt_ms` (see
    /// [`App::tick`]). Hosts call this once per frame for every open
    /// runner, whether or not the frame is drawn.
    pub fn tick(&mut self, dt_ms: u32, vfs: &dyn Vfs) {
        if self.delegate.tick(dt_ms, vfs) {
            self.redraw_pending = true;
        }
    }

    /// Move the app's queued VFS IPC requests into `out` (see
    /// [`App::drain_requests`]). Hosts call this once per frame for every
    /// open runner and carry out all of them.
    pub fn drain_requests(&mut self, out: &mut Vec<AppRequest>) {
        self.delegate.drain_requests(out);
    }

    /// Take the app's pending self-close request (see
    /// [`App::take_close_request`]). Hosts poll this once per frame after
    /// [`Self::apply_vfs_ops`] and [`Self::tick`] and, when it returns
    /// `true`, close the runner as for `AppAction::Exit`.
    pub fn take_close_request(&mut self) -> bool {
        self.delegate.take_close_request()
    }

    /// Destroy the app's backend resources before the runner is dropped
    /// (see [`App::release_resources`]).
    pub fn release_resources(&mut self, backend: &mut dyn SdiBackend) {
        self.delegate.release_resources(backend);
    }

    /// Ask the app whether its window may close now (titlebar close
    /// button, `wm close`, MCP; see [`App::on_close_requested`]). Returns
    /// `AppAction::Exit` to close; any other action keeps it open.
    pub fn request_close(&mut self, vfs: &dyn Vfs) -> AppAction {
        let action = self.delegate.on_close_requested(vfs);
        if action != AppAction::Exit {
            self.redraw_pending = true;
        }
        action
    }

    /// Whether the app's window needs to be redrawn: its content changed
    /// since the last [`Self::mark_drawn`], or the app animates
    /// continuously ([`App::wants_frame`]).
    ///
    /// Hosts with idle-frame elision OR this over every visible window.
    pub fn wants_frame(&self) -> bool {
        self.redraw_pending || self.delegate.wants_frame()
    }

    /// Record that the host just drew this runner's current content.
    pub fn mark_drawn(&mut self) {
        self.redraw_pending = false;
    }

    /// Force a redraw of this runner's window on the next frame (for
    /// hosts that mutate app state behind the runner's back).
    pub fn request_redraw(&mut self) {
        self.redraw_pending = true;
    }

    /// Set a Terminal runner's text cursor column (characters) within the
    /// trailing prompt line; `None` hides it. No-op for other runners.
    pub fn set_terminal_cursor(&mut self, col: Option<usize>) {
        // Downcast directly: `delegate_as_mut` would flag a redraw even when
        // the cursor didn't move.
        if let Some(simple) = self
            .delegate
            .as_any_mut()
            .downcast_mut::<super::simple_app::SimpleApp>()
            && simple.set_prompt_cursor(col)
        {
            self.redraw_pending = true;
        }
    }

    /// Sync terminal scrollback plus a trailing prompt line into a
    /// Terminal runner, incrementally (see
    /// [`SimpleApp::sync_terminal_lines`](super::simple_app::SimpleApp::sync_terminal_lines)):
    /// only lines the app doesn't already hold are cloned. Schedules a
    /// redraw only when the displayed content changed. Returns the number
    /// of lines cloned (0 if this runner is not a Terminal).
    pub fn sync_terminal_lines(
        &mut self,
        output: &[String],
        prompt: &str,
        scroll_offset: usize,
    ) -> usize {
        let Some(simple) = self
            .delegate
            .as_any_mut()
            .downcast_mut::<super::simple_app::SimpleApp>()
        else {
            return 0;
        };
        let before = (simple.content.lines.len(), simple.content.scroll);
        let cloned = simple.sync_terminal_lines(output, prompt, scroll_offset);
        if cloned > 0 || before != (simple.content.lines.len(), simple.content.scroll) {
            self.redraw_pending = true;
        }
        cloned
    }

    /// Draw the app into its window's content area (inside the clip region
    /// the window manager's `draw_with_clips` callback provides). Calls
    /// [`App::layout`] with the content size first.
    pub fn draw_windowed(
        &mut self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        self.delegate.layout(cw, ch, at);
        self.delegate.draw_windowed(cx, cy, cw, ch, backend, at)
    }

    /// Get a reference to the delegate app, downcasting with `as_any()`.
    pub fn delegate_as<T: 'static>(&self) -> Option<&T> {
        self.delegate.as_any().downcast_ref::<T>()
    }

    /// Get a mutable reference to the delegate app, downcasting with `as_any_mut()`.
    ///
    /// Conservatively marks the runner for redraw on a successful downcast
    /// (the caller may change anything the app draws).
    pub fn delegate_as_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let app = self.delegate.as_any_mut().downcast_mut::<T>();
        if app.is_some() {
            self.redraw_pending = true;
        }
        app
    }

    /// Like [`Self::delegate_as_mut`] but without scheduling a redraw: for
    /// hosts that poll app state every frame and request the redraw
    /// themselves ([`Self::request_redraw`]) when they change something
    /// visible.
    pub fn delegate_as_mut_untracked<T: 'static>(&mut self) -> Option<&mut T> {
        self.delegate.as_any_mut().downcast_mut::<T>()
    }
}

/// Fallback app for a title with no registered (or no file-capable) app.
fn placeholder(title: &str, path: &str, note: String) -> Box<dyn App> {
    Box::new(super::simple_app::SimpleApp::new(
        title,
        path,
        vec![title.to_string(), String::new(), note],
    ))
}

#[cfg(test)]
mod tests {
    //! Runner mechanics against apps that live in `oasis-core` (the
    //! Terminal `SimpleApp`, placeholders, test apps). Behaviour of the
    //! built-in app crates behind a runner is tested in `oasis-apps`.
    use super::*;
    use crate::apps::simple_app::SimpleApp;
    use crate::apps::{AppRegistration, ContentState};
    use crate::input::Button;
    use crate::vfs::MemoryVfs;

    const MAX_VISIBLE_LINES: usize = 13;

    fn registry() -> AppRegistry {
        let mut apps = AppRegistry::new();
        apps.register(AppRegistration::new("Terminal", |path, _vfs| {
            Box::new(SimpleApp::terminal(path))
        }));
        apps
    }

    fn launch(title: &str) -> AppRunner {
        AppRunner::launch(&registry(), &AppEntry::for_title(title), &MemoryVfs::new())
    }

    #[test]
    fn terminal_cursor_move_requests_redraw() {
        let mut runner = launch("Terminal");
        runner.set_terminal_cursor(Some(4));
        runner.mark_drawn();
        runner.set_terminal_cursor(Some(4));
        assert!(!runner.wants_frame(), "unchanged cursor must stay idle");
        runner.set_terminal_cursor(Some(3));
        assert!(runner.wants_frame(), "cursor-only move must redraw");
    }

    #[test]
    fn terminal_sync_one_line_does_not_clone_scrollback() {
        let mut runner = launch("Terminal");
        let mut output: Vec<String> = (0..2000).map(|i| format!("out {i}")).collect();
        runner.sync_terminal_lines(&output, "> ", 0);
        assert_eq!(runner.lines().len(), 2001);
        let kept_ptr = runner.lines()[500].as_ptr();

        output.push("one more".into());
        let cloned = runner.sync_terminal_lines(&output, "> ls", 0);
        assert_eq!(cloned, 2, "new line + prompt only");
        assert_eq!(runner.lines().len(), 2002);
        assert_eq!(runner.lines()[2000], "one more");
        assert_eq!(runner.lines()[2001], "> ls");
        assert_eq!(
            runner.lines()[500].as_ptr(),
            kept_ptr,
            "existing lines are kept in place, not re-cloned"
        );
        let app = runner
            .delegate_as::<SimpleApp>()
            .expect("terminal delegate");
        assert_eq!(app.content.lines, runner.lines());
    }

    #[test]
    fn unknown_title_launches_a_placeholder() {
        let runner = launch("Unknown App");
        assert_eq!(runner.title, "Unknown App");
        assert!(runner.lines().iter().any(|l| l.contains("No content")));
    }

    #[test]
    fn file_launch_of_an_app_without_files_is_a_placeholder() {
        let vfs = MemoryVfs::new();
        let entry = AppEntry::for_title("Terminal");
        let runner = AppRunner::launch_with_file(&registry(), &entry, "/a.txt", &vfs);
        assert!(
            runner
                .lines()
                .iter()
                .any(|l| l.contains("Cannot open /a.txt"))
        );
    }

    #[test]
    fn from_delegate_creates_runner() {
        let delegate: Box<dyn App> = Box::new(SimpleApp::new(
            "Plugin Test",
            "/plugins/test",
            vec!["Hello from plugin".to_string()],
        ));
        let runner = AppRunner::from_delegate(delegate);
        assert_eq!(runner.title, "Plugin Test");
        assert_eq!(runner.path, "/plugins/test");
        assert!(
            runner
                .lines()
                .iter()
                .any(|l| l.contains("Hello from plugin"))
        );
    }

    #[test]
    fn idle_runner_stops_wanting_frames() {
        let vfs = MemoryVfs::new();
        let mut runner = launch("Terminal");
        assert!(runner.wants_frame(), "a new window must be drawn once");
        runner.mark_drawn();
        for _ in 0..10 {
            runner.tick(16, &vfs);
            runner.apply_vfs_ops(&mut MemoryVfs::new());
        }
        assert!(!runner.wants_frame(), "an idle terminal elides frames");
    }

    #[test]
    fn input_and_content_changes_request_a_frame() {
        let vfs = MemoryVfs::new();
        let mut runner = launch("Terminal");
        runner.mark_drawn();

        runner.handle_text_input('a');
        assert!(runner.wants_frame(), "typing redraws");
        runner.mark_drawn();

        runner.handle_input(&Button::Down, &vfs);
        assert!(runner.wants_frame(), "navigation redraws");
        runner.mark_drawn();

        let output = vec!["$ ls".to_string(), "readme.txt".to_string()];
        runner.sync_terminal_lines(&output, "> ", 0);
        assert!(runner.wants_frame(), "new terminal output redraws");
        runner.mark_drawn();

        // Re-syncing identical content is not a change.
        runner.sync_terminal_lines(&output, "> ", 0);
        assert!(!runner.wants_frame());
        // A scroll is.
        let many: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        runner.sync_terminal_lines(&many, "> ", 0);
        runner.mark_drawn();
        runner.sync_terminal_lines(&many, "> ", 3);
        assert!(runner.wants_frame(), "scrolling redraws");
    }

    #[test]
    fn requests_from_one_frame_are_all_drained() {
        /// Posts two requests per button press.
        #[derive(Debug)]
        struct PostingApp {
            content: ContentState,
        }
        impl App for PostingApp {
            oasis_app_core::impl_content_app_methods!(content);
            fn handle_input(&mut self, _button: &Button, _vfs: &dyn Vfs) -> AppAction {
                self.content.post_request("/tmp/a", "1");
                self.content.post_request("/tmp/b", "2");
                AppAction::None
            }
        }

        let vfs = MemoryVfs::new();
        let mut runner = AppRunner::from_delegate(Box::new(PostingApp {
            content: ContentState::new("Posting", "/apps/posting"),
        }));
        runner.handle_input(&Button::Confirm, &vfs);
        let mut out = Vec::new();
        runner.drain_requests(&mut out);
        assert_eq!(
            out,
            vec![
                AppRequest::new("/tmp/a", "1"),
                AppRequest::new("/tmp/b", "2")
            ]
        );
        runner.drain_requests(&mut out);
        assert_eq!(out.len(), 2, "drained requests are gone");
    }

    /// Minimal app that asks to close itself from `tick`.
    #[derive(Debug)]
    struct SelfClosingApp {
        lines: Vec<String>,
        close: bool,
    }

    impl App for SelfClosingApp {
        fn title(&self) -> &str {
            "Self Closing"
        }
        fn path(&self) -> &str {
            "/apps/Self Closing"
        }
        fn handle_input(&mut self, _button: &Button, _vfs: &dyn Vfs) -> AppAction {
            AppAction::None
        }
        fn draw_windowed(
            &self,
            _cx: i32,
            _cy: i32,
            _cw: u32,
            _ch: u32,
            _backend: &mut dyn SdiBackend,
            _at: &ActiveTheme,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn tick(&mut self, _dt_ms: u32, _vfs: &dyn Vfs) -> bool {
            self.close = true;
            false
        }
        fn take_close_request(&mut self) -> bool {
            std::mem::take(&mut self.close)
        }
        fn lines(&self) -> &[String] {
            &self.lines
        }
    }

    #[test]
    fn take_close_request_forwards_to_delegate() {
        let vfs = MemoryVfs::new();
        let mut runner = AppRunner::from_delegate(Box::new(SelfClosingApp {
            lines: Vec::new(),
            close: false,
        }));
        assert!(!runner.take_close_request());
        runner.tick(16, &vfs);
        assert!(runner.take_close_request());
        assert!(!runner.take_close_request(), "request is consumed");

        // Apps that never self-close keep the default.
        let mut terminal = launch("Terminal");
        terminal.tick(16, &vfs);
        assert!(!terminal.take_close_request());
    }

    #[test]
    fn terminal_app_confirm_is_noop() {
        let vfs = MemoryVfs::new();
        let mut runner = launch("Terminal");
        let action = runner.handle_input(&Button::Confirm, &vfs);
        assert_eq!(action, AppAction::None);
    }

    #[test]
    fn scroll_down_when_content_exceeds_view() {
        let vfs = MemoryVfs::new();
        let mut runner = launch("Terminal");
        let app = runner.delegate_as_mut::<SimpleApp>().unwrap();
        for i in 0..20 {
            app.content.lines.push(format!("Extra line {i}"));
        }
        app.content.cached_max_visible = MAX_VISIBLE_LINES;
        // Move cursor to bottom of visible area.
        for _ in 0..MAX_VISIBLE_LINES - 1 {
            runner.handle_input(&Button::Down, &vfs);
        }
        let app = runner.delegate_as::<SimpleApp>().unwrap();
        assert_eq!(app.content.cursor, MAX_VISIBLE_LINES - 1);
        // Next down should scroll.
        runner.handle_input(&Button::Down, &vfs);
        let app = runner.delegate_as::<SimpleApp>().unwrap();
        assert_eq!(app.content.scroll, 1);
    }

    #[test]
    fn untracked_downcast_does_not_request_a_frame() {
        let mut runner = launch("Terminal");
        runner.mark_drawn();
        assert!(runner.delegate_as_mut_untracked::<SimpleApp>().is_some());
        assert!(!runner.wants_frame());
        assert!(runner.delegate_as_mut::<SimpleApp>().is_some());
        assert!(runner.wants_frame());
    }
}
