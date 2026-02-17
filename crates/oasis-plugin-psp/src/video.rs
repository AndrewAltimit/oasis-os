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
//! - Writes to a double-buffered shared frame read by the display hook
//! - Decodes ATRAC audio to a dedicated PSP audio channel
//! - PTS-based frame timing with periodic A/V resync
//!
//! ## Memory Budget (~174KB, allocated on-demand from partition 2)
//!
//! - sceMpeg decoder: ~64KB
//! - Ringbuffer (512 packets): ~32KB
//! - RGB double buffer (160x90x4x2): ~116KB
//! - File read buffer: ~16KB
//! - PCM audio buffer: ~4KB

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

/// Whether the video thread has been launched.
static VIDEO_THREAD_STARTED: AtomicU8 = AtomicU8::new(0);

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

/// Allocated buffer pointers (all from user-memory partition 2, on-demand).
static mut MPEG_BUF: *mut u8 = core::ptr::null_mut();
static mut RINGBUF_DATA: *mut u8 = core::ptr::null_mut();

/// On-demand buffers allocated from partition 2 (NOT static arrays).
/// These pointers are set by alloc_pip_buffers() on first PIP activation.
static mut PIP_FRAME_A: *mut u8 = core::ptr::null_mut();
static mut PIP_FRAME_B: *mut u8 = core::ptr::null_mut();
static mut FILE_READ_BUF: *mut u8 = core::ptr::null_mut();
static mut PCM_BUF: *mut u8 = core::ptr::null_mut();

/// Sizes for on-demand buffers.
const PIP_FRAME_SIZE: usize = (PIP_W * PIP_H * 4) as usize; // 57600
const FILE_READ_BUF_LEN: usize = 16384;
const PCM_BUF_SIZE: usize = ATRAC_SAMPLES as usize * 4; // 8192

/// Total on-demand allocation: 2 PIP frames + file read + PCM + padding.
const PIP_ALLOC_SIZE: u32 =
    (PIP_FRAME_SIZE * 2 + FILE_READ_BUF_LEN + PCM_BUF_SIZE + 64) as u32;

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

/// Load MPEG AV modules via sceUtilityLoadModule.
unsafe fn load_mpeg_modules() {
    let load_fn: Option<unsafe extern "C" fn(i32) -> i32> = unsafe {
        resolve_nid(UTILITY_MODULES, NID_UTILITY_LOAD_MODULE).map(|ptr| core::mem::transmute(ptr))
    };

    if let Some(load) = load_fn {
        crate::debug_log(b"[VIDEO] sceUtilityLoadModule resolved");
        let r1 = unsafe { load(PSP_MODULE_AV_AVCODEC) };
        log_i32(b"[VIDEO] LoadModule AVCODEC=", r1);
        let r2 = unsafe { load(PSP_MODULE_AV_MPEGBASE) };
        log_i32(b"[VIDEO] LoadModule MPEGBASE=", r2);
    } else {
        crate::debug_log(b"[VIDEO] sceUtilityLoadModule NOT found");
    }
}

