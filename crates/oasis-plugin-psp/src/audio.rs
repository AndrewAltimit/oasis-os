//! Background MP3 playback via runtime NID resolution.
//!
//! User-mode imports of `sceMp3*`/`sceAudio*` cause PRX load failure because
//! those module stubs aren't resolved in the game's kernel context. Instead we
//! use `psp::hook::find_function()` to resolve audio and MP3 driver NIDs at
//! runtime, then drive playback from a dedicated kernel thread.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::overlay;

// ---------------------------------------------------------------------------
// sceAudio driver NIDs
// ---------------------------------------------------------------------------

/// sceAudioChReserve(channel, samplecount, format) -> channel
const NID_AUDIO_CH_RESERVE: u32 = 0x5EC81C55;
/// sceAudioOutputBlocking(channel, vol, buf) -> bytes
const NID_AUDIO_OUTPUT_BLOCKING: u32 = 0x136CAF51;
/// sceAudioChRelease(channel) -> 0
const NID_AUDIO_CH_RELEASE: u32 = 0x6FC46853;
/// sceAudioChangeChannelVolume(channel, volL, volR) -> 0
const NID_AUDIO_SET_CH_VOL: u32 = 0xB7E1D8E7;

/// Module/library pairs for sceAudio driver.
const AUDIO_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceAudio_Driver\0", b"sceAudio\0"),
    (b"sceAudio_Driver\0", b"sceAudio_driver\0"),
];

// ---------------------------------------------------------------------------
// sceMp3 driver NIDs
// ---------------------------------------------------------------------------

/// sceMp3InitResource() -> 0
const NID_MP3_INIT_RESOURCE: u32 = 0x35750070;
/// sceMp3TermResource() -> 0
const NID_MP3_TERM_RESOURCE: u32 = 0xD0A56296;
/// sceMp3ReserveMp3Handle(init_struct) -> handle
const NID_MP3_RESERVE_HANDLE: u32 = 0x7F2A1880;
/// sceMp3ReleaseMp3Handle(handle) -> 0
const NID_MP3_RELEASE_HANDLE: u32 = 0x0DB149F4;
/// sceMp3Init(handle) -> 0
const NID_MP3_INIT: u32 = 0x44E07129;
/// sceMp3Decode(handle, out_buf_ptr) -> bytes decoded
const NID_MP3_DECODE: u32 = 0xD021C0FB;
/// sceMp3CheckStreamDataNeeded(handle) -> bool
const NID_MP3_CHECK_NEED_DATA: u32 = 0xD8F54A51;
/// sceMp3GetInfoToAddStreamData(handle, dst, to_write, src_pos) -> 0
const NID_MP3_GET_INFO_TO_ADD: u32 = 0x732B042A;
/// sceMp3NotifyAddStreamData(handle, size) -> 0
const NID_MP3_NOTIFY_ADD_DATA: u32 = 0x0DB149F4;

/// Module/library pairs for sceMp3 driver.
const MP3_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceMp3_Library\0", b"sceMp3\0"),
    (b"sceMp3\0", b"sceMp3\0"),
];

// ---------------------------------------------------------------------------
// Resolved function pointers (set once during init)
// ---------------------------------------------------------------------------

// SAFETY: All statics are set once during single-threaded init, then
// read-only from the audio thread. The audio thread is started after init.

static mut AUDIO_CH_RESERVE_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;
static mut AUDIO_OUTPUT_BLOCKING_FN: Option<
    unsafe extern "C" fn(i32, i32, *const u8) -> i32,
> = None;
static mut AUDIO_CH_RELEASE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut AUDIO_SET_CH_VOL_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;

static mut MP3_INIT_RESOURCE_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut MP3_TERM_RESOURCE_FN: Option<unsafe extern "C" fn() -> i32> = None;
static mut MP3_RESERVE_HANDLE_FN: Option<
    unsafe extern "C" fn(*const Mp3InitStruct) -> i32,
> = None;
static mut MP3_RELEASE_HANDLE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
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

// ---------------------------------------------------------------------------
// Audio state (atomics for cross-thread communication)
// ---------------------------------------------------------------------------

/// Audio commands from overlay to audio thread.
/// 0=none, 1=toggle play/pause, 2=next track, 3=prev track
static AUDIO_CMD: AtomicU8 = AtomicU8::new(0);

/// Volume level (0-255, default 128).
static AUDIO_VOLUME: AtomicU8 = AtomicU8::new(128);

