//! Stub extraction, module loading, main audio thread, and decoder backends.

use core::sync::atomic::Ordering;

use super::nids::*;
use super::network::*;
use super::radio::*;
use super::resolve::*;
use super::state::*;
use super::{copy_bytes, find_mp3_sync, log_i32, skip_id3v2, write_hex32};

use crate::overlay;

// ---------------------------------------------------------------------------
// Import stub extraction (resolve via game's resolved import stubs)
// ---------------------------------------------------------------------------

/// Try to resolve sceAudiocodec functions by finding the game's
/// resolved import stubs. The game imports sceAudiocodec, so its
/// import stubs contain the real function addresses.
///
/// Strategy:
/// 1. Scan user memory for a cluster of known sceAudiocodec NIDs
/// 2. Find the SceLibraryStubTable entry referencing the NID table
/// 3. Read the stubs and decode MIPS instructions for function addrs
pub(super) unsafe fn try_codec_stub_extraction() -> bool {
    crate::debug_log(b"[OASIS] trying codec stub extraction...");

    // Step 1: Find a known codec NID in user memory.
    let mut nid_addr: u32 = 0;
    let mut addr: u32 = 0x0880_0000;

    // SAFETY: Scanning user memory range (0x08800000-0x0A000000) for NID patterns.
    // From kernel mode we have full access to user memory. Volatile reads avoid
    // compiler optimizations that might skip changed memory.
    while addr < 0x0A00_0000 - 4 {
        let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
        if val == NID_CODEC_DECODE {
            // Verify: at least 2 other known codec NIDs within 24 bytes.
            let mut nearby = 0u32;
            let check_lo = addr.saturating_sub(24);
            let check_hi = (addr + 24).min(0x0A00_0000 - 4);
            let mut c = check_lo;
            while c <= check_hi {
                let v = unsafe { core::ptr::read_volatile(c as *const u32) };
                if v == NID_CODEC_INIT
                    || v == NID_CODEC_CHECK_NEED_MEM
                    || v == NID_CODEC_GET_EDRAM
                    || v == NID_CODEC_RELEASE_EDRAM
                {
                    nearby += 1;
                }
                c += 4;
            }
            if nearby >= 2 {
                nid_addr = addr;
                break;
            }
        }
        addr += 4;
    }

    if nid_addr == 0 {
        crate::debug_log(b"[OASIS] no codec NID cluster in user mem");
        return false;
    }
    log_hex(b"[OASIS] codec NID found @", nid_addr);

    // Step 2: Walk backwards to find the NID table start (sorted
    // ascending).
    // SAFETY: Volatile reads scanning backwards through user memory to find
    // the NID table start. Addresses are within validated user memory range.
    let mut table_start = nid_addr;
    while table_start > 0x0880_0004 {
        let prev = unsafe { core::ptr::read_volatile((table_start - 4) as *const u32) };
        let first = unsafe { core::ptr::read_volatile(table_start as *const u32) };
        if prev < first && prev > 0x0100_0000 {
            table_start -= 4;
        } else {
            break;
        }
    }

    // Walk forward to count entries.
    let mut table_end = table_start;
    // SAFETY: Volatile reads scanning forward to count NID table entries.
    let mut prev_val = 0u32;
    while table_end < nid_addr + 64 {
        let val = unsafe { core::ptr::read_volatile(table_end as *const u32) };
        if val > prev_val || table_end == table_start {
            prev_val = val;
            table_end += 4;
        } else {
            break;
        }
    }
    let entry_count = (table_end - table_start) / 4;

    log_hex(b"[OASIS] codec NID table @", table_start);
    log_i32(b"[OASIS] codec NID count=", entry_count as i32);

    if entry_count < 3 || entry_count > 32 {
        return false;
    }

    // Step 3: Scan user memory for a pointer to table_start.
    // This finds the SceLibraryStubTable's nid_table field (+0x0C).
    // SAFETY: Volatile reads scanning user memory for SceLibraryStubTable pointer.
    let mut stub_table_ptr: u32 = 0;
    addr = 0x0880_0000;
    while addr < 0x0A00_0000 - 8 {
        let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
        if val == table_start {
            // Next word should be the stub_table pointer (valid addr).
            let next = unsafe { core::ptr::read_volatile((addr + 4) as *const u32) };
            if is_valid_ptr(next) && (next & 3) == 0 {
                stub_table_ptr = next;
                log_hex(b"[OASIS] stub entry ref @", addr);
                log_hex(b"[OASIS] stub table @", stub_table_ptr);
                break;
            }
        }
        addr += 4;
    }

    if stub_table_ptr == 0 {
        crate::debug_log(b"[OASIS] stub table NOT found");
        return false;
    }

    // Step 4: Use the game's import stubs as function pointers.
    //
    // Each stub is `jr $ra; syscall N` (8 bytes). When called, the
    // CPU executes the syscall in the delay slot, which traps to the
    // kernel's syscall handler. The handler dispatches to the actual
    // codec function and returns to our caller. This works from
    // kernel mode because the PSP syscall mechanism doesn't require
    // user-mode context.
    let mut resolved = 0u32;
    // SAFETY: Volatile reads of NID table entries and MIPS instruction words
    // at validated user memory addresses. Stub addresses are within the game's
    // import stub table.
    let mut i = 0u32;
    while i < entry_count {
        let nid = unsafe { core::ptr::read_volatile((table_start + i * 4) as *const u32) };
        let stub_addr = stub_table_ptr + i * 8;
        let insn0 = unsafe { core::ptr::read_volatile(stub_addr as *const u32) };
        let insn1 = unsafe { core::ptr::read_volatile((stub_addr + 4) as *const u32) };

        // Only accept syscall stubs (jr $ra + syscall N).
        let is_syscall = insn0 == 0x03E0_0008 && (insn1 & 0x3F) == 0x0C;
        if !is_syscall {
            i += 1;
            continue;
        }

        {
            let mut buf = [0u8; 64];
            let mut p = copy_bytes(&mut buf, 0, b"[OASIS] stub ");
            p = write_hex32(&mut buf, p, nid);
            p = copy_bytes(&mut buf, p, b" @");
            p = write_hex32(&mut buf, p, stub_addr);
            crate::debug_log(&buf[..p]);
        }

        // Use the stub address as the function pointer. When called,
        // jr $ra + syscall N trampolines through the kernel.
        // SAFETY: Transmuting validated syscall stub addresses to typed fn pointers.
        // The stubs are game import stubs (jr $ra + syscall N) that trap to the
        // kernel's syscall handler, which dispatches to the actual codec function.
        unsafe {
            match nid {
                NID_CODEC_CHECK_NEED_MEM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_CHECK_NEED_MEM_FN,
                        Some(core::mem::transmute(stub_addr as usize)),
                    );
                    resolved += 1;
                },
                NID_CODEC_INIT => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_INIT_FN,
                        Some(core::mem::transmute(stub_addr as usize)),
                    );
                    resolved += 1;
                },
                NID_CODEC_DECODE => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_DECODE_FN,
                        Some(core::mem::transmute(stub_addr as usize)),
                    );
                    resolved += 1;
                },
                NID_CODEC_GET_EDRAM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_GET_EDRAM_FN,
                        Some(core::mem::transmute(stub_addr as usize)),
                    );
                    resolved += 1;
                },
                NID_CODEC_RELEASE_EDRAM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_RELEASE_EDRAM_FN,
                        Some(core::mem::transmute(stub_addr as usize)),
                    );
                    resolved += 1;
                },
                _ => {},
            }
        }
        i += 1;
    }

    log_i32(b"[OASIS] codec stubs resolved: ", resolved as i32);
    resolved >= 2
}

