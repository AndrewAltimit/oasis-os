//! Picture-in-picture video playback via sceMpeg (Media Engine).
//!
//! Decodes PSMF/PMF video files using the PSP's hardware Media Engine
//! coprocessor. Renders decoded frames into a 160x90 PIP window in the
//! bottom-right corner of the game's framebuffer.
//!
//! ## Architecture
//!
//! A dedicated kernel thread handles all sceMpeg operations:
//! - Scans `ms0:/VIDEO/` for `.pmf` files
//! - Initializes the sceMpeg decoder with ringbuffer
//! - Decodes AVC (H.264) video frames via the Media Engine
//! - Converts YCrCb to RGB via `sceMpegBaseCscVme`
//! - Downscales 480x272 to 160x90 PIP frame buffer
//! - Writes to a double-buffered shared frame read by the display hook
//! - Decodes ATRAC audio to a dedicated PSP audio channel
//! - PTS-based frame timing with periodic A/V resync
//!
//! ## Memory Budget (allocated on-demand from user partition 2)
//!
//! - PIP double buffer (160x90x4x2): ~116KB
//! - CSC output buffer (480x272x4): ~522KB
//! - PCM audio buffer: ~8KB
//! - sceMpeg decoder + ringbuffer: ~96KB (separate allocation)
//! - Total: ~742KB (allocated only when PIP is activated)

use core::sync::atomic::{AtomicU8, Ordering};

use crate::overlay;

// ---------------------------------------------------------------------------
// sceMpeg NIDs
// ---------------------------------------------------------------------------

const NID_MPEG_INIT: u32 = 0x682A619B;
const NID_MPEG_FINISH: u32 = 0x874624D6;
const NID_MPEG_CREATE: u32 = 0xD8C5F121;
const NID_MPEG_DELETE: u32 = 0x606A4649;
const NID_MPEG_QUERY_MEM_SIZE: u32 = 0xC132E22F;
const NID_MPEG_RINGBUF_QUERY_MEM_SIZE: u32 = 0xD7A29F46;
const NID_MPEG_RINGBUF_CONSTRUCT: u32 = 0x37295ED8;
const NID_MPEG_RINGBUF_DESTRUCT: u32 = 0x13407F13;
#[allow(dead_code)]
const NID_MPEG_RINGBUF_AVAILABLE: u32 = 0xB5F6DC87;
const NID_MPEG_RINGBUF_PUT: u32 = 0xB240A59E;
const NID_MPEG_QUERY_STREAM_OFFSET: u32 = 0x21FF80E4;
const NID_MPEG_QUERY_STREAM_SIZE: u32 = 0x611E9E11;
const NID_MPEG_REGIST_STREAM: u32 = 0x42560F23;
const NID_MPEG_UNREGIST_STREAM: u32 = 0x591A4AA2;
const NID_MPEG_FLUSH_ALL_STREAM: u32 = 0x707B7629;
const NID_MPEG_MALLOC_AVC_ES_BUF: u32 = 0xA780CF7E;
const NID_MPEG_FREE_AVC_ES_BUF: u32 = 0xCEB870B1;
const NID_MPEG_INIT_AU: u32 = 0x167AFD9E;
const NID_MPEG_GET_AVC_AU: u32 = 0xFE246728;
const NID_MPEG_AVC_DECODE_MODE: u32 = 0xA11C7026;
const NID_MPEG_AVC_DECODE: u32 = 0x0E3C2E9D;
#[allow(dead_code)]
const NID_MPEG_AVC_DECODE_STOP: u32 = 0x740FCCD1;
const NID_MPEG_GET_ATRAC_AU: u32 = 0xE1CE83A7;
const NID_MPEG_ATRAC_DECODE: u32 = 0x800C44DF;

// sceMpegbase NIDs
const NID_MPEG_BASE_CSC_INIT: u32 = 0x492B5E4B;
const NID_MPEG_BASE_CSC_VME: u32 = 0xCE8EB837;

// ---------------------------------------------------------------------------
// Module/library pairs for NID resolution
// ---------------------------------------------------------------------------

const MPEG_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceMpeg_library\0", b"sceMpeg\0"),
    (b"sceMpeg\0", b"sceMpeg\0"),
    (b"sceMpeg_Library\0", b"sceMpeg\0"),
    (b"sceMPEG_library\0", b"sceMpeg\0"),
    (b"mpeg_vsh\0", b"sceMpeg\0"),
    (b"mpeg.prx\0", b"sceMpeg\0"),
    (b"sceMpeg_library\0", b"sceMpeg_library\0"),
];

const MPEG_BASE_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceMpegbase_Driver\0", b"sceMpegbase\0"),
    (b"sceMpegbase\0", b"sceMpegbase\0"),
    (b"sceMpegBase_Driver\0", b"sceMpegbase\0"),
    (b"mpeg_vsh\0", b"sceMpegbase\0"),
    (b"mpegbase.prx\0", b"sceMpegbase\0"),
    (b"sceMpegbase_Driver\0", b"sceMpegbase_driver\0"),
    // Some firmware merges sceMpeg + sceMpegbase in one PRX.
    (b"sceMpeg_library\0", b"sceMpegbase\0"),
    (b"sceMpeg\0", b"sceMpegbase\0"),
    (b"mpeg.prx\0", b"sceMpegbase\0"),
    // Alternate library name capitalisation.
    (b"sceMpegbase_Driver\0", b"sceMpegBase\0"),
    (b"sceMpegbase\0", b"sceMpegBase\0"),
    (b"sceMpegbase_Driver\0", b"sceMpegbase_library\0"),
];

// sceUtility for loading AV modules
const NID_UTILITY_LOAD_MODULE: u32 = 0x2A2B3DE0;
const UTILITY_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceUtility_Driver\0", b"sceUtility_private\0"),
    (b"sceUtility_Driver\0", b"sceUtility_driver\0"),
    (b"sceUtility_Driver\0", b"sceUtility\0"),
    (b"sceUtility_private\0", b"sceUtility_private\0"),
    (b"sceUtility_private\0", b"sceUtility\0"),
];

/// PSP optional module IDs.
const PSP_MODULE_AV_AVCODEC: i32 = 0x0300;
const PSP_MODULE_AV_MPEGBASE: i32 = 0x0301;

// sceKernelLoadModule / sceKernelStartModule for kernel-mode PRX loading.
// sceUtilityLoadModule is user-mode only (returns 0x80111112 from kernel PRX),
// so we fall back to kernel module manager to load mpegbase from flash0.
const NID_LOAD_MODULE: u32 = 0x977DE386;
const NID_START_MODULE: u32 = 0x50F0C1EC;

const MODULE_MGR_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceModuleManager\0", b"ModuleMgrForKernel\0"),
    (b"sceModuleManager\0", b"ModuleMgrForUser\0"),
    (b"sceModuleManager\0", b"sceModuleManager\0"),
];

/// Flash0 paths to try for loading the mpegbase PRX.
const MPEGBASE_FLASH_PATHS: &[&[u8]] = &[
    b"flash0:/kd/mpegbase.prx\0",
    b"flash0:/kd/mpeg_vsh.prx\0",
    b"flash0:/kd/avcodec.prx\0",
    b"flash0:/vsh/module/mpegbase.prx\0",
];

// sceAudio for video ATRAC audio output
const NID_AUDIO_CH_RESERVE: u32 = 0x5EC81C55;
const NID_AUDIO_OUTPUT_BLOCKING: u32 = 0x136CAF51;
const NID_AUDIO_CH_RELEASE: u32 = 0x6FC46853;

const AUDIO_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceAudio_Driver\0", b"sceAudio_driver\0"),
    (b"sceAudio_Driver\0", b"sceAudio\0"),
    (b"sceAudio_Service\0", b"sceAudio_driver\0"),
    (b"sceAudio_Service\0", b"sceAudio\0"),
];

// ---------------------------------------------------------------------------
// Resolved function pointers
// ---------------------------------------------------------------------------

static mut MPEG_INIT_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut MPEG_FINISH_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut MPEG_CREATE_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, i32, *mut u32, i32, i32, i32) -> i32,
> = None;
static mut MPEG_DELETE_FN: Option<unsafe extern "C" fn(*mut u32) -> i32> = None;
static mut MPEG_QUERY_MEM_SIZE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut MPEG_RINGBUF_QUERY_MEM_SIZE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut MPEG_RINGBUF_CONSTRUCT_FN: Option<
    unsafe extern "C" fn(
        *mut u8, // ringbuf
        i32,     // packets
        *mut u8, // data
        i32,     // size
        unsafe extern "C" fn(*mut u8, i32, *mut u8) -> i32, // callback
        *mut u8, // cb_param (file descriptor pointer)
    ) -> i32,
> = None;
static mut MPEG_RINGBUF_DESTRUCT_FN: Option<unsafe extern "C" fn(*mut u8) -> i32> = None;
static mut MPEG_RINGBUF_PUT_FN: Option<unsafe extern "C" fn(*mut u8, i32, i32) -> i32> = None;
static mut MPEG_QUERY_STREAM_OFFSET_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, *mut u32) -> i32,
> = None;
static mut MPEG_QUERY_STREAM_SIZE_FN: Option<
    unsafe extern "C" fn(*mut u8, *mut u32) -> i32,
