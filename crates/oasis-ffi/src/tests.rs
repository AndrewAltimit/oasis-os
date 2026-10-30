//! Rust-side tests exercising the C-ABI exports directly.
//!
//! SAFETY (applies to every `unsafe` block in these tests): they call this
//! crate's own C-ABI functions or read their results. Handles come from
//! `create_instance()` (`oasis_create`) and are destroyed exactly once, at
//! the end of the test that made them (or are null where the null path is
//! under test). Pointer arguments point at live locals or `CString`s that
//! outlive the call. `CStr::from_ptr` only reads strings the exports
//! returned, after a non-null check and before `oasis_free_string`.

#![allow(clippy::undocumented_unsafe_blocks)]

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
    let cwd =
        unsafe { crate::handle::with_instance_ref(handle, String::new(), |inst| inst.cwd.clone()) };
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
    unsafe { oasis_register_callback(handle, OASIS_CB_COMMAND_EXEC, Some(test_cb)) };

    CALL_COUNT.store(0, Ordering::SeqCst);

    let cmd = CString::new("help").unwrap();
    let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
    unsafe { oasis_free_string(result) };

    assert!(CALL_COUNT.load(Ordering::SeqCst) > 0);
    unsafe { oasis_destroy(handle) };
}

#[test]
fn register_callback_ignores_unknown_codes() {
    extern "C" fn test_cb(_event: u32, _detail: *const c_char) {}

    let handle = create_instance();
    // 1/4/5/6 are the retired never-fired codes; 99 was never defined.
    for code in [0, 1, 4, 5, 6, 99] {
        unsafe { oasis_register_callback(handle, code, Some(test_cb)) };
    }
    unsafe { oasis_register_callback(handle, OASIS_CB_APP_LAUNCH, Some(test_cb)) };
    // SAFETY: `handle` is a live instance from `create_instance`.
    let codes: Vec<u32> = unsafe {
        crate::handle::with_instance_ref(handle, Vec::new(), |inst| {
            inst.callbacks.keys().copied().collect()
        })
    };
    assert_eq!(codes, vec![OASIS_CB_APP_LAUNCH]);
    unsafe { oasis_destroy(handle) };
}

#[test]
fn null_callback_unregisters() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALLS: AtomicU32 = AtomicU32::new(0);
    extern "C" fn cb(_event: u32, _detail: *const c_char) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }
    extern "C" fn audio_cb(_event: u32, _track: u64, _value: u32) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    let handle = create_instance();
    unsafe { oasis_register_callback(handle, OASIS_CB_COMMAND_EXEC, Some(cb)) };
    unsafe { oasis_set_audio_callback(handle, Some(audio_cb)) };
    // A C host passing NULL (= `None`) removes them instead of storing a
    // null function pointer.
    unsafe { oasis_register_callback(handle, OASIS_CB_COMMAND_EXEC, None) };
    unsafe { oasis_set_audio_callback(handle, None) };
    CALLS.store(0, Ordering::SeqCst);

    let cmd = CString::new("help").unwrap();
    let out = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
    unsafe { oasis_free_string(out) };
    assert!(unsafe { oasis_audio_set_volume(handle, 10) });
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    unsafe { oasis_destroy(handle) };
}

#[test]
fn add_vfs_file_null_data_with_zero_len_creates_empty_file() {
    let handle = create_instance();
    let path = CString::new("/tmp/zero.txt").unwrap();
    unsafe { oasis_add_vfs_file(handle, path.as_ptr(), std::ptr::null(), 0) };
    let exists = unsafe {
        crate::handle::with_instance_ref(handle, false, |inst| {
            oasis_core::vfs::Vfs::exists(&inst.vfs, "/tmp/zero.txt")
        })
    };
    assert!(exists);
    // Null data with a non-zero length is rejected.
    let bad = CString::new("/tmp/bad.txt").unwrap();
    unsafe { oasis_add_vfs_file(handle, bad.as_ptr(), std::ptr::null(), 8) };
    let exists = unsafe {
        crate::handle::with_instance_ref(handle, true, |inst| {
            oasis_core::vfs::Vfs::exists(&inst.vfs, "/tmp/bad.txt")
        })
    };
    assert!(!exists);
    unsafe { oasis_destroy(handle) };
}

#[test]
fn set_vfs_root_keeps_dashboard_apps_seeded() {
    let handle = create_instance();
    unsafe { oasis_set_vfs_root(handle, std::ptr::null()) };
    let apps = unsafe {
        crate::handle::with_instance_ref(handle, 0, |inst| {
            oasis_core::vfs::Vfs::readdir(&inst.vfs, "/apps").map_or(0, |e| e.len())
        })
    };
    assert!(apps > 0, "/apps must be re-seeded after a VFS reset");
    unsafe { oasis_destroy(handle) };
}

