//! TCP command server for remote development automation.
//!
//! **Off unless configured.** The server starts only when
//! `ms0:/PSP/GAME/OASISOS/cmdserver.key` holds a pre-shared key (8+ bytes),
//! and every connection must pass the PSK HMAC challenge-response in
//! [`auth`] (the desktop remote terminal's protocol) first. Without the key
//! the thread still auto-connects WiFi, then exits without listening.
//!
//! When enabled it listens on port 9293 after WiFi connects, retrying the
//! WiFi connection indefinitely if the initial auto-connect fails. One
//! command per connection: after `AUTH_OK` the server reads a single line,
//! replies and closes. `scripts/psp_remote.py` is the host-side client
//! (`PSP_PSK` holds the key).
//!
//! Commands:
//!   ping              → "pong\n"
//!   status            → JSON: kiosk, free_kb, max_blk_kb, frame,
//!                        audio_only, build
//!   log               → last 2 KB of eboot.log
//!   logfull           → last 32 KB of eboot.log
//!   screencap         → "480 272\n" + the on-screen framebuffer as
//!                        PSM_8888 pixels: bytes R,G,B,A (see `screen`)
//!   screenshot        → dump the on-screen framebuffer (512 stride)
//!                        to ms0:/seplugins/devloop_screen.raw
//!   press <button>    → inject button press+release (cross,circle,up,down,
//!                        left,right,triangle,square,start,select,ltrigger,
//!                        rtrigger)
//!   hold <button> <ms> → inject button press, wait ms, then release
//!   cursor <x> <y>    → move cursor to absolute position
//!   skins             → list available skin presets
//!   skin <name>       → switch skin (applied next frame)
//!   browse <url>      → open the Browser app at an http(s) URL
//!   video-status      → JSON: state, width, height, decoded, errors,
//!                        no_pic, processed, pushed, dropped, polled,
//!                        poll_try, upload_avg_us, audio_only, me_leaked,
//!                        decode_step
//!   audio-only [on|off] → toggle/set video decode bypass
//!   deploy <size> [crc] → receive EBOOT binary with optional CRC32
//!   upload <size> <path> → write file to ms0:
//!   js <expr>         → evaluate JavaScript on a fresh QuickJS runtime
//!   timetest          → `std::time::Instant` regression check
//!   reboot            → cold reset
//!   exit              → exit to XMB

mod auth;
mod diag;
mod screen;
mod transfer;
mod wire;

use core::sync::atomic::{AtomicI32, AtomicU8, Ordering};

use oasis_core::input::{Button, InputEvent, Trigger};

use crate::network::wifi_has_ip;

use wire::{close, parse_hex_u32, parse_u32, recv, send_response, set_recv_timeout, split_arg};

/// TCP port the command server listens on.
const PORT: u16 = 9293;

/// Build identifier reported by `status` -- bump this on each deploy
/// iteration to confirm the new EBOOT is the one running.
const BUILD_ID: &str = "v48-arena-8mb-tv";

/// Skin preset keys accepted by `skin` (mirrors `PspSkinPreset::key` in
/// the binary's `skins.rs`, which this library crate cannot see).
const SKIN_KEYS: &[&str] = &[
    "psix",
    "classic",
    "balatro",
    "retro-cga",
    "solarized",
    "highcontrast",
    "altimit",
];

// ---------------------------------------------------------------------------
// Shared state between main loop and TCP server
// ---------------------------------------------------------------------------

/// Injected input events from TCP commands. Main loop drains this each frame.
static INJECT_QUEUE: psp::sync::SpscQueue<InputEvent, 32> = psp::sync::SpscQueue::new();

/// Current kiosk app state, written by main loop. 0=None, 1=Terminal, etc.
static KIOSK_STATE: AtomicU8 = AtomicU8::new(0);

/// Free heap memory in KB, updated by main loop.
static FREE_MEM_KB: AtomicI32 = AtomicI32::new(0);

/// Max contiguous block in KB, updated by main loop.
static MAX_BLK_KB: AtomicI32 = AtomicI32::new(0);

/// Frame counter, updated by main loop.
static FRAME_COUNT: AtomicI32 = AtomicI32::new(0);

/// Pending skin change request from TCP server.
/// Written by server thread, read + cleared by main loop.
/// Single Mutex ensures the skin key is read and cleared atomically.
static PENDING_SKIN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Pending browser navigation request from the TCP server. When set,
/// the main loop switches to the Browser app (if not already there)
/// and drives a `BrowserWidget::navigate_to` to the URL.
static PENDING_BROWSE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Push a synthetic input event for the main loop to consume.
pub fn inject_event(ev: InputEvent) {
    let _ = INJECT_QUEUE.push(ev);
}

