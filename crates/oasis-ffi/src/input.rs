//! Input event marshaling from C types to Rust `InputEvent`.

use oasis_core::input::{Button, InputEvent, Key, Modifiers, Trigger};

use crate::handle::{OasisInstance, with_instance};
use crate::types::*;

pub(crate) fn button_from_code(code: u32) -> Option<Button> {
    match code {
        OASIS_BUTTON_UP => Some(Button::Up),
        OASIS_BUTTON_DOWN => Some(Button::Down),
        OASIS_BUTTON_LEFT => Some(Button::Left),
        OASIS_BUTTON_RIGHT => Some(Button::Right),
        OASIS_BUTTON_CONFIRM => Some(Button::Confirm),
        OASIS_BUTTON_CANCEL => Some(Button::Cancel),
        OASIS_BUTTON_TRIANGLE => Some(Button::Triangle),
        OASIS_BUTTON_SQUARE => Some(Button::Square),
        OASIS_BUTTON_START => Some(Button::Start),
        OASIS_BUTTON_SELECT => Some(Button::Select),
        _ => None,
    }
}

pub(crate) fn trigger_from_code(code: u32) -> Option<Trigger> {
    match code {
        OASIS_TRIGGER_LEFT => Some(Trigger::Left),
        OASIS_TRIGGER_RIGHT => Some(Trigger::Right),
        _ => None,
    }
}

/// Map an `OASIS_KEY_*` code (+ codepoint for `OASIS_KEY_CHAR`) to a [`Key`].
pub(crate) fn key_from_code(code: u32, character: u32) -> Option<Key> {
    let key = match code {
        OASIS_KEY_CHAR => {
            let ch = char::from_u32(character)?;
            if ch == ' ' {
                Key::Space
            } else if ch.is_control() || ch.is_whitespace() {
                return None;
            } else {
                // `Key::Char` letters are lowercase by contract.
                let mut lower = ch.to_lowercase();
                match (lower.next(), lower.next()) {
                    (Some(l), None) => Key::Char(l),
                    _ => Key::Char(ch),
                }
            }
        },
        OASIS_KEY_SPACE => Key::Space,
        OASIS_KEY_ENTER => Key::Enter,
        OASIS_KEY_ESCAPE => Key::Escape,
        OASIS_KEY_TAB => Key::Tab,
        OASIS_KEY_BACKSPACE => Key::Backspace,
        OASIS_KEY_DELETE => Key::Delete,
        OASIS_KEY_INSERT => Key::Insert,
        OASIS_KEY_HOME => Key::Home,
        OASIS_KEY_END => Key::End,
        OASIS_KEY_PAGE_UP => Key::PageUp,
        OASIS_KEY_PAGE_DOWN => Key::PageDown,
        OASIS_KEY_UP => Key::Up,
        OASIS_KEY_DOWN => Key::Down,
        OASIS_KEY_LEFT => Key::Left,
        OASIS_KEY_RIGHT => Key::Right,
        // Range is 101..=112, so the narrowing cast is lossless.
        OASIS_KEY_F1..=OASIS_KEY_F12 => Key::F((code - OASIS_KEY_F1 + 1) as u8),
        _ => return None,
    };
    Some(key)
}

/// Map an `OASIS_MOD_*` bitmask (passed in the `x` field) to [`Modifiers`].
/// Unknown bits are ignored.
pub(crate) fn mods_from_bits(bits: i32) -> Modifiers {
    // Only the low 4 bits are meaningful; truncation is intended.
    Modifiers::from_bits_truncate((bits & 0xF) as u8)
}

/// Convert a C input event into an [`InputEvent`], or `None` for unknown
/// event types / invalid payloads.
pub(crate) fn event_from_c(evt: &OasisInputEvent) -> Option<InputEvent> {
    match evt.event_type {
        OASIS_EVENT_CURSOR_MOVE => Some(InputEvent::CursorMove { x: evt.x, y: evt.y }),
        OASIS_EVENT_BUTTON_PRESS => button_from_code(evt.key).map(InputEvent::ButtonPress),
        OASIS_EVENT_BUTTON_RELEASE => button_from_code(evt.key).map(InputEvent::ButtonRelease),
        OASIS_EVENT_TRIGGER_PRESS => trigger_from_code(evt.key).map(InputEvent::TriggerPress),
        OASIS_EVENT_TRIGGER_RELEASE => trigger_from_code(evt.key).map(InputEvent::TriggerRelease),
        OASIS_EVENT_TEXT_INPUT => char::from_u32(evt.character).map(InputEvent::TextInput),
        OASIS_EVENT_POINTER_CLICK => Some(InputEvent::PointerClick { x: evt.x, y: evt.y }),
        OASIS_EVENT_POINTER_RELEASE => Some(InputEvent::PointerRelease { x: evt.x, y: evt.y }),
        OASIS_EVENT_FOCUS_GAINED => Some(InputEvent::FocusGained),
        OASIS_EVENT_FOCUS_LOST => Some(InputEvent::FocusLost),
        OASIS_EVENT_QUIT => Some(InputEvent::Quit),
        OASIS_EVENT_BACKSPACE => Some(InputEvent::Backspace),
        OASIS_EVENT_MOUSE_WHEEL => Some(InputEvent::MouseWheel { delta: evt.y }),
        OASIS_EVENT_TOGGLE_FULLSCREEN => Some(InputEvent::ToggleFullscreen),
        OASIS_EVENT_TAB => Some(InputEvent::Tab),
        OASIS_EVENT_SHIFT_TAB => Some(InputEvent::ShiftTab),
        OASIS_EVENT_KEY => key_from_code(evt.key, evt.character).map(|key| InputEvent::Key {
            key,
            mods: mods_from_bits(evt.x),
        }),
        _ => None,
    }
}