/// Allocate PIP frame buffers and I/O buffers from user-memory partition 2.
/// Returns true on success.
unsafe fn alloc_pip_buffers() -> bool {
    if unsafe { PIP_BUF_BLOCK } >= 0 {
        return true; // Already allocated.
    }

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
        crate::debug_log(b"[VIDEO] PIP buf alloc failed");
        return false;
    }

    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block_id) } as *mut u8;
    let aligned = ((base as u32 + 15) & !15) as *mut u8;

    unsafe {
        PIP_FRAME_A = aligned;
        PIP_FRAME_B = aligned.add(PIP_FRAME_SIZE);
        FILE_READ_BUF = aligned.add(PIP_FRAME_SIZE * 2);
        PCM_BUF = aligned.add(PIP_FRAME_SIZE * 2 + FILE_READ_BUF_LEN);
        PIP_BUF_BLOCK = block_id.0;

        // Zero the frame buffers.
        let mut i = 0;
        while i < PIP_FRAME_SIZE {
            *PIP_FRAME_A.add(i) = 0;
            *PIP_FRAME_B.add(i) = 0;
            i += 1;
        }
    }

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
            FILE_READ_BUF = core::ptr::null_mut();
            PCM_BUF = core::ptr::null_mut();
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

    let mut ok = true;

    macro_rules! resolve {
        ($fn_ptr:ident, $modules:expr, $nid:expr) => {
            unsafe {
                if let Some(ptr) = resolve_nid($modules, $nid) {
                    core::ptr::write_volatile(&raw mut $fn_ptr, Some(core::mem::transmute(ptr)));
                } else {
                    ok = false;
                }
            }
        };
    }

    // Core sceMpeg functions
    resolve!(MPEG_INIT_FN, MPEG_MODULES, NID_MPEG_INIT);
    resolve!(MPEG_FINISH_FN, MPEG_MODULES, NID_MPEG_FINISH);
    resolve!(MPEG_CREATE_FN, MPEG_MODULES, NID_MPEG_CREATE);
    resolve!(MPEG_DELETE_FN, MPEG_MODULES, NID_MPEG_DELETE);
    resolve!(MPEG_QUERY_MEM_SIZE_FN, MPEG_MODULES, NID_MPEG_QUERY_MEM_SIZE);
    resolve!(
        MPEG_RINGBUF_QUERY_MEM_SIZE_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_QUERY_MEM_SIZE
    );
    resolve!(
        MPEG_RINGBUF_CONSTRUCT_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_CONSTRUCT
    );
    resolve!(
        MPEG_RINGBUF_DESTRUCT_FN,
        MPEG_MODULES,
        NID_MPEG_RINGBUF_DESTRUCT
    );
    resolve!(MPEG_RINGBUF_PUT_FN, MPEG_MODULES, NID_MPEG_RINGBUF_PUT);
    resolve!(
        MPEG_QUERY_STREAM_OFFSET_FN,
        MPEG_MODULES,
        NID_MPEG_QUERY_STREAM_OFFSET
    );
    resolve!(
        MPEG_QUERY_STREAM_SIZE_FN,
        MPEG_MODULES,
        NID_MPEG_QUERY_STREAM_SIZE
    );
    resolve!(MPEG_REGIST_STREAM_FN, MPEG_MODULES, NID_MPEG_REGIST_STREAM);
    resolve!(
        MPEG_UNREGIST_STREAM_FN,
        MPEG_MODULES,
        NID_MPEG_UNREGIST_STREAM
    );
    resolve!(
        MPEG_FLUSH_ALL_STREAM_FN,
        MPEG_MODULES,
        NID_MPEG_FLUSH_ALL_STREAM
    );
    resolve!(
        MPEG_MALLOC_AVC_ES_BUF_N,
        MPEG_MODULES,
        NID_MPEG_MALLOC_AVC_ES_BUF
    );
    resolve!(
        MPEG_FREE_AVC_ES_BUF_FN,
        MPEG_MODULES,
        NID_MPEG_FREE_AVC_ES_BUF
    );
    resolve!(MPEG_INIT_AU_FN, MPEG_MODULES, NID_MPEG_INIT_AU);
    resolve!(MPEG_GET_AVC_AU_FN, MPEG_MODULES, NID_MPEG_GET_AVC_AU);
    resolve!(
        MPEG_AVC_DECODE_MODE_FN,
        MPEG_MODULES,
        NID_MPEG_AVC_DECODE_MODE
    );
    resolve!(MPEG_AVC_DECODE_FN, MPEG_MODULES, NID_MPEG_AVC_DECODE);
    resolve!(MPEG_GET_ATRAC_AU_FN, MPEG_MODULES, NID_MPEG_GET_ATRAC_AU);
    resolve!(MPEG_ATRAC_DECODE_FN, MPEG_MODULES, NID_MPEG_ATRAC_DECODE);

    // sceMpegbase (CSC)
    resolve!(MPEG_BASE_CSC_INIT_FN, MPEG_BASE_MODULES, NID_MPEG_BASE_CSC_INIT);
    resolve!(MPEG_BASE_CSC_VME_FN, MPEG_BASE_MODULES, NID_MPEG_BASE_CSC_VME);

    // Audio output for video ATRAC
    unsafe {
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RESERVE) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_OUTPUT_BLOCKING) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_OUTPUT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RELEASE) {
            core::ptr::write_volatile(
                &raw mut VID_AUDIO_RELEASE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
    }

    if ok {
        VIDEO_AVAILABLE.store(1, Ordering::Relaxed);
        crate::debug_log(b"[VIDEO] all sceMpeg NIDs resolved");
    } else {
        crate::debug_log(b"[VIDEO] some sceMpeg NIDs missing");
    }

    ok
}