> = None;
static mut MPEG_REGIST_STREAM_FN: Option<
    unsafe extern "C" fn(*mut u32, i32, i32) -> *mut u8,
> = None;
static mut MPEG_UNREGIST_STREAM_FN: Option<unsafe extern "C" fn(*mut u32, *mut u8) -> i32> = None;
static mut MPEG_FLUSH_ALL_STREAM_FN: Option<unsafe extern "C" fn(*mut u32) -> i32> = None;
static mut MPEG_MALLOC_AVC_ES_BUF_N: Option<unsafe extern "C" fn(*mut u32) -> *mut u8> = None;
static mut MPEG_FREE_AVC_ES_BUF_FN: Option<unsafe extern "C" fn(*mut u32, *mut u8) -> i32> = None;
static mut MPEG_INIT_AU_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, *mut u8) -> i32,
> = None;
static mut MPEG_GET_AVC_AU_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, *mut u8, *mut i32) -> i32,
> = None;
static mut MPEG_AVC_DECODE_MODE_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut i32) -> i32,
> = None;
static mut MPEG_AVC_DECODE_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, i32, *mut u8, *mut i32) -> i32,
> = None;
static mut MPEG_GET_ATRAC_AU_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, *mut u8, *mut i32) -> i32,
> = None;
static mut MPEG_ATRAC_DECODE_FN: Option<
    unsafe extern "C" fn(*mut u32, *mut u8, *mut u8, i32) -> i32,
> = None;

// sceMpegbase
static mut MPEG_BASE_CSC_INIT_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut MPEG_BASE_CSC_VME_FN: Option<
    unsafe extern "C" fn(*mut u8, *mut u8, i32, *mut i32) -> i32,
> = None;

// sceAudio for video ATRAC output (channel 7)
static mut VID_AUDIO_RESERVE_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> = None;
static mut VID_AUDIO_OUTPUT_FN: Option<unsafe extern "C" fn(i32, i32, *const u8) -> i32> = None;
#[allow(dead_code)]
static mut VID_AUDIO_RELEASE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;

// ---------------------------------------------------------------------------
// PIP constants
// ---------------------------------------------------------------------------

/// PIP window dimensions.
const PIP_W: u32 = 160;
const PIP_H: u32 = 90;

/// Full decoded video dimensions (PSP native).
const DECODE_W: u32 = 480;
const DECODE_H: u32 = 272;

/// PIP position (bottom-right corner with padding).
const PIP_X: u32 = 310;
const PIP_Y: u32 = 172;

/// PIP border width.
const PIP_BORDER: u32 = 2;

/// Ringbuffer packet count.
const RINGBUF_PACKETS: i32 = 512;

/// Maximum number of video files in playlist.
const MAX_VIDEOS: usize = 16;

/// Maximum filename length.
const MAX_FILENAME: usize = 80;

/// PSMF stream types.
const PSMF_AVC_STREAM: i32 = 0;
const PSMF_ATRAC_STREAM: i32 = 1;

/// Audio channel for video ATRAC output.
const VIDEO_AUDIO_CHANNEL: i32 = 7;

/// ATRAC output samples per decode call (2048 stereo samples).
const ATRAC_SAMPLES: i32 = 2048;

/// Decode mode: pixel format ABGR8888.
const DECODE_PIXEL_MODE: i32 = 3;

// ---------------------------------------------------------------------------
// Video state (atomics for cross-thread communication)
// ---------------------------------------------------------------------------

/// Video commands: 0=none, 1=toggle, 2=next, 3=stop.
static VIDEO_CMD: AtomicU8 = AtomicU8::new(0);

/// PIP active flag: 0=inactive, 1=active.
static PIP_ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Current frame buffer index (0 or 1) for double buffering.
static FRAME_INDEX: AtomicU8 = AtomicU8::new(0);

/// Whether the video subsystem is available (sceMpeg resolved).
static VIDEO_AVAILABLE: AtomicU8 = AtomicU8::new(0);

/// Whether to use software CSC fallback (sceMpegBaseCscVme unavailable).
/// 0 = hardware CscVme, 1 = software I420→RGB.
static USE_SW_CSC: AtomicU8 = AtomicU8::new(0);

/// Decode frame counter (for periodic logging).
static DECODE_FRAME_COUNT: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Static buffers and state
// ---------------------------------------------------------------------------

/// Video file playlist (small, stays in BSS -- 1280 bytes).
static mut VIDEO_LIST: [[u8; MAX_FILENAME]; MAX_VIDEOS] = [[0u8; MAX_FILENAME]; MAX_VIDEOS];
static mut VIDEO_COUNT: usize = 0;
static mut CURRENT_VIDEO: usize = 0;

/// Video filename being played (for OSD display, 48 bytes).
static mut VIDEO_NAME: [u8; 48] = [0u8; 48];

/// sceMpeg handle (64 bytes).
static mut MPEG_HANDLE: [u32; 16] = [0u32; 16];

/// Video stream and audio stream handles.
static mut VIDEO_STREAM: *mut u8 = core::ptr::null_mut();
static mut AUDIO_STREAM: *mut u8 = core::ptr::null_mut();

/// AVC ES buffer.
static mut AVC_ES_BUF: *mut u8 = core::ptr::null_mut();

/// AU (Access Unit) structs for video and audio (128 bytes total).
static mut VIDEO_AU: [u8; 64] = [0u8; 64];
static mut AUDIO_AU: [u8; 64] = [0u8; 64];

/// sceMpeg ringbuffer struct (128 bytes).
static mut RINGBUF: [u8; 128] = [0u8; 128];

/// Allocated buffer pointers for MPEG decoder (from user partition 2).
static mut MPEG_BUF: *mut u8 = core::ptr::null_mut();
static mut RINGBUF_DATA: *mut u8 = core::ptr::null_mut();

/// Memory block ID for MPEG decoder buffers (partition 2).
/// Stored here so cleanup_mpeg can always free it correctly.
static mut MPEG_BLOCK_ID: i32 = -1;

/// On-demand buffers allocated from partition 2 (NOT static arrays).
/// These pointers are set by alloc_pip_buffers() on first PIP activation.
static mut PIP_FRAME_A: *mut u8 = core::ptr::null_mut();
static mut PIP_FRAME_B: *mut u8 = core::ptr::null_mut();
static mut PCM_BUF: *mut u8 = core::ptr::null_mut();
/// Full-frame CSC output buffer (480*272*4 bytes, ABGR8888).
/// sceMpegBaseCscVme writes the full decoded frame here, then
/// we downscale to the 160x90 PIP frame buffer.
static mut CSC_BUF: *mut u8 = core::ptr::null_mut();

/// Sizes for on-demand buffers.
const PIP_FRAME_SIZE: usize = (PIP_W * PIP_H * 4) as usize; // 57600
const PCM_BUF_SIZE: usize = ATRAC_SAMPLES as usize * 4; // 8192
const CSC_BUF_SIZE: usize = (DECODE_W * DECODE_H * 4) as usize; // 522240

/// Total on-demand allocation: 2 PIP frames + PCM + CSC + padding.
const PIP_ALLOC_SIZE: u32 =
    (PIP_FRAME_SIZE * 2 + PCM_BUF_SIZE + CSC_BUF_SIZE + 64) as u32;

/// Memory block ID for PIP buffers (partition 2).
static mut PIP_BUF_BLOCK: i32 = -1;

/// File descriptor for current video file.
static mut VIDEO_FD: i32 = -1;

/// Stream offset and size (from PSMF header).
static mut STREAM_OFFSET: u32 = 0;
static mut STREAM_SIZE: u32 = 0;

/// PTS tracking for A/V sync.
static mut LAST_VIDEO_PTS: u32 = 0;
static mut LAST_AUDIO_PTS: u32 = 0;
static mut LAST_SYNC_TIME: u32 = 0;

/// Audio channel handle.
static mut AUDIO_CH_HANDLE: i32 = -1;

// ---------------------------------------------------------------------------
// NID resolution (same pattern as audio.rs)
// ---------------------------------------------------------------------------

