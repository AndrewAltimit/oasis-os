//! Background MP3 playback via the PSP's hardware MP3 decoder.
//!
//! Runs in a dedicated kernel thread, streaming MP3 files from
//! `ms0:/MUSIC/` through `sceMp3*` APIs to a reserved audio channel.
//!
//! The playlist is built by scanning the music directory at startup.
//! No heap allocator is needed for the core decode loop -- static buffers
//! are used for MP3 stream data and PCM output.

use crate::overlay;

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};

/// Maximum number of tracks in the playlist.
const MAX_TRACKS: usize = 32;
/// Maximum filename length (null-terminated).
const MAX_FILENAME: usize = 64;
/// MP3 stream buffer size (fed to the hardware decoder).
const MP3_BUF_SIZE: usize = 8 * 1024;
/// PCM output buffer (stereo i16, 1152 samples * 2 channels).
const PCM_BUF_SIZE: usize = 4608;
/// Audio output volume (0-0x8000).
const DEFAULT_VOLUME: i32 = 0x6000;
/// Volume step for up/down.
const VOLUME_STEP: i32 = 0x800;
/// Audio thread stack size.
const AUDIO_STACK_SIZE: i32 = 8192;
/// Audio thread priority (lower = higher priority, 30 is moderate).
const AUDIO_PRIORITY: i32 = 30;
/// File read buffer for streaming.
const FILE_BUF_SIZE: usize = 16 * 1024;

/// Playlist: array of null-terminated filenames.
static mut PLAYLIST: [[u8; MAX_FILENAME]; MAX_TRACKS] = [[0u8; MAX_FILENAME]; MAX_TRACKS];
/// Number of tracks in the playlist.
static mut TRACK_COUNT: usize = 0;
/// Current track index.
static CURRENT_TRACK: AtomicU8 = AtomicU8::new(0);
/// Current volume.
static VOLUME: AtomicI32 = AtomicI32::new(DEFAULT_VOLUME);
/// Playback paused flag.
static PAUSED: AtomicBool = AtomicBool::new(false);
/// Audio thread running flag.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Track changed flag (signals audio thread to restart decode).
static TRACK_CHANGED: AtomicBool = AtomicBool::new(false);

/// Music directory prefix (from config).
static mut MUSIC_DIR: [u8; 64] = [0u8; 64];
static mut MUSIC_DIR_LEN: usize = 0;

/// Get the current track's display name (for the overlay).
pub fn current_track_name() -> &'static [u8] {
    // SAFETY: PLAYLIST is read-only after scan_playlist().
    unsafe {
        let idx = CURRENT_TRACK.load(Ordering::Relaxed) as usize;
        if idx < TRACK_COUNT {
            &PLAYLIST[idx]
        } else {
            b"\0"
        }
    }
}

/// Toggle play/pause.
pub fn toggle_playback() {
    let was_paused = PAUSED.load(Ordering::Relaxed);
    PAUSED.store(!was_paused, Ordering::Relaxed);
    if was_paused {
        overlay::show_osd(b"Music: Playing");
    } else {
        overlay::show_osd(b"Music: Paused");
    }
}

/// Skip to next track.
pub fn next_track() {
    // SAFETY: TRACK_COUNT is read-only after init.
    let count = unsafe { TRACK_COUNT };
    if count == 0 {
        return;
    }
    let cur = CURRENT_TRACK.load(Ordering::Relaxed);
    let next = if (cur as usize + 1) >= count {
        0
    } else {
        cur + 1
    };
    CURRENT_TRACK.store(next, Ordering::Relaxed);
    TRACK_CHANGED.store(true, Ordering::Release);
    overlay::show_osd(b"Next track");
}

/// Skip to previous track.
pub fn prev_track() {
    // SAFETY: TRACK_COUNT is read-only after init.
    let count = unsafe { TRACK_COUNT };
    if count == 0 {
        return;
    }
    let cur = CURRENT_TRACK.load(Ordering::Relaxed);
    let prev = if cur == 0 {
        (count - 1) as u8
    } else {
        cur - 1
    };
    CURRENT_TRACK.store(prev, Ordering::Relaxed);
    TRACK_CHANGED.store(true, Ordering::Release);
    overlay::show_osd(b"Prev track");
}

/// Increase volume.
pub fn volume_up() {
    let vol = VOLUME.load(Ordering::Relaxed);
    let new_vol = (vol + VOLUME_STEP).min(0x8000);
    VOLUME.store(new_vol, Ordering::Relaxed);
    overlay::show_osd(b"Volume Up");
}