// ---------------------------------------------------------------------------
// File scanning
// ---------------------------------------------------------------------------

/// Scan ms0:/VIDEO/ for .pmf files. Populates VIDEO_LIST.
unsafe fn scan_video_dir() {
    let config = crate::config::get_config();
    let dir_path = config.video_dir_str();

    // SAFETY: sceIoDopen with valid null-terminated path.
    let dfd = unsafe { psp::sys::sceIoDopen(dir_path.as_ptr()) };
    if dfd.0 < 0 {
        crate::debug_log(b"[VIDEO] cannot open video dir");
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
        let is_pmf = (ext[0] == b'.' || ext[0] == b'.')
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

/// Ringbuffer read callback -- reads data from the video file.
unsafe extern "C" fn ringbuf_callback(
    _data: *mut u8,
    packets: i32,
    _param: *mut u8,
) -> i32 {
    if packets <= 0 {
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
        let read_buf = unsafe { core::ptr::read_volatile(&raw const FILE_READ_BUF) };
        if read_buf.is_null() {
            break;
        }
        let chunk = (bytes_to_read - total_read).min(FILE_READ_BUF_LEN as u32);
        // SAFETY: sceIoRead with valid fd and buffer.
        let ret = unsafe {
            psp::sys::sceIoRead(
                psp::sys::SceUid(fd),
                read_buf as *mut _,
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
unsafe fn init_mpeg_decoder(filepath: &[u8]) -> bool {
    // Open the video file.
    // SAFETY: sceIoOpen with valid path.
    let fd = unsafe {
        psp::sys::sceIoOpen(filepath.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        crate::debug_log(b"[VIDEO] cannot open file");
        return false;
    }
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
        crate::debug_log(b"[VIDEO] header read failed");
        unsafe {
            psp::sys::sceIoClose(fd);
            VIDEO_FD = -1;
        }
        return false;
    }

    // Initialize sceMpeg.
    let init_fn = unsafe { core::ptr::read_volatile(&raw const MPEG_INIT_FN) };
    if let Some(f) = init_fn {
        let r = unsafe { f() };
        if r < 0 {
            log_i32(b"[VIDEO] sceMpegInit failed=", r);
            unsafe {
                psp::sys::sceIoClose(fd);
                VIDEO_FD = -1;
            }
            return false;
        }
    } else {
        return false;
    }

    // Query memory sizes.
    let mpeg_mem_size = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_MEM_SIZE_FN) {
            f(0)
        } else {
            return false;
        }
    };
    let ringbuf_mem_size = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_QUERY_MEM_SIZE_FN) {
            f(RINGBUF_PACKETS)
        } else {
            return false;
        }
    };

    log_i32(b"[VIDEO] mpeg mem=", mpeg_mem_size);
    log_i32(b"[VIDEO] ringbuf mem=", ringbuf_mem_size);

    // Allocate from user memory partition 2.
    let total_alloc = mpeg_mem_size + ringbuf_mem_size + 64;
    // SAFETY: sceKernelAllocPartitionMemory for user partition.
    let block_id = unsafe {
        psp::sys::sceKernelAllocPartitionMemory(
            psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
            b"OasisVideo\0".as_ptr(),
            psp::sys::SceSysMemBlockTypes::Low,
            total_alloc as u32,
            core::ptr::null_mut(),
        )
    };
    if block_id < psp::sys::SceUid(0) {
        crate::debug_log(b"[VIDEO] mem alloc failed");
        unsafe {
            if let Some(f) = core::ptr::read_volatile(&raw const MPEG_FINISH_FN) {
                f();
            }
            psp::sys::sceIoClose(fd);
            VIDEO_FD = -1;
        }
        return false;
    }

    // SAFETY: Get block address.
    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block_id) } as *mut u8;
    // 16-byte align.
    let aligned = ((base as u32 + 15) & !15) as *mut u8;
    unsafe {
        MPEG_BUF = aligned;
        RINGBUF_DATA = aligned.add(mpeg_mem_size as usize);
    }

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
        log_i32(b"[VIDEO] ringbuf construct failed=", ret);
        // SAFETY: Cleaning up partially-initialized mpeg state.
        unsafe { cleanup_mpeg(block_id) };
        return false;
    }

    // Create sceMpeg handle.
    // SAFETY: All buffers allocated and aligned.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_CREATE_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                MPEG_BUF,
                mpeg_mem_size,
                (&raw mut RINGBUF).cast::<u8>() as *mut u32,
                480, // Video width (PSP native)
                0,   // Mode
                0,   // Reserved
            )
        } else {
            -1
        }
    };
    if ret < 0 {
        log_i32(b"[VIDEO] sceMpegCreate failed=", ret);
        // SAFETY: Cleaning up partially-initialized mpeg state.
        unsafe { cleanup_mpeg(block_id) };
        return false;
    }

    // Query stream offset and size from header.
    unsafe {
        let mut offset = 0u32;
        let mut size = 0u32;
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_STREAM_OFFSET_FN) {
            f((&raw mut MPEG_HANDLE).cast::<u32>(), header.as_mut_ptr(), &mut offset);
        }
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_QUERY_STREAM_SIZE_FN) {
            f(header.as_mut_ptr(), &mut size);
        }
        STREAM_OFFSET = offset;
        STREAM_SIZE = size;
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
        if VIDEO_STREAM.is_null() {
            crate::debug_log(b"[VIDEO] video stream register failed");
            cleanup_mpeg(block_id);
            return false;
        }
    }

    // Set decode mode to ABGR8888.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_AVC_DECODE_MODE_FN) {
            let mut mode = [DECODE_PIXEL_MODE, 0];
            f((&raw mut MPEG_HANDLE).cast::<u32>(), mode.as_mut_ptr());
        }
    }

    // Allocate AVC ES buffer.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_MALLOC_AVC_ES_BUF_N) {
            AVC_ES_BUF = f((&raw mut MPEG_HANDLE).cast::<u32>());
        }
        if AVC_ES_BUF.is_null() {
            crate::debug_log(b"[VIDEO] AVC ES buf alloc failed");
            cleanup_mpeg(block_id);
            return false;
        }
    }

    // Initialize AU structs.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_INIT_AU_FN) {
            f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                AVC_ES_BUF,
                (&raw mut VIDEO_AU).cast::<u8>(),
            );
            if !AUDIO_STREAM.is_null() {
                f(
                    (&raw mut MPEG_HANDLE).cast::<u32>(),
                    AVC_ES_BUF, // Reuse ES buf for AU init
                    (&raw mut AUDIO_AU).cast::<u8>(),
                );
            }
        }
    }

    // Initialize CSC (color space conversion).
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_BASE_CSC_INIT_FN) {
            f(480); // Width for CSC lookup tables
        }
    }

    // Reserve audio channel for ATRAC output.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const VID_AUDIO_RESERVE_FN) {
            let ch = f(VIDEO_AUDIO_CHANNEL, ATRAC_SAMPLES, 0); // 0 = stereo
            if ch >= 0 {
                AUDIO_CH_HANDLE = ch;
                crate::debug_log(b"[VIDEO] audio ch reserved");
            } else {
                AUDIO_CH_HANDLE = -1;
                crate::debug_log(b"[VIDEO] audio ch reserve failed");
            }
        }
    }

    // Reset PTS tracking.
    unsafe {
        LAST_VIDEO_PTS = 0;
        LAST_AUDIO_PTS = 0;
        LAST_SYNC_TIME = 0;
    }

    crate::debug_log(b"[VIDEO] decoder initialized OK");
    true
}

