//! End-to-end tests of the C ABI, driven exactly like a C / UE5 host would:
//! only the exported `extern "C"` functions and `#[repr(C)]` types are used.
//!
//! Covers the full lifecycle (create -> tick -> buffer -> input -> command
//! -> VFS -> audio -> destroy), the dirty-flag contract, callbacks, several
//! independent instances and null-pointer handling.

#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use oasis_ffi::*;

const W: u32 = 480;
const H: u32 = 272;

const CLASSIC_MANIFEST: &str = include_str!("../../../skins/classic/skin.toml");
const CLASSIC_LAYOUT: &str = include_str!("../../../skins/classic/layout.toml");
const CLASSIC_FEATURES: &str = include_str!("../../../skins/classic/features.toml");
const CLASSIC_THEME: &str = include_str!("../../../skins/classic/theme.toml");
const CLASSIC_STRINGS: &str = include_str!("../../../skins/classic/strings.toml");

/// Owns a handle and destroys it on drop (so a failing assert doesn't leak).
struct Os(*mut OasisInstance);

impl Drop for Os {
    fn drop(&mut self) {
        // SAFETY: the handle came from oasis_create*, or is null.
        unsafe { oasis_destroy(self.0) };
    }
}

fn create_default() -> Os {
    // SAFETY: null TOML pointers are allowed.
    let h = unsafe { oasis_create(W, H, std::ptr::null(), std::ptr::null(), std::ptr::null()) };
    assert!(!h.is_null());
    Os(h)
}

fn create_classic(w: u32, h: u32) -> Os {
    let c = |s: &str| CString::new(s).unwrap();
    let (m, l, f, t, s) = (
        c(CLASSIC_MANIFEST),
        c(CLASSIC_LAYOUT),
        c(CLASSIC_FEATURES),
        c(CLASSIC_THEME),
        c(CLASSIC_STRINGS),
    );
    // SAFETY: all pointers are valid C strings for the duration of the call.
    let handle = unsafe {
        oasis_create_full(
            w,
            h,
            m.as_ptr(),
            l.as_ptr(),
            f.as_ptr(),
            t.as_ptr(),
            s.as_ptr(),
        )
    };
    // The strings are freed here; the doc promises they are parsed during
    // the call and may be dropped afterwards.
    drop((m, l, f, t, s));
    assert!(!handle.is_null());
    Os(handle)
}

fn tick(os: &Os, dt: f32) {
    // SAFETY: valid handle.
    unsafe { oasis_tick(os.0, dt) };
}

fn dirty(os: &Os) -> bool {
    // SAFETY: valid handle.
    unsafe { oasis_get_dirty(os.0) }
}

fn frame(os: &Os) -> (u32, u32, Vec<u8>) {
    let (mut w, mut h) = (0u32, 0u32);
    // SAFETY: valid handle and out-params.
    let ptr = unsafe { oasis_get_buffer(os.0, &mut w, &mut h) };
    assert!(!ptr.is_null());
    // SAFETY: the buffer is w*h*4 bytes and valid until the next tick.
    let px = unsafe { std::slice::from_raw_parts(ptr, (w * h * 4) as usize) }.to_vec();
    (w, h, px)
}

fn distinct_colors(px: &[u8]) -> usize {
    let mut set = std::collections::HashSet::new();
    for p in px.as_chunks::<4>().0 {
        set.insert(*p);
        if set.len() > 64 {
            break;
        }
    }
    set.len()
}

fn send(os: &Os, event_type: u32, x: i32, y: i32, key: u32, character: u32) {
    let ev = OasisInputEvent {
        event_type,
        x,
        y,
        key,
        character,
    };
    // SAFETY: valid handle and event.
    unsafe { oasis_send_input(os.0, &ev) };
}

fn button(os: &Os, code: u32) {
    send(os, OASIS_EVENT_BUTTON_PRESS, 0, 0, code, 0);
    send(os, OASIS_EVENT_BUTTON_RELEASE, 0, 0, code, 0);
}

fn command(os: &Os, cmd: &str) -> String {
    let c = CString::new(cmd).unwrap();
    // SAFETY: valid handle and C string.
    let out = unsafe { oasis_send_command(os.0, c.as_ptr()) };
    assert!(!out.is_null(), "command {cmd:?} returned NULL");
    // SAFETY: returned by oasis_send_command, NUL-terminated.
    let text = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: freeing exactly once.
    unsafe { oasis_free_string(out) };
    text
}