/// Decrease volume.
pub fn volume_down() {
    let vol = VOLUME.load(Ordering::Relaxed);
    let new_vol = (vol - VOLUME_STEP).max(0);
    VOLUME.store(new_vol, Ordering::Relaxed);
    overlay::show_osd(b"Volume Down");
}

/// Start the background audio thread.
pub fn start_audio_thread() {
    if RUNNING.load(Ordering::Relaxed) {
        return;
    }

    // Copy music dir from config
    let cfg = crate::config::get_config();
    // SAFETY: Single-threaded init.
    unsafe {
        let len = cfg.music_dir_len.min(MUSIC_DIR.len() - 1);
        let mut i = 0;
        while i < len {
            MUSIC_DIR[i] = cfg.music_dir[i];
            i += 1;
        }
        MUSIC_DIR[len] = 0;
        MUSIC_DIR_LEN = len;
    }

    // Scan playlist
    scan_playlist();

    // SAFETY: TRACK_COUNT is set by scan_playlist.
    if unsafe { TRACK_COUNT } == 0 {
        overlay::show_osd(b"No MP3 files found");
        return;
    }

    // Create kernel thread for audio playback
    // SAFETY: Creating a kernel thread with valid parameters.
    unsafe {
        let tid = psp::sys::sceKernelCreateThread(
            b"OasisAudio\0".as_ptr(),
            audio_thread_entry,
            AUDIO_PRIORITY,
            AUDIO_STACK_SIZE,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if tid >= 0 {
            RUNNING.store(true, Ordering::Release);
            psp::sys::sceKernelStartThread(tid, 0, core::ptr::null_mut());
        }
    }
}

/// Scan the music directory for MP3 files.
fn scan_playlist() {
    // Build null-terminated path
    // SAFETY: MUSIC_DIR is valid after init.
    let dir_path = unsafe { &MUSIC_DIR[..MUSIC_DIR_LEN + 1] };

    // SAFETY: sceIoDopen with null-terminated path.
    let dfd = unsafe { psp::sys::sceIoDopen(dir_path.as_ptr()) };
    if dfd < 0 {
        return;
    }

    let mut dirent = unsafe { core::mem::zeroed::<psp::sys::SceIoDirent>() };

    // SAFETY: Iterating directory entries.
    unsafe {
        while TRACK_COUNT < MAX_TRACKS {
            let ret = psp::sys::sceIoDread(dfd, &mut dirent);
            if ret <= 0 {
                break;
            }

            // Check if it's a regular file ending in .mp3 or .MP3
            let name_ptr = dirent.d_name.as_ptr() as *const u8;
            let mut name_len = 0;
            while name_len < 256 && *name_ptr.add(name_len) != 0 {
                name_len += 1;
            }

            if name_len < 5 {
                continue;
            }

            // Check .mp3 extension (case-insensitive)
            let ext_start = name_len - 4;
            let b1 = (*name_ptr.add(ext_start)).to_ascii_lowercase();
            let b2 = (*name_ptr.add(ext_start + 1)).to_ascii_lowercase();
            let b3 = (*name_ptr.add(ext_start + 2)).to_ascii_lowercase();
            let b4 = (*name_ptr.add(ext_start + 3)).to_ascii_lowercase();

            if b1 != b'.' || b2 != b'm' || b3 != b'p' || b4 != b'3' {
                continue;
            }

            // Store filename (just the name, not full path)
            let store_len = name_len.min(MAX_FILENAME - 1);
            let mut i = 0;
            while i < store_len {
                PLAYLIST[TRACK_COUNT][i] = *name_ptr.add(i);
                i += 1;
            }
            PLAYLIST[TRACK_COUNT][store_len] = 0;
            TRACK_COUNT += 1;
        }

        psp::sys::sceIoDclose(dfd);
    }
}

/// Build a full path for a track: music_dir + filename.
///
/// Returns the length of the path (excluding null terminator).
fn build_track_path(buf: &mut [u8; 128], track_idx: usize) -> usize {
    // SAFETY: MUSIC_DIR and PLAYLIST are valid after init.
    unsafe {
        let mut pos = 0;

        // Copy music dir
        let mut i = 0;
        while i < MUSIC_DIR_LEN && pos < 127 {
            buf[pos] = MUSIC_DIR[i];
            pos += 1;
            i += 1;
        }

        // Ensure trailing slash
        if pos > 0 && buf[pos - 1] != b'/' {
            buf[pos] = b'/';
            pos += 1;
        }

        // Copy filename
        i = 0;
        while i < MAX_FILENAME && PLAYLIST[track_idx][i] != 0 && pos < 127 {
            buf[pos] = PLAYLIST[track_idx][i];
            pos += 1;
            i += 1;
        }

        buf[pos] = 0;
        pos
    }
}

/// Audio thread entry point.
///
/// Loops: open MP3 file -> init decoder -> decode+output loop -> next track.
///
/// # Safety
/// Called as a PSP kernel thread entry point.
unsafe extern "C" fn audio_thread_entry(_args: usize, _argp: *mut c_void) -> i32 {
    // Static buffers for MP3 decode (no heap)
    static mut MP3_BUF: [u8; MP3_BUF_SIZE] = [0u8; MP3_BUF_SIZE];
    static mut PCM_BUF: [i16; PCM_BUF_SIZE] = [0i16; PCM_BUF_SIZE];
    static mut FILE_BUF: [u8; FILE_BUF_SIZE] = [0u8; FILE_BUF_SIZE];

    // Reserve an audio channel
    let channel = unsafe {
        psp::sys::sceAudioChReserve(
            psp::sys::AUDIO_NEXT_CHANNEL,
            psp::sys::audio_sample_align(1152),
            psp::sys::AudioFormat::Stereo,
        )
    };
    if channel < 0 {
        overlay::show_osd(b"Audio: no channel");
        RUNNING.store(false, Ordering::Release);
        return -1;
    }

    while RUNNING.load(Ordering::Relaxed) {
        let track_idx = CURRENT_TRACK.load(Ordering::Relaxed) as usize;
        // SAFETY: TRACK_COUNT is read-only after init.
        let track_count = unsafe { TRACK_COUNT };
        if track_idx >= track_count {
            // SAFETY: sleep to avoid busy loop.
            unsafe {
                psp::sys::sceKernelDelayThread(100_000);
            }
            continue;
        }

        // Build full path
        let mut path_buf = [0u8; 128];
        build_track_path(&mut path_buf, track_idx);

        // Open MP3 file
        // SAFETY: path_buf is null-terminated.
        let fd = unsafe {
            psp::sys::sceIoOpen(
                path_buf.as_ptr(),
                psp::sys::IoOpenFlags::RD_ONLY,
                0,
            )
        };
        if fd < 0 {
            // Skip to next track on error
            advance_track();
            continue;
        }

        // Get file size
        // SAFETY: fd is valid.
        let file_size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) } as u32;
        unsafe {
            psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set);
        }

        // Init MP3 resource
        // SAFETY: sceMp3 syscalls.
        let mp3_ret = unsafe { psp::sys::sceMp3InitResource() };
        if mp3_ret < 0 {
            unsafe {
                psp::sys::sceIoClose(fd);
            }
            overlay::show_osd(b"MP3 init failed");
            advance_track();
            continue;
        }

        // Reserve MP3 handle with static buffers
        // SAFETY: Static buffers are valid, single audio thread.
        let mut init_arg = psp::sys::SceMp3InitArg {
            mp3_stream_start: 0,
            unk1: 0,
            mp3_stream_end: file_size,
            unk2: 0,
            mp3_buf: unsafe { MP3_BUF.as_mut_ptr() as *mut c_void },
            mp3_buf_size: MP3_BUF_SIZE as i32,
            pcm_buf: unsafe { PCM_BUF.as_mut_ptr() as *mut c_void },
            pcm_buf_size: (PCM_BUF_SIZE * 2) as i32,
        };

        let handle_id = unsafe { psp::sys::sceMp3ReserveMp3Handle(&mut init_arg) };
        if handle_id < 0 {
            unsafe {
                psp::sys::sceMp3TermResource();
                psp::sys::sceIoClose(fd);
            }
            advance_track();
            continue;
        }
        let handle = psp::sys::Mp3Handle(handle_id);

        // Feed initial data from file
        // SAFETY: Static buffers, fd is valid.
        let feed_ok = unsafe { feed_mp3_from_file(handle, fd, &mut FILE_BUF) };
        if !feed_ok {
            unsafe {
                psp::sys::sceMp3ReleaseMp3Handle(handle);
                psp::sys::sceMp3TermResource();
                psp::sys::sceIoClose(fd);
            }
            advance_track();
            continue;
        }

        // Initialize decoder
        let init_ret = unsafe { psp::sys::sceMp3Init(handle) };
        if init_ret < 0 {
            unsafe {
                psp::sys::sceMp3ReleaseMp3Handle(handle);
                psp::sys::sceMp3TermResource();
                psp::sys::sceIoClose(fd);
            }
            advance_track();
            continue;
        }

        TRACK_CHANGED.store(false, Ordering::Release);

        // Decode + output loop
        loop {
            if !RUNNING.load(Ordering::Relaxed) || TRACK_CHANGED.load(Ordering::Relaxed) {
                break;
            }

            // Handle pause
            if PAUSED.load(Ordering::Relaxed) {
                // SAFETY: Sleep while paused.
                unsafe {
                    psp::sys::sceKernelDelayThread(50_000);
                }
                continue;
            }

            // Feed more data if needed
            // SAFETY: handle and fd are valid.
            unsafe {
                if psp::sys::sceMp3CheckStreamDataNeeded(handle) > 0 {
                    if !feed_mp3_from_file(handle, fd, &mut FILE_BUF) {
                        break; // EOF or error
                    }
                }
            }

            // Decode a frame
            let mut out_ptr: *mut i16 = core::ptr::null_mut();
            let decoded = unsafe { psp::sys::sceMp3Decode(handle, &mut out_ptr) };
            if decoded <= 0 || out_ptr.is_null() {
                break; // End of track
            }

            // Output decoded PCM to audio channel
            let vol = VOLUME.load(Ordering::Relaxed);
            // SAFETY: out_ptr is valid PCM data from the decoder.
            unsafe {
                psp::sys::sceAudioOutputBlocking(channel, vol, out_ptr as *mut c_void);
            }
        }

        // Cleanup
        unsafe {
            psp::sys::sceMp3ReleaseMp3Handle(handle);
            psp::sys::sceMp3TermResource();
            psp::sys::sceIoClose(fd);
        }

        // If track wasn't manually changed, advance to next
        if !TRACK_CHANGED.load(Ordering::Relaxed) {
            advance_track();
        }
    }

    // Release audio channel
    unsafe {
        psp::sys::sceAudioChRelease(channel);
    }
    RUNNING.store(false, Ordering::Release);
    0
}

