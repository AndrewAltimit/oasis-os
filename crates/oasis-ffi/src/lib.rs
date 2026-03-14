//! OASIS_OS C-ABI FFI boundary.
//!
//! Exports an opaque handle API for UE5 (or any C/C++ host) to create,
//! drive, and render OASIS_OS instances. All internal Rust state is behind
//! an opaque `OasisInstance` pointer. UE5 never sees Rust types.
//!
//! # Safety
//!
//! All `extern "C"` functions that take an `*mut OasisInstance` require a
//! valid, non-null handle previously returned by `oasis_create`. Passing
//! null or a freed handle is undefined behavior.
//!
//! # Thread Safety
//!
//! `OasisInstance` is **not** thread-safe. All `extern "C"` functions must be
//! called from the same thread (typically the UE5 game thread). Calling any
//! `oasis_*` function from multiple threads concurrently is undefined behavior.
//! If you need to interact with OASIS_OS from a worker thread, synchronize
//! access externally (e.g. via a mutex in your C++ code).
//!
//! Recommended UE5 integration pattern:
//! ```cpp
//! // Create on game thread, store in a TSharedPtr
//! FCriticalSection OasisMutex;
//! void* OasisHandle = oasis_create(...);
//!
//! // Tick on game thread only:
//! FScopeLock Lock(&OasisMutex);
//! oasis_tick(OasisHandle);
//! ```

// Modules organized by functionality.
mod audio;
mod callbacks;
mod commands;
mod handle;
mod input;
mod lifecycle;
mod render;
mod types;
mod vfs;
mod video;

// Re-export all public C-ABI symbols and types so the crate root exports them.
pub use audio::*;
pub use callbacks::*;
pub use commands::*;
pub use handle::OasisInstance;
pub use input::*;
pub use lifecycle::*;
pub use render::*;
pub use types::*;
pub use vfs::*;
#[cfg(feature = "_video")]
pub use video::*;