/// Deliver an input event to the OS instance.
///
/// # Safety
///
/// `handle` must be valid and non-null. `event` must point to a valid
/// `OasisInputEvent`.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_send_input(
    handle: *mut OasisInstance,
    event: *const OasisInputEvent,
) {
    // SAFETY: Caller guarantees `event` is valid and non-null per function safety contract.
    let Some(evt) = (unsafe { event.as_ref() }) else {
        return;
    };

    if let Some(ie) = event_from_c(evt) {
        // SAFETY: Caller guarantees `handle` is valid per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                instance.input.push_event(ie);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // -- button_from_code tests --

    #[test]
    fn button_code_up() {
        assert_eq!(button_from_code(OASIS_BUTTON_UP), Some(Button::Up));
    }

    #[test]
    fn button_code_down() {
        assert_eq!(button_from_code(OASIS_BUTTON_DOWN), Some(Button::Down));
    }

    #[test]
    fn button_code_left() {
        assert_eq!(button_from_code(OASIS_BUTTON_LEFT), Some(Button::Left));
    }

    #[test]
    fn button_code_right() {
        assert_eq!(button_from_code(OASIS_BUTTON_RIGHT), Some(Button::Right));
    }

    #[test]
    fn button_code_confirm() {
        assert_eq!(
            button_from_code(OASIS_BUTTON_CONFIRM),
            Some(Button::Confirm)
        );
    }

    #[test]
    fn button_code_cancel() {
        assert_eq!(button_from_code(OASIS_BUTTON_CANCEL), Some(Button::Cancel));
    }

    #[test]
    fn button_code_triangle() {
        assert_eq!(
            button_from_code(OASIS_BUTTON_TRIANGLE),
            Some(Button::Triangle)
        );
    }

    #[test]
    fn button_code_square() {
        assert_eq!(button_from_code(OASIS_BUTTON_SQUARE), Some(Button::Square));
    }

    #[test]
    fn button_code_start() {
        assert_eq!(button_from_code(OASIS_BUTTON_START), Some(Button::Start));
    }

    #[test]
    fn button_code_select() {
        assert_eq!(button_from_code(OASIS_BUTTON_SELECT), Some(Button::Select));
    }

    #[test]
    fn button_code_invalid_returns_none() {
        assert_eq!(button_from_code(99), None);
        assert_eq!(button_from_code(u32::MAX), None);
    }

    // -- trigger_from_code tests --

    #[test]
    fn trigger_code_left() {
        assert_eq!(trigger_from_code(OASIS_TRIGGER_LEFT), Some(Trigger::Left));
    }

    #[test]
    fn trigger_code_right() {
        assert_eq!(trigger_from_code(OASIS_TRIGGER_RIGHT), Some(Trigger::Right));
    }

    #[test]
    fn trigger_code_invalid_returns_none() {
        assert_eq!(trigger_from_code(99), None);
        assert_eq!(trigger_from_code(u32::MAX), None);
    }

    // -- Button code constant ordering --

    #[test]
    fn button_codes_are_sequential() {
        assert_eq!(OASIS_BUTTON_UP, 0);
        assert_eq!(OASIS_BUTTON_DOWN, 1);
        assert_eq!(OASIS_BUTTON_LEFT, 2);
        assert_eq!(OASIS_BUTTON_RIGHT, 3);
        assert_eq!(OASIS_BUTTON_CONFIRM, 4);
        assert_eq!(OASIS_BUTTON_CANCEL, 5);
        assert_eq!(OASIS_BUTTON_TRIANGLE, 6);
        assert_eq!(OASIS_BUTTON_SQUARE, 7);
        assert_eq!(OASIS_BUTTON_START, 8);
        assert_eq!(OASIS_BUTTON_SELECT, 9);
    }

    #[test]
    fn trigger_codes_are_sequential() {
        assert_eq!(OASIS_TRIGGER_LEFT, 0);
        assert_eq!(OASIS_TRIGGER_RIGHT, 1);
    }

    // -- event_from_c: every InputEvent variant is reachable --

    fn ev(event_type: u32, x: i32, y: i32, key: u32, character: u32) -> OasisInputEvent {
        OasisInputEvent {
            event_type,
            x,
            y,
            key,
            character,
        }
    }

    #[test]
    fn every_input_event_variant_maps() {
        let cases = [
            (
                ev(OASIS_EVENT_CURSOR_MOVE, 1, 2, 0, 0),
                InputEvent::CursorMove { x: 1, y: 2 },
            ),
            (
                ev(OASIS_EVENT_BUTTON_PRESS, 0, 0, OASIS_BUTTON_UP, 0),
                InputEvent::ButtonPress(Button::Up),
            ),
            (
                ev(OASIS_EVENT_BUTTON_RELEASE, 0, 0, OASIS_BUTTON_UP, 0),
                InputEvent::ButtonRelease(Button::Up),
            ),
            (
                ev(OASIS_EVENT_TRIGGER_PRESS, 0, 0, OASIS_TRIGGER_LEFT, 0),
                InputEvent::TriggerPress(Trigger::Left),
            ),
            (
                ev(OASIS_EVENT_TRIGGER_RELEASE, 0, 0, OASIS_TRIGGER_LEFT, 0),
                InputEvent::TriggerRelease(Trigger::Left),
            ),
            (
                ev(OASIS_EVENT_TEXT_INPUT, 0, 0, 0, 'x' as u32),
                InputEvent::TextInput('x'),
            ),
            (
                ev(OASIS_EVENT_POINTER_CLICK, 3, 4, 0, 0),
                InputEvent::PointerClick { x: 3, y: 4 },
            ),
            (
                ev(OASIS_EVENT_POINTER_RELEASE, 3, 4, 0, 0),
                InputEvent::PointerRelease { x: 3, y: 4 },
            ),
            (
                ev(OASIS_EVENT_FOCUS_GAINED, 0, 0, 0, 0),
                InputEvent::FocusGained,
            ),
            (
                ev(OASIS_EVENT_FOCUS_LOST, 0, 0, 0, 0),
                InputEvent::FocusLost,
            ),
            (ev(OASIS_EVENT_QUIT, 0, 0, 0, 0), InputEvent::Quit),
            (ev(OASIS_EVENT_BACKSPACE, 0, 0, 0, 0), InputEvent::Backspace),
            (
                ev(OASIS_EVENT_MOUSE_WHEEL, 0, -2, 0, 0),
                InputEvent::MouseWheel { delta: -2 },
            ),
            (
                ev(OASIS_EVENT_TOGGLE_FULLSCREEN, 0, 0, 0, 0),
                InputEvent::ToggleFullscreen,
            ),
            (ev(OASIS_EVENT_TAB, 0, 0, 0, 0), InputEvent::Tab),
            (ev(OASIS_EVENT_SHIFT_TAB, 0, 0, 0, 0), InputEvent::ShiftTab),
            (
                ev(
                    OASIS_EVENT_KEY,
                    (OASIS_MOD_CTRL | OASIS_MOD_SHIFT) as i32,
                    0,
                    OASIS_KEY_CHAR,
                    'S' as u32,
                ),
                InputEvent::Key {
                    key: Key::Char('s'),
                    mods: Modifiers::CTRL | Modifiers::SHIFT,
                },
            ),
        ];
        for (c_event, expected) in cases {
            assert_eq!(event_from_c(&c_event), Some(expected));
        }
        assert_eq!(event_from_c(&ev(999, 0, 0, 0, 0)), None);
    }

    #[test]
    fn key_codes_map() {
        assert_eq!(key_from_code(OASIS_KEY_DELETE, 0), Some(Key::Delete));
        assert_eq!(key_from_code(OASIS_KEY_HOME, 0), Some(Key::Home));
        assert_eq!(key_from_code(OASIS_KEY_END, 0), Some(Key::End));
        assert_eq!(key_from_code(OASIS_KEY_PAGE_UP, 0), Some(Key::PageUp));
        assert_eq!(key_from_code(OASIS_KEY_PAGE_DOWN, 0), Some(Key::PageDown));
        assert_eq!(key_from_code(OASIS_KEY_F1, 0), Some(Key::F(1)));
        assert_eq!(key_from_code(OASIS_KEY_F12, 0), Some(Key::F(12)));
        assert_eq!(key_from_code(OASIS_KEY_F12 + 1, 0), None);
        assert_eq!(key_from_code(OASIS_KEY_CHAR, ' ' as u32), Some(Key::Space));
        assert_eq!(key_from_code(OASIS_KEY_CHAR, '\n' as u32), None);
        assert_eq!(key_from_code(OASIS_KEY_CHAR, 0xD800), None);
    }

    #[test]
    fn modifier_bits_match_modifiers() {
        assert_eq!(mods_from_bits(OASIS_MOD_SHIFT as i32), Modifiers::SHIFT);
        assert_eq!(mods_from_bits(OASIS_MOD_CTRL as i32), Modifiers::CTRL);
        assert_eq!(mods_from_bits(OASIS_MOD_ALT as i32), Modifiers::ALT);
        assert_eq!(mods_from_bits(OASIS_MOD_SUPER as i32), Modifiers::SUPER);
        assert_eq!(mods_from_bits(-1), Modifiers::from_bits_truncate(0xF));
    }
}
