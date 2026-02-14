//! Overlay state machine and menu logic.
//!
//! States: `Hidden` -> `OSD` (brief notification) -> `Menu` (full overlay)
//!
//! The NOTE button toggles the menu. Controller input is polled via
//! `sceCtrlPeekBufferPositive` (non-blocking, kernel-accessible).

use crate::audio;
use crate::config;
use crate::render::{self, colors, SCREEN_WIDTH};

use core::sync::atomic::{AtomicU8, Ordering};

/// Overlay display state.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
enum OverlayState {
    /// No overlay visible.
    Hidden = 0,
    /// Brief on-screen display (notification, fades after ~120 frames).
    Osd = 1,
    /// Full menu overlay with cursor.
    Menu = 2,
}

/// Current overlay state (atomic for thread-safe read from hook).
static STATE: AtomicU8 = AtomicU8::new(OverlayState::Hidden as u8);

/// Menu cursor position.
static mut CURSOR: u8 = 0;

/// OSD countdown (frames remaining).
static mut OSD_FRAMES: u16 = 0;

/// OSD message buffer.
static mut OSD_MSG: [u8; 48] = [0u8; 48];
static mut OSD_MSG_LEN: usize = 0;

/// Previous frame's button state (for edge detection).
static mut PREV_BUTTONS: u32 = 0;

/// Number of menu items.
const MENU_ITEMS: u8 = 7;

/// Menu item labels.
const MENU_LABELS: [&[u8]; 7] = [
    b"  Play / Pause",
    b"  Next Track",
    b"  Prev Track",
    b"  Volume Up",
    b"  Volume Down",
    b"  CPU Clock",
    b"  Hide Overlay",
];

/// Overlay rendering dimensions.
const OVERLAY_X: u32 = 80;
const OVERLAY_Y: u32 = 40;
const OVERLAY_W: u32 = 320;
const OVERLAY_H: u32 = 192;
const ITEM_H: u32 = 16;
const STATUS_Y: u32 = OVERLAY_Y + 8;
const MENU_START_Y: u32 = OVERLAY_Y + 48;

/// PSP button masks.
const BTN_UP: u32 = 0x10;
const BTN_DOWN: u32 = 0x40;
const BTN_CROSS: u32 = 0x4000;
const BTN_L_TRIGGER: u32 = 0x100;
const BTN_R_TRIGGER: u32 = 0x200;
const BTN_START: u32 = 0x8;

/// Called every frame from the display hook.
///
/// Polls controller input, updates state machine, and draws overlay
/// elements onto the game's framebuffer.
///
/// # Safety
/// `fb` must be a valid 32-bit ABGR framebuffer pointer with at least
/// `stride * 272` pixels. Called from the display thread context.
pub unsafe fn on_frame(fb: *mut u32, stride: u32) {
    // Poll controller (non-blocking)
    // SAFETY: SceCtrlData is repr(C), zeroed is valid.
    let mut pad = unsafe { core::mem::zeroed::<psp::sys::SceCtrlData>() };
    unsafe {
        psp::sys::sceCtrlPeekBufferPositive(&mut pad, 1);
    }

    let buttons = pad.buttons.bits();
    // SAFETY: Single-threaded access from display hook context.
    let prev = unsafe { PREV_BUTTONS };
    let pressed = buttons & !prev; // Rising edge
    unsafe {
        PREV_BUTTONS = buttons;
    }

    let trigger = config::get_config().trigger_mask();
    let state = OverlayState::from_u8(STATE.load(Ordering::Relaxed));

    // Accept either the config trigger button (NOTE/SCREEN) or L+R+START combo.
    // CFW often intercepts NOTE for its own menu, so the combo is a fallback.
    let combo = BTN_L_TRIGGER | BTN_R_TRIGGER | BTN_START;
    let combo_triggered = (buttons & combo) == combo && (prev & combo) != combo;
    let triggered = (pressed & trigger != 0) || combo_triggered;

    match state {
        OverlayState::Hidden => {
            if triggered {
                STATE.store(OverlayState::Menu as u8, Ordering::Relaxed);
                unsafe {
                    CURSOR = 0;
                }
            }
        }
        OverlayState::Osd => {
            // SAFETY: OSD state accessed only from display hook.
            unsafe {
                if OSD_FRAMES > 0 {
                    OSD_FRAMES -= 1;
                    draw_osd(fb, stride);
                }
                if OSD_FRAMES == 0 {
                    STATE.store(OverlayState::Hidden as u8, Ordering::Relaxed);
                }
            }
            if triggered {
                STATE.store(OverlayState::Menu as u8, Ordering::Relaxed);
                unsafe {
                    CURSOR = 0;
                }
            }
        }
        OverlayState::Menu => {
            if triggered {
                STATE.store(OverlayState::Hidden as u8, Ordering::Relaxed);
            } else {
                // SAFETY: CURSOR only modified in display hook.
                unsafe {
                    handle_menu_input(pressed);
                    draw_menu(fb, stride);
                }
            }
        }
    }

    // Flush dcache for the overlay region
    if state != OverlayState::Hidden {
        // SAFETY: Valid framebuffer region.
        unsafe {
            render::flush_framebuffer(fb, stride, OVERLAY_Y, OVERLAY_H);
        }
    }
}

