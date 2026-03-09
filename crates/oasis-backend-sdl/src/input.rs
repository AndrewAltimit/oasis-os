//! Input event handling for the SDL2 backend.
//!
//! Maps SDL2 keyboard, mouse, and window events to OASIS_OS `InputEvent`s.

use sdl2::event::Event;
use sdl2::keyboard::Keycode;

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

/// Map an SDL2 event to an OASIS_OS input event.
pub(crate) fn map_sdl_event(event: Event) -> Option<InputEvent> {
    match event {
        Event::Quit { .. } => Some(InputEvent::Quit),
        Event::KeyDown {
            keycode: Some(key), ..
        } => map_key_down(key),
        Event::KeyUp {
            keycode: Some(key), ..
        } => map_key_up(key),
        Event::MouseMotion { x, y, .. } => Some(InputEvent::CursorMove { x, y }),
        Event::MouseButtonDown { x, y, .. } => Some(InputEvent::PointerClick { x, y }),
        Event::MouseButtonUp { x, y, .. } => Some(InputEvent::PointerRelease { x, y }),
        Event::MouseWheel { y, .. } => Some(InputEvent::MouseWheel { delta: -y }),
        Event::Window {
            win_event: sdl2::event::WindowEvent::FocusGained,
            ..
        } => Some(InputEvent::FocusGained),
        Event::Window {
            win_event: sdl2::event::WindowEvent::FocusLost,
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
        Keycode::Tab => Some(InputEvent::ButtonPress(Button::Square)),
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
        Keycode::Tab => Some(InputEvent::ButtonRelease(Button::Square)),
        Keycode::F1 => Some(InputEvent::ButtonRelease(Button::Start)),
        Keycode::F2 => Some(InputEvent::ButtonRelease(Button::Select)),
        Keycode::Q => Some(InputEvent::TriggerRelease(Trigger::Left)),
        Keycode::E => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}
