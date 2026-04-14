//! TCP command server for remote development automation.
//!
//! Listens on port 9293 after WiFi connects. Retries WiFi connection
//! indefinitely if initial auto-connect fails.
//!
//! Commands:
//!   ping              → "pong\n"
//!   screenshot        → saves VRAM to ms0:/seplugins/devloop_screen.raw
//!   screencap         → streams raw ABGR pixels (480x272)
//!   reboot            → cold reset
//!   exit              → exit to XMB
//!   log               → responds with last 2KB of eboot.log
//!   logfull           → responds with last 32KB of eboot.log
//!   status            → JSON: kiosk, free_kb, max_blk_kb, frame,
//!                        audio_only, build
//!   video-status      → JSON: state, width, height, decoded, errors,
//!                        no_pic, processed, pushed, dropped, polled,
//!                        poll_try, upload_avg_us, audio_only, me_leaked,
//!                        frame_limit, decode_step
//!   video-limit <N>   → set max video frames for >480p
//!   audio-only [on|off] → toggle/set video decode bypass
//!   press <button>    → inject button press+release (cross,circle,up,down,
//!                        left,right,triangle,square,start,select,ltrigger,
//!                        rtrigger)
//!   hold <button> <ms> → inject button press, wait ms, then release
//!   cursor <x> <y>    → move cursor to absolute position
//!   skins             → list available skin presets
//!   skin <name>        → switch skin (applied next frame)
//!   deploy <size> [crc] → receive EBOOT binary with optional CRC32
//!   upload <size> <path> → write file to ms0:

use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU8, Ordering};

use oasis_core::input::{Button, InputEvent, Trigger};

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

/// Build identifier — bump this on each deploy iteration.
const BUILD_ID: &str = "v48-arena-8mb-tv";

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

