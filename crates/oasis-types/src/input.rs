//! Platform-agnostic input event types.
//!
//! Every backend maps its native input to these enums. The core framework
//! never sees raw platform input.

use serde::{Deserialize, Serialize};

/// A platform-agnostic input event.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Cursor / analog stick moved to absolute position.
    CursorMove { x: i32, y: i32 },
    /// A face / d-pad button pressed.
    ButtonPress(Button),
    /// A face / d-pad button released.
    ButtonRelease(Button),
    /// Shoulder trigger pressed.
    TriggerPress(Trigger),
    /// Shoulder trigger released.
    TriggerRelease(Trigger),
    /// Character typed (on-screen keyboard or physical keyboard).
    TextInput(char),
    /// Backspace / delete-left.
    Backspace,
    /// Pointer click at absolute position (mouse or touch).
    PointerClick { x: i32, y: i32 },
    /// Pointer released.
    PointerRelease { x: i32, y: i32 },
    /// The OS instance gained focus.
    FocusGained,
    /// The OS instance lost focus.
    FocusLost,
    /// User requested quit (window close, etc.).
    Quit,
}

/// Buttons that map across all platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
    Triangle,
    Square,
    Start,
    Select,
}

/// Shoulder / trigger buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Trigger {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- InputEvent variant construction and equality --

    #[test]
    fn cursor_move_event() {
        let e = InputEvent::CursorMove { x: 100, y: 200 };
        assert_eq!(e, InputEvent::CursorMove { x: 100, y: 200 });
    }

    #[test]
    fn cursor_move_negative_coords() {
        let e = InputEvent::CursorMove { x: -10, y: -20 };
        if let InputEvent::CursorMove { x, y } = e {
            assert_eq!(x, -10);
            assert_eq!(y, -20);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn button_press_all_variants() {
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
            let e = InputEvent::ButtonPress(btn);
            assert_eq!(e, InputEvent::ButtonPress(btn));
        }
    }

    #[test]
    fn button_release_differs_from_press() {
        let press = InputEvent::ButtonPress(Button::Confirm);
        let release = InputEvent::ButtonRelease(Button::Confirm);
        assert_ne!(press, release);
    }

    #[test]
    fn trigger_press_both_variants() {
        let left = InputEvent::TriggerPress(Trigger::Left);
        let right = InputEvent::TriggerPress(Trigger::Right);
        assert_ne!(left, right);
    }

    #[test]
    fn trigger_release_differs_from_press() {
        let press = InputEvent::TriggerPress(Trigger::Left);
        let release = InputEvent::TriggerRelease(Trigger::Left);
        assert_ne!(press, release);
    }

    #[test]
    fn text_input_ascii() {
        let e = InputEvent::TextInput('A');
        assert_eq!(e, InputEvent::TextInput('A'));
    }

    #[test]
    fn text_input_unicode() {
        let e = InputEvent::TextInput('\u{1F600}');
        if let InputEvent::TextInput(ch) = e {
            assert_eq!(ch, '\u{1F600}');
        }
    }

    #[test]
    fn backspace_event() {
        let e = InputEvent::Backspace;
        assert_eq!(e, InputEvent::Backspace);
    }

    #[test]
    fn pointer_click_event() {
        let e = InputEvent::PointerClick { x: 240, y: 136 };
        if let InputEvent::PointerClick { x, y } = e {
            assert_eq!(x, 240);
            assert_eq!(y, 136);
        }
    }

    #[test]
    fn pointer_release_event() {
        let e = InputEvent::PointerRelease { x: 0, y: 0 };
        assert_eq!(e, InputEvent::PointerRelease { x: 0, y: 0 });
    }

    #[test]
    fn focus_and_quit_events() {
        assert_eq!(InputEvent::FocusGained, InputEvent::FocusGained);
        assert_eq!(InputEvent::FocusLost, InputEvent::FocusLost);
        assert_eq!(InputEvent::Quit, InputEvent::Quit);
        assert_ne!(InputEvent::FocusGained, InputEvent::FocusLost);
        assert_ne!(InputEvent::FocusGained, InputEvent::Quit);
    }

    // -- Button properties --

    #[test]
    fn button_clone_and_copy() {
        let b = Button::Confirm;
        let b2 = b;
        let b3 = b.clone();
        assert_eq!(b, b2);
        assert_eq!(b, b3);
    }

    #[test]
    fn button_debug_format() {
        let dbg = format!("{:?}", Button::Triangle);
        assert_eq!(dbg, "Triangle");
    }

    #[test]
    fn button_hash_distinct() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Button::Up);
        set.insert(Button::Down);
        set.insert(Button::Up);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn button_serde_roundtrip() {
        let b = Button::Start;
        let json = serde_json::to_string(&b).unwrap();
        let b2: Button = serde_json::from_str(&json).unwrap();
        assert_eq!(b, b2);
    }

    // -- Trigger properties --

    #[test]
    fn trigger_clone_and_copy() {
        let t = Trigger::Right;
        let t2 = t;
        assert_eq!(t, t2);
    }

    #[test]
    fn trigger_hash_distinct() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Trigger::Left);
        set.insert(Trigger::Right);
        set.insert(Trigger::Left);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn trigger_serde_roundtrip() {
        let t = Trigger::Left;
        let json = serde_json::to_string(&t).unwrap();
        let t2: Trigger = serde_json::from_str(&json).unwrap();
        assert_eq!(t, t2);
    }

    // -- InputEvent clone --

    #[test]
    fn input_event_clone() {
        let e = InputEvent::CursorMove { x: 42, y: 99 };
        let e2 = e.clone();
        assert_eq!(e, e2);
    }

    // -- All variants are distinguishable --

    #[test]
    fn all_event_variants_distinct() {
        let events: Vec<InputEvent> = vec![
            InputEvent::CursorMove { x: 0, y: 0 },
            InputEvent::ButtonPress(Button::Up),
            InputEvent::ButtonRelease(Button::Up),
            InputEvent::TriggerPress(Trigger::Left),
            InputEvent::TriggerRelease(Trigger::Left),
            InputEvent::TextInput('x'),
            InputEvent::Backspace,
            InputEvent::PointerClick { x: 0, y: 0 },
            InputEvent::PointerRelease { x: 0, y: 0 },
            InputEvent::FocusGained,
            InputEvent::FocusLost,
            InputEvent::Quit,
        ];
        for (i, a) in events.iter().enumerate() {
            for (j, b) in events.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "variants {i} and {j} should differ");
                }
            }
        }
    }
}
