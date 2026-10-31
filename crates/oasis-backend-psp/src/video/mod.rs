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
/// [`VIDEO_TEX_PTR`]); this struct just carries dimensions so the main
/// thread can set the correct UV mapping.
pub struct DecodedFrame {
    /// Picture width in pixels (from the stream's SPS).
    pub width: u32,
    /// Picture height in pixels (from the stream's SPS).
    pub height: u32,
    /// CSC output stride in pixels (512 for ≤480p). Matches texture buf_w.
    pub stride: u32,
}

// ---------------------------------------------------------------------------
// Shared CSC target: the GU video texture buffer
// ---------------------------------------------------------------------------
//
// The main thread owns the buffer (allocates, frees, binds it to the GE);
// the video thread writes each decoded frame into it with
// `sceMpegBaseCscAvc` (zero-copy: no separate frame buffer fits next to the
// sceMpeg workspace). The two meet through three atomics:
//
// * `VIDEO_TEX_PTR` / `VIDEO_TEX_SIZE` — the published buffer. Only the
//   main thread writes them. `SIZE` changes only while `PTR` is null.
// * `VIDEO_TEX_IN_USE` — a hazard pointer: the buffer the video thread is
//   writing right now (null when idle). The video thread sets it, then
//   re-checks that the same buffer is still published before touching it
//   (`acquire_video_texture`); the main thread unpublishes first and only
//   frees or reuses a buffer once `IN_USE` no longer names it
//   (`retire_video_texture_buffer`).
//
// All accesses are `SeqCst`, which makes the store-then-load pair on each
// side a Dekker handshake: either the video thread's re-check sees the
// unpublish (and skips the frame), or the main thread's `IN_USE` load sees
// the hazard (and waits / defers the free). Never both miss.

/// Row stride of the video texture in pixels. CSC output for any ≤480-wide
/// picture is 512 px wide, and 512 is a power of two as the GE requires for
/// the texture buffer width.
pub const VIDEO_TEX_STRIDE: u32 = 512;

/// Rows pre-allocated at tune time when the stream is no taller: PSP-native
/// 480×272 (17 macroblock rows) fits without a reallocation.
pub const VIDEO_TEX_DEFAULT_ROWS: u32 = 272;

/// Rows tried when the [`VIDEO_TEX_DEFAULT_ROWS`] allocation fails: the
/// 512×256 buffer earlier builds always used (32 KB smaller), so a tight
/// heap never loses video for streams up to 256 rows.
pub const VIDEO_TEX_FALLBACK_ROWS: u32 = 256;

/// Rows of CSC output for a picture `height` px tall. The ME converts whole
/// macroblocks, so it writes `ceil16(height)` rows.
pub const fn video_tex_rows(height: u32) -> u32 {
    height.div_ceil(16) * 16
}

/// Bytes of CSC output for a picture `height` px tall at the fixed stride.
pub const fn video_tex_bytes(height: u32) -> usize {
    (VIDEO_TEX_STRIDE * video_tex_rows(height) * 4) as usize
}

/// How long [`retire_video_texture_buffer`] waits for the video thread to
/// finish an in-flight CSC write before deferring the free. One decode +
/// CSC takes ~50 ms on the ME.
const RETIRE_WAIT_MS: u32 = 150;

/// Pointer to the published GU video texture buffer (null when none).
/// Written only on the main thread by [`publish_video_texture_buffer`] /
/// [`unpublish_video_texture_buffer`]; the video thread reads it once per
/// frame through `acquire_video_texture`.
pub static VIDEO_TEX_PTR: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
/// Capacity in bytes of the buffer at `VIDEO_TEX_PTR`.
pub static VIDEO_TEX_SIZE: AtomicUsize = AtomicUsize::new(0);
/// Hazard pointer: the buffer the video thread is writing (null if idle).
static VIDEO_TEX_IN_USE: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