/// Audio state: 0=stopped, 1=playing, 2=paused.
static AUDIO_STATE: AtomicU8 = AtomicU8::new(0);

/// Whether audio driver was successfully initialized.
static AUDIO_AVAILABLE: AtomicU8 = AtomicU8::new(0);

/// Current track display name (read from display hook, written by audio thread).
/// SAFETY: Written only by audio thread, read only by display hook. Race
/// condition on partial reads is cosmetic only (garbled track name for one frame).
static mut TRACK_NAME: [u8; 48] = [0u8; 48];

// ---------------------------------------------------------------------------
// MP3 decoder structures
// ---------------------------------------------------------------------------

/// sceMp3 init structure, passed to sceMp3ReserveMp3Handle.
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

/// PSP audio format constants.
const AUDIO_FORMAT_STEREO: i32 = 0;

/// MP3 decoded sample count (1152 samples per MP3 frame, standard MPEG1 Layer3).
const MP3_SAMPLES_PER_FRAME: i32 = 1152;

/// Maximum playlist size.
const MAX_PLAYLIST: usize = 32;

/// Maximum filename length including path.
const MAX_FILENAME: usize = 128;

/// Maximum recursion depth for directory scanning.
const MAX_SCAN_DEPTH: usize = 4;

/// MP3 stream buffer size (64KB).
const MP3_BUF_SIZE: usize = 64 * 1024;

/// PCM decode buffer size (enough for several decoded frames).
const PCM_BUF_SIZE: usize = MP3_SAMPLES_PER_FRAME as usize * 4 * 4;

/// File read buffer size (reserved for future chunked reading).
#[allow(dead_code)]
const FILE_READ_BUF_SIZE: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// Playlist data
// ---------------------------------------------------------------------------

/// Playlist filenames (static, filled during init).
static mut PLAYLIST: [[u8; MAX_FILENAME]; MAX_PLAYLIST] =
    [[0u8; MAX_FILENAME]; MAX_PLAYLIST];
static mut PLAYLIST_LEN: usize = 0;
static mut CURRENT_TRACK: usize = 0;

// ---------------------------------------------------------------------------
// Public API (called from overlay, writes atomics)
// ---------------------------------------------------------------------------

/// Get the current track's display name.
pub fn current_track_name() -> &'static [u8] {
    // SAFETY: Cosmetic-only race. Read a snapshot via raw pointer.
    unsafe {
        core::slice::from_raw_parts(
            (&raw const TRACK_NAME).cast::<u8>(),
            48,
        )
    }
}

/// Get current audio state: 0=stopped, 1=playing, 2=paused.
pub fn audio_state() -> u8 {
    AUDIO_STATE.load(Ordering::Relaxed)
}

/// Toggle play/pause.
pub fn toggle_playback() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(1, Ordering::Relaxed);
}

/// Skip to next track.
pub fn next_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(2, Ordering::Relaxed);
}

/// Skip to previous track.
pub fn prev_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(3, Ordering::Relaxed);
}

/// Increase volume.
pub fn volume_up() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_add(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = 0;
    let s = b"Vol: ";
    let mut i = 0;
    while i < s.len() {
        buf[p] = s[i];
        p += 1;
        i += 1;
    }
    p = write_u8_decimal(&mut buf, p, new);
    overlay::show_osd(&buf[..p]);
}

/// Decrease volume.
pub fn volume_down() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_sub(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = 0;
    let s = b"Vol: ";
    let mut i = 0;
    while i < s.len() {
        buf[p] = s[i];
        p += 1;
        i += 1;
    }
    p = write_u8_decimal(&mut buf, p, new);
    overlay::show_osd(&buf[..p]);
}

// ---------------------------------------------------------------------------
// Init and audio thread
// ---------------------------------------------------------------------------