// ---------------------------------------------------------------------------
// Tests (Rust-side, exercising the FFI functions directly)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    use super::*;

    fn create_instance() -> *mut OasisInstance {
        unsafe {
            oasis_create(
                480,
                272,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        }
    }

    #[test]
    fn create_and_destroy() {
        let handle = create_instance();
        assert!(!handle.is_null());
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn destroy_null_is_safe() {
        unsafe { oasis_destroy(std::ptr::null_mut()) };
    }

    #[test]
    fn tick_advances_state() {
        let handle = create_instance();
        unsafe { oasis_tick(handle, 0.016) };
        // Should be dirty after first tick.
        assert!(unsafe { oasis_get_dirty(handle) });
        // After reading dirty, should be clear.
        assert!(!unsafe { oasis_get_dirty(handle) });
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn get_buffer_returns_valid_pointer() {
        let handle = create_instance();
        let mut w: u32 = 0;
        let mut h: u32 = 0;
        let ptr = unsafe { oasis_get_buffer(handle, &mut w, &mut h) };
        assert!(!ptr.is_null());
        assert_eq!(w, 480);
        assert_eq!(h, 272);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_command_help() {
        let handle = create_instance();
        let cmd = CString::new("help").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("help"));
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_command_status() {
        let handle = create_instance();
        let cmd = CString::new("status").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("OASIS"));
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_command_unknown() {
        let handle = create_instance();
        let cmd = CString::new("nonexistent_cmd").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("error"));
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_button() {
        let handle = create_instance();
        let evt = OasisInputEvent {
            event_type: OASIS_EVENT_BUTTON_PRESS,
            x: 0,
            y: 0,
            key: OASIS_BUTTON_DOWN,
            character: 0,
        };
        unsafe { oasis_send_input(handle, &evt) };
        // Tick to process the event.
        unsafe { oasis_tick(handle, 0.016) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_text() {
        let handle = create_instance();
        let evt = OasisInputEvent {
            event_type: OASIS_EVENT_TEXT_INPUT,
            x: 0,
            y: 0,
            key: 0,
            character: 'A' as u32,
        };
        unsafe { oasis_send_input(handle, &evt) };
        unsafe { oasis_tick(handle, 0.016) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn set_vfs_root_resets() {
        let handle = create_instance();
        // Write a file via command.
        let cmd = CString::new("mkdir /tmp").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        unsafe { oasis_free_string(result) };

        // Reset VFS.
        unsafe { oasis_set_vfs_root(handle, std::ptr::null()) };

        // CWD should be reset.
        // SAFETY: `handle` was returned by `oasis_create` and is valid.
        let cwd = unsafe {
            crate::handle::with_instance_ref(handle, String::new(), |inst| inst.cwd.clone())
        };
        assert_eq!(cwd, "/");

        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn add_vfs_file_and_read() {
        let handle = create_instance();
        let path = CString::new("/home/readme.txt").unwrap();
        let data = b"Welcome to the game!";
        unsafe { oasis_add_vfs_file(handle, path.as_ptr(), data.as_ptr(), data.len() as u32) };

        // Read the file via command.
        let cmd = CString::new("cat /home/readme.txt").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("Welcome to the game!"));
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn register_callback_fires() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

        extern "C" fn test_cb(_event: u32, _detail: *const c_char) {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let handle = create_instance();
        unsafe { oasis_register_callback(handle, OASIS_CB_COMMAND_EXEC, test_cb) };

        CALL_COUNT.store(0, Ordering::SeqCst);

        let cmd = CString::new("help").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        unsafe { oasis_free_string(result) };

        assert!(CALL_COUNT.load(Ordering::SeqCst) > 0);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn free_string_null_is_safe() {
        unsafe { oasis_free_string(std::ptr::null_mut()) };
    }

    #[test]
    fn null_handle_operations_are_safe() {
        let null = std::ptr::null_mut();
        unsafe {
            oasis_tick(null, 0.016);
            oasis_send_input(null, std::ptr::null());
            let _ = oasis_get_buffer(null, std::ptr::null_mut(), std::ptr::null_mut());
            let _ = oasis_get_dirty(null);
            let _ = oasis_send_command(null, std::ptr::null());
            oasis_set_vfs_root(null, std::ptr::null());
            oasis_register_callback(null, 0, {
                extern "C" fn dummy(_: u32, _: *const c_char) {}
                dummy
            });
            oasis_add_vfs_file(null, std::ptr::null(), std::ptr::null(), 0);
        }
    }

    // -- Edge case tests --

    #[test]
    fn send_command_empty_string() {
        let handle = create_instance();
        let cmd = CString::new("").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        // Empty command produces empty output (CommandOutput::None)
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_command_null_cmd() {
        let handle = create_instance();
        let result = unsafe { oasis_send_command(handle, std::ptr::null()) };
        assert!(result.is_null());
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn tick_large_delta() {
        let handle = create_instance();
        // Very large delta should not crash.
        unsafe { oasis_tick(handle, 1_000_000.0) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn tick_zero_delta() {
        let handle = create_instance();
        unsafe { oasis_tick(handle, 0.0) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn tick_negative_delta() {
        let handle = create_instance();
        unsafe { oasis_tick(handle, -1.0) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn multiple_ticks() {
        let handle = create_instance();
        for _ in 0..100 {
            unsafe { oasis_tick(handle, 0.016) };
        }
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn add_vfs_file_empty_data() {
        let handle = create_instance();
        let path = CString::new("/tmp/empty.txt").unwrap();
        let data: &[u8] = &[];
        unsafe { oasis_add_vfs_file(handle, path.as_ptr(), data.as_ptr(), 0) };

        let cmd = CString::new("cat /tmp/empty.txt").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        assert!(!result.is_null());
        unsafe { oasis_free_string(result) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn add_vfs_file_null_data() {
        let handle = create_instance();
        let path = CString::new("/tmp/null.txt").unwrap();
        // Null data pointer should be handled safely.
        unsafe { oasis_add_vfs_file(handle, path.as_ptr(), std::ptr::null(), 0) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn add_vfs_file_null_path() {
        let handle = create_instance();
        let data = b"hello";
        // Null path should be handled safely.
        unsafe { oasis_add_vfs_file(handle, std::ptr::null(), data.as_ptr(), data.len() as u32) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn get_buffer_null_out_params() {
        let handle = create_instance();
        // Null out_width and out_height should be handled.
        let ptr = unsafe { oasis_get_buffer(handle, std::ptr::null_mut(), std::ptr::null_mut()) };
        assert!(!ptr.is_null());
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn get_buffer_stable_across_ticks() {
        let handle = create_instance();
        let mut w1: u32 = 0;
        let mut h1: u32 = 0;
        let ptr1 = unsafe { oasis_get_buffer(handle, &mut w1, &mut h1) };

        unsafe { oasis_tick(handle, 0.016) };

        let mut w2: u32 = 0;
        let mut h2: u32 = 0;
        let ptr2 = unsafe { oasis_get_buffer(handle, &mut w2, &mut h2) };

        // Dimensions should remain stable.
        assert_eq!(w1, w2);
        assert_eq!(h1, h2);
        // Buffer pointer should be stable (same backing allocation).
        assert_eq!(ptr1, ptr2);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn set_vfs_root_then_add_file() {
        let handle = create_instance();
        // Add a file, reset VFS, add another file.
        let path1 = CString::new("/home/first.txt").unwrap();
        let data1 = b"first";
        unsafe { oasis_add_vfs_file(handle, path1.as_ptr(), data1.as_ptr(), data1.len() as u32) };

        unsafe { oasis_set_vfs_root(handle, std::ptr::null()) };

        let path2 = CString::new("/home/second.txt").unwrap();
        let data2 = b"second";
        unsafe { oasis_add_vfs_file(handle, path2.as_ptr(), data2.as_ptr(), data2.len() as u32) };

        // first.txt should be gone after reset.
        let cmd = CString::new("cat /home/first.txt").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(
            output.contains("error"),
            "first.txt should be gone after reset"
        );
        unsafe { oasis_free_string(result) };

        // second.txt should exist.
        let cmd = CString::new("cat /home/second.txt").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("second"));
        unsafe { oasis_free_string(result) };

        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_all_event_types() {
        let handle = create_instance();
        let events = [
            OasisInputEvent {
                event_type: OASIS_EVENT_CURSOR_MOVE,
                x: 100,
                y: 100,
                key: 0,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_BUTTON_PRESS,
                x: 0,
                y: 0,
                key: OASIS_BUTTON_UP,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_BUTTON_RELEASE,
                x: 0,
                y: 0,
                key: OASIS_BUTTON_UP,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_TRIGGER_PRESS,
                x: 0,
                y: 0,
                key: OASIS_TRIGGER_LEFT,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_TRIGGER_RELEASE,
                x: 0,
                y: 0,
                key: OASIS_TRIGGER_RIGHT,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_TEXT_INPUT,
                x: 0,
                y: 0,
                key: 0,
                character: 'Z' as u32,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_POINTER_CLICK,
                x: 50,
                y: 50,
                key: 0,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_POINTER_RELEASE,
                x: 50,
                y: 50,
                key: 0,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_FOCUS_GAINED,
                x: 0,
                y: 0,
                key: 0,
                character: 0,
            },
            OasisInputEvent {
                event_type: OASIS_EVENT_FOCUS_LOST,
                x: 0,
                y: 0,
                key: 0,
                character: 0,
            },
        ];
        for evt in &events {
            unsafe { oasis_send_input(handle, evt) };
        }
        unsafe { oasis_tick(handle, 0.016) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_invalid_button_code() {
        let handle = create_instance();
        let evt = OasisInputEvent {
            event_type: OASIS_EVENT_BUTTON_PRESS,
            x: 0,
            y: 0,
            key: 999, // Invalid button code
            character: 0,
        };
        // Should not crash -- invalid code is silently ignored.
        unsafe { oasis_send_input(handle, &evt) };
        unsafe { oasis_tick(handle, 0.016) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_invalid_event_type() {
        let handle = create_instance();
        let evt = OasisInputEvent {
            event_type: 999,
            x: 0,
            y: 0,
            key: 0,
            character: 0,
        };
        unsafe { oasis_send_input(handle, &evt) };
        unsafe { oasis_tick(handle, 0.016) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn send_input_null_event() {
        let handle = create_instance();
        unsafe { oasis_send_input(handle, std::ptr::null()) };
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn cwd_persists_across_commands() {
        let handle = create_instance();

        let cmd = CString::new("cd /home").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        unsafe { oasis_free_string(result) };

        let cmd = CString::new("pwd").unwrap();
        let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
        let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert!(output.contains("/home"));
        unsafe { oasis_free_string(result) };

        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn button_from_code_all_valid() {
        use crate::input::{button_from_code, trigger_from_code};
        use oasis_core::input::{Button, Trigger};

        assert_eq!(button_from_code(OASIS_BUTTON_UP), Some(Button::Up));
        assert_eq!(button_from_code(OASIS_BUTTON_DOWN), Some(Button::Down));
        assert_eq!(button_from_code(OASIS_BUTTON_LEFT), Some(Button::Left));
        assert_eq!(button_from_code(OASIS_BUTTON_RIGHT), Some(Button::Right));
        assert_eq!(
            button_from_code(OASIS_BUTTON_CONFIRM),
            Some(Button::Confirm)
        );
        assert_eq!(button_from_code(OASIS_BUTTON_CANCEL), Some(Button::Cancel));
        assert_eq!(
            button_from_code(OASIS_BUTTON_TRIANGLE),
            Some(Button::Triangle)
        );
        assert_eq!(button_from_code(OASIS_BUTTON_SQUARE), Some(Button::Square));
        assert_eq!(button_from_code(OASIS_BUTTON_START), Some(Button::Start));
        assert_eq!(button_from_code(OASIS_BUTTON_SELECT), Some(Button::Select));
    }

    #[test]
    fn button_from_code_invalid() {
        use crate::input::button_from_code;

        assert_eq!(button_from_code(10), None);
        assert_eq!(button_from_code(u32::MAX), None);
    }

    #[test]
    fn trigger_from_code_all_valid() {
        use crate::input::trigger_from_code;
        use oasis_core::input::Trigger;

        assert_eq!(trigger_from_code(OASIS_TRIGGER_LEFT), Some(Trigger::Left));
        assert_eq!(trigger_from_code(OASIS_TRIGGER_RIGHT), Some(Trigger::Right));
    }

    #[test]
    fn trigger_from_code_invalid() {
        use crate::input::trigger_from_code;

        assert_eq!(trigger_from_code(2), None);
        assert_eq!(trigger_from_code(u32::MAX), None);
    }

    // -- Audio FFI tests --

    #[test]
    fn audio_load_and_play() {
        let handle = create_instance();
        let data = b"fake audio data";
        let track_id = unsafe { oasis_audio_load(handle, data.as_ptr(), data.len() as u32) };
        assert_ne!(track_id, u64::MAX);
        assert!(unsafe { oasis_audio_play(handle, track_id) });
        assert!(unsafe { oasis_audio_is_playing(handle) });
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_pause_resume_stop() {
        let handle = create_instance();
        let data = b"data";
        let track_id = unsafe { oasis_audio_load(handle, data.as_ptr(), data.len() as u32) };
        unsafe { oasis_audio_play(handle, track_id) };

        assert!(unsafe { oasis_audio_pause(handle) });
        assert!(!unsafe { oasis_audio_is_playing(handle) });

        assert!(unsafe { oasis_audio_resume(handle) });
        assert!(unsafe { oasis_audio_is_playing(handle) });

        assert!(unsafe { oasis_audio_stop(handle) });
        assert!(!unsafe { oasis_audio_is_playing(handle) });

        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_volume() {
        let handle = create_instance();
        assert!(unsafe { oasis_audio_set_volume(handle, 42) });
        assert_eq!(unsafe { oasis_audio_get_volume(handle) }, 42);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_load_null_data() {
        let handle = create_instance();
        let id = unsafe { oasis_audio_load(handle, std::ptr::null(), 100) };
        assert_eq!(id, u64::MAX);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_load_zero_len() {
        let handle = create_instance();
        let data = b"x";
        let id = unsafe { oasis_audio_load(handle, data.as_ptr(), 0) };
        assert_eq!(id, u64::MAX);
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_play_invalid_track() {
        let handle = create_instance();
        assert!(!unsafe { oasis_audio_play(handle, 999) });
        unsafe { oasis_destroy(handle) };
    }

    #[test]
    fn audio_null_handle_is_safe() {
        let null = std::ptr::null_mut();
        unsafe {
            oasis_audio_load(null, std::ptr::null(), 0);
            oasis_audio_play(null, 0);
            oasis_audio_pause(null);
            oasis_audio_resume(null);
            oasis_audio_stop(null);
            oasis_audio_set_volume(null, 50);
            assert_eq!(oasis_audio_get_volume(null), 0);
            assert!(!oasis_audio_is_playing(null));
        }
    }

    #[test]
    fn audio_callback_fires() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static AUDIO_EVENTS: AtomicU32 = AtomicU32::new(0);

        extern "C" fn audio_cb(_event: u32, _track_id: u64, _value: u32) {
            AUDIO_EVENTS.fetch_add(1, Ordering::SeqCst);
        }

        let handle = create_instance();
        AUDIO_EVENTS.store(0, Ordering::SeqCst);
        unsafe { oasis_set_audio_callback(handle, audio_cb) };

        let data = b"audio";
        let track_id = unsafe { oasis_audio_load(handle, data.as_ptr(), data.len() as u32) };
        unsafe { oasis_audio_play(handle, track_id) };
        unsafe { oasis_audio_stop(handle) };

        // At least: TrackLoaded + Play + Stop = 3 events.
        assert!(AUDIO_EVENTS.load(Ordering::SeqCst) >= 3);
        unsafe { oasis_destroy(handle) };
    }
}
