//! ME decode watchdog: hooks sceKernelWaitEventFlag to add a timeout
//! for the SceMediaEngineRpc event flag, preventing infinite deadlocks
//! in sceMpegAvcDecode's ME RPC path.

use core::sync::atomic::{AtomicI32, Ordering};

/// The SceMediaEngineRpc event flag UID.
static ME_EVFLAG_UID: AtomicI32 = AtomicI32::new(-1);

/// Original WaitEventFlag function pointer.
static mut ORIG_WAIT_EVFLAG: Option<
    unsafe extern "C" fn(i32, u32, u32, *mut u32, *mut u32) -> i32,
> = None;

/// NID for sceKernelWaitEventFlag.
const NID_WAIT_EVENT_FLAG: u32 = 0x402FCF22;

/// Hooked WaitEventFlag: adds 3-second timeout for ME RPC event flag.
unsafe extern "C" fn hooked_wait_event_flag(
    ev_id: i32,
    bits: u32,
    wait: u32,
    out_bits: *mut u32,
    timeout: *mut u32,
) -> i32 {
    let orig = match ORIG_WAIT_EVFLAG {
        Some(f) => f,
        None => return -1,
    };

    let me_uid = ME_EVFLAG_UID.load(Ordering::Relaxed);

    // ME RPC event flag with infinite timeout → add 3s timeout.
    if ev_id == me_uid && timeout.is_null() && me_uid > 0 {
        let mut timeout_us: u32 = 3_000_000;
        return orig(ev_id, bits, wait, out_bits, &mut timeout_us);
    }

    orig(ev_id, bits, wait, out_bits, timeout)
}

/// Find the SceMediaEngineRpc event flag (kernel mode).
fn find_me_rpc_event_flag() -> Option<i32> {
    let mut info: psp::sys::SceKernelEventFlagInfo = unsafe { core::mem::zeroed() };
    info.size = core::mem::size_of::<psp::sys::SceKernelEventFlagInfo>();

    let mut found_any = false;
    for uid in 1..0x10000i32 {
        info.name = [0u8; 32];
        let ret = unsafe {
            psp::sys::sceKernelReferEventFlagStatus(
                psp::sys::SceUid(uid),
                &mut info,
            )
        };
        if ret >= 0 {
            let name = info.name.split(|&b| b == 0).next().unwrap_or(&[]);
            // Log ALL found event flags (first scan only).
            if !found_any || name.starts_with(b"Sce") {
                crate::debug_log(name);
            }
            found_any = true;
            if name == b"SceMediaEngineRpc"
                || name.starts_with(b"SceMeRpc")
                || name.starts_with(b"SceMedia")
                || name.starts_with(b"SceMpeg")
                || name.starts_with(b"SceME")
                || name.starts_with(b"SceMe")
            {
                crate::debug_log(b"[ME-WD] ^^^ MATCH!");
                return Some(uid);
            }
        }
    }
    None
}

/// Spawn a kernel thread that waits for the MPEG subsystem to init,
/// then finds the event flag and installs the hook.
pub fn install() {
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisMeWd\0".as_ptr(),
            me_watchdog_thread,
            30,
            4096,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid >= psp::sys::SceUid(0) {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
        } else {
            crate::debug_log(b"[ME-WD] thread create failed");
        }
    }
}

unsafe extern "C" fn me_watchdog_thread(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    // Scan every 5 seconds up to 60 seconds for the event flag.
    for attempt in 0..12u32 {
        psp::sys::sceKernelDelayThread(5_000_000);
        if let Some(uid) = find_me_rpc_event_flag() {
            ME_EVFLAG_UID.store(uid, Ordering::Release);
            crate::debug_log(b"[ME-WD] found SceMediaEngineRpc");
            install_hook();
            return 0;
        }
        if attempt == 0 {
            crate::debug_log(b"[ME-WD] scanning for event flag...");
        }
    }
    crate::debug_log(b"[ME-WD] event flag not found after 60s");
    0
}

/// Install the hook on sceKernelWaitEventFlag.
fn install_hook() {
    let modules: &[(&[u8], &[u8])] = &[
        (b"sceThreadManager\0", b"ThreadManForKernel\0"),
        (b"sceThreadManager\0", b"ThreadManForUser\0"),
    ];

    for &(module, lib) in modules {
        let hook = unsafe {
            psp::hook::SyscallHook::install(
                module.as_ptr(),
                lib.as_ptr(),
                NID_WAIT_EVENT_FLAG,
                hooked_wait_event_flag as *mut u8,
            )
        };
        if let Some(h) = hook {
            unsafe {
                let call_addr = if h.is_inline {
                    h.trampoline.as_ptr() as *const u8
                } else {
                    h.original as *const u8
                };
                ORIG_WAIT_EVFLAG = Some(core::mem::transmute(call_addr));
            }
            core::mem::forget(h);
            crate::debug_log(b"[ME-WD] hook installed OK");
            return;
        }
    }
    crate::debug_log(b"[ME-WD] WaitEventFlag hook FAILED");
}