/// Resolve audio driver function pointers. Returns true if enough
/// functions were resolved for playback.
unsafe fn init_audio_drivers() -> bool {
    // Resolve sceAudio driver functions.
    unsafe {
        for &(module, library) in AUDIO_MODULES {
            if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN).is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_AUDIO_CH_RESERVE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut AUDIO_CH_RESERVE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(
                &raw const AUDIO_OUTPUT_BLOCKING_FN,
            )
            .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_AUDIO_OUTPUT_BLOCKING,
                ) {
                    core::ptr::write_volatile(
                        &raw mut AUDIO_OUTPUT_BLOCKING_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const AUDIO_CH_RELEASE_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_AUDIO_CH_RELEASE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut AUDIO_CH_RELEASE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(),
                    library.as_ptr(),
                    NID_AUDIO_SET_CH_VOL,
                ) {
                    core::ptr::write_volatile(
                        &raw mut AUDIO_SET_CH_VOL_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
        }

        if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN).is_none()
            || core::ptr::read_volatile(
                &raw const AUDIO_OUTPUT_BLOCKING_FN,
            )
            .is_none()
        {
            crate::debug_log(b"[OASIS] sceAudio driver NOT found");
            // Try loading the audio module explicitly.
            let mod_id = psp::sys::sceKernelLoadModule(
                b"flash0:/kd/audio.prx\0".as_ptr(),
                0,
                core::ptr::null_mut(),
            );
            if mod_id.0 >= 0 {
                psp::sys::sceKernelStartModule(
                    mod_id, 0, core::ptr::null_mut(),
                    core::ptr::null_mut(), core::ptr::null_mut(),
                );
                crate::debug_log(b"[OASIS] loaded audio.prx");
                // Retry resolution.
                for &(module, library) in AUDIO_MODULES {
                    if core::ptr::read_volatile(
                        &raw const AUDIO_CH_RESERVE_FN,
                    )
                    .is_none()
                    {
                        if let Some(ptr) = psp::hook::find_function(
                            module.as_ptr(), library.as_ptr(),
                            NID_AUDIO_CH_RESERVE,
                        ) {
                            core::ptr::write_volatile(
                                &raw mut AUDIO_CH_RESERVE_FN,
                                Some(core::mem::transmute(ptr)),
                            );
                        }
                    }
                    if core::ptr::read_volatile(
                        &raw const AUDIO_OUTPUT_BLOCKING_FN,
                    )
                    .is_none()
                    {
                        if let Some(ptr) = psp::hook::find_function(
                            module.as_ptr(), library.as_ptr(),
                            NID_AUDIO_OUTPUT_BLOCKING,
                        ) {
                            core::ptr::write_volatile(
                                &raw mut AUDIO_OUTPUT_BLOCKING_FN,
                                Some(core::mem::transmute(ptr)),
                            );
                        }
                    }
                }
            }
        }

        if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN).is_none()
            || core::ptr::read_volatile(
                &raw const AUDIO_OUTPUT_BLOCKING_FN,
            )
            .is_none()
        {
            crate::debug_log(b"[OASIS] audio: critical fns missing");
            return false;
        }
        crate::debug_log(b"[OASIS] audio driver resolved");
    }

    // Resolve sceMp3 driver functions.
    unsafe {
        resolve_mp3_fns();

        if core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
            .is_none()
        {
            // Try loading MP3 modules explicitly.
            let modules: &[&[u8]] = &[
                b"flash0:/kd/mpeg.prx\0",
                b"flash0:/kd/mpegbase.prx\0",
                b"flash0:/kd/libmp3.prx\0",
            ];
            for path in modules {
                let mod_id = psp::sys::sceKernelLoadModule(
                    path.as_ptr(),
                    0,
                    core::ptr::null_mut(),
                );
                if mod_id.0 >= 0 {
                    psp::sys::sceKernelStartModule(
                        mod_id, 0, core::ptr::null_mut(),
                        core::ptr::null_mut(), core::ptr::null_mut(),
                    );
                }
            }
            crate::debug_log(b"[OASIS] loaded mp3 modules");
            resolve_mp3_fns();
        }

        if core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
            .is_none()
            || core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN)
                .is_none()
            || core::ptr::read_volatile(&raw const MP3_DECODE_FN).is_none()
        {
            crate::debug_log(b"[OASIS] mp3 driver NOT found");
            return false;
        }
        crate::debug_log(b"[OASIS] mp3 driver resolved");
    }

    true
}

