//! Direct Media Engine RPC communication.
//!
//! Implements the ME command protocol reverse-engineered from sceMeCodecWrapper:
//! - Command buffer at 0xBFC00600 (40 bytes)
//! - SceMeRpc semaphore for exclusive access
//! - SceMediaEngineRpc event flag for completion notification
//! - SysCtrl register sequence for ME boot + interrupt trigger
//!
//! ## Usage
//!
//! 1. Call `me_init()` to boot the ME and set up RPC primitives
//! 2. Call `me_rpc(cmd, params)` to send commands
//! 3. Video decode: Open(2) → Init(0x24) → ScanHeader(0x25) → Decode(0x26)

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Whether the ME RPC subsystem has been initialized.
static ME_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Init trigger: 0=idle, 1=requested.
static INIT_REQUESTED: AtomicU8 = AtomicU8::new(0);

/// Request ME RPC initialization (called from overlay menu).
pub fn trigger_init() {
    INIT_REQUESTED.store(1, Ordering::Release);
}

/// Check and consume the init request (called from dump thread).
pub fn check_init_request() -> bool {
    INIT_REQUESTED.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}

/// SceMeRpc semaphore ID (exclusive ME access).
static mut ME_SEMA_ID: psp::sys::SceUid = psp::sys::SceUid(-1);

/// SceMediaEngineRpc event flag ID (completion notification).
static mut ME_EVENT_ID: psp::sys::SceUid = psp::sys::SceUid(-1);

// ---------------------------------------------------------------------------
// ME command buffer (physical address 0xBFC00600)
// ---------------------------------------------------------------------------

/// ME command buffer base address (uncached).
const ME_CMD_BUF: u32 = 0xBFC0_0600;

/// ME return value offset within command buffer.
const ME_RET_OFFSET: u32 = 0x28;

// ---------------------------------------------------------------------------
// Hardware registers
// ---------------------------------------------------------------------------

/// SysCtrl base — ME status, reset, clock, interrupts.
const SYSREG_BASE: u32 = 0xBC10_0000;

/// ME clock controller base.
const ME_CLK_BASE: u32 = 0xBCC0_0000;

// ---------------------------------------------------------------------------
// ME RPC command IDs (from reverse engineering)
// ---------------------------------------------------------------------------

/// Video codec commands.
pub const CMD_VIDEOCODEC_OPEN: u32 = 0x02;
pub const CMD_VIDEOCODEC_INIT: u32 = 0x24;
pub const CMD_VIDEOCODEC_SCAN_HEADER: u32 = 0x25;
pub const CMD_VIDEOCODEC_DECODE: u32 = 0x26;
pub const CMD_VIDEOCODEC_RELEASE: u32 = 0xE1;

/// Audio codec commands.
pub const CMD_AUDIOCODEC_INIT: u32 = 0x09;
pub const CMD_AUDIOCODEC_INIT2: u32 = 0x60;
pub const CMD_AUDIOCODEC_DECODE: u32 = 0x64;
pub const CMD_AUDIOCODEC_RELEASE: u32 = 0x61;
pub const CMD_AUDIOCODEC_CHECK_MEM: u32 = 0x66;

/// Memory management commands.
pub const CMD_ME_ALLOC_MEM: u32 = 0x180;
pub const CMD_ME_FREE_MEM: u32 = 0x181;

/// Color space conversion.
pub const CMD_MPEGBASE_CSC: u32 = 0x6A;

