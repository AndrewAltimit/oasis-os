//! Background MP3 playback via runtime NID resolution.
//!
//! User-mode imports cause PRX load failure, so we use
//! `psp::hook::find_function()` (wrapping `sctrlHENFindFunction`) to resolve
//! all audio NIDs at runtime.
//!
//! ## Strategy
//!
//! 1. Resolve `sceUtilityLoadModule` from `sceUtility_Driver` -- this is the
//!    official PSP API for loading optional system modules and it properly
//!    registers them so `find_function` can discover their exports.
//! 2. Use it to load `PSP_MODULE_AV_AVCODEC` (0x0300) and `PSP_MODULE_AV_MP3`
//!    (0x0302).
//! 3. Resolve sceMp3 NIDs (preferred -- higher-level streaming API).
//! 4. If sceMp3 fails, try sceAudiocodec NIDs (lower-level codec API).
//! 5. If a named module search fails, retry with NULL module name (searches
//!    all loaded modules on PRO/ME/ARK CFW).

use core::sync::atomic::{AtomicU8, Ordering};

use crate::overlay;

// ---------------------------------------------------------------------------
// sceAudio driver NIDs
// ---------------------------------------------------------------------------

const NID_AUDIO_CH_RESERVE: u32 = 0x5EC81C55;
const NID_AUDIO_OUTPUT_BLOCKING: u32 = 0x136CAF51;
const NID_AUDIO_CH_RELEASE: u32 = 0x6FC46853;
const NID_AUDIO_SET_CH_VOL: u32 = 0xB7E1D8E7;

// SRC (Sample Rate Conversion) channel -- separate output that
// does NOT conflict with the 8 regular PCM channels games use.
const NID_AUDIO_SRC_CH_RESERVE: u32 = 0x01562BA3;
const NID_AUDIO_SRC_OUTPUT_BLOCKING: u32 = 0xE0727056;
const NID_AUDIO_SRC_CH_RELEASE: u32 = 0x5C37C0AE;

const AUDIO_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceAudio_Driver\0", b"sceAudio_driver\0"),
    (b"sceAudio_Driver\0", b"sceAudio\0"),
    (b"sceAudio_Service\0", b"sceAudio_driver\0"),
    (b"sceAudio_Service\0", b"sceAudio\0"),
];

// ---------------------------------------------------------------------------
// sceUtility NIDs (for loading optional AV modules)
// ---------------------------------------------------------------------------

/// sceUtilityLoadModule(module_id) -> 0
const NID_UTILITY_LOAD_MODULE: u32 = 0x2A2B3DE0;

const UTILITY_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceUtility_Driver\0", b"sceUtility_private\0"),
    (b"sceUtility_Driver\0", b"sceUtility_driver\0"),
    (b"sceUtility_Driver\0", b"sceUtility\0"),
    (b"sceUtility_private\0", b"sceUtility_private\0"),
    (b"sceUtility_private\0", b"sceUtility\0"),
];

/// PSP optional module IDs for sceUtilityLoadModule.
const PSP_MODULE_AV_AVCODEC: i32 = 0x0300;
const PSP_MODULE_AV_MPEGBASE: i32 = 0x0301;
const PSP_MODULE_AV_MP3: i32 = 0x0302;

// ---------------------------------------------------------------------------
// sceMp3 NIDs (preferred -- higher-level streaming API)
// ---------------------------------------------------------------------------

const NID_MP3_INIT_RESOURCE: u32 = 0x35750070;
#[allow(dead_code)]
const NID_MP3_TERM_RESOURCE: u32 = 0xD0A56296;
const NID_MP3_RESERVE_HANDLE: u32 = 0x7F2A1880;
const NID_MP3_RELEASE_HANDLE: u32 = 0x0DB149F4;
const NID_MP3_INIT: u32 = 0x44E07129;
const NID_MP3_DECODE: u32 = 0xD021C0FB;
const NID_MP3_CHECK_NEED_DATA: u32 = 0xD8F54A51;
const NID_MP3_GET_INFO_TO_ADD: u32 = 0x732B042A;
const NID_MP3_NOTIFY_ADD_DATA: u32 = 0x29BFF3EC;

const MP3_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceMp3\0", b"sceMp3\0"),
    (b"sceMp3_Library\0", b"sceMp3\0"),
    (b"libmp3\0", b"sceMp3\0"),
    (b"sceMp3_Service\0", b"sceMp3\0"),
];

// ---------------------------------------------------------------------------
// sceAudiocodec NIDs (fallback -- lower-level codec API)
// ---------------------------------------------------------------------------

const NID_CODEC_CHECK_NEED_MEM: u32 = 0x9D3F790C;
const NID_CODEC_INIT: u32 = 0x5B37EB1D;
const NID_CODEC_DECODE: u32 = 0x70A703F8;
const NID_CODEC_GET_EDRAM: u32 = 0x3A20A200;
const NID_CODEC_RELEASE_EDRAM: u32 = 0x29681260;

const CODEC_MODULES: &[(&[u8], &[u8])] = &[
    // mp3play.prx uses this exact module/library pair:
    (b"sceAvcodec_wrapper\0", b"sceAudiocodec\0"),
    (b"sceAVcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAvcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec_Driver\0", b"sceAudiocodec\0"),
    (b"avcodec\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec\0", b"sceAudiocodec\0"),
];

const CODEC_TYPE_MP3: i32 = 0x1002;

// ---------------------------------------------------------------------------
// Module enumeration (for discovering loaded AV modules)
// ---------------------------------------------------------------------------

/// sceKernelGetModuleIdList(readbuf, readbufsize, idcount)
const NID_GET_MODULE_ID_LIST: u32 = 0x644CF325;
/// sceKernelQueryModuleInfo(uid, info)
const NID_QUERY_MODULE_INFO: u32 = 0x748CBED9;

/// Module/library pairs for ModuleMgrForKernel.
const MOD_MGR_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceModuleManager\0", b"ModuleMgrForKernel\0"),
    (b"ModuleMgrForKernel\0", b"ModuleMgrForKernel\0"),
];

/// Target module name substrings to match when enumerating modules.
/// If a loaded module's name contains one of these, we try to walk
/// its exports for sceMp3 / sceAudiocodec NIDs.
const MP3_NAME_PATTERNS: &[&[u8]] = &[b"mp3", b"Mp3", b"MP3"];
const CODEC_NAME_PATTERNS: &[&[u8]] =
    &[b"codec", b"Codec", b"avcodec", b"Avcodec"];

/// SceKernelModuleInfo struct size.
const MODULE_INFO_SIZE: u32 = 96;
/// Offset of text_addr in SceKernelModuleInfo.
const MODINFO_TEXT_ADDR: usize = 0x30;
/// Offset of name in SceKernelModuleInfo.
const MODINFO_NAME: usize = 0x44;

/// SceModuleInfo (embedded in module binary) offsets for export table.
/// ent_top at +0x24, ent_end at +0x28 (NOT size -- subtract for size).
const SCEMODINFO_ENT_TOP: usize = 0x24;
const SCEMODINFO_ENT_END: usize = 0x28;

