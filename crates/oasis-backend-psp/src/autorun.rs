//! Boot-time autorun script support (feature `autorun-script`).
//!
//! Reads `ms0:/PSP/GAME/OASISOS/AUTORUN.txt` at boot. If present, parses one
//! command per line and dispatches one command per frame from the main loop.
//! Output (logs, raw screenshots) is written back to the memstick where a
//! host test harness can pick it up.
//!
//! # Grammar
//!
//! Lines starting with `#` are comments. Blank lines are skipped. Each
//! non-empty line is `<verb> [arg1 [arg2 ...]]`:
//!
//! - `launch <app_id>`        -- click the named dashboard icon (e.g. `settings`,
//!                               `terminal`, `browser`). App ids come from
//!                               `crate::types::APPS`.
//! - `press <button>`         -- queue a 1-frame button press + release.
//!                               Names: `cross|x|confirm`, `circle|o|cancel`,
//!                               `square`, `triangle`, `up|down|left|right`,
//!                               `start`, `select`, `ltrigger|l`, `rtrigger|r`.
//! - `cursor <x> <y>`         -- move the cursor to PSP coords (480x272).
//! - `skin <key>`             -- apply a theme preset (`psix`, `classic`,
//!                               `balatro`, `retro-cga`, `solarized`,
//!                               `highcontrast`, `altimit`).
//! - `wait <frames>`          -- pause N frames before the next command.
//! - `screenshot <ms0:/path>` -- raw 480x272 ABGR dump (one row = 512 px stride).
//! - `log <message>`          -- append a line to `autorun.log`.
//! - `exit [code]`            -- write `autorun.done`, then `sceKernelExitGame`.
//!
//! The sentinel file is deleted after parsing so a crashed script doesn't
//! re-run on next boot.

use core::ffi::c_void;

use psp::sys::{IoOpenFlags, SceUid};

use oasis_backend_psp::{Button, InputEvent, Trigger};

use oasis_backend_psp::cmd_server;

use crate::desktop;
use crate::types::APPS;

const SCRIPT_PATH: &str = "ms0:/PSP/GAME/OASISOS/AUTORUN.txt";
const LOG_PATH: &str = "ms0:/PSP/GAME/OASISOS/autorun.log";
const DONE_MARKER: &str = "ms0:/PSP/GAME/OASISOS/autorun.done";

/// One parsed autorun command.
#[derive(Debug)]
enum Cmd {
    Launch(String),
    Press(InputEvent),
    Cursor(i32, i32),
    Skin(String),
    Wait(u32),
    Screenshot(String),
    Log(String),
    Exit(i32),
}

pub struct AutorunRunner {
    cmds: Vec<Cmd>,
    wait_frames: u32,
    /// Pending button release queued by `press` — emitted next frame so the
    /// main loop sees press and release on different ticks (matches the
    /// edge-detection in `input.rs::poll_events_inner`).
    pending_release: Option<InputEvent>,
    /// When set, autorun is blocked until the host harness removes this
    /// sentinel file. Used by `screenshot` so the host has time to grab
    /// the PPSSPP window with scrot before the next command advances the
    /// emulator state.
    waiting_for_sentinel: Option<String>,
}

impl AutorunRunner {
    /// Look for the sentinel and parse it. Returns `None` if absent or
    /// unreadable. Deletes the sentinel after parsing.
    pub fn load() -> Option<Self> {
        if psp::io::stat(SCRIPT_PATH).is_err() {
            return None;
        }
        let body = match read_file(SCRIPT_PATH) {
            Some(s) => s,
            None => {
                append_log("[autorun] failed to read script");
                return None;
            },
        };
        let _ = psp::io::remove_file(SCRIPT_PATH);
        let _ = psp::io::remove_file(LOG_PATH);
        let _ = psp::io::remove_file(DONE_MARKER);

        let cmds = parse_script(&body);
        append_log(&format!("[autorun] loaded {} commands", cmds.len()));
        Some(Self {
            cmds,
            wait_frames: 0,
            pending_release: None,
            waiting_for_sentinel: None,
        })
    }

    pub fn is_done(&self) -> bool {
        self.cmds.is_empty()
            && self.pending_release.is_none()
            && self.waiting_for_sentinel.is_none()
            && self.wait_frames == 0
    }

