//! Shared App trait, layout, and rendering helpers for OASIS_OS applications.
//!
//! This crate provides the common interface and utilities that all extracted
//! app crates depend on, avoiding a dependency on the full `oasis-core`.

mod app_trait;
pub mod file_viewer;
pub mod layout;
pub mod render;

pub use app_trait::{App, ContentState};

/// Action returned by an app after handling input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// App consumed the input, no mode change needed.
    None,
    /// User wants to exit this app and return to dashboard.
    Exit,
    /// App wants to switch to terminal mode.
    SwitchToTerminal,
    /// App requests entering fullscreen kiosk mode.
    RequestFullscreen,
}

/// Generate the boilerplate `App` trait methods that delegate to a
/// `ContentState` field.
///
/// Apps that embed a `ContentState` field and use the standard
/// `render_app_chrome` / `render_content_sdi` / `draw_content_windowed`
/// / `hide_app_sdi` helpers all share identical implementations of:
///
/// - `title()`, `path()`, `lines()`, `update_sdi()`, `draw_windowed()`,
///   `hide_sdi()`, `take_pending_request()`, `peek_pending_request()`,
///   `as_any()`, `as_any_mut()`
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
        fn update_sdi(&mut self, sdi: &mut oasis_sdi::SdiRegistry, at: &oasis_skin::ActiveTheme) {
            self.$field.update_layout(at);
            self.$field.animate_selection(0.3);
            $crate::render::render_app_chrome(sdi, at);
            $crate::render::render_content_sdi(&self.$field, sdi, at);
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
        fn hide_sdi(&self, sdi: &mut oasis_sdi::SdiRegistry) {
            $crate::render::hide_app_sdi(sdi);
        }
        fn take_pending_request(&mut self) -> Option<(String, String)> {
            self.$field.pending_vfs_request.take()
        }
        fn peek_pending_request(&self) -> Option<&(String, String)> {
            self.$field.pending_vfs_request.as_ref()
        }
        fn lines(&self) -> &[String] {
            &self.$field.lines
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}
