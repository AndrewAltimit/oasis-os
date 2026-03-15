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
    #![allow(clippy::unwrap_used)]

    use super::*;
    use oasis_core::input::{Button, Trigger};

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

    // -------------------------------------------------------------------
    // Item 69: UE5/FFI input mapping tests (5 new tests)
    // -------------------------------------------------------------------

    #[test]
    fn trigger_events_round_trip() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::TriggerPress(Trigger::Left));
        backend.push_event(InputEvent::TriggerRelease(Trigger::Left));
        backend.push_event(InputEvent::TriggerPress(Trigger::Right));
        backend.push_event(InputEvent::TriggerRelease(Trigger::Right));
        let events = backend.poll_events();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0], InputEvent::TriggerPress(Trigger::Left));
        assert_eq!(events[1], InputEvent::TriggerRelease(Trigger::Left));
        assert_eq!(events[2], InputEvent::TriggerPress(Trigger::Right));
        assert_eq!(events[3], InputEvent::TriggerRelease(Trigger::Right));
    }

    #[test]
    fn mouse_wheel_event_round_trip() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::MouseWheel { delta: -3 });
        backend.push_event(InputEvent::MouseWheel { delta: 5 });
        let events = backend.poll_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], InputEvent::MouseWheel { delta: -3 });
        assert_eq!(events[1], InputEvent::MouseWheel { delta: 5 });
    }

    #[test]
    fn backspace_and_text_input_round_trip() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::TextInput('H'));
        backend.push_event(InputEvent::TextInput('i'));
        backend.push_event(InputEvent::Backspace);
        let events = backend.poll_events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], InputEvent::TextInput('H'));
        assert_eq!(events[1], InputEvent::TextInput('i'));
        assert_eq!(events[2], InputEvent::Backspace);
    }

    #[test]
    fn toggle_fullscreen_round_trip() {
        let mut backend = FfiInputBackend::new();
        backend.push_event(InputEvent::ToggleFullscreen);
        let events = backend.poll_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::ToggleFullscreen);
    }

    #[test]
    fn interleaved_event_types_preserve_order() {
        let mut backend = FfiInputBackend::new();
        let expected = vec![
            InputEvent::CursorMove { x: 10, y: 20 },
            InputEvent::ButtonPress(Button::Confirm),
            InputEvent::TextInput('x'),
            InputEvent::PointerClick { x: 100, y: 200 },
            InputEvent::TriggerPress(Trigger::Left),
            InputEvent::Backspace,
            InputEvent::MouseWheel { delta: -1 },
            InputEvent::ButtonRelease(Button::Confirm),
            InputEvent::PointerRelease { x: 100, y: 200 },
            InputEvent::TriggerRelease(Trigger::Left),
            InputEvent::FocusLost,
            InputEvent::FocusGained,
            InputEvent::ToggleFullscreen,
            InputEvent::Quit,
        ];
        for e in &expected {
            backend.push_event(e.clone());
        }
        let events = backend.poll_events();
        assert_eq!(events, expected);
    }
}
