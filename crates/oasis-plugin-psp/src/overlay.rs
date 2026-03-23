//! Overlay state machine and menu logic.
//!
//! States: `Hidden` -> `OSD` (brief notification) -> `Menu` (full overlay)
//!
//! The NOTE button toggles the menu. Controller input is polled via
//! `sceCtrlPeekBufferPositive` (non-blocking, kernel-accessible).

use crate::audio;
use crate::config;
use crate::me_dump;
use crate::render::{self, SCREEN_WIDTH, colors};
use crate::video;

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
const MENU_ITEMS: u8 = 13;

/// Menu item labels.
const MENU_LABELS: [&[u8]; 13] = [
    b"  Play / Pause",
    b"  Next",
    b"  Prev",
    b"  Volume Up",
    b"  Volume Down",
    b"  Radio On/Off",
    b"  Tune Station",
    b"  CPU Clock",
    b"  PIP Play/Stop",
    b"  PIP Next",
    b"  Dump ME FW",
    b"  Init ME RPC",
    b"  Hide Overlay",
];

/// Overlay rendering dimensions.
const OVERLAY_X: u32 = 80;
const OVERLAY_Y: u32 = 40;
const OVERLAY_W: u32 = 320;
const OVERLAY_H: u32 = 232;
const ITEM_H: u32 = 14;
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
    // Validate stride to prevent integer overflow in pixel offset
    // calculations (py * stride + px). Games may pass unexpected values.
    if stride == 0 || stride > render::MAX_STRIDE {
        return;
    }

    // Poll controller via kernel-mode driver (user-mode API doesn't work
    // from the display hook context).
    let buttons = crate::hook::poll_buttons();
    // SAFETY: Single-threaded access from display hook context.
    let prev = unsafe { PREV_BUTTONS };
    let pressed = buttons & !prev; // Rising edge
    // SAFETY: Single-threaded access from display hook context.
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
                // SAFETY: CURSOR only modified from display hook (single-threaded).
                unsafe {
                    CURSOR = 0;
                }
            }
        },
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
                // SAFETY: CURSOR only modified from display hook (single-threaded).
                unsafe {
                    CURSOR = 0;
                }
            }
        },
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
        },
    }

    // Draw PIP video frame if active (before overlay UI so menu draws on top).
    if video::is_pip_active() {
        // SAFETY: fb and stride are valid; pip_frame returns valid pointer.
        unsafe {
            draw_pip(fb, stride);
        }
    }

    // No dcache flush needed -- the hook passes an uncached framebuffer
    // pointer (addr | 0x40000000), so all writes go directly to physical
    // memory and are immediately visible to the display hardware.
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
        5 => audio::toggle_radio(),
        6 => {
            if audio::is_radio_active() {
                audio::next_station();
            }
        },
        7 => cycle_cpu_clock(),
        8 => video::toggle_pip(),
        9 => video::next_video(),
        10 => {
            me_dump::trigger_dump();
            show_osd(b"ME dump started...");
        },
        11 => show_osd(b"ME RPC disabled"),
        12 => STATE.store(OverlayState::Hidden as u8, Ordering::Relaxed),
        _ => {},
    }
}

/// CPU clock preset index.
static mut CLOCK_INDEX: u8 = 0;

/// Clock presets: (PLL, CPU, Bus) in MHz.
const CLOCK_PRESETS: [(i32, i32, i32); 4] = [
    (333, 333, 166), // Max
    (266, 266, 133), // High
    (222, 222, 111), // Medium
    (133, 133, 66),  // Low (battery saver)
];

