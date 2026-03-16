//! Input event handling for the SDL3 backend.
//!
//! Maps SDL3 keyboard, mouse, and window events to OASIS_OS `InputEvent`s.

use sdl3::event::Event;
use sdl3::keyboard::Keycode;

use oasis_core::input::{Button, InputEvent, Trigger};

use super::SdlBackend;

impl oasis_core::backend::InputBackend for SdlBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for event in self.event_pump.poll_iter() {
            if let Some(e) = map_sdl_event(event) {
                events.push(e);
            }
        }
        events
    }
}

/// Map an SDL3 event to an OASIS_OS input event.
pub(crate) fn map_sdl_event(event: Event) -> Option<InputEvent> {
    match event {
        Event::Quit { .. } => Some(InputEvent::Quit),
        Event::KeyDown {
            keycode: Some(key),
            keymod,
            ..
        } => {
            if key == Keycode::Tab {
                if keymod
                    .intersects(sdl3::keyboard::Mod::LSHIFTMOD | sdl3::keyboard::Mod::RSHIFTMOD)
                {
                    return Some(InputEvent::ShiftTab);
                }
                return Some(InputEvent::Tab);
            }
            map_key_down(key)
        },
        Event::KeyUp {
            keycode: Some(key), ..
        } => map_key_up(key),
        // SDL3 mouse coordinates are f32; truncate to i32.
        Event::MouseMotion { x, y, .. } => Some(InputEvent::CursorMove {
            x: x as i32,
            y: y as i32,
        }),
        Event::MouseButtonDown { x, y, .. } => Some(InputEvent::PointerClick {
            x: x as i32,
            y: y as i32,
        }),
        Event::MouseButtonUp { x, y, .. } => Some(InputEvent::PointerRelease {
            x: x as i32,
            y: y as i32,
        }),
        // SDL3 mouse wheel y is f32; truncate to i32.
        Event::MouseWheel { y, .. } => Some(InputEvent::MouseWheel { delta: -(y as i32) }),
        Event::Window {
            win_event: sdl3::event::WindowEvent::FocusGained,
            ..
        } => Some(InputEvent::FocusGained),
        Event::Window {
            win_event: sdl3::event::WindowEvent::FocusLost,
            ..
        } => Some(InputEvent::FocusLost),
        Event::TextInput { text, .. } => text.chars().next().map(InputEvent::TextInput),
        _ => None,
    }
}

pub(crate) fn map_key_down(key: Keycode) -> Option<InputEvent> {
    match key {
        Keycode::Up => Some(InputEvent::ButtonPress(Button::Up)),
        Keycode::Down => Some(InputEvent::ButtonPress(Button::Down)),
        Keycode::Left => Some(InputEvent::ButtonPress(Button::Left)),
        Keycode::Right => Some(InputEvent::ButtonPress(Button::Right)),
        Keycode::Return => Some(InputEvent::ButtonPress(Button::Confirm)),
        Keycode::Escape => Some(InputEvent::ButtonPress(Button::Cancel)),
        Keycode::Space => Some(InputEvent::ButtonPress(Button::Triangle)),
        Keycode::F1 => Some(InputEvent::ButtonPress(Button::Start)),
        Keycode::F2 => Some(InputEvent::ButtonPress(Button::Select)),
        Keycode::Backspace => Some(InputEvent::Backspace),
        Keycode::Q => Some(InputEvent::TriggerPress(Trigger::Left)),
        Keycode::E => Some(InputEvent::TriggerPress(Trigger::Right)),
        Keycode::F11 => Some(InputEvent::ToggleFullscreen),
        _ => None,
    }
}

