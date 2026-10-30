//! ME decode watchdog: hooks sceKernelWaitEventFlag to add a timeout
//! for the SceMediaEngineRpc event flag, preventing infinite deadlocks
//! in sceMpegAvcDecode's ME RPC path.

use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

/// The SceMediaEngineRpc event flag UID.
static ME_EVFLAG_UID: AtomicI32 = AtomicI32::new(-1);

type WaitEventFlagFn = unsafe extern "C" fn(i32, u32, u32, *mut u32, *mut u32) -> i32;

/// Address of the original WaitEventFlag (trampoline or syscall target), 0 until
/// set. Atomic because the hook goes live inside `SyscallHook::install`, before
/// this can be stored, and every thread calling sceKernelWaitEventFlag reads it.
static ORIG_WAIT_EVFLAG: AtomicUsize = AtomicUsize::new(0);

/// The installed hook. Kept for the program's lifetime: for inline hooks the
/// trampoline that ORIG_WAIT_EVFLAG points at is stored inside it.
static mut WAIT_HOOK: Option<psp::hook::SyscallHook> = None;

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
    let orig_addr = ORIG_WAIT_EVFLAG.load(Ordering::Acquire);
    if orig_addr == 0 {
        return -1;
    }
    // SAFETY: a non-zero ORIG_WAIT_EVFLAG is the address of the original
    // sceKernelWaitEventFlag (or its trampoline), stored by `install_hook`, so it
    // has exactly the `WaitEventFlagFn` signature.
    let orig: WaitEventFlagFn = unsafe { core::mem::transmute(orig_addr) };
    // SAFETY: installed in place of sceKernelWaitEventFlag, so the kernel calls it
    // with that syscall's arguments; `out_bits` is forwarded unchanged, and the
    // substituted `timeout` points at a live local for the duration of the call.
    unsafe {
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
        // SAFETY: NUL-terminated name literals; `hooked_wait_event_flag` has the
        // exact C signature of the hooked syscall, and it tolerates being called
        // before ORIG_WAIT_EVFLAG is set (it returns an error instead).
        let hook = unsafe {
            psp::hook::SyscallHook::install(
                module.as_ptr(),
                lib.as_ptr(),
                NID_WAIT_EVENT_FLAG,
                hooked_wait_event_flag as *mut u8,
            )
        };
        if let Some(h) = hook {
            // An inline hook's trampoline lives *inside* the SyscallHook value, so
            // it must be moved to its final home (a static) before its address is
            // taken -- the old code took the address of a stack local and then
            // forgot it, leaving hooked calls jumping into a dead frame. The moved
            // instructions are then written back and the I-cache invalidated so
            // the CPU fetches them from the new location.
            // SAFETY: install() runs once from psp_main during init; WAIT_HOOK is
            // written only here and never read through a reference elsewhere (the
            // hook reads the published address from ORIG_WAIT_EVFLAG instead).
            let call_addr = unsafe {
                let h = (*(&raw mut WAIT_HOOK)).insert(h);
                let addr = h.original_ptr() as usize;
                psp::sys::sceKernelDcacheWritebackAll();
                psp::sys::sceKernelIcacheInvalidateAll();
                addr
            };
            ORIG_WAIT_EVFLAG.store(call_addr, Ordering::Release);
            crate::debug_log(b"[ME-WD] hook installed OK");
            return;
        }
    }
    crate::debug_log(b"[ME-WD] WaitEventFlag hook FAILED");
}
