//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once -- PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video H.264 frames are decoded via `sceMpeg` (Media Engine) on real PSP
//! hardware via the Media Engine coprocessor.
//!
//! Uses `psp::mpeg::AvcDecoder` (NAL direct feeding via `sceMpegGetAvcNalAu`)
//! with `mpeg_vsh370.prx` providing the sceMpeg implementation. The sceMpeg
//! API is used instead of the lower-level sceVideocodec because sceVideocodec
//! weak imports fail to resolve on many CFW configurations (error 0x806201fe).
//!
//! Layout: this module owns the shared state (queues, flags, stats) and the
//! thread loop; [`stream`] runs streaming playback, [`decoder`] wraps the
//! ME decoder, [`sps`] parses H.264 SPS headers, [`file`] plays local MP4
//! files and [`modules`] loads the firmware modules sceMpeg needs.

mod decoder;
mod file;
mod modules;
mod sps;
mod stream;

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use psp::sync::SpscQueue;
use psp::sys::SceUid;
use psp::thread::ThreadBuilder;

use crate::threading::{AudioCmd, send_audio_cmd};

/// Whether verbose video logging is enabled. Disabled during active
/// decode to avoid ~5-20ms Memory Stick I/O stalls per log write.
static VLOG_ENABLED: AtomicBool = AtomicBool::new(true);

/// Audio-only mode: skip video decode entirely. Default false — the NAL
/// decoder will attempt to decode frames before safely falling back
/// to audio-only mode (avoiding the ME deadlock on >480p).
/// Set to true via TCP "audio-only on" to skip video entirely.
static AUDIO_ONLY: AtomicBool = AtomicBool::new(false);

/// Set to true after the decoder is leaked (ME deadlock recovery).
/// Once leaked, sceMpegCreate will fail on subsequent tunes because
/// the DDR workspace is still allocated. Skip video init entirely.
static ME_LEAKED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Video statistics (readable from cmd_server via public accessors)
// ---------------------------------------------------------------------------

/// Video decode state for diagnostics.
/// 0=Idle, 1=WaitingKeyframe, 2=Decoding, 3=AudioOnly, 4=MeLeaked
static VIDEO_STATE: AtomicU32 = AtomicU32::new(0);
static VIDEO_DECODE_COUNT: AtomicU32 = AtomicU32::new(0);
static VIDEO_ERROR_COUNT: AtomicU32 = AtomicU32::new(0);
static VIDEO_NOPIC_COUNT: AtomicU32 = AtomicU32::new(0);
static VIDEO_WIDTH: AtomicU32 = AtomicU32::new(0);
static VIDEO_HEIGHT: AtomicU32 = AtomicU32::new(0);
static VIDEO_FRAMES_PROCESSED: AtomicU32 = AtomicU32::new(0);
/// Frames successfully pushed to VIDEO_FRAME_QUEUE.
static VIDEO_FRAMES_PUSHED: AtomicU32 = AtomicU32::new(0);
/// Frames dropped because VIDEO_FRAME_QUEUE was full.
static VIDEO_FRAMES_DROPPED: AtomicU32 = AtomicU32::new(0);
/// Frames polled by the main thread (successful pops).
static VIDEO_FRAMES_POLLED: AtomicU32 = AtomicU32::new(0);

/// Video state constants for `VIDEO_STATE`.
pub const VSTATE_IDLE: u32 = 0;
pub const VSTATE_WAITING_KEYFRAME: u32 = 1;
pub const VSTATE_DECODING: u32 = 2;
pub const VSTATE_AUDIO_ONLY: u32 = 3;
pub const VSTATE_ME_LEAKED: u32 = 4;

/// Snapshot of video decode statistics for diagnostics.
pub struct VideoStats {
    pub state: u32,
    pub width: u32,
    pub height: u32,
    pub decoded: u32,
    pub errors: u32,
    pub no_pic: u32,
    pub processed: u32,
    pub pushed: u32,
    pub dropped: u32,
    pub polled: u32,
    pub poll_attempts: u32,
    pub upload_us: u32,
    pub upload_count: u32,
    pub audio_only: bool,
    pub me_leaked: bool,
    pub decode_step: u32,
}

