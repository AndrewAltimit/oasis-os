//! Display framebuffer hook via CFW syscall patching.
//!
//! Intercepts `sceDisplaySetFrameBuf` to draw the overlay on top of the
//! game's framebuffer after each frame. The hook calls the original function
//! first (so the game renders normally), then draws overlay elements.

use crate::overlay;

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the hook is currently installed.
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Original `sceDisplaySetFrameBuf` function pointer.
static mut ORIGINAL_SET_FRAME_BUF: Option<
    unsafe extern "C" fn(*const u8, usize, u32, u32) -> u32,
> = None;

/// NID for sceDisplaySetFrameBuf.
const NID_SCE_DISPLAY_SET_FRAME_BUF: u32 = 0x289D82FE;

/// Our hook function that replaces `sceDisplaySetFrameBuf`.
///
/// Called in the game's display thread context every vsync. Must be fast:
/// - Call original to let the game's frame through
/// - Poll controller for trigger button
/// - If overlay active, blit the pre-rendered overlay buffer
///
/// # Safety
/// Called by the PSP OS as a syscall replacement. Arguments match
/// `sceDisplaySetFrameBuf` signature.
unsafe extern "C" fn hooked_set_frame_buf(
    top_addr: *const u8,
    buffer_width: usize,
    pixel_format: u32,
    sync: u32,
) -> u32 {
    // Call original first so the game's frame is displayed
    // SAFETY: ORIGINAL_SET_FRAME_BUF is set before the hook is active.
    let result = unsafe {
        if let Some(original) = ORIGINAL_SET_FRAME_BUF {
            original(top_addr, buffer_width, pixel_format, sync)
        } else {
            0
        }
    };

    // Only draw overlay on 32-bit ABGR framebuffers (pixel_format == 3)
    // and valid framebuffer pointers
    if !top_addr.is_null() && pixel_format == 3 {
        let fb = top_addr as *mut u32;
        let stride = buffer_width as u32;

        // Debug beacon: 2x2 green dot at (1,1) confirms the hook is running.
        // Remove once overlay is confirmed working.
        // SAFETY: Writing within screen bounds to valid framebuffer.
        unsafe {
            *fb.add((1 * stride + 1) as usize) = 0xFF00FF00;
            *fb.add((1 * stride + 2) as usize) = 0xFF00FF00;
            *fb.add((2 * stride + 1) as usize) = 0xFF00FF00;
            *fb.add((2 * stride + 2) as usize) = 0xFF00FF00;
        }

        // SAFETY: fb is a valid framebuffer pointer provided by the OS.
        // stride is the buffer width in pixels. We only write within
        // screen bounds (480x272).
        unsafe {
            overlay::on_frame(fb, stride);
        }
    }

    result
}

/// Module/library name pairs to try for finding sceDisplaySetFrameBuf.
///
/// Different CFW versions and firmware versions expose the display driver
/// under different module names. We try them in order until one works.
const DISPLAY_MODULE_NAMES: &[(&[u8], &[u8])] = &[
    (b"sceDisplay_Service\0", b"sceDisplay\0"),
    (b"sceDisplay\0", b"sceDisplay\0"),
    (b"sceDisplay_Service\0", b"sceDisplay_driver\0"),
    (b"sceDisplay\0", b"sceDisplay_driver\0"),
];

