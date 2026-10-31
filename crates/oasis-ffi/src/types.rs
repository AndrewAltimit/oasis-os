//! C-compatible types and constants for the OASIS_OS FFI boundary.

use std::os::raw::c_char;

/// Input event passed from C to Rust.
///
/// Field usage per event type:
///
/// - cursor move / pointer click / pointer release: `x`, `y`
/// - button / trigger press + release: `key` = button or trigger code
/// - `OASIS_EVENT_TEXT_INPUT`: `character` = Unicode codepoint
/// - `OASIS_EVENT_MOUSE_WHEEL`: `y` = delta (positive = scroll down)
/// - `OASIS_EVENT_KEY`: `key` = `OASIS_KEY_*`, `x` = `OASIS_MOD_*` bits,
///   `character` = codepoint when `key == OASIS_KEY_CHAR`
/// - all other event types carry no payload
#[repr(C)]
pub struct OasisInputEvent {
    /// Event type (one of the `OASIS_EVENT_*` constants).
    pub event_type: u32,
    /// X coordinate (cursor/pointer events); modifier bitmask for
    /// `OASIS_EVENT_KEY`.
    pub x: i32,
    /// Y coordinate (cursor/pointer events); wheel delta for
    /// `OASIS_EVENT_MOUSE_WHEEL`.
    pub y: i32,
    /// Button/trigger code (button/trigger events) or `OASIS_KEY_*` code
    /// (`OASIS_EVENT_KEY`).
    pub key: u32,
    /// Unicode codepoint (text input events and `OASIS_KEY_CHAR`).
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
pub const OASIS_EVENT_BACKSPACE: u32 = 12;
/// Mouse wheel; delta in `y` (positive = scroll down).
pub const OASIS_EVENT_MOUSE_WHEEL: u32 = 13;
pub const OASIS_EVENT_TOGGLE_FULLSCREEN: u32 = 14;
pub const OASIS_EVENT_TAB: u32 = 15;
pub const OASIS_EVENT_SHIFT_TAB: u32 = 16;
/// Raw keyboard key press: `key` = `OASIS_KEY_*`, `x` = `OASIS_MOD_*`
/// bitmask, `character` = codepoint when `key == OASIS_KEY_CHAR`.
///
/// Hosts that also send the gamepad-style twin (e.g. `BUTTON_PRESS` for an
/// arrow key) should send the `KEY` event first.
pub const OASIS_EVENT_KEY: u32 = 17;

// Key codes for `OASIS_EVENT_KEY` (match the `Key` enum).
/// Printable key; the lowercase character is in `character`.
pub const OASIS_KEY_CHAR: u32 = 0;
pub const OASIS_KEY_SPACE: u32 = 1;
pub const OASIS_KEY_ENTER: u32 = 2;
pub const OASIS_KEY_ESCAPE: u32 = 3;
pub const OASIS_KEY_TAB: u32 = 4;
pub const OASIS_KEY_BACKSPACE: u32 = 5;
pub const OASIS_KEY_DELETE: u32 = 6;
pub const OASIS_KEY_INSERT: u32 = 7;
pub const OASIS_KEY_HOME: u32 = 8;
pub const OASIS_KEY_END: u32 = 9;
pub const OASIS_KEY_PAGE_UP: u32 = 10;
pub const OASIS_KEY_PAGE_DOWN: u32 = 11;
pub const OASIS_KEY_UP: u32 = 12;
pub const OASIS_KEY_DOWN: u32 = 13;
pub const OASIS_KEY_LEFT: u32 = 14;
pub const OASIS_KEY_RIGHT: u32 = 15;
/// `F1`; `F2`..`F12` are `OASIS_KEY_F1 + 1` .. `OASIS_KEY_F1 + 11`.
pub const OASIS_KEY_F1: u32 = 101;
pub const OASIS_KEY_F12: u32 = 112;

// Modifier bits for `OASIS_EVENT_KEY` (match `Modifiers` bits).
pub const OASIS_MOD_SHIFT: u32 = 1;
pub const OASIS_MOD_CTRL: u32 = 2;
pub const OASIS_MOD_ALT: u32 = 4;
pub const OASIS_MOD_SUPER: u32 = 8;

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

// Callback event types. Codes 1, 4, 5 and 6 (`FILE_ACCESS`, `LOGIN`,
// `NETWORK_SEND`, `PLUGIN_LOAD`) were reserved placeholders that never fired;
// they are retired and must not be reused for a different meaning.
/// Fired by `oasis_send_command`; detail = the command line.
pub const OASIS_CB_COMMAND_EXEC: u32 = 2;
/// Fired when a dashboard app is launched; detail = the app title.
pub const OASIS_CB_APP_LAUNCH: u32 = 3;
/// Fired by `oasis_tick` with the output of background shell work: a
/// finished `cmd &` job (its output plus the `[N]+ Done` notice) or the
/// `cron` jobs that ran (lines prefixed `[cron <name>]`).
pub const OASIS_CB_SHELL_OUTPUT: u32 = 7;

/// Whether `event` is a callback code the library fires.
pub(crate) fn is_callback_event(event: u32) -> bool {
    matches!(
        event,
        OASIS_CB_COMMAND_EXEC | OASIS_CB_APP_LAUNCH | OASIS_CB_SHELL_OUTPUT
    )
}

/// Callback function type: receives an event type and a null-terminated detail string.
pub type OasisCallback = extern "C" fn(event: u32, detail: *const c_char);

/// Audio event callback type.
///
/// Parameters: event type (AudioEvent), track ID (0 if N/A), extra value.
pub type OasisAudioCallback = extern "C" fn(event: u32, track_id: u64, value: u32);

/// [`OasisCallback`] with a host context pointer: `user_data` is the
/// pointer given to `oasis_register_callback_ex`, passed back unchanged
/// (the library never dereferences it).
pub type OasisCallbackEx =
    extern "C" fn(event: u32, detail: *const c_char, user_data: *mut std::ffi::c_void);

/// [`OasisAudioCallback`] with the `user_data` pointer given to
/// `oasis_set_audio_callback_ex`.
pub type OasisAudioCallbackEx =
    extern "C" fn(event: u32, track_id: u64, value: u32, user_data: *mut std::ffi::c_void);

/// An OS-event callback registered with either registration export.
#[derive(Clone, Copy)]
pub(crate) enum RegisteredCallback {
    Plain(OasisCallback),
    /// The context pointer is kept as its exposed address (the instance
    /// stays `Send`); it is only handed back to the host.
    WithData(OasisCallbackEx, usize),
}

impl RegisteredCallback {
    pub(crate) fn call(self, event: u32, detail: *const c_char) {
        match self {
            Self::Plain(cb) => cb(event, detail),
            Self::WithData(cb, user_data) => cb(
                event,
                detail,
                std::ptr::with_exposed_provenance_mut(user_data),
            ),
        }
    }
}