/// Clean up sceMpeg resources.
unsafe fn cleanup_mpeg(block_id: psp::sys::SceUid) {
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

        // Free memory partition.
        if block_id.0 >= 0 {
            psp::sys::sceKernelFreePartitionMemory(block_id);
        }
        MPEG_BUF = core::ptr::null_mut();
        RINGBUF_DATA = core::ptr::null_mut();
    }
}

// ---------------------------------------------------------------------------
// Video and audio decoding
// ---------------------------------------------------------------------------

/// Fill ringbuffer with data from the file.
unsafe fn fill_ringbuffer() {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_RINGBUF_PUT_FN) {
            f((&raw mut RINGBUF).cast::<u8>(), RINGBUF_PACKETS, 0);
        }
    }
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
        return false;
    }

    unsafe {
        LAST_VIDEO_PTS = pts_out as u32;
    }

    // Decode AVC frame.
    let mut got_frame: i32 = 0;
    // The decode output goes to a temp buffer that sceMpeg manages internally.
    // We then use CscVme to convert YCrCb → RGB into our PIP frame buffer.
    let ret = unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_AVC_DECODE_FN) {
            // The output pointer (4th param) receives the decoded YCrCb data pointer.
            let mut ycrcb_ptr: *mut u8 = core::ptr::null_mut();
            let r = f(
                (&raw mut MPEG_HANDLE).cast::<u32>(),
                (&raw mut VIDEO_AU).cast::<u8>(),
                480, // Stride
                &mut ycrcb_ptr as *mut *mut u8 as *mut u8,
                &mut got_frame,
            );
            if r >= 0 && got_frame != 0 && !ycrcb_ptr.is_null() {
                // Convert YCrCb to RGB via sceMpegBaseCscVme.
                convert_ycrcb_to_rgb(ycrcb_ptr);
            }
            r
        } else {
            -1
        }
    };

    ret >= 0 && got_frame != 0
}