/// Install the `sceDisplaySetFrameBuf` hook.
///
/// Returns `true` on success. Must be called from kernel mode during plugin
/// initialization.
pub fn install_display_hook() -> bool {
    if HOOK_INSTALLED.load(Ordering::Relaxed) {
        return true;
    }

    // Check if sctrlHENFindFunction import was resolved properly.
    // If the stub wasn't patched by CFW, the pointer will be 0 or garbage.
    let fn_addr = psp::sys::sctrlHENFindFunction as usize;
    {
        let mut buf = [0u8; 48];
        let mut pos = write_log_bytes(&mut buf, 0, b"[OASIS] FindFunc addr=0x");
        pos = write_log_hex(&mut buf, pos, fn_addr as u32);
        crate::debug_log(&buf[..pos]);
    }

    if fn_addr == 0 || fn_addr < 0x08000000 {
        crate::debug_log(b"[OASIS] hook: sctrlHEN import NOT resolved!");
        return false;
    }

    unsafe {
        psp::sys::sceKernelDcacheWritebackAll();
    }

    // Read the actual MIPS instructions at the import stub to see what
    // the firmware patched it to (j addr, syscall, jr $ra, etc.)
    unsafe {
        let stub = fn_addr as *const u32;
        let word0 = *stub;         // First instruction
        let word1 = *stub.add(1);  // Second instruction
        let mut buf = [0u8; 64];
        let mut pos = write_log_bytes(&mut buf, 0, b"[OASIS] stub: ");
        pos = write_log_hex(&mut buf, pos, word0);
        pos = write_log_bytes(&mut buf, pos, b" ");
        pos = write_log_hex(&mut buf, pos, word1);
        crate::debug_log(&buf[..pos]);

        // Also read PatchSyscall stub
        let fn2_addr = psp::sys::sctrlHENPatchSyscall as usize;
        let stub2 = fn2_addr as *const u32;
        let w0 = *stub2;
        let w1 = *stub2.add(1);
        let mut buf2 = [0u8; 64];
        let mut pos2 = write_log_bytes(&mut buf2, 0, b"[OASIS] stub2: ");
        pos2 = write_log_hex(&mut buf2, pos2, w0);
        pos2 = write_log_bytes(&mut buf2, pos2, b" ");
        pos2 = write_log_hex(&mut buf2, pos2, w1);
        crate::debug_log(&buf2[..pos2]);
    }

    // Now try calling FindFunction
    crate::debug_log(b"[OASIS] hook: calling FindFunc...");
    unsafe {
        let test_ptr = psp::sys::sctrlHENFindFunction(
            b"sceDisplay_Service\0".as_ptr(),
            b"sceDisplay\0".as_ptr(),
            NID_SCE_DISPLAY_SET_FRAME_BUF,
        );
        if test_ptr.is_null() {
            crate::debug_log(b"[OASIS] hook: FindFunction returned NULL");
        } else {
            crate::debug_log(b"[OASIS] hook: FindFunction returned non-NULL");
        }

        crate::debug_log(b"[OASIS] hook: calling PatchSyscall...");
        if !test_ptr.is_null() {
            let ret = psp::sys::sctrlHENPatchSyscall(
                test_ptr,
                hooked_set_frame_buf as *mut u8,
            );
            if ret < 0 {
                crate::debug_log(b"[OASIS] hook: PatchSyscall FAILED");
            } else {
                crate::debug_log(b"[OASIS] hook: PatchSyscall OK");

                ORIGINAL_SET_FRAME_BUF =
                    Some(core::mem::transmute(test_ptr));

                psp::sys::sceKernelIcacheInvalidateAll();
                psp::sys::sceKernelDcacheWritebackAll();

                HOOK_INSTALLED.store(true, Ordering::Release);
                return true;
            }
        }
    }

    false
}

/// Log diagnostic info about sctrlHENFindFunction results.
///
/// Tries all known module/library name combinations and logs which ones
/// return a valid pointer vs null. Writes to the debug log file.
pub fn log_find_function_result() {
    // SAFETY: sctrlHENFindFunction is safe to call from kernel mode.
    // It just looks up function pointers without side effects.
    unsafe {
        for &(module, library) in DISPLAY_MODULE_NAMES {
            let ptr = psp::sys::sctrlHENFindFunction(
                module.as_ptr(),
                library.as_ptr(),
                NID_SCE_DISPLAY_SET_FRAME_BUF,
            );

            // Build log message: "[OASIS] FindFunc mod=X lib=Y -> 0xADDR"
            let mut buf = [0u8; 96];
            let mut pos = 0usize;
            pos = write_log_bytes(&mut buf, pos, b"[OASIS] FindFunc mod=");
            // Copy module name (without null terminator)
            pos = write_log_cstr(&mut buf, pos, module);
            pos = write_log_bytes(&mut buf, pos, b" lib=");
            pos = write_log_cstr(&mut buf, pos, library);
            pos = write_log_bytes(&mut buf, pos, b" -> ");
            if ptr.is_null() {
                pos = write_log_bytes(&mut buf, pos, b"NULL");
            } else {
                pos = write_log_bytes(&mut buf, pos, b"0x");
                pos = write_log_hex(&mut buf, pos, ptr as u32);
            }
            crate::debug_log(&buf[..pos]);
        }
    }
}

/// Write bytes into a log buffer. Returns new position.
fn write_log_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
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

/// Write a null-terminated C string (without the null) into a log buffer.
fn write_log_cstr(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    for &b in s {
        if b == 0 || p >= buf.len() {
            break;
        }
        buf[p] = b;
        p += 1;
    }
    p
}

/// Write a u32 as hexadecimal into a log buffer.
fn write_log_hex(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let mut p = pos;
    let hex = b"0123456789ABCDEF";
    // Write 8 hex digits
    let mut i = 0;
    while i < 8 {
        if p >= buf.len() {
            break;
        }
        let nibble = (val >> (28 - i * 4)) & 0xF;
        buf[p] = hex[nibble as usize];
        p += 1;
        i += 1;
    }
    p
}
