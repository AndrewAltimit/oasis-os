//! Picture-in-picture video playback from raw RGBA files.
//!
//! Plays pre-converted 160x90 raw RGBA video files at 15 fps. No hardware
//! decoder needed -- just `sceIoRead` directly into the PIP frame buffer.
//!
//! ## Preparing videos
//!
//! Use FFmpeg to convert any video to the required format:
//! ```sh
//! ffmpeg -i input.mp4 -vf scale=160:90 -pix_fmt rgba -f rawvideo -r 15 output.rgb
//! ```
//!
//! Place `.rgb` files in `ms0:/VIDEO/`. File size determines duration:
//! each frame is 57,600 bytes (160 x 90 x 4). At 15 fps, 1 minute ~ 49 MB.
//!
//! ## Architecture
//!
//! A dedicated kernel thread reads frames sequentially from the file and
//! writes them into a double-buffered PIP frame. The display hook blits
//! the front buffer to the bottom-right corner of the game's framebuffer.
//! Background MP3 audio continues uninterrupted during PIP playback.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::overlay;

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

/// Bytes per frame (160 x 90 x 4 = RGBA).
const FRAME_SIZE: usize = (PIP_W * PIP_H * 4) as usize; // 57600

/// Target frame delay in microseconds (15 fps).
const FPS_DELAY_US: u32 = 66_667;

/// Maximum number of video files in playlist.
const MAX_VIDEOS: usize = 16;

/// Maximum filename length (including path).
const MAX_FILENAME: usize = 80;

// ---------------------------------------------------------------------------
// Video state (atomics for cross-thread communication)
// ---------------------------------------------------------------------------

/// Video commands: 0=none, 1=toggle, 2=next, 3=stop.
static VIDEO_CMD: AtomicU8 = AtomicU8::new(0);

/// PIP active flag: 0=inactive, 1=active.
static PIP_ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Current frame buffer index (0 or 1) for double buffering.
static FRAME_INDEX: AtomicU8 = AtomicU8::new(0);

// ---------------------------------------------------------------------------
// Static buffers and state
// ---------------------------------------------------------------------------

/// Video file playlist (in BSS -- 1280 bytes).
static mut VIDEO_LIST: [[u8; MAX_FILENAME]; MAX_VIDEOS] = [[0u8; MAX_FILENAME]; MAX_VIDEOS];
static mut VIDEO_COUNT: usize = 0;
static mut CURRENT_VIDEO: usize = 0;

/// Video filename for OSD display.
static mut VIDEO_NAME: [u8; 48] = [0u8; 48];

/// On-demand PIP frame buffers (from user partition 2).
static mut PIP_FRAME_A: *mut u8 = core::ptr::null_mut();
static mut PIP_FRAME_B: *mut u8 = core::ptr::null_mut();

/// Memory block ID for PIP buffers.
static mut PIP_BUF_BLOCK: i32 = -1;

/// Total on-demand allocation: 2 PIP frames + alignment padding.
const PIP_ALLOC_SIZE: u32 = (FRAME_SIZE * 2 + 64) as u32;

/// File descriptor for current video file.
static mut VIDEO_FD: i32 = -1;

/// Total frames in current file.
static mut TOTAL_FRAMES: u32 = 0;

// ---------------------------------------------------------------------------
// Buffer allocation
// ---------------------------------------------------------------------------