/// Read a consistent snapshot of video decode statistics.
pub fn video_stats() -> VideoStats {
    VideoStats {
        state: VIDEO_STATE.load(Ordering::Relaxed),
        width: VIDEO_WIDTH.load(Ordering::Relaxed),
        height: VIDEO_HEIGHT.load(Ordering::Relaxed),
        decoded: VIDEO_DECODE_COUNT.load(Ordering::Relaxed),
        errors: VIDEO_ERROR_COUNT.load(Ordering::Relaxed),
        no_pic: VIDEO_NOPIC_COUNT.load(Ordering::Relaxed),
        processed: VIDEO_FRAMES_PROCESSED.load(Ordering::Relaxed),
        pushed: VIDEO_FRAMES_PUSHED.load(Ordering::Relaxed),
        dropped: VIDEO_FRAMES_DROPPED.load(Ordering::Relaxed),
        polled: VIDEO_FRAMES_POLLED.load(Ordering::Relaxed),
        poll_attempts: VIDEO_POLL_ATTEMPTS.load(Ordering::Relaxed),
        upload_us: VIDEO_UPLOAD_US.load(Ordering::Relaxed),
        upload_count: VIDEO_UPLOAD_COUNT.load(Ordering::Relaxed),
        audio_only: AUDIO_ONLY.load(Ordering::Relaxed),
        me_leaked: ME_LEAKED.load(Ordering::Relaxed),
        decode_step: psp::mpeg::DECODE_STEP.load(Ordering::Relaxed),
    }
}

fn reset_stats() {
    VIDEO_DECODE_COUNT.store(0, Ordering::Relaxed);
    VIDEO_ERROR_COUNT.store(0, Ordering::Relaxed);
    VIDEO_NOPIC_COUNT.store(0, Ordering::Relaxed);
    VIDEO_FRAMES_PROCESSED.store(0, Ordering::Relaxed);
    VIDEO_FRAMES_PUSHED.store(0, Ordering::Relaxed);
    VIDEO_FRAMES_DROPPED.store(0, Ordering::Relaxed);
    VIDEO_FRAMES_POLLED.store(0, Ordering::Relaxed);
    VIDEO_POLL_ATTEMPTS.store(0, Ordering::Relaxed);
    VIDEO_UPLOAD_US.store(0, Ordering::Relaxed);
    VIDEO_UPLOAD_COUNT.store(0, Ordering::Relaxed);
    VIDEO_WIDTH.store(0, Ordering::Relaxed);
    VIDEO_HEIGHT.store(0, Ordering::Relaxed);
}

/// Set audio-only mode. When true, video frames are discarded and only
/// audio plays. When false, video decode is attempted (may crash/hang).
pub fn set_audio_only(enabled: bool) {
    AUDIO_ONLY.store(enabled, Ordering::Relaxed);
}

/// Check if audio-only mode is active.
pub fn is_audio_only() -> bool {
    AUDIO_ONLY.load(Ordering::Relaxed)
}

/// File-based debug logging (works from video thread, unlike psp::dprintln).
/// Suppressed when `VLOG_ENABLED` is false (during active decode).
fn vlog(msg: &str) {
    if !VLOG_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    vlog_force(msg);
}

/// Unconditional log — always writes regardless of VLOG_ENABLED.
/// Use sparingly during active decode (each write costs ~5-20ms).
pub fn vlog_force(msg: &str) {
    // SAFETY: sceIo calls with valid path and buffer pointers.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Commands for the video decode thread.
pub enum VideoCmd {
    /// Play a downloaded MP4 file (audio only; see [`file::play_mp4`]).
    /// Streaming playback is started with [`request_stream_start`] instead.
    Play { path: String, seek_secs: u64 },
    /// Stop current playback.
    Stop,
    /// Shut down the thread.
    Shutdown,
}

/// A pre-demuxed H.264 access unit pushed by the I/O thread for decode.
pub struct StreamFrame {
    /// Raw AVCC-format data (NAL length prefix + NAL units). The decoder
    /// feeds this directly to the ME via `psp::mpeg::AvcNal`.
    pub data: Vec<u8>,
    /// AVCC NAL length prefix size (typically 4).
    pub nal_prefix_size: u8,
    /// SPS from MP4 avcC atom (raw NAL, no start codes).
    /// Only set on keyframes to avoid per-frame cloning.
    pub avcc_sps: Option<Vec<u8>>,
    /// PPS from MP4 avcC atom (raw NAL, no start codes).
    /// Only set on keyframes to avoid per-frame cloning.
    pub avcc_pps: Option<Vec<u8>>,
    pub timestamp_secs: f64,
    pub is_keyframe: bool,
}

/// A decoded video frame ready for texture upload.
///
/// CSC writes directly into the GU video texture buffer (see
/// `VIDEO_TEX_PTR`); this struct just carries dimensions so the main
/// thread can set the correct UV mapping.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    /// CSC output stride in pixels (512 for ≤480p). Matches texture buf_w.
    pub stride: u32,
}

