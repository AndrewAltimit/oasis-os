//! Framebuffer capture: `screencap` (stream) and `screenshot` (file).
//!
//! # Pixel format
//!
//! The shell renders into `PSM_8888` framebuffers. A pixel is the
//! little-endian `u32` `0xAABBGGRR` -- which is why the SDK and this crate
//! call it "ABGR" -- so in memory, and therefore on the wire, every pixel
//! is the **bytes R, G, B, A**. The display ignores alpha and the GE
//! usually leaves it 0, so consumers must drop it rather than write it to
//! a PNG alpha channel. `scripts/psp_remote.py` (`psm8888_to_rgb`) is the
//! reference decoder; ffmpeg's matching input format is `rgb0`.

use core::ffi::c_void;

use psp::sys::{DisplayPixelFormat, DisplaySetBufSync, IoOpenFlags, SceUid};

use super::wire::send_response;

const SCREEN_W: usize = crate::SCREEN_WIDTH as usize;
const SCREEN_H: usize = crate::SCREEN_HEIGHT as usize;
/// VRAM row stride in pixels.
const STRIDE: usize = 512;
const BYTES_PER_PIXEL: usize = 4;

/// Uncached mirror of the start of VRAM (the first framebuffer).
const VRAM_UNCACHED: usize = 0x4400_0000;
/// OR-mask that maps a VRAM address to its uncached mirror.
const UNCACHED_BIT: usize = 0x4000_0000;

const SCREENSHOT_PATH: &[u8] = b"ms0:/seplugins/devloop_screen.raw\0";

/// The framebuffer currently on screen, as an uncached pointer.
///
/// The backend double-buffers (two framebuffers in VRAM, swapped every
/// frame), so a fixed address would capture the half-drawn back buffer
/// every other frame. Asks the display controller instead and falls back
/// to the first framebuffer if it reports nothing usable (e.g. the
/// display is not up yet, or a utility dialog switched it to 16 bpp).
fn displayed_framebuffer() -> *const u8 {
    let fb = psp::display::get_framebuf(DisplaySetBufSync::Immediate);
    let usable = !fb.top_addr.is_null()
        && fb.buf_width == STRIDE
        && matches!(fb.pixel_format, DisplayPixelFormat::Psm8888);
    if usable {
        (fb.top_addr as usize | UNCACHED_BIT) as *const u8
    } else {
        VRAM_UNCACHED as *const u8
    }
}

/// `screencap`: send the header `"480 272\n"`, then 480x272 `PSM_8888`
/// pixels (the 512-pixel stride cropped to the visible 480).
pub(super) fn send_screencap(cfd: i32) {
    send_response(cfd, format!("{SCREEN_W} {SCREEN_H}\n").as_bytes());
    let base = displayed_framebuffer();
    for row in 0..SCREEN_H {
        // SAFETY: `base` is the uncached mirror of a 512x272 32-bpp VRAM
        // framebuffer; each row slice stays inside it. VRAM is always
        // mapped and readable.
        let row_slice = unsafe {
            core::slice::from_raw_parts(
                base.add(row * STRIDE * BYTES_PER_PIXEL),
                SCREEN_W * BYTES_PER_PIXEL,
            )
        };
        send_response(cfd, row_slice);
    }
}

/// `screenshot`: dump the on-screen framebuffer, stride and all
/// (512x272 `PSM_8888`, 557,056 bytes), to `ms0:/seplugins/devloop_screen.raw`.
pub(super) fn take_screenshot() {
    let base = displayed_framebuffer();
    // SAFETY: SCREENSHOT_PATH is NUL-terminated; `base` points at a full
    // 512x272 32-bpp framebuffer in always-mapped VRAM.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            SCREENSHOT_PATH.as_ptr(),
            IoOpenFlags::CREAT | IoOpenFlags::WR_ONLY | IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= SceUid(0) {
            psp::sys::sceIoWrite(
                fd,
                base as *const c_void,
                STRIDE * SCREEN_H * BYTES_PER_PIXEL,
            );
            psp::sys::sceIoClose(fd);
        }
    }
}