/// Request a skin change from the TCP server thread.
fn request_skin_change(key: &str) {
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
        // command runs QuickJS-NG inline on this thread. QuickJS's
        // parser + bytecode compiler uses modest stack depth compared
        // to boa's old AST, but we keep the generous allocation so a
        // user dropping a megabyte of source into `js <code>` doesn't
        // trip a guard-page crash. The user partition has slack.
        .stack_size(512 * 1024)
        .spawn(move || {
            // Auto-connect WiFi in background before server starts.
            auto_connect_wifi();
            server_main();
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

    // Check if already connected via apctl state (more reliable than
    // psp::net::is_connected which uses a separate flag).
    let mut state = psp::sys::ApctlState::Disconnected;
    unsafe { psp::sys::sceNetApctlGetState(&mut state) };
    if matches!(state, psp::sys::ApctlState::GotIp) {
        log_msg("[CMD] WiFi already connected");
        return;
    }

    // Load net modules + init stack if not done yet.
    crate::network::load_net_modules_once();
    match psp::net::init(0x20000) {
        Ok(_) => {
            log_msg("[CMD] net init OK");
            crate::network::mark_net_stack_initialized();
        }
        Err(_) => log_msg("[CMD] net init err (may be already init)"),
    }

    // Check WLAN switch is on.
    let wlan = unsafe { psp::sys::sceWlanGetSwitchState() };
    log_msg(if wlan != 0 {
        "[CMD] WLAN switch ON"
    } else {
        "[CMD] WLAN switch OFF, skipping auto-connect"
    });
    if wlan == 0 {
        return;
    }

    // Try profiles 1 and 0.
    for profile in [1i32, 0] {
        let ret = unsafe { psp::sys::sceNetApctlConnect(profile) };
        if ret < 0 {
            // Log hex error code.
            log_msg(&format!(
                "[CMD] apctl connect({}) = 0x{:08x}", profile, ret as u32
            ));
            continue;
        }

        let mut connected = false;
        for _ in 0..30 {
            let mut state = psp::sys::ApctlState::Disconnected;
            unsafe { psp::sys::sceNetApctlGetState(&mut state) };
            if matches!(state, psp::sys::ApctlState::GotIp) {
                connected = true;
                break;
            }
            psp::thread::sleep_ms(500);
        }

        if connected {
            log_msg("[CMD] WiFi auto-connected");
            return;
        }
        unsafe { psp::sys::sceNetApctlDisconnect() };
        psp::thread::sleep_ms(500);
    }

    log_msg("[CMD] WiFi auto-connect failed, will retry on demand");
}

fn log_msg(msg: &str) {
    crate::video::vlog_force(msg);
}

fn server_main() {
    // Retry WiFi connection indefinitely. Previous behavior gave up
    // after 2 attempts, requiring a hard reboot to recover networking.
    loop {
        // Check if already connected.
        let mut state = psp::sys::ApctlState::Disconnected;
        unsafe { psp::sys::sceNetApctlGetState(&mut state) };
        if matches!(state, psp::sys::ApctlState::GotIp) {
            log_msg("[CMD] WiFi connected");
            break;
        }

        // Wait up to 60 seconds for auto-connect to succeed.
        let mut connected = false;
        for _ in 0..30 {
            let mut st = psp::sys::ApctlState::Disconnected;
            unsafe { psp::sys::sceNetApctlGetState(&mut st) };
            if matches!(st, psp::sys::ApctlState::GotIp) {
                connected = true;
                break;
            }
            psp::thread::sleep_ms(2000);
        }
        if connected {
            log_msg("[CMD] WiFi connected");
            break;
        }

        // Not connected — retry auto-connect after a pause.
        log_msg("[CMD] WiFi not connected, retrying in 10s...");
        psp::thread::sleep_ms(10_000);
        auto_connect_wifi();
    }

    // Create TCP server socket.
    let fd = unsafe {
        psp::sys::sceNetInetSocket(2, 1, 0) // AF_INET, SOCK_STREAM
    };
    if fd < 0 {
        log_msg("[CMD] socket failed");
        return;
    }

    // Bind to 0.0.0.0:9293.
    let sa = crate::network::make_sockaddr_in_pub([0, 0, 0, 0], 9293);
    let ret = unsafe {
        psp::sys::sceNetInetBind(
            fd,
            &sa as *const _ as *const psp::sys::sockaddr,
            core::mem::size_of::<psp::sys::sockaddr>() as u32,
        )
    };
    if ret < 0 {
        log_msg("[CMD] bind failed");
        unsafe { psp::sys::sceNetInetClose(fd) };
        return;
    }

    let ret = unsafe { psp::sys::sceNetInetListen(fd, 2) };
    if ret < 0 {
        log_msg("[CMD] listen failed");
        unsafe { psp::sys::sceNetInetClose(fd) };
        return;
    }

    log_msg("[CMD] TCP :9293 ready");

    // Accept loop.
    loop {
        let mut client_addr: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
        let mut addr_len: u32 = core::mem::size_of::<psp::sys::sockaddr>() as u32;
        let cfd = unsafe {
            psp::sys::sceNetInetAccept(fd, &mut client_addr, &mut addr_len)
        };
        if cfd < 0 {
            psp::thread::sleep_ms(500);
            continue;
        }

        // Set receive timeout (30 seconds) so stale connections don't block
        // the server thread indefinitely.
        set_recv_timeout(cfd, 30);

        handle_client(cfd);
    }
}

fn handle_client(cfd: i32) {
    let mut buf = [0u8; 256];
    let n = unsafe {
        psp::sys::sceNetInetRecv(
            cfd,
            buf.as_mut_ptr() as *mut c_void,
            256,
            0,
        )
    };

    if n <= 0 {
        unsafe { psp::sys::sceNetInetClose(cfd) };
        return;
    }

    let raw = &buf[..n as usize];
    let cmd = raw.split(|&b| b == b'\n' || b == b'\r')
        .next()
        .unwrap_or(raw);

    if cmd == b"ping" {
        send_response(cfd, b"pong\n");
    } else if cmd == b"timetest" {
        run_instant_timetest(cfd);
    } else if cmd.starts_with(b"js ") {
        let script = &cmd[3..];
        run_js_eval(cfd, script);
    } else if cmd == b"screenshot" {
        take_screenshot();
        send_response(cfd, b"ok\n");
        log_msg("[CMD] screenshot");
    } else if cmd == b"exit" {
        send_response(cfd, b"ok\n");
        unsafe { psp::sys::sceNetInetClose(cfd) };
        log_msg("[CMD] exiting to XMB");
        psp::thread::sleep_ms(500);
        unsafe { psp::sys::sceKernelExitGame() };
        return;
    } else if cmd == b"reboot" {
        send_response(cfd, b"ok\n");
        unsafe { psp::sys::sceNetInetClose(cfd) };
        log_msg("[CMD] cold rebooting PSP");
        psp::thread::sleep_ms(500);
        // Full hardware reboot — reloads firmware, CFW, plugins.
        unsafe { psp::sys::scePowerRequestColdReset(0) };
        return;
    } else if cmd == b"log" {
        send_log(cfd, 2048);
    } else if cmd == b"logfull" {
        send_log(cfd, 32768);
    } else if cmd == b"status" {
        send_status(cfd);
    } else if cmd.starts_with(b"press ") {
        handle_press(cfd, &cmd[6..], 0);
    } else if cmd.starts_with(b"hold ") {
        handle_hold(cfd, &cmd[5..]);
    } else if cmd.starts_with(b"cursor ") {
        handle_cursor(cfd, &cmd[7..]);
    } else if cmd.starts_with(b"video-limit ") {
        let n = parse_u32(&cmd[12..]);
        crate::video::set_video_frame_limit(n);
        send_response(cfd, &format!("video-limit: {n}\n").into_bytes());
    } else if cmd == b"audio-only" {
        let current = crate::video::is_audio_only();
        crate::video::set_audio_only(!current);
        if !current {
            send_response(cfd, b"audio-only: on\n");
        } else {
            send_response(cfd, b"audio-only: off (video decode enabled)\n");
        }
    } else if cmd == b"video-status" {
        send_video_status(cfd);
    } else if cmd == b"audio-only on" {
        crate::video::set_audio_only(true);
        send_response(cfd, b"ok\n");
    } else if cmd == b"audio-only off" {
        crate::video::set_audio_only(false);
        send_response(cfd, b"ok\n");
    } else if cmd == b"screencap" {
        send_screencap(cfd);
    } else if cmd.starts_with(b"upload ") {
        // Protocol: "upload <size> <path>\n" then <size> bytes of file data.
        // Example: "upload 1234 ms0:/seplugins/oasis.prx\n"<data>
        let args = &cmd[7..];
        let parts: Vec<&[u8]> = args.splitn(2, |&b| b == b' ').collect();
        if parts.len() >= 2 {
            let size = parse_u32(parts[0]);
            let path = parts[1];
            if size > 0 && size < 24_000_000 && !path.is_empty() {
                let header_end = raw.iter().position(|&b| b == b'\n')
                    .map(|p| p + 1).unwrap_or(n as usize);
                let leftover = &buf[header_end..n as usize];
                receive_file(cfd, size, path, leftover);
            } else {
                send_response(cfd, b"err: bad size or path\n");
            }
        } else {
            send_response(cfd, b"err: usage: upload <size> <path>\n");
        }
    } else if cmd.starts_with(b"deploy ") {
        // Protocol: "deploy <size> [<crc32_hex>]\n" then <size> bytes of EBOOT data.
        // CRC32 is optional for backward compatibility.
        let args = &cmd[7..];
        let parts: Vec<&[u8]> = args.splitn(2, |&b| b == b' ').collect();
        let size = parse_u32(parts[0]);
        let expected_crc = if parts.len() >= 2 && !parts[1].is_empty() {
            Some(parse_hex_u32(parts[1]))
        } else {
            None
        };
        if size > 0 && size < 24_000_000 {
            let header_end = raw.iter().position(|&b| b == b'\n')
                .map(|p| p + 1).unwrap_or(n as usize);
            let leftover = &buf[header_end..n as usize];
            receive_deploy(cfd, size, leftover, expected_crc);
        } else {
            send_response(cfd, b"err: bad size\n");
        }
    } else if cmd == b"skins" {
        send_response(
            cfd,
            b"psix classic balatro retro-cga solarized highcontrast terminal altimit tactical\n",
        );
    } else if cmd.starts_with(b"browse ") {
        // Protocol: "browse <url>\n" — queue a browser navigation for
        // the main loop. Switches to the Browser app if needed.
        let url = core::str::from_utf8(&cmd[7..]).unwrap_or("").trim();
        if url.is_empty() {
            send_response(cfd, b"err: usage: browse <url>\n");
        } else if !(url.starts_with("http://") || url.starts_with("https://")) {
            send_response(cfd, b"err: url must start with http:// or https://\n");
        } else {
            request_browse(url);
            send_response(cfd, format!("ok: navigating to {}\n", url).as_bytes());
            log_msg(&format!("[CMD] browse -> {}", url));
        }
    } else if cmd.starts_with(b"skin ") {
        let key = core::str::from_utf8(&cmd[5..]).unwrap_or("").trim();
        let known = [
            "psix", "classic", "balatro", "retro-cga", "solarized",
            "highcontrast", "terminal", "altimit", "tactical",
        ];
        if known.contains(&key) {
            request_skin_change(key);
            send_response(cfd, format!("ok: skin={}\n", key).as_bytes());
            log_msg(&format!("[CMD] skin -> {}", key));
        } else {
            send_response(
                cfd,
                format!(
                    "err: unknown skin '{}'. use: {}\n",
                    key,
                    known.join(", "),
                )
                .as_bytes(),
            );
        }
    } else {
        send_response(cfd, b"err: unknown command\n");
    }

    unsafe { psp::sys::sceNetInetClose(cfd) };
}

// ---------------------------------------------------------------------------
// Input injection
// ---------------------------------------------------------------------------

fn parse_button(name: &[u8]) -> Option<InputEvent> {
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

fn release_for(press: &InputEvent) -> InputEvent {
    match press {
        InputEvent::ButtonPress(b) => InputEvent::ButtonRelease(*b),
        InputEvent::TriggerPress(t) => InputEvent::TriggerRelease(*t),
        _ => InputEvent::ButtonRelease(Button::Confirm), // fallback
    }
}

fn handle_press(cfd: i32, name: &[u8], hold_ms: u32) {
    if let Some(press) = parse_button(name) {
        let release = release_for(&press);
        let _ = INJECT_QUEUE.push(press);
        if hold_ms > 0 {
            psp::thread::sleep_ms(hold_ms);
        } else {
            // Brief hold so the main loop sees press and release on different frames.
            psp::thread::sleep_ms(100);
        }
        let _ = INJECT_QUEUE.push(release);
        send_response(cfd, b"ok\n");
    } else {
        send_response(cfd, b"err: unknown button\n");
    }
}

fn handle_hold(cfd: i32, args: &[u8]) {
    // "hold <button> <ms>"
    let parts: Vec<&[u8]> = args.splitn(2, |&b| b == b' ').collect();
    if parts.len() < 2 {
        send_response(cfd, b"err: usage: hold <button> <ms>\n");
        return;
    }
    let ms = parse_u32(parts[1]);
    if ms == 0 || ms > 10_000 {
        send_response(cfd, b"err: ms must be 1-10000\n");
        return;
    }
    handle_press(cfd, parts[0], ms);
}

fn handle_cursor(cfd: i32, args: &[u8]) {
    // "cursor <x> <y>"
    let parts: Vec<&[u8]> = args.splitn(2, |&b| b == b' ').collect();
    if parts.len() < 2 {
        send_response(cfd, b"err: usage: cursor <x> <y>\n");
        return;
    }
    let x = parse_u32(parts[0]) as i32;
    let y = parse_u32(parts[1]) as i32;
    let _ = INJECT_QUEUE.push(InputEvent::CursorMove { x, y });
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
    let kiosk = KIOSK_STATE.load(Ordering::Relaxed);
    let free = FREE_MEM_KB.load(Ordering::Relaxed);
    let max_blk = MAX_BLK_KB.load(Ordering::Relaxed);
    let frame = FRAME_COUNT.load(Ordering::Relaxed);

    let audio_only = crate::video::is_audio_only();
    let resp = format!(
        "{{\"kiosk\":\"{}\",\"free_kb\":{},\"max_blk_kb\":{},\
         \"frame\":{},\"audio_only\":{},\"build\":\"{}\"}}\n",
        kiosk_name(kiosk), free, max_blk, frame, audio_only, BUILD_ID,
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
         \"audio_only\":{},\"me_leaked\":{},\"frame_limit\":{},\
         \"decode_step\":{}}}\n",
        s.width, s.height, s.decoded, s.errors, s.no_pic,
        s.processed, s.pushed, s.dropped, s.polled, s.poll_attempts,
        if s.upload_count > 0 { s.upload_us / s.upload_count } else { 0 },
        s.audio_only, s.me_leaked, s.frame_limit, s.decode_step,
    );
    send_response(cfd, resp.as_bytes());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Set SO_RCVTIMEO on a socket (timeout in seconds).
fn set_recv_timeout(fd: i32, secs: u32) {
    // struct timeval { long tv_sec; long tv_usec; }
    let timeval: [i32; 2] = [secs as i32, 0];
    unsafe {
        psp::sys::sceNetInetSetsockopt(
            fd,
            0xFFFF, // SOL_SOCKET
            0x1006, // SO_RCVTIMEO
            timeval.as_ptr() as *const c_void,
            8,      // sizeof(timeval) on PSP
        );
    }
}

/// CRC32 (ISO 3309) — same polynomial as zlib.crc32() on the host.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Incrementally update a running CRC32 value with a new chunk.
fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Parse a hex string (e.g. b"1a2b3c4d") into a u32.
fn parse_hex_u32(data: &[u8]) -> u32 {
    let mut val = 0u32;
    for &b in data {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        val = (val << 4) | nibble as u32;
    }
    val
}

fn parse_u32(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |acc, &b| {
        if b >= b'0' && b <= b'9' { acc * 10 + (b - b'0') as u32 } else { acc }
    })
}

/// Probe `std::time::Instant` on PSP hardware. **Now passes** —
/// kept around as a regression check for the rust-psp std overlay
/// fix in branch `fix/psp-hardware-std-overlay-alignment-and-time`
/// (see `MEMORY.md` and `docs/browser-backlog.md` for the full
/// debugging story). Earlier sessions believed `Instant::now`
/// crashed on Allegrex; the real cause was that the rust-psp std
/// overlay had no `target_os = "psp"` arm in the new
/// `sys/time/mod.rs`, so PSP fell through to the panicking
/// `unsupported::Instant` shim. With the overlay wired up, every
/// `Instant`/`Duration` API works as expected on real hardware.
fn run_instant_timetest(cfd: i32) {
    log_msg("[timetest] start");
    send_response(cfd, b"timetest: start\n");

    log_msg("[timetest] calling Instant::now() #1");
    let t0 = std::time::Instant::now();
    log_msg("[timetest] Instant::now() #1 ok");

    psp::thread::sleep_ms(10);

    log_msg("[timetest] calling Instant::now() #2");
    let t1 = std::time::Instant::now();
    log_msg("[timetest] Instant::now() #2 ok");

    log_msg("[timetest] computing duration_since");
    let d = t1.duration_since(t0);
    log_msg("[timetest] duration_since ok");

    log_msg("[timetest] calling as_micros");
    let us = d.as_micros();
    let msg = format!("[timetest] elapsed={}us\n", us);
    log_msg(&msg);

    log_msg("[timetest] calling elapsed() on t0");
    let el = t0.elapsed();
    let el_us = el.as_micros();
    let msg2 = format!("[timetest] t0.elapsed()={}us\n", el_us);
    log_msg(&msg2);

    log_msg("[timetest] all ok");
    let reply = format!("timetest: ok elapsed={}us t0_elapsed={}us\n", us, el_us);
    send_response(cfd, reply.as_bytes());
}

/// Evaluate a one-shot JavaScript expression on the QuickJS-NG-backed
/// `oasis_js::JsEngine` and stream the result back over TCP.
///
/// Protocol: `js <script>\n` -> `<value>\n` on success or
/// `js: error: <message>\n` on failure. The engine is created fresh
/// per invocation — no shared state across calls — so this is purely
/// for smoke-testing the runtime end-to-end on real hardware. The
/// browser-facing JS path will own a long-lived engine when DOM
/// bindings are ported (see `docs/browser-backlog.md` "PSP JavaScript
/// integration").
///
/// Examples (from the host):
///
/// ```text
/// $ echo "js 1 + 2 + 3" | nc -w 5 192.168.0.249 9293
/// js: 6
/// $ echo "js 'foo' + 'bar'" | nc -w 5 192.168.0.249 9293
/// js: foobar
/// ```
fn run_js_eval(cfd: i32, script: &[u8]) {
    let script_str = match core::str::from_utf8(script) {
        Ok(s) => s.trim(),
        Err(_) => {
            send_response(cfd, b"js: error: script is not valid UTF-8\n");
            return;
        }
    };
    log_msg(&format!("[js] eval ({} bytes) source={:?}", script_str.len(), script_str));

    // Construct QuickJS without going through `oasis_js::JsEngine` so
    // we can log between every rquickjs step. This lets us pinpoint
    // the exact call that hard-crashes the console on real hardware
    // — the higher-level wrapper would hide where in the init chain
    // we died.
    use oasis_js::rquickjs;

    log_msg("[js] rquickjs::Runtime::new() begin");
    let runtime = match rquickjs::Runtime::new() {
        Ok(r) => {
            log_msg("[js] rquickjs::Runtime::new() ok");
            r
        },
        Err(e) => {
            log_msg(&format!("[js] Runtime::new() err: {e}"));
            send_response(cfd, b"js: error: runtime init\n");
            return;
        },
    };

    log_msg("[js] rquickjs::Context::base() begin");
    let context = match rquickjs::Context::base(&runtime) {
        Ok(c) => {
            log_msg("[js] rquickjs::Context::base() ok");
            c
        },
        Err(e) => {
            log_msg(&format!("[js] Context::base() err: {e}"));
            send_response(cfd, b"js: error: context init\n");
            return;
        },
    };

    log_msg("[js] context.with(eval) begin");
    context.with(|ctx| {
        log_msg("[js] inside context.with");

        log_msg("[js] probe 1: grabbing raw ctx ptr");
        let raw_ctx = unsafe { ctx.as_raw().as_ptr() };
        log_msg(&format!("[js] probe 1 ctx ptr = {:?}", raw_ctx));

        // Probe 2: get the global object. No parsing, no allocation
        // beyond a refcount bump. If this crashes, the JSContext
        // itself is corrupt. If it survives, ctx is fine and the
        // crash is parser/eval-specific.
        log_msg("[js] probe 2: JS_GetGlobalObject");
        let gval = unsafe {
            oasis_js::rquickjs::qjs::JS_GetGlobalObject(raw_ctx)
        };
        log_msg("[js] probe 2: JS_GetGlobalObject returned");
        unsafe { oasis_js::rquickjs::qjs::JS_FreeValue(raw_ctx, gval) };
        log_msg("[js] probe 2: JS_FreeValue done");

        // Probe 3: raw JS_Eval of "0" — shortest legal program.
        // Isolates whether it's the parser, the codegen, or the
        // interpreter that faults.
        log_msg("[js] probe 3: raw JS_Eval \"0\"");
        let src = b"0\0".as_ptr() as *const core::ffi::c_char;
        let fname = b"<probe>\0".as_ptr() as *const core::ffi::c_char;
        let val = unsafe {
            oasis_js::rquickjs::qjs::JS_Eval(
                raw_ctx,
                src,
                1,
                fname,
                oasis_js::rquickjs::qjs::JS_EVAL_TYPE_GLOBAL as i32,
            )
        };
        log_msg("[js] probe 3: JS_Eval returned");
        unsafe { oasis_js::rquickjs::qjs::JS_FreeValue(raw_ctx, val) };
        log_msg("[js] probe 3: JS_FreeValue done");
    });
    log_msg("[js] context.with returned");

    send_response(cfd, b"js: ok (see log for details)\n");
}

fn send_response(cfd: i32, data: &[u8]) {
    unsafe {
        psp::sys::sceNetInetSend(
            cfd,
            data.as_ptr() as *const c_void,
            data.len(),
            0,
        );
    }
}

fn take_screenshot() {
    const FB_SIZE: usize = 512 * 272 * 4;
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/seplugins/devloop_screen.raw\0".as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(
                fd,
                0x44000000u32 as *const c_void,
                FB_SIZE,
            );
            psp::sys::sceIoClose(fd);
        }
    }
}

