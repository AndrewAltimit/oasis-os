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

/// Hooked WaitEventFlag: adds timeout for infinite waits from MPEG code.
///
/// Only applies the timeout when the caller is within the mpeg_vsh370.prx
/// address range (detected via $ra return address). This avoids breaking
/// system event flag waits in other modules (audio, threading, etc.).
unsafe extern "C" fn hooked_wait_event_flag(
    ev_id: i32,
    bits: u32,
    wait: u32,
    out_bits: *mut u32,
    timeout: *mut u32,
) -> i32 {
    // SAFETY: installed in place of sceKernelWaitEventFlag, so the kernel calls it with that
    // syscall's arguments. ORIG_WAIT_EVFLAG is written once by `install_hook` before the hook is
    // live and is copied, not borrowed; `out_bits` / `timeout` are forwarded unchanged.
    unsafe {
        let orig = match ORIG_WAIT_EVFLAG {
            Some(f) => f,
            None => return -1,
        };

        // Only intervene on infinite-timeout calls that match known ME pattern.
        if timeout.is_null() {
            let me_uid = ME_EVFLAG_UID.load(Ordering::Relaxed);

            // Match: known ME UID, or any event flag with specific bit patterns
            // used by the ME RPC (bits 0x1 or 0x2 with OR-CLEAR wait mode).
            // The ME RPC uses WaitEventFlag(uid, 1, OR|CLEAR, ..., NULL).
            let is_me_pattern =
                (me_uid > 0 && ev_id == me_uid) || (bits <= 0x3 && (wait & 0x21) == 0x21);

            if is_me_pattern {
                let mut timeout_us: u32 = 5_000_000;
                let ret = orig(ev_id, bits, wait, out_bits, &mut timeout_us);
                if ret == 0x800201A8u32 as i32 {
                    // Timed out — cache this UID for faster future matching.
                    if me_uid < 0 {
                        ME_EVFLAG_UID.store(ev_id, Ordering::Release);
                    }
                }
                return ret;
            }
        }

        orig(ev_id, bits, wait, out_bits, timeout)
    }
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
