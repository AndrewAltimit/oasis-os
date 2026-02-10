//! Controller input via `psp::input::Controller`.
//!
//! Uses the high-level Controller API for automatic edge detection
//! (press/release) and normalized analog stick with deadzone.

use psp::sys::CtrlButtons;

use oasis_core::input::{Button, InputEvent, Trigger};

use crate::PspBackend;

/// Analog stick deadzone fraction (0.0–1.0). ~31% matches the old
/// integer deadzone of 40/128.
const ANALOG_DEADZONE: f32 = 0.31;

/// Cursor speed multiplier for analog stick movement.
const CURSOR_SPEED: f32 = 8.0;

/// Button-to-event mapping table for digital buttons.
const BUTTON_MAP: &[(CtrlButtons, Button)] = &[
    (CtrlButtons::UP, Button::Up),
    (CtrlButtons::DOWN, Button::Down),
    (CtrlButtons::LEFT, Button::Left),
    (CtrlButtons::RIGHT, Button::Right),
    (CtrlButtons::CROSS, Button::Confirm),
    (CtrlButtons::CIRCLE, Button::Cancel),
    (CtrlButtons::TRIANGLE, Button::Triangle),
    (CtrlButtons::SQUARE, Button::Square),
    (CtrlButtons::START, Button::Start),
    (CtrlButtons::SELECT, Button::Select),
];

/// Trigger-to-event mapping table for shoulder buttons.
const TRIGGER_MAP: &[(CtrlButtons, Trigger)] = &[
    (CtrlButtons::LTRIGGER, Trigger::Left),
    (CtrlButtons::RTRIGGER, Trigger::Right),
];

impl PspBackend {
    /// Poll controller input, returning events with edge detection.
    pub fn poll_events_inner(&mut self) -> Vec<InputEvent> {
        self.controller.update();
        let mut events = Vec::new();

        // Digital buttons.
        for &(psp_btn, btn) in BUTTON_MAP {
            if self.controller.is_pressed(psp_btn) {
                events.push(InputEvent::ButtonPress(btn));
            }
            if self.controller.is_released(psp_btn) {
                events.push(InputEvent::ButtonRelease(btn));
            }
        }

        // Shoulder triggers.
        for &(psp_btn, trigger) in TRIGGER_MAP {
            if self.controller.is_pressed(psp_btn) {
                events.push(InputEvent::TriggerPress(trigger));
            }
            if self.controller.is_released(psp_btn) {
                events.push(InputEvent::TriggerRelease(trigger));
            }
        }

        // Analog stick -> cursor movement.
        let ax = self.controller.analog_x_f32(ANALOG_DEADZONE);
        let ay = self.controller.analog_y_f32(ANALOG_DEADZONE);
        if ax != 0.0 || ay != 0.0 {
            let move_x = (ax * CURSOR_SPEED) as i32;
            let move_y = (ay * CURSOR_SPEED) as i32;
            self.cursor_x =
                (self.cursor_x + move_x).clamp(0, self.width as i32 - 1);
            self.cursor_y =
                (self.cursor_y + move_y).clamp(0, self.height as i32 - 1);
            events.push(InputEvent::CursorMove {
                x: self.cursor_x,
                y: self.cursor_y,
            });
        }

        events
    }
}

impl oasis_core::backend::InputBackend for PspBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        self.poll_events_inner()
    }
}