/// Pointer to the GU video texture's pixel buffer. Set by `PspBackend::
/// alloc_video_texture` and read by `NalDecoder::decode` so CSC writes
/// directly into the GU-bound buffer (zero-copy: no separate frame
/// buffer, no per-frame memcpy). The buffer is allocated by the main
/// thread before the channel tune begins (when ~7.5 MB of partition
/// memory is still free) and reused for every subsequent stream — see
/// `decoder::PERSISTENT_DECODER` for the parallel reasoning on the
/// sceMpeg side.
///
/// Invariant: the stored pointer outlives the program (allocated through
/// the texture system, freed only at app exit). Written only on the main
/// thread before the video thread starts decoding.
pub static VIDEO_TEX_PTR: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
/// Size in bytes of the buffer pointed to by `VIDEO_TEX_PTR`.
pub static VIDEO_TEX_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Publish the texture buffer pointer + size for the video thread to
/// use as CSC output target. Called by `PspBackend::alloc_video_texture`.
pub fn set_video_texture_buffer(ptr: *mut u8, size: usize) {
    VIDEO_TEX_PTR.store(ptr, Ordering::Release);
    VIDEO_TEX_SIZE.store(size, Ordering::Release);
}

/// Borrow the GU video texture buffer as a mutable slice. Returns
/// `None` if no texture has been allocated yet.
///
/// # Safety
///
/// The caller must guarantee single-writer access. The video thread is
/// the sole writer during `NalDecoder::decode`; the main thread does not
/// touch the buffer except via the GU (which reads via DMA).
unsafe fn video_texture_slice() -> Option<&'static mut [u8]> {
    let ptr = VIDEO_TEX_PTR.load(Ordering::Acquire);
    let size = VIDEO_TEX_SIZE.load(Ordering::Acquire);
    if ptr.is_null() || size == 0 {
        None
    } else {
        // SAFETY: ptr/size point at a single texture buffer owned by
        // the texture system for the lifetime of the program.
        Some(unsafe { core::slice::from_raw_parts_mut(ptr, size) })
    }
}

// ---------------------------------------------------------------------------
// Static queues and state
// ---------------------------------------------------------------------------

/// Commands: main thread -> video thread.
static VIDEO_CMD_QUEUE: SpscQueue<VideoCmd, 4> = SpscQueue::new();
/// Decoded frames: video thread -> main thread (double-buffered).
static VIDEO_FRAME_QUEUE: SpscQueue<DecodedFrame, 2> = SpscQueue::new();
/// Pre-demuxed H.264 frames: I/O thread -> video thread (streaming mode).
/// 8 slots provide enough buffering to absorb I/O jitter while keeping
/// memory usage bounded (~8 × avg H.264 AU ≈ 200KB for 480p content).
static VIDEO_STREAM_QUEUE: SpscQueue<StreamFrame, 8> = SpscQueue::new();
/// Whether video is currently playing.
static VIDEO_PLAYING: AtomicBool = AtomicBool::new(false);
/// Set by I/O thread to signal video thread to enter streaming mode.
/// The video thread clears it once it starts `play_stream()`.
static STREAM_REQUESTED: AtomicBool = AtomicBool::new(false);
/// Guards against starting a new stream before the previous decoder
/// has fully cleaned up (sceMpegDelete + sceMpegFinish). Set to false
/// before decoder drop, true after drop + ME cooldown delay.
static DECODER_READY: AtomicBool = AtomicBool::new(true);