/// Feed MP3 data from a file to the decoder's stream buffer.
///
/// # Safety
/// `handle` must be a valid MP3 handle, `fd` a valid file descriptor,
/// `file_buf` must be a valid mutable buffer.
unsafe fn feed_mp3_from_file(
    handle: psp::sys::Mp3Handle,
    fd: psp::sys::SceUid,
    file_buf: &mut [u8; FILE_BUF_SIZE],
) -> bool {
    let mut dst_ptr: *mut u8 = core::ptr::null_mut();
    let mut to_write: i32 = 0;
    let mut src_pos: i32 = 0;

    // SAFETY: sceMp3 syscalls with valid handle.
    let ret = unsafe {
        psp::sys::sceMp3GetInfoToAddStreamData(handle, &mut dst_ptr, &mut to_write, &mut src_pos)
    };
    if ret < 0 || to_write <= 0 || dst_ptr.is_null() {
        return false;
    }

    // Seek to the position the decoder wants
    // SAFETY: fd is valid.
    unsafe {
        psp::sys::sceIoLseek(fd, src_pos as i64, psp::sys::IoWhence::Set);
    }

    // Read from file in chunks
    let mut total_read = 0i32;
    while total_read < to_write {
        let chunk = ((to_write - total_read) as usize).min(FILE_BUF_SIZE);
        // SAFETY: file_buf is valid, fd is valid.
        let bytes_read = unsafe {
            psp::sys::sceIoRead(fd, file_buf.as_mut_ptr() as *mut _, chunk as u32)
        };
        if bytes_read <= 0 {
            break;
        }

        // Copy to decoder buffer
        // SAFETY: dst_ptr is valid memory from sceMp3GetInfoToAddStreamData.
        let mut i = 0;
        while i < bytes_read as usize {
            unsafe {
                *dst_ptr.add(total_read as usize + i) = file_buf[i];
            }
            i += 1;
        }
        total_read += bytes_read;
    }

    if total_read <= 0 {
        // SAFETY: Notify decoder with 0 bytes (EOF).
        unsafe {
            psp::sys::sceMp3NotifyAddStreamData(handle, 0);
        }
        return false;
    }

    // SAFETY: Notify decoder of added data.
    let ret = unsafe { psp::sys::sceMp3NotifyAddStreamData(handle, total_read) };
    ret >= 0
}

/// Advance to the next track (wrap around).
fn advance_track() {
    // SAFETY: TRACK_COUNT is read-only.
    let count = unsafe { TRACK_COUNT };
    if count == 0 {
        return;
    }
    let cur = CURRENT_TRACK.load(Ordering::Relaxed);
    let next = if (cur as usize + 1) >= count {
        0
    } else {
        cur + 1
    };
    CURRENT_TRACK.store(next, Ordering::Relaxed);
}