// ---------------------------------------------------------------------------
// Resolved function pointers
// ---------------------------------------------------------------------------

static mut AUDIO_CH_RESERVE_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;
static mut AUDIO_OUTPUT_BLOCKING_FN: Option<
    unsafe extern "C" fn(i32, i32, *const u8) -> i32,
> = None;
#[allow(dead_code)]
static mut AUDIO_CH_RELEASE_FN: Option<unsafe extern "C" fn(i32) -> i32> =
    None;
static mut AUDIO_SET_CH_VOL_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;

// SRC channel function pointers (preferred -- no conflict with games)
static mut AUDIO_SRC_RESERVE_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;
static mut AUDIO_SRC_OUTPUT_FN: Option<
    unsafe extern "C" fn(i32, *const u8) -> i32,
> = None;
#[allow(dead_code)]
static mut AUDIO_SRC_RELEASE_FN: Option<unsafe extern "C" fn() -> i32> =
    None;
/// Whether we use SRC output (true) or regular channel (false).
static mut USE_SRC_OUTPUT: bool = false;

// Which decoder backend is active: 0=none, 1=sceMp3, 2=sceAudiocodec
static mut DECODER_BACKEND: u8 = 0;

// sceMp3 function pointers
static mut MP3_INIT_RESOURCE_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut MP3_RESERVE_HANDLE_FN: Option<
    unsafe extern "C" fn(*const Mp3InitStruct) -> i32,
> = None;
static mut MP3_RELEASE_HANDLE_FN: Option<unsafe extern "C" fn(i32) -> i32> =
    None;
static mut MP3_INIT_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut MP3_DECODE_FN: Option<
    unsafe extern "C" fn(i32, *mut *const i16) -> i32,
> = None;
static mut MP3_CHECK_NEED_DATA_FN: Option<
    unsafe extern "C" fn(i32) -> i32,
> = None;
static mut MP3_GET_INFO_TO_ADD_FN: Option<
    unsafe extern "C" fn(i32, *mut *mut u8, *mut i32, *mut i32) -> i32,
> = None;
static mut MP3_NOTIFY_ADD_DATA_FN: Option<
    unsafe extern "C" fn(i32, i32) -> i32,
> = None;

// sceAudiocodec function pointers
static mut CODEC_CHECK_NEED_MEM_FN: Option<
    unsafe extern "C" fn(*mut u32, i32) -> i32,
> = None;
static mut CODEC_INIT_FN: Option<
    unsafe extern "C" fn(*mut u32, i32) -> i32,
> = None;
static mut CODEC_DECODE_FN: Option<
    unsafe extern "C" fn(*mut u32, i32) -> i32,
> = None;
static mut CODEC_GET_EDRAM_FN: Option<
    unsafe extern "C" fn(*mut u32, i32) -> i32,
> = None;
static mut CODEC_RELEASE_EDRAM_FN: Option<
    unsafe extern "C" fn(*mut u32) -> i32,
> = None;

// sceKernelGetModuleIdList function pointer
static mut GET_MODULE_ID_LIST_FN: Option<
    unsafe extern "C" fn(*mut i32, i32, *mut i32) -> i32,
> = None;
// sceKernelQueryModuleInfo function pointer
static mut QUERY_MODULE_INFO_FN: Option<
    unsafe extern "C" fn(i32, *mut u8) -> i32,
> = None;

/// Text addresses of discovered MP3/codec modules (from enumeration).
static mut MP3_TEXT_ADDR: u32 = 0;
static mut CODEC_TEXT_ADDR: u32 = 0;

// ---------------------------------------------------------------------------
// Audio state (atomics for cross-thread communication)
// ---------------------------------------------------------------------------

static AUDIO_CMD: AtomicU8 = AtomicU8::new(0);
static AUDIO_VOLUME: AtomicU8 = AtomicU8::new(128);
static AUDIO_STATE: AtomicU8 = AtomicU8::new(0);
static AUDIO_AVAILABLE: AtomicU8 = AtomicU8::new(0);
static mut TRACK_NAME: [u8; 48] = [0u8; 48];

// ---------------------------------------------------------------------------
// Structures and constants
// ---------------------------------------------------------------------------

/// sceMp3 init structure.
#[repr(C)]
struct Mp3InitStruct {
    mp3_stream_start: i32,
    _unk1: i32,
    mp3_stream_end: i32,
    _unk2: i32,
    mp3_buf: *mut u8,
    mp3_buf_size: i32,
    pcm_buf: *mut u8,
    pcm_buf_size: i32,
}

const AUDIO_FORMAT_STEREO: i32 = 0;
const MP3_SAMPLES_PER_FRAME: i32 = 1152;
const MAX_PLAYLIST: usize = 32;
const MAX_FILENAME: usize = 128;
const MAX_SCAN_DEPTH: usize = 4;

/// sceMp3 stream buffer (64KB).
const MP3_BUF_SIZE: usize = 64 * 1024;
/// sceMp3 PCM decode buffer.
const PCM_BUF_SIZE: usize = MP3_SAMPLES_PER_FRAME as usize * 4 * 4;
/// sceAudiocodec read buffer (32KB).
const READ_BUF_SIZE: usize = 32 * 1024;
/// sceAudiocodec codec buffer (65 u32 = 260 bytes, must be 64-byte aligned).
const CODEC_BUF_WORDS: usize = 65;
/// sceAudiocodec working memory size (CheckNeedMem reports ~15208, round up).
const CODEC_WORK_SIZE: usize = 16 * 1024;

/// Total user-memory allocation for codec buffers.
/// Layout: [64-byte pad] [codec: 260] [pcm: 4608] [work: 16384] [read: 32768]
const UMEM_CODEC_SIZE: usize =
    64 + (CODEC_BUF_WORDS * 4) + (1152 * 2 * 2) + CODEC_WORK_SIZE + READ_BUF_SIZE;

/// UID of user-memory block, 0 = not allocated.
static mut UMEM_BLOCK_ID: psp::sys::SceUid = psp::sys::SceUid(0);
/// Pointer to codec buffer in user memory (64-byte aligned).
static mut UMEM_CODEC: *mut u32 = core::ptr::null_mut();
/// Pointer to PCM buffer in user memory.
static mut UMEM_PCM: *mut i16 = core::ptr::null_mut();
/// Pointer to codec working memory (replaces sceAudiocodecGetEDRAM).
static mut UMEM_WORK: *mut u8 = core::ptr::null_mut();
/// Pointer to read buffer in user memory.
static mut UMEM_READ: *mut u8 = core::ptr::null_mut();

