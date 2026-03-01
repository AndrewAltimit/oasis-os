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
                let ke: KeyboardEvent = e.dyn_into().unwrap();
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
            win_target
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }

        // keyup
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let ke: KeyboardEvent = e.dyn_into().unwrap();
                if let Some(input) = map_keyup(&ke) {
                    ke.prevent_default();
                    ev.borrow_mut().push(input);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            win_target
                .add_event_listener_with_callback("keyup", closure.as_ref().unchecked_ref())
                .unwrap();
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
                let me: MouseEvent = e.dyn_into().unwrap();
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::CursorMove { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("mousemove", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }

        // mousedown
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let me: MouseEvent = e.dyn_into().unwrap();
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::PointerClick { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }

        // mouseup
        {
            let ev = Rc::clone(&events);
            let cw = width;
            let ch = height;
            let canvas_clone = canvas.clone();
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let me: MouseEvent = e.dyn_into().unwrap();
                let (x, y) = scale_mouse(&canvas_clone, &me, cw, ch);
                ev.borrow_mut().push(InputEvent::PointerRelease { x, y });
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }

        // wheel
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
                let we: WheelEvent = e.dyn_into().unwrap();
                we.prevent_default();
                let delta = if we.delta_y() > 0.0 { 1 } else { -1 };
                ev.borrow_mut().push(InputEvent::MouseWheel { delta });
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("wheel", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }

        // focus / blur (on window)
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |_e: web_sys::Event| {
                ev.borrow_mut().push(InputEvent::FocusGained);
            }) as Box<dyn FnMut(web_sys::Event)>);
            win_target
                .add_event_listener_with_callback("focus", closure.as_ref().unchecked_ref())
                .unwrap();
            closures.push(closure);
        }
        {
            let ev = Rc::clone(&events);
            let closure = Closure::wrap(Box::new(move |_e: web_sys::Event| {
                ev.borrow_mut().push(InputEvent::FocusLost);
            }) as Box<dyn FnMut(web_sys::Event)>);
            win_target
                .add_event_listener_with_callback("blur", closure.as_ref().unchecked_ref())
                .unwrap();
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
                let te: web_sys::TouchEvent = e.dyn_into().unwrap();
                if let Some(touch) = te.touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::PointerClick { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("touchstart", closure.as_ref().unchecked_ref())
                .unwrap();
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
                let te: web_sys::TouchEvent = e.dyn_into().unwrap();
                if let Some(touch) = te.touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::CursorMove { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("touchmove", closure.as_ref().unchecked_ref())
                .unwrap();
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
                let te: web_sys::TouchEvent = e.dyn_into().unwrap();
                if let Some(touch) = te.changed_touches().get(0) {
                    let (x, y) = scale_touch(&canvas_clone, &touch, cw, ch);
                    ev.borrow_mut().push(InputEvent::PointerRelease { x, y });
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            target
                .add_event_listener_with_callback("touchend", closure.as_ref().unchecked_ref())
                .unwrap();
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
    let elem_w = rect.width();
    let elem_h = rect.height();
    let canvas_w = cw as f64;
    let canvas_h = ch as f64;

    // Compute rendered content rect inside the element (object-fit: contain).
    let scale = (elem_w / canvas_w).min(elem_h / canvas_h);
    let rendered_w = canvas_w * scale;
    let rendered_h = canvas_h * scale;
    let offset_x = (elem_w - rendered_w) / 2.0;
    let offset_y = (elem_h - rendered_h) / 2.0;

    // Map from viewport to the rendered content area, then to virtual coords.
    let rel_x = client_x - rect.left() - offset_x;
    let rel_y = client_y - rect.top() - offset_y;
    let x = (rel_x / scale) as i32;
    let y = (rel_y / scale) as i32;
    (x.clamp(0, cw as i32 - 1), y.clamp(0, ch as i32 - 1))
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
        "Tab" => Some(InputEvent::ButtonPress(Button::Square)),
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
        "Tab" => Some(InputEvent::ButtonRelease(Button::Square)),
        "F1" => Some(InputEvent::ButtonRelease(Button::Start)),
        "F2" => Some(InputEvent::ButtonRelease(Button::Select)),
        "q" | "Q" => Some(InputEvent::TriggerRelease(Trigger::Left)),
        "e" | "E" => Some(InputEvent::TriggerRelease(Trigger::Right)),
        _ => None,
    }
}
