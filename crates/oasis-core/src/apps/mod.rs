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
pub mod file_manager;
pub mod layout_calc;
mod runner;
pub mod simple_app;
pub mod tv_guide;

pub use app_trait::{App, ContentState};
pub use runner::{AppAction, AppRunner};
