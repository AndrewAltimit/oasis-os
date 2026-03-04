//! Form state management and interaction for the browser engine.
//!
//! Provides [`FormManager`] to track multiple HTML forms on a page,
//! manage focus/tab navigation between form elements, handle keyboard
//! input into text fields, and collect [`FormData`] for submission.

mod manager;
mod state;
mod types;

pub use manager::FormManager;
pub use state::FormState;
pub use types::{FormAction, FormData, FormElement, FormKey, FormMethod, InputType, SelectOption};
