//! Input dispatch for the window manager.

use oasis_sdi::SdiRegistry;
use oasis_types::input::InputEvent;

use super::{WindowManager, WmEvent};

impl WindowManager {
    /// Process an input event through the WM. Returns what happened.
    pub fn handle_input(&mut self, event: &InputEvent, sdi: &mut SdiRegistry) -> WmEvent {
        match event {
            InputEvent::PointerClick { x, y } => self.handle_click(*x, *y, sdi),
            InputEvent::CursorMove { x, y } => self.handle_cursor_move(*x, *y, sdi),
            InputEvent::PointerRelease { .. } => self.handle_release(),
            _ => WmEvent::None,
        }
    }
}
