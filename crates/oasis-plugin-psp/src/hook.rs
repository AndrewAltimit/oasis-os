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

// -- scePower driver NIDs --

/// scePowerSetClockFrequency(pll, cpu, bus)
const NID_POWER_SET_CLOCK: u32 = 0x545A7F3C;
/// scePowerGetCpuClockFrequency() -> i32
const NID_POWER_GET_CPU_CLOCK: u32 = 0xFEE03A2F;
/// scePowerGetBatteryLifePercent() -> i32
const NID_POWER_GET_BATTERY: u32 = 0x2085D15D;

/// Module/library pairs for scePower driver.
const POWER_MODULES: &[(&[u8], &[u8])] = &[
    (b"scePower_Service\0", b"scePower_driver\0"),
    (b"scePower_Service\0", b"scePower\0"),
];

/// Resolved scePower function pointers.
static mut POWER_SET_CLOCK_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> = None;
static mut POWER_GET_CPU_CLOCK_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut POWER_GET_BATTERY_FN: Option<unsafe extern "C" fn() -> i32> = None;

/// Current button state, updated by the controller polling thread.
/// The display hook reads this atomically -- no API calls needed.
static CURRENT_BUTTONS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Poll controller buttons. Reads the value set by the ctrl thread.
pub fn poll_buttons() -> u32 {
    CURRENT_BUTTONS.load(Ordering::Relaxed)
}

/// Controller polling thread entry point.
///
/// Runs in a normal kernel thread context where all APIs work.
/// Polls sceCtrlPeekBufferPositive at ~60Hz and stores the result
/// in CURRENT_BUTTONS for the display hook to read.
unsafe extern "C" fn ctrl_thread_entry(_args: usize, _argp: *mut core::ffi::c_void) -> i32 {
    // SAFETY: PSP kernel syscall to sleep thread; called from kernel thread context.
    unsafe { psp::sys::sceKernelDelayThread(500_000) };

    let mut logged = false;

    loop {
        // SAFETY: CTRL_PEEK_FN is set once before this thread starts.
        let peek = unsafe { core::ptr::read_volatile(&raw const CTRL_PEEK_FN) };
        if let Some(peek) = peek {
            let mut data = [0u32; 4]; // SceCtrlData = 16 bytes
            // SAFETY: peek is a resolved kernel-mode sceCtrlPeekBufferPositive fn ptr.
            unsafe { peek(data.as_mut_ptr() as *mut u8, 1) };
            // SAFETY: Volatile read of stack-local data written by the peek call above.
            let buttons = unsafe { core::ptr::read_volatile(&raw const data[1]) };
            CURRENT_BUTTONS.store(buttons, Ordering::Relaxed);

            // One-time diagnostic (file I/O works from thread context).
            if !logged {
                logged = true;
                // SAFETY: Volatile read of stack-local data populated by peek above.
                let ts = unsafe { core::ptr::read_volatile(&raw const data[0]) };
                let mut buf = [0u8; 64];
                let mut pos = write_log_bytes(&mut buf, 0, b"[OASIS] ctrl ts=");
                pos = write_log_hex(&mut buf, pos, ts);
                pos = write_log_bytes(&mut buf, pos, b" btn=");
                pos = write_log_hex(&mut buf, pos, buttons);
                crate::debug_log(&buf[..pos]);
            }
        }
        // SAFETY: PSP kernel syscall to sleep thread for frame pacing.
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
            0x18,                                // priority
            0x1000,                              // 4KB stack
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
    // SAFETY: PSP kernel syscall to delay thread during single-threaded init.
    unsafe {
        psp::sys::sceKernelDelayThread(2_000_000);
    }

    // Try each module/library pair until we find sceDisplaySetFrameBuf.
    // SAFETY: SyscallHook::install performs kernel-mode CFW syscall patching
    // via sctrlHENFindFunction. Called during single-threaded init from psp_main.
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
    // SAFETY: Resolving kernel driver function pointers via sctrlHENFindFunction
    // and transmuting to typed fn pointers. Single-threaded init; statics are
    // written once here and read-only afterwards.
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

    // Resolve scePower driver functions for CPU clock and battery.
    // SAFETY: Resolving kernel driver function pointers via sctrlHENFindFunction
    // and transmuting to typed fn pointers. Volatile reads/writes to statics
    // during single-threaded init; read-only afterwards.
    unsafe {
        for &(module, library) in POWER_MODULES {
            if core::ptr::read_volatile(&raw const POWER_SET_CLOCK_FN).is_none() {
                if let Some(ptr) =
                    psp::hook::find_function(module.as_ptr(), library.as_ptr(), NID_POWER_SET_CLOCK)
                {
                    core::ptr::write_volatile(
                        &raw mut POWER_SET_CLOCK_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const POWER_GET_CPU_CLOCK_FN).is_none() {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_POWER_GET_CPU_CLOCK,
                ) {
                    core::ptr::write_volatile(
                        &raw mut POWER_GET_CPU_CLOCK_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const POWER_GET_BATTERY_FN).is_none() {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_POWER_GET_BATTERY,
                ) {
                    core::ptr::write_volatile(
                        &raw mut POWER_GET_BATTERY_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
        }

        if core::ptr::read_volatile(&raw const POWER_SET_CLOCK_FN).is_some() {
            crate::debug_log(b"[OASIS] power driver resolved");
        } else {
            crate::debug_log(b"[OASIS] power driver NOT found");
        }
    }

    HOOK_INSTALLED.store(true, Ordering::Release);
    crate::debug_log(b"[OASIS] hook installed OK");
    true
}

/// Set CPU/bus clock frequencies.
///
/// # Safety
/// Must only be called after `install_display_hook()`.
pub unsafe fn set_clock(pll: i32, cpu: i32, bus: i32) -> bool {
    // SAFETY: POWER_SET_CLOCK_FN is set once during init.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const POWER_SET_CLOCK_FN) {
            f(pll, cpu, bus);
            true
        } else {
            false
        }
    }
}

/// Get current CPU clock frequency in MHz. Returns 0 if unavailable.
pub fn get_cpu_clock() -> i32 {
    // SAFETY: POWER_GET_CPU_CLOCK_FN is set once during init.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const POWER_GET_CPU_CLOCK_FN) {
            f()
        } else {
            0
        }
    }
}

/// Get battery life percentage. Returns -1 if unavailable.
pub fn get_battery_percent() -> i32 {
    // SAFETY: POWER_GET_BATTERY_FN is set once during init.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const POWER_GET_BATTERY_FN) {
            f()
        } else {
            -1
        }
    }
}

/// Bounded write helper for fixed-size log buffers. Appends `s` to
/// `buf` starting at `pos`, clamping to the buffer length via
/// `saturating_add`. Returns the new write position (never exceeds
/// `buf.len()`).
fn write_log_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let end = pos.saturating_add(s.len()).min(buf.len());
    let count = end.saturating_sub(pos);
    if count > 0 {
        buf[pos..end].copy_from_slice(&s[..count]);
    }
    end
}

/// Bounded hex-format helper. Writes up to 8 hex digits of `val` into
/// `buf` starting at `pos`, clamping to the buffer length via
/// `saturating_add`. Returns the new write position.
fn write_log_hex(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let hex = b"0123456789ABCDEF";
    let needed = 8usize;
    let end = pos.saturating_add(needed).min(buf.len());
    let count = end.saturating_sub(pos);
    let mut i = 0;
    while i < count {
        let nibble = (val >> (28 - i * 4)) & 0xF;
        buf[pos + i] = hex[nibble as usize];
        i += 1;
    }
    end
}
