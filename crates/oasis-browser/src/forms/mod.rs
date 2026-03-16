//! Form state management and interaction for the browser engine.
//!
//! Provides [`FormManager`] to track multiple HTML forms on a page,
//! manage focus/tab navigation between form elements, handle keyboard
//! input into text fields, and collect [`FormData`] for submission.
//!
//! Implementation is split across sub-modules:
//! - `manager` — Core `FormManager` struct and element manipulation API
//! - `input_handling` — Keyboard input dispatch and focus navigation
//! - `serialization` — Form data collection helpers

mod input_handling;
mod manager;
mod serialization;
mod state;
mod types;
pub mod validation;

pub use manager::FormManager;
pub use state::FormState;
pub use types::{FormAction, FormData, FormElement, FormKey, FormMethod, InputType, SelectOption};