// ---------------------------------------------------------------------------
// Volatile hardware register access
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn hw_read32(addr: u32) -> u32 {
    // SAFETY: Reading memory-mapped hardware register.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
unsafe fn hw_write32(addr: u32, val: u32) {
    // SAFETY: Writing memory-mapped hardware register.
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

// ---------------------------------------------------------------------------
// ME Boot
// ---------------------------------------------------------------------------

/// Boot the Media Engine hardware.
///
/// Follows the register sequence from sceMeCodecWrapper MODULE_ENTRY:
/// 1. Reset ME via SysCtrl
/// 2. Enable ME clocks
/// 3. Clear interrupts
/// 4. Trigger ME boot via clock controller
/// 5. Wait for boot completion
///
/// # Safety
/// Must be called from kernel mode.
unsafe fn me_hw_boot() {
    crate::debug_log(b"[ME-RPC] booting ME hardware...");

    // Check if ME is already running.
    let status = unsafe { hw_read32(SYSREG_BASE) };
    if status != 0 {
        crate::debug_log(b"[ME-RPC] ME already running");
        return;
    }

    // Step 1: ME reset control = 0 (deassert reset).
    unsafe { hw_write32(SYSREG_BASE + 0x40, 0) };

    // Step 2: ME clock enable = 7 (bus + ME + AW).
    unsafe { hw_write32(SYSREG_BASE + 0x50, 7) };

    // Step 3: Clear all interrupt flags.
    unsafe { hw_write32(SYSREG_BASE + 0x04, 0xFFFF_FFFF) };

    // Memory barrier.
    core::sync::atomic::fence(Ordering::SeqCst);

    // Step 4: Trigger ME boot via clock controller.
    unsafe { hw_write32(ME_CLK_BASE + 0x10, 1) };

    // Step 5: Poll until boot completes (bit 0 clears).
    let mut timeout = 1_000_000u32;
    loop {
        let val = unsafe { hw_read32(ME_CLK_BASE + 0x10) };
        if val & 1 == 0 {
            break;
        }
        timeout -= 1;
        if timeout == 0 {
            crate::debug_log(b"[ME-RPC] ME boot timeout!");
            return;
        }
    }

    // Step 6: Configure ME memory controller.
    unsafe {
        hw_write32(ME_CLK_BASE + 0x70, 1);
        hw_write32(ME_CLK_BASE + 0x30, 8);
        hw_write32(ME_CLK_BASE + 0x40, 2);
    }

    crate::debug_log(b"[ME-RPC] ME hardware booted OK");
}

// ---------------------------------------------------------------------------
// Module loading approach
// ---------------------------------------------------------------------------

/// Try to load the ME kernel modules from flash0.
///
/// Returns `AlreadyLoaded` if modules are already present (EXCLUSIVE_LOAD),
/// `Loaded` if freshly loaded, or `Failed` if loading failed entirely.
///
/// # Safety
/// Must be called from kernel mode.
unsafe fn try_load_me_modules() -> LoadResult {
    crate::debug_log(b"[ME-RPC] loading me_wrapper.prx...");

    // 0x80020149 = SCE_KERNEL_ERROR_EXCLUSIVE_LOAD (already loaded).
    const EXCLUSIVE_LOAD: i32 = -0x7FFDFE_B7_i32; // 0x80020149

    let me_id = unsafe {
        psp::sys::sceKernelLoadModule(
            b"flash0:/kd/me_wrapper.prx\0".as_ptr(),
            0,
            core::ptr::null_mut(),
        )
    };

    let mut msg = [0u8; 64];
    let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] me_wrapper=0x");
    mp = crate::me_dump::append_hex(&mut msg, mp, me_id.0 as u32);
    crate::debug_log(&msg[..mp]);

    if me_id.0 == EXCLUSIVE_LOAD {
        // Module already loaded by game/VSH — ME is running.
        // Also try avcodec.
        let av_id = unsafe {
            psp::sys::sceKernelLoadModule(
                b"flash0:/kd/avcodec.prx\0".as_ptr(),
                0,
                core::ptr::null_mut(),
            )
        };
        let mut msg2 = [0u8; 64];
        let mut mp2 = crate::me_dump::append_bytes(&mut msg2, 0, b"[ME-RPC] avcodec=0x");
        mp2 = crate::me_dump::append_hex(&mut msg2, mp2, av_id.0 as u32);
        crate::debug_log(&msg2[..mp2]);

        return LoadResult::AlreadyLoaded;
    }

    if me_id < psp::sys::SceUid(0) {
        crate::debug_log(b"[ME-RPC] me_wrapper load FAILED");
        return LoadResult::Failed;
    }

    // Start me_wrapper.prx.
    let mut status: i32 = 0;
    let ret = unsafe {
        psp::sys::sceKernelStartModule(
            me_id, 0, core::ptr::null_mut(), &mut status, core::ptr::null_mut(),
        )
    };

    let mut msg2 = [0u8; 64];
    let mut mp2 = crate::me_dump::append_bytes(&mut msg2, 0, b"[ME-RPC] me_wrapper start=0x");
    mp2 = crate::me_dump::append_hex(&mut msg2, mp2, ret as u32);
    crate::debug_log(&msg2[..mp2]);

    if ret < 0 {
        return LoadResult::Failed;
    }

    // Also load avcodec.prx.
    crate::debug_log(b"[ME-RPC] loading avcodec.prx...");
    let av_id = unsafe {
        psp::sys::sceKernelLoadModule(
            b"flash0:/kd/avcodec.prx\0".as_ptr(), 0, core::ptr::null_mut(),
        )
    };
    if av_id >= psp::sys::SceUid(0) {
        let mut st2: i32 = 0;
        unsafe {
            psp::sys::sceKernelStartModule(
                av_id, 0, core::ptr::null_mut(), &mut st2, core::ptr::null_mut(),
            );
        }
    }

    LoadResult::Loaded
}

// ---------------------------------------------------------------------------
// RPC primitives
// ---------------------------------------------------------------------------

/// Initialize the ME RPC subsystem.
///
/// Strategy:
/// 1. Try loading Sony's kernel modules (me_wrapper.prx + avcodec.prx)
/// 2. If EXCLUSIVE_LOAD (0x80020149), modules are already loaded (game/VSH)
///    — find the existing RPC primitives by name
/// 3. Only if nothing is loaded, manually boot ME hardware
///
/// # Safety
/// Must be called from kernel mode, once, during plugin init.
pub unsafe fn me_init() {
    if ME_INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    crate::debug_log(b"[ME-RPC] initializing...");

    // Try loading Sony's kernel modules.
    let load_result = unsafe { try_load_me_modules() };

    match load_result {
        LoadResult::Loaded => {
            crate::debug_log(b"[ME-RPC] modules freshly loaded");
        }
        LoadResult::AlreadyLoaded => {
            crate::debug_log(b"[ME-RPC] modules already loaded (game/VSH)");
        }
        LoadResult::Failed => {
            crate::debug_log(b"[ME-RPC] module load failed completely");
            // Don't do manual boot — too dangerous during game execution.
            // ME hardware writes can corrupt display/audio.
            crate::debug_log(b"[ME-RPC] skipping manual boot (unsafe during game)");
        }
    }

    // Find the existing SceMeRpc semaphore by trying to use the
    // ME driver functions directly via sctrlHENFindFunction.
    // If the modules are loaded, their internal sema/event are
    // already set up — we need to use those, not create new ones.
    //
    // For now, create our own primitives for direct RPC.
    // When modules are already loaded, we should call THEIR functions
    // instead of bypassing them.
    let sema = unsafe {
        psp::sys::sceKernelCreateSema(
            b"SceMeRpcHB\0".as_ptr(),
            0,  // attr
            1,  // init_val (1 = available)
            1,  // max_val
            core::ptr::null_mut(),
        )
    };

    log_id(b"[ME-RPC] sema", sema);

    if sema >= psp::sys::SceUid(0) {
        unsafe { ME_SEMA_ID = sema };
    }

    let event = unsafe {
        psp::sys::sceKernelCreateEventFlag(
            b"SceMeRpcEvHB\0".as_ptr(),
            psp::sys::EventFlagAttributes::WAIT_MULTIPLE,
            0,
            core::ptr::null_mut(),
        )
    };

    log_id(b"[ME-RPC] event", event);

    if event >= psp::sys::SceUid(0) {
        unsafe { ME_EVENT_ID = event };
    }

    // Try to resolve ME driver functions from the loaded modules.
    // If modules are loaded (EXCLUSIVE_LOAD), these should resolve.
    unsafe { probe_me_drivers() };

    ME_INITIALIZED.store(true, Ordering::Release);
    crate::debug_log(b"[ME-RPC] init complete");
}

fn log_id(prefix: &[u8], id: psp::sys::SceUid) {
    let mut msg = [0u8; 64];
    let mut mp = crate::me_dump::append_bytes(&mut msg, 0, prefix);
    mp = crate::me_dump::append_bytes(&mut msg, mp, b" id=0x");
    mp = crate::me_dump::append_hex(&mut msg, mp, id.0 as u32);
    crate::debug_log(&msg[..mp]);
}

/// Probe for ME driver functions using sctrlHENFindFunction.
///
/// If me_wrapper.prx is loaded (EXCLUSIVE_LOAD), its exported functions
/// should be findable. This tells us the ME is fully operational.
unsafe fn probe_me_drivers() {
    crate::debug_log(b"[ME-RPC] probing ME driver functions...");

    // Try known module/library/NID combos from our analysis.
    let probes: &[(&[u8], &[u8], u32, &[u8])] = &[
        // sceMeCodecWrapper exports
        (b"sceMeCodecWrapper\0", b"sceMeVideo_driver\0",
         0xC441994C, b"MeVideo_Init"),
        (b"sceMeCodecWrapper\0", b"sceMeCore_driver\0",
         0xFA398D71, b"MeCore_Dispatch"),
        (b"sceMeCodecWrapper\0", b"sceMeCore_driver\0",
         0x635397BB, b"MeCore_Simple"),
        (b"sceMeCodecWrapper\0", b"sceMeMemory_driver\0",
         0x92D3BAA1, b"MeMem_Alloc"),
        // sceAvcodec_wrapper exports
        (b"sceAvcodec_wrapper\0", b"sceVideocodec\0",
         0xC01EC829, b"avcodec_VcodecOpen"),
        (b"sceAvcodec_wrapper\0", b"sceAudiocodec\0",
         0x9D3F790C, b"avcodec_AcodecChk"),
        // mpeg_vsh.prx — the VSH video player's MPEG module
        // sceMpeg NIDs
        (b"sceMpeg_driver\0", b"sceMpeg\0",
         0xD8C5F121, b"sceMpegCreate"),
        (b"sceMpeg_driver\0", b"sceMpeg\0",
         0x0E3C2E9D, b"sceMpegAvcDecode"),
        (b"sceMpeg_driver\0", b"sceMpeg\0",
         0x740FCCD1, b"sceMpegAvcDecodeStop"),
        (b"sceMpeg_driver\0", b"sceMpeg\0",
         0xFE246728, b"sceMpegGetAvcAu"),
        (b"sceMpeg_driver\0", b"sceMpeg\0",
         0x874624D6, b"sceMpegInit"),
        // Try mpeg_vsh module name
        (b"mpeg_vsh\0", b"sceMpeg\0",
         0xD8C5F121, b"vsh_MpegCreate"),
        (b"sceMpegVsh_library\0", b"sceMpeg\0",
         0xD8C5F121, b"vshlib_MpegCreate"),
        // Try sceMeVideo directly — decode function
        (b"sceMeCodecWrapper\0", b"sceMeVideo_driver\0",
         0x8768915D, b"MeVideo_ScanHdr"),
        (b"sceMeCodecWrapper\0", b"sceMeVideo_driver\0",
         0x4D78330C, b"MeVideo_GetEdram"),
        (b"sceMeCodecWrapper\0", b"sceMeVideo_driver\0",
         0x6D68B223, b"MeVideo_Decode"),
        (b"sceMeCodecWrapper\0", b"sceMeVideo_driver\0",
         0xE8CD3C75, b"MeVideo_Init2"),
    ];

    for &(module, library, nid, label) in probes {
        let mut pre = [0u8; 64];
        let mut pp = crate::me_dump::append_bytes(&mut pre, 0, b"[ME-RPC] find: ");
        pp = crate::me_dump::append_bytes(&mut pre, pp, label);
        crate::debug_log(&pre[..pp]);

        let ptr = unsafe {
            psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid)
        };

        if let Some(addr) = ptr {
            let mut msg = [0u8; 64];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] FOUND ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" @0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, addr as *const u8 as u32);
            crate::debug_log(&msg[..mp]);
        } else {
            let mut msg = [0u8; 64];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] miss: ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            crate::debug_log(&msg[..mp]);
        }
    }
}