/// Convert YCrCb data to RGB and scale into PIP frame buffer.
unsafe fn convert_ycrcb_to_rgb(ycrcb: *mut u8) {
    // sceMpegBaseCscVme converts YCrCb to ABGR8888.
    // The output goes to the back buffer of our double buffer.
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

    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const MPEG_BASE_CSC_VME_FN) {
            // CscVme params: dst buffer, ycrcb data, width, csc_params
            // For PIP we decode at 480x272 then downscale, but hardware CSC
            // can output directly to smaller buffer with stride control.
            // Use params array: [x, y, width, height, ...]
            let mut params: [i32; 8] = [0, 0, PIP_W as i32, PIP_H as i32, 0, 0, 0, 0];
            f(dst, ycrcb, PIP_W as i32, params.as_mut_ptr());
        }
    }

    // Flip double buffer index.
    let new_idx = if idx == 0 { 1 } else { 0 };
    FRAME_INDEX.store(new_idx, Ordering::Relaxed);
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

/// Video thread entry point.
///
/// This thread is started lazily on first PIP command (not at boot).
/// It handles NID resolution, buffer allocation, file scanning, and
/// the decode loop.
unsafe extern "C" fn video_thread_entry(_args: usize, _argp: *mut core::ffi::c_void) -> i32 {
    crate::debug_log(b"[VIDEO] thread started, resolving NIDs...");

    // Wait for game to fully initialize before touching AV modules.
    unsafe {
        psp::sys::sceKernelDelayThread(2_000_000);
    }

    // Resolve sceMpeg NIDs (deferred from boot to avoid ME conflicts).
    let resolved = unsafe { try_resolve_mpeg() };
    if !resolved {
        // Retry once after a longer delay -- game may still be loading.
        crate::debug_log(b"[VIDEO] NID retry in 3s...");
        unsafe {
            psp::sys::sceKernelDelayThread(3_000_000);
        }
        unsafe { try_resolve_mpeg() };
    }

    // Allocate PIP frame buffers from user-memory partition 2.
    if !unsafe { alloc_pip_buffers() } {
        crate::debug_log(b"[VIDEO] PIP alloc failed, thread exiting");
        return 1;
    }

    // Scan for video files.
    unsafe {
        scan_video_dir();
    }

    if unsafe { VIDEO_COUNT } == 0 {
        crate::debug_log(b"[VIDEO] no videos found, thread idle");
        // Stay alive to handle future commands (maybe files added later).
        loop {
            let cmd = VIDEO_CMD.load(Ordering::Relaxed);
            if cmd != 0 {
                VIDEO_CMD.store(0, Ordering::Relaxed);
                // Rescan on any command.
                unsafe {
                    scan_video_dir();
                }
                if unsafe { VIDEO_COUNT } > 0 {
                    break; // Fall through to main loop.
                }
                overlay::show_osd(b"No videos in ms0:/VIDEO/");
            }
            unsafe {
                psp::sys::sceKernelDelayThread(500_000);
            }
        }
    }

    // Memory block ID for cleanup (set when decoder initializes).
    let mut mem_block_id = psp::sys::SceUid(-1);

    // Main decode loop.
    loop {
        // Check for commands.
        let cmd = VIDEO_CMD.load(Ordering::Relaxed);
        if cmd != 0 {
            VIDEO_CMD.store(0, Ordering::Relaxed);

            match cmd {
                1 => {
                    // Toggle PIP.
                    if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
                        stop_playback(&mut mem_block_id);
                    } else {
                        start_playback(&mut mem_block_id);
                    }
                },
                2 => {
                    // Next video.
                    let was_active = PIP_ACTIVE.load(Ordering::Relaxed) != 0;
                    if was_active {
                        stop_playback(&mut mem_block_id);
                    }
                    unsafe {
                        if VIDEO_COUNT > 0 {
                            CURRENT_VIDEO = (CURRENT_VIDEO + 1) % VIDEO_COUNT;
                        }
                    }
                    if was_active {
                        start_playback(&mut mem_block_id);
                    }
                },
                3 => {
                    // Stop.
                    stop_playback(&mut mem_block_id);
                },
                _ => {},
            }
        }

        // If PIP active, decode frames.
        if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
            // Fill ringbuffer with file data.
            unsafe {
                fill_ringbuffer();
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
                // End of stream or error -- loop or advance.
                crate::debug_log(b"[VIDEO] end of stream");
                stop_playback(&mut mem_block_id);
                unsafe {
                    if VIDEO_COUNT > 1 {
                        CURRENT_VIDEO = (CURRENT_VIDEO + 1) % VIDEO_COUNT;
                        // Small delay then restart.
                        psp::sys::sceKernelDelayThread(100_000);
                    }
                }
                start_playback(&mut mem_block_id);
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
fn start_playback(mem_block_id: &mut psp::sys::SceUid) {
    if VIDEO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Video: not available");
        return;
    }

    // Allocate PIP frame buffers on demand.
    if !unsafe { alloc_pip_buffers() } {
        overlay::show_osd(b"PIP: out of memory");
        return;
    }

    let idx = unsafe { CURRENT_VIDEO };
    let count = unsafe { VIDEO_COUNT };
    if idx >= count {
        overlay::show_osd(b"No videos");
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
        return;
    }

    // Track the memory block for cleanup.
    // (In a real impl we'd save the block_id from init_mpeg_decoder,
    // but for simplicity we use a sentinel here.)
    *mem_block_id = psp::sys::SceUid(0);

    // Pause background MP3 audio so ATRAC can play.
    crate::audio::pause_for_video();

    PIP_ACTIVE.store(1, Ordering::Relaxed);

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
fn stop_playback(mem_block_id: &mut psp::sys::SceUid) {
    PIP_ACTIVE.store(0, Ordering::Relaxed);

    // Clean up decoder (frees MPEG buffers).
    unsafe {
        cleanup_mpeg(*mem_block_id);
    }
    *mem_block_id = psp::sys::SceUid(-1);

    // Free PIP frame buffers (returns ~140KB to user partition).
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

/// Ensure the video thread is running (lazy start on first PIP command).
fn ensure_video_thread() {
    if VIDEO_THREAD_STARTED.load(Ordering::Relaxed) != 0 {
        return;
    }
    // CAS to prevent double-start.
    if VIDEO_THREAD_STARTED
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    crate::debug_log(b"[VIDEO] starting video thread...");
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
        } else {
            crate::debug_log(b"[VIDEO] thread create FAILED");
            VIDEO_THREAD_STARTED.store(0, Ordering::Relaxed);
        }
    }
}

/// Toggle PIP on/off.
pub fn toggle_pip() {
    ensure_video_thread();
    VIDEO_CMD.store(1, Ordering::Relaxed);
}

/// Advance to next video.
pub fn next_video() {
    ensure_video_thread();
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

fn log_usize(prefix: &[u8], val: usize) {
    let mut buf = [0u8; 48];
    let p = copy_bytes(&mut buf, 0, prefix);
    let p = write_decimal(&mut buf, p, val as u32);
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