impl OverlayState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Osd,
            2 => Self::Menu,
            _ => Self::Hidden,
        }
    }
}

/// Show a brief OSD notification.
pub fn show_osd(msg: &[u8]) {
    // SAFETY: Called from single-threaded context (audio thread or menu action).
    unsafe {
        let len = msg.len().min(47);
        let mut i = 0;
        while i < len {
            OSD_MSG[i] = msg[i];
            i += 1;
        }
        OSD_MSG[len] = 0;
        OSD_MSG_LEN = len;
        OSD_FRAMES = 120; // ~2 seconds at 60fps
    }
    STATE.store(OverlayState::Osd as u8, Ordering::Relaxed);
}

/// Draw the OSD notification bar at the top of the screen.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_osd(fb: *mut u32, stride: u32) {
    // SAFETY: OSD_MSG is valid, called from display hook.
    unsafe {
        let msg_len = OSD_MSG_LEN;
        let bar_w = (msg_len as u32 * 8) + 16;
        let bar_x = (SCREEN_WIDTH - bar_w) / 2;
        render::fill_rect_alpha(fb, stride, bar_x, 4, bar_w, 14, colors::OVERLAY_BG);
        render::draw_string(fb, stride, bar_x + 8, 7, &OSD_MSG[..msg_len], colors::WHITE);
    }
}

/// Handle menu navigation and selection.
///
/// # Safety
/// Accessed from display hook only.
unsafe fn handle_menu_input(pressed: u32) {
    // SAFETY: CURSOR only accessed from display hook.
    unsafe {
        if pressed & BTN_UP != 0 && CURSOR > 0 {
            CURSOR -= 1;
        }
        if pressed & BTN_DOWN != 0 && CURSOR < MENU_ITEMS - 1 {
            CURSOR += 1;
        }
        if pressed & BTN_CROSS != 0 {
            execute_menu_action(CURSOR);
        }
    }
}

/// Execute the selected menu action.
///
/// # Safety
/// Called from display hook context.
unsafe fn execute_menu_action(item: u8) {
    match item {
        0 => audio::toggle_playback(),
        1 => audio::next_track(),
        2 => audio::prev_track(),
        3 => audio::volume_up(),
        4 => audio::volume_down(),
        5 => cycle_cpu_clock(),
        6 => STATE.store(OverlayState::Hidden as u8, Ordering::Relaxed),
        _ => {}
    }
}

/// Cycle CPU clock between 333/266/222 MHz.
fn cycle_cpu_clock() {
    // SAFETY: Power syscalls.
    let current = unsafe { psp::sys::scePowerGetCpuClockFrequency() };
    let (cpu, bus) = match current {
        333 => (266, 133),
        266 => (222, 111),
        _ => (333, 166),
    };
    // SAFETY: Setting CPU/bus frequency.
    unsafe {
        psp::sys::scePowerSetClockFrequency(cpu, cpu, bus);
    }
    show_osd(match cpu {
        333 => b"CPU: 333 MHz (max)",
        266 => b"CPU: 266 MHz (balanced)",
        _ => b"CPU: 222 MHz (power save)",
    });
}

