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

/// Hooked WaitEventFlag: adds timeout for infinite waits.
///
/// Strategy: ANY WaitEventFlag call with a null timeout (infinite wait)
/// gets a 5-second timeout instead. This is safe because:
/// - Normal event flag waits complete in microseconds
/// - Only a deadlocked ME RPC would ever hit 5 seconds
/// - The caller gets SCE_KERNEL_ERROR_WAIT_TIMEOUT and can handle it
///
/// We apply this globally rather than matching a specific UID because
/// the ME event flag UID isn't known until after sceMpegCreate runs,
/// but the deadlock can happen on the first decode call.
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

    // Infinite timeout → add 5-second safety timeout.
    if timeout.is_null() {
        let mut timeout_us: u32 = 5_000_000;
        let ret = orig(ev_id, bits, wait, out_bits, &mut timeout_us);
        // If timed out, cache this UID as likely ME event flag.
        if ret == 0x800201A8u32 as i32 {
            // SCE_KERNEL_ERROR_WAIT_TIMEOUT
            let prev = ME_EVFLAG_UID.load(Ordering::Relaxed);
            if prev < 0 {
                ME_EVFLAG_UID.store(ev_id, Ordering::Release);
            }
        }
        return ret;
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

/// Install the WaitEventFlag hook immediately (no deferred scanning).
/// The hook applies a 5-second timeout to ALL infinite waits, so it
/// doesn't need to know the ME event flag UID in advance.
pub fn install() {
    install_hook();
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