/// Resolve a NID trying each module/library pair, then null-module fallback.
unsafe fn resolve_nid(modules: &[(&[u8], &[u8])], nid: u32) -> Option<*mut u8> {
    for &(module, library) in modules {
        if let Some(ptr) =
            unsafe { psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    // Fallback: NULL module name (searches all loaded modules on PRO/ME/ARK).
    for &(_, library) in modules {
        if let Some(ptr) =
            unsafe { psp::hook::find_function(core::ptr::null(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Module loading
// ---------------------------------------------------------------------------

/// Load MPEG AV modules.
///
/// Strategy 1: sceUtilityLoadModule (user-mode, may fail from kernel PRX).
/// Strategy 2: sceKernelLoadModule from flash0 (kernel-mode fallback).
unsafe fn load_mpeg_modules() {
    // --- Strategy 1: sceUtilityLoadModule ---
    let utility_load: Option<unsafe extern "C" fn(i32) -> i32> = unsafe {
        resolve_nid(UTILITY_MODULES, NID_UTILITY_LOAD_MODULE).map(|ptr| core::mem::transmute(ptr))
    };

    let mut mpegbase_loaded = false;

    if let Some(load) = utility_load {
        crate::debug_log(b"[VIDEO] sceUtilityLoadModule resolved");
        let r1 = unsafe { load(PSP_MODULE_AV_AVCODEC) };
        log_i32(b"[VIDEO] LoadModule AVCODEC=", r1);
        let r2 = unsafe { load(PSP_MODULE_AV_MPEGBASE) };
        log_i32(b"[VIDEO] LoadModule MPEGBASE=", r2);
        if r2 >= 0 {
            mpegbase_loaded = true;
        }
    } else {
        crate::debug_log(b"[VIDEO] sceUtilityLoadModule NOT found");
    }

    if mpegbase_loaded {
        return;
    }

    // --- Strategy 2: kernel-mode module loading from flash0 ---
    // sceUtilityLoadModule fails from kernel PRX (0x80111112). Use
    // sceKernelLoadModule + sceKernelStartModule to load mpegbase.prx
    // directly from flash0.
    crate::debug_log(b"[VIDEO] trying kernel module load...");

    let load_fn: Option<unsafe extern "C" fn(*const u8, u32, *mut u8) -> i32> = unsafe {
        resolve_nid(MODULE_MGR_MODULES, NID_LOAD_MODULE).map(|ptr| core::mem::transmute(ptr))
    };
    let start_fn: Option<unsafe extern "C" fn(i32, u32, *mut u8, *mut i32, *mut u8) -> i32> =
        unsafe {
            resolve_nid(MODULE_MGR_MODULES, NID_START_MODULE)
                .map(|ptr| core::mem::transmute(ptr))
        };

    if load_fn.is_none() {
        crate::debug_log(b"[VIDEO] sceKernelLoadModule NOT found");
    }
    if start_fn.is_none() {
        crate::debug_log(b"[VIDEO] sceKernelStartModule NOT found");
    }

    if let (Some(load), Some(start)) = (load_fn, start_fn) {
        for &path in MPEGBASE_FLASH_PATHS {
            let mod_id = unsafe { load(path.as_ptr(), 0, core::ptr::null_mut()) };
            if mod_id >= 0 {
                log_i32(b"[VIDEO] kmod loaded id=", mod_id);
                let mut status = 0i32;
                let ret = unsafe {
                    start(
                        mod_id,
                        0,
                        core::ptr::null_mut(),
                        &mut status,
                        core::ptr::null_mut(),
                    )
                };
                log_i32(b"[VIDEO] kmod start=", ret);
                if ret >= 0 {
                    crate::debug_log(b"[VIDEO] mpegbase loaded from flash0");
                    // Give module time to register its library exports.
                    unsafe {
                        psp::sys::sceKernelDelayThread(200_000);
                    }
                    return;
                }
            } else {
                log_i32(b"[VIDEO] kmod load=", mod_id);
            }
        }
        crate::debug_log(b"[VIDEO] all flash0 paths failed");
    }
}

/// Allocate PIP frame buffers, CSC buffer, and PCM buffer from user-memory
/// partition 2. Returns true on success.
unsafe fn alloc_pip_buffers() -> bool {
    if unsafe { PIP_BUF_BLOCK } >= 0 {
        crate::debug_log(b"[VIDEO] PIP bufs already allocated");
        return true; // Already allocated.
    }

    log_u32(b"[VIDEO] PIP alloc requesting bytes=", PIP_ALLOC_SIZE);
    let block_id = unsafe {
        psp::sys::sceKernelAllocPartitionMemory(
            psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
            b"OasisPIP\0".as_ptr(),
            psp::sys::SceSysMemBlockTypes::Low,
            PIP_ALLOC_SIZE,
            core::ptr::null_mut(),
        )
    };
    if block_id < psp::sys::SceUid(0) {
        log_i32(b"[VIDEO] PIP buf alloc FAILED=", block_id.0);
        return false;
    }

    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block_id) } as *mut u8;
    let aligned = ((base as u32 + 15) & !15) as *mut u8;
    log_hex(b"[VIDEO] PIP buf base=", aligned as u32);

    unsafe {
        PIP_FRAME_A = aligned;
        PIP_FRAME_B = aligned.add(PIP_FRAME_SIZE);
        PCM_BUF = aligned.add(PIP_FRAME_SIZE * 2);
        CSC_BUF = aligned.add(PIP_FRAME_SIZE * 2 + PCM_BUF_SIZE);
        PIP_BUF_BLOCK = block_id.0;

        // Zero the frame buffers.
        let mut i = 0;
        while i < PIP_FRAME_SIZE {
            *PIP_FRAME_A.add(i) = 0;
            *PIP_FRAME_B.add(i) = 0;
            i += 1;
        }
    }

    log_hex(b"[VIDEO] PIP_FRAME_A=", unsafe { PIP_FRAME_A } as u32);
    log_hex(b"[VIDEO] PIP_FRAME_B=", unsafe { PIP_FRAME_B } as u32);
    log_hex(b"[VIDEO] PCM_BUF=", unsafe { PCM_BUF } as u32);
    log_hex(b"[VIDEO] CSC_BUF=", unsafe { CSC_BUF } as u32);
    log_i32(b"[VIDEO] PIP bufs allocated, block=", block_id.0);
    true
}

/// Free PIP frame buffers.
unsafe fn free_pip_buffers() {
    let block = unsafe { PIP_BUF_BLOCK };
    if block >= 0 {
        unsafe {
            psp::sys::sceKernelFreePartitionMemory(psp::sys::SceUid(block));
            PIP_BUF_BLOCK = -1;
            PIP_FRAME_A = core::ptr::null_mut();
            PIP_FRAME_B = core::ptr::null_mut();
            PCM_BUF = core::ptr::null_mut();
            CSC_BUF = core::ptr::null_mut();
        }
        crate::debug_log(b"[VIDEO] PIP bufs freed");
    }
}

/// Try to resolve all sceMpeg NIDs. Returns true if core functions resolved.
unsafe fn try_resolve_mpeg() -> bool {
    // Load required AV modules first.
    unsafe {
        load_mpeg_modules();
        // Give loaded modules time to register their exports.
        psp::sys::sceKernelDelayThread(500_000);
    }

    let mut core_ok = true;
    let mut resolved_count: u32 = 0;
    let mut total_count: u32 = 0;

    macro_rules! resolve {
        ($fn_ptr:ident, $modules:expr, $nid:expr, $name:expr, $required:expr) => {
            total_count += 1;
            unsafe {
                if let Some(ptr) = resolve_nid($modules, $nid) {
                    core::ptr::write_volatile(&raw mut $fn_ptr, Some(core::mem::transmute(ptr)));
                    resolved_count += 1;
                    log_nid_ok($name, ptr as u32);
                } else {
                    if $required {
                        core_ok = false;
                    }
                    log_nid_fail($name, $nid);
                }
            }
        };
    }

    crate::debug_log(b"[VIDEO] resolving sceMpeg NIDs...");

    // Core sceMpeg functions (all required)
    resolve!(MPEG_INIT_FN, MPEG_MODULES, NID_MPEG_INIT, b"MpegInit", true);
    resolve!(MPEG_FINISH_FN, MPEG_MODULES, NID_MPEG_FINISH, b"MpegFinish", true);
    resolve!(MPEG_CREATE_FN, MPEG_MODULES, NID_MPEG_CREATE, b"MpegCreate", true);
    resolve!(MPEG_DELETE_FN, MPEG_MODULES, NID_MPEG_DELETE, b"MpegDelete", true);
    resolve!(MPEG_QUERY_MEM_SIZE_FN, MPEG_MODULES, NID_MPEG_QUERY_MEM_SIZE, b"QueryMemSz", true);
    resolve!(
        MPEG_RINGBUF_QUERY_MEM_SIZE_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_QUERY_MEM_SIZE,
        b"RbQuerySz",
        true
    );
    resolve!(
        MPEG_RINGBUF_CONSTRUCT_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_CONSTRUCT,
        b"RbConstruct",
        true
    );
    resolve!(
        MPEG_RINGBUF_DESTRUCT_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_DESTRUCT,
        b"RbDestruct",
        true
    );
    resolve!(MPEG_RINGBUF_PUT_FN, MPEG_MODULES, NID_MPEG_RINGBUF_PUT, b"RbPut", true);
    resolve!(
        MPEG_QUERY_STREAM_OFFSET_FN,
        MPEG_MODULES,
        NID_MPEG_QUERY_STREAM_OFFSET,
        b"QStreamOff",
        true
    );
    resolve!(
        MPEG_QUERY_STREAM_SIZE_FN,
        MPEG_MODULES,
        NID_MPEG_QUERY_STREAM_SIZE,
        b"QStreamSz",
        true
    );
    resolve!(MPEG_REGIST_STREAM_FN, MPEG_MODULES, NID_MPEG_REGIST_STREAM, b"RegStream", true);
    resolve!(
        MPEG_UNREGIST_STREAM_FN,
        MPEG_MODULES,
        NID_MPEG_UNREGIST_STREAM,
        b"UnregStream",
        true
    );
    resolve!(
        MPEG_FLUSH_ALL_STREAM_FN,
        MPEG_MODULES,
        NID_MPEG_FLUSH_ALL_STREAM,
        b"FlushStream",
        true
    );
    resolve!(
        MPEG_MALLOC_AVC_ES_BUF_N,
        MPEG_MODULES,
        NID_MPEG_MALLOC_AVC_ES_BUF,
        b"MallocEsBuf",
        true
    );
    resolve!(
        MPEG_FREE_AVC_ES_BUF_FN,
        MPEG_MODULES,
        NID_MPEG_FREE_AVC_ES_BUF,
        b"FreeEsBuf",
        true
    );
    resolve!(MPEG_INIT_AU_FN, MPEG_MODULES, NID_MPEG_INIT_AU, b"InitAu", true);
    resolve!(MPEG_GET_AVC_AU_FN, MPEG_MODULES, NID_MPEG_GET_AVC_AU, b"GetAvcAu", true);
    resolve!(
        MPEG_AVC_DECODE_MODE_FN,
        MPEG_MODULES,
        NID_MPEG_AVC_DECODE_MODE,
        b"AvcDecMode",
        true
    );
    resolve!(MPEG_AVC_DECODE_FN, MPEG_MODULES, NID_MPEG_AVC_DECODE, b"AvcDecode", true);

    // Audio decode (optional -- PIP can work without audio)
    resolve!(MPEG_GET_ATRAC_AU_FN, MPEG_MODULES, NID_MPEG_GET_ATRAC_AU, b"GetAtracAu", false);
    resolve!(MPEG_ATRAC_DECODE_FN, MPEG_MODULES, NID_MPEG_ATRAC_DECODE, b"AtracDec", false);

    // sceMpegbase CSC (optional -- software fallback if unavailable)
    resolve!(MPEG_BASE_CSC_INIT_FN, MPEG_BASE_MODULES, NID_MPEG_BASE_CSC_INIT, b"CscInit", false);
    resolve!(MPEG_BASE_CSC_VME_FN, MPEG_BASE_MODULES, NID_MPEG_BASE_CSC_VME, b"CscVme", false);

    // Audio output for video ATRAC (optional)
    unsafe {
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RESERVE) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
            crate::debug_log(b"[VIDEO] AudioChReserve OK");
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_OUTPUT_BLOCKING) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_OUTPUT_FN,
                Some(core::mem::transmute(ptr)),
            );
            crate::debug_log(b"[VIDEO] AudioOutput OK");
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RELEASE) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_RELEASE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
    }

    // Summary log.
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[VIDEO] NIDs: ");
    p = write_decimal(&mut buf, p, resolved_count);
    p = copy_bytes(&mut buf, p, b"/");
    p = write_decimal(&mut buf, p, total_count);
    if core_ok {
        p = copy_bytes(&mut buf, p, b" ALL OK");
    } else {
        p = copy_bytes(&mut buf, p, b" MISSING");
    }
    crate::debug_log(&buf[..p]);

    // Check if CscVme is available; if not, use software CSC fallback.
    let has_csc = unsafe {
        core::ptr::read_volatile(&raw const MPEG_BASE_CSC_VME_FN).is_some()
    };
    if !has_csc {
        USE_SW_CSC.store(1, Ordering::Relaxed);
        crate::debug_log(b"[VIDEO] CscVme missing -> software CSC");
    } else {
        USE_SW_CSC.store(0, Ordering::Relaxed);
        crate::debug_log(b"[VIDEO] CscVme available -> hardware CSC");
    }

    if core_ok {
        VIDEO_AVAILABLE.store(1, Ordering::Relaxed);
        crate::debug_log(b"[VIDEO] video subsystem AVAILABLE");
    } else {
        crate::debug_log(b"[VIDEO] video subsystem NOT available");
    }

    core_ok
}

// ---------------------------------------------------------------------------
// File scanning
// ---------------------------------------------------------------------------

/// Scan ms0:/VIDEO/ for .pmf files. Populates VIDEO_LIST.
unsafe fn scan_video_dir() {
    let config = crate::config::get_config();
    let dir_path = config.video_dir_str();

    crate::debug_log(b"[VIDEO] scanning video dir...");

    // SAFETY: sceIoDopen with valid null-terminated path.
    let dfd = unsafe { psp::sys::sceIoDopen(dir_path.as_ptr()) };
    if dfd.0 < 0 {
        log_i32(b"[VIDEO] dopen failed=", dfd.0);
        return;
    }

    // SAFETY: Single-threaded, VIDEO_LIST only written here.
    unsafe {
        VIDEO_COUNT = 0;
    }

    loop {
        // SceIoDirent is 0x148 bytes on PSP.
        let mut dirent = [0u8; 0x148];
        // SAFETY: sceIoDread with valid fd and buffer.
        let ret = unsafe {
            psp::sys::sceIoDread(
                psp::sys::SceUid(dfd.0),
                &mut dirent as *mut _ as *mut psp::sys::SceIoDirent,
            )
        };
        if ret <= 0 {
            break;
        }

        // Filename is at offset 0x104 in SceIoDirent (d_name, 256 bytes).
        let name_offset = 0x104;
        let name = &dirent[name_offset..];

        // Find name length.
        let mut name_len = 0;
        while name_len < 255 && name[name_len] != 0 {
            name_len += 1;
        }
        if name_len < 5 {
            continue;
        }

        // Check for .pmf extension (case-insensitive).
        let ext = &name[name_len - 4..name_len];
        let is_pmf = (ext[0] == b'.')
            && (ext[1] == b'p' || ext[1] == b'P')
            && (ext[2] == b'm' || ext[2] == b'M')
            && (ext[3] == b'f' || ext[3] == b'F');

        if !is_pmf {
            continue;
        }

        // SAFETY: VIDEO_COUNT is bounded.
        unsafe {
            if VIDEO_COUNT >= MAX_VIDEOS {
                break;
            }

            // Build full path: dir + filename.
            let dir_len = dir_path.len() - 1; // Exclude null terminator.
            let total = dir_len + name_len;
            if total >= MAX_FILENAME - 1 {
                continue;
            }

            let slot_ptr = (&raw mut VIDEO_LIST)
                .cast::<[u8; MAX_FILENAME]>()
                .add(VIDEO_COUNT)
                .cast::<u8>();
            let mut i = 0;
            while i < dir_len {
                *slot_ptr.add(i) = dir_path[i];
                i += 1;
            }
            let mut j = 0;
            while j < name_len {
                *slot_ptr.add(i + j) = name[j];
                j += 1;
            }
            *slot_ptr.add(i + j) = 0; // Null terminate.

            // Log each found file.
            crate::debug_log(core::slice::from_raw_parts(slot_ptr, i + j));

            VIDEO_COUNT += 1;
        }
    }

    // SAFETY: Close directory.
    unsafe {
        psp::sys::sceIoDclose(psp::sys::SceUid(dfd.0));
    }

    log_usize(b"[VIDEO] found videos: ", unsafe { VIDEO_COUNT });
}

// ---------------------------------------------------------------------------
// sceMpeg initialization and teardown
// ---------------------------------------------------------------------------

/// Ringbuffer read callback -- reads data from the video file into the
/// ringbuffer's data area.
///
/// `data` is the destination buffer in the ringbuffer where we must write.
/// `packets` is the number of 2048-byte packets to fill.
unsafe extern "C" fn ringbuf_callback(
    data: *mut u8,
    packets: i32,
    _param: *mut u8,
) -> i32 {
    if packets <= 0 || data.is_null() {
        return 0;
    }

    // SAFETY: VIDEO_FD set before ringbuffer is active.
    let fd = unsafe { core::ptr::read_volatile(&raw const VIDEO_FD) };
    if fd < 0 {
        return 0;
    }

    // Each ringbuffer packet is 2048 bytes.
    let bytes_to_read = (packets as u32) * 2048;
    let mut total_read: u32 = 0;

    while total_read < bytes_to_read {
        let chunk = (bytes_to_read - total_read).min(65536); // Read in 64KB chunks.
        // SAFETY: sceIoRead with valid fd; data is the ringbuffer's own memory.
        let ret = unsafe {
            psp::sys::sceIoRead(
                psp::sys::SceUid(fd),
                data.add(total_read as usize) as *mut _,
                chunk,
            )
        };
        if ret <= 0 {
            break;
        }
        total_read += ret as u32;
    }

    (total_read / 2048) as i32
}

/// Initialize sceMpeg decoder for a video file. Returns true on success.
/// On success, stores the memory block ID in MPEG_BLOCK_ID for cleanup.
unsafe fn init_mpeg_decoder(filepath: &[u8]) -> bool {
    crate::debug_log(b"[VIDEO] init_mpeg_decoder...");

    // Log the filepath being opened (up to 64 bytes).
    let mut fpath_len = 0;
    while fpath_len < filepath.len() && filepath[fpath_len] != 0 {
        fpath_len += 1;
    }
    if fpath_len > 0 {
        crate::debug_log(&filepath[..fpath_len]);
    }

    // Open the video file.
    // SAFETY: sceIoOpen with valid path.
    let fd = unsafe {
        psp::sys::sceIoOpen(filepath.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        log_i32(b"[VIDEO] open failed=", fd.0);
        return false;
    }
    log_i32(b"[VIDEO] file fd=", fd.0);
    unsafe {
        VIDEO_FD = fd.0;
    }

    // Read PSMF header (first 2048 bytes).
    let mut header = [0u8; 2048];
    // SAFETY: Valid fd and buffer.
    let ret = unsafe {
        psp::sys::sceIoRead(fd, header.as_mut_ptr() as *mut _, 2048)
    };
    if ret < 2048 {
        log_i32(b"[VIDEO] header read only=", ret);
        unsafe {
            psp::sys::sceIoClose(fd);
            VIDEO_FD = -1;
        }
        return false;
    }

    // Log PSMF magic bytes for validation.
    let mut magic_buf = [0u8; 32];
    let mut mp = copy_bytes(&mut magic_buf, 0, b"[VIDEO] magic=");
    mp = write_hex_byte(&mut magic_buf, mp, header[0]);
    mp = write_hex_byte(&mut magic_buf, mp, header[1]);
    mp = write_hex_byte(&mut magic_buf, mp, header[2]);
    mp = write_hex_byte(&mut magic_buf, mp, header[3]);
    crate::debug_log(&magic_buf[..mp]);

    // Initialize sceMpeg.
    let init_fn = unsafe { core::ptr::read_volatile(&raw const MPEG_INIT_FN) };
    if let Some(f) = init_fn {
        let r = unsafe { f() };
        log_i32(b"[VIDEO] sceMpegInit=", r);
        if r < 0 {
            unsafe {
                psp::sys::sceIoClose(fd);
                VIDEO_FD = -1;
            }
            return false;
        }
    } else {
        crate::debug_log(b"[VIDEO] sceMpegInit FN missing");
        return false;
    }

    // Query memory sizes.
    let mpeg_mem_size = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_MEM_SIZE_FN) {
            f(0)
        } else {
            crate::debug_log(b"[VIDEO] QueryMemSize FN missing");
            return false;
        }
    };
    let ringbuf_mem_size = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_QUERY_MEM_SIZE_FN) {
            f(RINGBUF_PACKETS)
        } else {
            crate::debug_log(b"[VIDEO] RbQueryMemSize FN missing");
            return false;
        }
    };

    log_i32(b"[VIDEO] mpeg mem=", mpeg_mem_size);
    log_i32(b"[VIDEO] ringbuf mem=", ringbuf_mem_size);

    // Allocate from user memory partition 2.
    let total_alloc = mpeg_mem_size + ringbuf_mem_size + 64;
    log_i32(b"[VIDEO] decoder alloc=", total_alloc);
    // SAFETY: sceKernelAllocPartitionMemory for user partition.
    let block_id = unsafe {
        psp::sys::sceKernelAllocPartitionMemory(
            psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
            b"OasisMpeg\0".as_ptr(),
            psp::sys::SceSysMemBlockTypes::Low,
            total_alloc as u32,
            core::ptr::null_mut(),
        )
    };
    if block_id < psp::sys::SceUid(0) {
        log_i32(b"[VIDEO] decoder alloc FAILED=", block_id.0);
        unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_FINISH_FN) {
                f();
            }
            psp::sys::sceIoClose(fd);
            VIDEO_FD = -1;
        }
        return false;
    }
    log_i32(b"[VIDEO] decoder block=", block_id.0);

    // Store block ID for cleanup.
    unsafe {
        MPEG_BLOCK_ID = block_id.0;
    }

    // SAFETY: Get block address.
    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block_id) } as *mut u8;
    // 16-byte align.
    let aligned = ((base as u32 + 15) & !15) as *mut u8;
    unsafe {
        MPEG_BUF = aligned;
        RINGBUF_DATA = aligned.add(mpeg_mem_size as usize);
    }
    log_hex(b"[VIDEO] MPEG_BUF=", unsafe { MPEG_BUF } as u32);

    // Construct ringbuffer.
    // SAFETY: All pointers valid, callback is safe.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_CONSTRUCT_FN) {
            f(
                (&raw mut RINGBUF).cast::<u8>(),
                RINGBUF_PACKETS,
                RINGBUF_DATA,
                ringbuf_mem_size,
                ringbuf_callback,
                core::ptr::null_mut(),
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        log_i32(b"[VIDEO] ringbuf construct=", ret);
        // SAFETY: Cleaning up partially-initialized mpeg state.
        unsafe { cleanup_mpeg() };
        return false;
    }
    crate::debug_log(b"[VIDEO] ringbuf constructed OK");

    // Create sceMpeg handle.
    // SAFETY: All buffers allocated and aligned.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_CREATE_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                MPEG_BUF,
                mpeg_mem_size,
                (&raw mut RINGBUF).cast::<u8>() as *mut u32,
                DECODE_W as i32, // Video width (PSP native)
                0,               // Mode
                0,               // Reserved
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        log_i32(b"[VIDEO] sceMpegCreate=", ret);
        // SAFETY: Cleaning up partially-initialized mpeg state.
        unsafe { cleanup_mpeg() };
        return false;
    }
    crate::debug_log(b"[VIDEO] sceMpegCreate OK");

    // Query stream offset and size from header.
    unsafe {
        let mut offset = 0u32;
        let mut size = 0u32;
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_STREAM_OFFSET_FN) {
            let r = f((&raw mut MPEG_HANDLE).cast::<u32>(), header.as_mut_ptr(), &mut offset);
            log_i32(b"[VIDEO] QStreamOffset ret=", r);
        }
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_STREAM_SIZE_FN) {
            let r = f(header.as_mut_ptr(), &mut size);
            log_i32(b"[VIDEO] QStreamSize ret=", r);
        }
        STREAM_OFFSET = offset;
        STREAM_SIZE = size;
        log_u32(b"[VIDEO] stream offset=", offset);
        log_u32(b"[VIDEO] stream size=", size);
    }

    // Seek to stream data start.
    // SAFETY: Valid fd.
    unsafe {
        psp::sys::sceIoLseek(fd, STREAM_OFFSET as i64, psp::sys::IoWhence::Set);
    }

    // Register video (AVC) and audio (ATRAC) streams.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_REGIST_STREAM_FN) {
            VIDEO_STREAM = f((&raw mut MPEG_HANDLE).cast::<u32>(), PSMF_AVC_STREAM, 0);
            AUDIO_STREAM = f((&raw mut MPEG_HANDLE).cast::<u32>(), PSMF_ATRAC_STREAM, 0);
        }
        log_hex(b"[VIDEO] VIDEO_STREAM=", VIDEO_STREAM as u32);
        log_hex(b"[VIDEO] AUDIO_STREAM=", AUDIO_STREAM as u32);
        if VIDEO_STREAM.is_null() {
            crate::debug_log(b"[VIDEO] video stream register FAILED");
            cleanup_mpeg();
            return false;
        }
    }

    // Set decode mode: [-1, pixel_format]. First field is decode mode
    // (-1 = default), second is pixel format (3 = ABGR8888).
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_AVC_DECODE_MODE_FN) {
            let mut mode = [-1i32, DECODE_PIXEL_MODE];
            let r = f((&raw mut MPEG_HANDLE).cast::<u32>(), mode.as_mut_ptr());
            log_i32(b"[VIDEO] AvcDecodeMode=", r);
        }
    }

    // Allocate AVC ES buffer.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_MALLOC_AVC_ES_BUF_N) {
            AVC_ES_BUF = f((&raw mut MPEG_HANDLE).cast::<u32>());
        }
        log_hex(b"[VIDEO] AVC_ES_BUF=", AVC_ES_BUF as u32);
        if AVC_ES_BUF.is_null() {
            crate::debug_log(b"[VIDEO] AVC ES buf alloc FAILED");
            cleanup_mpeg();
            return false;
        }
    }

    // Initialize AU structs.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_INIT_AU_FN) {
            let r1 = f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                AVC_ES_BUF,
                (&raw mut VIDEO_AU).cast::<u8>(),
            );
            log_i32(b"[VIDEO] InitAu video=", r1);
            if !AUDIO_STREAM.is_null() {
                let r2 = f(
                    (&raw mut MPEG_HANDLE).cast::<u32>(),
                    AVC_ES_BUF, // Reuse ES buf for AU init
                    (&raw mut AUDIO_AU).cast::<u8>(),
                );
                log_i32(b"[VIDEO] InitAu audio=", r2);
            }
        }
    }

    // Initialize CSC (color space conversion) at full decode width.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_BASE_CSC_INIT_FN) {
            let r = f(DECODE_W as i32);
            log_i32(b"[VIDEO] CscInit=", r);
        }
    }

    // Reserve audio channel for ATRAC output.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const VID_AUDIO_RESERVE_FN) {
            let ch = f(VIDEO_AUDIO_CHANNEL, ATRAC_SAMPLES, 0); // 0 = stereo
            log_i32(b"[VIDEO] audio ch reserve=", ch);
            if ch >= 0 {
                AUDIO_CH_HANDLE = ch;
            } else {
                AUDIO_CH_HANDLE = -1;
                crate::debug_log(b"[VIDEO] audio ch reserve FAILED");
            }
        }
    }

    // Reset PTS tracking.
    unsafe {
        LAST_VIDEO_PTS = 0;
        LAST_AUDIO_PTS = 0;
        LAST_SYNC_TIME = 0;
    }

    // Reset decode counter.
    DECODE_FRAME_COUNT.store(0, Ordering::Relaxed);

    crate::debug_log(b"[VIDEO] decoder initialized OK");
    true
}

