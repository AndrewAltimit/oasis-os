//! Display framebuffer hook via CFW syscall patching.
//!
//! Intercepts `sceDisplaySetFrameBuf` to draw the overlay on top of the
//! game's framebuffer after each frame. Uses `psp::hook::SyscallHook` from
//! the SDK which handles kernel stub quirks, syscall patching, and inline
//! hook fallback automatically.

use crate::overlay;

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the hook is currently installed.
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// The display hook handle (owns the trampoline for inline hooks).
static mut DISPLAY_HOOK: Option<psp::hook::SyscallHook> = None;

/// NID for sceDisplaySetFrameBuf.
const NID_SCE_DISPLAY_SET_FRAME_BUF: u32 = 0x289D82FE;

/// NID for sceCtrlPeekBufferPositive.
const NID_SCE_CTRL_PEEK_BUF_POS: u32 = 0x3A622550;

/// Resolved kernel-mode sceCtrlPeekBufferPositive function pointer.
static mut CTRL_PEEK_FN: Option<unsafe extern "C" fn(*mut u8, i32) -> i32> = None;

/// Current button state, updated by the controller polling thread.
/// The display hook reads this atomically -- no API calls needed.
static CURRENT_BUTTONS: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Poll controller buttons. Reads the value set by the ctrl thread.
pub fn poll_buttons() -> u32 {
    CURRENT_BUTTONS.load(Ordering::Relaxed)
}

/// Controller polling thread entry point.
///
/// Runs in a normal kernel thread context where all APIs work.
/// Polls sceCtrlPeekBufferPositive at ~60Hz and stores the result
/// in CURRENT_BUTTONS for the display hook to read.
unsafe extern "C" fn ctrl_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    // Brief delay to let the game fully start.
    unsafe { psp::sys::sceKernelDelayThread(500_000) };

    let mut logged = false;

    loop {
        // SAFETY: CTRL_PEEK_FN is set once before this thread starts.
        let peek = unsafe { core::ptr::read_volatile(&raw const CTRL_PEEK_FN) };
        if let Some(peek) = peek {
            let mut data = [0u32; 4]; // SceCtrlData = 16 bytes
            unsafe { peek(data.as_mut_ptr() as *mut u8, 1) };
            let buttons = unsafe { core::ptr::read_volatile(&raw const data[1]) };
            CURRENT_BUTTONS.store(buttons, Ordering::Relaxed);

            // One-time diagnostic (file I/O works from thread context).
            if !logged {
                logged = true;
                let ts = unsafe { core::ptr::read_volatile(&raw const data[0]) };
                let mut buf = [0u8; 64];
                let mut pos = write_log_bytes(&mut buf, 0, b"[OASIS] ctrl ts=");
                pos = write_log_hex(&mut buf, pos, ts);
                pos = write_log_bytes(&mut buf, pos, b" btn=");
                pos = write_log_hex(&mut buf, pos, buttons);
                crate::debug_log(&buf[..pos]);
            }
        }
        unsafe { psp::sys::sceKernelDelayThread(16_000) }; // ~60fps
    }
}

/// Start the controller polling thread.
unsafe fn start_ctrl_thread() {
    // SAFETY: Creating a kernel thread for controller polling.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisCtrl\0".as_ptr(),
            ctrl_thread_entry,
            0x18, // priority
            0x1000, // 4KB stack
            psp::sys::ThreadAttributes::empty(), // kernel thread
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            crate::debug_log(b"[OASIS] ctrl thread started");
        } else {
            crate::debug_log(b"[OASIS] ctrl thread FAILED");
        }
    }
}

