//! FFI input backend.
//!
//! Queues `InputEvent`s pushed from the FFI layer and delivers them
//! when polled by the core framework.

use oasis_core::backend::InputBackend;
use oasis_core::input::InputEvent;

/// Input backend driven by FFI event pushes.
///
/// The FFI layer converts C-ABI `OasisInputEvent` structs into Rust
/// `InputEvent` values and pushes them here. The core framework polls
/// via `InputBackend::poll_events()`.
pub struct FfiInputBackend {
    events: Vec<InputEvent>,
}

impl FfiInputBackend {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Push an input event into the queue (called from FFI layer).
    pub fn push_event(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Number of queued events.
    pub fn pending_count(&self) -> usize {
        self.events.len()
    }
}

impl Default for FfiInputBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBackend for FfiInputBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::input::Button;

    #[test]
    fn empty_by_default() {
        let mut backend = FfiInputBackend::new();
        assert!(backend.poll_events().is_empty());
    }

    #[test]
    fn push_and_poll() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::ButtonPress(Button::Confirm));
        backend.push_event(InputEvent::TextInput('A'));
        assert_eq!(backend.pending_count(), 2);

        let events = backend.poll_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], InputEvent::ButtonPress(Button::Confirm));
        assert_eq!(events[1], InputEvent::TextInput('A'));

        // Queue is drained after poll.
        assert!(backend.poll_events().is_empty());
    }

    #[test]
    fn default_constructor() {
        let _backend = FfiInputBackend::default();
    }

    #[test]
    fn push_cursor_move_preserves_coordinates() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::CursorMove { x: 100, y: 200 });
        let events = backend.poll_events();
        assert_eq!(events[0], InputEvent::CursorMove { x: 100, y: 200 });
    }

    #[test]
    fn push_pointer_click_coordinates() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::PointerClick { x: 0, y: 0 });
        backend.push_event(InputEvent::PointerClick { x: 479, y: 271 });
        backend.push_event(InputEvent::PointerClick { x: -1, y: -1 });
        let events = backend.poll_events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], InputEvent::PointerClick { x: 0, y: 0 });
        assert_eq!(events[1], InputEvent::PointerClick { x: 479, y: 271 });
        // Negative coordinates pass through (clamping is caller's
        // responsibility in the FFI layer).
        assert_eq!(events[2], InputEvent::PointerClick { x: -1, y: -1 });
    }

    #[test]
    fn push_pointer_release_coordinates() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::PointerRelease {
            x: i32::MAX,
            y: i32::MIN,
        });
        let events = backend.poll_events();
        assert_eq!(
            events[0],
            InputEvent::PointerRelease {
                x: i32::MAX,
                y: i32::MIN,
            }
        );
    }

    #[test]
    fn multiple_poll_drains_each_time() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::FocusGained);
        assert_eq!(backend.poll_events().len(), 1);
        assert_eq!(backend.poll_events().len(), 0);
        backend.push_event(InputEvent::FocusLost);
        backend.push_event(InputEvent::Quit);
        assert_eq!(backend.poll_events().len(), 2);
        assert_eq!(backend.poll_events().len(), 0);
    }

    #[test]
    fn all_button_types_round_trip() {
        let mut backend = FfiInputBackend::new();
        let buttons = [
            Button::Up,
            Button::Down,
            Button::Left,
            Button::Right,
            Button::Confirm,
            Button::Cancel,
            Button::Triangle,
            Button::Square,
            Button::Start,
            Button::Select,
        ];
        for btn in buttons {
            backend.push_event(InputEvent::ButtonPress(btn));
            backend.push_event(InputEvent::ButtonRelease(btn));
        }
        let events = backend.poll_events();
        assert_eq!(events.len(), 20);
    }
}