fn log_hex(prefix: &[u8], val: u32) {
    let mut buf = [0u8; 64];
    let mut p = copy_bytes(&mut buf, 0, prefix);
    p = write_hex32(&mut buf, p, val);
    crate::debug_log(&buf[..p]);
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

/// Load PSP AV modules via multiple strategies.
///
/// Strategy 1: sceUtilityLoadModule (proper API, handles dependencies).
/// Strategy 2: sceKernelLoadModule for flash0 PRXs (kernel-loads them
///             so they appear in the kernel module list where
///             sctrlHENFindFunction can find their exports).
pub(super) unsafe fn load_av_modules() {
    // Strategy 1: sceUtilityLoadModule (loads into user space).
    // SAFETY: Resolving sceUtilityLoadModule via sctrlHENFindFunction and
    // transmuting the raw pointer to a typed fn pointer.
    let load_fn: Option<unsafe extern "C" fn(i32) -> i32> = unsafe {
        resolve_nid(UTILITY_MODULES, NID_UTILITY_LOAD_MODULE).map(|ptr| core::mem::transmute(ptr))
    };

    if let Some(load) = load_fn {
        crate::debug_log(b"[OASIS] sceUtilityLoadModule resolved");
        // SAFETY: Calling resolved sceUtilityLoadModule with valid module IDs.
        let r1 = unsafe { load(PSP_MODULE_AV_AVCODEC) };
        log_i32(b"[OASIS] LoadModule AVCODEC=", r1);
        let r2 = unsafe { load(PSP_MODULE_AV_MPEGBASE) };
        log_i32(b"[OASIS] LoadModule MPEGBASE=", r2);
        let r3 = unsafe { load(PSP_MODULE_AV_MP3) };
        log_i32(b"[OASIS] LoadModule MP3=", r3);
    } else {
        crate::debug_log(b"[OASIS] sceUtilityLoadModule NOT found");
    }

    // Strategy 2: Also try sceKernelLoadModule from flash0.
    // When loaded from kernel context, these modules may get
    // registered in the kernel module list where sctrlHENFindFunction
    // can discover them (unlike sceUtilityLoadModule which loads
    // into user space only).
    let kprxs: &[&[u8]] = &[
        b"flash0:/kd/avcodec.prx\0",
        b"flash0:/kd/mpegbase.prx\0",
        b"flash0:/kd/libmp3.prx\0",
        b"flash0:/vsh/module/libmp3.prx\0",
    ];
    for path in kprxs {
        // SAFETY: sceKernelLoadModule/sceKernelStartModule with valid
        // null-terminated flash0 paths. Kernel-mode PRX loading.
        unsafe {
            let mod_id = psp::sys::sceKernelLoadModule(path.as_ptr(), 0, core::ptr::null_mut());
            if mod_id.0 >= 0 {
                psp::sys::sceKernelStartModule(
                    mod_id,
                    0,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
                log_i32(b"[OASIS] KernLoad OK id=", mod_id.0);
            }
            // Silently ignore failures (module already loaded, etc).
        }
    }
}

/// Try to resolve sceMp3 function pointers. Uses sctrlHENFindFunction
/// first, then falls back to manual export table walking for user-mode
/// modules.
pub(super) unsafe fn try_resolve_mp3() -> bool {
    // SAFETY: Resolving sceMp3 NIDs via combined resolution (sctrlHENFindFunction +
    // export table walking). Transmuting raw pointers to typed fn pointers.
    // Volatile writes to statics during single-threaded audio init.
    unsafe {
        if let Some(ptr) =
            resolve_nid_any(MP3_MODULES, &raw const MP3_TEXT_ADDR, NID_MP3_INIT_RESOURCE)
        {
            core::ptr::write_volatile(
                &raw mut MP3_INIT_RESOURCE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_RESERVE_HANDLE,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_RESERVE_HANDLE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_RELEASE_HANDLE,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_RELEASE_HANDLE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(MP3_MODULES, &raw const MP3_TEXT_ADDR, NID_MP3_INIT) {
            core::ptr::write_volatile(&raw mut MP3_INIT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid_any(MP3_MODULES, &raw const MP3_TEXT_ADDR, NID_MP3_DECODE) {
            core::ptr::write_volatile(&raw mut MP3_DECODE_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_CHECK_NEED_DATA,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_CHECK_NEED_DATA_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_GET_INFO_TO_ADD,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_GET_INFO_TO_ADD_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_NOTIFY_ADD_DATA,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_NOTIFY_ADD_DATA_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Check critical functions.
        core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_DECODE_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_GET_INFO_TO_ADD_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_NOTIFY_ADD_DATA_FN).is_some()
    }
}

/// Try to resolve sceAudiocodec function pointers. Uses combined
/// resolution (sctrlHENFindFunction + export table walking).
pub(super) unsafe fn try_resolve_codec() -> bool {
    // SAFETY: Resolving sceAudiocodec NIDs via combined resolution
    // (sctrlHENFindFunction + export table walking). Transmuting raw pointers
    // to typed fn pointers. Volatile writes to statics during audio init.
    unsafe {
        if let Some(ptr) = resolve_nid_any(
            CODEC_MODULES,
            &raw const CODEC_TEXT_ADDR,
            NID_CODEC_CHECK_NEED_MEM,
        ) {
            core::ptr::write_volatile(
                &raw mut CODEC_CHECK_NEED_MEM_FN,
                Some(core::mem::transmute(ptr)),
            );
        } else {
            crate::debug_log(b"[OASIS] NID miss: CheckNeedMem");
        }
        if let Some(ptr) =
            resolve_nid_any(CODEC_MODULES, &raw const CODEC_TEXT_ADDR, NID_CODEC_INIT)
        {
            core::ptr::write_volatile(&raw mut CODEC_INIT_FN, Some(core::mem::transmute(ptr)));
        } else {
            crate::debug_log(b"[OASIS] NID miss: CodecInit");
        }
        if let Some(ptr) =
            resolve_nid_any(CODEC_MODULES, &raw const CODEC_TEXT_ADDR, NID_CODEC_DECODE)
        {
            core::ptr::write_volatile(&raw mut CODEC_DECODE_FN, Some(core::mem::transmute(ptr)));
        } else {
            crate::debug_log(b"[OASIS] NID miss: CodecDecode");
        }
        if let Some(ptr) = resolve_nid_any(
            CODEC_MODULES,
            &raw const CODEC_TEXT_ADDR,
            NID_CODEC_GET_EDRAM,
        ) {
            core::ptr::write_volatile(
                &raw mut CODEC_GET_EDRAM_FN,
                Some(core::mem::transmute(ptr)),
            );
        } else {
            crate::debug_log(b"[OASIS] NID miss: GetEDRAM");
        }
        if let Some(ptr) = resolve_nid_any(
            CODEC_MODULES,
            &raw const CODEC_TEXT_ADDR,
            NID_CODEC_RELEASE_EDRAM,
        ) {
            core::ptr::write_volatile(
                &raw mut CODEC_RELEASE_EDRAM_FN,
                Some(core::mem::transmute(ptr)),
            );
        } else {
            crate::debug_log(b"[OASIS] NID miss: RelEDRAM");
        }

        core::ptr::read_volatile(&raw const CODEC_INIT_FN).is_some()
            && core::ptr::read_volatile(&raw const CODEC_DECODE_FN).is_some()
    }
}

/// Resolve all audio driver function pointers.
pub(super) unsafe fn init_audio_drivers() -> bool {
    // Step 1: Resolve sceAudio driver (always available in games).
    // SAFETY: Resolving sceAudio driver NIDs via sctrlHENFindFunction and
    // transmuting to typed fn pointers. Volatile writes to statics during
    // single-threaded audio init.
    unsafe {
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RESERVE) {
            core::ptr::write_volatile(
                &raw mut AUDIO_CH_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_OUTPUT_BLOCKING) {
            core::ptr::write_volatile(
                &raw mut AUDIO_OUTPUT_BLOCKING_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RELEASE) {
            core::ptr::write_volatile(
                &raw mut AUDIO_CH_RELEASE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_SET_CH_VOL) {
            core::ptr::write_volatile(
                &raw mut AUDIO_SET_CH_VOL_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Also resolve SRC channel functions (preferred for plugin audio).
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_CH_RESERVE) {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_OUTPUT_BLOCKING) {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_OUTPUT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_CH_RELEASE) {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_RELEASE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Prefer SRC output if both SRC reserve and output are available.
        if core::ptr::read_volatile(&raw const AUDIO_SRC_RESERVE_FN).is_some()
            && core::ptr::read_volatile(&raw const AUDIO_SRC_OUTPUT_FN).is_some()
        {
            core::ptr::write_volatile(&raw mut USE_SRC_OUTPUT, true);
            crate::debug_log(b"[OASIS] audio SRC driver resolved");
        } else if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN).is_none()
            || core::ptr::read_volatile(&raw const AUDIO_OUTPUT_BLOCKING_FN).is_none()
        {
            crate::debug_log(b"[OASIS] sceAudio driver NOT found");
            return false;
        }
        crate::debug_log(b"[OASIS] audio driver resolved");
    }

    // Step 2: Wait for the game to load AVCODEC modules during its own
    // init, then piggyback on them.  This avoids sceUtilityLoadModule
    // conflicts.  Check every 15s for up to 30s (3 attempts) before
    // falling back to loading modules ourselves.  Kept infrequent to
    // minimise stutter from the NID scan + stub extraction.
    {
        let mut attempt = 0u32;
        while attempt < 3 {
            // SAFETY: try_resolve_codec resolves codec NIDs; safe during audio init.
            if unsafe { try_resolve_codec() } {
                // SAFETY: Volatile write to DECODER_BACKEND during single-threaded init.
                unsafe {
                    core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2);
                }
                crate::debug_log(b"[OASIS] using sceAudiocodec backend");
                return true;
            }
            // SAFETY: try_codec_stub_extraction scans user memory for codec stubs.
            if unsafe { try_codec_stub_extraction() } {
                // SAFETY: Volatile write to DECODER_BACKEND during single-threaded init.
                unsafe {
                    core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2);
                }
                crate::debug_log(b"[OASIS] sceAudiocodec via stubs!");
                return true;
            }
            attempt += 1;
            if attempt < 3 {
                // SAFETY: PSP kernel syscall to sleep thread between retry attempts.
                unsafe { psp::sys::sceKernelDelayThread(15_000_000) };
            }
        }
    }

    // Step 3: Game didn't load AVCODEC -- load modules ourselves and
    // retry.  This game likely doesn't use audio codecs, so the
    // conflict risk is low.
    crate::debug_log(b"[OASIS] loading AV modules (fallback)");
    // SAFETY: load_av_modules loads PSP AV system modules from flash0.
    unsafe { load_av_modules() };

    // Retry sceAudiocodec after module load.
    // SAFETY: Retry codec resolution after loading AV modules.
    if unsafe { try_resolve_codec() } {
        // SAFETY: Volatile write to DECODER_BACKEND during single-threaded init.
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2) };
        crate::debug_log(b"[OASIS] using sceAudiocodec backend");
        return true;
    }

    // Try sceMp3 as last resort.
    // SAFETY: init_module_enum/enumerate_modules resolve kernel APIs and scan modules.
    unsafe { init_module_enum() };
    unsafe { enumerate_modules() };
    // SAFETY: try_resolve_mp3 resolves sceMp3 NIDs.
    if unsafe { try_resolve_mp3() } {
        // SAFETY: Volatile write to DECODER_BACKEND during single-threaded init.
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 1) };
        crate::debug_log(b"[OASIS] using sceMp3 backend");
        return true;
    }

    crate::debug_log(b"[OASIS] all audio resolution methods failed");

    false
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

pub(super) unsafe extern "C" fn audio_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    // Wait for the host application to load before probing.
    // OASIS OS EBOOT takes several seconds to boot (progress screens),
    // so we check multiple times with increasing delays to catch it.
    {
        let delays: [u32; 4] = [3_000_000, 3_000_000, 4_000_000, 5_000_000];
        for (i, &delay) in delays.iter().enumerate() {
            // SAFETY: PSP kernel syscall to sleep thread.
            unsafe { psp::sys::sceKernelDelayThread(delay) };
            // SAFETY: is_oasis_running scans user memory from kernel mode.
            if unsafe { is_oasis_running() } {
                crate::debug_log(b"[OASIS] OASIS_OS detected, skipping PRX audio");
                return 0;
            }
            if i < delays.len() - 1 {
                crate::debug_log(b"[OASIS] check: OASIS_OS not found, retrying...");
            }
        }
        crate::debug_log(b"[OASIS] OASIS_OS not detected after 15s, proceeding");
    }

    // SAFETY: init_audio_drivers resolves audio/codec NIDs; called from audio thread.
    if !unsafe { init_audio_drivers() } {
        crate::debug_log(b"[OASIS] audio init failed");
        return 1;
    }

    AUDIO_AVAILABLE.store(1, Ordering::Relaxed);
    // SAFETY: scan_playlist accesses PLAYLIST statics from this thread only.
    unsafe { scan_playlist() };

    // SAFETY: Volatile read of PLAYLIST_LEN; accessed only from audio thread.
    if unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) } == 0 {
        crate::debug_log(b"[OASIS] no mp3 files found");
        // Don't return -- thread stays alive for radio streaming.
    }

    // Init MP3 resource manager (sceMp3 backend only).
    // SAFETY: Volatile read of DECODER_BACKEND; set during init, read-only after.
    let backend = unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
    if backend == 1 {
        // SAFETY: Calling resolved sceMp3InitResource fn pointer.
        unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN) {
                let ret = f();
                if ret < 0 {
                    crate::debug_log(b"[OASIS] mp3 InitResource failed");
                    return 1;
                }
            }
        }
    }

    // Allocate user-memory buffers for sceAudiocodec backend.
    // Codec functions called through syscall stubs validate that pointers
    // are in user memory (0x08800000-0x0A000000), so kernel-space statics
    // get rejected.  Even when resolved via sctrlHENFindFunction, the
    // kernel functions may still do pointer validation.
    if backend == 2 {
        // SAFETY: alloc_codec_user_mem allocates from PSP user-memory partition.
        if !unsafe { alloc_codec_user_mem() } {
            crate::debug_log(b"[OASIS] codec user mem failed");
            return 1;
        }
    }

    // Reserve audio output.  Prefer the SRC (Sample Rate Conversion)
    // channel which is a dedicated output path that does NOT conflict
    // with the 8 regular PCM channels games use.
    // SAFETY: Volatile read of USE_SRC_OUTPUT; set during init, read-only after.
    let use_src = unsafe { core::ptr::read_volatile(&raw const USE_SRC_OUTPUT) };
    let channel: i32;
    if use_src {
        // SAFETY: Calling resolved sceAudioSRCChReserve fn pointer with valid params.
        let ret = unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SRC_RESERVE_FN) {
                // sceAudioSRCChReserve(sample_count, sample_rate, channels)
                // channels: 2 = stereo
                f(MP3_SAMPLES_PER_FRAME, 44100, 2)
            } else {
                -1
            }
        };
        if ret < 0 {
            crate::debug_log(b"[OASIS] SRC reserve failed");
            return 1;
        }
        channel = -1; // SRC doesn't use channel numbers
        crate::debug_log(b"[OASIS] audio SRC reserved");
    } else {
        // Fallback to regular channel (less desirable).
        // SAFETY: Calling resolved sceAudioChReserve fn pointer.
        channel = unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN) {
                let mut ch = f(7, MP3_SAMPLES_PER_FRAME, AUDIO_FORMAT_STEREO);
                if ch < 0 {
                    ch = f(6, MP3_SAMPLES_PER_FRAME, AUDIO_FORMAT_STEREO);
                }
                ch
            } else {
                return 1;
            }
        };
        if channel < 0 {
            crate::debug_log(b"[OASIS] audio ch reserve failed");
            return 1;
        }
        log_i32(b"[OASIS] audio ch=", channel);
    }

    let autoplay = crate::config::get_config().autoplay;
    if autoplay {
        AUDIO_STATE.store(1, Ordering::Relaxed);
    } else {
        AUDIO_STATE.store(0, Ordering::Relaxed);
    }
    // SAFETY: Volatile write to CURRENT_TRACK; accessed only from audio thread.
    unsafe { core::ptr::write_volatile(&raw mut CURRENT_TRACK, 0) };

    // Apply radio config from INI.
    let cfg = crate::config::get_config();
    RADIO_STATION_IDX.store(cfg.radio_station, Ordering::Relaxed);
    if cfg.radio_mode {
        // SAFETY: Volatile read of DECODER_BACKEND; set during init.
        let backend = unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
        if backend == 2 {
            overlay::show_osd(b"Connecting WiFi...");
            // SAFETY: init_network initializes PSP network stack.
            if unsafe { init_network() } {
                RADIO_ACTIVE.store(true, Ordering::Relaxed);
                AUDIO_STATE.store(1, Ordering::Relaxed);
                overlay::show_osd(b"Radio ON");
            } else {
                overlay::show_osd(b"WiFi failed");
            }
        }
    }

    // Consecutive radio stream failure counter.
    let mut radio_failures: u32 = 0;

    // Main playback loop.
    loop {
        let cmd = AUDIO_CMD.swap(0, Ordering::Relaxed);
        match cmd {
            1 => {
                let state = AUDIO_STATE.load(Ordering::Relaxed);
                if state == 1 {
                    AUDIO_STATE.store(2, Ordering::Relaxed);
                    overlay::show_osd(b"Paused");
                } else {
                    AUDIO_STATE.store(1, Ordering::Relaxed);
                    overlay::show_osd(b"Playing");
                }
            },
            2 => {
                // Next: context-sensitive (station vs track).
                if RADIO_ACTIVE.load(Ordering::Relaxed) {
                    let idx = RADIO_STATION_IDX.load(Ordering::Relaxed);
                    RADIO_STATION_IDX
                        .store((idx + 1) % RADIO_STATIONS.len() as u8, Ordering::Relaxed);
                } else {
                    // SAFETY: Volatile reads/writes of CURRENT_TRACK and PLAYLIST_LEN;
                    // accessed only from audio thread.
                    unsafe {
                        let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
                        let pl = core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                        if pl > 0 {
                            core::ptr::write_volatile(&raw mut CURRENT_TRACK, (cur + 1) % pl);
                        }
                    }
                }
            },
            3 => {
                // Prev: context-sensitive (station vs track).
                if RADIO_ACTIVE.load(Ordering::Relaxed) {
                    let idx = RADIO_STATION_IDX.load(Ordering::Relaxed);
                    let new = if idx == 0 {
                        RADIO_STATIONS.len() as u8 - 1
                    } else {
                        idx - 1
                    };
                    RADIO_STATION_IDX.store(new, Ordering::Relaxed);
                } else {
                    // SAFETY: Volatile reads/writes of CURRENT_TRACK and PLAYLIST_LEN;
                    // accessed only from audio thread.
                    unsafe {
                        let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
                        let pl = core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                        if pl > 0 {
                            core::ptr::write_volatile(
                                &raw mut CURRENT_TRACK,
                                if cur == 0 { pl - 1 } else { cur - 1 },
                            );
                        }
                    }
                }
            },
            4 => {
                // Toggle radio.
                let active = RADIO_ACTIVE.load(Ordering::Relaxed);
                if active {
                    RADIO_ACTIVE.store(false, Ordering::Relaxed);
                    overlay::show_osd(b"Radio OFF");
                } else {
                    // SAFETY: Volatile read of DECODER_BACKEND; set during init.
                    let backend = unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
                    if backend != 2 {
                        overlay::show_osd(b"Radio: no codec");
                    } else {
                        // SAFETY: Volatile read of NET_INITIALIZED; written from this thread.
                        if !unsafe { core::ptr::read_volatile(&raw const NET_INITIALIZED) } {
                            overlay::show_osd(b"Connecting WiFi...");
                            // SAFETY: init_network initializes PSP network stack.
                            if !unsafe { init_network() } {
                                overlay::show_osd(b"WiFi failed");
                            } else {
                                RADIO_ACTIVE.store(true, Ordering::Relaxed);
                                AUDIO_STATE.store(1, Ordering::Relaxed);
                                radio_failures = 0;
                                overlay::show_osd(b"Radio ON");
                            }
                        } else {
                            RADIO_ACTIVE.store(true, Ordering::Relaxed);
                            AUDIO_STATE.store(1, Ordering::Relaxed);
                            radio_failures = 0;
                            overlay::show_osd(b"Radio ON");
                        }
                    }
                }
            },
            5 => {
                // Next station (explicit).
                let idx = RADIO_STATION_IDX.load(Ordering::Relaxed);
                RADIO_STATION_IDX
                    .store((idx + 1) % RADIO_STATIONS.len() as u8, Ordering::Relaxed);
            },
            6 => {
                // Prev station (explicit).
                let idx = RADIO_STATION_IDX.load(Ordering::Relaxed);
                let new = if idx == 0 {
                    RADIO_STATIONS.len() as u8 - 1
                } else {
                    idx - 1
                };
                RADIO_STATION_IDX.store(new, Ordering::Relaxed);
            },
            7 => {
                // Video MP3 interrupt -- just break out of current decode.
                // No playlist advancement needed.
            },
            _ => {},
        }

        let state = AUDIO_STATE.load(Ordering::Relaxed);
        if state == 0 || state == 2 {
            // SAFETY: PSP kernel syscall to sleep thread.
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Radio streaming branch.
        if RADIO_ACTIVE.load(Ordering::Relaxed) {
            let idx = RADIO_STATION_IDX.load(Ordering::Relaxed);
            // SAFETY: play_radio_stream uses resolved network/codec fn pointers.
            let result = unsafe { play_radio_stream(idx, channel) };
            if result < 0 {
                radio_failures += 1;
                crate::debug_log(b"[OASIS] radio stream error");
                if radio_failures >= 3 {
                    RADIO_ACTIVE.store(false, Ordering::Relaxed);
                    overlay::show_osd(b"Radio: failed");
                    radio_failures = 0;
                } else {
                    // SAFETY: PSP kernel syscall to sleep thread before retry.
                    unsafe { psp::sys::sceKernelDelayThread(2_000_000) };
                }
            } else {
                radio_failures = 0;
            }
            continue;
        }

        // PIP video companion MP3 branch.
        // When active, play the video's audio track on loop instead of
        // the normal playlist. Does not touch CURRENT_TRACK.
        if VIDEO_MP3_ACTIVE.load(Ordering::Acquire) {
            // SAFETY: VIDEO_MP3_PATH written before VIDEO_MP3_ACTIVE (Release ordering);
            // Acquire load above ensures we see the completed write.
            let vpath = unsafe { &*(&raw const VIDEO_MP3_PATH) };
            // SAFETY: set_track_name writes to TRACK_NAME; called from audio thread.
            unsafe { set_track_name(vpath) };
            // SAFETY: Volatile read of DECODER_BACKEND; set during init.
            let backend = unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
            // SAFETY: play_track_mp3/play_track_codec use resolved codec fn pointers.
            let result = match backend {
                1 => unsafe { play_track_mp3(vpath, channel) },
                2 => unsafe { play_track_codec(vpath, channel) },
                _ => -1,
            };
            if result < 0 {
                crate::debug_log(b"[OASIS] video mp3 error");
                VIDEO_MP3_ACTIVE.store(false, Ordering::Release);
                // Brief delay before resuming normal playback.
                // SAFETY: PSP kernel syscall to sleep thread.
                unsafe { psp::sys::sceKernelDelayThread(50_000) };
            }
            // Loop back -- if VIDEO_MP3_ACTIVE is still true, replay.
            // If false, fall through to normal playlist next iteration.
            continue;
        }

        // File playback (only when we have tracks).
        // SAFETY: Volatile read of PLAYLIST_LEN; accessed only from audio thread.
        let pl_len = unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) };
        if pl_len == 0 {
            // SAFETY: PSP kernel syscall to sleep thread.
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // SAFETY: Volatile reads of CURRENT_TRACK and PLAYLIST; accessed only
        // from audio thread. track_idx bounded by PLAYLIST_LEN.
        let track_idx = unsafe { core::ptr::read_volatile(&raw const CURRENT_TRACK) };
        let track_path = unsafe { &(*(&raw const PLAYLIST))[track_idx] };
        // SAFETY: set_track_name writes to TRACK_NAME; called from audio thread.
        unsafe { set_track_name(track_path) };

        // SAFETY: Volatile read of DECODER_BACKEND; set during init.
        let backend = unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
        // SAFETY: play_track_mp3/play_track_codec use resolved codec fn pointers.
        let result = match backend {
            1 => unsafe { play_track_mp3(track_path, channel) },
            2 => unsafe { play_track_codec(track_path, channel) },
            _ => -1,
        };
        if result < 0 {
            crate::debug_log(b"[OASIS] track playback error");
        }

        // Advance to next track only on natural completion.  Skip when
        // interrupted by any command (2=next, 3=prev, 7=video MP3) -- those
        // are handled explicitly at the top of the main loop.
        let pending = AUDIO_CMD.load(Ordering::Relaxed);
        if pending != 2 && pending != 3 && pending != 7 {
            // SAFETY: Volatile read/write of CURRENT_TRACK; accessed only from audio thread.
            unsafe {
                let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
                core::ptr::write_volatile(&raw mut CURRENT_TRACK, (cur + 1) % pl_len);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Backend 1: sceMp3 streaming
// ---------------------------------------------------------------------------

pub(super) unsafe fn play_track_mp3(path: &[u8], channel: i32) -> i32 {
    // SAFETY: sceIoOpen with valid null-terminated path and read-only flag.
    let fd = unsafe { psp::sys::sceIoOpen(path.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
    if fd < psp::sys::SceUid(0) {
        return -1;
    }

    // SAFETY: sceIoLseek with valid fd to determine file size, then reset to start.
    let file_size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) } as i32;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
    if file_size <= 0 {
        // SAFETY: sceIoClose with valid fd.
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    static mut S_MP3_BUF: [u8; MP3_BUF_SIZE] = [0u8; MP3_BUF_SIZE];
    static mut S_PCM_BUF: [u8; PCM_BUF_SIZE] = [0u8; PCM_BUF_SIZE];

    // SAFETY: Accessing function-local statics; play_track_mp3 is only called
    // from the single audio thread, so no concurrent access.
    let mp3_buf_ptr = unsafe { (*(&raw mut S_MP3_BUF)).as_mut_ptr() };
    let pcm_buf_ptr = unsafe { (*(&raw mut S_PCM_BUF)).as_mut_ptr() };

    let init = Mp3InitStruct {
        mp3_stream_start: 0,
        _unk1: 0,
        mp3_stream_end: file_size,
        _unk2: 0,
        mp3_buf: mp3_buf_ptr,
        mp3_buf_size: MP3_BUF_SIZE as i32,
        pcm_buf: pcm_buf_ptr,
        pcm_buf_size: PCM_BUF_SIZE as i32,
    };

    // SAFETY: Volatile read of resolved sceMp3ReserveMpegHandle fn pointer;
    // calling it with valid init struct. sceIoClose on error path.
    let handle = unsafe {
        match core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN) {
            Some(f) => f(&init),
            None => {
                psp::sys::sceIoClose(fd);
                return -1;
            },
        }
    };
    if handle < 0 {
        // SAFETY: sceIoClose with valid fd on error path.
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // SAFETY: fill_stream_data reads file data into MP3 stream buffer.
    unsafe { fill_stream_data(handle, fd) };

    // SAFETY: Volatile read of resolved sceMp3Init fn pointer; calling with valid handle.
    let ret = unsafe {
        match core::ptr::read_volatile(&raw const MP3_INIT_FN) {
            Some(f) => f(handle),
            None => -1,
        }
    };
    if ret < 0 {
        // SAFETY: Releasing MP3 handle and closing fd on error path.
        unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MP3_RELEASE_HANDLE_FN) {
                f(handle);
            }
            psp::sys::sceIoClose(fd);
        }
        return -1;
    }

    let mut result = 0i32;
    loop {
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 || cmd == 7 {
            break;
        }
        if cmd == 1 {
            AUDIO_CMD.store(0, Ordering::Relaxed);
            let state = AUDIO_STATE.load(Ordering::Relaxed);
            if state == 1 {
                AUDIO_STATE.store(2, Ordering::Relaxed);
                overlay::show_osd(b"Paused");
            } else {
                AUDIO_STATE.store(1, Ordering::Relaxed);
                overlay::show_osd(b"Playing");
            }
        }
        if AUDIO_STATE.load(Ordering::Relaxed) != 1 {
            // SAFETY: PSP kernel syscall to sleep thread while paused.
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // SAFETY: Volatile read of resolved sceMp3CheckStreamDataNeeded fn pointer.
        let needs_data = unsafe {
            match core::ptr::read_volatile(&raw const MP3_CHECK_NEED_DATA_FN) {
                Some(f) => f(handle),
                None => 0,
            }
        };
        if needs_data > 0 {
            // SAFETY: fill_stream_data reads file data into MP3 stream buffer.
            let filled = unsafe { fill_stream_data(handle, fd) };
            if filled <= 0 {
                break;
            }
        }

        let mut pcm_out: *const i16 = core::ptr::null();
        // SAFETY: Volatile read of resolved sceMp3Decode fn pointer;
        // calling with valid handle, pcm_out receives decoded PCM pointer.
        let decoded = unsafe {
            match core::ptr::read_volatile(&raw const MP3_DECODE_FN) {
                Some(f) => f(handle, &mut pcm_out),
                None => break,
            }
        };
        if decoded <= 0 {
            break;
        }

        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000) / 255;
        // SAFETY: Volatile read of USE_SRC_OUTPUT flag; set during init.
        let use_src = unsafe { core::ptr::read_volatile(&raw const USE_SRC_OUTPUT) };
        // SAFETY: Volatile reads of resolved audio output fn pointers;
        // calling with valid channel, volume, and PCM buffer from decoder.
        unsafe {
            if use_src {
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SRC_OUTPUT_FN) {
                    let ret = f(vol, pcm_out as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            } else {
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN) {
                    f(channel, vol, vol);
                }
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_OUTPUT_BLOCKING_FN) {
                    let ret = f(channel, vol, pcm_out as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            }
        }
    }

    // SAFETY: Releasing MP3 handle and closing file descriptor on cleanup.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MP3_RELEASE_HANDLE_FN) {
            f(handle);
        }
        psp::sys::sceIoClose(fd);
    }
    result
}

unsafe fn fill_stream_data(handle: i32, fd: psp::sys::SceUid) -> i32 {
    let mut dst_ptr: *mut u8 = core::ptr::null_mut();
    let mut to_write: i32 = 0;
    let mut src_pos: i32 = 0;

    // SAFETY: Volatile reads of resolved sceMp3 fn pointers (GetInfoToAddStreamData,
    // NotifyAddStreamData); calling with valid handle. sceIoLseek/sceIoRead with valid fd.
    unsafe {
        let get_info = match core::ptr::read_volatile(&raw const MP3_GET_INFO_TO_ADD_FN) {
            Some(f) => f,
            None => return -1,
        };
        let notify = match core::ptr::read_volatile(&raw const MP3_NOTIFY_ADD_DATA_FN) {
            Some(f) => f,
            None => return -1,
        };

        let ret = get_info(handle, &mut dst_ptr, &mut to_write, &mut src_pos);
        if ret < 0 || to_write <= 0 {
            return 0;
        }

        psp::sys::sceIoLseek(fd, src_pos as i64, psp::sys::IoWhence::Set);
        let read = psp::sys::sceIoRead(fd, dst_ptr as *mut _, to_write as u32);
        if read <= 0 {
            return 0;
        }
        notify(handle, read);
        read
    }
}

// ---------------------------------------------------------------------------
// Backend 2: sceAudiocodec frame-by-frame
// ---------------------------------------------------------------------------

pub(super) unsafe fn play_track_codec(path: &[u8], channel: i32) -> i32 {
    // SAFETY: sceIoOpen with valid null-terminated path and read-only flag.
    let fd = unsafe { psp::sys::sceIoOpen(path.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
    if fd < psp::sys::SceUid(0) {
        return -1;
    }

    // SAFETY: sceIoLseek with valid fd to determine file size, then reset to start.
    let file_size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) } as usize;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
    if file_size == 0 {
        // SAFETY: sceIoClose with valid fd.
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // Use user-memory buffers (allocated in audio_thread_entry).
    // SAFETY: Volatile reads of UMEM_* pointers; set during init, read-only after.
    let codec = unsafe { core::ptr::read_volatile(&raw const UMEM_CODEC) };
    let pcm_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_PCM) };
    let read_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_READ) };
    if codec.is_null() || pcm_buf.is_null() || read_buf.is_null() {
        crate::debug_log(b"[OASIS] codec bufs null");
        // SAFETY: sceIoClose with valid fd.
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }
    // SAFETY: Zeroing codec buffer via pointer arithmetic; codec points to
    // CODEC_BUF_WORDS words of allocated user memory.
    unsafe {
        let mut i = 0;
        while i < CODEC_BUF_WORDS {
            *codec.add(i) = 0;
            i += 1;
        }
    }

    #[allow(unused_assignments)]
    let mut edram_allocated = false;
    // SAFETY: Volatile reads of resolved sceAudiocodec fn pointers
    // (CheckNeedMem, GetEDRAM, Init); calling with valid codec buffer.
    // sceIoClose on error paths.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN) {
            f(codec, CODEC_TYPE_MP3);
        }
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_GET_EDRAM_FN) {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret >= 0 {
                edram_allocated = true;
            } else {
                psp::sys::sceIoClose(fd);
                return -1;
            }
        } else {
            psp::sys::sceIoClose(fd);
            return -1;
        }
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_INIT_FN) {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret < 0 {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                        rel(codec);
                    }
                }
                psp::sys::sceIoClose(fd);
                return -1;
            }
        } else {
            crate::debug_log(b"[OASIS] no CodecInit fn");
            if edram_allocated {
                if let Some(rel) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                    rel(codec);
                }
            }
            psp::sys::sceIoClose(fd);
            return -1;
        }
    }

    let mut file_pos: usize;
    let mut buf_valid: usize;
    let mut buf_pos: usize = 0;

    // SAFETY: sceIoRead with valid fd into allocated read_buf.
    let initial_read = unsafe { psp::sys::sceIoRead(fd, read_buf as *mut _, READ_BUF_SIZE as u32) };
    if initial_read <= 0 {
        // SAFETY: Releasing EDRAM and closing fd on error path.
        unsafe {
            if edram_allocated {
                if let Some(f) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                    f(codec);
                }
            }
            psp::sys::sceIoClose(fd);
        }
        return -1;
    }
    buf_valid = initial_read as usize;
    file_pos = buf_valid;

    // Skip ID3v2 tag.
    // SAFETY: read_buf points to buf_valid bytes of data read from the file.
    let slice = unsafe { core::slice::from_raw_parts(read_buf, buf_valid) };
    let id3_skip = skip_id3v2(slice);
    if id3_skip > 0 && id3_skip < buf_valid {
        buf_pos = id3_skip;
    }

    // SAFETY: Volatile read of resolved sceAudiocodecDecode fn pointer;
    // releasing EDRAM and closing fd on error path.
    let decode_fn = unsafe {
        match core::ptr::read_volatile(&raw const CODEC_DECODE_FN) {
            Some(f) => f,
            None => {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                        rel(codec);
                    }
                }
                psp::sys::sceIoClose(fd);
                return -1;
            },
        }
    };

    let mut result = 0i32;
    let mut frame_count: u32 = 0;
    let mut zero_consumed: u32 = 0;

    loop {
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 || cmd == 7 {
            break;
        }
        if cmd == 1 {
            AUDIO_CMD.store(0, Ordering::Relaxed);
            let state = AUDIO_STATE.load(Ordering::Relaxed);
            if state == 1 {
                AUDIO_STATE.store(2, Ordering::Relaxed);
                overlay::show_osd(b"Paused");
            } else {
                AUDIO_STATE.store(1, Ordering::Relaxed);
                overlay::show_osd(b"Playing");
            }
        }
        if AUDIO_STATE.load(Ordering::Relaxed) != 1 {
            // SAFETY: PSP kernel syscall to sleep thread while paused.
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Compact + stream-refill: when half the buffer is consumed,
        // shift remaining data to the front and top up in small chunks
        // (4KB) to avoid a single large blocking read that stalls audio.
        if buf_pos > READ_BUF_SIZE / 2 && file_pos < file_size {
            let remaining = buf_valid - buf_pos;
            if remaining > 0 {
                // SAFETY: Manual byte copy within allocated read_buf;
                // source [buf_pos..buf_pos+remaining] and dest [0..remaining]
                // are within bounds as buf_pos < buf_valid <= READ_BUF_SIZE.
                unsafe {
                    let mut i = 0;
                    while i < remaining {
                        *read_buf.add(i) = *read_buf.add(buf_pos + i);
                        i += 1;
                    }
                }
            }
            buf_valid = remaining;
            buf_pos = 0;
        }
        // Top up buffer if there's room (small reads to avoid stalls).
        if buf_valid < READ_BUF_SIZE && file_pos < file_size {
            let room = READ_BUF_SIZE - buf_valid;
            let chunk = if room > 4096 { 4096 } else { room };
            // SAFETY: sceIoRead with valid fd into read_buf at offset buf_valid;
            // buf_valid + chunk <= READ_BUF_SIZE.
            let read =
                unsafe { psp::sys::sceIoRead(fd, read_buf.add(buf_valid) as *mut _, chunk as u32) };
            if read > 0 {
                buf_valid += read as usize;
                file_pos += read as usize;
            }
        }

        if buf_valid - buf_pos < 4 {
            break;
        }

        // SAFETY: read_buf points to buf_valid bytes of valid data.
        let slice = unsafe { core::slice::from_raw_parts(read_buf, buf_valid) };
        let sync_pos = match find_mp3_sync(slice, buf_pos) {
            Some(pos) => pos,
            None => break,
        };
        buf_pos = sync_pos;
        if buf_valid - buf_pos < 8 {
            break;
        }

        let avail = buf_valid - buf_pos;
        // SAFETY: Setting codec buffer fields via pointer arithmetic;
        // codec points to CODEC_BUF_WORDS words of allocated user memory.
        // Indices 6-10 are within the sceAudiocodec context structure.
        unsafe {
            *codec.add(6) = read_buf.add(buf_pos) as u32;
            *codec.add(7) = avail as u32;
            *codec.add(8) = pcm_buf as u32;
            *codec.add(9) = (1152 * 4) as u32;
            *codec.add(10) = avail as u32;
        }

        // SAFETY: Calling resolved sceAudiocodecDecode with valid codec buffer.
        let ret = unsafe { decode_fn(codec, CODEC_TYPE_MP3) };
        if ret < 0 {
            // Limit consecutive failures to avoid infinite spin.
            frame_count += 1;
            if frame_count > 100 {
                crate::debug_log(b"[OASIS] too many decode errors");
                break;
            }
            buf_pos += 1;
            continue;
        }

        // SAFETY: Reading consumed byte count from codec buffer field 7.
        let consumed = unsafe { *codec.add(7) } as usize;
        if consumed == 0 {
            zero_consumed += 1;
            if zero_consumed > 100 {
                crate::debug_log(b"[OASIS] too many zero-consumed decodes");
                break;
            }
            buf_pos += 1;
            continue;
        }
        zero_consumed = 0;
        buf_pos += consumed;

        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000) / 255;
        // SAFETY: Volatile read of USE_SRC_OUTPUT flag; set during init.
        let use_src = unsafe { core::ptr::read_volatile(&raw const USE_SRC_OUTPUT) };
        // SAFETY: Volatile reads of resolved audio output fn pointers;
        // calling with valid channel, volume, and decoded PCM buffer.
        unsafe {
            if use_src {
                // SRC output: sceAudioSRCOutputBlocking(volume, buffer)
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SRC_OUTPUT_FN) {
                    let ret = f(vol, pcm_buf as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            } else {
                // Regular channel output.
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN) {
                    f(channel, vol, vol);
                }
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_OUTPUT_BLOCKING_FN) {
                    let ret = f(channel, vol, pcm_buf as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            }
        }
        frame_count += 1;
    }

    // SAFETY: Releasing EDRAM allocation and closing file descriptor on cleanup.
    unsafe {
        if edram_allocated {
            if let Some(f) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                f(codec);
            }
        }
        psp::sys::sceIoClose(fd);
    }
    result
}
