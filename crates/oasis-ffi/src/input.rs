//! Input event marshaling from C types to Rust `InputEvent`.

use oasis_core::input::{Button, InputEvent, Trigger};

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

    let input_event = match evt.event_type {
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
        _ => None,
    };

    if let Some(ie) = input_event {
        // SAFETY: Caller guarantees `handle` is valid per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                instance.input.push_event(ie);
            });
        }
    }
}
