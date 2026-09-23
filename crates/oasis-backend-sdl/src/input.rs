//! Input event handling for the SDL3 backend.
//!
//! Maps SDL3 keyboard, mouse, and window events to OASIS_OS `InputEvent`s.
//!
//! Every key-down produces an [`InputEvent::Key`] (physical key +
//! modifiers) followed immediately by its gamepad-style twin from
//! [`Key::legacy_press`], if the key has one. See `oasis_types::input` for
//! the ordering contract hosts rely on.

use sdl3::event::Event;
use sdl3::keyboard::{Keycode, Mod};

use oasis_core::input::{InputEvent, Key, Modifiers};

use super::SdlBackend;

impl oasis_core::backend::InputBackend for SdlBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for event in self.event_pump.poll_iter() {
            map_sdl_event(event, &mut events);
        }
        events
    }
}

/// Map an SDL3 event to zero or more OASIS_OS input events, appended to
/// `out` in delivery order.
pub(crate) fn map_sdl_event(event: Event, out: &mut Vec<InputEvent>) {
    let mapped = match event {
        Event::Quit { .. } => Some(InputEvent::Quit),
        Event::KeyDown {
            keycode: Some(keycode),
            keymod,
            ..
        } => {
            if let Some(key) = map_keycode(keycode) {
                let mods = map_keymod(keymod);
                out.push(InputEvent::Key { key, mods });
                key.legacy_press(mods)
            } else {
                None
            }
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
    };
    if let Some(ev) = mapped {
        out.push(ev);
    }
}

/// Map an SDL keycode to a backend-independent [`Key`].
///
/// SDL3 keycodes for printable keys are the key's unshifted Unicode
/// codepoint (letters lowercase), so those map straight to [`Key::Char`].
pub(crate) fn map_keycode(key: Keycode) -> Option<Key> {
    let k = match key {
        Keycode::Up => Key::Up,
        Keycode::Down => Key::Down,
        Keycode::Left => Key::Left,
        Keycode::Right => Key::Right,
        Keycode::Return | Keycode::KpEnter => Key::Enter,
        Keycode::Escape => Key::Escape,
        Keycode::Tab => Key::Tab,
        Keycode::Backspace => Key::Backspace,
        Keycode::Delete => Key::Delete,
        Keycode::Insert => Key::Insert,
        Keycode::Home => Key::Home,
        Keycode::End => Key::End,
        Keycode::PageUp => Key::PageUp,
        Keycode::PageDown => Key::PageDown,
        Keycode::Space => Key::Space,
        Keycode::F1 => Key::F(1),
        Keycode::F2 => Key::F(2),
        Keycode::F3 => Key::F(3),
        Keycode::F4 => Key::F(4),
        Keycode::F5 => Key::F(5),
        Keycode::F6 => Key::F(6),
        Keycode::F7 => Key::F(7),
        Keycode::F8 => Key::F(8),
        Keycode::F9 => Key::F(9),
        Keycode::F10 => Key::F(10),
        Keycode::F11 => Key::F(11),
        Keycode::F12 => Key::F(12),
        other => {
            let ch = char::from_u32(other as u32)?;
            if ch.is_control() || ch.is_whitespace() {
                return None;
            }
            Key::Char(ch.to_ascii_lowercase())
        },
    };
    Some(k)
}

/// Collapse SDL's left/right modifier bits into [`Modifiers`].
pub(crate) fn map_keymod(m: Mod) -> Modifiers {
    let mut mods = Modifiers::NONE;
    if m.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) {
        mods |= Modifiers::SHIFT;
    }
    if m.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) {
        mods |= Modifiers::CTRL;
    }
    if m.intersects(Mod::LALTMOD | Mod::RALTMOD) {
        mods |= Modifiers::ALT;
    }
    if m.intersects(Mod::LGUIMOD | Mod::RGUIMOD) {
        mods |= Modifiers::SUPER;
    }
    mods
}

/// Gamepad-style event for a key-down with no modifiers held.
///
/// Tab is excluded (it depends on Shift, so `map_sdl_event` derives it
/// from the real modifiers). Test helper; the live path is `map_sdl_event`.
#[cfg(test)]
pub(crate) fn map_key_down(key: Keycode) -> Option<InputEvent> {
    if key == Keycode::Tab {
        return None;
    }
    map_keycode(key)?.legacy_press(Modifiers::NONE)
}

