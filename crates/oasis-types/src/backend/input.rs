//! Input backend trait.

use crate::input::InputEvent;

/// Input backend trait.
///
/// Maps platform-specific input to the platform-agnostic `InputEvent` enum.
pub trait InputBackend {
    /// Poll for pending input events.
    fn poll_events(&mut self) -> Vec<InputEvent>;
}