/// Allocate codec buffers in user memory partition (partition 2).
/// Required because syscall stubs validate that pointers are in user range.
unsafe fn alloc_codec_user_mem() -> bool {
    let block = unsafe {
        psp::sys::sceKernelAllocPartitionMemory(
            psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
            b"oasis_codec\0".as_ptr(),
            psp::sys::SceSysMemBlockTypes::Low,
            UMEM_CODEC_SIZE as u32,
            core::ptr::null_mut(),
        )
    };
    if block < psp::sys::SceUid(0) {
        crate::debug_log(b"[OASIS] user mem alloc failed");
        return false;
    }
    let base = unsafe {
        psp::sys::sceKernelGetBlockHeadAddr(block)
    } as *mut u8;
    if base.is_null() {
        crate::debug_log(b"[OASIS] user mem addr null");
        return false;
    }
    unsafe {
        UMEM_BLOCK_ID = block;
        // Align codec buffer to 64 bytes.
        let codec_off = (64 - (base as usize % 64)) % 64;
        UMEM_CODEC = base.add(codec_off) as *mut u32;
        let pcm_off = codec_off + CODEC_BUF_WORDS * 4;
        UMEM_PCM = base.add(pcm_off) as *mut i16;
        let work_off = pcm_off + 1152 * 2 * 2;
        UMEM_WORK = base.add(work_off) as *mut u8;
        let read_off = work_off + CODEC_WORK_SIZE;
        UMEM_READ = base.add(read_off) as *mut u8;
    }
    log_i32(b"[OASIS] user mem @", base as i32);
    true
}

/// Free user-memory block if allocated.
unsafe fn free_codec_user_mem() {
    unsafe {
        if UMEM_BLOCK_ID >= psp::sys::SceUid(0) && UMEM_BLOCK_ID != psp::sys::SceUid(0) {
            psp::sys::sceKernelFreePartitionMemory(UMEM_BLOCK_ID);
            UMEM_BLOCK_ID = psp::sys::SceUid(0);
        }
    }
}

// ---------------------------------------------------------------------------
// Playlist data
// ---------------------------------------------------------------------------

static mut PLAYLIST: [[u8; MAX_FILENAME]; MAX_PLAYLIST] =
    [[0u8; MAX_FILENAME]; MAX_PLAYLIST];
static mut PLAYLIST_LEN: usize = 0;
static mut CURRENT_TRACK: usize = 0;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn current_track_name() -> &'static [u8] {
    unsafe {
        core::slice::from_raw_parts(
            (&raw const TRACK_NAME).cast::<u8>(),
            48,
        )
    }
}

pub fn audio_state() -> u8 {
    AUDIO_STATE.load(Ordering::Relaxed)
}

pub fn toggle_playback() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(1, Ordering::Relaxed);
}

pub fn next_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(2, Ordering::Relaxed);
}

pub fn prev_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(3, Ordering::Relaxed);
}

pub fn volume_up() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_add(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = copy_bytes(&mut buf, 0, b"Vol: ");
    p = write_u32_decimal(&mut buf, p, new as u32);
    overlay::show_osd(&buf[..p]);
}

pub fn volume_down() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_sub(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = copy_bytes(&mut buf, 0, b"Vol: ");
    p = write_u32_decimal(&mut buf, p, new as u32);
    overlay::show_osd(&buf[..p]);
}

