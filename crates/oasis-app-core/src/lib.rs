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
