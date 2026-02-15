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
    (b"sceAVcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAvcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec_Driver\0", b"sceAudiocodec\0"),
    (b"avcodec\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec\0", b"sceAudiocodec\0"),
];

const CODEC_TYPE_MP3: i32 = 0x1002;

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
/// sceAudiocodec codec buffer (128 bytes = 32 u32).
const CODEC_BUF_WORDS: usize = 32;

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

/// Log a NID resolution result for diagnostics.
unsafe fn resolve_nid_logged(
    modules: &[(&[u8], &[u8])],
    nid: u32,
    label: &[u8],
) -> Option<*mut u8> {
    let result = unsafe { resolve_nid(modules, nid) };
    if result.is_none() {
        let mut buf = [0u8; 64];
        let mut p = copy_bytes(&mut buf, 0, b"[OASIS] NID miss: ");
        p = copy_bytes(&mut buf, p, label);
        crate::debug_log(&buf[..p]);
    }
    result
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

/// Load PSP AV modules via sceUtilityLoadModule.
///
/// This is the proper way to load optional system modules (MP3 codec,
/// MPEG, etc). It handles dependencies and registers exports so that
/// `sctrlHENFindFunction` can discover them.
unsafe fn load_av_modules() {
    // Resolve sceUtilityLoadModule from the utility driver.
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

        // Fallback: try sceKernelLoadModule for flash0 PRXs.
        let modules: &[&[u8]] = &[
            b"flash0:/kd/avcodec.prx\0",
            b"flash0:/kd/mpegbase.prx\0",
            b"flash0:/kd/mpeg.prx\0",
            b"flash0:/kd/libmp3.prx\0",
        ];
        for path in modules {
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
                }
            }
        }
        crate::debug_log(b"[OASIS] loaded PRXs via sceKernelLoadModule");
    }
}

