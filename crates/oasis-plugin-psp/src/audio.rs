//! Background MP3 playback via runtime NID resolution.
//!
//! User-mode imports of `sceMp3*`/`sceAudio*` cause PRX load failure because
//! those module stubs aren't resolved in the game's kernel context. Instead we
//! use `psp::hook::find_function()` to resolve sceAudio and sceAudiocodec
//! driver NIDs at runtime, then drive playback from a dedicated kernel thread.
//!
//! sceMp3 is a user-mode library that `sctrlHENFindFunction` cannot resolve
//! from kernel context. We use the lower-level `sceAudiocodec` API instead,
//! which is a kernel-accessible codec driver that supports MP3 (type 0x1002).

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
    (b"sceAudio_Driver\0", b"sceAudio_driver\0"),
    (b"sceAudio_Driver\0", b"sceAudio\0"),
    (b"sceAudio_Service\0", b"sceAudio_driver\0"),
    (b"sceAudio_Service\0", b"sceAudio\0"),
];

// ---------------------------------------------------------------------------
// sceAudiocodec driver NIDs (kernel-accessible, unlike sceMp3)
// ---------------------------------------------------------------------------

/// sceAudiocodecCheckNeedMem(buffer, type) -> 0
const NID_CODEC_CHECK_NEED_MEM: u32 = 0x9D3F790C;
/// sceAudiocodecInit(buffer, type) -> 0
const NID_CODEC_INIT: u32 = 0x5B37EB1D;
/// sceAudiocodecDecode(buffer, type) -> 0
const NID_CODEC_DECODE: u32 = 0x70A703F8;
/// sceAudiocodecGetEDRAM(buffer, type) -> 0
const NID_CODEC_GET_EDRAM: u32 = 0x3A20A200;
/// sceAudiocodecReleaseEDRAM(buffer) -> 0
const NID_CODEC_RELEASE_EDRAM: u32 = 0x29681260;

/// Module/library pairs for sceAudiocodec driver.
const CODEC_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceAudiocodec_Driver\0", b"sceAudiocodec\0"),
    (b"avcodec\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec\0", b"sceAudiocodec\0"),
    (b"sceAVcodec_driver\0", b"sceAudiocodec\0"),
];

/// MP3 codec type for sceAudiocodec.
const CODEC_TYPE_MP3: i32 = 0x1002;

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
#[allow(dead_code)]
static mut AUDIO_CH_RELEASE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut AUDIO_SET_CH_VOL_FN: Option<
    unsafe extern "C" fn(i32, i32, i32) -> i32,
> = None;

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
// Codec buffer and constants
// ---------------------------------------------------------------------------

/// SceAudiocodecCodec buffer: 128 bytes (32 u32 values).
///
/// Key offsets (from PPSSPP source):
///   [0]  = unk_init
///   [6]  = inBuf pointer (input MP3 frame data)
///   [7]  = srcBytesRead (output: bytes consumed)
///   [8]  = outBuf pointer (output PCM buffer)
///   [9]  = dstSamplesWritten (output: samples decoded)
const CODEC_BUF_WORDS: usize = 32;

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

/// Read buffer for streaming MP3 data from file.
/// 32KB is enough for many MP3 frames (typical frame ~417 bytes at 128kbps).
const READ_BUF_SIZE: usize = 32 * 1024;

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

/// Resolve a single NID from multiple module/library pairs.
unsafe fn resolve_nid(
    modules: &[(&[u8], &[u8])],
    nid: u32,
) -> Option<*mut u8> {
    for &(module, library) in modules {
        // SAFETY: find_function requires kernel mode and null-terminated strings.
        if let Some(ptr) = unsafe {
            psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid)
        } {
            return Some(ptr);
        }
    }
    None
}

