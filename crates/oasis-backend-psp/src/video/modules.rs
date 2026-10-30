//! Firmware modules the hardware H.264 path depends on.

use core::sync::atomic::{AtomicBool, Ordering};

use psp::sys::SceUid;

use super::vlog;

const MPEG_VSH_PATH: &[u8] = b"ms0:/PSP/GAME/OASISOS/mpeg_vsh370.prx\0";
const ME_BOOT_PATH: &[u8] = b"ms0:/PSP/GAME/OASISOS/oasis-me-boot.prx\0";

/// Pre-load AvMp3 module during init (before mpeg_vsh370.prx).
pub(super) fn load_avmp3_module() {
    // SAFETY: loads a firmware utility module by id; no pointers involved.
    let r = unsafe { psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMp3) };
    vlog(&format!("[VIDEO] AvMp3 = {r:#x}"));
}

/// Load and start a user PRX from `path` (NUL-terminated).
///
/// Returns the module id and `sceKernelStartModule`'s result, or `None`
/// if the module could not be loaded.
fn load_and_start(path: &[u8]) -> Option<(SceUid, i32)> {
    debug_assert_eq!(path.last(), Some(&0));
    // SAFETY: `path` is a NUL-terminated string; a null option block asks
    // for default load options.
    let id = unsafe { psp::sys::sceKernelLoadModule(path.as_ptr(), 0, core::ptr::null_mut()) };
    if id < SceUid(0) {
        vlog(&format!("[VIDEO] sceKernelLoadModule = {:#x}", id.0));
        return None;
    }
    let mut status: i32 = 0;
    // SAFETY: `id` is a freshly loaded module; no arguments are passed and
    // `status` is a valid out-pointer for the duration of the call.
    let ret = unsafe {
        psp::sys::sceKernelStartModule(
            id,
            0,
            core::ptr::null_mut(),
            &mut status,
            core::ptr::null_mut(),
        )
    };
    vlog(&format!(
        "[VIDEO] start {} = {ret:#x}, status={status:#x}",
        core::str::from_utf8(&path[..path.len() - 1]).unwrap_or("?")
    ));
    Some((id, ret))
}

/// Load sceMpeg implementation via mpeg_vsh370.prx (once).
///
/// AvMpegBase (the system's built-in sceMpeg) does NOT work with the NAL
/// decode path — it returns 0x80628002 even with correct mode + DDR top
/// parameters. The mode 4/5 + DDR top convention is specific to mpeg_vsh370
/// (decrypted FW 3.71 module used by PMPlayer). AvMpegBase only supports
/// the standard PSMF ringbuffer path (sceMpegGetAvcAu), not the NAL path
/// (sceMpegGetAvcNalAu).
///
/// mpeg_vsh370.prx registers "sceMpeg" via self-import when started,
/// which triggers the kernel to resolve the EBOOT's weak import stubs.
pub(super) fn load_mpeg_vsh_module() {
    static LOADED: AtomicBool = AtomicBool::new(false);
    if LOADED.swap(true, Ordering::Relaxed) {
        return;
    }

    // Boot the ME first (mpeg_vsh370 needs it running).
    if load_and_start(ME_BOOT_PATH).is_none() {
        vlog("[VIDEO] no ME boot PRX found (ME may already be booted by AAC)");
    }

    match load_and_start(MPEG_VSH_PATH) {
        None => vlog("[VIDEO] mpeg_vsh370 load FAILED — H.264 decode unavailable"),
        Some((id, ret)) if ret < 0 => {
            vlog("[VIDEO] mpeg_vsh370 start FAILED");
            // SAFETY: `id` is the module loaded above, which failed to start.
            unsafe { psp::sys::sceKernelUnloadModule(id) };
        },
        Some(_) => vlog("[VIDEO] sceMpeg stubs resolved via mpeg_vsh370"),
    }
}