pub(crate) fn map_key_up(key: Keycode) -> Option<InputEvent> {
    match key {
        Keycode::Up => Some(InputEvent::ButtonRelease(Button::Up)),
        Keycode::Down => Some(InputEvent::ButtonRelease(Button::Down)),
        Keycode::Left => Some(InputEvent::ButtonRelease(Button::Left)),
        Keycode::Right => Some(InputEvent::ButtonRelease(Button::Right)),
        Keycode::Return => Some(InputEvent::ButtonRelease(Button::Confirm)),
        Keycode::Escape => Some(InputEvent::ButtonRelease(Button::Cancel)),
        Keycode::Space => Some(InputEvent::ButtonRelease(Button::Triangle)),
        Keycode::F1 => Some(InputEvent::ButtonRelease(Button::Start)),
        Keycode::F2 => Some(InputEvent::ButtonRelease(Button::Select)),
        Keycode::Q => Some(InputEvent::TriggerRelease(Trigger::Left)),
        Keycode::E => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}

// -----------------------------------------------------------------------
// Item 69: SDL input mapping tests (20 tests)
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // -- Key down mapping tests --

    #[test]
    fn keydown_arrow_up() {
        assert_eq!(
            map_key_down(Keycode::Up),
            Some(InputEvent::ButtonPress(Button::Up))
        );
    }

    #[test]
    fn keydown_arrow_down() {
        assert_eq!(
            map_key_down(Keycode::Down),
            Some(InputEvent::ButtonPress(Button::Down))
        );
    }

    #[test]
    fn keydown_arrow_left() {
        assert_eq!(
            map_key_down(Keycode::Left),
            Some(InputEvent::ButtonPress(Button::Left))
        );
    }

    #[test]
    fn keydown_arrow_right() {
        assert_eq!(
            map_key_down(Keycode::Right),
            Some(InputEvent::ButtonPress(Button::Right))
        );
    }

    #[test]
    fn keydown_return_maps_to_confirm() {
        assert_eq!(
            map_key_down(Keycode::Return),
            Some(InputEvent::ButtonPress(Button::Confirm))
        );
    }

    #[test]
    fn keydown_escape_maps_to_cancel() {
        assert_eq!(
            map_key_down(Keycode::Escape),
            Some(InputEvent::ButtonPress(Button::Cancel))
        );
    }

    #[test]
    fn keydown_space_maps_to_triangle() {
        assert_eq!(
            map_key_down(Keycode::Space),
            Some(InputEvent::ButtonPress(Button::Triangle))
        );
    }

    #[test]
    fn keydown_tab_not_in_key_down() {
        // Tab is handled in map_sdl_event (with Shift detection), not
        // map_key_down, so it returns None here.
        assert_eq!(map_key_down(Keycode::Tab), None);
    }

    #[test]
    fn keydown_f1_maps_to_start() {
        assert_eq!(
            map_key_down(Keycode::F1),
            Some(InputEvent::ButtonPress(Button::Start))
        );
    }

    #[test]
    fn keydown_f2_maps_to_select() {
        assert_eq!(
            map_key_down(Keycode::F2),
            Some(InputEvent::ButtonPress(Button::Select))
        );
    }

    #[test]
    fn keydown_backspace_maps_to_backspace() {
        assert_eq!(
            map_key_down(Keycode::Backspace),
            Some(InputEvent::Backspace)
        );
    }

    #[test]
    fn keydown_q_maps_to_trigger_left() {
        assert_eq!(
            map_key_down(Keycode::Q),
            Some(InputEvent::TriggerPress(Trigger::Left))
        );
    }

    #[test]
    fn keydown_e_maps_to_trigger_right() {
        assert_eq!(
            map_key_down(Keycode::E),
            Some(InputEvent::TriggerPress(Trigger::Right))
        );
    }

    #[test]
    fn keydown_f11_maps_to_toggle_fullscreen() {
        assert_eq!(
            map_key_down(Keycode::F11),
            Some(InputEvent::ToggleFullscreen)
        );
    }

    #[test]
    fn keydown_unknown_key_returns_none() {
        assert_eq!(map_key_down(Keycode::A), None);
        assert_eq!(map_key_down(Keycode::_0), None);
        assert_eq!(map_key_down(Keycode::F3), None);
    }

    // -- Key up mapping tests --

    #[test]
    fn keyup_arrow_keys() {
        assert_eq!(
            map_key_up(Keycode::Up),
            Some(InputEvent::ButtonRelease(Button::Up))
        );
        assert_eq!(
            map_key_up(Keycode::Down),
            Some(InputEvent::ButtonRelease(Button::Down))
        );
        assert_eq!(
            map_key_up(Keycode::Left),
            Some(InputEvent::ButtonRelease(Button::Left))
        );
        assert_eq!(
            map_key_up(Keycode::Right),
            Some(InputEvent::ButtonRelease(Button::Right))
        );
    }

    #[test]
    fn keyup_confirm_cancel() {
        assert_eq!(
            map_key_up(Keycode::Return),
            Some(InputEvent::ButtonRelease(Button::Confirm))
        );
        assert_eq!(
            map_key_up(Keycode::Escape),
            Some(InputEvent::ButtonRelease(Button::Cancel))
        );
    }

    #[test]
    fn keyup_triggers() {
        assert_eq!(
            map_key_up(Keycode::Q),
            Some(InputEvent::TriggerRelease(Trigger::Left))
        );
        assert_eq!(
            map_key_up(Keycode::E),
            Some(InputEvent::TriggerRelease(Trigger::Right))
        );
    }

    #[test]
    fn keyup_unknown_key_returns_none() {
        assert_eq!(map_key_up(Keycode::A), None);
        assert_eq!(map_key_up(Keycode::Backspace), None);
        assert_eq!(map_key_up(Keycode::F11), None);
    }

    // -- Symmetry test: every key-down mapping has a matching key-up --

    #[test]
    fn keydown_keyup_symmetry() {
        // All keys that produce a ButtonPress on down should produce
        // a ButtonRelease on up (except Backspace and F11 which are
        // down-only).
        // Tab is handled in map_sdl_event (not map_key_down/map_key_up).
        let symmetric_keys = [
            Keycode::Up,
            Keycode::Down,
            Keycode::Left,
            Keycode::Right,
            Keycode::Return,
            Keycode::Escape,
            Keycode::Space,
            Keycode::F1,
            Keycode::F2,
            Keycode::Q,
            Keycode::E,
        ];
        for key in symmetric_keys {
            let down = map_key_down(key);
            let up = map_key_up(key);
            assert!(down.is_some(), "key {key:?} should map on key-down");
            assert!(up.is_some(), "key {key:?} should map on key-up");
            // Verify press/release match the same logical button.
            match (down.unwrap(), up.unwrap()) {
                (InputEvent::ButtonPress(a), InputEvent::ButtonRelease(b)) => {
                    assert_eq!(a, b, "key {key:?} press/release mismatch");
                },
                (InputEvent::TriggerPress(a), InputEvent::TriggerRelease(b)) => {
                    assert_eq!(a, b, "key {key:?} trigger press/release mismatch");
                },
                (d, u) => panic!("key {key:?}: unexpected pair ({d:?}, {u:?})"),
            }
        }
    }
}