/// Cycle through CPU clock presets via scePower driver.
fn cycle_cpu_clock() {
    // SAFETY: CLOCK_INDEX only modified from display hook (single-threaded).
    unsafe {
        CLOCK_INDEX = (CLOCK_INDEX + 1) % 4;
        let (pll, cpu, bus) = CLOCK_PRESETS[CLOCK_INDEX as usize];
        if crate::hook::set_clock(pll, cpu, bus) {
            let mut buf = [0u8; 32];
            let mut p = write_str(&mut buf, 0, b"CPU: ");
            p = write_u32(&mut buf, p, cpu as u32);
            p = write_str(&mut buf, p, b"MHz");
            show_osd(&buf[..p]);
        } else {
            show_osd(b"CPU clock: not available");
        }
    }
}

/// Draw the full menu overlay.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_menu(fb: *mut u32, stride: u32) {
    // SAFETY: All render functions check bounds.
    unsafe {
        // Background
        render::fill_rect_alpha(
            fb,
            stride,
            OVERLAY_X,
            OVERLAY_Y,
            OVERLAY_W,
            OVERLAY_H,
            colors::OVERLAY_BG,
        );

        // Title bar
        render::fill_rect(
            fb,
            stride,
            OVERLAY_X,
            OVERLAY_Y,
            OVERLAY_W,
            12,
            colors::ACCENT,
        );
        render::draw_string(
            fb,
            stride,
            OVERLAY_X + 4,
            OVERLAY_Y + 2,
            b"OASIS OVERLAY",
            colors::BLACK,
        );

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
                    fb,
                    stride,
                    OVERLAY_X + 4,
                    item_y,
                    OVERLAY_W - 8,
                    ITEM_H - 2,
                    colors::HIGHLIGHT,
                );
                render::draw_string(fb, stride, OVERLAY_X + 8, item_y + 4, b">", colors::ACCENT);
            }
            render::draw_string(
                fb,
                stride,
                OVERLAY_X + 16,
                item_y + 4,
                MENU_LABELS[i as usize],
                if i == cursor {
                    colors::WHITE
                } else {
                    colors::GRAY
                },
            );
            i += 1;
        }
    }
}

/// Draw the status line with CPU clock and battery info.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_status_line(fb: *mut u32, stride: u32) {
    // SAFETY: render functions check bounds.
    unsafe {
        let mut buf = [0u8; 48];
        let mut p = write_str(&mut buf, 0, b"OASIS  ");

        let cpu_mhz = crate::hook::get_cpu_clock();
        if cpu_mhz > 0 {
            p = write_u32(&mut buf, p, cpu_mhz as u32);
            p = write_str(&mut buf, p, b"MHz  ");
        }

        let batt = crate::hook::get_battery_percent();
        if batt >= 0 {
            p = write_str(&mut buf, p, b"Batt:");
            p = write_u32(&mut buf, p, batt as u32);
            p = write_str(&mut buf, p, b"%");
        }

        render::draw_string(
            fb,
            stride,
            OVERLAY_X + 8,
            STATUS_Y,
            &buf[..p],
            colors::GREEN,
        );
    }
}

/// Draw the now-playing info: radio station + ICY metadata, or track name.
///
/// # Safety
/// `fb` must be valid.
unsafe fn draw_now_playing(fb: *mut u32, stride: u32) {
    let state = audio::audio_state();
    let y = OVERLAY_Y + 24;

    if audio::is_radio_active() {
        // Radio mode: show station name and ICY metadata.
        let station = audio::radio_station_name();
        let icon = if state == 1 { b">" } else { b"|" };

        // SAFETY: render functions check bounds.
        unsafe {
            render::draw_string(fb, stride, OVERLAY_X + 8, y, b"RADIO", colors::GREEN);
            render::draw_string(fb, stride, OVERLAY_X + 56, y, icon, colors::ACCENT);
            render::draw_string(fb, stride, OVERLAY_X + 68, y, station, colors::YELLOW);

            // ICY metadata (second line).
            let meta = audio::radio_meta();
            let mut meta_len = 0;
            while meta_len < meta.len() && meta[meta_len] != 0 {
                meta_len += 1;
            }
            if meta_len > 0 {
                // Truncate to fit overlay width.
                let max_chars = ((OVERLAY_W - 16) / 8) as usize;
                let show = meta_len.min(max_chars);
                render::draw_string(
                    fb,
                    stride,
                    OVERLAY_X + 8,
                    y + 10,
                    &meta[..show],
                    colors::GRAY,
                );
            }
        }
        return;
    }

    if state == 0 {
        return; // Audio not active.
    }

    let track = audio::current_track_name();
    // Find length (up to null terminator).
    let mut name_len = 0;
    while name_len < track.len() && track[name_len] != 0 {
        name_len += 1;
    }
    if name_len == 0 {
        return;
    }

    // Draw play/pause indicator + track name.
    let icon = if state == 1 { b"> " } else { b"||" };

    // SAFETY: render functions check bounds.
    unsafe {
        render::draw_string(fb, stride, OVERLAY_X + 8, y, icon, colors::ACCENT);
        render::draw_string(
            fb,
            stride,
            OVERLAY_X + 24,
            y,
            &track[..name_len],
            colors::YELLOW,
        );
    }
}

