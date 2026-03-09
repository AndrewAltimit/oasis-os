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

        events
    }
}

/// Compute cursor movement from normalized analog stick values.
///
/// Returns `(dx, dy)` in pixels, or `(0, 0)` if both axes are zero.
/// `ax` and `ay` are the deadzone-filtered analog values in `[-1.0, 1.0]`.
pub(crate) fn analog_to_cursor_delta(ax: f32, ay: f32) -> (i32, i32) {
    if ax == 0.0 && ay == 0.0 {
        return (0, 0);
    }
    ((ax * CURSOR_SPEED) as i32, (ay * CURSOR_SPEED) as i32)
}

/// Clamp cursor position within screen bounds.
pub(crate) fn clamp_cursor(
    cursor_x: i32,
    cursor_y: i32,
    dx: i32,
    dy: i32,
    width: u32,
    height: u32,
) -> (i32, i32) {
    let x = (cursor_x + dx).clamp(0, width as i32 - 1);
    let y = (cursor_y + dy).clamp(0, height as i32 - 1);
    (x, y)
}

/// Apply analog deadzone: returns 0.0 if `|value| < deadzone`.
///
/// This mirrors the logic in `psp::input::Controller::analog_x_f32`.
pub(crate) fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    if value.abs() < deadzone { 0.0 } else { value }
}

impl oasis_core::backend::InputBackend for PspBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        self.poll_events_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Analog deadzone tests --

    #[test]
    fn deadzone_zero_input() {
        assert_eq!(apply_deadzone(0.0, ANALOG_DEADZONE), 0.0);
    }

    #[test]
    fn deadzone_below_threshold() {
        assert_eq!(apply_deadzone(0.30, ANALOG_DEADZONE), 0.0);
        assert_eq!(apply_deadzone(-0.30, ANALOG_DEADZONE), 0.0);
    }

    #[test]
    fn deadzone_at_threshold() {
        // Exactly at 0.31 is not less than 0.31, so it passes.
        assert_ne!(apply_deadzone(0.31, ANALOG_DEADZONE), 0.0);
    }

    #[test]
    fn deadzone_above_threshold() {
        assert_eq!(apply_deadzone(0.5, ANALOG_DEADZONE), 0.5);
        assert_eq!(apply_deadzone(-0.8, ANALOG_DEADZONE), -0.8);
        assert_eq!(apply_deadzone(1.0, ANALOG_DEADZONE), 1.0);
    }

    // -- Cursor speed tests --

    #[test]
    fn analog_to_cursor_delta_zero() {
        assert_eq!(analog_to_cursor_delta(0.0, 0.0), (0, 0));
    }

    #[test]
    fn analog_to_cursor_delta_full_right() {
        let (dx, dy) = analog_to_cursor_delta(1.0, 0.0);
        assert_eq!(dx, 5); // 1.0 * 5.0 = 5
        assert_eq!(dy, 0);
    }

    #[test]
    fn analog_to_cursor_delta_full_down() {
        let (dx, dy) = analog_to_cursor_delta(0.0, 1.0);
        assert_eq!(dx, 0);
        assert_eq!(dy, 5);
    }

    #[test]
    fn analog_to_cursor_delta_negative() {
        let (dx, dy) = analog_to_cursor_delta(-1.0, -1.0);
        assert_eq!(dx, -5);
        assert_eq!(dy, -5);
    }

    #[test]
    fn analog_to_cursor_delta_fractional() {
        // 0.5 * 5.0 = 2.5 -> truncated to 2.
        let (dx, dy) = analog_to_cursor_delta(0.5, 0.5);
        assert_eq!(dx, 2);
        assert_eq!(dy, 2);
    }

    // -- Cursor clamping tests --

    #[test]
    fn clamp_cursor_center() {
        let (x, y) = clamp_cursor(240, 136, 0, 0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    #[test]
    fn clamp_cursor_at_origin() {
        let (x, y) = clamp_cursor(0, 0, -5, -5, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn clamp_cursor_at_max() {
        let (x, y) = clamp_cursor(479, 271, 5, 5, 480, 272);
        assert_eq!(x, 479);
        assert_eq!(y, 271);
    }

    #[test]
    fn clamp_cursor_normal_movement() {
        let (x, y) = clamp_cursor(100, 100, 3, -2, 480, 272);
        assert_eq!(x, 103);
        assert_eq!(y, 98);
    }

    #[test]
    fn clamp_cursor_large_negative_delta() {
        let (x, y) = clamp_cursor(10, 10, -100, -100, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

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