/// Our hook function that replaces `sceDisplaySetFrameBuf`.
///
/// # Safety
/// Called by the PSP OS as a syscall replacement.
unsafe extern "C" fn hooked_set_frame_buf(
    top_addr: *const u8,
    buffer_width: usize,
    pixel_format: u32,
    sync: u32,
) -> u32 {
    // Draw overlay BEFORE calling original so the buffer is fully
    // composited when the display hardware starts scanning it out.
    // Use uncached pointer (| 0x40000000) so writes go directly to
    // physical memory, bypassing the data cache. This eliminates
    // horizontal striping from stale cache lines.
    if !top_addr.is_null() && pixel_format == 3 {
        let fb = (top_addr as u32 | 0x4000_0000) as *mut u32;
        let stride = buffer_width as u32;

        // Debug beacon: 2x2 green dot at (1,1) confirms the hook is running.
        // SAFETY: Writing within screen bounds to valid framebuffer.
        unsafe {
            *fb.add((1 * stride + 1) as usize) = 0xFF00FF00;
            *fb.add((1 * stride + 2) as usize) = 0xFF00FF00;
            *fb.add((2 * stride + 1) as usize) = 0xFF00FF00;
            *fb.add((2 * stride + 2) as usize) = 0xFF00FF00;
        }

        // SAFETY: fb is a valid uncached framebuffer pointer.
        unsafe {
            overlay::on_frame(fb, stride);
        }
    }

    // Call original to submit the buffer to the display hardware.
    // SAFETY: DISPLAY_HOOK is set before the hook is active.
    unsafe {
        if let Some(ref hook) = DISPLAY_HOOK {
            let original: unsafe extern "C" fn(*const u8, usize, u32, u32) -> u32 =
                core::mem::transmute(hook.original_ptr());
            original(top_addr, buffer_width, pixel_format, sync)
        } else {
            0
        }
    }
}

/// Module/library name pairs to try for finding sceDisplaySetFrameBuf.
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

    // Wait for CFW and game to fully initialize.
    crate::debug_log(b"[OASIS] hook: waiting for system init...");
    unsafe {
        psp::sys::sceKernelDelayThread(2_000_000);
    }

    // Try each module/library pair until we find sceDisplaySetFrameBuf.
    let hook = unsafe {
        let mut result = None;
        for &(module, library) in DISPLAY_MODULE_NAMES {
            result = psp::hook::SyscallHook::install(
                module.as_ptr(),
                library.as_ptr(),
                NID_SCE_DISPLAY_SET_FRAME_BUF,
                hooked_set_frame_buf as *mut u8,
            );
            if result.is_some() {
                crate::debug_log(b"[OASIS] display hook installed");
                break;
            }
        }
        result
    };

    let Some(hook) = hook else {
        crate::debug_log(b"[OASIS] hook: all module/library pairs failed");
        return false;
    };

    // SAFETY: Single-threaded init, DISPLAY_HOOK is read-only after this.
    unsafe {
        DISPLAY_HOOK = Some(hook);
    }

    // Resolve sceCtrlPeekBufferPositive from the kernel driver.
    // The user-mode import doesn't work from the display hook context.
    let ctrl_names: &[(&[u8], &[u8])] = &[
        (b"sceController_Service\0", b"sceCtrl_driver\0"),
        (b"sceController_Service\0", b"sceCtrl\0"),
    ];
    unsafe {
        for &(module, library) in ctrl_names {
            if let Some(ptr) = psp::hook::find_function(
                module.as_ptr(),
                library.as_ptr(),
                NID_SCE_CTRL_PEEK_BUF_POS,
            ) {
                CTRL_PEEK_FN = Some(core::mem::transmute(ptr));
                crate::debug_log(b"[OASIS] ctrl driver resolved");
                break;
            }
        }

        if core::ptr::read_volatile(&raw const CTRL_PEEK_FN).is_none() {
            crate::debug_log(b"[OASIS] ctrl driver NOT found");
        } else {
            // Initialize controller sampling via kernel driver.
            let set_cycle = psp::hook::find_function(
                b"sceController_Service\0".as_ptr(),
                b"sceCtrl_driver\0".as_ptr(),
                0x6A2774F3, // sceCtrlSetSamplingCycle
            );
            if let Some(ptr) = set_cycle {
                let f: unsafe extern "C" fn(i32) -> i32 = core::mem::transmute(ptr);
                f(0); // 0 = VBlank sampling
            }

            let set_mode = psp::hook::find_function(
                b"sceController_Service\0".as_ptr(),
                b"sceCtrl_driver\0".as_ptr(),
                0x1F4011E6, // sceCtrlSetSamplingMode
            );
            if let Some(ptr) = set_mode {
                let f: unsafe extern "C" fn(i32) -> i32 = core::mem::transmute(ptr);
                f(1); // 1 = analog mode
                crate::debug_log(b"[OASIS] ctrl sampling initialized");
            }

            // Start the controller polling thread.
            start_ctrl_thread();
        }
    }

    HOOK_INSTALLED.store(true, Ordering::Release);
    crate::debug_log(b"[OASIS] hook installed OK");
    true
}

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

fn write_log_hex(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let mut p = pos;
    let hex = b"0123456789ABCDEF";
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
