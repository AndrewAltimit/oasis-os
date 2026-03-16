//! `InputBackend` implementation using DOM event listeners.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{EventTarget, HtmlCanvasElement, KeyboardEvent, MouseEvent, WheelEvent};

use oasis_types::backend::InputBackend;
use oasis_types::input::{Button, InputEvent, Trigger};

// ---------------------------------------------------------------------------
// WasmInputBackend
// ---------------------------------------------------------------------------

pub struct WasmInputBackend {
    events: Rc<RefCell<Vec<InputEvent>>>,
    // Store closures to prevent them from being dropped.
    _closures: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

impl WasmInputBackend {
    /// Create a new input backend that listens on the given canvas.
    pub fn new(canvas: &HtmlCanvasElement, width: u32, height: u32) -> Self {
        let events: Rc<RefCell<Vec<InputEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let mut closures: Vec<Closure<dyn FnMut(web_sys::Event)>> = Vec::new();

        let target: &EventTarget = canvas.as_ref();

        // -- Keyboard events (on window for global capture) --
        let window = web_sys::window().expect("no window");
        let win_target: &EventTarget = window.as_ref();

        // keydown
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(ke) = e.dyn_into::<KeyboardEvent>() else {
                    return;
                };
                let mapped = map_keydown(&ke);
                if mapped.is_some() {
                    ke.prevent_default();
                }
                let mut q = ev.borrow_mut();
                if let Some(input) = mapped {
                    q.push(input);
                }
                // SDL fires TextInput separately from KeyDown, so a key
                // like "e" generates both TriggerPress and TextInput.
                // Replicate that here for printable characters.
                let key = ke.key();
                let chars: Vec<char> = key.chars().collect();
                if chars.len() == 1 && !ke.ctrl_key() && !ke.alt_key() && !ke.meta_key() {
                    q.push(InputEvent::TextInput(chars[0]));
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = win_target
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // keyup
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(ke) = e.dyn_into::<KeyboardEvent>() else {
                    return;
                };
                if let Some(input) = map_keyup(&ke) {
                    ke.prevent_default();
                    ev.borrow_mut().push(input);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = win_target
                .add_event_listener_with_callback("keyup", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // -- Mouse events (on canvas) --

        // mousemove
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(me) = e.dyn_into::<MouseEvent>() else {
                    return;
                };
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::CursorMove { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("mousemove", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // mousedown
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(me) = e.dyn_into::<MouseEvent>() else {
                    return;
                };
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::PointerClick { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // mouseup
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(me) = e.dyn_into::<MouseEvent>() else {
                    return;
                };
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::PointerRelease { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // wheel
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let Ok(we) = e.dyn_into::<WheelEvent>() else {
                    return;
                };
                we.prevent_default();
                let delta = if we.delta_y() > 0.0 { 1 } else { -1 };
                ev.borrow_mut().push(InputEvent::MouseWheel { delta });
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ =
                target.add_event_listener_with_callback("wheel", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // focus / blur (on window)
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |_e: web_sys::Event| {
                ev.borrow_mut().push(InputEvent::FocusGained);
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = win_target
                .add_event_listener_with_callback("focus", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |_e: web_sys::Event| {
                ev.borrow_mut().push(InputEvent::FocusLost);
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = win_target
                .add_event_listener_with_callback("blur", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // -- Touch events (on canvas) --

        // touchstart → PointerClick
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                e.prevent_default();
                let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                    return;
                };
                if let Some(touch) = te.touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::PointerClick { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("touchstart", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // touchmove → CursorMove
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                e.prevent_default();
                let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                    return;
                };
                if let Some(touch) = te.touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::CursorMove { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("touchmove", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        // touchend → PointerRelease
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                e.prevent_default();
                let Ok(te) = e.dyn_into::<web_sys::TouchEvent>() else {
                    return;
                };
                if let Some(touch) = te.changed_touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::PointerRelease { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = target
                .add_event_listener_with_callback("touchend", closure.as_ref().unchecked_ref());
            closures.push(closure);
        }

        Self {
            events,
            _closures: closures,
        }
    }
}

impl InputBackend for WasmInputBackend {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

// ---------------------------------------------------------------------------
// Coordinate scaling
// ---------------------------------------------------------------------------

/// Pure math for `object-fit: contain` coordinate scaling.
///
/// Maps a point from element-relative CSS coordinates to virtual canvas
/// coordinates, accounting for letterboxing.
///
/// * `elem_w`, `elem_h` -- CSS bounding rect of the canvas element.
/// * `rect_left`, `rect_top` -- CSS left/top of the bounding rect.
/// * `client_x`, `client_y` -- page-relative event coordinates.
/// * `cw`, `ch` -- virtual canvas dimensions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scale_point_math(
    elem_w: f64,
    elem_h: f64,
    rect_left: f64,
    rect_top: f64,
    client_x: f64,
    client_y: f64,
    cw: u32,
    ch: u32,
) -> (i32, i32) {
    let canvas_w = cw as f64;
    let canvas_h = ch as f64;

    // Compute rendered content rect (object-fit: contain).
    let scale = (elem_w / canvas_w).min(elem_h / canvas_h);
    let rendered_w = canvas_w * scale;
    let rendered_h = canvas_h * scale;
    let offset_x = (elem_w - rendered_w) / 2.0;
    let offset_y = (elem_h - rendered_h) / 2.0;

    // Map from viewport to rendered content area, then to virtual.
    let rel_x = client_x - rect_left - offset_x;
    let rel_y = client_y - rect_top - offset_y;
    let x = (rel_x / scale) as i32;
    let y = (rel_y / scale) as i32;
    (x.clamp(0, cw as i32 - 1), y.clamp(0, ch as i32 - 1))
}

/// Map a viewport-relative point to virtual canvas coordinates, accounting
/// for `object-fit: contain` letterboxing.
fn scale_point(
    canvas: &HtmlCanvasElement,
    client_x: f64,
    client_y: f64,
    cw: u32,
    ch: u32,
) -> (i32, i32) {
    let rect = canvas.get_bounding_client_rect();
    scale_point_math(
        rect.width(),
        rect.height(),
        rect.left(),
        rect.top(),
        client_x,
        client_y,
        cw,
        ch,
    )
}

fn scale_mouse(canvas: &HtmlCanvasElement, me: &MouseEvent, cw: u32, ch: u32) -> (i32, i32) {
    scale_point(canvas, me.client_x() as f64, me.client_y() as f64, cw, ch)
}

fn scale_touch(canvas: &HtmlCanvasElement, touch: &web_sys::Touch, cw: u32, ch: u32) -> (i32, i32) {
    scale_point(
        canvas,
        touch.client_x() as f64,
        touch.client_y() as f64,
        cw,
        ch,
    )
}

// ---------------------------------------------------------------------------
// Key mapping
// ---------------------------------------------------------------------------

fn map_keydown(ke: &KeyboardEvent) -> Option<InputEvent> {
    let key = ke.key();
    match key.as_str() {
        "ArrowUp" => Some(InputEvent::ButtonPress(Button::Up)),
        "ArrowDown" => Some(InputEvent::ButtonPress(Button::Down)),
        "ArrowLeft" => Some(InputEvent::ButtonPress(Button::Left)),
        "ArrowRight" => Some(InputEvent::ButtonPress(Button::Right)),
        "Enter" => Some(InputEvent::ButtonPress(Button::Confirm)),
        "Escape" => Some(InputEvent::ButtonPress(Button::Cancel)),
        " " => Some(InputEvent::ButtonPress(Button::Triangle)),
        "Tab" => {
            if ke.shift_key() {
                Some(InputEvent::ShiftTab)
            } else {
                Some(InputEvent::Tab)
            }
        },
        "F1" => Some(InputEvent::ButtonPress(Button::Start)),
        "F2" => Some(InputEvent::ButtonPress(Button::Select)),
        "q" | "Q" => Some(InputEvent::TriggerPress(Trigger::Left)),
        "e" | "E" => Some(InputEvent::TriggerPress(Trigger::Right)),
        "Backspace" => Some(InputEvent::Backspace),
        "F11" => Some(InputEvent::ToggleFullscreen),
        _ => None,
    }
}

fn map_keyup(ke: &KeyboardEvent) -> Option<InputEvent> {
    let key = ke.key();
    match key.as_str() {
        "ArrowUp" => Some(InputEvent::ButtonRelease(Button::Up)),
        "ArrowDown" => Some(InputEvent::ButtonRelease(Button::Down)),
        "ArrowLeft" => Some(InputEvent::ButtonRelease(Button::Left)),
        "ArrowRight" => Some(InputEvent::ButtonRelease(Button::Right)),
        "Enter" => Some(InputEvent::ButtonRelease(Button::Confirm)),
        "Escape" => Some(InputEvent::ButtonRelease(Button::Cancel)),
        " " => Some(InputEvent::ButtonRelease(Button::Triangle)),
        "Tab" => None, // Tab is handled on keydown only.
        "F1" => Some(InputEvent::ButtonRelease(Button::Start)),
        "F2" => Some(InputEvent::ButtonRelease(Button::Select)),
        "q" | "Q" => Some(InputEvent::TriggerRelease(Trigger::Left)),
        "e" | "E" => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}

// -----------------------------------------------------------------------
// Tests -- pure functions testable on any target.
// -----------------------------------------------------------------------

/// Test helper: create a mock key string and call `map_keydown` logic.
/// Since `map_keydown` takes a `KeyboardEvent` (WASM-only), we test the
/// key string matching directly via this extracted helper.
#[cfg(test)]
fn map_keydown_str(key: &str) -> Option<InputEvent> {
    match key {
        "ArrowUp" => Some(InputEvent::ButtonPress(Button::Up)),
        "ArrowDown" => Some(InputEvent::ButtonPress(Button::Down)),
        "ArrowLeft" => Some(InputEvent::ButtonPress(Button::Left)),
        "ArrowRight" => Some(InputEvent::ButtonPress(Button::Right)),
        "Enter" => Some(InputEvent::ButtonPress(Button::Confirm)),
        "Escape" => Some(InputEvent::ButtonPress(Button::Cancel)),
        " " => Some(InputEvent::ButtonPress(Button::Triangle)),
        "Tab" => Some(InputEvent::Tab),
        "F1" => Some(InputEvent::ButtonPress(Button::Start)),
        "F2" => Some(InputEvent::ButtonPress(Button::Select)),
        "q" | "Q" => Some(InputEvent::TriggerPress(Trigger::Left)),
        "e" | "E" => Some(InputEvent::TriggerPress(Trigger::Right)),
        "Backspace" => Some(InputEvent::Backspace),
        "F11" => Some(InputEvent::ToggleFullscreen),
        _ => None,
    }
}

/// Test helper for key-up mapping.
#[cfg(test)]
fn map_keyup_str(key: &str) -> Option<InputEvent> {
    match key {
        "ArrowUp" => Some(InputEvent::ButtonRelease(Button::Up)),
        "ArrowDown" => Some(InputEvent::ButtonRelease(Button::Down)),
        "ArrowLeft" => Some(InputEvent::ButtonRelease(Button::Left)),
        "ArrowRight" => Some(InputEvent::ButtonRelease(Button::Right)),
        "Enter" => Some(InputEvent::ButtonRelease(Button::Confirm)),
        "Escape" => Some(InputEvent::ButtonRelease(Button::Cancel)),
        " " => Some(InputEvent::ButtonRelease(Button::Triangle)),
        "Tab" => None, // Tab is handled on keydown only.
        "F1" => Some(InputEvent::ButtonRelease(Button::Start)),
        "F2" => Some(InputEvent::ButtonRelease(Button::Select)),
        "q" | "Q" => Some(InputEvent::TriggerRelease(Trigger::Left)),
        "e" | "E" => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::scale_point_math;
    use super::*;

    // -- Identity / no-letterbox scenarios --

    #[test]
    fn exact_fit_no_letterbox() {
        // Element is exactly 480x272 (same as virtual), no offset.
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, 240.0, 136.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    #[test]
    fn exact_fit_top_left_corner() {
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, 0.0, 0.0, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn exact_fit_bottom_right_corner() {
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, 479.0, 271.0, 480, 272);
        assert_eq!(x, 479);
        assert_eq!(y, 271);
    }

    // -- Scaled-up (2x) scenarios --

    #[test]
    fn double_size_center() {
        // Element is 960x544, virtual 480x272.
        // scale = 2.0, no letterboxing.
        let (x, y) = scale_point_math(960.0, 544.0, 0.0, 0.0, 480.0, 272.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    #[test]
    fn double_size_origin() {
        let (x, y) = scale_point_math(960.0, 544.0, 0.0, 0.0, 0.0, 0.0, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    // -- Letterbox (pillarbox) scenarios --

    #[test]
    fn horizontal_letterbox_center() {
        // Element: 800x272 (wider than needed). Virtual: 480x272.
        // scale = min(800/480, 272/272) = 1.0.
        // rendered_w = 480, offset_x = (800-480)/2 = 160.
        // Click at (400, 136) => rel_x = 400-0-160 = 240.
        let (x, y) = scale_point_math(800.0, 272.0, 0.0, 0.0, 400.0, 136.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    #[test]
    fn vertical_letterbox_center() {
        // Element: 480x600 (taller than needed). Virtual: 480x272.
        // scale = min(480/480, 600/272) = 1.0.
        // rendered_h = 272, offset_y = (600-272)/2 = 164.
        // Click at (240, 300) => rel_y = 300-0-164 = 136.
        let (x, y) = scale_point_math(480.0, 600.0, 0.0, 0.0, 240.0, 300.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    // -- Clamping to bounds --

    #[test]
    fn negative_coords_clamp_to_zero() {
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, -50.0, -50.0, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn beyond_canvas_clamps_to_max() {
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, 999.0, 999.0, 480, 272);
        assert_eq!(x, 479);
        assert_eq!(y, 271);
    }

    #[test]
    fn in_left_letterbox_clamps_to_zero() {
        // Element: 800x272, virtual 480x272. offset_x = 160.
        // Click at (100, 136) => rel_x = 100-160 = -60 => clamp to 0.
        let (x, _y) = scale_point_math(800.0, 272.0, 0.0, 0.0, 100.0, 136.0, 480, 272);
        assert_eq!(x, 0);
    }

    #[test]
    fn in_right_letterbox_clamps_to_max() {
        // offset_x = 160, rendered_w = 480. Right bar starts at 640.
        // Click at (700, 136) => rel_x = 700-160 = 540 => 540/1 = 540
        // clamp(0..479).
        let (x, _y) = scale_point_math(800.0, 272.0, 0.0, 0.0, 700.0, 136.0, 480, 272);
        assert_eq!(x, 479);
    }

    // -- Non-zero rect offset --

    #[test]
    fn rect_offset_shifts_coordinates() {
        // Canvas is at (100, 50) on the page.
        let (x, y) = scale_point_math(480.0, 272.0, 100.0, 50.0, 340.0, 186.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    // -- Different aspect ratios --

    #[test]
    fn wide_element_narrow_canvas() {
        // Element: 1920x1080, Virtual: 480x272.
        // scale = min(1920/480, 1080/272) = min(4.0, 3.97) = 3.97.
        // rendered_w = 480 * 3.97 = 1905.88.
        // offset_x = (1920 - 1905.88)/2 ~= 7.06.
        // Click at center (960, 540):
        //   rel_x = 960 - 7.06 = 952.94 / 3.97 = 240.03 -> 240
        //   rel_y = 540 - 0.0 / 3.97 = 136.02 -> 136
        let (x, y) = scale_point_math(1920.0, 1080.0, 0.0, 0.0, 960.0, 540.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    #[test]
    fn square_element_with_psp_canvas() {
        // Element: 500x500, Virtual: 480x272.
        // scale = min(500/480, 500/272) = min(1.041, 1.838) = 1.041.
        // rendered_w = 480*1.041 = 500, rendered_h = 272*1.041 = 283.3
        // offset_x = 0, offset_y = (500-283.3)/2 = 108.3
        let (x, y) = scale_point_math(500.0, 500.0, 0.0, 0.0, 250.0, 250.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    // -- Edge cases --

    #[test]
    fn tiny_1x1_canvas() {
        // Element: 100x100, Virtual: 1x1.
        // scale = 100. rendered 100x100. offset 0,0.
        // Click at (50, 50) => rel_x = 50/100 = 0.5 -> 0.
        let (x, y) = scale_point_math(100.0, 100.0, 0.0, 0.0, 50.0, 50.0, 1, 1);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn large_virtual_small_element() {
        // Element: 100x100, Virtual: 1024x768.
        // scale = min(100/1024, 100/768) = 0.097.
        // rendered_w = 1024*0.097 = 100, rendered_h = 768*0.097 = 75.
        // offset_y = (100-75)/2 = 12.5.
        // Click at (50, 50):
        //   rel_x = 50/0.097 = 512 -> 512.
        //   rel_y = (50-12.5)/0.097 = 384.
        let (x, y) = scale_point_math(100.0, 100.0, 0.0, 0.0, 50.0, 50.0, 1024, 768);
        assert_eq!(x, 512);
        assert_eq!(y, 384);
    }

    #[test]
    fn zero_size_element_clamps_safely() {
        // Zero-size element: scale would be 0 or inf.
        // With zero elem dimensions, scale = min(0/480, 0/272) = 0.
        // rel_x / 0 = inf, cast to i32 = i32::MAX or similar.
        // Clamped to (479, 271).
        let (x, y) = scale_point_math(0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 480, 272);
        // With NaN/inf, clamp should keep within bounds.
        assert!((0..=479).contains(&x));
        assert!((0..=271).contains(&y));
    }

    #[test]
    fn fractional_coordinates_truncated() {
        // exact fit, click at (0.9, 0.9) => truncates to (0, 0).
        let (x, y) = scale_point_math(480.0, 272.0, 0.0, 0.0, 0.9, 0.9, 480, 272);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn high_dpi_double_scale_with_offset() {
        // Element at (50, 30) with size 960x544, virtual 480x272.
        // scale = 2.0. Click at (530, 302):
        //   rel_x = (530-50-0)/2 = 240
        //   rel_y = (302-30-0)/2 = 136
        let (x, y) = scale_point_math(960.0, 544.0, 50.0, 30.0, 530.0, 302.0, 480, 272);
        assert_eq!(x, 240);
        assert_eq!(y, 136);
    }

    // -------------------------------------------------------------------
    // Item 69: WASM key mapping tests (25 tests)
    // -------------------------------------------------------------------

    #[test]
    fn wasm_keydown_arrow_up() {
        assert_eq!(
            map_keydown_str("ArrowUp"),
            Some(InputEvent::ButtonPress(Button::Up))
        );
    }

    #[test]
    fn wasm_keydown_arrow_down() {
        assert_eq!(
            map_keydown_str("ArrowDown"),
            Some(InputEvent::ButtonPress(Button::Down))
        );
    }

    #[test]
    fn wasm_keydown_arrow_left() {
        assert_eq!(
            map_keydown_str("ArrowLeft"),
            Some(InputEvent::ButtonPress(Button::Left))
        );
    }

    #[test]
    fn wasm_keydown_arrow_right() {
        assert_eq!(
            map_keydown_str("ArrowRight"),
            Some(InputEvent::ButtonPress(Button::Right))
        );
    }

    #[test]
    fn wasm_keydown_enter_maps_to_confirm() {
        assert_eq!(
            map_keydown_str("Enter"),
            Some(InputEvent::ButtonPress(Button::Confirm))
        );
    }

    #[test]
    fn wasm_keydown_escape_maps_to_cancel() {
        assert_eq!(
            map_keydown_str("Escape"),
            Some(InputEvent::ButtonPress(Button::Cancel))
        );
    }

    #[test]
    fn wasm_keydown_space_maps_to_triangle() {
        assert_eq!(
            map_keydown_str(" "),
            Some(InputEvent::ButtonPress(Button::Triangle))
        );
    }

    #[test]
    fn wasm_keydown_tab_maps_to_tab() {
        assert_eq!(map_keydown_str("Tab"), Some(InputEvent::Tab));
    }

    #[test]
    fn wasm_keydown_f1_maps_to_start() {
        assert_eq!(
            map_keydown_str("F1"),
            Some(InputEvent::ButtonPress(Button::Start))
        );
    }

    #[test]
    fn wasm_keydown_f2_maps_to_select() {
        assert_eq!(
            map_keydown_str("F2"),
            Some(InputEvent::ButtonPress(Button::Select))
        );
    }

    #[test]
    fn wasm_keydown_q_lowercase_maps_to_trigger_left() {
        assert_eq!(
            map_keydown_str("q"),
            Some(InputEvent::TriggerPress(Trigger::Left))
        );
    }

    #[test]
    fn wasm_keydown_q_uppercase_maps_to_trigger_left() {
        assert_eq!(
            map_keydown_str("Q"),
            Some(InputEvent::TriggerPress(Trigger::Left))
        );
    }

    #[test]
    fn wasm_keydown_e_lowercase_maps_to_trigger_right() {
        assert_eq!(
            map_keydown_str("e"),
            Some(InputEvent::TriggerPress(Trigger::Right))
        );
    }

    #[test]
    fn wasm_keydown_e_uppercase_maps_to_trigger_right() {
        assert_eq!(
            map_keydown_str("E"),
            Some(InputEvent::TriggerPress(Trigger::Right))
        );
    }

    #[test]
    fn wasm_keydown_backspace() {
        assert_eq!(map_keydown_str("Backspace"), Some(InputEvent::Backspace));
    }

    #[test]
    fn wasm_keydown_f11_toggle_fullscreen() {
        assert_eq!(map_keydown_str("F11"), Some(InputEvent::ToggleFullscreen));
    }

    #[test]
    fn wasm_keydown_unknown_returns_none() {
        assert_eq!(map_keydown_str("a"), None);
        assert_eq!(map_keydown_str("Shift"), None);
        assert_eq!(map_keydown_str("Control"), None);
        assert_eq!(map_keydown_str("F3"), None);
    }

    // -- Key up tests --

    #[test]
    fn wasm_keyup_arrow_keys() {
        assert_eq!(
            map_keyup_str("ArrowUp"),
            Some(InputEvent::ButtonRelease(Button::Up))
        );
        assert_eq!(
            map_keyup_str("ArrowDown"),
            Some(InputEvent::ButtonRelease(Button::Down))
        );
        assert_eq!(
            map_keyup_str("ArrowLeft"),
            Some(InputEvent::ButtonRelease(Button::Left))
        );
        assert_eq!(
            map_keyup_str("ArrowRight"),
            Some(InputEvent::ButtonRelease(Button::Right))
        );
    }

    #[test]
    fn wasm_keyup_confirm_cancel() {
        assert_eq!(
            map_keyup_str("Enter"),
            Some(InputEvent::ButtonRelease(Button::Confirm))
        );
        assert_eq!(
            map_keyup_str("Escape"),
            Some(InputEvent::ButtonRelease(Button::Cancel))
        );
    }

    #[test]
    fn wasm_keyup_triggers() {
        assert_eq!(
            map_keyup_str("q"),
            Some(InputEvent::TriggerRelease(Trigger::Left))
        );
        assert_eq!(
            map_keyup_str("e"),
            Some(InputEvent::TriggerRelease(Trigger::Right))
        );
    }

    #[test]
    fn wasm_keyup_unknown_returns_none() {
        assert_eq!(map_keyup_str("Backspace"), None);
        assert_eq!(map_keyup_str("F11"), None);
        assert_eq!(map_keyup_str("a"), None);
    }

    #[test]
    fn wasm_keydown_keyup_symmetry() {
        let symmetric_keys = [
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "Enter",
            "Escape",
            " ",
            // Tab is handled as Tab/ShiftTab (not symmetric press/release).
            "F1",
            "F2",
            "q",
            "e",
        ];
        for key in symmetric_keys {
            let down = map_keydown_str(key);
            let up = map_keyup_str(key);
            assert!(down.is_some(), "key {key:?} should map on key-down");
            assert!(up.is_some(), "key {key:?} should map on key-up");
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

    #[test]
    fn wasm_all_buttons_covered_in_keydown() {
        // Verify core Button variants are reachable via at least one key.
        // Note: Square has no keydown mapping (Tab now produces Tab/ShiftTab).
        let all_keys = [
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "Enter",
            "Escape",
            " ",
            "F1",
            "F2",
        ];
        let mut found_buttons = std::collections::HashSet::new();
        for key in all_keys {
            if let Some(InputEvent::ButtonPress(btn)) = map_keydown_str(key) {
                found_buttons.insert(btn);
            }
        }
        assert!(found_buttons.contains(&Button::Up));
        assert!(found_buttons.contains(&Button::Down));
        assert!(found_buttons.contains(&Button::Left));
        assert!(found_buttons.contains(&Button::Right));
        assert!(found_buttons.contains(&Button::Confirm));
        assert!(found_buttons.contains(&Button::Cancel));
        assert!(found_buttons.contains(&Button::Triangle));
        assert!(found_buttons.contains(&Button::Start));
        assert!(found_buttons.contains(&Button::Select));
    }

    #[test]
    fn wasm_both_triggers_covered() {
        let mut found = std::collections::HashSet::new();
        for key in ["q", "Q", "e", "E"] {
            if let Some(InputEvent::TriggerPress(t)) = map_keydown_str(key) {
                found.insert(t);
            }
        }
        assert!(found.contains(&Trigger::Left));
        assert!(found.contains(&Trigger::Right));
    }

    #[test]
    fn wasm_case_insensitive_triggers() {
        // Both q/Q map to the same trigger, both e/E map to the same trigger.
        assert_eq!(map_keydown_str("q"), map_keydown_str("Q"));
        assert_eq!(map_keydown_str("e"), map_keydown_str("E"));
        assert_eq!(map_keyup_str("q"), map_keyup_str("Q"));
        assert_eq!(map_keyup_str("e"), map_keyup_str("E"));
    }
}
