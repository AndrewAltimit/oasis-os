//! Bounded allocation from the GU display list.
//!
//! `sceGuGetMemory` (rust-psp's port of the pspsdk function) never fails: it
//! bumps the list's write pointer and returns the old one, with no check
//! against the end of the buffer handed to `sceGuStart`. A frame that needs
//! more than `DISPLAY_LIST_SIZE` bytes of commands + vertices therefore
//! writes past `DISPLAY_LIST` into whatever follows it in BSS, and the GE
//! then executes garbage. Every draw primitive goes through [`alloc`] (or
//! [`has_room`] for batches the `psp` crate allocates itself), which drops
//! the draw instead once the list is nearly full.
//!
//! [`RESERVE`] bytes are always kept free for the commands emitted outside
//! these checks: clip / blend state, `sceGuClear`, and the `sceGuFinish` that
//! closes the frame. A draw that is refused emits nothing at all (callers
//! check before their first state command), so dropped draws cannot eat into
//! the reserve.

use core::sync::atomic::{AtomicU32, Ordering};

use psp::sys;

use crate::DISPLAY_LIST_SIZE;

/// Bytes kept free for unchecked state commands and `sceGuFinish`.
const RESERVE: usize = 64 * 1024;

/// Allowance for the state commands (texture bind, enable / disable, the
/// `sceGuDrawArray` itself) a primitive emits besides its vertices.
pub(crate) const STATE_SLACK: usize = 256;

/// Draws dropped because the display list was full, since boot.
static DROPPED: AtomicU32 = AtomicU32::new(0);

/// Dropped draws that are logged before going silent.
const LOG_LIMIT: u32 = 10;

/// Bytes of `DISPLAY_LIST` the open GU list has used so far.
fn used() -> usize {
    // SAFETY: sceGuCheckList only reads the current list's start and write
    // pointers. Every caller runs inside a frame opened by `sceGuStart`
    // (init / swap_buffers_inner / reinit_gu_frame), so the list is set.
    let words = unsafe { sys::sceGuCheckList() };
    // sceGuCheckList counts u32 words (`offset_from` on `*mut u32`).
    (words.max(0) as usize) * 4
}

/// Whether `bytes` more (plus [`STATE_SLACK`]) fit in the display list while
/// keeping [`RESERVE`] free. Records a dropped draw when they don't.
pub(crate) fn has_room(bytes: usize, what: &str) -> bool {
    let need = bytes.saturating_add(STATE_SLACK).saturating_add(RESERVE);
    if used().saturating_add(need) <= DISPLAY_LIST_SIZE {
        return true;
    }
    let n = DROPPED.fetch_add(1, Ordering::Relaxed);
    if n < LOG_LIMIT {
        // eboot.log, not dprintln (which draws on the screen).
        crate::video::vlog_force(&format!(
            "[GU] display list full: dropped {what} ({bytes} bytes) [{}/{LOG_LIMIT}]",
            n + 1
        ));
    }
    false
}

/// Allocate room for `count` values of `T` in the display list, or `None`
/// (logged, nothing emitted) when the list is nearly full.
///
/// The memory is valid until the GE has executed the frame (the next
/// `sceGuSync` in `swap_buffers_inner`); it is uninitialized, so every
/// element the draw uses must be written.
///
/// # Safety
///
/// Must be called inside an open GU frame (between `sceGuStart` and
/// `sceGuFinish`). `T` must need at most 4-byte alignment (sceGuGetMemory
/// returns 4-byte-aligned memory).
pub(crate) unsafe fn alloc<T>(count: usize, what: &str) -> Option<*mut T> {
    debug_assert!(core::mem::align_of::<T>() <= 4);
    let bytes = count.saturating_mul(core::mem::size_of::<T>());
    if bytes > i32::MAX as usize || !has_room(bytes, what) {
        return None;
    }
    // SAFETY: inside an open frame (caller contract), and `has_room` just
    // checked that `bytes` fit before the end of DISPLAY_LIST.
    let ptr = unsafe { sys::sceGuGetMemory(bytes as i32) };
    Some(ptr.cast::<T>())
}

/// Saturating conversion of a screen coordinate to the GE's 16-bit vertex
/// position. A plain `as i16` wraps, which flips far-off-screen geometry
/// (huge browser boxes, scrolled content) back across the screen.
pub(crate) fn coord(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