/// Allocate PIP frame buffers from user-memory partition 2.
/// Returns true on success.
unsafe fn alloc_pip_buffers() -> bool {
    // SAFETY: Single-threaded access; PIP_BUF_BLOCK only written from this thread.
    if unsafe { PIP_BUF_BLOCK } >= 0 {
        return true; // Already allocated.
    }

    // SAFETY: sceKernelAllocPartitionMemory for user partition.
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
        log_i32(b"[VIDEO] alloc FAILED=", block_id.0);
        return false;
    }

    // SAFETY: block_id is valid, get base address and 16-byte align.
    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block_id) } as *mut u8;
    let aligned = ((base as u32 + 15) & !15) as *mut u8;

    // SAFETY: Single-threaded init, pointers within allocated block.
    unsafe {
        PIP_FRAME_A = aligned;
        PIP_FRAME_B = aligned.add(FRAME_SIZE);
        PIP_BUF_BLOCK = block_id.0;

        // Zero the frame buffers.
        let mut i = 0;
        while i < FRAME_SIZE {
            *PIP_FRAME_A.add(i) = 0;
            *PIP_FRAME_B.add(i) = 0;
            i += 1;
        }
    }

    log_i32(b"[VIDEO] PIP bufs OK, block=", block_id.0);
    true
}

// ---------------------------------------------------------------------------
// File scanning
// ---------------------------------------------------------------------------

/// Scan ms0:/VIDEO/ for .rgb files. Populates VIDEO_LIST.
unsafe fn scan_video_dir() {
    let config = crate::config::get_config();
    let dir_path = config.video_dir_str();

    crate::debug_log(b"[VIDEO] scanning for .rgb files...");

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
        // SAFETY: SceIoDirent is repr(C), zero-initialized is valid.
        let mut dirent = unsafe { core::mem::zeroed::<psp::sys::SceIoDirent>() };
        // SAFETY: sceIoDread with valid fd and properly-sized struct.
        let ret = unsafe { psp::sys::sceIoDread(dfd, &mut dirent) };
        if ret <= 0 {
            break;
        }

        let name = &dirent.d_name;

        // Find name length.
        let mut name_len = 0;
        while name_len < 255 && name[name_len] != 0 {
            name_len += 1;
        }
        if name_len < 5 {
            continue;
        }

        // Check for .rgb extension (case-insensitive).
        let ext = &name[name_len - 4..name_len];
        let is_rgb = (ext[0] == b'.')
            && (ext[1] == b'r' || ext[1] == b'R')
            && (ext[2] == b'g' || ext[2] == b'G')
            && (ext[3] == b'b' || ext[3] == b'B');

        if !is_rgb {
            continue;
        }

        // SAFETY: VIDEO_COUNT is bounded by MAX_VIDEOS.
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

            crate::debug_log(core::slice::from_raw_parts(slot_ptr, i + j));
            VIDEO_COUNT += 1;
        }
    }

    // SAFETY: Close directory.
    unsafe {
        psp::sys::sceIoDclose(psp::sys::SceUid(dfd.0));
    }

    // SAFETY: VIDEO_COUNT written only from this thread during scan.
    log_usize(b"[VIDEO] found .rgb files: ", unsafe { VIDEO_COUNT });
}

// ---------------------------------------------------------------------------
// Frame reading
// ---------------------------------------------------------------------------

/// Read one frame from the video file into the back PIP buffer.
/// Returns true if a full frame was read, false on EOF or error.
unsafe fn read_frame() -> bool {
    // SAFETY: Volatile read of VIDEO_FD which may be written from video thread.
    let fd = unsafe { core::ptr::read_volatile(&raw const VIDEO_FD) };
    if fd < 0 {
        return false;
    }

    // Write to the back buffer (opposite of what display hook reads).
    let idx = FRAME_INDEX.load(Ordering::Relaxed);
    // SAFETY: Volatile reads of PIP frame buffer pointers; they are set once
    // during alloc_pip_buffers and remain valid while PIP is active.
    let dst = if idx == 0 {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_B) }
    } else {
        unsafe { core::ptr::read_volatile(&raw const PIP_FRAME_A) }
    };
    if dst.is_null() {
        return false;
    }

    // Read one frame of raw RGBA data.
    let mut total_read: usize = 0;
    while total_read < FRAME_SIZE {
        let remaining = (FRAME_SIZE - total_read) as u32;
        // SAFETY: fd is valid, dst is within allocated PIP buffer.
        let ret = unsafe {
            psp::sys::sceIoRead(
                psp::sys::SceUid(fd),
                dst.add(total_read) as *mut _,
                remaining,
            )
        };
        if ret <= 0 {
            return false; // EOF or error.
        }
        total_read += ret as usize;
    }

    // Flip double buffer index.
    FRAME_INDEX.store(if idx == 0 { 1 } else { 0 }, Ordering::Relaxed);
    true
}