/// Attempt to resolve all sceMp3 function pointers.
unsafe fn resolve_mp3_fns() {
    unsafe {
        for &(module, library) in MP3_MODULES {
            if core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_INIT_RESOURCE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_INIT_RESOURCE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_TERM_RESOURCE_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_TERM_RESOURCE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_TERM_RESOURCE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_RESERVE_HANDLE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_RESERVE_HANDLE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_RELEASE_HANDLE_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_RELEASE_HANDLE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_RELEASE_HANDLE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_INIT_FN).is_none() {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_INIT,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_INIT_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_DECODE_FN).is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), NID_MP3_DECODE,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_DECODE_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_CHECK_NEED_DATA_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(),
                    NID_MP3_CHECK_NEED_DATA,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_CHECK_NEED_DATA_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_GET_INFO_TO_ADD_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(),
                    NID_MP3_GET_INFO_TO_ADD,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_GET_INFO_TO_ADD_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
            if core::ptr::read_volatile(&raw const MP3_NOTIFY_ADD_DATA_FN)
                .is_none()
            {
                if let Some(ptr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(),
                    NID_MP3_NOTIFY_ADD_DATA,
                ) {
                    core::ptr::write_volatile(
                        &raw mut MP3_NOTIFY_ADD_DATA_FN,
                        Some(core::mem::transmute(ptr)),
                    );
                }
            }
        }
    }
}

/// Scan the music directory for .mp3 files and populate the playlist.
/// Recursively descends into subdirectories up to `MAX_SCAN_DEPTH` levels.
///
/// # Safety
/// Must be called from a thread context where sceIo calls work.
unsafe fn scan_playlist() {
    let config = crate::config::get_config();

    // SAFETY: PLAYLIST is only written during single-threaded init.
    unsafe {
        core::ptr::write_volatile(&raw mut PLAYLIST_LEN, 0);
    }

    // Start recursive scan from the music directory.
    unsafe {
        scan_dir_recursive(&config.music_dir, config.music_dir_len, 0);
    }

    let mut log_buf = [0u8; 48];
    let mut p = 0;
    let s = b"[OASIS] found ";
    let mut i = 0;
    while i < s.len() {
        log_buf[p] = s[i];
        p += 1;
        i += 1;
    }
    // SAFETY: PLAYLIST_LEN set above via raw pointer.
    p = write_u8_decimal(
        &mut log_buf,
        p,
        unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) as u8 },
    );
    let s2 = b" mp3 files";
    i = 0;
    while i < s2.len() {
        log_buf[p] = s2[i];
        p += 1;
        i += 1;
    }
    crate::debug_log(&log_buf[..p]);
}

/// Recursively scan a directory for .mp3 files.
///
/// `dir_path` is the directory path (null-terminated in the backing array).
/// `dir_len` is the length of the path NOT including null, but INCLUDING
/// the trailing `/`.
///
/// # Safety
/// Requires sceIo APIs to be available (thread context).
unsafe fn scan_dir_recursive(dir_path: &[u8], dir_len: usize, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }

    let pl_len = unsafe {
        core::ptr::read_volatile(&raw const PLAYLIST_LEN)
    };
    if pl_len >= MAX_PLAYLIST {
        return;
    }

    // SAFETY: sceIoDopen with null-terminated path.
    let dfd = unsafe {
        psp::sys::sceIoDopen(dir_path.as_ptr())
    };
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

            // Get filename and length.
            let name_ptr = dirent.d_name.as_ptr() as *const u8;
            let mut name_len = 0usize;
            while name_len < 256 {
                if *name_ptr.add(name_len) == 0 {
                    break;
                }
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

            // Check if this entry is a directory (st_attr bit 0x0010).
            let is_dir =
                (dirent.d_stat.st_attr.bits() & 0x0010) != 0;

            if is_dir {
                // Build subdirectory path: dir + name + '/' + '\0'
                // Need dir_len + name_len + 1 (slash) + 1 (null)
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

                // Recurse into subdirectory.
                scan_dir_recursive(&sub_path, sub_len, depth + 1);
            } else {
                // Check if filename ends with ".mp3" (case-insensitive).
                if name_len < 5 {
                    continue;
                }
                let ext_start = name_len - 4;
                let c1 = *name_ptr.add(ext_start);
                let c2 = (*name_ptr.add(ext_start + 1))
                    .to_ascii_lowercase();
                let c3 = (*name_ptr.add(ext_start + 2))
                    .to_ascii_lowercase();
                let c4 = (*name_ptr.add(ext_start + 3))
                    .to_ascii_lowercase();
                if c1 != b'.'
                    || c2 != b'm'
                    || c3 != b'p'
                    || c4 != b'3'
                {
                    continue;
                }

                // Build full path: dir + filename + '\0'
                let total_len = dir_len + name_len;
                if total_len + 1 > MAX_FILENAME {
                    continue;
                }

                let entry =
                    &mut (*(&raw mut PLAYLIST))[pl_len];
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
                entry[j + k] = 0; // null terminate

                core::ptr::write_volatile(
                    &raw mut PLAYLIST_LEN,
                    pl_len + 1,
                );
            }
        }
        psp::sys::sceIoDclose(dfd);
    }
}