/// Clean up sceMpeg resources. Uses MPEG_BLOCK_ID for memory block.
unsafe fn cleanup_mpeg() {
    crate::debug_log(b"[VIDEO] cleanup_mpeg...");
    unsafe {
        // Free AVC ES buffer.
        if !AVC_ES_BUF.is_null() {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_FREE_AVC_ES_BUF_FN) {
                f((&raw mut MPEG_HANDLE).cast::<u32>(), AVC_ES_BUF);
            }
            AVC_ES_BUF = core::ptr::null_mut();
        }

        // Unregister streams.
        if !VIDEO_STREAM.is_null() {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_UNREGIST_STREAM_FN) {
                f((&raw mut MPEG_HANDLE).cast::<u32>(), VIDEO_STREAM);
            }
            VIDEO_STREAM = core::ptr::null_mut();
        }
        if !AUDIO_STREAM.is_null() {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_UNREGIST_STREAM_FN) {
                f((&raw mut MPEG_HANDLE).cast::<u32>(), AUDIO_STREAM);
            }
            AUDIO_STREAM = core::ptr::null_mut();
        }

        // Delete sceMpeg handle.
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_DELETE_FN) {
            f((&raw mut MPEG_HANDLE).cast::<u32>());
        }

        // Destruct ringbuffer.
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_DESTRUCT_FN) {
            f((&raw mut RINGBUF).cast::<u8>());
        }

        // Finish sceMpeg.
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_FINISH_FN) {
            f();
        }

        // Release audio channel.
        if AUDIO_CH_HANDLE >= 0 {
            if let Some(f) = core::ptr::read_volatile(&raw const VID_AUDIO_RELEASE_FN) {
                f(AUDIO_CH_HANDLE);
            }
            AUDIO_CH_HANDLE = -1;
        }

        // Close file.
        if VIDEO_FD >= 0 {
            psp::sys::sceIoClose(psp::sys::SceUid(VIDEO_FD));
            VIDEO_FD = -1;
        }

        // Free memory partition using stored block ID.
        let block = MPEG_BLOCK_ID;
        if block >= 0 {
            psp::sys::sceKernelFreePartitionMemory(psp::sys::SceUid(block));
            MPEG_BLOCK_ID = -1;
            crate::debug_log(b"[VIDEO] decoder mem freed");
        }
        MPEG_BUF = core::ptr::null_mut();
        RINGBUF_DATA = core::ptr::null_mut();
    }
    crate::debug_log(b"[VIDEO] cleanup done");
}