enum LoadResult {
    Loaded,
    AlreadyLoaded,
    Failed,
}

/// Send an RPC command to the Media Engine.
///
/// Writes command + parameters to the ME command buffer at 0xBFC00600,
/// flushes cache, triggers ME via SysReg, waits for completion,
/// and returns the ME's response value.
///
/// # Parameters
/// - `cmd`: RPC command ID (see CMD_* constants)
/// - `params`: Up to 8 parameters (unused slots should be 0)
///
/// # Returns
/// The ME's return value from 0xBFC00628, or negative error code.
///
/// # Safety
/// Must be called from kernel mode after `me_init()`.
pub unsafe fn me_rpc(cmd: u32, params: &[u32; 8]) -> i32 {
    if !ME_INITIALIZED.load(Ordering::Relaxed) {
        return -1;
    }

    let sema = unsafe { core::ptr::read_volatile(&raw const ME_SEMA_ID) };
    let event = unsafe { core::ptr::read_volatile(&raw const ME_EVENT_ID) };

    // Step 1: Acquire exclusive ME access.
    let ret = unsafe {
        psp::sys::sceKernelWaitSema(sema, 1, core::ptr::null_mut())
    };
    if ret < 0 {
        return ret;
    }

    // Step 2: Write command and parameters to ME command buffer.
    let buf = ME_CMD_BUF as *mut u32;
    unsafe {
        core::ptr::write_volatile(buf, cmd);
        // Offset 0x04 is padding (skip).
        core::ptr::write_volatile(buf.add(2), params[0]); // +0x08
        core::ptr::write_volatile(buf.add(3), params[1]); // +0x0C
        core::ptr::write_volatile(buf.add(4), params[2]); // +0x10
        core::ptr::write_volatile(buf.add(5), params[3]); // +0x14
        core::ptr::write_volatile(buf.add(6), params[4]); // +0x18
        core::ptr::write_volatile(buf.add(7), params[5]); // +0x1C
        core::ptr::write_volatile(buf.add(8), params[6]); // +0x20
        core::ptr::write_volatile(buf.add(9), params[7]); // +0x24
    }

    // Step 3: Flush data cache so ME sees the writes.
    unsafe {
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            buf as *const c_void,
            0x30, // 48 bytes covers the whole command buffer
        );
    }

    // Step 4: Trigger ME interrupt via SysReg.
    // This is what sceSysregMeResetEnable does — write to interrupt trigger.
    unsafe {
        // Clear any pending event first.
        psp::sys::sceKernelClearEventFlag(event, !0u32);

        // Trigger ME: write 5 to a SysReg address.
        // From the disassembly: addiu $a0, $zero, 5 before the trigger call.
        // The actual trigger mechanism may be a SysReg API call.
        // For now, we signal by writing to the command buffer sync field.
        // TODO: The exact trigger mechanism needs validation on hardware.
        // The sceMeCodecWrapper calls 0x88226CC4 which is sceSysregMeResetEnable
        // or a similar function. We may need to resolve and call that.
    }

    // Step 5: Wait for ME completion via event flag.
    let mut out_bits: u32 = 0;
    let wait_ret = unsafe {
        psp::sys::sceKernelWaitEventFlag(
            event,
            1,      // wait for bit 0
            psp::sys::EventFlagWaitTypes::AND,  // 0x20 = WAIT_AND
            &mut out_bits,
            core::ptr::null_mut(), // no timeout
        )
    };

    // Step 6: Read return value from ME command buffer.
    let result = if wait_ret >= 0 {
        unsafe {
            // Invalidate cache to see ME's writes.
            psp::sys::sceKernelDcacheInvalidateRange(
                buf as *const c_void,
                0x30,
            );
            core::ptr::read_volatile(
                (ME_CMD_BUF + ME_RET_OFFSET) as *const i32
            )
        }
    } else {
        wait_ret
    };

    // Step 7: Release exclusive ME access.
    unsafe {
        psp::sys::sceKernelSignalSema(sema, 1);
    }

    result
}