// ---------------------------------------------------------------------------
// Video thread
// ---------------------------------------------------------------------------

/// Whether the video subsystem has been initialized (scan done).
static INIT_DONE: AtomicU8 = AtomicU8::new(0);

/// Video thread entry point.
///
/// Started at boot from psp_main(). Idles until the first VIDEO_CMD
/// arrives, then scans for .rgb files and enters the playback loop.
unsafe extern "C" fn video_thread_entry(_args: usize, _argp: *mut core::ffi::c_void) -> i32 {
    crate::debug_log(b"[VIDEO] thread started, waiting for cmd...");

    // Idle loop: wait for first PIP command before doing any file I/O.
    loop {
        let cmd = VIDEO_CMD.load(Ordering::Relaxed);
        if cmd != 0 {
            break;
        }
        // SAFETY: Sleep to avoid busy-waiting.
        unsafe {
            psp::sys::sceKernelDelayThread(100_000); // 100ms
        }
    }

    // Scan for video files.
    crate::debug_log(b"[VIDEO] scanning...");
    // SAFETY: Called from video thread; scan_video_dir accesses statics only
    // from this thread context.
    unsafe {
        scan_video_dir();
    }

    INIT_DONE.store(1, Ordering::Release);

    // Main loop.
    loop {
        // Check for commands.
        let cmd = VIDEO_CMD.load(Ordering::Relaxed);
        if cmd != 0 {
            VIDEO_CMD.store(0, Ordering::Relaxed);

            match cmd {
                1 => {
                    // Toggle PIP.
                    if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
                        stop_playback();
                    } else {
                        // Rescan if no videos found yet.
                        // SAFETY: VIDEO_COUNT and scan_video_dir accessed only from video thread.
                        if unsafe { VIDEO_COUNT } == 0 {
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
                    // SAFETY: VIDEO_COUNT and CURRENT_VIDEO accessed only from video thread.
                    unsafe {
                        if VIDEO_COUNT > 0 {
                            CURRENT_VIDEO = (CURRENT_VIDEO + 1) % VIDEO_COUNT;
                        }
                    }
                    if was_active {
                        start_playback();
                    }
                },
                3 => stop_playback(),
                _ => {},
            }
        }

        // If PIP active, read and display frames.
        if PIP_ACTIVE.load(Ordering::Relaxed) != 0 {
            // SAFETY: read_frame reads from valid fd into allocated PIP buffer.
            let got_frame = unsafe { read_frame() };

            if !got_frame {
                // EOF -- loop the video by seeking back to start.
                // SAFETY: VIDEO_FD accessed only from video thread.
                let fd = unsafe { VIDEO_FD };
                if fd >= 0 {
                    // SAFETY: Valid fd, seek to beginning.
                    unsafe {
                        psp::sys::sceIoLseek(
                            psp::sys::SceUid(fd),
                            0,
                            psp::sys::IoWhence::Set,
                        );
                    }
                    // Try reading again after seek.
                    // SAFETY: read_frame reads from valid fd into allocated PIP buffer.
                    let retry = unsafe { read_frame() };
                    if !retry {
                        crate::debug_log(b"[VIDEO] read error, stopping");
                        stop_playback();
                        overlay::show_osd(b"PIP: read error");
                    }
                } else {
                    stop_playback();
                }
            }

            // Frame pacing: ~15 fps.
            // SAFETY: Sleep for frame timing.
            unsafe {
                psp::sys::sceKernelDelayThread(FPS_DELAY_US);
            }
        } else {
            // Idle -- sleep longer.
            // SAFETY: Sleep to avoid busy-waiting.
            unsafe {
                psp::sys::sceKernelDelayThread(100_000);
            }
        }
    }
}

/// Start video playback of the current video.
fn start_playback() {
    // Allocate PIP frame buffers on demand.
    // SAFETY: alloc_pip_buffers allocates from PSP user-memory partition.
    if !unsafe { alloc_pip_buffers() } {
        overlay::show_osd(b"PIP: out of memory");
        return;
    }

    // SAFETY: CURRENT_VIDEO and VIDEO_COUNT accessed only from video thread.
    let idx = unsafe { CURRENT_VIDEO };
    let count = unsafe { VIDEO_COUNT };
    if idx >= count || count == 0 {
        overlay::show_osd(b"No .rgb in ms0:/VIDEO/");
        crate::debug_log(b"[VIDEO] no files");
        return;
    }

    // SAFETY: idx is bounded by VIDEO_COUNT < MAX_VIDEOS; VIDEO_LIST is valid.
    let filepath_ptr = unsafe {
        (&raw const VIDEO_LIST)
            .cast::<[u8; MAX_FILENAME]>()
            .add(idx)
    };
    // SAFETY: filepath_ptr points to a valid entry in VIDEO_LIST.
    let filepath = unsafe { &*filepath_ptr };

    // Set display name for OSD.
    // SAFETY: set_video_name writes to VIDEO_NAME static; single-threaded access.
    unsafe {
        set_video_name(filepath);
    }

    // Open file.
    // SAFETY: sceIoOpen with valid null-terminated path.
    let fd = unsafe {
        psp::sys::sceIoOpen(filepath.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        log_i32(b"[VIDEO] open failed=", fd.0);
        overlay::show_osd(b"PIP: open failed");
        return;
    }
    // SAFETY: VIDEO_FD accessed only from video thread.
    unsafe {
        VIDEO_FD = fd.0;
    }

    // Get file size to calculate frame count.
    // SAFETY: Valid fd, seek operations.
    let file_size = unsafe {
        let end = psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End);
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set);
        end as u32
    };
    // SAFETY: TOTAL_FRAMES accessed only from video thread.
    unsafe {
        TOTAL_FRAMES = file_size / FRAME_SIZE as u32;
    }
    // SAFETY: TOTAL_FRAMES accessed only from video thread.
    log_u32(b"[VIDEO] frames=", unsafe { TOTAL_FRAMES });

    // SAFETY: TOTAL_FRAMES accessed only from video thread.
    if unsafe { TOTAL_FRAMES } == 0 {
        crate::debug_log(b"[VIDEO] file too small");
        // SAFETY: Valid fd; VIDEO_FD accessed only from video thread.
        unsafe {
            psp::sys::sceIoClose(fd);
            VIDEO_FD = -1;
        }
        overlay::show_osd(b"PIP: file too small");
        return;
    }

    PIP_ACTIVE.store(1, Ordering::Relaxed);
    crate::debug_log(b"[VIDEO] PIP ACTIVE");

    // Check for companion .mp3 file (same name, .mp3 extension).
    try_play_companion_mp3(filepath);

    // Show OSD with filename.
    let mut buf = [0u8; 48];
    let p = copy_bytes(&mut buf, 0, b"PIP: ");
    let name_len = video_name_len();
    // SAFETY: VIDEO_NAME is a valid static buffer; name_len is bounded by 48.
    let name_slice = unsafe {
        core::slice::from_raw_parts((&raw const VIDEO_NAME).cast::<u8>(), name_len)
    };
    let p = copy_bytes(&mut buf, p, name_slice);
    overlay::show_osd(&buf[..p]);
}