// ---------------------------------------------------------------------------
// Video and audio decoding
// ---------------------------------------------------------------------------

/// Fill ringbuffer with data from the file.
unsafe fn fill_ringbuffer() -> i32 {
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_PUT_FN) {
            f((&raw mut RINGBUF).cast::<u8>(), RINGBUF_PACKETS, 0)
        } else {
            -1
        }
    };
    ret
}

/// Decode one video frame. Returns true if a frame was decoded.
unsafe fn decode_video_frame() -> bool {
    let mut pts_out: i32 = 0;

    // Get next AVC access unit.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_GET_AVC_AU_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                VIDEO_STREAM,
                (&raw mut VIDEO_AU).cast::<u8>(),
                &mut pts_out,
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        // Log only periodically to avoid flooding.
        let count = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
        if count < 3 {
            log_i32(b"[VIDEO] GetAvcAu=", ret);
        }
        return false;
    }

    unsafe {
        LAST_VIDEO_PTS = pts_out as u32;
    }

    // Decode AVC frame.
    let mut got_frame: i32 = 0;
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_AVC_DECODE_FN) {
            // The output pointer (4th param) receives the decoded YCrCb data pointer.
            let mut ycrcb_ptr: *mut u8 = core::ptr::null_mut();
            let r = f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                (&raw mut VIDEO_AU).cast::<u8>(),
                DECODE_W as i32, // Frame width
                &mut ycrcb_ptr as *mut *mut u8 as *mut u8,
                &mut got_frame,
            );
            if r >= 0 && got_frame != 0 && !ycrcb_ptr.is_null() {
                // Log first successful decode.
                let count = DECODE_FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
                if count == 0 {
                    log_hex(b"[VIDEO] 1st frame ycrcb=", ycrcb_ptr as u32);
                }
                // Convert YCrCb to RGB via CscVme or software, then downscale.
                convert_ycrcb_to_rgb(ycrcb_ptr);
            } else if r >= 0 && got_frame == 0 {
                let count = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
                if count < 3 {
                    crate::debug_log(b"[VIDEO] AvcDecode: no frame yet");
                }
            }
            r
        } else {
            -1
        }
    };

    if ret < 0 {
        let count = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
        if count < 5 {
            log_i32(b"[VIDEO] AvcDecode=", ret);
        }
    }

    ret >= 0 && got_frame != 0
}