#[test]
fn multi_output_is_flattened_recursively() {
    use oasis_core::terminal::CommandOutput;
    let out = crate::commands::output_text_for_test(CommandOutput::Multi(vec![
        CommandOutput::Text("a".into()),
        CommandOutput::None,
        CommandOutput::Multi(vec![CommandOutput::Text("b".into())]),
        CommandOutput::Table {
            headers: vec!["h1".into(), "h2".into()],
            rows: vec![vec!["x".into(), "y".into()]],
        },
    ]));
    assert_eq!(out, "a\nb\nh1 | h2\nx | y");
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
            Some(dummy)
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
    assert!(!result.is_null());
    let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert!(
        output.contains("error"),
        "first.txt should be gone after reset"
    );
    unsafe { oasis_free_string(result) };

    // second.txt should exist.
    let cmd = CString::new("cat /home/second.txt").unwrap();
    let result = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
    assert!(!result.is_null());
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
    assert!(!result.is_null());
    let output = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert!(output.contains("/home"));
    unsafe { oasis_free_string(result) };

    unsafe { oasis_destroy(handle) };
}

#[test]
fn button_from_code_all_valid() {
    use crate::input::button_from_code;
    use oasis_core::input::Button;

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
    unsafe { oasis_set_audio_callback(handle, Some(audio_cb)) };

    let data = b"audio";
    let track_id = unsafe { oasis_audio_load(handle, data.as_ptr(), data.len() as u32) };
    unsafe { oasis_audio_play(handle, track_id) };
    unsafe { oasis_audio_stop(handle) };

    // At least: TrackLoaded + Play + Stop = 3 events.
    assert!(AUDIO_EVENTS.load(Ordering::SeqCst) >= 3);
    unsafe { oasis_destroy(handle) };
}

#[test]
fn panic_in_export_returns_default_and_instance_survives() {
    let handle = create_instance();

    // A panic inside oasis_tick is contained (returns `()` without unwinding
    // into the caller).
    crate::handle::arm_panic_injection();
    unsafe { oasis_tick(handle, 0.016) };

    // A panic inside oasis_send_command yields the documented default (null).
    crate::handle::arm_panic_injection();
    let cmd = CString::new("help").unwrap();
    let out = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
    assert!(out.is_null(), "panicking export must return its default");

    // The handle is not poisoned: subsequent calls work normally.
    let out = unsafe { oasis_send_command(handle, cmd.as_ptr()) };
    assert!(!out.is_null());
    let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    assert!(!text.is_empty());
    unsafe { oasis_free_string(out) };

    unsafe { oasis_tick(handle, 1.0) };
    assert!(unsafe { oasis_get_dirty(handle) });
    unsafe { oasis_destroy(handle) };
}

#[test]
fn ffi_guard_passes_through_value_without_panic() {
    assert_eq!(crate::handle::ffi_guard("test", 0u32, || 7), 7);
    assert_eq!(
        crate::handle::ffi_guard("test", 5u32, || panic!("boom {}", 1)),
        5
    );
}

/// A skin's static background layer is baked into one render target
/// on the first rendered frame and reused afterwards (the FFI used to
/// redraw every primitive each frame, and the UE5 backend could not
/// composite a baked layer at all).
#[test]
fn static_background_layers_are_baked_once() {
    let toml = |s: &str| CString::new(s).unwrap();
    let manifest = toml(include_str!("../../../skins/paper/skin.toml"));
    let layout = toml(include_str!("../../../skins/paper/layout.toml"));
    let features = toml(include_str!("../../../skins/paper/features.toml"));
    let theme = toml(include_str!("../../../skins/paper/theme.toml"));
    let strings = toml(include_str!("../../../skins/paper/strings.toml"));
    let handle = unsafe {
        oasis_create_full(
            160,
            90,
            manifest.as_ptr(),
            layout.as_ptr(),
            features.as_ptr(),
            theme.as_ptr(),
            strings.as_ptr(),
        )
    };
    assert!(!handle.is_null());
    assert!(
        !unsafe { &*handle }
            .active_theme
            .background_layers
            .is_empty(),
        "the paper skin has background layers"
    );
    unsafe { oasis_tick(handle, 1.0) };
    let baked = unsafe { &*handle }.backend.live_render_targets();
    assert!(baked >= 1, "static layer baked into a render target");
    for _ in 0..3 {
        unsafe { oasis_tick(handle, 1.0) };
    }
    assert_eq!(
        unsafe { &*handle }.backend.live_render_targets(),
        baked,
        "the bake is reused, not redone per frame"
    );
    unsafe { oasis_destroy(handle) };
}
