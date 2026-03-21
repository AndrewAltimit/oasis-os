//! VRAM double-buffering for RGB565 framebuffer streaming.
//!
//! Two buffers in uncached VRAM. Host sends RGB565 chunks (stride-padded
//! to 512px/row), we memcpy directly into the draw buffer. On FRAME_DONE
//! the buffers swap and sceDisplaySetFrameBuf updates the display.
//!
//! Static defaults: DRAW_BUF=1, DISP_BUF=0. No explicit init() needed —
//! calling init() before USB transfers breaks recv (root cause unknown,
//! likely BSS layout sensitivity on PSP Allegrex).

use psp::sys::{DisplayPixelFormat, DisplaySetBufSync};

/// VRAM uncached mirror base address
const VRAM_UNCACHED: u32 = 0x44000000;

/// Stride-padded dimensions
const STRIDE: u32 = 512;
const BPP: u32 = 2; // RGB565

/// Size of one framebuffer in bytes (stride-padded): 512 × 272 × 2 = 278,528
const FB_SIZE: u32 = STRIDE * 272 * BPP;

/// Maximum chunk payload
pub const MAX_CHUNK_PAYLOAD: usize = 16376;

// ---------------------------------------------------------------------------
// State (volatile — written from USB callback, read from main)
// ---------------------------------------------------------------------------

/// Buffer index: 0 or 1. Static defaults work; do NOT call init() before USB.
static mut DRAW_BUF: u32 = 1; // draw to buffer 1
static mut DISP_BUF: u32 = 0; // display buffer 0

/// Frame counter
static mut FRAMES_DONE: u32 = 0;

// ---------------------------------------------------------------------------
// sceDisplaySetFrameBuf binding
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn sceDisplaySetFrameBuf(
        topaddr: *const u8,
        bufferwidth: i32,
        pixelformat: DisplayPixelFormat,
        sync: DisplaySetBufSync,
    ) -> i32;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Write a chunk of RGB565 data to the draw buffer.
///
/// `chunk_index` is 0..17 (the `flags` field from FRAME_CHUNK).
/// `data` is raw RGB565 pixels, up to MAX_CHUNK_PAYLOAD bytes.
///
/// # Safety
/// Called from USB interrupt context — no syscalls, no file I/O.
pub unsafe fn write_chunk(chunk_index: u8, data: &[u8]) {
    unsafe {
        let draw = core::ptr::read_volatile(&raw const DRAW_BUF);
        let base = vram_ptr(draw);
        let offset = chunk_index as usize * MAX_CHUNK_PAYLOAD;
        let max_remaining = (FB_SIZE as usize).saturating_sub(offset);
        let len = data.len().min(max_remaining);

        if len > 0 {
            let dst = base.add(offset);
            byte_copy(dst, data.as_ptr(), len);
        }
    }
}

/// Swap draw and display buffers, update the hardware display pointer.
///
/// Calls sceDisplaySetFrameBuf directly — tested working from USB callback
/// context on PSP hardware.
///
/// # Safety
/// Called from USB interrupt context.
pub unsafe fn swap() {
    unsafe {
        let draw = core::ptr::read_volatile(&raw const DRAW_BUF);
        let disp = core::ptr::read_volatile(&raw const DISP_BUF);

        // Swap buffer indices
        core::ptr::write_volatile(&raw mut DISP_BUF, draw);
        core::ptr::write_volatile(&raw mut DRAW_BUF, disp);

        // Update hardware display — the old draw buffer is now displayed
        sceDisplaySetFrameBuf(
            vram_ptr(draw),
            STRIDE as i32,
            DisplayPixelFormat::Psm5650,
            DisplaySetBufSync::NextFrame,
        );

        let count = core::ptr::read_volatile(&raw const FRAMES_DONE);
        core::ptr::write_volatile(&raw mut FRAMES_DONE, count + 1);
    }
}

/// Get the number of frames displayed so far.
pub fn frames_done() -> u32 {
    unsafe { core::ptr::read_volatile(&raw const FRAMES_DONE) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the uncached VRAM pointer for buffer 0 or 1.
#[inline(always)]
fn vram_ptr(index: u32) -> *mut u8 {
    (VRAM_UNCACHED + index * FB_SIZE) as *mut u8
}

/// Manual copy (avoids LLVM memcpy intrinsic recursion on MIPS).
/// Copies in 4-byte words where possible for better throughput.
///
/// # Safety
/// `dst` and `src` must be valid for `len` bytes, non-overlapping.
#[inline(never)]
unsafe fn byte_copy(dst: *mut u8, src: *const u8, len: usize) {
    let words = len / 4;
    let dst32 = dst as *mut u32;
    let src32 = src as *const u32;
    for i in 0..words {
        unsafe {
            core::ptr::write_volatile(dst32.add(i), core::ptr::read(src32.add(i)));
        }
    }
    let tail = words * 4;
    for i in tail..len {
        unsafe {
            core::ptr::write_volatile(dst.add(i), core::ptr::read(src.add(i)));
        }
    }
}