/// Convert YCrCb data to RGB and downscale to PIP.
///
/// Two paths:
/// - Hardware: sceMpegBaseCscVme → full 480x272 ABGR8888 → downscale
/// - Software: I420 planar YCbCr → direct downscale+convert to 160x90
unsafe fn convert_ycrcb_to_rgb(ycrcb: *mut u8) {
    let idx = FRAME_INDEX.load(Ordering::Relaxed);
    // Write to the back buffer (opposite of what display hook reads).
    let dst = if idx == 0 {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_B) }
    } else {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_A) }
    };
    if dst.is_null() {
        return;
    }

    if USE_SW_CSC.load(Ordering::Relaxed) != 0 {
        // Software YCbCr→RGB with integrated downscaling.
        unsafe { convert_ycrcb_sw(ycrcb, dst as *mut u32) };
    } else {
        // Hardware CSC path.
        let csc_buf = unsafe { core::ptr::read_volatile(&raw const CSC_BUF) };
        if csc_buf.is_null() {
            crate::debug_log(b"[VIDEO] CSC_BUF null!");
            return;
        }

        // CscVme converts full 480x272 YCrCb to ABGR8888.
        let csc_ok = unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_BASE_CSC_VME_FN) {
                let mut params: [i32; 8] = [0, 0, DECODE_W as i32, DECODE_H as i32, 0, 0, 0, 0];
                let r = f(csc_buf, ycrcb, DECODE_W as i32, params.as_mut_ptr());
                let count = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
                if count <= 1 {
                    log_i32(b"[VIDEO] CscVme=", r);
                }
                r >= 0
            } else {
                false
            }
        };

        if !csc_ok {
            return;
        }

        // Downscale 480x272 → 160x90 using nearest-neighbor sampling.
        let src = csc_buf as *const u32;
        let dst = dst as *mut u32;
        unsafe {
            let mut py = 0u32;
            while py < PIP_H {
                let src_y = py * DECODE_H / PIP_H;
                let mut px = 0u32;
                while px < PIP_W {
                    let src_x = px * DECODE_W / PIP_W;
                    let pixel = *src.add((src_y * DECODE_W + src_x) as usize);
                    *dst.add((py * PIP_W + px) as usize) = pixel;
                    px += 1;
                }
                py += 1;
            }
        }
    }

    // Flip double buffer index.
    let new_idx = if idx == 0 { 1 } else { 0 };
    FRAME_INDEX.store(new_idx, Ordering::Relaxed);
}