/// Stop video playback and clean up.
fn stop_playback() {
    if PIP_ACTIVE.load(Ordering::Relaxed) == 0 {
        return;
    }
    PIP_ACTIVE.store(0, Ordering::Relaxed);
    crate::debug_log(b"[VIDEO] stopping...");

    // Stop companion MP3 audio (resumes normal music playlist).
    crate::audio::stop_video_mp3();

    // Close file.
    // SAFETY: VIDEO_FD accessed only from video thread.
    let fd = unsafe { VIDEO_FD };
    if fd >= 0 {
        // SAFETY: Valid fd.
        unsafe {
            psp::sys::sceIoClose(psp::sys::SceUid(fd));
            VIDEO_FD = -1;
        }
    }

    // Keep PIP frame buffers allocated -- the display hook may still be
    // mid-blit after checking is_pip_active(). Buffers are reused on
    // next start_playback() (alloc_pip_buffers is idempotent).

    overlay::show_osd(b"PIP stopped");
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start the video thread. Must be called from psp_main() where kernel
/// syscalls work (NOT the display hook).
pub fn start_video_thread() {
    crate::debug_log(b"[VIDEO] creating thread...");
    // SAFETY: Creating a kernel thread for video playback.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisVideo\0".as_ptr(),
            video_thread_entry,
            0x1A,   // Priority (below audio at 24).
            0x4000, // 16KB stack.
            psp::sys::ThreadAttributes::empty(), // kernel thread
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            log_i32(b"[VIDEO] thread id=", thid.0);
        } else {
            log_i32(b"[VIDEO] thread FAILED=", thid.0);
        }
    }
}