/// Gamepad-style event for a key-up.
pub(crate) fn map_key_up(key: Keycode) -> Option<InputEvent> {
    map_keycode(key)?.legacy_release()
}

// -----------------------------------------------------------------------
// Item 69: SDL input mapping tests (20 tests)
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use oasis_core::input::{Button, Trigger};

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

    // -- Raw Key mapping tests --

    fn key_down(keycode: Keycode, keymod: Mod) -> Vec<InputEvent> {
        let mut out = Vec::new();
        map_sdl_event(
            Event::KeyDown {
                timestamp: 0,
                window_id: 0,
                keycode: Some(keycode),
                scancode: None,
                keymod,
                repeat: false,
                which: 0,
                raw: 0,
            },
            &mut out,
        );
        out
    }

    #[test]
    fn keycode_navigation_keys() {
        assert_eq!(map_keycode(Keycode::Delete), Some(Key::Delete));
        assert_eq!(map_keycode(Keycode::Home), Some(Key::Home));
        assert_eq!(map_keycode(Keycode::End), Some(Key::End));
        assert_eq!(map_keycode(Keycode::PageUp), Some(Key::PageUp));
        assert_eq!(map_keycode(Keycode::PageDown), Some(Key::PageDown));
        assert_eq!(map_keycode(Keycode::Insert), Some(Key::Insert));
        assert_eq!(map_keycode(Keycode::KpEnter), Some(Key::Enter));
        assert_eq!(map_keycode(Keycode::Backspace), Some(Key::Backspace));
        assert_eq!(map_keycode(Keycode::Space), Some(Key::Space));
    }

    #[test]
    fn keycode_function_keys() {
        assert_eq!(map_keycode(Keycode::F1), Some(Key::F(1)));
        assert_eq!(map_keycode(Keycode::F5), Some(Key::F(5)));
        assert_eq!(map_keycode(Keycode::F12), Some(Key::F(12)));
    }

    #[test]
    fn keycode_printable_chars() {
        assert_eq!(map_keycode(Keycode::A), Some(Key::Char('a')));
        assert_eq!(map_keycode(Keycode::S), Some(Key::Char('s')));
        assert_eq!(map_keycode(Keycode::_0), Some(Key::Char('0')));
        assert_eq!(map_keycode(Keycode::Slash), Some(Key::Char('/')));
        assert_eq!(map_keycode(Keycode::LCtrl), None);
    }

    #[test]
    fn keymod_collapses_left_right() {
        assert_eq!(map_keymod(Mod::NOMOD), Modifiers::NONE);
        assert_eq!(map_keymod(Mod::RCTRLMOD), Modifiers::CTRL);
        assert_eq!(
            map_keymod(Mod::LSHIFTMOD | Mod::LALTMOD | Mod::RGUIMOD),
            Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER
        );
        // Caps / Num lock are not modifiers.
        assert_eq!(map_keymod(Mod::CAPSMOD | Mod::NUMMOD), Modifiers::NONE);
    }

    #[test]
    fn keydown_emits_key_then_legacy_twin() {
        assert_eq!(
            key_down(Keycode::Q, Mod::NOMOD),
            vec![
                InputEvent::Key {
                    key: Key::Char('q'),
                    mods: Modifiers::NONE,
                },
                InputEvent::TriggerPress(Trigger::Left),
            ]
        );
        assert_eq!(
            key_down(Keycode::Space, Mod::NOMOD),
            vec![
                InputEvent::Key {
                    key: Key::Space,
                    mods: Modifiers::NONE,
                },
                InputEvent::ButtonPress(Button::Triangle),
            ]
        );
    }

    #[test]
    fn keydown_shortcut_key_only() {
        assert_eq!(
            key_down(Keycode::S, Mod::LCTRLMOD),
            vec![InputEvent::Key {
                key: Key::Char('s'),
                mods: Modifiers::CTRL,
            }]
        );
        assert_eq!(
            key_down(Keycode::Delete, Mod::NOMOD),
            vec![InputEvent::Key {
                key: Key::Delete,
                mods: Modifiers::NONE,
            }]
        );
    }

    #[test]
    fn keydown_tab_respects_shift() {
        assert_eq!(
            key_down(Keycode::Tab, Mod::NOMOD).last(),
            Some(&InputEvent::Tab)
        );
        assert_eq!(
            key_down(Keycode::Tab, Mod::RSHIFTMOD).last(),
            Some(&InputEvent::ShiftTab)
        );
    }
}