/// Drain all injected events into the given vector.
pub fn drain_injected(out: &mut Vec<InputEvent>) {
    while let Some(ev) = INJECT_QUEUE.pop() {
        out.push(ev);
    }
}

/// Update status from main loop (call each frame or periodically).
pub fn update_status(kiosk: u8, free_kb: i32, max_blk_kb: i32, frame: i32) {
    KIOSK_STATE.store(kiosk, Ordering::Relaxed);
    FREE_MEM_KB.store(free_kb, Ordering::Relaxed);
    MAX_BLK_KB.store(max_blk_kb, Ordering::Relaxed);
    FRAME_COUNT.store(frame, Ordering::Relaxed);
}

/// Check for a pending skin change request. Returns the skin key if one
/// is pending. Clears the pending request atomically under the same lock.
pub fn take_pending_skin() -> Option<String> {
    let mut guard = PENDING_SKIN.lock().unwrap_or_else(|e| e.into_inner());
    guard.take()
}

/// Request a skin change. Used by the TCP server thread and the autorun
/// script runner — both pump pending changes through the same Mutex,
/// drained by `take_pending_skin` from the main loop.
pub fn request_skin_change(key: &str) {
    let mut guard = PENDING_SKIN.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(key.to_string());
}

/// Check for a pending browser navigation request. Returns the URL
/// if one is pending and clears it atomically.
pub fn take_pending_browse() -> Option<String> {
    let mut guard = PENDING_BROWSE.lock().unwrap_or_else(|e| e.into_inner());
    guard.take()
}

/// Request a browser navigation from the TCP server thread.
fn request_browse(url: &str) {
    let mut guard = PENDING_BROWSE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(url.to_string());
}

// ---------------------------------------------------------------------------
// Server thread
// ---------------------------------------------------------------------------

/// Start the network auto-connect + command server thread.
/// Call early in EBOOT init — connects WiFi in background without
/// blocking the UI, then starts TCP server.
pub fn spawn() {
    if let Ok(handle) = psp::thread::ThreadBuilder::new(b"cmd_srv\0")
        .priority(40)
        // 512 KB is large for a network handler thread, but the `js`
        // command runs QuickJS-NG inline on this thread. We keep the
        // generous allocation so a user dropping a megabyte of source into
        // `js <code>` doesn't trip a guard-page crash. The user partition
        // has slack.
        .stack_size(512 * 1024)
        .spawn(move || {
            // Auto-connect WiFi in background before server starts.
            auto_connect_wifi();
            match auth::load_psk() {
                Some(psk) => server_main(&psk),
                None => log_msg("[CMD] no cmdserver.key; command server disabled"),
            }
            0
        })
    {
        core::mem::forget(handle);
    }
}

/// Try to auto-connect to saved WiFi profile without dialog.
/// Initializes the full network stack if not already done.
fn auto_connect_wifi() {
    // Wait for WLAN hardware to initialize after boot.
    // Without this delay, sceNetApctlConnect fails because the
    // WLAN chip isn't ready yet (especially on cold reboot).
    psp::thread::sleep_ms(5000);

    if wifi_has_ip() {
        log_msg("[CMD] WiFi already connected");
        return;
    }

    // Load net modules + init stack if not done yet.
    crate::network::load_net_modules_once();
    match psp::net::init(0x20000) {
        Ok(_) => {
            log_msg("[CMD] net init OK");
            crate::network::mark_net_stack_initialized();
        },
        Err(_) => log_msg("[CMD] net init err (may be already init)"),
    }

    // SAFETY: argument-less query of the WLAN switch.
    if unsafe { psp::sys::sceWlanGetSwitchState() } == 0 {
        log_msg("[CMD] WLAN switch OFF, skipping auto-connect");
        return;
    }
    log_msg("[CMD] WLAN switch ON");

    if crate::network::connect_saved_profiles() {
        log_msg("[CMD] WiFi auto-connected");
    } else {
        log_msg("[CMD] WiFi auto-connect failed, will retry on demand");
    }
}

fn log_msg(msg: &str) {
    crate::video::vlog_force(msg);
}

/// Block until WiFi has an IP, re-running the auto-connect every ~70 s.
fn wait_for_wifi() {
    loop {
        // Wait up to 60 seconds for auto-connect to succeed.
        for _ in 0..30 {
            if wifi_has_ip() {
                log_msg("[CMD] WiFi connected");
                return;
            }
            psp::thread::sleep_ms(2000);
        }

        // Not connected — retry auto-connect after a pause. (Giving up
        // after a few attempts used to require a hard reboot.)
        log_msg("[CMD] WiFi not connected, retrying in 10s...");
        psp::thread::sleep_ms(10_000);
        auto_connect_wifi();
    }
}