/// Resolve audio driver function pointers. Returns true if enough
/// functions were resolved for playback.
unsafe fn init_audio_drivers() -> bool {
    // Resolve sceAudio driver functions.
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
                if core::ptr::read_volatile(&raw const AUDIO_CH_RESERVE_FN)
                    .is_none()
                {
                    if let Some(ptr) =
                        resolve_nid(AUDIO_MODULES, NID_AUDIO_CH_RESERVE)
                    {
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
                    if let Some(ptr) =
                        resolve_nid(AUDIO_MODULES, NID_AUDIO_OUTPUT_BLOCKING)
                    {
                        core::ptr::write_volatile(
                            &raw mut AUDIO_OUTPUT_BLOCKING_FN,
                            Some(core::mem::transmute(ptr)),
                        );
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

    // Resolve sceAudiocodec driver functions (kernel-accessible MP3 codec).
    unsafe {
        resolve_codec_fns();

        if core::ptr::read_volatile(&raw const CODEC_DECODE_FN).is_none() {
            // Try loading avcodec module explicitly.
            let modules: &[&[u8]] = &[
                b"flash0:/kd/avcodec.prx\0",
                b"flash0:/kd/audiocodec.prx\0",
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
            crate::debug_log(b"[OASIS] loaded codec modules");
            resolve_codec_fns();
        }

        if core::ptr::read_volatile(&raw const CODEC_DECODE_FN).is_none()
            || core::ptr::read_volatile(&raw const CODEC_INIT_FN).is_none()
        {
            crate::debug_log(b"[OASIS] codec driver NOT found");
            return false;
        }
        crate::debug_log(b"[OASIS] codec driver resolved");
    }

    true
}

/// Attempt to resolve all sceAudiocodec function pointers.
unsafe fn resolve_codec_fns() {
    unsafe {
        if core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN)
            .is_none()
        {
            if let Some(ptr) =
                resolve_nid(CODEC_MODULES, NID_CODEC_CHECK_NEED_MEM)
            {
                core::ptr::write_volatile(
                    &raw mut CODEC_CHECK_NEED_MEM_FN,
                    Some(core::mem::transmute(ptr)),
                );
            }
        }
        if core::ptr::read_volatile(&raw const CODEC_INIT_FN).is_none() {
            if let Some(ptr) = resolve_nid(CODEC_MODULES, NID_CODEC_INIT) {
                core::ptr::write_volatile(
                    &raw mut CODEC_INIT_FN,
                    Some(core::mem::transmute(ptr)),
                );
            }
        }
        if core::ptr::read_volatile(&raw const CODEC_DECODE_FN).is_none() {
            if let Some(ptr) = resolve_nid(CODEC_MODULES, NID_CODEC_DECODE) {
                core::ptr::write_volatile(
                    &raw mut CODEC_DECODE_FN,
                    Some(core::mem::transmute(ptr)),
                );
            }
        }
        if core::ptr::read_volatile(&raw const CODEC_GET_EDRAM_FN).is_none()
        {
            if let Some(ptr) =
                resolve_nid(CODEC_MODULES, NID_CODEC_GET_EDRAM)
            {
                core::ptr::write_volatile(
                    &raw mut CODEC_GET_EDRAM_FN,
                    Some(core::mem::transmute(ptr)),
                );
            }
        }
        if core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN)
            .is_none()
        {
            if let Some(ptr) =
                resolve_nid(CODEC_MODULES, NID_CODEC_RELEASE_EDRAM)
            {
                core::ptr::write_volatile(
                    &raw mut CODEC_RELEASE_EDRAM_FN,
                    Some(core::mem::transmute(ptr)),
                );
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
/// Called as a kernel thread function. All sceIo/sceAudio/sceAudiocodec
/// calls are valid from thread context.
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

    // Reserve audio channel (1152 stereo samples per MP3 frame).
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

    // Start in playing state if autoplay is enabled, otherwise wait
    // for user command from the overlay menu.
    let autoplay = crate::config::get_config().autoplay;
    if autoplay {
        AUDIO_STATE.store(1, Ordering::Relaxed);
    } else {
        AUDIO_STATE.store(0, Ordering::Relaxed);
    }
    unsafe { core::ptr::write_volatile(&raw mut CURRENT_TRACK, 0) };

    // Main playback loop.
    loop {
        // Check for commands.
        let cmd = AUDIO_CMD.swap(0, Ordering::Relaxed);
        match cmd {
            1 => {
                // Toggle play/pause (also starts from stopped state).
                let state = AUDIO_STATE.load(Ordering::Relaxed);
                if state == 1 {
                    AUDIO_STATE.store(2, Ordering::Relaxed);
                    overlay::show_osd(b"Paused");
                } else {
                    // From stopped (0) or paused (2) -> playing.
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

        // If stopped or paused, sleep briefly and continue.
        let state = AUDIO_STATE.load(Ordering::Relaxed);
        if state == 0 || state == 2 {
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

/// Skip past an ID3v2 tag at the start of an MP3 file.
///
/// Returns the byte offset where MP3 audio data begins (0 if no tag).
fn skip_id3v2(data: &[u8]) -> usize {
    if data.len() < 10 {
        return 0;
    }
    // ID3v2 header: "ID3" + version(2) + flags(1) + size(4 synchsafe)
    if data[0] != b'I' || data[1] != b'D' || data[2] != b'3' {
        return 0;
    }
    // Synchsafe integer: each byte uses 7 bits.
    let size = ((data[6] as u32) << 21)
        | ((data[7] as u32) << 14)
        | ((data[8] as u32) << 7)
        | (data[9] as u32);
    10 + size as usize
}

/// Find the next MP3 sync word (0xFFE0 mask) in a buffer.
///
/// Returns the offset of the sync word, or None if not found.
fn find_mp3_sync(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < data.len() {
        if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
            // Verify it's a valid MP3 frame header (not just random 0xFFEx).
            // Check MPEG version != reserved and layer != reserved.
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

/// Play a single MP3 track to completion (or until a skip command).
///
/// Uses `sceAudiocodec` for frame-by-frame decoding. The codec handles
/// MP3 frame parsing internally -- we just point it at the start of each
/// frame and it reports how many bytes it consumed.
///
/// Returns 0 on success, negative on error.
///
/// # Safety
/// Must be called from the audio thread with valid resolved function ptrs.
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
    } as usize;
    unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };

    if file_size == 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        return -1;
    }

    // Static buffers for streaming (in BSS, zero-cost binary size).
    static mut READ_BUF: [u8; READ_BUF_SIZE] = [0u8; READ_BUF_SIZE];
    // PCM output: 1152 stereo i16 samples = 4608 bytes.
    static mut PCM_BUF: [i16; 1152 * 2] = [0i16; 1152 * 2];

    // Initialize sceAudiocodec for MP3.
    // SAFETY: CODEC_BUF is only used from the audio thread.
    static mut CODEC_BUF: [u32; CODEC_BUF_WORDS] = [0u32; CODEC_BUF_WORDS];
    let codec = unsafe { (*(&raw mut CODEC_BUF)).as_mut_ptr() };

    // Zero the codec buffer.
    unsafe {
        let mut i = 0;
        while i < CODEC_BUF_WORDS {
            *codec.add(i) = 0;
            i += 1;
        }
    }

    // CheckNeedMem + GetEDRAM + Init.
    let mut edram_allocated = false;
    unsafe {
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN)
        {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret < 0 {
                crate::debug_log(b"[OASIS] codec CheckNeedMem failed");
            }
        }
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_GET_EDRAM_FN)
        {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret < 0 {
                crate::debug_log(b"[OASIS] codec GetEDRAM failed");
            } else {
                edram_allocated = true;
            }
        }
        if let Some(f) =
            core::ptr::read_volatile(&raw const CODEC_INIT_FN)
        {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret < 0 {
                crate::debug_log(b"[OASIS] codec Init failed");
                // Release EDRAM if allocated.
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

    crate::debug_log(b"[OASIS] codec initialized for track");

    // Read initial chunk and skip ID3v2 tag.
    let read_buf = unsafe { (*(&raw mut READ_BUF)).as_mut_ptr() };
    let pcm_buf = unsafe { (*(&raw mut PCM_BUF)).as_mut_ptr() };

    let mut file_pos: usize = 0;
    let mut buf_valid: usize = 0; // bytes of valid data in READ_BUF
    let mut buf_pos: usize = 0; // current read position within READ_BUF

    // Read first chunk.
    let initial_read = unsafe {
        psp::sys::sceIoRead(
            fd,
            read_buf as *mut _,
            READ_BUF_SIZE as u32,
        )
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

    // Skip ID3v2 tag if present.
    let read_buf_slice = unsafe {
        core::slice::from_raw_parts(read_buf, buf_valid)
    };
    let id3_skip = skip_id3v2(read_buf_slice);
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
    let mut frames_decoded = 0u32;

    // Decode loop: find MP3 sync, point codec at it, decode, output PCM.
    loop {
        // Check for skip/toggle commands.
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd == 2 || cmd == 3 {
            // Skip -- don't consume the command, let outer loop handle it.
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

        // Refill buffer if running low (< 2KB remaining).
        if buf_valid - buf_pos < 2048 && file_pos < file_size {
            // Shift remaining data to front.
            let remaining = buf_valid - buf_pos;
            if remaining > 0 && buf_pos > 0 {
                unsafe {
                    let src = read_buf.add(buf_pos);
                    let dst = read_buf;
                    let mut i = 0;
                    while i < remaining {
                        *dst.add(i) = *src.add(i);
                        i += 1;
                    }
                }
            }
            buf_valid = remaining;
            buf_pos = 0;

            // Read more from file.
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

        // Need at least 4 bytes for a frame header.
        if buf_valid - buf_pos < 4 {
            break; // EOF
        }

        // Find MP3 sync word.
        let buf_slice = unsafe {
            core::slice::from_raw_parts(read_buf, buf_valid)
        };
        let sync_pos = match find_mp3_sync(buf_slice, buf_pos) {
            Some(pos) => pos,
            None => {
                // No sync found in remaining data -- EOF or corrupt.
                break;
            }
        };
        buf_pos = sync_pos;

        // Need enough data for at least a minimal frame.
        let avail = buf_valid - buf_pos;
        if avail < 8 {
            break; // Not enough for header + some data.
        }

        // Set up codec buffer for this frame.
        // [6] = inBuf pointer, [8] = outBuf pointer
        unsafe {
            *codec.add(6) = read_buf.add(buf_pos) as u32;
            *codec.add(8) = pcm_buf as u32;
        }

        // Decode one MP3 frame.
        let ret = unsafe { decode_fn(codec, CODEC_TYPE_MP3) };
        if ret < 0 {
            // Decode error -- skip one byte and try to resync.
            buf_pos += 1;
            continue;
        }

        // Read how many bytes the codec consumed.
        let consumed = unsafe { *codec.add(7) } as usize;
        if consumed == 0 {
            // Codec didn't consume anything -- skip ahead.
            buf_pos += 1;
            continue;
        }
        buf_pos += consumed;
        frames_decoded += 1;

        // Apply volume and output PCM to audio channel.
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

        // Output decoded PCM (blocking -- paces to real-time).
        unsafe {
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

    // Log decode stats.
    if frames_decoded > 0 {
        let mut log_buf = [0u8; 40];
        let mut p = 0;
        let s = b"[OASIS] decoded ";
        let mut i = 0;
        while i < s.len() {
            log_buf[p] = s[i];
            p += 1;
            i += 1;
        }
        p = write_u32_decimal(&mut log_buf, p, frames_decoded);
        let s2 = b" frames";
        i = 0;
        while i < s2.len() {
            log_buf[p] = s2[i];
            p += 1;
            i += 1;
        }
        crate::debug_log(&log_buf[..p]);
    }

    // Cleanup: release EDRAM and close file.
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

/// Start the background audio thread.
pub fn start_audio_thread() {
    crate::debug_log(b"[OASIS] starting audio thread...");

    // SAFETY: Creating a kernel thread for audio playback.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisAudio\0".as_ptr(),
            audio_thread_entry,
            0x1E, // slightly lower priority than ctrl thread
            0x4000, // 16KB stack
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
    write_u32_decimal(buf, pos, val as u32)
}

/// Write a u32 as decimal ASCII into a buffer.
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