fn add_file(os: &Os, path: &str, data: &[u8]) {
    let p = CString::new(path).unwrap();
    // SAFETY: valid handle, path and data slice.
    unsafe { oasis_add_vfs_file(os.0, p.as_ptr(), data.as_ptr(), data.len() as u32) };
}

// ── Callbacks (fired synchronously on the calling thread) ──────────────

thread_local! {
    static OS_EVENTS: RefCell<Vec<(u32, String)>> = const { RefCell::new(Vec::new()) };
    static AUDIO_EVENTS: RefCell<Vec<(u32, u64, u32)>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn on_os_event(event: u32, detail: *const c_char) {
    // SAFETY: the library passes a valid NUL-terminated string.
    let s = unsafe { CStr::from_ptr(detail) }
        .to_string_lossy()
        .into_owned();
    OS_EVENTS.with(|e| e.borrow_mut().push((event, s)));
}

extern "C" fn on_audio_event(event: u32, track: u64, value: u32) {
    AUDIO_EVENTS.with(|e| e.borrow_mut().push((event, track, value)));
}

fn take_os_events() -> Vec<(u32, String)> {
    OS_EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

fn take_audio_events() -> Vec<(u32, u64, u32)> {
    AUDIO_EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

// ── Tests ──────────────────────────────────────────────────────────────

#[test]
fn full_lifecycle_like_a_host() {
    let os = create_classic(W, H);
    // SAFETY: valid handle; callback functions live for the whole program.
    unsafe {
        oasis_register_callback(os.0, OASIS_CB_COMMAND_EXEC, on_os_event);
        oasis_register_callback(os.0, OASIS_CB_APP_LAUNCH, on_os_event);
        oasis_set_audio_callback(os.0, on_audio_event);
    }
    take_os_events();
    take_audio_events();

    // First frame renders the skinned dashboard.
    tick(&os, 1.0 / 60.0);
    assert!(dirty(&os), "first tick must produce a frame");
    let (w, h, first) = frame(&os);
    assert_eq!((w, h), (W, H));
    assert_eq!(first.len(), (W * H * 4) as usize);
    assert!(
        distinct_colors(&first) > 8,
        "skinned frame looks blank ({} colors)",
        distinct_colors(&first)
    );
    assert!(
        first.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
        "framebuffer must be opaque RGBA"
    );

    // Navigating the dashboard with the d-pad changes the picture.
    button(&os, OASIS_BUTTON_RIGHT);
    tick(&os, 1.0 / 60.0);
    assert!(dirty(&os), "input must force a redraw");
    let (_, _, moved) = frame(&os);
    assert_ne!(
        first, moved,
        "moving the selection did not change the frame"
    );

    // Confirm launches the selected app -> APP_LAUNCH callback with its name.
    button(&os, OASIS_BUTTON_CONFIRM);
    tick(&os, 1.0 / 60.0);
    let launches: Vec<_> = take_os_events()
        .into_iter()
        .filter(|(e, _)| *e == OASIS_CB_APP_LAUNCH)
        .collect();
    assert_eq!(launches.len(), 1, "{launches:?}");
    assert!(!launches[0].1.is_empty());

    // Terminal command round trip + COMMAND_EXEC callback.
    let out = command(&os, "echo hello ffi");
    assert!(out.contains("hello ffi"), "{out:?}");
    assert_eq!(
        take_os_events(),
        vec![(OASIS_CB_COMMAND_EXEC, "echo hello ffi".to_string())]
    );

    // Host-provided file shows up in the shell.
    add_file(&os, "/home/readme.txt", "héllo from UE5\n".as_bytes());
    assert!(command(&os, "ls /home").contains("readme.txt"));
    assert!(command(&os, "cat /home/readme.txt").contains("héllo from UE5"));
    // cd persists across calls.
    command(&os, "cd /home");
    assert!(command(&os, "pwd").contains("/home"));
    assert!(command(&os, "cat readme.txt").contains("UE5"));
    // Writes from the terminal go to the overlay and read back.
    command(&os, "touch /tmp/new.txt");
    assert!(command(&os, "ls /tmp").contains("new.txt"));

    // Audio state machine + callbacks the host mirrors.
    // SAFETY: valid handle and data.
    unsafe {
        let data = b"ID3fake-mp3";
        let track = oasis_audio_load(os.0, data.as_ptr(), data.len() as u32);
        assert_ne!(track, u64::MAX);
        assert!(oasis_audio_play(os.0, track));
        assert!(oasis_audio_is_playing(os.0));
        assert!(oasis_audio_pause(os.0));
        assert!(!oasis_audio_is_playing(os.0));
        assert!(oasis_audio_resume(os.0));
        assert!(oasis_audio_is_playing(os.0));
        assert!(oasis_audio_set_volume(os.0, 30));
        assert_eq!(oasis_audio_get_volume(os.0), 30);
        assert!(oasis_audio_stop(os.0));
        assert!(!oasis_audio_is_playing(os.0));
        let events: Vec<u32> = take_audio_events().iter().map(|e| e.0).collect();
        // TrackLoaded, Play, Pause, Resume, VolumeChange, Stop.
        assert_eq!(events, vec![5, 0, 1, 2, 4, 3]);
    }

    // Destroy fires the audio Shutdown event.
    drop(os);
    assert_eq!(take_audio_events(), vec![(7, 0, 0)]);
}

#[test]
fn default_instance_renders_and_honours_dirty_contract() {
    let os = create_default();
    tick(&os, 0.016);
    assert!(dirty(&os));
    assert!(!dirty(&os), "reading the dirty flag must clear it");
    let (w, h, px) = frame(&os);
    assert_eq!((w, h), (W, H));
    assert!(px.iter().any(|&b| b != 0), "default frame is all zero");
    // Nothing changes on a quick idle tick: no redraw, not dirty.
    tick(&os, 0.001);
    assert!(!dirty(&os), "idle tick redrew without cause");
    // The periodic refresh (clock) eventually redraws.
    tick(&os, 1.0);
    assert!(dirty(&os));
    // Input always forces a redraw.
    button(&os, OASIS_BUTTON_UP);
    tick(&os, 0.0);
    assert!(dirty(&os));
}

#[test]
fn pointer_click_on_a_dashboard_icon_launches_it() {
    let os = create_classic(W, H);
    // SAFETY: valid handle.
    unsafe { oasis_register_callback(os.0, OASIS_CB_APP_LAUNCH, on_os_event) };
    tick(&os, 0.016);
    // Which app is selected before clicking (Confirm launches it).
    take_os_events();
    button(&os, OASIS_BUTTON_CONFIRM);
    tick(&os, 0.016);
    let initially_selected = take_os_events().pop().unwrap().1;
    tick(&os, 1.0);
    let (_, _, before) = frame(&os);

    // Find a click position on a *different* icon: scan a coarse grid over
    // the dashboard area like a user poking at icons.
    let mut launched = None;
    'scan: for y in (30..240).step_by(12) {
        for x in (8..470).step_by(12) {
            send(&os, OASIS_EVENT_POINTER_CLICK, x, y, 0, 0);
            send(&os, OASIS_EVENT_POINTER_RELEASE, x, y, 0, 0);
            tick(&os, 0.016);
            if let Some((_, app)) = take_os_events()
                .into_iter()
                .find(|(e, _)| *e == OASIS_CB_APP_LAUNCH)
                && app != initially_selected
            {
                launched = Some((x, y, app));
                break 'scan;
            }
        }
    }
    let (x, y, app) = launched.expect("no dashboard icon reacts to a pointer click");
    assert!(!app.is_empty());
    // The clicked icon is now selected: Confirm launches the same app.
    button(&os, OASIS_BUTTON_CONFIRM);
    tick(&os, 0.016);
    let again: Vec<_> = take_os_events()
        .into_iter()
        .filter(|(e, _)| *e == OASIS_CB_APP_LAUNCH)
        .map(|(_, a)| a)
        .collect();
    assert_eq!(
        again,
        vec![app.clone()],
        "click at ({x}, {y}) did not select {app}"
    );
    let (_, _, after) = frame(&os);
    assert_ne!(before, after);
    // Clicking empty space launches nothing.
    send(&os, OASIS_EVENT_POINTER_CLICK, 2, H as i32 - 2, 0, 0);
    tick(&os, 0.016);
    assert!(take_os_events().is_empty());
}

#[test]
fn every_event_type_and_garbage_input_is_accepted_without_crashing() {
    let os = create_classic(W, H);
    for event_type in 0..=20u32 {
        for (x, y, key, ch) in [
            (0, 0, 0, 0),
            (-5, -5, 99, 0x1F600),
            (i32::MAX, i32::MIN, u32::MAX, 0xD800), // invalid codepoint
            (100, 100, OASIS_KEY_F12, u32::from('Q')),
        ] {
            send(&os, event_type, x, y, key, ch);
        }
        tick(&os, 0.016);
    }
    let (w, h, _) = frame(&os);
    assert_eq!((w, h), (W, H));
}

#[test]
fn several_instances_are_independent() {
    let a = create_default();
    let b = create_classic(320, 200);
    let c = create_classic(1024, 768);
    add_file(&a, "/home/only_a.txt", b"a");
    command(&b, "cd /tmp");
    assert!(command(&a, "ls /home").contains("only_a.txt"));
    assert!(!command(&b, "ls /home").contains("only_a.txt"));
    assert!(command(&a, "pwd").trim_end().ends_with('/'));
    assert!(command(&b, "pwd").contains("/tmp"));

    // SAFETY: valid handles.
    unsafe {
        assert!(oasis_audio_set_volume(a.0, 10));
        assert_eq!(
            oasis_audio_get_volume(b.0),
            80,
            "volume leaked across instances"
        );
    }

    for os in [&a, &b, &c] {
        tick(os, 0.016);
    }
    assert_eq!(frame(&b).0, 320);
    assert_eq!((frame(&c).0, frame(&c).1), (1024, 768));
    assert!(distinct_colors(&frame(&c).2) > 8);
    // Destroying one leaves the others fully usable.
    drop(b);
    tick(&a, 0.5);
    tick(&c, 0.5);
    assert!(command(&c, "echo still here").contains("still here"));
}

#[test]
fn create_rejects_bad_dimensions_and_tolerates_bad_skin() {
    for (w, h) in [(0, 272), (480, 0), (4097, 10), (10, 4097)] {
        // SAFETY: null TOML pointers are allowed.
        let p = unsafe { oasis_create(w, h, std::ptr::null(), std::ptr::null(), std::ptr::null()) };
        assert!(p.is_null(), "{w}x{h} accepted");
    }
    // 1x1 and 4096x... edge sizes work.
    // SAFETY: as above.
    let tiny =
        Os(unsafe { oasis_create(1, 1, std::ptr::null(), std::ptr::null(), std::ptr::null()) });
    assert!(!tiny.0.is_null());
    tick(&tiny, 0.016);
    assert_eq!(frame(&tiny).2.len(), 4);

    // Malformed skin TOML falls back to the default theme instead of failing.
    let junk = CString::new("this is [not toml").unwrap();
    // SAFETY: valid C strings.
    let os = Os(unsafe { oasis_create(W, H, junk.as_ptr(), junk.as_ptr(), junk.as_ptr()) });
    assert!(!os.0.is_null());
    tick(&os, 0.016);
    // Only some of the skin strings: skin ignored, still a working instance.
    let m = CString::new(CLASSIC_MANIFEST).unwrap();
    // SAFETY: valid C string / null pointers.
    let os = Os(unsafe { oasis_create(W, H, m.as_ptr(), std::ptr::null(), std::ptr::null()) });
    assert!(!os.0.is_null());
    tick(&os, 0.016);
    // Invalid UTF-8 in a TOML string is rejected gracefully.
    let bad = [0xFFu8, 0xFE, 0];
    // SAFETY: NUL-terminated byte string.
    let os = Os(unsafe {
        oasis_create_full(
            W,
            H,
            bad.as_ptr().cast(),
            bad.as_ptr().cast(),
            bad.as_ptr().cast(),
            bad.as_ptr().cast(),
            bad.as_ptr().cast(),
        )
    });
    assert!(!os.0.is_null());
}

#[test]
fn set_vfs_root_resets_files_and_cwd() {
    let os = create_default();
    add_file(&os, "/home/keep.txt", b"x");
    command(&os, "cd /home");
    let root = CString::new("/game").unwrap();
    // SAFETY: valid handle / C string, and null path is allowed.
    unsafe { oasis_set_vfs_root(os.0, root.as_ptr()) };
    assert!(command(&os, "pwd").trim_end().ends_with('/'));
    assert!(!command(&os, "ls /home").contains("keep.txt"));
    // Still usable after the reset.
    add_file(&os, "/home/new.txt", b"fresh");
    assert!(command(&os, "cat /home/new.txt").contains("fresh"));
    // SAFETY: null path is documented as ignored.
    unsafe { oasis_set_vfs_root(os.0, std::ptr::null()) };
    tick(&os, 0.016);
}

#[test]
fn command_output_with_interior_nul_or_errors_is_still_a_string() {
    let os = create_default();
    let out = command(&os, "definitely-not-a-command");
    assert!(!out.is_empty());
    let out = command(&os, "");
    let _ = out;
    let out = command(&os, "cat /does/not/exist");
    assert!(!out.is_empty(), "missing file must report an error");
    // A file whose content contains NUL: the output can't be a C string as
    // is, but the call must not return NULL (the host then can't tell an
    // error from a crash).
    add_file(&os, "/home/bin.dat", b"ab\0cd");
    let c = CString::new("cat /home/bin.dat").unwrap();
    // SAFETY: valid handle and C string.
    let p = unsafe { oasis_send_command(os.0, c.as_ptr()) };
    assert!(!p.is_null(), "output with an interior NUL returned NULL");
    // SAFETY: returned by oasis_send_command.
    let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    assert!(text.starts_with("ab"), "{text:?}");
    // SAFETY: freeing once.
    unsafe { oasis_free_string(p) };
}

#[test]
fn audio_edge_cases() {
    let os = create_default();
    // SAFETY: valid handle; callback lives forever.
    unsafe {
        oasis_set_audio_callback(os.0, on_audio_event);
        take_audio_events();
        // Nothing loaded: play of an unknown id and resume fail.
        assert!(!oasis_audio_play(os.0, 12345));
        assert!(
            !oasis_audio_resume(os.0),
            "resume with nothing paused succeeded"
        );
        assert!(!oasis_audio_is_playing(os.0));
        // Volume is clamped, and the host is told the clamped value.
        assert!(oasis_audio_set_volume(os.0, 250));
        assert_eq!(oasis_audio_get_volume(os.0), 100);
        let ev = take_audio_events();
        assert_eq!(ev.last(), Some(&(4, 0, 100)), "{ev:?}");
        // Stop after play then resume must not restart playback.
        let data = b"x";
        let t = oasis_audio_load(os.0, data.as_ptr(), 1);
        assert!(oasis_audio_play(os.0, t));
        assert!(oasis_audio_stop(os.0));
        assert!(!oasis_audio_resume(os.0), "resume after stop succeeded");
        assert!(!oasis_audio_is_playing(os.0));
    }
}

#[test]
fn null_pointers_are_rejected_everywhere() {
    let null = std::ptr::null_mut();
    let s = CString::new("help").unwrap();
    // SAFETY: every export documents null-handle / null-pointer tolerance.
    unsafe {
        oasis_destroy(null);
        oasis_tick(null, 0.016);
        let (mut w, mut h) = (7u32, 7u32);
        assert!(oasis_get_buffer(null, &mut w, &mut h).is_null());
        assert_eq!((w, h), (7, 7), "null handle must not write out-params");
        assert!(!oasis_get_dirty(null));
        oasis_send_input(null, std::ptr::null());
        assert!(oasis_send_command(null, s.as_ptr()).is_null());
        oasis_free_string(std::ptr::null_mut());
        oasis_set_vfs_root(null, std::ptr::null());
        oasis_add_vfs_file(null, s.as_ptr(), b"x".as_ptr(), 1);
        oasis_register_callback(null, OASIS_CB_LOGIN, on_os_event);
        oasis_set_audio_callback(null, on_audio_event);
        assert_eq!(oasis_audio_load(null, b"x".as_ptr(), 1), u64::MAX);
    }
    let os = create_default();
    // SAFETY: valid handle with null secondary pointers.
    unsafe {
        oasis_send_input(os.0, std::ptr::null());
        assert!(oasis_send_command(os.0, std::ptr::null()).is_null());
        oasis_add_vfs_file(os.0, std::ptr::null(), b"x".as_ptr(), 1);
        oasis_add_vfs_file(os.0, s.as_ptr(), std::ptr::null(), 5);
        assert!(!oasis_get_buffer(os.0, std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        // Registering the same event twice replaces the callback (no double fire).
        oasis_register_callback(os.0, OASIS_CB_COMMAND_EXEC, on_os_event);
        oasis_register_callback(os.0, OASIS_CB_COMMAND_EXEC, on_os_event);
    }
    take_os_events();
    command(&os, "echo x");
    assert_eq!(take_os_events().len(), 1);
}

#[test]
fn buffer_pointer_is_stable_and_many_frames_do_not_leak_state() {
    let os = create_classic(W, H);
    let (mut w, mut h) = (0, 0);
    // SAFETY: valid handle.
    let p0 = unsafe { oasis_get_buffer(os.0, &mut w, &mut h) };
    for i in 0..300 {
        if i % 10 == 0 {
            button(
                &os,
                if i % 20 == 0 {
                    OASIS_BUTTON_RIGHT
                } else {
                    OASIS_BUTTON_DOWN
                },
            );
        }
        tick(&os, 1.0 / 60.0);
    }
    // SAFETY: valid handle.
    let p1 = unsafe { oasis_get_buffer(os.0, &mut w, &mut h) };
    assert_eq!(p0, p1, "framebuffer reallocated between ticks");
}
