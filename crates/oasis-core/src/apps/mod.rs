//! Application runner -- manages launched app screens.
//!
//! When the user selects an app from the dashboard and presses Confirm,
//! an `AppRunner` is created. It renders a title bar and scrollable
//! content area, and handles input for navigation and exit.
//!
//! The `App` trait defines the extensible interface that each application
//! implements. `AppRunner` delegates to the active app implementation.

mod app_trait;
pub mod file_manager;
pub(crate) mod file_viewer;
pub mod layout_calc;
mod runner;
mod runner_sdi;
pub mod simple_app;
#[cfg(feature = "wasm-youtube")]
pub mod video_embed;

pub use app_trait::{App, AppAction, ContentState};
pub use runner::AppRunner;

/// Re-export TV Guide crate for backwards compatibility with external crates.
pub use oasis_app_tv_guide as tv_guide;