/// Draw the full menu overlay.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_menu(fb: *mut u32, stride: u32) {
    // SAFETY: All render functions check bounds.
    unsafe {
        // Background
        render::fill_rect_alpha(fb, stride, OVERLAY_X, OVERLAY_Y, OVERLAY_W, OVERLAY_H, colors::OVERLAY_BG);

        // Title bar
        render::fill_rect(fb, stride, OVERLAY_X, OVERLAY_Y, OVERLAY_W, 12, colors::ACCENT);
        render::draw_string(fb, stride, OVERLAY_X + 4, OVERLAY_Y + 2, b"OASIS OVERLAY", colors::BLACK);

        // Status line
        draw_status_line(fb, stride);

        // Now playing
        draw_now_playing(fb, stride);

        // Menu items
        let cursor = CURSOR;
        let mut i = 0u8;
        while (i as usize) < MENU_LABELS.len() {
            let item_y = MENU_START_Y + (i as u32 * ITEM_H);
            if i == cursor {
                render::fill_rect_alpha(
                    fb, stride,
                    OVERLAY_X + 4, item_y,
                    OVERLAY_W - 8, ITEM_H - 2,
                    colors::HIGHLIGHT,
                );
                render::draw_string(
                    fb, stride,
                    OVERLAY_X + 8, item_y + 4,
                    b">",
                    colors::ACCENT,
                );
            }
            render::draw_string(
                fb, stride,
                OVERLAY_X + 16, item_y + 4,
                MENU_LABELS[i as usize],
                if i == cursor { colors::WHITE } else { colors::GRAY },
            );
            i += 1;
        }
    }
}

/// Draw the status line (battery, CPU, time).
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_status_line(fb: *mut u32, stride: u32) {
    let mut buf = [0u8; 64];
    let mut pos = 0usize;

    // Battery
    // SAFETY: Power syscalls, no side effects.
    let bat = unsafe { psp::sys::scePowerGetBatteryLifePercent() };
    let charging = unsafe { psp::sys::scePowerIsBatteryCharging() } != 0;
    pos = write_str(&mut buf, pos, b"Bat:");
    pos = write_u32(&mut buf, pos, bat as u32);
    pos = write_str(&mut buf, pos, b"%");
    if charging {
        pos = write_str(&mut buf, pos, b"+");
    }
    pos = write_str(&mut buf, pos, b"  CPU:");

    // CPU clock
    let cpu = unsafe { psp::sys::scePowerGetCpuClockFrequency() };
    pos = write_u32(&mut buf, pos, cpu as u32);
    pos = write_str(&mut buf, pos, b"MHz  ");

    // Time
    // SAFETY: ScePspDateTime is repr(C), zeroed is valid.
    let mut dt = unsafe { core::mem::zeroed::<psp::sys::ScePspDateTime>() };
    if unsafe { psp::sys::sceRtcGetCurrentClockLocalTime(&mut dt) } >= 0 {
        pos = write_u32_pad2(&mut buf, pos, dt.hour as u32);
        pos = write_str(&mut buf, pos, b":");
        pos = write_u32_pad2(&mut buf, pos, dt.minutes as u32);
    }

    // SAFETY: buf is valid, render functions check bounds.
    unsafe {
        render::draw_string(fb, stride, OVERLAY_X + 8, STATUS_Y, &buf[..pos], colors::GREEN);
    }
}

/// Draw the now-playing track name.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_now_playing(fb: *mut u32, stride: u32) {
    let track = audio::current_track_name();
    if track[0] != 0 {
        let mut buf = [0u8; 56];
        let mut pos = write_str(&mut buf, 0, b"Now: ");
        // Copy track name (truncated)
        let mut i = 0;
        while i < track.len() && track[i] != 0 && pos < buf.len() - 1 {
            buf[pos] = track[i];
            pos += 1;
            i += 1;
        }
        // SAFETY: render functions check bounds.
        unsafe {
            render::draw_string(
                fb, stride,
                OVERLAY_X + 8, STATUS_Y + 16,
                &buf[..pos],
                colors::YELLOW,
            );
        }
    }
}

/// Write a byte string into a buffer. Returns new position.
fn write_str(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    for &b in s {
        if p >= buf.len() {
            break;
        }
        buf[p] = b;
        p += 1;
    }
    p
}

/// Write a u32 as decimal ASCII into a buffer.
fn write_u32(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            return pos + 1;
        }
        return pos;
    }
    // Write digits in reverse, then flip
    let mut digits = [0u8; 10];
    let mut n = val;
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    let mut p = pos;
    while count > 0 {
        count -= 1;
        if p >= buf.len() {
            break;
        }
        buf[p] = digits[count];
        p += 1;
    }
    p
}

/// Write a u32 as 2-digit zero-padded decimal.
fn write_u32_pad2(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let mut p = pos;
    if p + 1 < buf.len() {
        buf[p] = b'0' + ((val / 10) % 10) as u8;
        buf[p + 1] = b'0' + (val % 10) as u8;
        p += 2;
    }
    p
}