    /// Run at most one command per frame. Call this from the main loop
    /// before input dispatch so injected events land in the same frame.
    ///
    /// `current_page` is the dashboard page (used by `launch` to skip past
    /// off-screen apps when paged dashboards are added later).
    pub fn tick(&mut self) {
        // Drain a queued release before processing the next command.
        if let Some(ev) = self.pending_release.take() {
            cmd_server::inject_event(ev);
        }
        // Block on a sentinel file (e.g. screenshot capture) until the
        // host harness removes it. Re-checked every frame.
        if let Some(req) = &self.waiting_for_sentinel {
            if psp::io::stat(req).is_err() {
                self.waiting_for_sentinel = None;
            } else {
                return;
            }
        }
        if self.wait_frames > 0 {
            self.wait_frames -= 1;
            return;
        }
        if self.cmds.is_empty() {
            return;
        }
        let cmd = self.cmds.remove(0);
        match cmd {
            Cmd::Launch(app_id) => self.do_launch(&app_id),
            Cmd::Press(press) => {
                cmd_server::inject_event(press.clone());
                self.pending_release = Some(release_for(&press));
            },
            Cmd::Cursor(x, y) => {
                cmd_server::inject_event(InputEvent::CursorMove { x, y });
                append_log(&format!("[autorun] cursor {x} {y}"));
            },
            Cmd::Skin(key) => {
                cmd_server::request_skin_change(&key);
                append_log(&format!("[autorun] skin {key}"));
            },
            Cmd::Wait(frames) => {
                self.wait_frames = frames;
                append_log(&format!("[autorun] wait {frames}"));
            },
            Cmd::Screenshot(path) => {
                save_screenshot_raw(&path);
                // Block until the host harness deletes the sentinel.
                self.waiting_for_sentinel = Some(format!("{path}.req"));
                append_log(&format!("[autorun] screenshot {path} (await capture)"));
            },
            Cmd::Log(msg) => append_log(&format!("[autorun] {msg}")),
            Cmd::Exit(code) => {
                append_log(&format!("[autorun] exit {code}"));
                touch_marker(DONE_MARKER, code);
                // SAFETY: terminating the application — no further code runs.
                unsafe { psp::sys::sceKernelExitGame() };
            },
        }
    }