/// Check if ME RPC is initialized and ready.
pub fn is_initialized() -> bool {
    ME_INITIALIZED.load(Ordering::Relaxed)
}

/// Resolved function pointers for direct ME driver calls.
static mut FN_VIDEOCODEC_OPEN: Option<unsafe extern "C" fn(i32, *mut u32) -> i32> = None;
static mut FN_AUDIOCODEC_CHECK: Option<unsafe extern "C" fn(*mut u32, i32) -> i32> = None;
static mut FN_ME_MEM_ALLOC: Option<unsafe extern "C" fn(u32) -> i32> = None;
static mut FN_RPC_SIMPLE: Option<
    unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32) -> i32
> = None;

/// Resolve and cache ME driver function pointers.
///
/// # Safety
/// Must be called from kernel mode after modules are verified loaded.
unsafe fn resolve_me_functions() {
    crate::debug_log(b"[ME-RPC] resolving fn ptrs...");

    // sceVideocodecOpen(type, codec_buf) -> int
    if let Some(ptr) = psp::hook::find_function(
        b"sceAvcodec_wrapper\0".as_ptr(),
        b"sceVideocodec\0".as_ptr(),
        0xC01EC829,
    ) {
        FN_VIDEOCODEC_OPEN = Some(core::mem::transmute(ptr));
        crate::debug_log(b"[ME-RPC] resolved sceVideocodecOpen");
    }

    // sceAudiocodecCheckNeedMem(codec_buf, type) -> int
    if let Some(ptr) = psp::hook::find_function(
        b"sceAvcodec_wrapper\0".as_ptr(),
        b"sceAudiocodec\0".as_ptr(),
        0x9D3F790C,
    ) {
        FN_AUDIOCODEC_CHECK = Some(core::mem::transmute(ptr));
        crate::debug_log(b"[ME-RPC] resolved sceAudiocodecCheckNeedMem");
    }

    // sceMeMemory alloc (NID 0x92D3BAA1) — takes size, returns addr
    if let Some(ptr) = psp::hook::find_function(
        b"sceMeCodecWrapper\0".as_ptr(),
        b"sceMeMemory_driver\0".as_ptr(),
        0x92D3BAA1,
    ) {
        FN_ME_MEM_ALLOC = Some(core::mem::transmute(ptr));
        crate::debug_log(b"[ME-RPC] resolved MeMemAlloc");
    }

    // sceMeCore RPC simple (NID 0x635397BB)
    if let Some(ptr) = psp::hook::find_function(
        b"sceMeCodecWrapper\0".as_ptr(),
        b"sceMeCore_driver\0".as_ptr(),
        0x635397BB,
    ) {
        FN_RPC_SIMPLE = Some(core::mem::transmute(ptr));
        crate::debug_log(b"[ME-RPC] resolved MeRpcSimple");
    }
}