// ---------------------------------------------------------------------------
// NID resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a NID trying each module/library pair, then null-module fallback.
unsafe fn resolve_nid(
    modules: &[(&[u8], &[u8])],
    nid: u32,
) -> Option<*mut u8> {
    // Try each named module/library pair.
    for &(module, library) in modules {
        if let Some(ptr) = unsafe {
            psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid)
        } {
            return Some(ptr);
        }
    }
    // Fallback: NULL module name (searches all loaded modules on PRO/ME/ARK).
    for &(_, library) in modules {
        if let Some(ptr) = unsafe {
            psp::hook::find_function(
                core::ptr::null(),
                library.as_ptr(),
                nid,
            )
        } {
            return Some(ptr);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Module enumeration and export walking
// ---------------------------------------------------------------------------

/// Check if a pointer looks like a valid PSP memory address.
fn is_valid_ptr(addr: u32) -> bool {
    if addr == 0 || addr & 3 != 0 {
        return false;
    }
    // User space:  0x08800000 - 0x0BFFFFFF
    // Kernel KSEG0: 0x80000000 - 0x8BFFFFFF
    // Kernel KSEG1: 0xA0000000 - 0xABFFFFFF
    (addr >= 0x0800_0000 && addr < 0x0C00_0000)
        || (addr >= 0x8000_0000 && addr < 0x8C00_0000)
        || (addr >= 0xA000_0000 && addr < 0xAC00_0000)
}

/// Resolve module enumeration APIs.
unsafe fn init_module_enum() -> bool {
    unsafe {
        if let Some(ptr) =
            resolve_nid(MOD_MGR_MODULES, NID_GET_MODULE_ID_LIST)
        {
            core::ptr::write_volatile(
                &raw mut GET_MODULE_ID_LIST_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(MOD_MGR_MODULES, NID_QUERY_MODULE_INFO)
        {
            core::ptr::write_volatile(
                &raw mut QUERY_MODULE_INFO_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        let have_list = core::ptr::read_volatile(
            &raw const GET_MODULE_ID_LIST_FN,
        )
        .is_some();
        let have_query = core::ptr::read_volatile(
            &raw const QUERY_MODULE_INFO_FN,
        )
        .is_some();
        if have_list && have_query {
            crate::debug_log(b"[OASIS] module enum APIs resolved");
            true
        } else {
            crate::debug_log(b"[OASIS] module enum APIs NOT found");
            false
        }
    }
}

/// Enumerate all loaded modules and log their names.
/// Stores text_addr for MP3/codec modules when found.
unsafe fn enumerate_modules() {
    unsafe {
        let get_list = match core::ptr::read_volatile(
            &raw const GET_MODULE_ID_LIST_FN,
        ) {
            Some(f) => f,
            None => return,
        };
        let query = match core::ptr::read_volatile(
            &raw const QUERY_MODULE_INFO_FN,
        ) {
            Some(f) => f,
            None => return,
        };

        let mut ids = [0i32; 128];
        let mut count: i32 = 0;
        let ret = get_list(
            ids.as_mut_ptr(),
            (128 * 4) as i32,
            &mut count,
        );
        if ret < 0 {
            log_i32(b"[OASIS] GetModuleIdList err=", ret);
            return;
        }
        log_i32(b"[OASIS] loaded modules: ", count);

        let n = (count as usize).min(128);
        let mut i = 0;
        while i < n {
            let mut info = [0u8; 96];
            // Set size field at offset 0.
            let size_ptr = info.as_mut_ptr() as *mut u32;
            *size_ptr = MODULE_INFO_SIZE;

            let qr = query(ids[i], info.as_mut_ptr());
            if qr < 0 {
                i += 1;
                continue;
            }

            // text_addr at offset 0x30.
            let text_addr =
                *(info.as_ptr().add(MODINFO_TEXT_ADDR) as *const u32);
            // name at offset 0x44 (28 bytes, null-terminated).
            let name = &info[MODINFO_NAME..MODINFO_NAME + 28];

            // Log: "mod: <name> @XXXXXXXX"
            let mut buf = [0u8; 64];
            let mut p = copy_bytes(&mut buf, 0, b"[OASIS] mod: ");
            let mut k = 0;
            while k < 28 && name[k] != 0 && p < buf.len() - 12 {
                buf[p] = name[k];
                p += 1;
                k += 1;
            }
            p = copy_bytes(&mut buf, p, b" @");
            p = write_hex32(&mut buf, p, text_addr);
            crate::debug_log(&buf[..p]);

            // Check if this is an MP3 or codec module.
            if contains_pattern(name, MP3_NAME_PATTERNS) {
                core::ptr::write_volatile(
                    &raw mut MP3_TEXT_ADDR,
                    text_addr,
                );
                crate::debug_log(b"[OASIS] => MP3 module!");
            }
            if contains_pattern(name, CODEC_NAME_PATTERNS) {
                core::ptr::write_volatile(
                    &raw mut CODEC_TEXT_ADDR,
                    text_addr,
                );
                crate::debug_log(b"[OASIS] => CODEC module!");
            }

            i += 1;
        }
    }
}

/// Check if `name` contains any of the given patterns.
fn contains_pattern(name: &[u8], patterns: &[&[u8]]) -> bool {
    for &pat in patterns {
        if byte_contains(name, pat) {
            return true;
        }
    }
    false
}

fn byte_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hlen = {
        let mut l = 0;
        while l < haystack.len() && haystack[l] != 0 {
            l += 1;
        }
        l
    };
    if needle.len() > hlen {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= hlen {
        let mut matched = true;
        let mut j = 0;
        while j < needle.len() {
            if haystack[i + j] != needle[j] {
                matched = false;
                break;
            }
            j += 1;
        }
        if matched {
            return true;
        }
        i += 1;
    }
    false
}

/// Walk exports starting from a module's text segment address.
///
/// The SceModuleInfo header is at the start of the text segment for
/// PRX modules. It contains ent_top (+0x24) and ent_end (+0x28).
unsafe fn walk_exports_from_text(
    text_addr: u32,
    nid: u32,
) -> Option<*mut u8> {
    if !is_valid_ptr(text_addr) {
        return None;
    }
    unsafe {
        let base = text_addr as *const u8;
        let ent_top_val =
            *(base.add(SCEMODINFO_ENT_TOP) as *const u32);
        let ent_end_val =
            *(base.add(SCEMODINFO_ENT_END) as *const u32);

        if !is_valid_ptr(ent_top_val) || !is_valid_ptr(ent_end_val) {
            return None;
        }
        if ent_end_val <= ent_top_val {
            return None;
        }
        let ent_size = (ent_end_val - ent_top_val) as usize;
        if ent_size > 0x10000 {
            return None;
        }

        walk_export_table(ent_top_val as *const u8, ent_size, nid)
    }
}

/// Walk an export table (array of SceLibraryEntryTable entries).
unsafe fn walk_export_table(
    ent_top: *const u8,
    ent_size: usize,
    nid: u32,
) -> Option<*mut u8> {
    unsafe {
        let mut offset = 0usize;
        while offset < ent_size {
            let entry = ent_top.add(offset);

            // SceLibraryEntryTable:
            //   +0x00: name (char*)
            //   +0x04: version (u16) | attribute (u16)
            //   +0x08: entLen (u8) | varCount (u8) | funcCount (u16)
            //   +0x0C: entrytable (u32*)
            let ent_len = *entry.add(8) as usize;
            if ent_len < 4 || ent_len > 16 {
                break;
            }

            let var_count = *entry.add(9) as usize;
            let func_count =
                *(entry.add(10) as *const u16) as usize;
            let entrytable =
                *(entry.add(12) as *const u32) as *const u32;

            if !entrytable.is_null()
                && func_count > 0
                && func_count < 256
                && is_valid_ptr(entrytable as u32)
            {
                let mut i = 0;
                while i < func_count {
                    if *entrytable.add(i) == nid {
                        let func_ptr = *entrytable
                            .add(func_count + var_count + i);
                        if func_ptr != 0 {
                            return Some(func_ptr as *mut u8);
                        }
                    }
                    i += 1;
                }
            }

            offset += ent_len * 4;
        }
    }
    None
}

/// Resolve a NID: try sctrlHENFindFunction, then export walking.
unsafe fn resolve_nid_any(
    modules: &[(&[u8], &[u8])],
    text_addr_ptr: *const u32,
    nid: u32,
) -> Option<*mut u8> {
    // Fast path: sctrlHENFindFunction (kernel modules).
    if let Some(ptr) = unsafe { resolve_nid(modules, nid) } {
        return Some(ptr);
    }
    // Slow path: walk exports from discovered text_addr.
    let text_addr =
        unsafe { core::ptr::read_volatile(text_addr_ptr) };
    if text_addr != 0 {
        if let Some(ptr) =
            unsafe { walk_exports_from_text(text_addr, nid) }
        {
            return Some(ptr);
        }
    }
    None
}

// (Memory dump infrastructure removed -- served its purpose for
// offline syscall table analysis.)

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
unsafe fn try_codec_stub_extraction() -> bool {
    crate::debug_log(b"[OASIS] trying codec stub extraction...");

    // Step 1: Find a known codec NID in user memory.
    let mut nid_addr: u32 = 0;
    let mut addr: u32 = 0x0880_0000;

    while addr < 0x0A00_0000 - 4 {
        let val = unsafe {
            core::ptr::read_volatile(addr as *const u32)
        };
        if val == NID_CODEC_DECODE {
            // Verify: at least 2 other known codec NIDs within 24 bytes.
            let mut nearby = 0u32;
            let check_lo = addr.saturating_sub(24);
            let check_hi = (addr + 24).min(0x0A00_0000 - 4);
            let mut c = check_lo;
            while c <= check_hi {
                let v = unsafe {
                    core::ptr::read_volatile(c as *const u32)
                };
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
        crate::debug_log(
            b"[OASIS] no codec NID cluster in user mem",
        );
        return false;
    }
    log_hex(b"[OASIS] codec NID found @", nid_addr);

    // Step 2: Walk backwards to find the NID table start (sorted
    // ascending).
    let mut table_start = nid_addr;
    while table_start > 0x0880_0004 {
        let prev = unsafe {
            core::ptr::read_volatile(
                (table_start - 4) as *const u32,
            )
        };
        let first = unsafe {
            core::ptr::read_volatile(table_start as *const u32)
        };
        if prev < first && prev > 0x0100_0000 {
            table_start -= 4;
        } else {
            break;
        }
    }

    // Walk forward to count entries.
    let mut table_end = table_start;
    let mut prev_val = 0u32;
    while table_end < nid_addr + 64 {
        let val = unsafe {
            core::ptr::read_volatile(table_end as *const u32)
        };
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
    let mut stub_table_ptr: u32 = 0;
    addr = 0x0880_0000;
    while addr < 0x0A00_0000 - 8 {
        let val = unsafe {
            core::ptr::read_volatile(addr as *const u32)
        };
        if val == table_start {
            // Next word should be the stub_table pointer (valid addr).
            let next = unsafe {
                core::ptr::read_volatile((addr + 4) as *const u32)
            };
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
    let mut i = 0u32;
    while i < entry_count {
        let nid = unsafe {
            core::ptr::read_volatile(
                (table_start + i * 4) as *const u32,
            )
        };
        let stub_addr = stub_table_ptr + i * 8;
        let insn0 = unsafe {
            core::ptr::read_volatile(stub_addr as *const u32)
        };
        let insn1 = unsafe {
            core::ptr::read_volatile(
                (stub_addr + 4) as *const u32,
            )
        };

        // Only accept syscall stubs (jr $ra + syscall N).
        let is_syscall =
            insn0 == 0x03E0_0008 && (insn1 & 0x3F) == 0x0C;
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
        unsafe {
            match nid {
                NID_CODEC_CHECK_NEED_MEM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_CHECK_NEED_MEM_FN,
                        Some(core::mem::transmute(
                            stub_addr as usize,
                        )),
                    );
                    resolved += 1;
                }
                NID_CODEC_INIT => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_INIT_FN,
                        Some(core::mem::transmute(
                            stub_addr as usize,
                        )),
                    );
                    resolved += 1;
                }
                NID_CODEC_DECODE => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_DECODE_FN,
                        Some(core::mem::transmute(
                            stub_addr as usize,
                        )),
                    );
                    resolved += 1;
                }
                NID_CODEC_GET_EDRAM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_GET_EDRAM_FN,
                        Some(core::mem::transmute(
                            stub_addr as usize,
                        )),
                    );
                    resolved += 1;
                }
                NID_CODEC_RELEASE_EDRAM => {
                    core::ptr::write_volatile(
                        &raw mut CODEC_RELEASE_EDRAM_FN,
                        Some(core::mem::transmute(
                            stub_addr as usize,
                        )),
                    );
                    resolved += 1;
                }
                _ => {}
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
unsafe fn load_av_modules() {
    // Strategy 1: sceUtilityLoadModule (loads into user space).
    let load_fn: Option<unsafe extern "C" fn(i32) -> i32> = unsafe {
        resolve_nid(UTILITY_MODULES, NID_UTILITY_LOAD_MODULE)
            .map(|ptr| core::mem::transmute(ptr))
    };

    if let Some(load) = load_fn {
        crate::debug_log(b"[OASIS] sceUtilityLoadModule resolved");
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
        unsafe {
            let mod_id = psp::sys::sceKernelLoadModule(
                path.as_ptr(),
                0,
                core::ptr::null_mut(),
            );
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
unsafe fn try_resolve_mp3() -> bool {
    unsafe {
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_INIT_RESOURCE,
        ) {
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
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_INIT,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_INIT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid_any(
            MP3_MODULES,
            &raw const MP3_TEXT_ADDR,
            NID_MP3_DECODE,
        ) {
            core::ptr::write_volatile(
                &raw mut MP3_DECODE_FN,
                Some(core::mem::transmute(ptr)),
            );
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
        core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
            .is_some()
            && core::ptr::read_volatile(
                &raw const MP3_RESERVE_HANDLE_FN,
            )
            .is_some()
            && core::ptr::read_volatile(&raw const MP3_DECODE_FN)
                .is_some()
            && core::ptr::read_volatile(
                &raw const MP3_GET_INFO_TO_ADD_FN,
            )
            .is_some()
            && core::ptr::read_volatile(
                &raw const MP3_NOTIFY_ADD_DATA_FN,
            )
            .is_some()
    }
}

/// Try to resolve sceAudiocodec function pointers. Uses combined
/// resolution (sctrlHENFindFunction + export table walking).
unsafe fn try_resolve_codec() -> bool {
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
        if let Some(ptr) = resolve_nid_any(
            CODEC_MODULES,
            &raw const CODEC_TEXT_ADDR,
            NID_CODEC_INIT,
        ) {
            core::ptr::write_volatile(
                &raw mut CODEC_INIT_FN,
                Some(core::mem::transmute(ptr)),
            );
        } else {
            crate::debug_log(b"[OASIS] NID miss: CodecInit");
        }
        if let Some(ptr) = resolve_nid_any(
            CODEC_MODULES,
            &raw const CODEC_TEXT_ADDR,
            NID_CODEC_DECODE,
        ) {
            core::ptr::write_volatile(
                &raw mut CODEC_DECODE_FN,
                Some(core::mem::transmute(ptr)),
            );
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
            && core::ptr::read_volatile(&raw const CODEC_DECODE_FN)
                .is_some()
    }
}

/// Resolve all audio driver function pointers.
unsafe fn init_audio_drivers() -> bool {
    // Step 1: Resolve sceAudio driver (always available in games).
    unsafe {
        if let Some(ptr) = resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RESERVE) {
            core::ptr::write_volatile(
                &raw mut AUDIO_CH_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(AUDIO_MODULES, NID_AUDIO_OUTPUT_BLOCKING)
        {
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
        if let Some(ptr) =
            resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_CH_RESERVE)
        {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_RESERVE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_OUTPUT_BLOCKING)
        {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_OUTPUT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(AUDIO_MODULES, NID_AUDIO_SRC_CH_RELEASE)
        {
            core::ptr::write_volatile(
                &raw mut AUDIO_SRC_RELEASE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Prefer SRC output if both SRC reserve and output are available.
        if core::ptr::read_volatile(&raw const AUDIO_SRC_RESERVE_FN)
            .is_some()
            && core::ptr::read_volatile(&raw const AUDIO_SRC_OUTPUT_FN)
                .is_some()
        {
            core::ptr::write_volatile(&raw mut USE_SRC_OUTPUT, true);
            crate::debug_log(b"[OASIS] audio SRC driver resolved");
        } else if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN)
            .is_none()
            || core::ptr::read_volatile(
                &raw const AUDIO_OUTPUT_BLOCKING_FN,
            )
            .is_none()
        {
            crate::debug_log(b"[OASIS] sceAudio driver NOT found");
            return false;
        }
        crate::debug_log(b"[OASIS] audio driver resolved");
    }

    // Step 2: Load AV modules via sceUtilityLoadModule (or fallback).
    unsafe { load_av_modules() };

    // Step 3: Enumerate all loaded modules to find MP3/codec modules.
    // sctrlHENFindFunction only searches kernel modules, so we use
    // sceKernelGetModuleIdList + sceKernelQueryModuleInfo to find
    // user-mode modules loaded by sceUtilityLoadModule, get their
    // text_addr, and walk their export tables manually.
    unsafe { init_module_enum() };
    unsafe { enumerate_modules() };

    // Step 4: Try sceMp3 first (preferred -- streaming API).
    if unsafe { try_resolve_mp3() } {
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 1) };
        crate::debug_log(b"[OASIS] using sceMp3 backend");
        return true;
    }
    crate::debug_log(b"[OASIS] sceMp3 NOT found, trying sceAudiocodec");

    // Step 5: Try sceAudiocodec (fallback -- frame-by-frame).
    if unsafe { try_resolve_codec() } {
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2) };
        crate::debug_log(b"[OASIS] using sceAudiocodec backend");
        return true;
    }
    crate::debug_log(b"[OASIS] NO decoder backend available");

    // Step 6: Try extracting sceAudiocodec from game's import stubs.
    // Fast scan (~0.1s): finds the game's resolved import stubs
    // and extracts function addresses from MIPS J-instructions.
    if unsafe { try_codec_stub_extraction() } {
        unsafe {
            core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2);
        }
        crate::debug_log(
            b"[OASIS] sceAudiocodec via stub extraction!",
        );
        return true;
    }

    crate::debug_log(b"[OASIS] all audio resolution methods failed");

    false
}

// ---------------------------------------------------------------------------
// Playlist scanning
// ---------------------------------------------------------------------------

unsafe fn scan_playlist() {
    let config = crate::config::get_config();
    unsafe {
        core::ptr::write_volatile(&raw mut PLAYLIST_LEN, 0);
        scan_dir_recursive(&config.music_dir, config.music_dir_len, 0);
    }
    let count =
        unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) };
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[OASIS] found ");
    p = write_u32_decimal(&mut buf, p, count as u32);
    p = copy_bytes(&mut buf, p, b" mp3 files");
    crate::debug_log(&buf[..p]);
}

unsafe fn scan_dir_recursive(dir_path: &[u8], dir_len: usize, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let pl_len =
        unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) };
    if pl_len >= MAX_PLAYLIST {
        return;
    }

    let dfd = unsafe { psp::sys::sceIoDopen(dir_path.as_ptr()) };
    if dfd.0 < 0 {
        if depth == 0 {
            crate::debug_log(b"[OASIS] music dir not found");
        }
        return;
    }

    unsafe {
        let mut dirent = core::mem::zeroed::<psp::sys::SceIoDirent>();
        loop {
            let ret = psp::sys::sceIoDread(dfd, &mut dirent);
            if ret <= 0 {
                break;
            }
            let pl_len =
                core::ptr::read_volatile(&raw const PLAYLIST_LEN);
            if pl_len >= MAX_PLAYLIST {
                break;
            }

            let name_ptr = dirent.d_name.as_ptr() as *const u8;
            let mut name_len = 0usize;
            while name_len < 256 && *name_ptr.add(name_len) != 0 {
                name_len += 1;
            }
            if name_len == 0 {
                continue;
            }
            // Skip "." and ".."
            if name_len == 1 && *name_ptr == b'.' {
                continue;
            }
            if name_len == 2
                && *name_ptr == b'.'
                && *name_ptr.add(1) == b'.'
            {
                continue;
            }

            let is_dir = (dirent.d_stat.st_attr.bits() & 0x0010) != 0;

            if is_dir {
                let sub_len = dir_len + name_len + 1;
                if sub_len + 1 > MAX_FILENAME {
                    continue;
                }
                let mut sub_path = [0u8; MAX_FILENAME];
                let mut j = 0;
                while j < dir_len {
                    sub_path[j] = dir_path[j];
                    j += 1;
                }
                let mut k = 0;
                while k < name_len {
                    sub_path[j + k] = *name_ptr.add(k);
                    k += 1;
                }
                sub_path[j + name_len] = b'/';
                sub_path[j + name_len + 1] = 0;
                scan_dir_recursive(&sub_path, sub_len, depth + 1);
            } else {
                if name_len < 5 {
                    continue;
                }
                let e = name_len - 4;
                if *name_ptr.add(e) != b'.'
                    || (*name_ptr.add(e + 1)).to_ascii_lowercase() != b'm'
                    || (*name_ptr.add(e + 2)).to_ascii_lowercase() != b'p'
                    || (*name_ptr.add(e + 3)).to_ascii_lowercase() != b'3'
                {
                    continue;
                }
                let total_len = dir_len + name_len;
                if total_len + 1 > MAX_FILENAME {
                    continue;
                }
                let entry = &mut (*(&raw mut PLAYLIST))[pl_len];
                let mut j = 0;
                while j < dir_len {
                    entry[j] = dir_path[j];
                    j += 1;
                }
                let mut k = 0;
                while k < name_len {
                    entry[j + k] = *name_ptr.add(k);
                    k += 1;
                }
                entry[j + k] = 0;
                core::ptr::write_volatile(
                    &raw mut PLAYLIST_LEN,
                    pl_len + 1,
                );
            }
        }
        psp::sys::sceIoDclose(dfd);
    }
}

// ---------------------------------------------------------------------------
// Track name
// ---------------------------------------------------------------------------

unsafe fn set_track_name(path: &[u8]) {
    unsafe {
        let mut last_slash = 0;
        let mut i = 0;
        while i < path.len() && path[i] != 0 {
            if path[i] == b'/' {
                last_slash = i + 1;
            }
            i += 1;
        }
        let name = &path[last_slash..];
        let mut len = 0;
        while len < name.len() && name[len] != 0 {
            len += 1;
        }
        if len >= 4
            && name[len - 4] == b'.'
            && name[len - 3].to_ascii_lowercase() == b'm'
            && name[len - 2].to_ascii_lowercase() == b'p'
            && name[len - 1].to_ascii_lowercase() == b'3'
        {
            len -= 4;
        }
        let copy_len = len.min(47);
        let mut j = 0;
        while j < copy_len {
            (*(&raw mut TRACK_NAME))[j] = name[j];
            j += 1;
        }
        while j < 48 {
            (*(&raw mut TRACK_NAME))[j] = 0;
            j += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

unsafe extern "C" fn audio_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    unsafe { psp::sys::sceKernelDelayThread(1_000_000) };

    if !unsafe { init_audio_drivers() } {
        crate::debug_log(b"[OASIS] audio init failed");
        return 1;
    }

    AUDIO_AVAILABLE.store(1, Ordering::Relaxed);
    unsafe { scan_playlist() };

    if unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) } == 0 {
        crate::debug_log(b"[OASIS] no mp3 files found");
        return 0;
    }

    // Init MP3 resource manager (sceMp3 backend only).
    let backend =
        unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
    if backend == 1 {
        unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
            {
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
        if !unsafe { alloc_codec_user_mem() } {
            crate::debug_log(b"[OASIS] codec user mem failed");
            return 1;
        }
    }

    // Reserve audio output.  Prefer the SRC (Sample Rate Conversion)
    // channel which is a dedicated output path that does NOT conflict
    // with the 8 regular PCM channels games use.
    let use_src = unsafe { core::ptr::read_volatile(&raw const USE_SRC_OUTPUT) };
    let channel: i32;
    if use_src {
        let ret = unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const AUDIO_SRC_RESERVE_FN)
            {
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
        channel = unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN)
            {
                let mut ch =
                    f(7, MP3_SAMPLES_PER_FRAME, AUDIO_FORMAT_STEREO);
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
    unsafe { core::ptr::write_volatile(&raw mut CURRENT_TRACK, 0) };

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
            }
            2 => unsafe {
                let cur =
                    core::ptr::read_volatile(&raw const CURRENT_TRACK);
                let pl =
                    core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                core::ptr::write_volatile(
                    &raw mut CURRENT_TRACK,
                    (cur + 1) % pl,
                );
            },
            3 => unsafe {
                let cur =
                    core::ptr::read_volatile(&raw const CURRENT_TRACK);
                let pl =
                    core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                core::ptr::write_volatile(
                    &raw mut CURRENT_TRACK,
                    if cur == 0 { pl - 1 } else { cur - 1 },
                );
            },
            _ => {}
        }

        let state = AUDIO_STATE.load(Ordering::Relaxed);
        if state == 0 || state == 2 {
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        let track_idx =
            unsafe { core::ptr::read_volatile(&raw const CURRENT_TRACK) };
        let track_path =
            unsafe { &(*(&raw const PLAYLIST))[track_idx] };
        unsafe { set_track_name(track_path) };

        let backend =
            unsafe { core::ptr::read_volatile(&raw const DECODER_BACKEND) };
        let result = match backend {
            1 => unsafe { play_track_mp3(track_path, channel) },
            2 => unsafe { play_track_codec(track_path, channel) },
            _ => -1,
        };
        if result < 0 {
            crate::debug_log(b"[OASIS] track playback error");
        }

        // Advance to next track.
        unsafe {
            let cur =
                core::ptr::read_volatile(&raw const CURRENT_TRACK);
            let pl =
                core::ptr::read_volatile(&raw const PLAYLIST_LEN);
            core::ptr::write_volatile(
                &raw mut CURRENT_TRACK,
                (cur + 1) % pl,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Backend 1: sceMp3 streaming
// ---------------------------------------------------------------------------

unsafe fn play_track_mp3(path: &[u8], channel: i32) -> i32 {
    let fd = unsafe {
        psp::sys::sceIoOpen(path.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        return -1;
    }

    let file_size = unsafe {
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End)
    } as i32;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
    if file_size <= 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    static mut S_MP3_BUF: [u8; MP3_BUF_SIZE] = [0u8; MP3_BUF_SIZE];
    static mut S_PCM_BUF: [u8; PCM_BUF_SIZE] = [0u8; PCM_BUF_SIZE];

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

    let handle = unsafe {
        match core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN) {
            Some(f) => f(&init),
            None => {
                psp::sys::sceIoClose(fd);
                return -1;
            }
        }
    };
    if handle < 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    unsafe { fill_stream_data(handle, fd) };

    let ret = unsafe {
        match core::ptr::read_volatile(&raw const MP3_INIT_FN) {
            Some(f) => f(handle),
            None => -1,
        }
    };
    if ret < 0 {
        unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const MP3_RELEASE_HANDLE_FN)
            {
                f(handle);
            }
            psp::sys::sceIoClose(fd);
        }
        return -1;
    }

    let mut result = 0i32;
    loop {
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 {
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
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        let needs_data = unsafe {
            match core::ptr::read_volatile(
                &raw const MP3_CHECK_NEED_DATA_FN,
            ) {
                Some(f) => f(handle),
                None => 0,
            }
        };
        if needs_data > 0 {
            let filled = unsafe { fill_stream_data(handle, fd) };
            if filled <= 0 {
                break;
            }
        }

        let mut pcm_out: *const i16 = core::ptr::null();
        let decoded = unsafe {
            match core::ptr::read_volatile(&raw const MP3_DECODE_FN) {
                Some(f) => f(handle, &mut pcm_out),
                None => break,
            }
        };
        if decoded <= 0 {
            break;
        }

        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000)
            / 255;
        let use_src = unsafe {
            core::ptr::read_volatile(&raw const USE_SRC_OUTPUT)
        };
        unsafe {
            if use_src {
                if let Some(f) = core::ptr::read_volatile(
                    &raw const AUDIO_SRC_OUTPUT_FN,
                ) {
                    let ret = f(vol, pcm_out as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            } else {
                if let Some(f) = core::ptr::read_volatile(
                    &raw const AUDIO_SET_CH_VOL_FN,
                ) {
                    f(channel, vol, vol);
                }
                if let Some(f) = core::ptr::read_volatile(
                    &raw const AUDIO_OUTPUT_BLOCKING_FN,
                ) {
                    let ret = f(channel, vol, pcm_out as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            }
        }
    }

    unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const MP3_RELEASE_HANDLE_FN)
        {
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

    unsafe {
        let get_info = match core::ptr::read_volatile(
            &raw const MP3_GET_INFO_TO_ADD_FN,
        ) {
            Some(f) => f,
            None => return -1,
        };
        let notify = match core::ptr::read_volatile(
            &raw const MP3_NOTIFY_ADD_DATA_FN,
        ) {
            Some(f) => f,
            None => return -1,
        };

        let ret =
            get_info(handle, &mut dst_ptr, &mut to_write, &mut src_pos);
        if ret < 0 || to_write <= 0 {
            return 0;
        }

        psp::sys::sceIoLseek(fd, src_pos as i64, psp::sys::IoWhence::Set);
        let read = psp::sys::sceIoRead(
            fd,
            dst_ptr as *mut _,
            to_write as u32,
        );
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

unsafe fn play_track_codec(path: &[u8], channel: i32) -> i32 {
    let fd = unsafe {
        psp::sys::sceIoOpen(path.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        return -1;
    }

    let file_size = unsafe {
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End)
    } as usize;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
    log_i32(b"[OASIS] file_size=", file_size as i32);
    if file_size == 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // Use user-memory buffers (allocated in audio_thread_entry).
    let codec = unsafe { core::ptr::read_volatile(&raw const UMEM_CODEC) };
    let pcm_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_PCM) };
    let read_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_READ) };
    if codec.is_null() || pcm_buf.is_null() || read_buf.is_null() {
        crate::debug_log(b"[OASIS] codec bufs null");
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }
    log_i32(b"[OASIS] codec_buf @", codec as i32);

    // Zero the codec buffer.
    unsafe {
        let mut i = 0;
        while i < CODEC_BUF_WORDS {
            *codec.add(i) = 0;
            i += 1;
        }
    }

    unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN)
        {
            let ret = f(codec, CODEC_TYPE_MP3);
            log_i32(b"[OASIS] CheckNeedMem=", ret);
        } else {
            crate::debug_log(b"[OASIS] no CheckNeedMem fn");
        }
        // Instead of sceAudiocodecGetEDRAM (which steals shared eDRAM from
        // the game's GE/media engine), point codec[5] at our own working
        // buffer allocated from user RAM.
        let work = core::ptr::read_volatile(&raw const UMEM_WORK);
        if work.is_null() {
            crate::debug_log(b"[OASIS] no work buf");
            psp::sys::sceIoClose(fd);
            return -1;
        }
        *codec.add(5) = work as u32;
        log_i32(b"[OASIS] work_buf @", work as i32);

        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_INIT_FN)
        {
            let ret = f(codec, CODEC_TYPE_MP3);
            log_i32(b"[OASIS] CodecInit=", ret);
            if ret < 0 {
                psp::sys::sceIoClose(fd);
                return -1;
            }
        } else {
            crate::debug_log(b"[OASIS] no CodecInit fn");
            psp::sys::sceIoClose(fd);
            return -1;
        }
    }

    let mut file_pos: usize;
    let mut buf_valid: usize;
    let mut buf_pos: usize = 0;

    let initial_read = unsafe {
        psp::sys::sceIoRead(fd, read_buf as *mut _, READ_BUF_SIZE as u32)
    };
    if initial_read <= 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }
    buf_valid = initial_read as usize;
    file_pos = buf_valid;

    // Skip ID3v2 tag.
    let slice =
        unsafe { core::slice::from_raw_parts(read_buf, buf_valid) };
    let id3_skip = skip_id3v2(slice);
    if id3_skip > 0 && id3_skip < buf_valid {
        buf_pos = id3_skip;
    }

    // Dump codec buffer after init (first 16 words) for debugging.
    unsafe {
        let mut i = 0u32;
        while i < 16 {
            let val = *codec.add(i as usize);
            if val != 0 {
                log_i32(b"[OASIS] cb[", i as i32);
                log_i32(b"]=", val as i32);
            }
            i += 1;
        }
    }

    let decode_fn = unsafe {
        match core::ptr::read_volatile(&raw const CODEC_DECODE_FN) {
            Some(f) => f,
            None => {
                psp::sys::sceIoClose(fd);
                return -1;
            }
        }
    };

    let mut result = 0i32;
    let mut frame_count: u32 = 0;

    loop {
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 {
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
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Refill buffer when running low.
        if buf_valid - buf_pos < 2048 && file_pos < file_size {
            let remaining = buf_valid - buf_pos;
            if remaining > 0 && buf_pos > 0 {
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
            let to_read = READ_BUF_SIZE - buf_valid;
            if to_read > 0 {
                let read = unsafe {
                    psp::sys::sceIoRead(
                        fd,
                        read_buf.add(buf_valid) as *mut _,
                        to_read as u32,
                    )
                };
                if read > 0 {
                    buf_valid += read as usize;
                    file_pos += read as usize;
                }
            }
        }

        if buf_valid - buf_pos < 4 {
            break;
        }

        let slice = unsafe {
            core::slice::from_raw_parts(read_buf, buf_valid)
        };
        let sync_pos = match find_mp3_sync(slice, buf_pos) {
            Some(pos) => pos,
            None => break,
        };
        buf_pos = sync_pos;
        if buf_valid - buf_pos < 8 {
            break;
        }

        let avail = buf_valid - buf_pos;
        unsafe {
            *codec.add(6) = read_buf.add(buf_pos) as u32;
            *codec.add(7) = avail as u32;
            *codec.add(8) = pcm_buf as u32;
            *codec.add(9) = (1152 * 4) as u32;
            *codec.add(10) = avail as u32;
        }

        // Log first 4 bytes of source data on first frame.
        if frame_count == 0 {
            let b0 = unsafe { *read_buf.add(buf_pos) } as i32;
            let b1 = unsafe { *read_buf.add(buf_pos + 1) } as i32;
            let b2 = unsafe { *read_buf.add(buf_pos + 2) } as i32;
            let b3 = unsafe { *read_buf.add(buf_pos + 3) } as i32;
            log_i32(b"[OASIS] hdr0=", b0);
            log_i32(b"[OASIS] hdr1=", b1);
            log_i32(b"[OASIS] hdr2=", b2);
            log_i32(b"[OASIS] hdr3=", b3);
        }

        let ret = unsafe { decode_fn(codec, CODEC_TYPE_MP3) };
        if frame_count < 5 {
            log_i32(b"[OASIS] decode ret=", ret);
            log_i32(b"[OASIS] avail=", avail as i32);
        }
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

        let consumed = unsafe { *codec.add(7) } as usize;
        if frame_count < 5 {
            log_i32(b"[OASIS] consumed=", consumed as i32);
        }
        if consumed == 0 {
            buf_pos += 1;
            continue;
        }
        buf_pos += consumed;

        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000)
            / 255;
        let use_src = unsafe {
            core::ptr::read_volatile(&raw const USE_SRC_OUTPUT)
        };
        unsafe {
            if use_src {
                // SRC output: sceAudioSRCOutputBlocking(volume, buffer)
                if let Some(f) = core::ptr::read_volatile(
                    &raw const AUDIO_SRC_OUTPUT_FN,
                ) {
                    let ret = f(vol, pcm_buf as *const u8);
                    if frame_count < 5 {
                        log_i32(b"[OASIS] srcOut=", ret);
                    }
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            } else {
                // Regular channel output.
                if let Some(f) =
                    core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN)
                {
                    f(channel, vol, vol);
                }
                if let Some(f) = core::ptr::read_volatile(
                    &raw const AUDIO_OUTPUT_BLOCKING_FN,
                ) {
                    let ret = f(channel, vol, pcm_buf as *const u8);
                    if frame_count < 5 {
                        log_i32(b"[OASIS] audioOut=", ret);
                    }
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            }
        }
        frame_count += 1;
    }

    unsafe { psp::sys::sceIoClose(fd) };
    result
}

fn skip_id3v2(data: &[u8]) -> usize {
    if data.len() < 10 {
        return 0;
    }
    if data[0] != b'I' || data[1] != b'D' || data[2] != b'3' {
        return 0;
    }
    let size = ((data[6] as u32) << 21)
        | ((data[7] as u32) << 14)
        | ((data[8] as u32) << 7)
        | (data[9] as u32);
    10 + size as usize
}

fn find_mp3_sync(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < data.len() {
        if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
            let version = (data[i + 1] >> 3) & 0x03;
            let layer = (data[i + 1] >> 1) & 0x03;
            if version != 1 && layer != 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Thread start
// ---------------------------------------------------------------------------

pub fn start_audio_thread() {
    crate::debug_log(b"[OASIS] starting audio thread...");

    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisAudio\0".as_ptr(),
            audio_thread_entry,
            0x1E,
            0x4000,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            crate::debug_log(b"[OASIS] audio thread started");
        } else {
            crate::debug_log(b"[OASIS] audio thread create FAILED");
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn copy_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    let mut i = 0;
    while i < s.len() && p < buf.len() {
        buf[p] = s[i];
        p += 1;
        i += 1;
    }
    p
}

fn write_u32_decimal(buf: &mut [u8], pos: usize, val: u32) -> usize {
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
    while i < 8 && p < buf.len() {
        let nibble = (val >> (28 - i * 4)) & 0xF;
        buf[p] = hex[nibble as usize];
        p += 1;
        i += 1;
    }
    p
}

fn log_i32(prefix: &[u8], val: i32) {
    let mut buf = [0u8; 64];
    let mut p = copy_bytes(&mut buf, 0, prefix);
    if val < 0 {
        if p < buf.len() {
            buf[p] = b'-';
            p += 1;
        }
        p = write_u32_decimal(&mut buf, p, (-(val as i64)) as u32);
    } else {
        p = write_u32_decimal(&mut buf, p, val as u32);
    }
    crate::debug_log(&buf[..p]);
}