/// Send raw framebuffer (480x272 ABGR) over TCP.
/// Reads from VRAM at 0x44000000 with stride 512, crops to 480 width.
fn send_screencap(cfd: i32) {
    // Send header: "480 272\n" then 480*272*4 bytes of pixel data.
    send_response(cfd, b"480 272\n");
    let vram = 0x44000000u32 as *const u8;
    for row in 0..272u32 {
        let row_ptr = unsafe { vram.add((row * 512 * 4) as usize) };
        // SAFETY: reading from VRAM mapped region, 480*4 bytes per row.
        let row_slice = unsafe {
            core::slice::from_raw_parts(row_ptr, 480 * 4)
        };
        send_response(cfd, row_slice);
    }
}

/// Receive an arbitrary file over TCP and write to the given ms0: path.
fn receive_file(cfd: i32, size: u32, path: &[u8], leftover: &[u8]) {
    // Build null-terminated path.
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path);
    path_buf.push(0);

    log_msg(&format!("[CMD] upload: {} bytes → {}",
        size, core::str::from_utf8(path).unwrap_or("?")));

    let fd = unsafe {
        psp::sys::sceIoOpen(
            path_buf.as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send_response(cfd, b"err: can't create file\n");
        return;
    }

    let mut received = 0u32;
    if !leftover.is_empty() {
        unsafe {
            psp::sys::sceIoWrite(fd, leftover.as_ptr() as *const c_void, leftover.len());
        }
        received += leftover.len() as u32;
    }

    let mut buf = [0u8; 4096];
    while received < size {
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                cfd, buf.as_mut_ptr() as *mut c_void,
                buf.len().min((size - received) as usize), 0,
            )
        };
        if n <= 0 { break; }
        unsafe {
            psp::sys::sceIoWrite(fd, buf.as_ptr() as *const c_void, n as usize);
        }
        received += n as u32;
    }

    unsafe { psp::sys::sceIoClose(fd) };

    if received == size {
        log_msg("[CMD] upload: OK");
        send_response(cfd, b"ok\n");
    } else {
        log_msg("[CMD] upload: incomplete");
        send_response(cfd, b"err: incomplete transfer\n");
    }
}