/// Toggle PIP on/off. Safe to call from display hook (atomics only).
pub fn toggle_pip() {
    VIDEO_CMD.store(1, Ordering::Relaxed);
}

/// Advance to next video. Safe to call from display hook.
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
    // Volatile reads ensure we see the latest value from the video thread.
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

/// Try to find and play a companion .mp3 file for the given .rgb path.
/// Replaces ".rgb" with ".mp3" and checks if the file exists.
fn try_play_companion_mp3(rgb_path: &[u8]) {
    // Find path length (up to null terminator).
    let mut path_len = 0;
    while path_len < rgb_path.len() && rgb_path[path_len] != 0 {
        path_len += 1;
    }

    // Need at least ".rgb" (4 chars) at the end.
    if path_len < 5 {
        return;
    }

    // Build the .mp3 path by replacing the last 4 bytes.
    let mut mp3_path = [0u8; MAX_FILENAME];
    if path_len >= MAX_FILENAME {
        return;
    }
    let mut i = 0;
    while i < path_len - 4 {
        mp3_path[i] = rgb_path[i];
        i += 1;
    }
    mp3_path[i] = b'.';
    mp3_path[i + 1] = b'm';
    mp3_path[i + 2] = b'p';
    mp3_path[i + 3] = b'3';
    mp3_path[i + 4] = 0;

    // Check if the .mp3 file exists by trying to open it.
    // SAFETY: sceIoOpen with valid null-terminated path.
    let fd = unsafe {
        psp::sys::sceIoOpen(mp3_path.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
    };
    if fd < psp::sys::SceUid(0) {
        crate::debug_log(b"[VIDEO] no companion .mp3");
        return;
    }
    // File exists -- close it and tell audio to play it.
    // SAFETY: Valid fd.
    unsafe {
        psp::sys::sceIoClose(fd);
    }

    crate::debug_log(b"[VIDEO] companion .mp3 found");
    crate::audio::play_video_mp3(&mp3_path[..i + 5]);
}

/// Extract filename from full path and set VIDEO_NAME.
unsafe fn set_video_name(path: &[u8]) {
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
    // SAFETY: VIDEO_NAME is a static buffer, single-threaded access.
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
        // SAFETY: name_ptr points to VIDEO_NAME static; len is bounded by 48.
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