/// Software YCbCr (I420 planar) → ABGR8888 conversion with integrated
/// nearest-neighbor downscaling from 480x272 to 160x90.
///
/// Assumes the ME output is standard I420 planar:
/// - Y  plane: offset 0, stride = DECODE_W, size = DECODE_W * DECODE_H
/// - Cb plane: after Y, stride = DECODE_W/2, size = DECODE_W/2 * DECODE_H/2
/// - Cr plane: after Cb, stride = DECODE_W/2, size = DECODE_W/2 * DECODE_H/2
///
/// Fixed-point YCbCr→RGB (BT.601):
///   R = Y + 1.402*(Cr-128) ≈ Y + (359*(Cr-128)) >> 8
///   G = Y - 0.344*(Cb-128) - 0.714*(Cr-128) ≈ Y - (88*(Cb-128) + 183*(Cr-128)) >> 8
///   B = Y + 1.772*(Cb-128) ≈ Y + (454*(Cb-128)) >> 8
///
/// Only converts the ~14400 pixels needed for PIP, not the full frame.
unsafe fn convert_ycrcb_sw(ycrcb: *mut u8, dst: *mut u32) {
    let y_plane = ycrcb;
    // SAFETY: Pointer arithmetic within the ME decode buffer (I420 layout).
    let cb_plane = unsafe { ycrcb.add((DECODE_W * DECODE_H) as usize) };
    let cr_plane = unsafe { cb_plane.add(((DECODE_W / 2) * (DECODE_H / 2)) as usize) };
    let y_stride = DECODE_W as usize;
    let c_stride = (DECODE_W / 2) as usize;

    let count = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
    if count == 1 {
        crate::debug_log(b"[VIDEO] SW CSC: 1st frame converting");
    }

    unsafe {
        let mut py = 0u32;
        while py < PIP_H {
            let src_y = (py * DECODE_H / PIP_H) as usize;
            let c_y = src_y / 2;
            let mut px = 0u32;
            while px < PIP_W {
                let src_x = (px * DECODE_W / PIP_W) as usize;
                let c_x = src_x / 2;

                let y_val = *y_plane.add(src_y * y_stride + src_x) as i32;
                let cb_val = *cb_plane.add(c_y * c_stride + c_x) as i32;
                let cr_val = *cr_plane.add(c_y * c_stride + c_x) as i32;

                let cb_off = cb_val - 128;
                let cr_off = cr_val - 128;

                let mut r = y_val + ((359 * cr_off) >> 8);
                let mut g = y_val - ((88 * cb_off + 183 * cr_off) >> 8);
                let mut b = y_val + ((454 * cb_off) >> 8);

                // Clamp to 0-255.
                if r < 0 { r = 0; } else if r > 255 { r = 255; }
                if g < 0 { g = 0; } else if g > 255 { g = 255; }
                if b < 0 { b = 0; } else if b > 255 { b = 255; }

                // ABGR8888: A=0xFF, B, G, R (PSP pixel format).
                let pixel = 0xFF00_0000
                    | ((b as u32) << 16)
                    | ((g as u32) << 8)
                    | (r as u32);
                *dst.add((py * PIP_W + px) as usize) = pixel;

                px += 1;
            }
            py += 1;
        }
    }
}

/// Decode one ATRAC audio frame and output to audio channel.
unsafe fn decode_audio_frame() {
    if unsafe { AUDIO_STREAM.is_null() || AUDIO_CH_HANDLE < 0 } {
        return;
    }

    let mut pts_out: i32 = 0;

    // Get next ATRAC access unit.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_GET_ATRAC_AU_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                AUDIO_STREAM,
                (&raw mut AUDIO_AU).cast::<u8>(),
                &mut pts_out,
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        return;
    }

    unsafe {
        LAST_AUDIO_PTS = pts_out as u32;
    }

    // Decode ATRAC to PCM.
    let pcm = unsafe { core::ptr::read_volatile(&raw const PCM_BUF) };
    if pcm.is_null() {
        return;
    }
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_ATRAC_DECODE_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                (&raw mut AUDIO_AU).cast::<u8>(),
                pcm,
                0, // Padding
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        return;
    }

    // Output PCM to audio channel.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const VID_AUDIO_OUTPUT_FN) {
            f(AUDIO_CH_HANDLE, 0x8000, pcm); // 0x8000 = max volume
        }
    }
}

/// Check A/V sync every 60 seconds. Skip or hold frames if drift > 1 second.
unsafe fn check_av_sync() {
    let now = unsafe {
        // Use sceKernelGetSystemTimeLow for monotonic microseconds.
        psp::sys::sceKernelGetSystemTimeLow()
    };

    let last = unsafe { LAST_SYNC_TIME };
    // 60 seconds = 60_000_000 microseconds.
    if last != 0 && now.wrapping_sub(last) < 60_000_000 {
        return;
    }
    unsafe {
        LAST_SYNC_TIME = now;
    }

    let v_pts = unsafe { LAST_VIDEO_PTS };
    let a_pts = unsafe { LAST_AUDIO_PTS };

    if v_pts == 0 || a_pts == 0 {
        return;
    }

    // PTS is in 90kHz ticks. 1 second = 90000 ticks.
    let diff = if v_pts > a_pts {
        v_pts - a_pts
    } else {
        a_pts - v_pts
    };

    if diff > 90000 {
        // Drift > 1 second, flush streams to resync.
        crate::debug_log(b"[VIDEO] A/V drift >1s, flushing");
        unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_FLUSH_ALL_STREAM_FN) {
                f((&raw mut MPEG_HANDLE).cast::<u32>());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Video thread
// ---------------------------------------------------------------------------

/// Whether the video subsystem has been initialized (NIDs + scan done).
static INIT_DONE: AtomicU8 = AtomicU8::new(0);

/// Video thread entry point.
///
/// Started at boot from psp_main() (where kernel syscalls work).
/// Idles until the first VIDEO_CMD arrives, then initializes sceMpeg
/// NIDs, scans for video files, and enters the decode loop.
///
/// We CANNOT create this thread from the display hook context because
/// no kernel syscalls (sceKernelCreateThread, sceIoOpen, etc.) work
/// in the sceDisplaySetFrameBuf hook callback.
unsafe extern "C" fn video_thread_entry(_args: usize, _argp: *mut core::ffi::c_void) -> i32 {
    crate::debug_log(b"[VIDEO] thread started, waiting for cmd...");

    // Idle loop: wait for first PIP command before doing any heavy init.
    // This avoids loading AV modules at boot which can conflict with games.
    loop {
        let cmd = VIDEO_CMD.load(Ordering::Relaxed);
        if cmd != 0 {
            crate::debug_log(b"[VIDEO] first cmd received, initializing...");
            // Don't consume the command yet -- let the main loop handle it.
            break;
        }
        unsafe {
            psp::sys::sceKernelDelayThread(100_000); // 100ms idle poll
        }
    }

    // Wait for game to be fully running before touching AV modules.
    crate::debug_log(b"[VIDEO] waiting 1s for game stability...");
    unsafe {
        psp::sys::sceKernelDelayThread(1_000_000);
    }

    // Resolve sceMpeg NIDs.
    crate::debug_log(b"[VIDEO] resolving NIDs (attempt 1)...");
    let resolved = unsafe { try_resolve_mpeg() };
    if !resolved {
        // Retry once after a longer delay.
        crate::debug_log(b"[VIDEO] NID retry in 3s...");
        unsafe {
            psp::sys::sceKernelDelayThread(3_000_000);
        }
        crate::debug_log(b"[VIDEO] resolving NIDs (attempt 2)...");
        let resolved2 = unsafe { try_resolve_mpeg() };
        if !resolved2 {
            crate::debug_log(b"[VIDEO] NIDs failed, thread stays for cmds");
        }
    }

    // Scan for video files.
    unsafe {
        scan_video_dir();
    }

    // Log thread ready state.
    let avail = VIDEO_AVAILABLE.load(Ordering::Relaxed);
    let count = unsafe { VIDEO_COUNT };
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[VIDEO] ready: avail=");
    p = write_decimal(&mut buf, p, avail as u32);
    p = copy_bytes(&mut buf, p, b" vids=");
    p = write_decimal(&mut buf, p, count as u32);
    crate::debug_log(&buf[..p]);

    INIT_DONE.store(1, Ordering::Release);

    // Main decode loop.
    loop {
        // Check for commands.
        let cmd = VIDEO_CMD.load(Ordering::Relaxed);
        if cmd != 0 {
            VIDEO_CMD.store(0, Ordering::Relaxed);

            // Log command.
            let mut cbuf = [0u8; 32];
            let cp = copy_bytes(&mut cbuf, 0, b"[VIDEO] CMD=");
            let cp = write_decimal(&mut cbuf, cp, cmd as u32);
            crate::debug_log(&cbuf[..cp]);

            match cmd {
                1 => {
                    // Toggle PIP.
                    if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
                        stop_playback();
                    } else {
                        // If no videos, try rescanning first.
                        if unsafe { VIDEO_COUNT } == 0 {
                            crate::debug_log(b"[VIDEO] rescan on toggle...");
                            unsafe { scan_video_dir() };
                        }
                        start_playback();
                    }
                },
                2 => {
                    // Next video.
                    let was_active = PIP_ACTIVE.load(Ordering::Relaxed) != 0;
                    if was_active {
                        stop_playback();
                    }
                    unsafe {
                        if VIDEO_COUNT > 0 {
                            CURRENT_VIDEO = (CURRENT_VIDEO + 1) % VIDEO_COUNT;
                        }
                    }
                    if was_active {
                        start_playback();
                    }
                },
                3 => {
                    // Stop.
                    stop_playback();
                },
                _ => {},
            }
        }

        // If PIP active, decode frames.
        if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
            // Fill ringbuffer with file data.
            let rb_ret = unsafe { fill_ringbuffer() };

            // Log ringbuffer fill results periodically.
            let fcount = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
            if fcount < 3 {
                log_i32(b"[VIDEO] ringbuf put=", rb_ret);
            }

            // Decode video frame.
            let got_frame = unsafe { decode_video_frame() };

            // Decode audio frame.
            unsafe {
                decode_audio_frame();
            }

            // A/V sync check.
            unsafe {
                check_av_sync();
            }

            if !got_frame {
                // Check if we've decoded any frames at all.
                let total = DECODE_FRAME_COUNT.load(Ordering::Relaxed);
                if total == 0 {
                    // Never got a frame -- might be wrong format or broken file.
                    crate::debug_log(b"[VIDEO] no frames decoded, stopping");
                    stop_playback();
                    overlay::show_osd(b"PIP: bad video format");
                } else {
                    // End of stream -- loop or advance.
                    log_u32(b"[VIDEO] stream end, frames=", total);
                    stop_playback();
                    unsafe {
                        if VIDEO_COUNT > 1 {
                            CURRENT_VIDEO = (CURRENT_VIDEO + 1) % VIDEO_COUNT;
                            psp::sys::sceKernelDelayThread(100_000);
                        }
                    }
                    start_playback();
                }
            }

            // Frame pacing: ~33ms for 30fps video.
            unsafe {
                psp::sys::sceKernelDelayThread(33_000);
            }
        } else {
            // Idle -- sleep longer.
            unsafe {
                psp::sys::sceKernelDelayThread(100_000);
            }
        }
    }
}