/// Draw the PIP video frame at the bottom-right corner with accent border.
///
/// # Safety
/// `fb` must be valid. Called from display hook when PIP is active.
unsafe fn draw_pip(fb: *mut u32, stride: u32) {
    let (pip_x, pip_y, pip_w, pip_h) = video::pip_rect();
    let border = video::pip_border();

    // Draw accent border around PIP window.
    // SAFETY: render functions check bounds.
    unsafe {
        render::fill_rect(
            fb,
            stride,
            pip_x - border,
            pip_y - border,
            pip_w + border * 2,
            pip_h + border * 2,
            colors::ACCENT,
        );
    }

    // Blit the decoded video frame.
    let (frame_ptr, w, h) = video::pip_frame();
    if !frame_ptr.is_null() {
        // SAFETY: frame_ptr is a valid PIP_W*PIP_H ABGR8888 buffer.
        unsafe {
            render::blit_rgb_rect(fb, stride, pip_x, pip_y, frame_ptr as *const u32, w, h);
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // OverlayState::from_u8
    // -----------------------------------------------------------------------

    #[test]
    fn overlay_state_from_u8_hidden() {
        assert_eq!(OverlayState::from_u8(0), OverlayState::Hidden);
    }

    #[test]
    fn overlay_state_from_u8_osd() {
        assert_eq!(OverlayState::from_u8(1), OverlayState::Osd);
    }

    #[test]
    fn overlay_state_from_u8_menu() {
        assert_eq!(OverlayState::from_u8(2), OverlayState::Menu);
    }

    #[test]
    fn overlay_state_from_u8_invalid_defaults_hidden() {
        assert_eq!(OverlayState::from_u8(3), OverlayState::Hidden);
        assert_eq!(OverlayState::from_u8(255), OverlayState::Hidden);
    }

    #[test]
    fn overlay_state_repr_values() {
        assert_eq!(OverlayState::Hidden as u8, 0);
        assert_eq!(OverlayState::Osd as u8, 1);
        assert_eq!(OverlayState::Menu as u8, 2);
    }

    // -----------------------------------------------------------------------
    // write_str
    // -----------------------------------------------------------------------

    #[test]
    fn write_str_basic() {
        let mut buf = [0u8; 32];
        let p = write_str(&mut buf, 0, b"hello");
        assert_eq!(p, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn write_str_at_offset() {
        let mut buf = [0u8; 32];
        let p = write_str(&mut buf, 3, b"ABC");
        assert_eq!(p, 6);
        assert_eq!(&buf[3..6], b"ABC");
    }

    #[test]
    fn write_str_truncation() {
        let mut buf = [0u8; 4];
        let p = write_str(&mut buf, 0, b"hello world");
        assert_eq!(p, 4);
        assert_eq!(&buf, b"hell");
    }

    #[test]
    fn write_str_empty() {
        let mut buf = [0u8; 8];
        let p = write_str(&mut buf, 0, b"");
        assert_eq!(p, 0);
    }

    #[test]
    fn write_str_at_end_of_buffer() {
        let mut buf = [0u8; 4];
        let p = write_str(&mut buf, 4, b"x");
        assert_eq!(p, 4); // No room
    }

    // -----------------------------------------------------------------------
    // write_u32
    // -----------------------------------------------------------------------

    #[test]
    fn write_u32_zero() {
        let mut buf = [0u8; 16];
        let p = write_u32(&mut buf, 0, 0);
        assert_eq!(p, 1);
        assert_eq!(buf[0], b'0');
    }

    #[test]
    fn write_u32_single_digit() {
        let mut buf = [0u8; 16];
        let p = write_u32(&mut buf, 0, 7);
        assert_eq!(p, 1);
        assert_eq!(buf[0], b'7');
    }

    #[test]
    fn write_u32_multi_digit() {
        let mut buf = [0u8; 16];
        let p = write_u32(&mut buf, 0, 333);
        assert_eq!(p, 3);
        assert_eq!(&buf[..3], b"333");
    }

    #[test]
    fn write_u32_large() {
        let mut buf = [0u8; 16];
        let p = write_u32(&mut buf, 0, 4294967295);
        assert_eq!(p, 10);
        assert_eq!(&buf[..10], b"4294967295");
    }

    #[test]
    fn write_u32_at_offset() {
        let mut buf = [0u8; 16];
        let p1 = write_str(&mut buf, 0, b"CPU: ");
        let p2 = write_u32(&mut buf, p1, 333);
        let p3 = write_str(&mut buf, p2, b"MHz");
        assert_eq!(&buf[..p3], b"CPU: 333MHz");
    }

    #[test]
    fn write_u32_buffer_too_small() {
        let mut buf = [0u8; 2];
        let p = write_u32(&mut buf, 0, 12345);
        // Can only fit 2 digits
        assert_eq!(p, 2);
        assert_eq!(&buf, b"12");
    }

    #[test]
    fn write_u32_zero_no_room() {
        let mut buf = [0u8; 0];
        let p = write_u32(&mut buf, 0, 0);
        assert_eq!(p, 0);
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn menu_items_matches_labels() {
        assert_eq!(MENU_ITEMS as usize, MENU_LABELS.len());
    }

    #[test]
    fn clock_presets_valid() {
        // Verify all presets have bus = pll / 2
        for &(pll, cpu, bus) in &CLOCK_PRESETS {
            assert_eq!(cpu, pll, "CPU should equal PLL");
            assert_eq!(bus, pll / 2, "Bus should be PLL / 2");
            assert!(pll > 0, "PLL must be positive");
        }
    }

    #[test]
    fn button_masks_distinct() {
        let buttons = [
            BTN_UP, BTN_DOWN, BTN_CROSS, BTN_L_TRIGGER, BTN_R_TRIGGER,
            BTN_START,
        ];
        for i in 0..buttons.len() {
            for j in (i + 1)..buttons.len() {
                assert_ne!(
                    buttons[i], buttons[j],
                    "Button masks must be distinct"
                );
            }
        }
    }

    #[test]
    fn overlay_dimensions_fit_screen() {
        // Overlay must fit within 480x272 PSP screen
        assert!(OVERLAY_X + OVERLAY_W <= render::SCREEN_WIDTH);
        assert!(OVERLAY_Y + OVERLAY_H <= 272);
    }

    #[test]
    fn menu_items_fit_overlay() {
        // All menu items should fit within overlay height
        let menu_bottom = MENU_START_Y + (MENU_ITEMS as u32 * ITEM_H);
        assert!(
            menu_bottom <= OVERLAY_Y + OVERLAY_H,
            "Menu items overflow overlay: bottom={} > {}",
            menu_bottom,
            OVERLAY_Y + OVERLAY_H
        );
    }
}