/// The sceMpeg internal semaphore ID (at mpeg_data+0x66c), published by
/// `NalDecoder::try_init` for the main-thread watchdog.
static MPEG_INTERNAL_SEMA: AtomicI32 = AtomicI32::new(-1);

/// Signal the sceMpeg internal semaphore to unblock a stuck AvcDecode.
/// Called from the main thread watchdog when DECODE_STEP == 2.
pub fn unblock_stuck_decode() {
    let id = MPEG_INTERNAL_SEMA.load(Ordering::Acquire);
    if id > 0 {
        vlog_force(&format!("[VIDEO] unblocking stuck decode, sema={id:#x}"));
        // SAFETY: id is a valid semaphore read from the mpeg instance.
        unsafe {
            psp::sys::sceKernelSignalSema(SceUid(id), 1);
        }
    }
}

/// Kernel semaphore signalled by I/O thread when a stream frame is pushed.
/// The video thread waits on this instead of polling with sleep.
static STREAM_FRAME_SEMA: AtomicI32 = AtomicI32::new(-1);

/// Sleep the calling thread for `us` microseconds.
fn delay_us(us: u32) {
    // SAFETY: plain kernel sleep; no memory is touched.
    unsafe { psp::sys::sceKernelDelayThread(us) };
}

/// Microseconds since boot.
fn now_us() -> u64 {
    // SAFETY: argument-less clock read.
    unsafe { psp::sys::sceKernelGetSystemTimeWide() as u64 }
}

/// Create the stream frame semaphore (called once at video thread start).
fn create_stream_sema() {
    // SAFETY: sceKernelCreateSema with valid name and parameters.
    let id = unsafe {
        psp::sys::sceKernelCreateSema(
            b"oasis_vframe\0".as_ptr(),
            0, // attr: FIFO
            0, // initial value
            8, // max value (matches VIDEO_STREAM_QUEUE capacity)
            core::ptr::null_mut(),
        )
    };
    if id >= SceUid(0) {
        STREAM_FRAME_SEMA.store(id.0, Ordering::Release);
    }
}

/// Signal the stream frame semaphore (called after pushing a frame).
fn signal_stream_frame() {
    let id = STREAM_FRAME_SEMA.load(Ordering::Acquire);
    if id >= 0 {
        // SAFETY: id is a valid semaphore created by create_stream_sema.
        unsafe {
            psp::sys::sceKernelSignalSema(SceUid(id), 1);
        }
    }
}

/// Wait for a stream frame to be available (with timeout).
/// Returns true if signalled, false on timeout.
fn wait_stream_frame(timeout_us: u32) -> bool {
    let id = STREAM_FRAME_SEMA.load(Ordering::Acquire);
    if id < 0 {
        // Semaphore not created; fall back to sleep.
        delay_us(timeout_us);
        return false;
    }
    let mut timeout = timeout_us;
    // SAFETY: id is a valid semaphore, timeout pointer is valid.
    let ret = unsafe { psp::sys::sceKernelWaitSema(SceUid(id), 1, &mut timeout) };
    ret >= 0
}