/// Start video playback of the current video.
fn start_playback() {
    if VIDEO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Video: not available");
        crate::debug_log(b"[VIDEO] start: not available");
        return;
    }

    // Allocate PIP frame buffers on demand.
    if !unsafe { alloc_pip_buffers() } {
        overlay::show_osd(b"PIP: out of memory");
        return;
    }

    let idx = unsafe { CURRENT_VIDEO };
    let count = unsafe { VIDEO_COUNT };
    if idx >= count || count == 0 {
        overlay::show_osd(b"No videos in ms0:/VIDEO/");
        crate::debug_log(b"[VIDEO] start: no videos");
        return;
    }

    let filepath_ptr = unsafe {
        (&raw const VIDEO_LIST)
            .cast::<[u8; MAX_FILENAME]>()
            .add(idx)
    };
    let filepath = unsafe { &*filepath_ptr };

    // Set display name for OSD.
    unsafe {
        set_video_name(filepath);
    }

    // Initialize decoder.
    let ok = unsafe { init_mpeg_decoder(filepath) };
    if !ok {
        overlay::show_osd(b"Video init failed");
        crate::debug_log(b"[VIDEO] start: decoder init failed");
        return;
    }

    // Pause background MP3 audio so ATRAC can play.
    crate::audio::pause_for_video();

    PIP_ACTIVE.store(1, Ordering::Relaxed);
    crate::debug_log(b"[VIDEO] PIP ACTIVE");

    // Show OSD.
    let mut buf = [0u8; 48];
    let p = copy_bytes(&mut buf, 0, b"PIP: ");
    let name_len = video_name_len();
    let name_slice = unsafe {
        core::slice::from_raw_parts((&raw const VIDEO_NAME).cast::<u8>(), name_len)
    };
    let p = copy_bytes(&mut buf, p, name_slice);
    overlay::show_osd(&buf[..p]);
}

/// Stop video playback and clean up.
fn stop_playback() {
    if PIP_ACTIVE.load(Ordering::Relaxed) == 0 {
        return; // Already stopped.
    }
    PIP_ACTIVE.store(0, Ordering::Relaxed);
    crate::debug_log(b"[VIDEO] stopping playback...");

    // Clean up decoder (frees MPEG buffers via MPEG_BLOCK_ID).
    unsafe {
        cleanup_mpeg();
    }

    // Free PIP frame buffers (returns ~650KB to user partition).
    unsafe {
        free_pip_buffers();
    }

    // Resume background MP3 audio.
    crate::audio::resume_after_video();

    overlay::show_osd(b"PIP stopped");
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start the video thread. Must be called from psp_main() or another
/// context where kernel syscalls work (NOT the display hook).
///
/// The thread idles until it receives its first VIDEO_CMD, then
/// initializes sceMpeg and enters the decode loop.
pub fn start_video_thread() {
    crate::debug_log(b"[VIDEO] creating video thread...");
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisVideo\0".as_ptr(),
            video_thread_entry,
            0x1A, // Priority 26 (below audio at 24).
            0x4000, // 16KB stack.
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            log_i32(b"[VIDEO] thread id=", thid.0);
        } else {
            log_i32(b"[VIDEO] thread create FAILED=", thid.0);
        }
    }
}

/// Toggle PIP on/off. Safe to call from display hook context
/// (only touches atomics, no syscalls).
pub fn toggle_pip() {
    VIDEO_CMD.store(1, Ordering::Relaxed);
}

/// Advance to next video. Safe to call from display hook context.
pub fn next_video() {
    VIDEO_CMD.store(2, Ordering::Relaxed);
}

/// Check if PIP is currently active.
pub fn is_pip_active() -> bool {
    PIP_ACTIVE.load(Ordering::Relaxed) != 0
}

/// Get a pointer to the current display frame (front buffer).
/// Returns (ptr, width, height). ptr may be null if buffers not allocated.
pub fn pip_frame() -> (*const u8, u32, u32) {
    let idx = FRAME_INDEX.load(Ordering::Relaxed);
    // SAFETY: PIP_FRAME_A/B are either null or valid allocated pointers.
    let ptr = if idx == 0 {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_A) }
    } else {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_B) }
    };
    (ptr, PIP_W, PIP_H)
}

/// PIP window position and size for overlay rendering.
pub const fn pip_rect() -> (u32, u32, u32, u32) {
    (PIP_X, PIP_Y, PIP_W, PIP_H)
}

/// PIP border width.
pub const fn pip_border() -> u32 {
    PIP_BORDER
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract filename from full path and set VIDEO_NAME.
unsafe fn set_video_name(path: &[u8]) {
    // Find last '/'.
    let mut last_slash = 0;
    let mut i = 0;
    while i < path.len() && path[i] != 0 {
        if path[i] == b'/' {
            last_slash = i + 1;
        }
        i += 1;
    }

    let name = &path[last_slash..i];
    let len = name.len().min(47);
    unsafe {
        let name_ptr = (&raw mut VIDEO_NAME).cast::<u8>();
        let mut j = 0;
        while j < len {
            *name_ptr.add(j) = name[j];
            j += 1;
        }
        *name_ptr.add(len) = 0;
    }
}

/// Get VIDEO_NAME length (up to null terminator).
fn video_name_len() -> usize {
    let name_ptr = (&raw const VIDEO_NAME).cast::<u8>();
    let mut len = 0;
    while len < 48 {
        if unsafe { *name_ptr.add(len) } == 0 {
            break;
        }
        len += 1;
    }
    len
}

fn copy_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    for &b in s {
        if p >= buf.len() || b == 0 {
            break;
        }
        buf[p] = b;
        p += 1;
    }
    p
}

fn log_i32(prefix: &[u8], val: i32) {
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, prefix);
    if val < 0 {
        if p < buf.len() {
            buf[p] = b'-';
            p += 1;
        }
        p = write_decimal(&mut buf, p, (-(val as i64)) as u32);
    } else {
        p = write_decimal(&mut buf, p, val as u32);
    }
    crate::debug_log(&buf[..p]);
}

fn log_u32(prefix: &[u8], val: u32) {
    let mut buf = [0u8; 48];
    let p = copy_bytes(&mut buf, 0, prefix);
    let p = write_decimal(&mut buf, p, val);
    crate::debug_log(&buf[..p]);
}

fn log_usize(prefix: &[u8], val: usize) {
    let mut buf = [0u8; 48];
    let p = copy_bytes(&mut buf, 0, prefix);
    let p = write_decimal(&mut buf, p, val as u32);
    crate::debug_log(&buf[..p]);
}

fn log_hex(prefix: &[u8], val: u32) {
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, prefix);
    p = copy_bytes(&mut buf, p, b"0x");
    p = write_hex32(&mut buf, p, val);
    crate::debug_log(&buf[..p]);
}

/// Log a NID resolution failure with the NID value in hex.
fn log_nid_fail(name: &[u8], nid: u32) {
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[VIDEO] ");
    p = copy_bytes(&mut buf, p, name);
    p = copy_bytes(&mut buf, p, b" FAIL ");
    p = write_hex32(&mut buf, p, nid);
    crate::debug_log(&buf[..p]);
}

/// Log a NID resolution success with the function address in hex.
fn log_nid_ok(name: &[u8], addr: u32) {
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[VIDEO] ");
    p = copy_bytes(&mut buf, p, name);
    p = copy_bytes(&mut buf, p, b" OK @");
    p = write_hex32(&mut buf, p, addr);
    crate::debug_log(&buf[..p]);
}

fn write_decimal(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            return pos + 1;
        }
        return pos;
    }
    let mut digits = [0u8; 10];
    let mut n = val;
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    let mut p = pos;
    while count > 0 {
        count -= 1;
        if p >= buf.len() {
            break;
        }
        buf[p] = digits[count];
        p += 1;
    }
    p
}

fn write_hex32(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut p = pos;
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

fn write_hex_byte(buf: &mut [u8], pos: usize, val: u8) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut p = pos;
    if p + 1 < buf.len() {
        buf[p] = hex[(val >> 4) as usize];
        buf[p + 1] = hex[(val & 0xF) as usize];
        p += 2;
    }
    p
}
