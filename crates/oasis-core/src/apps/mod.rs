//! Application runner -- manages launched app screens.
//!
//! When the user selects an app from the dashboard and presses Confirm,
//! an `AppRunner` is created. It renders a title bar and scrollable
//! content area, and handles input for navigation and exit.
//!
//! The `App` trait defines the extensible interface that each application
//! implements. `AppRunner` delegates to the active app implementation.

mod app_trait;
pub mod browsing_app;
pub mod calculator;
pub mod clock;
pub mod file_manager;
pub(crate) mod file_viewer;
pub mod games;
pub mod layout_calc;
pub mod paint;
mod runner;
mod runner_sdi;
pub mod simple_app;
pub mod text_editor;
pub mod tv_guide;

pub use app_trait::{App, ContentState};
pub use runner::{AppAction, AppRunner};