/// Set the track name display from a full file path.
///
/// # Safety
/// TRACK_NAME is written only by the audio thread.
unsafe fn set_track_name(path: &[u8]) {
    unsafe {
        // Find the last '/' to get just the filename.
        let mut last_slash = 0;
        let mut i = 0;
        while i < path.len() && path[i] != 0 {
            if path[i] == b'/' {
                last_slash = i + 1;
            }
            i += 1;
        }
        let name = &path[last_slash..];

        // Copy into TRACK_NAME, stripping .mp3 extension.
        let mut len = 0;
        while len < name.len() && name[len] != 0 {
            len += 1;
        }
        // Remove .mp3 extension if present.
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
        (*(&raw mut TRACK_NAME))[copy_len] = 0;
        // Clear remainder.
        while j < 48 {
            (*(&raw mut TRACK_NAME))[j] = 0;
            j += 1;
        }
    }
}

/// Audio thread entry point.
///
/// # Safety
/// Called as a kernel thread function. All sceIo/sceAudio/sceMp3 calls are
/// valid from thread context.
unsafe extern "C" fn audio_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    // Brief delay for system to settle.
    unsafe { psp::sys::sceKernelDelayThread(1_000_000) };

    // Initialize audio drivers via runtime NID resolution.
    if !unsafe { init_audio_drivers() } {
        crate::debug_log(b"[OASIS] audio init failed");
        return 1;
    }

    AUDIO_AVAILABLE.store(1, Ordering::Relaxed);

    // Scan playlist.
    unsafe { scan_playlist() };

    // SAFETY: PLAYLIST_LEN set during scan above.
    if unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) } == 0 {
        crate::debug_log(b"[OASIS] no mp3 files found");
        return 0;
    }

    // Init MP3 resource manager.
    unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const MP3_INIT_RESOURCE_FN)
        {
            let ret = f();
            if ret < 0 {
                crate::debug_log(b"[OASIS] mp3 init resource failed");
                return 1;
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

    // Start playing.
    AUDIO_STATE.store(1, Ordering::Relaxed);
    unsafe { core::ptr::write_volatile(&raw mut CURRENT_TRACK, 0) };

    // Main playback loop.
    loop {
        // Check for commands.
        let cmd = AUDIO_CMD.swap(0, Ordering::Relaxed);
        match cmd {
            1 => {
                // Toggle play/pause.
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
                // Next track.
                let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
                let pl_len =
                    core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                core::ptr::write_volatile(
                    &raw mut CURRENT_TRACK,
                    (cur + 1) % pl_len,
                );
            },
            3 => unsafe {
                // Previous track.
                let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
                let pl_len =
                    core::ptr::read_volatile(&raw const PLAYLIST_LEN);
                if cur == 0 {
                    core::ptr::write_volatile(
                        &raw mut CURRENT_TRACK,
                        pl_len - 1,
                    );
                } else {
                    core::ptr::write_volatile(
                        &raw mut CURRENT_TRACK,
                        cur - 1,
                    );
                }
            },
            _ => {}
        }

        // If paused, sleep briefly and continue.
        if AUDIO_STATE.load(Ordering::Relaxed) == 2 {
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Play the current track.
        let track_idx =
            unsafe { core::ptr::read_volatile(&raw const CURRENT_TRACK) };
        let track_path =
            unsafe { &(*(&raw const PLAYLIST))[track_idx] };
        unsafe { set_track_name(track_path) };

        let result = unsafe { play_track(track_path, channel) };
        if result < 0 {
            crate::debug_log(b"[OASIS] track playback error");
        }

        // Advance to next track.
        unsafe {
            let cur = core::ptr::read_volatile(&raw const CURRENT_TRACK);
            let pl_len =
                core::ptr::read_volatile(&raw const PLAYLIST_LEN);
            core::ptr::write_volatile(
                &raw mut CURRENT_TRACK,
                (cur + 1) % pl_len,
            );
        }
    }
}

/// Play a single MP3 track to completion (or until a skip command).
///
/// Returns 0 on success, negative on error.
///
/// # Safety
/// Must be called from the audio thread with valid resolved function pointers.
unsafe fn play_track(path: &[u8], channel: i32) -> i32 {
    // Open the MP3 file.
    // SAFETY: path is null-terminated.
    let fd = unsafe {
        psp::sys::sceIoOpen(
            path.as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        return -1;
    }

    // Get file size.
    let file_size = unsafe {
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End)
    } as i32;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };

    if file_size <= 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // Allocate buffers on stack (within kernel thread's stack budget).
    // The MP3 stream buffer and PCM buffer are static to avoid stack overflow.
    static mut MP3_BUF: [u8; MP3_BUF_SIZE] = [0u8; MP3_BUF_SIZE];
    static mut PCM_BUF: [u8; PCM_BUF_SIZE] = [0u8; PCM_BUF_SIZE];

    let mp3_buf_ptr = unsafe { (*(&raw mut MP3_BUF)).as_mut_ptr() };
    let pcm_buf_ptr = unsafe { (*(&raw mut PCM_BUF)).as_mut_ptr() };

    // Set up MP3 init struct.
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

    // Reserve MP3 handle.
    let handle = unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const MP3_RESERVE_HANDLE_FN)
        {
            f(&init)
        } else {
            psp::sys::sceIoClose(fd);
            return -1;
        }
    };
    if handle < 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // Fill initial stream data.
    unsafe { fill_stream_data(handle, fd) };

    // Init the MP3 decoder for this handle.
    let ret = unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const MP3_INIT_FN)
        {
            f(handle)
        } else {
            -1
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

    // Decode and output loop.
    let mut result = 0i32;
    loop {
        // Check for skip/toggle commands.
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 {
            // Skip -- don't consume the command, let the outer loop handle it.
            break;
        }
        if cmd == 1 {
            // Toggle pause.
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

        // If paused, sleep and loop.
        if AUDIO_STATE.load(Ordering::Relaxed) == 2 {
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Check if the decoder needs more stream data.
        let needs_data = unsafe {
            if let Some(f) = core::ptr::read_volatile(
                &raw const MP3_CHECK_NEED_DATA_FN,
            ) {
                f(handle)
            } else {
                0
            }
        };
        if needs_data > 0 {
            let filled = unsafe { fill_stream_data(handle, fd) };
            if filled <= 0 {
                // End of file -- no more data to feed.
                break;
            }
        }

        // Decode one MP3 frame.
        let mut pcm_out: *const i16 = core::ptr::null();
        let decoded = unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const MP3_DECODE_FN)
            {
                f(handle, &mut pcm_out)
            } else {
                break;
            }
        };

        if decoded <= 0 {
            // Decoder done or error.
            break;
        }

        // Apply volume and output.
        let vol_raw = AUDIO_VOLUME.load(Ordering::Relaxed) as i32;
        // PSP volume: 0-0x8000 per channel.
        let vol = (vol_raw * 0x8000) / 255;

        // Set channel volume.
        unsafe {
            if let Some(f) =
                core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN)
            {
                f(channel, vol, vol);
            }
        }

        // Output decoded PCM (blocking).
        unsafe {
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

    // Cleanup.
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

/// Fill the MP3 stream buffer with data from the file.
///
/// Returns number of bytes added, or <= 0 on EOF/error.
///
/// # Safety
/// Must be called with valid handle and fd.
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

        let ret = get_info(handle, &mut dst_ptr, &mut to_write, &mut src_pos);
        if ret < 0 || to_write <= 0 {
            return 0;
        }

        // Seek to the position the decoder expects.
        psp::sys::sceIoLseek(fd, src_pos as i64, psp::sys::IoWhence::Set);

        // Read file data into the stream buffer.
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

/// Start the background audio thread.
pub fn start_audio_thread() {
    crate::debug_log(b"[OASIS] starting audio thread...");

    // SAFETY: Creating a kernel thread for audio playback.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisAudio\0".as_ptr(),
            audio_thread_entry,
            0x1E, // slightly lower priority than ctrl thread
            0x4000, // 16KB stack for decode buffers
            psp::sys::ThreadAttributes::empty(), // kernel thread
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

/// Write a u8 as decimal ASCII into a buffer.
fn write_u8_decimal(buf: &mut [u8], pos: usize, val: u8) -> usize {
    let val = val as u32;
    if val == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            return pos + 1;
        }
        return pos;
    }
    let mut digits = [0u8; 3];
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
