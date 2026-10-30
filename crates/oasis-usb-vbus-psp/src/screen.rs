//! Direct VRAM screen rendering — single-frame refresh, no dprintln.
//!
//! Renders text to the PSP's VRAM framebuffer directly, avoiding the
//! psp::debug system which redraws the entire screen on every print call.

use psp::sys::{self, DisplayPixelFormat, DisplaySetBufSync};

const VRAM_UNCACHED: u32 = 0x4000_0000;
const BUF_WIDTH: usize = 512;
const SCREEN_W: usize = 480;
const SCREEN_H: usize = 272;
const CHAR_W: usize = 6; // we render 6px wide (8px font, skip 2 right)
const CHAR_H: usize = 10; // 8px font + 2px line gap

/// 8x8 MSX bitmap font, 256 glyphs × 8 bytes each.
static FONT: [u8; 2048] = *include_bytes!("msxfont.bin");

/// Framebuffer state.
static mut VRAM_BASE: *mut u32 = core::ptr::null_mut();

/// Ensure display is initialized and get VRAM pointer.
unsafe fn vram() -> *mut u32 {
    // SAFETY: VRAM_BASE is only read and written by value from the single main thread;
    // sceGeEdramGetAddr returns the always-mapped EDRAM base, used (uncached) as a BUF_WIDTH-stride
    // 32-bit framebuffer.
    unsafe {
        let base = VRAM_BASE;
        if !base.is_null() {
            return base;
        }
        let edram = sys::sceGeEdramGetAddr() as u32;
        let ptr = (VRAM_UNCACHED | edram) as *mut u32;
        sys::sceDisplaySetMode(sys::DisplayMode::Lcd, SCREEN_W, SCREEN_H);
        sys::sceDisplaySetFrameBuf(
            ptr as *const u8,
            BUF_WIDTH,
            DisplayPixelFormat::Psm8888,
            DisplaySetBufSync::NextFrame,
        );
        VRAM_BASE = ptr;
        ptr
    }
}

/// Clear entire visible framebuffer to a color.
unsafe fn clear(base: *mut u32, color: u32) {
    // SAFETY: the caller passes the framebuffer from `vram()`; x < SCREEN_W and y < SCREEN_H keep
    // every write inside its BUF_WIDTH-stride rows.
    unsafe {
        for y in 0..SCREEN_H {
            let row = base.add(y * BUF_WIDTH);
            for x in 0..SCREEN_W {
                // SAFETY: writing to uncached VRAM within bounds.
                *row.add(x) = color;
            }
        }
    }
}

/// Draw a single 8x8 glyph at pixel position (px, py).
unsafe fn put_char(base: *mut u32, px: usize, py: usize, ch: u8, color: u32) {
    // SAFETY: the caller passes the framebuffer from `vram()`; rows past SCREEN_H and columns past
    // SCREEN_W are skipped, so every write stays inside it.
    unsafe {
        let glyph = &FONT[ch as usize * 8..][..8];
        for row in 0..8 {
            let y = py + row;
            if y >= SCREEN_H {
                break;
            }
            let bits = glyph[row];
            let dst = base.add(y * BUF_WIDTH + px);
            for col in 0..8 {
                let x = px + col;
                if x >= SCREEN_W {
                    break;
                }
                if bits & (0x80 >> col) != 0 {
                    // SAFETY: writing to uncached VRAM within bounds.
                    *dst.add(col) = color;
                }
            }
        }
    }
}

/// Draw a string at pixel position (px, py). Returns chars drawn.
unsafe fn put_str(base: *mut u32, px: usize, py: usize, s: &[u8], color: u32) -> usize {
    // SAFETY: same contract as `put_char`, which bounds-checks every pixel.
    unsafe {
        let mut n = 0;
        for &ch in s {
            let x = px + n * CHAR_W;
            if x + CHAR_W > SCREEN_W {
                break;
            }
            if ch != 0 {
                put_char(base, x, py, ch, color);
            }
            n += 1;
        }
        n
    }
}

/// Color constants (ABGR8888 — PSP pixel format).
const WHITE: u32 = 0xFFFF_FFFF;
const YELLOW: u32 = 0xFF00_FFFF; // ABGR
const GRAY: u32 = 0xFF88_8888;
const CYAN: u32 = 0xFFFF_FF00;
const BG: u32 = 0xFF18_1818; // dark gray background

/// Render the full menu screen in one frame.
///
/// Clears VRAM, draws header + menu items + footer, then returns.
/// Only one VRAM write pass — no flicker.
pub fn draw_menu(items: &[&str], cursor: usize) {
    // SAFETY: kernel-mode, we own the framebuffer.
    unsafe {
        let base = vram();
        clear(base, BG);

        // Title bar
        put_str(base, 4, 2, b"USB VBUS Tool", CYAN);
        put_str(
            base,
            SCREEN_W - 25 * CHAR_W,
            2,
            b"UP/DN=nav  X=run  TRI=exit",
            GRAY,
        );

        // Separator
        for x in 0..SCREEN_W {
            *base.add(14 * BUF_WIDTH + x) = GRAY;
        }

        // Menu items
        let start_y = 18;
        for (i, label) in items.iter().enumerate() {
            let y = start_y + i * CHAR_H;
            if y + CHAR_H > SCREEN_H {
                break;
            }

            let (prefix, color) = if i == cursor {
                (b" > " as &[u8], YELLOW)
            } else {
                (b"   " as &[u8], WHITE)
            };

            put_str(base, 0, y, prefix, color);
            put_str(base, 3 * CHAR_W, y, label.as_bytes(), color);
        }
    }
}

/// Clear screen to background color.
pub fn clear_screen() {
    // SAFETY: kernel-mode, we own the framebuffer.
    unsafe {
        let base = vram();
        clear(base, BG);
    }
}
