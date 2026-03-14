//! C-compatible types and constants for the OASIS_OS FFI boundary.

use std::os::raw::c_char;

/// Input event passed from C to Rust.
#[repr(C)]
pub struct OasisInputEvent {
    /// Event type (one of the `OASIS_EVENT_*` constants).
    pub event_type: u32,
    /// X coordinate (for cursor/pointer events).
    pub x: i32,
    /// Y coordinate (for cursor/pointer events).
    pub y: i32,
    /// Button/trigger code (for button/trigger events).
    pub key: u32,
    /// Unicode codepoint (for text input events).
    pub character: u32,
}

// Event types.
pub const OASIS_EVENT_CURSOR_MOVE: u32 = 1;
pub const OASIS_EVENT_BUTTON_PRESS: u32 = 2;
pub const OASIS_EVENT_BUTTON_RELEASE: u32 = 3;
pub const OASIS_EVENT_TRIGGER_PRESS: u32 = 4;
pub const OASIS_EVENT_TRIGGER_RELEASE: u32 = 5;
pub const OASIS_EVENT_TEXT_INPUT: u32 = 6;
pub const OASIS_EVENT_POINTER_CLICK: u32 = 7;
pub const OASIS_EVENT_POINTER_RELEASE: u32 = 8;
pub const OASIS_EVENT_FOCUS_GAINED: u32 = 9;
pub const OASIS_EVENT_FOCUS_LOST: u32 = 10;
pub const OASIS_EVENT_QUIT: u32 = 11;

// Button codes (match the `Button` enum order).
pub const OASIS_BUTTON_UP: u32 = 0;
pub const OASIS_BUTTON_DOWN: u32 = 1;
pub const OASIS_BUTTON_LEFT: u32 = 2;
pub const OASIS_BUTTON_RIGHT: u32 = 3;
pub const OASIS_BUTTON_CONFIRM: u32 = 4;
pub const OASIS_BUTTON_CANCEL: u32 = 5;
pub const OASIS_BUTTON_TRIANGLE: u32 = 6;
pub const OASIS_BUTTON_SQUARE: u32 = 7;
pub const OASIS_BUTTON_START: u32 = 8;
pub const OASIS_BUTTON_SELECT: u32 = 9;

// Trigger codes.
pub const OASIS_TRIGGER_LEFT: u32 = 0;
pub const OASIS_TRIGGER_RIGHT: u32 = 1;

// Callback event types.
pub const OASIS_CB_FILE_ACCESS: u32 = 1;
pub const OASIS_CB_COMMAND_EXEC: u32 = 2;
pub const OASIS_CB_APP_LAUNCH: u32 = 3;
pub const OASIS_CB_LOGIN: u32 = 4;
pub const OASIS_CB_NETWORK_SEND: u32 = 5;
pub const OASIS_CB_PLUGIN_LOAD: u32 = 6;

/// Callback function type: receives an event type and a null-terminated detail string.
pub type OasisCallback = extern "C" fn(event: u32, detail: *const c_char);

/// Audio event callback type.
///
/// Parameters: event type (AudioEvent), track ID (0 if N/A), extra value.
pub type OasisAudioCallback = extern "C" fn(event: u32, track_id: u64, value: u32);