/// Publish `ptr` (`size` bytes, 16-byte aligned) as the CSC output target.
/// Main thread only. A previously published buffer is unpublished first;
/// retiring it is the caller's job.
pub fn publish_video_texture_buffer(ptr: *mut u8, size: usize) {
    VIDEO_TEX_PTR.store(core::ptr::null_mut(), Ordering::SeqCst);
    VIDEO_TEX_SIZE.store(size, Ordering::SeqCst);
    VIDEO_TEX_PTR.store(ptr, Ordering::SeqCst);
}

/// Stop offering the current buffer to the video thread. Main thread only.
/// A CSC write already in flight may still land; see
/// [`retire_video_texture_buffer`].
pub fn unpublish_video_texture_buffer() {
    VIDEO_TEX_PTR.store(core::ptr::null_mut(), Ordering::SeqCst);
}

/// True while the video thread may be writing into `ptr`.
pub fn video_texture_buffer_in_use(ptr: *const u8) -> bool {
    core::ptr::eq(VIDEO_TEX_IN_USE.load(Ordering::SeqCst), ptr)
}

/// Unpublish `ptr` (if it is the published CSC target) and wait — bounded,
/// yielding the CPU — until the video thread no longer writes into it.
/// Main thread only.
///
/// Returns `true` when `ptr` may be freed or reused. `false` means the video
/// thread is still inside a decode that holds it (e.g. a hung
/// `sceMpegAvcDecode` the watchdog has not broken yet): the caller must not
/// free the buffer now, only later once [`video_texture_buffer_in_use`]
/// reports `false`.
pub fn retire_video_texture_buffer(ptr: *const u8) -> bool {
    if core::ptr::eq(VIDEO_TEX_PTR.load(Ordering::SeqCst), ptr) {
        unpublish_video_texture_buffer();
    }
    for _ in 0..RETIRE_WAIT_MS {
        if !video_texture_buffer_in_use(ptr) {
            return true;
        }
        delay_us(1_000);
    }
    !video_texture_buffer_in_use(ptr)
}

/// Video-thread access to the published texture buffer for one CSC write.
/// Clears the hazard pointer on drop.
pub(crate) struct VideoTexGuard {
    ptr: *mut u8,
    size: usize,
}

impl VideoTexGuard {
    /// The buffer as a mutable byte slice.
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `acquire_video_texture` confirmed that `ptr` was the
        // published buffer of `size` bytes *after* setting the hazard
        // pointer, and the main thread never frees or reuses a buffer the
        // hazard pointer names (`retire_video_texture_buffer`). The video
        // thread is the only CPU writer; the GE only reads it.
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.size) }
    }
}

impl Drop for VideoTexGuard {
    fn drop(&mut self) {
        VIDEO_TEX_IN_USE.store(core::ptr::null_mut(), Ordering::SeqCst);
    }
}

/// Claim the published texture buffer for one frame's CSC write. Video
/// thread only; call once per frame (the buffer may change between frames).
/// Returns `None` while no buffer is published (between tunes, or for the
/// instant the main thread swaps buffers): the caller skips the frame.
pub(crate) fn acquire_video_texture() -> Option<VideoTexGuard> {
    // A retry is only needed if the main thread swaps buffers between the
    // loads; several in a row means it is busy doing so — skip the frame.
    for _ in 0..3 {
        let ptr = VIDEO_TEX_PTR.load(Ordering::SeqCst);
        if ptr.is_null() {
            return None;
        }
        let size = VIDEO_TEX_SIZE.load(Ordering::SeqCst);
        VIDEO_TEX_IN_USE.store(ptr, Ordering::SeqCst);
        if VIDEO_TEX_PTR.load(Ordering::SeqCst) == ptr
            && VIDEO_TEX_SIZE.load(Ordering::SeqCst) == size
            && size > 0
        {
            return Some(VideoTexGuard { ptr, size });
        }
        VIDEO_TEX_IN_USE.store(core::ptr::null_mut(), Ordering::SeqCst);
    }
    None
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
    // SAFETY: only the main thread (the caller) publishes, retires and
    // frees the buffer behind VIDEO_TEX_PTR, so it stays allocated for as
    // long as the caller holds this borrow (it must not reallocate the
    // video texture while holding it). The video thread may overwrite the
    // bytes with the next frame; the main thread only compares the slice's
    // address against the texture buffer and hands it to the GE.
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