/// Try to resolve sceMp3 function pointers. Returns true if all critical
/// functions were found.
unsafe fn try_resolve_mp3() -> bool {
    unsafe {
        if let Some(ptr) = resolve_nid(MP3_MODULES, NID_MP3_INIT_RESOURCE) {
            core::ptr::write_volatile(
                &raw mut MP3_INIT_RESOURCE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(MP3_MODULES, NID_MP3_RESERVE_HANDLE) {
            core::ptr::write_volatile(
                &raw mut MP3_RESERVE_HANDLE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(MP3_MODULES, NID_MP3_RELEASE_HANDLE) {
            core::ptr::write_volatile(
                &raw mut MP3_RELEASE_HANDLE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(MP3_MODULES, NID_MP3_INIT) {
            core::ptr::write_volatile(
                &raw mut MP3_INIT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(MP3_MODULES, NID_MP3_DECODE) {
            core::ptr::write_volatile(
                &raw mut MP3_DECODE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(MP3_MODULES, NID_MP3_CHECK_NEED_DATA)
        {
            core::ptr::write_volatile(
                &raw mut MP3_CHECK_NEED_DATA_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(MP3_MODULES, NID_MP3_GET_INFO_TO_ADD)
        {
            core::ptr::write_volatile(
                &raw mut MP3_GET_INFO_TO_ADD_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid(MP3_MODULES, NID_MP3_NOTIFY_ADD_DATA)
        {
            core::ptr::write_volatile(
                &raw mut MP3_NOTIFY_ADD_DATA_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Check critical functions.
        core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN)
                .is_some()
            && core::ptr::read_volatile(&raw const MP3_DECODE_FN).is_some()
            && core::ptr::read_volatile(&raw const MP3_GET_INFO_TO_ADD_FN)
                .is_some()
            && core::ptr::read_volatile(&raw const MP3_NOTIFY_ADD_DATA_FN)
                .is_some()
    }
}

/// Try to resolve sceAudiocodec function pointers. Returns true if all
/// critical functions were found.
unsafe fn try_resolve_codec() -> bool {
    unsafe {
        if let Some(ptr) =
            resolve_nid_logged(
                CODEC_MODULES,
                NID_CODEC_CHECK_NEED_MEM,
                b"CheckNeedMem",
            )
        {
            core::ptr::write_volatile(
                &raw mut CODEC_CHECK_NEED_MEM_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid_logged(CODEC_MODULES, NID_CODEC_INIT, b"CodecInit")
        {
            core::ptr::write_volatile(
                &raw mut CODEC_INIT_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid_logged(
                CODEC_MODULES,
                NID_CODEC_DECODE,
                b"CodecDecode",
            )
        {
            core::ptr::write_volatile(
                &raw mut CODEC_DECODE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid_logged(
                CODEC_MODULES,
                NID_CODEC_GET_EDRAM,
                b"GetEDRAM",
            )
        {
            core::ptr::write_volatile(
                &raw mut CODEC_GET_EDRAM_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) =
            resolve_nid_logged(
                CODEC_MODULES,
                NID_CODEC_RELEASE_EDRAM,
                b"RelEDRAM",
            )
        {
            core::ptr::write_volatile(
                &raw mut CODEC_RELEASE_EDRAM_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        core::ptr::read_volatile(&raw const CODEC_INIT_FN).is_some()
            && core::ptr::read_volatile(&raw const CODEC_DECODE_FN).is_some()
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

        if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN).is_none()
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

    // Step 3: Try sceMp3 first (preferred -- streaming API).
    if unsafe { try_resolve_mp3() } {
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 1) };
        crate::debug_log(b"[OASIS] using sceMp3 backend");
        return true;
    }
    crate::debug_log(b"[OASIS] sceMp3 NOT found, trying sceAudiocodec");

    // Step 4: Try sceAudiocodec (fallback -- frame-by-frame).
    if unsafe { try_resolve_codec() } {
        unsafe { core::ptr::write_volatile(&raw mut DECODER_BACKEND, 2) };
        crate::debug_log(b"[OASIS] using sceAudiocodec backend");
        return true;
    }
    crate::debug_log(b"[OASIS] NO decoder backend available");

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

    // Reserve audio channel.
    let channel = unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN)
        {
            f(-1, MP3_SAMPLES_PER_FRAME, AUDIO_FORMAT_STEREO)
        } else {
            return 1;
        }
    };
    if channel < 0 {
        crate::debug_log(b"[OASIS] audio channel reserve failed");
        return 1;
    }
    crate::debug_log(b"[OASIS] audio channel reserved");

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
        unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN)
            {
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
    if file_size == 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    static mut READ_BUF: [u8; READ_BUF_SIZE] = [0u8; READ_BUF_SIZE];
    static mut C_PCM_BUF: [i16; 1152 * 2] = [0i16; 1152 * 2];
    static mut CODEC_BUF: [u32; CODEC_BUF_WORDS] = [0u32; CODEC_BUF_WORDS];

    let codec = unsafe { (*(&raw mut CODEC_BUF)).as_mut_ptr() };
    unsafe {
        let mut i = 0;
        while i < CODEC_BUF_WORDS {
            *codec.add(i) = 0;
            i += 1;
        }
    }

    let mut edram_allocated = false;
    unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN)
        {
            f(codec, CODEC_TYPE_MP3);
        }
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_GET_EDRAM_FN)
        {
            if f(codec, CODEC_TYPE_MP3) >= 0 {
                edram_allocated = true;
            }
        }
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_INIT_FN)
        {
            if f(codec, CODEC_TYPE_MP3) < 0 {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(
                        &raw const CODEC_RELEASE_EDRAM_FN,
                    ) {
                        rel(codec);
                    }
                }
                psp::sys::sceIoClose(fd);
                return -1;
            }
        } else {
            psp::sys::sceIoClose(fd);
            return -1;
        }
    }

    let read_buf = unsafe { (*(&raw mut READ_BUF)).as_mut_ptr() };
    let pcm_buf = unsafe { (*(&raw mut C_PCM_BUF)).as_mut_ptr() };

    let mut file_pos: usize;
    let mut buf_valid: usize;
    let mut buf_pos: usize = 0;

    let initial_read = unsafe {
        psp::sys::sceIoRead(fd, read_buf as *mut _, READ_BUF_SIZE as u32)
    };
    if initial_read <= 0 {
        unsafe {
            if edram_allocated {
                if let Some(f) = core::ptr::read_volatile(
                    &raw const CODEC_RELEASE_EDRAM_FN,
                ) {
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
    let slice =
        unsafe { core::slice::from_raw_parts(read_buf, buf_valid) };
    let id3_skip = skip_id3v2(slice);
    if id3_skip > 0 && id3_skip < buf_valid {
        buf_pos = id3_skip;
    }

    let decode_fn = unsafe {
        match core::ptr::read_volatile(&raw const CODEC_DECODE_FN) {
            Some(f) => f,
            None => {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(
                        &raw const CODEC_RELEASE_EDRAM_FN,
                    ) {
                        rel(codec);
                    }
                }
                psp::sys::sceIoClose(fd);
                return -1;
            }
        }
    };

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

        unsafe {
            *codec.add(6) = read_buf.add(buf_pos) as u32;
            *codec.add(8) = pcm_buf as u32;
        }

        let ret = unsafe { decode_fn(codec, CODEC_TYPE_MP3) };
        if ret < 0 {
            buf_pos += 1;
            continue;
        }

        let consumed = unsafe { *codec.add(7) } as usize;
        if consumed == 0 {
            buf_pos += 1;
            continue;
        }
        buf_pos += consumed;

        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000)
            / 255;
        unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN)
            {
                f(channel, vol, vol);
            }
            if let Some(f) = core::ptr::read_volatile(
                &raw const AUDIO_OUTPUT_BLOCKING_FN,
            ) {
                let ret = f(channel, vol, pcm_buf as *const u8);
                if ret < 0 {
                    result = ret;
                    break;
                }
            }
        }
    }

    unsafe {
        if edram_allocated {
            if let Some(f) = core::ptr::read_volatile(
                &raw const CODEC_RELEASE_EDRAM_FN,
            ) {
                f(codec);
            }
        }
        psp::sys::sceIoClose(fd);
    }
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