/// Empty the decoded-frame queue (and, with `streams`, the input queue).
fn drain_queues(streams: bool) {
    if streams {
        while VIDEO_STREAM_QUEUE.pop().is_some() {}
    }
    while VIDEO_FRAME_QUEUE.pop().is_some() {}
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send a command to the video decode thread.
pub fn send_video_cmd(cmd: VideoCmd) {
    let _ = VIDEO_CMD_QUEUE.push(cmd);
}

/// Number of times poll_video_frame was called (regardless of result).
static VIDEO_POLL_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
/// Cumulative microseconds spent in frame_pixels + update_video_texture.
static VIDEO_UPLOAD_US: AtomicU32 = AtomicU32::new(0);
/// Number of texture uploads performed.
static VIDEO_UPLOAD_COUNT: AtomicU32 = AtomicU32::new(0);

/// Record texture upload timing from main thread.
pub fn record_upload_time(us: u32) {
    VIDEO_UPLOAD_US.fetch_add(us, Ordering::Relaxed);
    VIDEO_UPLOAD_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Poll for the next decoded video frame (non-blocking).
///
/// Returns the frame metadata; the pixels are already in the GU video
/// texture (see [`frame_pixels_raw`]).
pub fn poll_video_frame() -> Option<DecodedFrame> {
    VIDEO_POLL_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let frame = VIDEO_FRAME_QUEUE.pop();
    if frame.is_some() {
        VIDEO_FRAMES_POLLED.fetch_add(1, Ordering::Relaxed);
    }
    frame
}

/// Return the decoded frame's pixels (CSC stride, 512 px, matching the
/// texture's buf_w).
///
/// Zero-copy: returns a borrow of the GU video texture buffer that the
/// video thread wrote into via `decode_csc_direct`. The main thread
/// hands this back to `update_video_texture`, which is now a no-op
/// (the bytes are already in the texture buffer).
pub fn frame_pixels_raw(frame: &DecodedFrame) -> &[u8] {
    let size = (frame.stride * frame.height * 4) as usize;
    let ptr = VIDEO_TEX_PTR.load(Ordering::Acquire);
    let buf_size = VIDEO_TEX_SIZE.load(Ordering::Acquire);
    if ptr.is_null() || buf_size == 0 {
        return &[];
    }
    let take = size.min(buf_size);
    // SAFETY: VIDEO_TEX_PTR is a stable pointer set once when the
    // texture is allocated and never reassigned. Single-threaded read
    // from the main thread; the video thread is the only writer and
    // we observe its writes via the atomic Acquire load above.
    unsafe { core::slice::from_raw_parts(ptr, take) }
}

/// Check if video is currently playing.
pub fn is_video_playing() -> bool {
    VIDEO_PLAYING.load(Ordering::Relaxed)
}

/// Set the playing flag from outside the video thread.
pub fn set_video_playing(val: bool) {
    VIDEO_PLAYING.store(val, Ordering::Release);
}

/// Request the video thread to enter streaming mode.
/// Called by the I/O thread (avoids SPSC queue two-producer issue).
pub fn request_stream_start() {
    STREAM_REQUESTED.store(true, Ordering::Release);
}

/// Push a pre-demuxed H.264 frame for streaming decode.
/// Returns `Ok(())` on success, or `Err(frame)` if the queue was full
/// (caller should retry after a short sleep).
pub fn try_push_stream_frame(frame: StreamFrame) -> Result<(), StreamFrame> {
    match VIDEO_STREAM_QUEUE.push(frame) {
        Ok(()) => {
            signal_stream_frame();
            Ok(())
        },
        Err(f) => Err(f),
    }
}

/// Pre-initialize the MPEG subsystem before any audio modules load.
///
/// Must be called from the main thread before spawning the audio thread.
pub fn preinit_mpeg() {
    crate::audio::load_av_modules_once_pub();
    modules::load_avmp3_module();
    vlog("[VIDEO] preinit done");
}

/// Spawn the video decode thread (priority 24, between audio=16 and I/O=32).
pub fn spawn_video_thread() {
    if let Ok(handle) = ThreadBuilder::new(b"oasis_video\0")
        .priority(24)
        .spawn(move || {
            video_thread_fn();
            0
        })
    {
        core::mem::forget(handle);
    }
}

// ---------------------------------------------------------------------------
// Thread function
// ---------------------------------------------------------------------------

fn video_thread_fn() {
    create_stream_sema();

    loop {
        // Check for streaming mode request (set by I/O thread via atomic).
        if STREAM_REQUESTED.swap(false, Ordering::Acquire) {
            VIDEO_PLAYING.store(true, Ordering::Relaxed);
            if stream::play_stream() {
                break;
            }
        }

        match VIDEO_CMD_QUEUE.pop() {
            Some(VideoCmd::Play { path, seek_secs }) => {
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                if file::play_mp4(&path, seek_secs) {
                    break;
                }
            },
            Some(VideoCmd::Stop) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                drain_queues(true);
                send_audio_cmd(AudioCmd::VideoAudioStop);
            },
            Some(VideoCmd::Shutdown) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                drain_queues(false);
                break;
            },
            None => delay_us(10_000),
        }
    }
}
