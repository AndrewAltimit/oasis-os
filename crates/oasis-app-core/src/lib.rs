//! The `App` trait and the helpers every OASIS_OS app shares.
//!
//! App crates depend on this crate (never on `oasis-core`): the [`App`]
//! trait and its [`AppAction`] / [`AppRequest`] vocabulary, [`ContentState`]
//! plus [`impl_content_app_methods!`] for line-list apps, the windowed
//! line-list renderer ([`render`]), file viewers ([`file_viewer`]) and,
//! behind the `testing` feature, an end-to-end app harness.

mod app_trait;
pub mod file_viewer;
pub mod render;
#[cfg(feature = "testing")]
pub mod testing;

pub use app_trait::{App, AppRequest, AsAny, ContentState, MAX_WHEEL_STEPS};

/// Action returned by an app after handling input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    /// App consumed the input, no mode change needed.
    None,
    /// User wants to exit this app and return to dashboard.
    Exit,
    /// App wants to switch to terminal mode.
    SwitchToTerminal,
    /// App requests entering fullscreen kiosk mode.
    RequestFullscreen,
    /// App asks the host to launch `app_title` with `file_path` pre-loaded.
    /// Used by File Manager when the user opens a typed file (image, audio,
    /// text) to hand off to the appropriate viewer.
    LaunchAppWithFile {
        app_title: String,
        file_path: String,
    },
}

/// Generate the boilerplate `App` trait methods that delegate to a
/// `ContentState` field.
///
/// Apps that embed a `ContentState` field and draw it with
/// [`render::draw_content_windowed`] share identical implementations of:
///
/// - `title()`, `path()`, `lines()`, `browse_dir()`, `viewing_file()`
/// - `layout()` ([`ContentState::update_layout_windowed`], so scrolling and
///   click-to-line mapping use the rows the window really shows)
/// - `draw_windowed()`
/// - `drain_requests()` (hands over `ContentState::requests`; queue them
///   with [`ContentState::post_request`])
///
/// Downcasting (`as_any`) comes from the [`AsAny`] blanket impl. The app
/// implements `handle_input` and whatever optional hooks it needs.
///
/// # Usage
///
/// ```ignore
/// impl App for MyApp {
///     impl_content_app_methods!(content);
///     fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction { ... }
/// }
/// ```
#[macro_export]
macro_rules! impl_content_app_methods {
    ($field:ident) => {
        fn title(&self) -> &str {
            &self.$field.title
        }
        fn path(&self) -> &str {
            &self.$field.app_path
        }
        fn layout(&mut self, _cw: u32, ch: u32, at: &oasis_skin::ActiveTheme) {
            self.$field.update_layout_windowed(ch, at);
        }
        fn draw_windowed(
            &self,
            cx: i32,
            cy: i32,
            cw: u32,
            ch: u32,
            backend: &mut dyn oasis_types::backend::SdiBackend,
            at: &oasis_skin::ActiveTheme,
        ) -> oasis_types::error::Result<()> {
            $crate::render::draw_content_windowed(&self.$field, cx, cy, cw, ch, backend, at)
        }
        fn drain_requests(&mut self, out: &mut Vec<$crate::AppRequest>) {
            out.append(&mut self.$field.requests);
        }
        fn lines(&self) -> &[String] {
            &self.$field.lines
        }
        fn browse_dir(&self) -> Option<&str> {
            self.$field.browse_dir.as_deref()
        }
        fn viewing_file(&self) -> Option<&str> {
            self.$field.viewing_file.as_deref()
        }
    };
}
