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
        // SAFETY: top_addr is a valid framebuffer pointer provided by the OS.
        // buffer_width is the stride in pixels. We only write within
        // screen bounds (480x272).
        unsafe {
            overlay::on_frame(top_addr as *mut u32, buffer_width as u32);
        }
    }

    result
}

/// Install the `sceDisplaySetFrameBuf` hook.
///
/// Returns `true` on success. Must be called from kernel mode during plugin
/// initialization.
pub fn install_display_hook() -> bool {
    if HOOK_INSTALLED.load(Ordering::Relaxed) {
        return true;
    }

    // SAFETY: We are in kernel mode (module_kernel!). The hook module/library
    // names and NID are well-known constants for the PSP display driver.
    unsafe {
        let hook = psp::hook::SyscallHook::install(
            b"sceDisplay_Service\0".as_ptr(),
            b"sceDisplay\0".as_ptr(),
            NID_SCE_DISPLAY_SET_FRAME_BUF,
            hooked_set_frame_buf as *mut u8,
        );

        match hook {
            Some(h) => {
                // Store the original function pointer for the trampoline
                ORIGINAL_SET_FRAME_BUF = Some(core::mem::transmute(h.original_ptr()));

                // Flush caches to ensure the patched syscall is visible
                psp::sys::sceKernelIcacheInvalidateAll();
                psp::sys::sceKernelDcacheWritebackAll();

                HOOK_INSTALLED.store(true, Ordering::Release);
                true
            }
            None => false,
        }
    }
}