/// Receive EBOOT binary over TCP and write to ms0:.
/// Writes to a temp file first, then renames over the live EBOOT.
/// If `expected_crc` is provided, validates CRC32 after transfer.
fn receive_deploy(cfd: i32, size: u32, leftover: &[u8], expected_crc: Option<u32>) {
    const TEMP_PATH: *const u8 =
        b"ms0:/PSP/GAME/OASISOS/EBOOT.PBP.tmp\0".as_ptr();
    const FINAL_PATH: *const u8 =
        b"ms0:/PSP/GAME/OASISOS/EBOOT.PBP\0".as_ptr();

    log_msg("[CMD] deploy: receiving EBOOT");

    // Open temp file.
    let fd = unsafe {
        psp::sys::sceIoOpen(
            TEMP_PATH,
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send_response(cfd, b"err: can't create temp file\n");
        return;
    }

    let mut received = 0u32;
    let mut running_crc: u32 = 0;

    // Write leftover bytes from the initial recv.
    if !leftover.is_empty() {
        unsafe {
            psp::sys::sceIoWrite(
                fd,
                leftover.as_ptr() as *const c_void,
                leftover.len(),
            );
        }
        if expected_crc.is_some() {
            running_crc = crc32_update(running_crc, leftover);
        }
        received += leftover.len() as u32;
    }

    // Receive remaining data in chunks.
    let mut buf = [0u8; 4096];
    while received < size {
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                cfd,
                buf.as_mut_ptr() as *mut c_void,
                buf.len().min((size - received) as usize),
                0,
            )
        };
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        unsafe {
            psp::sys::sceIoWrite(
                fd,
                chunk.as_ptr() as *const c_void,
                chunk.len(),
            );
        }
        if expected_crc.is_some() {
            running_crc = crc32_update(running_crc, chunk);
        }
        received += n as u32;
    }

    unsafe { psp::sys::sceIoClose(fd) };

    if received != size {
        // Incomplete transfer — clean up temp.
        unsafe { psp::sys::sceIoRemove(TEMP_PATH) };
        log_msg("[CMD] deploy: incomplete");
        send_response(cfd, b"err: incomplete transfer\n");
        return;
    }

    // CRC32 validation if client provided a checksum.
    if let Some(expected) = expected_crc {
        if running_crc != expected {
            unsafe { psp::sys::sceIoRemove(TEMP_PATH) };
            log_msg(&format!(
                "[CMD] deploy: CRC mismatch (got {:08x}, expected {:08x})",
                running_crc, expected,
            ));
            send_response(cfd, b"err: crc mismatch\n");
            return;
        }
    }

    // Rename temp → final (atomic-ish on FAT).
    unsafe {
        psp::sys::sceIoRemove(FINAL_PATH);
        psp::sys::sceIoRename(TEMP_PATH, FINAL_PATH);
    }
    let resp = format!("ok {:08x}\n", running_crc);
    log_msg("[CMD] deploy: OK");
    send_response(cfd, resp.as_bytes());
}

fn send_log(cfd: i32, max_bytes: usize) {
    let fd = unsafe {
        psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send_response(cfd, b"err: no log\n");
        return;
    }

    // Seek to last N bytes.
    let size = unsafe {
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End)
    };
    let offset = if size > max_bytes as i64 {
        size - max_bytes as i64
    } else {
        0
    };
    unsafe {
        psp::sys::sceIoLseek(fd, offset, psp::sys::IoWhence::Set);
    }

    // Read and send in chunks (stack-friendly).
    let mut buf = [0u8; 2048];
    let mut remaining = max_bytes;
    loop {
        let to_read = remaining.min(buf.len());
        let n = unsafe {
            psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, to_read as u32)
        };
        if n <= 0 {
            break;
        }
        send_response(cfd, &buf[..n as usize]);
        remaining -= n as usize;
        if remaining == 0 {
            break;
        }
    }

    unsafe { psp::sys::sceIoClose(fd) };
}