/// Create the listening socket on 0.0.0.0:[`PORT`].
fn listen() -> Option<i32> {
    // SAFETY: plain socket creation (AF_INET, SOCK_STREAM).
    let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
    if fd < 0 {
        log_msg("[CMD] socket failed");
        return None;
    }

    let sa = crate::network::make_sockaddr_in_pub([0, 0, 0, 0], PORT);
    // SAFETY: `sa` is a sockaddr_in that lives across the call, and the
    // length passed is that of a `sockaddr` (same size as sockaddr_in).
    let ret = unsafe {
        psp::sys::sceNetInetBind(
            fd,
            &sa as *const _ as *const psp::sys::sockaddr,
            core::mem::size_of::<psp::sys::sockaddr>() as u32,
        )
    };
    if ret < 0 {
        log_msg("[CMD] bind failed");
        close(fd);
        return None;
    }

    // SAFETY: scalar arguments on a bound socket.
    if unsafe { psp::sys::sceNetInetListen(fd, 2) } < 0 {
        log_msg("[CMD] listen failed");
        close(fd);
        return None;
    }
    Some(fd)
}

fn server_main(psk: &str) {
    wait_for_wifi();
    let Some(fd) = listen() else {
        return;
    };
    log_msg(&format!("[CMD] TCP :{PORT} ready"));

    // Accept loop.
    loop {
        // SAFETY: `sockaddr` is plain old data; all-zero is a valid value.
        let mut client_addr: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
        let mut addr_len: u32 = core::mem::size_of::<psp::sys::sockaddr>() as u32;
        // SAFETY: both out-pointers are valid for the duration of the call.
        let cfd = unsafe { psp::sys::sceNetInetAccept(fd, &mut client_addr, &mut addr_len) };
        if cfd < 0 {
            psp::thread::sleep_ms(500);
            continue;
        }

        // Unauthenticated peers get 10 s to answer the challenge; then a
        // 30 s receive timeout so stale connections don't block the server
        // thread indefinitely.
        set_recv_timeout(cfd, 10);
        if auth::authenticate(cfd, psk) {
            set_recv_timeout(cfd, 30);
            handle_client(cfd);
        } else {
            close(cfd);
        }
    }
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn handle_client(cfd: i32) {
    let mut buf = [0u8; 256];
    let Some(n) = recv(cfd, &mut buf) else {
        close(cfd);
        return;
    };
    let raw = &buf[..n];
    let cmd = raw
        .split(|&b| b == b'\n' || b == b'\r')
        .next()
        .unwrap_or(raw);
    // Payload bytes (deploy/upload) that arrived in the same packet as
    // the command line.
    let leftover = raw
        .iter()
        .position(|&b| b == b'\n')
        .map_or(&raw[n..], |p| &raw[p + 1..]);

    match cmd {
        b"ping" => send_response(cfd, b"pong\n"),
        b"timetest" => diag::run_instant_timetest(cfd),
        b"screenshot" => {
            screen::take_screenshot();
            send_response(cfd, b"ok\n");
            log_msg("[CMD] screenshot");
        },
        b"screencap" => screen::send_screencap(cfd),
        b"exit" => {
            send_response(cfd, b"ok\n");
            close(cfd);
            log_msg("[CMD] exiting to XMB");
            psp::thread::sleep_ms(500);
            // SAFETY: terminates the application; nothing runs after it.
            unsafe { psp::sys::sceKernelExitGame() };
            return;
        },
        b"reboot" => {
            send_response(cfd, b"ok\n");
            close(cfd);
            log_msg("[CMD] cold rebooting PSP");
            psp::thread::sleep_ms(500);
            // Full hardware reboot — reloads firmware, CFW, plugins.
            // SAFETY: scalar argument; the system resets.
            unsafe { psp::sys::scePowerRequestColdReset(0) };
            return;
        },
        b"log" => transfer::send_log(cfd, 2 * 1024),
        b"logfull" => transfer::send_log(cfd, 32 * 1024),
        b"status" => send_status(cfd),
        b"video-status" => send_video_status(cfd),
        b"audio-only" => {
            let on = !crate::video::is_audio_only();
            crate::video::set_audio_only(on);
            let reply: &[u8] = if on {
                b"audio-only: on\n"
            } else {
                b"audio-only: off (video decode enabled)\n"
            };
            send_response(cfd, reply);
        },
        b"audio-only on" => {
            crate::video::set_audio_only(true);
            send_response(cfd, b"ok\n");
        },
        b"audio-only off" => {
            crate::video::set_audio_only(false);
            send_response(cfd, b"ok\n");
        },
        b"skins" => send_response(cfd, format!("{}\n", SKIN_KEYS.join(" ")).as_bytes()),
        _ => {
            if let Some(script) = cmd.strip_prefix(b"js ") {
                diag::run_js_eval(cfd, script);
            } else if let Some(name) = cmd.strip_prefix(b"press ") {
                handle_press(cfd, name, 0);
            } else if let Some(args) = cmd.strip_prefix(b"hold ") {
                handle_hold(cfd, args);
            } else if let Some(args) = cmd.strip_prefix(b"cursor ") {
                handle_cursor(cfd, args);
            } else if let Some(args) = cmd.strip_prefix(b"upload ") {
                handle_upload(cfd, args, leftover);
            } else if let Some(args) = cmd.strip_prefix(b"deploy ") {
                handle_deploy(cfd, args, leftover);
            } else if let Some(url) = cmd.strip_prefix(b"browse ") {
                handle_browse(cfd, url);
            } else if let Some(key) = cmd.strip_prefix(b"skin ") {
                handle_skin(cfd, key);
            } else {
                send_response(cfd, b"err: unknown command\n");
            }
        },
    }

    close(cfd);
}

/// `upload <size> <path>` then `<size>` bytes of file data, e.g.
/// `upload 1234 ms0:/seplugins/oasis.prx\n<data>`.
fn handle_upload(cfd: i32, args: &[u8], leftover: &[u8]) {
    let Some((size, path)) = split_arg(args) else {
        send_response(cfd, b"err: usage: upload <size> <path>\n");
        return;
    };
    let size = parse_u32(size);
    if size > 0 && size < transfer::MAX_TRANSFER_BYTES && !path.is_empty() {
        transfer::receive_file(cfd, size, path, leftover);
    } else {
        send_response(cfd, b"err: bad size or path\n");
    }
}

/// `deploy <size> [<crc32_hex>]` then `<size>` bytes of EBOOT data. The
/// CRC32 is optional for backward compatibility.
fn handle_deploy(cfd: i32, args: &[u8], leftover: &[u8]) {
    let (size, crc) = split_arg(args).unwrap_or((args, &[]));
    let size = parse_u32(size);
    let expected_crc = (!crc.is_empty()).then(|| parse_hex_u32(crc));
    if size > 0 && size < transfer::MAX_TRANSFER_BYTES {
        transfer::receive_deploy(cfd, size, leftover, expected_crc);
    } else {
        send_response(cfd, b"err: bad size\n");
    }
}

/// `browse <url>`: queue a browser navigation for the main loop, which
/// switches to the Browser app if needed.
fn handle_browse(cfd: i32, url: &[u8]) {
    let url = core::str::from_utf8(url).unwrap_or("").trim();
    if url.is_empty() {
        send_response(cfd, b"err: usage: browse <url>\n");
    } else if !(url.starts_with("http://") || url.starts_with("https://")) {
        send_response(cfd, b"err: url must start with http:// or https://\n");
    } else {
        request_browse(url);
        send_response(cfd, format!("ok: navigating to {url}\n").as_bytes());
        log_msg(&format!("[CMD] browse -> {url}"));
    }
}

fn handle_skin(cfd: i32, key: &[u8]) {
    let key = core::str::from_utf8(key).unwrap_or("").trim();
    if SKIN_KEYS.contains(&key) {
        request_skin_change(key);
        send_response(cfd, format!("ok: skin={key}\n").as_bytes());
        log_msg(&format!("[CMD] skin -> {key}"));
    } else {
        send_response(
            cfd,
            format!("err: unknown skin '{key}'. use: {}\n", SKIN_KEYS.join(", ")).as_bytes(),
        );
    }
}

// ---------------------------------------------------------------------------
// Input injection
// ---------------------------------------------------------------------------

/// Map a button name to its press event. Shared with the autorun script
/// runner so both accept the same names.
pub fn parse_button(name: &[u8]) -> Option<InputEvent> {
    match name {
        b"cross" | b"confirm" | b"x" => Some(InputEvent::ButtonPress(Button::Confirm)),
        b"circle" | b"cancel" | b"o" => Some(InputEvent::ButtonPress(Button::Cancel)),
        b"triangle" => Some(InputEvent::ButtonPress(Button::Triangle)),
        b"square" => Some(InputEvent::ButtonPress(Button::Square)),
        b"up" => Some(InputEvent::ButtonPress(Button::Up)),
        b"down" => Some(InputEvent::ButtonPress(Button::Down)),
        b"left" => Some(InputEvent::ButtonPress(Button::Left)),
        b"right" => Some(InputEvent::ButtonPress(Button::Right)),
        b"start" => Some(InputEvent::ButtonPress(Button::Start)),
        b"select" => Some(InputEvent::ButtonPress(Button::Select)),
        b"ltrigger" | b"l" => Some(InputEvent::TriggerPress(Trigger::Left)),
        b"rtrigger" | b"r" => Some(InputEvent::TriggerPress(Trigger::Right)),
        _ => None,
    }
}

/// The release event matching a press from [`parse_button`].
pub fn release_for(press: &InputEvent) -> InputEvent {
    match press {
        InputEvent::ButtonPress(b) => InputEvent::ButtonRelease(*b),
        InputEvent::TriggerPress(t) => InputEvent::TriggerRelease(*t),
        _ => InputEvent::ButtonRelease(Button::Confirm), // fallback
    }
}

fn handle_press(cfd: i32, name: &[u8], hold_ms: u32) {
    let Some(press) = parse_button(name) else {
        send_response(cfd, b"err: unknown button\n");
        return;
    };
    let release = release_for(&press);
    inject_event(press);
    // Without an explicit hold, wait briefly so the main loop sees press
    // and release on different frames.
    psp::thread::sleep_ms(if hold_ms > 0 { hold_ms } else { 100 });
    inject_event(release);
    send_response(cfd, b"ok\n");
}

/// `hold <button> <ms>`
fn handle_hold(cfd: i32, args: &[u8]) {
    let Some((name, ms)) = split_arg(args) else {
        send_response(cfd, b"err: usage: hold <button> <ms>\n");
        return;
    };
    let ms = parse_u32(ms);
    if ms == 0 || ms > 10_000 {
        send_response(cfd, b"err: ms must be 1-10000\n");
        return;
    }
    handle_press(cfd, name, ms);
}

/// `cursor <x> <y>`
fn handle_cursor(cfd: i32, args: &[u8]) {
    let Some((x, y)) = split_arg(args) else {
        send_response(cfd, b"err: usage: cursor <x> <y>\n");
        return;
    };
    inject_event(InputEvent::CursorMove {
        x: parse_u32(x) as i32,
        y: parse_u32(y) as i32,
    });
    send_response(cfd, b"ok\n");
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

fn kiosk_name(id: u8) -> &'static str {
    match id {
        0 => "none",
        1 => "terminal",
        2 => "file_manager",
        3 => "photo_viewer",
        4 => "music_player",
        5 => "browser",
        6 => "radio",
        7 => "tv_guide",
        _ => "unknown",
    }
}

fn send_status(cfd: i32) {
    let resp = format!(
        "{{\"kiosk\":\"{}\",\"free_kb\":{},\"max_blk_kb\":{},\
         \"frame\":{},\"audio_only\":{},\"build\":\"{BUILD_ID}\"}}\n",
        kiosk_name(KIOSK_STATE.load(Ordering::Relaxed)),
        FREE_MEM_KB.load(Ordering::Relaxed),
        MAX_BLK_KB.load(Ordering::Relaxed),
        FRAME_COUNT.load(Ordering::Relaxed),
        crate::video::is_audio_only(),
    );
    send_response(cfd, resp.as_bytes());
}

fn send_video_status(cfd: i32) {
    let s = crate::video::video_stats();
    let state_name = match s.state {
        crate::video::VSTATE_IDLE => "idle",
        crate::video::VSTATE_WAITING_KEYFRAME => "waiting_keyframe",
        crate::video::VSTATE_DECODING => "decoding",
        crate::video::VSTATE_AUDIO_ONLY => "audio_only",
        crate::video::VSTATE_ME_LEAKED => "me_leaked",
        _ => "unknown",
    };
    let resp = format!(
        "{{\"state\":\"{state_name}\",\"width\":{},\"height\":{},\
         \"decoded\":{},\"errors\":{},\"no_pic\":{},\"processed\":{},\
         \"pushed\":{},\"dropped\":{},\"polled\":{},\"poll_try\":{},\
         \"upload_avg_us\":{},\
         \"audio_only\":{},\"me_leaked\":{},\
         \"decode_step\":{}}}\n",
        s.width,
        s.height,
        s.decoded,
        s.errors,
        s.no_pic,
        s.processed,
        s.pushed,
        s.dropped,
        s.polled,
        s.poll_attempts,
        s.upload_us.checked_div(s.upload_count).unwrap_or(0),
        s.audio_only,
        s.me_leaked,
        s.decode_step,
    );
    send_response(cfd, resp.as_bytes());
}
