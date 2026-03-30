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
const CURSOR_SPEED: f32 = 5.0;

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
    /// Also drains any injected events from the TCP command server.
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
            self.cursor_x = (self.cursor_x + move_x).clamp(0, self.width as i32 - 1);
            self.cursor_y = (self.cursor_y + move_y).clamp(0, self.height as i32 - 1);
            events.push(InputEvent::CursorMove {
                x: self.cursor_x,
                y: self.cursor_y,
            });
        }

        // Drain injected events from TCP command server.
        // Update internal cursor position for injected CursorMove events
        // so hit-testing uses the correct coordinates.
        let pre_len = events.len();
        crate::cmd_server::drain_injected(&mut events);
        for ev in &events[pre_len..] {
            if let InputEvent::CursorMove { x, y } = ev {
                self.cursor_x = *x;
                self.cursor_y = *y;
            }
        }

        events
    }
}

impl oasis_core::backend::InputBackend for PspBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        self.poll_events_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Button mapping table tests --

    #[test]
    fn button_map_has_10_entries() {
        assert_eq!(BUTTON_MAP.len(), 10);
    }

    #[test]
    fn trigger_map_has_2_entries() {
        assert_eq!(TRIGGER_MAP.len(), 2);
    }

    #[test]
    fn button_map_covers_all_buttons() {
        let mapped: Vec<Button> = BUTTON_MAP.iter().map(|(_, btn)| *btn).collect();
        assert!(mapped.contains(&Button::Up));
        assert!(mapped.contains(&Button::Down));
        assert!(mapped.contains(&Button::Left));
        assert!(mapped.contains(&Button::Right));
        assert!(mapped.contains(&Button::Confirm));
        assert!(mapped.contains(&Button::Cancel));
        assert!(mapped.contains(&Button::Triangle));
        assert!(mapped.contains(&Button::Square));
        assert!(mapped.contains(&Button::Start));
        assert!(mapped.contains(&Button::Select));
    }

    #[test]
    fn trigger_map_covers_both_triggers() {
        let mapped: Vec<Trigger> = TRIGGER_MAP.iter().map(|(_, t)| *t).collect();
        assert!(mapped.contains(&Trigger::Left));
        assert!(mapped.contains(&Trigger::Right));
    }

    // -- Constant value tests --

    #[test]
    fn analog_deadzone_is_031() {
        assert!((ANALOG_DEADZONE - 0.31).abs() < f32::EPSILON);
    }

    #[test]
    fn cursor_speed_is_5() {
        assert!((CURSOR_SPEED - 5.0).abs() < f32::EPSILON);
    }
}