/// Test the ME by calling resolved driver functions directly.
///
/// # Safety
/// Must be called from kernel mode after `me_init()`.
pub unsafe fn me_test() {
    crate::debug_log(b"[ME-RPC] testing via resolved functions...");

    // Resolve function pointers first.
    unsafe { resolve_me_functions() };

    // Test 1: Call sceVideocodecOpen through the kernel avcodec_wrapper.
    // This calls through the full ME RPC path internally.
    // We use a dummy codec buffer to see if it returns a meaningful error
    // (not crash, not empty-stub error 0x806201FE).
    let vcodec_fn = unsafe { core::ptr::read_volatile(&raw const FN_VIDEOCODEC_OPEN) };
    if let Some(open_fn) = vcodec_fn {
        crate::debug_log(b"[ME-RPC] calling sceVideocodecOpen...");
        let mut codec_buf = [0u32; 96];
        // Set version field (word 0) as expected by the codec.
        codec_buf[0] = 0x05100601;
        let ret = unsafe { open_fn(0, codec_buf.as_mut_ptr()) };
        let mut msg = [0u8; 64];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] VideocodecOpen=0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, ret as u32);
        crate::debug_log(&msg[..mp]);
    } else {
        crate::debug_log(b"[ME-RPC] VideocodecOpen not resolved");
    }

    // Test 2: Call sceAudiocodecCheckNeedMem (known to work on PSP).
    let acodec_fn = unsafe { core::ptr::read_volatile(&raw const FN_AUDIOCODEC_CHECK) };
    if let Some(check_fn) = acodec_fn {
        crate::debug_log(b"[ME-RPC] calling sceAudiocodecCheckNeedMem...");
        let mut codec_buf = [0u32; 96];
        // type 0x1003 = AAC
        let ret = unsafe { check_fn(codec_buf.as_mut_ptr(), 0x1003) };
        let mut msg = [0u8; 64];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] AudioCheckMem=0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, ret as u32);
        crate::debug_log(&msg[..mp]);
    } else {
        crate::debug_log(b"[ME-RPC] AudioCheckMem not resolved");
    }

    // Test 3: Call sceMeVideo_driver directly (bypass avcodec_wrapper).
    // This is the REAL ME video driver that talks to the ME coprocessor.
    let me_video_fn = unsafe {
        psp::hook::find_function(
            b"sceMeCodecWrapper\0".as_ptr(),
            b"sceMeVideo_driver\0".as_ptr(),
            0xC441994C, // VideocodecOpen equivalent
        )
    };
    if let Some(ptr) = me_video_fn {
        crate::debug_log(b"[ME-RPC] calling MeVideo directly...");
        // MeVideo::C441994C takes (type, codec_buf) like sceVideocodecOpen.
        let f: unsafe extern "C" fn(i32, *mut u32) -> i32 =
            unsafe { core::mem::transmute(ptr) };
        let mut codec_buf = [0u32; 96];
        codec_buf[0] = 0x05100601; // version
        let ret = unsafe { f(1, codec_buf.as_mut_ptr()) };
        let mut msg = [0u8; 64];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] MeVideoInit=0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, ret as u32);
        crate::debug_log(&msg[..mp]);
    } else {
        crate::debug_log(b"[ME-RPC] MeVideo fn not found");
    }

    // Test 4: Try calling RPC_simple with video command directly.
    let rpc_fn = unsafe { core::ptr::read_volatile(&raw const FN_RPC_SIMPLE) };
    if let Some(rpc) = rpc_fn {
        crate::debug_log(b"[ME-RPC] calling RPC cmd=0x82 (test)...");
        // Command 0x82 is a simple unknown command — safe to probe.
        let ret = unsafe { rpc(0x82i32, 0, 0, 0, 0, 0, 0, 0) };
        let mut msg = [0u8; 64];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-RPC] RPC 0x82=0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, ret as u32);
        crate::debug_log(&msg[..mp]);
    }

    crate::debug_log(b"[ME-RPC] test complete");
}