    fn do_launch(&mut self, app_id: &str) {
        let idx = APPS.iter().position(|a| a.id == app_id);
        let Some(idx) = idx else {
            append_log(&format!("[autorun] launch: unknown app '{app_id}'"));
            return;
        };
        let Some((cx, cy)) = desktop::dashboard_icon_center(idx) else {
            append_log(&format!("[autorun] launch: no center for '{app_id}'"));
            return;
        };
        cmd_server::inject_event(InputEvent::CursorMove { x: cx, y: cy });
        cmd_server::inject_event(InputEvent::ButtonPress(Button::Confirm));
        self.pending_release = Some(InputEvent::ButtonRelease(Button::Confirm));
        append_log(&format!("[autorun] launch {app_id} @({cx},{cy})"));
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse_script(body: &str) -> Vec<Cmd> {
    let mut out = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        match parse_one(verb, rest) {
            Ok(cmd) => out.push(cmd),
            Err(e) => append_log(&format!("[autorun] parse err line {}: {e}", lineno + 1)),
        }
    }
    out
}

fn parse_one(verb: &str, rest: &str) -> Result<Cmd, String> {
    match verb {
        "launch" => {
            if rest.is_empty() {
                return Err("launch needs an app id".into());
            }
            Ok(Cmd::Launch(rest.to_string()))
        },
        "press" => {
            let ev = parse_button(rest).ok_or_else(|| format!("unknown button '{rest}'"))?;
            Ok(Cmd::Press(ev))
        },
        "cursor" => {
            let mut it = rest.split_whitespace();
            let x: i32 = it
                .next()
                .ok_or("cursor needs x y")?
                .parse()
                .map_err(|_| "bad x")?;
            let y: i32 = it
                .next()
                .ok_or("cursor needs x y")?
                .parse()
                .map_err(|_| "bad y")?;
            Ok(Cmd::Cursor(x, y))
        },
        "skin" => {
            if rest.is_empty() {
                return Err("skin needs a key".into());
            }
            Ok(Cmd::Skin(rest.to_string()))
        },
        "wait" => {
            let n: u32 = rest.parse().map_err(|_| "wait needs frames")?;
            Ok(Cmd::Wait(n))
        },
        "screenshot" => {
            if rest.is_empty() {
                return Err("screenshot needs a path".into());
            }
            Ok(Cmd::Screenshot(rest.to_string()))
        },
        "log" => Ok(Cmd::Log(rest.to_string())),
        "exit" => {
            let code = if rest.is_empty() {
                0
            } else {
                rest.parse().map_err(|_| "exit needs an int code")?
            };
            Ok(Cmd::Exit(code))
        },
        other => Err(format!("unknown verb '{other}'")),
    }
}

fn parse_button(name: &str) -> Option<InputEvent> {
    match name {
        "cross" | "x" | "confirm" => Some(InputEvent::ButtonPress(Button::Confirm)),
        "circle" | "o" | "cancel" => Some(InputEvent::ButtonPress(Button::Cancel)),
        "square" => Some(InputEvent::ButtonPress(Button::Square)),
        "triangle" => Some(InputEvent::ButtonPress(Button::Triangle)),
        "up" => Some(InputEvent::ButtonPress(Button::Up)),
        "down" => Some(InputEvent::ButtonPress(Button::Down)),
        "left" => Some(InputEvent::ButtonPress(Button::Left)),
        "right" => Some(InputEvent::ButtonPress(Button::Right)),
        "start" => Some(InputEvent::ButtonPress(Button::Start)),
        "select" => Some(InputEvent::ButtonPress(Button::Select)),
        "ltrigger" | "l" => Some(InputEvent::TriggerPress(Trigger::Left)),
        "rtrigger" | "r" => Some(InputEvent::TriggerPress(Trigger::Right)),
        _ => None,
    }
}

fn release_for(press: &InputEvent) -> InputEvent {
    match press {
        InputEvent::ButtonPress(b) => InputEvent::ButtonRelease(*b),
        InputEvent::TriggerPress(t) => InputEvent::TriggerRelease(*t),
        _ => InputEvent::ButtonRelease(Button::Confirm),
    }
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

fn read_file(path: &str) -> Option<String> {
    let mut path_z = path.as_bytes().to_vec();
    path_z.push(0);
    // SAFETY: null-terminated UTF-8 path; sceIo* are scalar FFI.
    unsafe {
        let fd = psp::sys::sceIoOpen(path_z.as_ptr(), IoOpenFlags::RD_ONLY, 0);
        if fd < SceUid(0) {
            return None;
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, buf.len() as u32);
            if n <= 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        psp::sys::sceIoClose(fd);
        String::from_utf8(out).ok()
    }
}

fn append_log(msg: &str) {
    let mut path_z = LOG_PATH.as_bytes().to_vec();
    path_z.push(0);
    // SAFETY: null-terminated path, valid byte buffer.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            path_z.as_ptr(),
            IoOpenFlags::APPEND | IoOpenFlags::CREAT | IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const c_void, msg.len());
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const c_void, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

fn touch_marker(path: &str, code: i32) {
    let mut path_z = path.as_bytes().to_vec();
    path_z.push(0);
    let body = format!("{code}\n");
    // SAFETY: null-terminated path, body is valid UTF-8.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            path_z.as_ptr(),
            IoOpenFlags::CREAT | IoOpenFlags::WR_ONLY | IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= SceUid(0) {
            psp::sys::sceIoWrite(fd, body.as_ptr() as *const c_void, body.len());
            psp::sys::sceIoClose(fd);
        }
    }
}

/// Request a screenshot at the given ms0: path.
///
/// On PSP hardware we'd dump the live framebuffer here (see
/// `cmd_server::take_screenshot`). In PPSSPP, however, the GU renders to
/// internal textures and never syncs pixels back to the PSP RAM mirror,
/// so reads from `0x04000000`/`0x04088000` return only the stale boot
/// screen. As a working alternative, we drop a 0-byte sentinel
/// `<path>.req` file. The host test harness watches for these and
/// captures the PPSSPP window via `scrot`. This lets the same script
/// drive both targets — real HW gets a separate VRAM-dump path; PPSSPP
/// uses the host capture.
fn save_screenshot_raw(path: &str) {
    let req_path = format!("{path}.req");
    let mut path_z = req_path.as_bytes().to_vec();
    path_z.push(0);
    // SAFETY: null-terminated path; touching a 0-byte sentinel.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            path_z.as_ptr(),
            IoOpenFlags::CREAT | IoOpenFlags::WR_ONLY | IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= SceUid(0) {
            psp::sys::sceIoClose(fd);
        }
    }
}
