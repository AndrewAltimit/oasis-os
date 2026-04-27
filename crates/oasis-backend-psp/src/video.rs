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

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use psp::sync::SpscQueue;
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

/// Max video frames to decode for >480p before switching to audio-only.
/// With the kernel PRX ME watchdog hook (3s timeout on WaitEventFlag),
/// the deadlock auto-recovers. This limit is a secondary safety net.
/// Adjustable via TCP "video-limit <N>".
static VIDEO_FRAME_LIMIT: AtomicU32 = AtomicU32::new(500);

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
    pub frame_limit: u32,
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
        frame_limit: VIDEO_FRAME_LIMIT.load(Ordering::Relaxed),
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

/// Set the video frame limit for >480p content.
pub fn set_video_frame_limit(n: u32) {
    VIDEO_FRAME_LIMIT.store(n, Ordering::Relaxed);
}

/// Get current video frame limit.
pub fn video_frame_limit() -> u32 {
    VIDEO_FRAME_LIMIT.load(Ordering::Relaxed)
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
    /// Start decoding a downloaded MP4 file.
    Play { path: String, seek_secs: u64 },
    /// Begin streaming mode — I/O thread will push frames via the stream queue.
    StreamStart,
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
    /// Always 0 — single-buffered. Kept for backwards-compatible API
    /// shape; future work could add a second slot for tear-free
    /// double buffering when memory permits.
    pub buf_idx: u8,
    pub width: u32,
    pub height: u32,
    /// CSC output stride in pixels (512 for ≤480p). Matches texture buf_w.
    pub stride: u32,
}

/// Pointer to the GU video texture's pixel buffer. Set by `PspBackend::
/// alloc_video_texture` and read by `NalDecoder::decode` so CSC writes
/// directly into the GU-bound buffer (zero-copy). When non-null, the
/// video thread writes CSC output here instead of `FRAME_BUFFERS`,
/// eliminating a 491 KB allocation and a per-frame memcpy. The buffer
/// is allocated by the main thread before the channel tune begins
/// (when ~7.5 MB of partition memory is still free) and reused for
/// every subsequent stream — see `PERSISTENT_DECODER` for the parallel
/// reasoning on the sceMpeg side.
///
/// SAFETY: stored pointer outlives the program (allocated through the
/// texture system, freed only at app exit). Accessed read-only by the
/// video thread and written only on the main thread before the video
/// thread starts decoding.
pub static VIDEO_TEX_PTR: core::sync::atomic::AtomicPtr<u8> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());
/// Size in bytes of the buffer pointed to by `VIDEO_TEX_PTR`.
pub static VIDEO_TEX_SIZE: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Publish the texture buffer pointer + size for the video thread to
/// use as CSC output target. Called by `PspBackend::alloc_video_texture`.
pub fn set_video_texture_buffer(ptr: *mut u8, size: usize) {
    VIDEO_TEX_PTR.store(ptr, Ordering::Release);
    VIDEO_TEX_SIZE.store(size, Ordering::Release);
}

/// Borrow the GU video texture buffer as a mutable slice. Returns
/// `None` if no texture has been allocated yet.
///
/// SAFETY: caller must guarantee single-writer access. The video thread
/// is the sole writer during `NalDecoder::decode`; the main thread does
/// not touch the buffer except via the GU (which reads via DMA).
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

/// The sceMpeg internal semaphore ID (at mpeg_data+0x66c).
static MPEG_INTERNAL_SEMA: core::sync::atomic::AtomicI32 =
    core::sync::atomic::AtomicI32::new(-1);

/// The SceMediaEngineRpc event flag UID. The ME kernel RPC handler blocks
/// on this with infinite timeout. Signalling it unblocks a stuck decode.
static ME_RPC_EVFLAG: core::sync::atomic::AtomicI32 =
    core::sync::atomic::AtomicI32::new(-1);

/// Scan kernel UIDs to find the SceMediaEngineRpc event flag.
fn find_me_rpc_event_flag() -> Option<psp::sys::SceUid> {
    let mut info: psp::sys::SceKernelEventFlagInfo = unsafe { core::mem::zeroed() };
    info.size = core::mem::size_of::<psp::sys::SceKernelEventFlagInfo>();

    // Event flag UIDs are typically in a low range. Scan 1..4096.
    for uid in 1..4096i32 {
        info.name = [0u8; 32];
        let ret = unsafe {
            psp::sys::sceKernelReferEventFlagStatus(
                psp::sys::SceUid(uid),
                &mut info,
            )
        };
        if ret >= 0 {
            let name = info.name.split(|&b| b == 0).next().unwrap_or(&[]);
            if name == b"SceMediaEngineRpc" {
                vlog_force(&format!(
                    "[VIDEO] found SceMediaEngineRpc evflag uid={uid:#x} \
                     pattern={:#x}",
                    info.current_pattern,
                ));
                return Some(psp::sys::SceUid(uid));
            }
        }
    }
    vlog_force("[VIDEO] SceMediaEngineRpc event flag NOT FOUND");
    None
}

/// Signal the ME RPC event flag to unblock a stuck WaitEventFlag.
pub fn signal_me_rpc_event_flag() {
    let id = ME_RPC_EVFLAG.load(core::sync::atomic::Ordering::Acquire);
    if id > 0 {
        unsafe {
            psp::sys::sceKernelSetEventFlag(psp::sys::SceUid(id), 1);
        }
    }
}

/// Address of the `jal 0x9fd4` instruction in the loaded mpeg_vsh370.prx
/// (at PRX VA 0x8678). When patched to `jr $ra; nop`, the ME kernel call
/// is skipped and the decode returns 0 (no frame produced).
static PRX_KERNEL_CALL_ADDR: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
/// Original instruction at the kernel call site (for restore).
static PRX_KERNEL_CALL_ORIG: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Patch the loaded PRX to skip the ME kernel call (return 0 = no frame).
/// Call this before sceMpegAvcDecode to prevent the ME deadlock.
pub fn patch_skip_me_call() {
    let addr = PRX_KERNEL_CALL_ADDR.load(core::sync::atomic::Ordering::Relaxed);
    if addr == 0 { return; }
    unsafe {
        let ptr = addr as *mut u32;
        // Replace `jal 0x9fd4` with `addiu $v0, $zero, -1` (return error)
        // so the caller skips the frame instead of using stale ME data.
        // 0x2402ffff = addiu $v0, $zero, -1
        *ptr = 0x2402ffff;
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            ptr as *const core::ffi::c_void, 8,
        );
        // PSP has split I/D cache. Flush D-cache to RAM and invalidate
        // I-cache so the CPU fetches the patched instruction.
        psp::sys::sceKernelDcacheWritebackInvalidateAll();
        // No icache binding available; use full D-cache flush which
        // on PSP also synchronizes the instruction stream.
    }
}

/// Restore the original ME kernel call instruction.
pub fn unpatch_me_call() {
    let addr = PRX_KERNEL_CALL_ADDR.load(core::sync::atomic::Ordering::Relaxed);
    let orig = PRX_KERNEL_CALL_ORIG.load(core::sync::atomic::Ordering::Relaxed);
    if addr == 0 || orig == 0 { return; }
    unsafe {
        let ptr = addr as *mut u32;
        *ptr = orig;
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            ptr as *const core::ffi::c_void, 8,
        );
        // PSP has split I/D cache. Flush D-cache to RAM and invalidate
        // I-cache so the CPU fetches the patched instruction.
        psp::sys::sceKernelDcacheWritebackInvalidateAll();
        // No icache binding available; use full D-cache flush which
        // on PSP also synchronizes the instruction stream.
    }
}

/// Signal the sceMpeg internal semaphore to unblock a stuck AvcDecode.
/// Called from the main thread watchdog when DECODE_STEP == 2.
pub fn unblock_stuck_decode() {
    let id = MPEG_INTERNAL_SEMA.load(core::sync::atomic::Ordering::Acquire);
    if id > 0 {
        vlog_force(&format!("[VIDEO] unblocking stuck decode, sema={id:#x}"));
        // SAFETY: id is a valid semaphore read from the mpeg instance.
        unsafe {
            psp::sys::sceKernelSignalSema(psp::sys::SceUid(id), 1);
        }
    }
}

/// Kernel semaphore signalled by I/O thread when a stream frame is pushed.
/// The video thread waits on this instead of polling with sleep.
static STREAM_FRAME_SEMA: core::sync::atomic::AtomicI32 =
    core::sync::atomic::AtomicI32::new(-1);

/// Create the stream frame semaphore (called once at video thread start).
fn create_stream_sema() {
    // SAFETY: sceKernelCreateSema with valid name and parameters.
    let id = unsafe {
        psp::sys::sceKernelCreateSema(
            b"oasis_vframe\0".as_ptr(),
            0,   // attr: FIFO
            0,   // initial value
            8,   // max value (matches VIDEO_STREAM_QUEUE capacity)
            core::ptr::null_mut(),
        )
    };
    if id >= psp::sys::SceUid(0) {
        STREAM_FRAME_SEMA.store(id.0, core::sync::atomic::Ordering::Release);
    }
}

/// Signal the stream frame semaphore (called by I/O thread after pushing).
pub fn signal_stream_frame() {
    let id = STREAM_FRAME_SEMA.load(core::sync::atomic::Ordering::Acquire);
    if id >= 0 {
        // SAFETY: id is a valid semaphore created by create_stream_sema.
        unsafe {
            psp::sys::sceKernelSignalSema(psp::sys::SceUid(id), 1);
        }
    }
}

/// Wait for a stream frame to be available (with timeout).
/// Returns true if signalled, false on timeout.
fn wait_stream_frame(timeout_us: u32) -> bool {
    let id = STREAM_FRAME_SEMA.load(core::sync::atomic::Ordering::Acquire);
    if id < 0 {
        // Semaphore not created; fall back to sleep.
        unsafe { psp::sys::sceKernelDelayThread(timeout_us) };
        return false;
    }
    let mut timeout = timeout_us;
    // SAFETY: id is a valid semaphore, timeout pointer is valid.
    let ret = unsafe {
        psp::sys::sceKernelWaitSema(
            psp::sys::SceUid(id),
            1,
            &mut timeout,
        )
    };
    ret >= 0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send a command to the video decode thread.
pub fn send_video_cmd(cmd: VideoCmd) {
    let _ = VIDEO_CMD_QUEUE.push(cmd);
}

/// Poll for the next decoded video frame (non-blocking).
///
/// Returns the frame metadata. Pixel data is in the static
/// `FRAME_BUFFERS[frame.buf_idx]` — the caller must copy it
/// before the next `poll_video_frame` call overwrites it.
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

pub fn poll_video_frame() -> Option<DecodedFrame> {
    VIDEO_POLL_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let frame = VIDEO_FRAME_QUEUE.pop();
    if frame.is_some() {
        VIDEO_FRAMES_POLLED.fetch_add(1, Ordering::Relaxed);
    }
    frame
}

/// Return raw frame buffer reference WITHOUT alpha fixup.
/// Buffer uses CSC stride (512px), matching texture buf_w.
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
        }
        Err(f) => Err(f),
    }
}

/// Pre-initialize the MPEG subsystem before any audio modules load.
///
/// Must be called from the main thread before spawning the audio thread.
pub fn preinit_mpeg() {
    crate::audio::load_av_modules_once_pub();
    load_avmp3_module();
    vlog("[VIDEO] preinit done");
}

/// Pre-load AvMp3 module during init (before mpeg_vsh370.prx).
fn load_avmp3_module() {
    unsafe {
        let r = psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMp3);
        vlog(&format!("[VIDEO] AvMp3 = {r:#x}"));
    }
}

/// Load sceMpeg implementation via mpeg_vsh370.prx.
///
/// AvMpegBase (the system's built-in sceMpeg) does NOT work with the NAL
/// decode path — it returns 0x80628002 even with correct mode + DDR top
/// parameters. The mode 4/5 + DDR top convention is specific to mpeg_vsh370
/// (decrypted FW 3.71 module used by PMPlayer). AvMpegBase only supports
/// the standard PSMF ringbuffer path (sceMpegGetAvcAu), not the NAL path
/// (sceMpegGetAvcNalAu).
///
/// mpeg_vsh370.prx registers "sceMpeg" via self-import when started,
/// which triggers the kernel to resolve the EBOOT's weak import stubs.
fn load_mpeg_vsh_module() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOADED: AtomicBool = AtomicBool::new(false);
    if LOADED.swap(true, Ordering::Relaxed) {
        return;
    }

    // Boot the ME first (mpeg_vsh370 needs it running).
    load_me_boot_prx();

    let id = unsafe {
        psp::sys::sceKernelLoadModule(
            b"ms0:/PSP/GAME/OASISOS/mpeg_vsh370.prx\0".as_ptr(),
            0, core::ptr::null_mut(),
        )
    };
    vlog(&format!("[VIDEO] sceKernelLoadModule = {:#x}", id.0));
    if id < psp::sys::SceUid(0) {
        vlog("[VIDEO] mpeg_vsh370 load FAILED — H.264 decode unavailable");
        return;
    }

    let mut status: i32 = 0;
    let ret = unsafe {
        psp::sys::sceKernelStartModule(
            id, 0, core::ptr::null_mut(),
            &mut status, core::ptr::null_mut(),
        )
    };
    vlog(&format!("[VIDEO] sceKernelStartModule = {ret:#x}, status={status:#x}"));
    if ret < 0 {
        vlog("[VIDEO] mpeg_vsh370 start FAILED");
        unsafe { psp::sys::sceKernelUnloadModule(id); }
        return;
    }
    vlog("[VIDEO] sceMpeg stubs resolved via mpeg_vsh370");
}

/// Boot the Media Engine via oasis-me-boot.prx (if available).
fn load_me_boot_prx() {
    let boot_paths: &[&[u8]] = &[
        b"ms0:/PSP/GAME/OASISOS/oasis-me-boot.prx\0",
    ];
    for path in boot_paths {
        let id = unsafe {
            psp::sys::sceKernelLoadModule(
                path.as_ptr(), 0, core::ptr::null_mut(),
            )
        };
        if id >= psp::sys::SceUid(0) {
            let mut status: i32 = 0;
            let ret = unsafe {
                psp::sys::sceKernelStartModule(
                    id, 0, core::ptr::null_mut(),
                    &mut status, core::ptr::null_mut(),
                )
            };
            let name = core::str::from_utf8(&path[..path.len()-1]).unwrap_or("?");
            vlog(&format!("[VIDEO] ME boot {name} = {ret:#x}"));
            return;
        }
    }
    vlog("[VIDEO] no ME boot PRX found (ME may already be booted by AAC)");
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
// PSP file reader wrapper for demux_lite
// ---------------------------------------------------------------------------

/// Adapter implementing `Read + Seek` over PSP `sceIo*` file I/O.
struct PspFileReader {
    fd: psp::sys::SceUid,
}

impl PspFileReader {
    fn seek_read(&mut self, offset: u64, buf: &mut [u8]) {
        unsafe {
            psp::sys::sceIoLseek(self.fd, offset as i64, psp::sys::IoWhence::Set);
            psp::sys::sceIoRead(self.fd, buf.as_mut_ptr() as *mut _, buf.len() as u32);
        }
    }

    fn open(path: &str) -> Option<Self> {
        let mut path_bytes: Vec<u8> = path.as_bytes().to_vec();
        path_bytes.push(0);
        // SAFETY: path_bytes is a null-terminated byte string.
        let fd =
            unsafe { psp::sys::sceIoOpen(path_bytes.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
        if fd < psp::sys::SceUid(0) {
            return None;
        }
        Some(Self { fd })
    }
}

impl std::io::Read for PspFileReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: self.fd is a valid file descriptor opened above.
        // buf pointer and len are valid.
        let n =
            unsafe { psp::sys::sceIoRead(self.fd, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
        if n < 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "sceIoRead failed",
            ))
        } else {
            Ok(n as usize)
        }
    }
}

impl std::io::Seek for PspFileReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let (offset, whence) = match pos {
            std::io::SeekFrom::Start(n) => (n as i64, psp::sys::IoWhence::Set),
            std::io::SeekFrom::End(n) => (n, psp::sys::IoWhence::End),
            std::io::SeekFrom::Current(n) => (n, psp::sys::IoWhence::Cur),
        };
        // SAFETY: self.fd is a valid file descriptor.
        let result = unsafe { psp::sys::sceIoLseek(self.fd, offset, whence) };
        if result < 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "sceIoLseek failed",
            ))
        } else {
            Ok(result as u64)
        }
    }
}

impl Drop for PspFileReader {
    fn drop(&mut self) {
        if self.fd >= psp::sys::SceUid(0) {
            // SAFETY: fd is valid; close after use.
            unsafe { psp::sys::sceIoClose(self.fd) };
        }
    }
}


// ---------------------------------------------------------------------------
// NAL-based H.264 decoder — thin wrapper around psp::mpeg::AvcDecoder
// ---------------------------------------------------------------------------

/// NAL-based H.264 decoder using the high-level `psp::mpeg::AvcDecoder`.
///
/// Adds OASIS-specific logging and SPS/PPS extraction on top of the
/// reusable rust-psp abstraction.
struct NalDecoder {
    /// Wrapped in `Option` so `Drop` can move the decoder back into
    /// `PERSISTENT_DECODER` for reuse on the next channel tune. Always
    /// `Some` for the lifetime of a valid `NalDecoder`.
    decoder: Option<psp::mpeg::AvcDecoder>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    nal_prefix_size: i32,
    first_frame: bool,
    /// Frames decoded since last flush/keyframe.
    frames_since_flush: u32,
    /// Maximum frames before a preventive flush.
    flush_interval: u32,
    /// Timestamp of last decode call (for rate throttling).
    last_decode_us: u64,
}

/// Persistent `AvcDecoder` kept alive across channel switches.
///
/// `rust-psp` skips `sceMpegDelete` in `AvcDecoder::Drop` to avoid an
/// intermittent firmware crash, which leaves the sceMpeg instance leaked.
/// On the next `sceMpegCreate`, mpeg_vsh370 returns 0x80628002
/// (`SCE_ERROR_MPEG_NO_MEMORY`) because the firmware still tracks the
/// prior instance. Reusing one decoder for every tune sidesteps the
/// Delete/Create cycle entirely.
///
/// On stream end, `Drop for NalDecoder` parks the decoder back here.
/// On the next tune, `NalDecoder::try_init` takes it back. The first
/// allocation is at the actual stream dimensions; subsequent streams
/// with ≤480p content reuse it (CSC stride is fixed at 512 for any
/// `width≤480`, so the same instance handles 320×240, 336×240, 480×272,
/// etc. — only `is_first_frame=true` and `flush()` are needed to reset
/// pic_num between streams).
///
/// SAFETY: Touched only on the video thread (single-producer through
/// `try_init` / `drop`). No cross-thread access.
static mut PERSISTENT_DECODER: Option<psp::mpeg::AvcDecoder> = None;

impl NalDecoder {
    /// Initialize the NAL decoder from the first keyframe.
    fn try_init(first_frame: &StreamFrame) -> Result<Self, String> {
        vlog("[VIDEO] NalDecoder::try_init");
        crate::audio::load_av_modules_once_pub();
        load_mpeg_vsh_module();

        // SPS/PPS from MP4 avcC atom (always present on keyframes).
        let (mut sps, pps) = if let (Some(s), Some(p)) =
            (&first_frame.avcc_sps, &first_frame.avcc_pps)
        {
            (s.clone(), p.clone())
        } else {
            return Err("no SPS/PPS on first keyframe".to_string());
        };

        if sps.len() >= 4 {
            vlog(&format!(
                "[VIDEO] NAL: SPS={} PPS={} profile={:#x} level={:#x}",
                sps.len(), pps.len(), sps[1], sps[3]
            ));
        }

        // Parse dimensions and ref frame count from the raw SPS NAL.
        let sps_info = parse_sps_info(&sps);
        let (width, height) = sps_info
            .as_ref()
            .map(|i| (i.width, i.height))
            .unwrap_or((480, 272));

        // Use actual dimensions — mode 5 for >480p with Psm5650 output
        // to reduce ME internal buffer pressure.
        let (dec_w, dec_h) = (width, height);

        let max_ref_frames = sps_info.as_ref().map_or(4, |i| i.max_ref_frames);
        let dpb_frame_bytes = (dec_w * dec_h * 3 / 2) as usize;
        let dpb_total = dpb_frame_bytes * (max_ref_frames as usize + 1);
        vlog(&format!(
            "[VIDEO] NAL: {width}x{height} refs={max_ref_frames} \
             dpb={dpb_total}B (2MB workspace={})",
            if dpb_total > 0x20_0000 { "OVERFLOW" } else { "ok" }
        ));

        // Reuse the persistent decoder if available; otherwise allocate a
        // fresh one. See `PERSISTENT_DECODER` for why we never drop.
        // SAFETY: Single-threaded access (video thread only). Use raw
        // pointer ops to avoid creating a `&mut` to the `static mut`,
        // which Rust 2024 forbids (`static_mut_refs`).
        let prev: Option<psp::mpeg::AvcDecoder> = unsafe {
            core::ptr::replace(core::ptr::addr_of_mut!(PERSISTENT_DECODER), None)
        };
        let mut decoder = match prev {
            Some(mut dec) => {
                vlog(&format!(
                    "[VIDEO] NAL: reusing persistent decoder ({}x{}) for {dec_w}x{dec_h} stream",
                    dec.width(),
                    dec.height(),
                ));
                // Reset stream-specific state so the ME treats this as a
                // fresh stream start. `flush()` zeroes pic_num.
                dec.flush();
                dec
            },
            None => {
                let dec = psp::mpeg::AvcDecoder::new(dec_w, dec_h)
                    .map_err(|e| format!("AvcDecoder::new: {e}"))?;
                vlog(&format!(
                    "[VIDEO] NAL: created persistent decoder ({dec_w}x{dec_h}), ddr={:#x}",
                    dec.ddr_top(),
                ));
                dec
            },
        };
        vlog(&format!(
            "[VIDEO] NAL: decoder ready, ddr={:#x}",
            decoder.ddr_top()
        ));

        // CSC writes directly into the pre-allocated GU video texture
        // (see `VIDEO_TEX_PTR` / `set_video_texture_buffer`). No
        // FRAME_BUFFERS allocation is needed — the texture buffer is
        // sized for the maximum ≤480p frame (512 × 256 × 4 = 524288
        // bytes) and is reused across streams. Skipping the 982 KB
        // double-buffer allocation here is required because by this
        // point the persistent sceMpeg DDR workspace has already eaten
        // ~6.5 MB of partition memory, leaving roughly 1 MB free.
        let csc_stride = decoder.stride();
        let out_h = ((dec_h + 15) / 16) * 16; // CSC rounds up to 16
        let _ = (csc_stride, out_h); // referenced for diagnostic logging only
        if VIDEO_TEX_PTR.load(Ordering::Acquire).is_null() {
            // Park the decoder back into PERSISTENT_DECODER before
            // returning. Otherwise the local `decoder` drops here without
            // having been wrapped in `NalDecoder` — and `NalDecoder::Drop`
            // is the only code that re-parks. Leaking it would force the
            // next tune to call `AvcDecoder::new` again, which fails with
            // 0x80628002 because the firmware still tracks the previous
            // instance.
            // SAFETY: single-threaded access (video thread only); same
            // pattern as the take above.
            unsafe {
                core::ptr::replace(
                    core::ptr::addr_of_mut!(PERSISTENT_DECODER),
                    Some(decoder),
                );
            }
            return Err(
                "GU video texture not pre-allocated (TV Guide tune-press \
                 path is responsible for calling PspBackend::alloc_video_texture)"
                    .to_string(),
            );
        }

        // Flush interval: only used for >480p content where the DPB
        // exceeds the 2MB workspace. For <=480p, no periodic reset
        // (sceMpegDelete crashes after prolonged decode).
        let flush_interval = if dpb_total > 0x1C_0000 {
            70u32
        } else {
            u32::MAX
        };
        // Find the PRX base address and locate the ME kernel call
        // instruction for runtime patching.
        unsafe {
            let stub_ptr = psp::sys::sceMpegAvcDecode as *const u32;
            let insn0 = *stub_ptr;
            let insn1 = *stub_ptr.add(1);
            let target = if (insn0 >> 26) == 0x02 {
                (insn0 & 0x03FFFFFF) << 2
            } else {
                0
            };
            vlog(&format!(
                "[VIDEO] NAL: AvcDecode stub={:#010x} insn={insn0:#010x} \
                 target={target:#010x}",
                stub_ptr as u32,
            ));
            // Extract the import stub address from the psp_extern wrapper.
            // Layout: [0]=addiu sp  [1]=sw ra  [2]=lui $v0,HI  [3]=lw arg
            //         [4]=addiu $v0,LO  [5]=sw $v0  [6]=jal  [7]=sw arg
            let lui_insn = *stub_ptr.add(2);    // lui $v0, HI
            let addiu_insn = *stub_ptr.add(4);  // addiu $v0, $v0, LO
            let hi = (lui_insn & 0xFFFF) << 16;
            let lo = (addiu_insn & 0xFFFF) as i16 as i32 as u32;
            let import_stub = hi.wrapping_add(lo);
            vlog(&format!(
                "[VIDEO] NAL: import stub @ {import_stub:#010x}"
            ));
            // Dump the import stub (should be: j <prx_func>; nop)
            if import_stub > 0x08000000 {
                let isp = import_stub as *const u32;
                let is0 = *isp;
                let is1 = *isp.add(1);
                vlog(&format!(
                    "[VIDEO] NAL: import stub code: {is0:#010x} {is1:#010x}"
                ));
                // Decode j target
                if (is0 >> 26) == 2 {
                    let prx_func = ((is0 & 0x03FFFFFF) << 2)
                        | (import_stub & 0xF0000000);
                    vlog(&format!(
                        "[VIDEO] NAL: -> PRX func @ {prx_func:#010x}"
                    ));
                    // Dump first 4 instructions of the PRX function
                    let pp = prx_func as *const u32;
                    vlog(&format!(
                        "[VIDEO] NAL: PRX[0-3]: {:08x} {:08x} {:08x} {:08x}",
                        *pp, *pp.add(1), *pp.add(2), *pp.add(3),
                    ));
                } else if (is0 >> 26) == 3 {
                    // JAL - might be kernel syscall
                    let target = ((is0 & 0x03FFFFFF) << 2)
                        | (import_stub & 0xF0000000);
                    vlog(&format!(
                        "[VIDEO] NAL: -> kernel @ {target:#010x}"
                    ));
                } else {
                    // Might be a syscall or other pattern
                    vlog(&format!(
                        "[VIDEO] NAL: import stub opcode={}", is0 >> 26
                    ));
                }
            }
            // Trace the full call chain to find where sceMpegAvcDecode
            // actually ends up (kernel syscall? PRX user-mode code?).
            // Follow jal/j instructions up to 3 levels deep.
            let mut addr = stub_ptr as u32;
            for level in 0..4u32 {
                let p = addr as *const u32;
                // Scan up to 16 instructions for a jal or j
                let mut found_target = 0u32;
                for i in 0..16u32 {
                    let w = *p.add(i as usize);
                    let op = w >> 26;
                    if op == 2 || op == 3 {
                        // J (2) or JAL (3)
                        found_target = ((w & 0x03FFFFFF) << 2)
                            | (addr & 0xF0000000);
                        vlog(&format!(
                            "[VIDEO] NAL: L{level} @{addr:#010x}+{}: \
                             {} {found_target:#010x}",
                            i * 4,
                            if op == 2 { "j" } else { "jal" },
                        ));
                        break;
                    }
                    // Check for syscall instruction (opcode 0, func 0xC)
                    if (w & 0xFC00003F) == 0x0000000C {
                        let code = (w >> 6) & 0xFFFFF;
                        vlog(&format!(
                            "[VIDEO] NAL: L{level} @{addr:#010x}+{}: \
                             SYSCALL {code:#x}",
                            i * 4,
                        ));
                        found_target = 0;
                        break;
                    }
                }
                if found_target == 0 {
                    // Dump 8 raw words at current address for manual analysis
                    let dp = addr as *const u32;
                    vlog(&format!(
                        "[VIDEO] NAL: L{level} @{addr:#010x} raw: \
                         {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                        *dp, *dp.add(1), *dp.add(2), *dp.add(3),
                        *dp.add(4), *dp.add(5), *dp.add(6), *dp.add(7),
                    ));
                    break;
                }
                addr = found_target;
            }
            // Compute PRX base and find the ME kernel call site.
            // Import stub → j <prx_dispatch> → PRX dispatch at VA 0x71c0
            // PRX base = dispatch_addr - 0x71c0
            if import_stub > 0x08000000 {
                let isp = import_stub as *const u32;
                let is0 = *isp;
                if (is0 >> 26) == 2 {
                    let dispatch_addr = ((is0 & 0x03FFFFFF) << 2)
                        | (import_stub & 0xF0000000);
                    let prx_base = dispatch_addr.wrapping_sub(0x71c0);
                    // The kernel call is at PRX VA 0x8678
                    let kernel_call_addr = prx_base + 0x8678;
                    let orig_insn = *(kernel_call_addr as *const u32);
                    vlog(&format!(
                        "[VIDEO] NAL: PRX base={prx_base:#010x} \
                         kernel_call@{kernel_call_addr:#010x}={orig_insn:#010x}"
                    ));
                    PRX_KERNEL_CALL_ADDR.store(
                        kernel_call_addr,
                        core::sync::atomic::Ordering::Release,
                    );
                    PRX_KERNEL_CALL_ORIG.store(
                        orig_insn,
                        core::sync::atomic::Ordering::Release,
                    );
                }
            }

            // Follow the target and dump a few instructions
            if target > 0x08000000 {
                let t = target as *const u32;
                let i0 = *t;
                let i1 = *t.add(1);
                let i2 = *t.add(2);
                vlog(&format!(
                    "[VIDEO] NAL: @target: {i0:#010x} {i1:#010x} {i2:#010x}"
                ));
                // Follow trampoline if j instruction
                if (i0 >> 26) == 0x02 {
                    let t2 = (i0 & 0x03FFFFFF) << 2;
                    vlog(&format!("[VIDEO] NAL: trampoline -> {t2:#010x}"));
                }
            }
        }

        // Find the ME RPC event flag for timeout-based recovery.
        if ME_RPC_EVFLAG.load(core::sync::atomic::Ordering::Relaxed) < 0 {
            if let Some(evf) = find_me_rpc_event_flag() {
                ME_RPC_EVFLAG.store(evf.0, core::sync::atomic::Ordering::Release);
            }
        }

        // Publish the internal semaphore ID for the watchdog.
        if let Some(sema) = decoder.internal_sema_id() {
            MPEG_INTERNAL_SEMA.store(sema.0, core::sync::atomic::Ordering::Release);
            vlog(&format!(
                "[VIDEO] NAL: internal sema={:#x}",
                sema.0,
            ));
        }

        vlog(&format!(
            "[VIDEO] NAL: flush_interval={flush_interval}"
        ));

        Ok(Self {
            decoder: Some(decoder),
            sps,
            pps,
            nal_prefix_size: first_frame.nal_prefix_size as i32,
            first_frame: true,
            frames_since_flush: 0,
            flush_interval,
            last_decode_us: 0,
        })
    }

    /// Decode one H.264 access unit (raw AVCC format) into a pre-allocated
    /// static buffer. Returns:
    /// - `Ok(Some(frame))` — frame decoded successfully
    /// - `Ok(None)` — no picture yet (B-frame reordering)
    /// - `Err(())` — decode error
    fn decode(
        &mut self, avcc_data: &[u8], _pts_secs: f64,
        avcc_prefix: u8, is_keyframe: bool,
    ) -> Result<Option<DecodedFrame>, ()> {
        if avcc_data.is_empty() {
            return Ok(None);
        }

        static DECODE_COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);
        let call_num = DECODE_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let verbose = call_num < 10;

        self.frames_since_flush += 1;

        let is_first = self.first_frame;
        self.first_frame = false;

        let nal = psp::mpeg::AvcNal {
            sps: &self.sps,
            pps: &self.pps,
            data: avcc_data,
            prefix_size: avcc_prefix as i32,
            is_first_frame: is_first,
        };

        // Zero-copy CSC: write directly into the GU video texture buffer.
        // The texture is pre-allocated by the main thread when the user
        // presses X to tune (`PspBackend::alloc_video_texture` →
        // `set_video_texture_buffer`). Single-buffered to fit memory
        // alongside the persistent sceMpeg DDR workspace.
        // SAFETY: video thread is the sole writer; main thread reads via
        // GU DMA, which fences on `sceGuFinish`.
        let dst = match unsafe { video_texture_slice() } {
            Some(s) => s,
            None => {
                if verbose {
                    vlog("[VIDEO] NAL: no texture buffer registered, skipping");
                }
                return Err(());
            },
        };

        // Time the decode call — if it takes >2 seconds, the ME is stuck
        // and we should stop video decode entirely.
        let t0 = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;

        // SAFETY: `decoder` is `Some` for the lifetime of NalDecoder; only
        // `Drop` ever takes it.
        let dec = self
            .decoder
            .as_mut()
            .expect("decoder Some until Drop");
        let result = dec.decode_csc_direct(&nal, dst);
        let (out_w, out_h, out_stride) = (dec.width(), dec.height(), dec.stride());
        match result {
            Ok(true) => {
                let dt_ms = (unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64
                    - t0) / 1000;
                if verbose {
                    vlog(&format!("[VIDEO] NAL: FRAME DECODED! ({}ms)", dt_ms));
                }
                self.last_decode_us = dt_ms * 1000;
                Ok(Some(DecodedFrame {
                    buf_idx: 0,
                    width: out_w,
                    height: out_h,
                    stride: out_stride,
                }))
            }
            Ok(false) => {
                if verbose {
                    vlog("[VIDEO] NAL: no picture yet (reordering)");
                }
                Ok(None)
            }
            Err(e) => {
                if verbose || call_num < 50 {
                    vlog(&format!("[VIDEO] NAL: decode error: {e}"));
                }
                Err(())
            }
        }
    }

}

impl Drop for NalDecoder {
    fn drop(&mut self) {
        // Park the decoder in PERSISTENT_DECODER for the next stream
        // instead of letting `AvcDecoder::Drop` run. See PERSISTENT_DECODER
        // for the firmware bug we're working around.
        if let Some(dec) = self.decoder.take() {
            // SAFETY: Video thread is the only writer of PERSISTENT_DECODER.
            // `try_init` is the only reader, also on the video thread, and
            // is single-threaded with this Drop. The previous value should
            // already be `None` (taken in `try_init`); if it isn't, the
            // returned `Option` drops it harmlessly.
            unsafe {
                let _ = core::ptr::replace(
                    core::ptr::addr_of_mut!(PERSISTENT_DECODER),
                    Some(dec),
                );
            }
        }
    }
}

/// Patch the `max_num_ref_frames` field in a raw SPS NAL unit.
///
/// Navigates to the exp-golomb coded field and rewrites it in-place.
/// Returns true if the patch was applied successfully.
fn patch_sps_max_ref_frames(sps: &mut Vec<u8>, new_refs: u32) -> bool {
    if sps.len() < 5 {
        return false;
    }

    let profile_idc = sps[1];
    let mut reader = BitReader::new(&sps[4..]);

    // Navigate to max_num_ref_frames (same path as parse_sps_info)
    if reader.read_ue().is_none() { return false; } // sps_id

    if profile_idc == 100 || profile_idc == 110 || profile_idc == 122
        || profile_idc == 244 || profile_idc == 44 || profile_idc == 83
        || profile_idc == 86 || profile_idc == 118 || profile_idc == 128
    {
        let chroma = reader.read_ue();
        if chroma.is_none() { return false; }
        if chroma == Some(3) { if reader.skip(1).is_none() { return false; } }
        if reader.read_ue().is_none() { return false; } // bit_depth_luma
        if reader.read_ue().is_none() { return false; } // bit_depth_chroma
        if reader.skip(1).is_none() { return false; }
        let scaling = reader.read_bit();
        if scaling == Some(1) {
            let count = if chroma != Some(3) { 8 } else { 12 };
            for i in 0..count {
                let present = reader.read_bit();
                if present == Some(1) {
                    let size = if i < 6 { 16 } else { 64 };
                    let mut last = 8i32;
                    let mut next = 8i32;
                    for _ in 0..size {
                        if next != 0 {
                            let d = match reader.read_se() { Some(v) => v, None => return false };
                            next = (last + d + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }

    if reader.read_ue().is_none() { return false; } // log2_max_frame_num
    let poc_type = match reader.read_ue() { Some(v) => v, None => return false };
    if poc_type == 0 {
        if reader.read_ue().is_none() { return false; }
    } else if poc_type == 1 {
        if reader.skip(1).is_none() { return false; }
        if reader.read_se().is_none() { return false; }
        if reader.read_se().is_none() { return false; }
        let n = match reader.read_ue() { Some(v) => v, None => return false };
        for _ in 0..n { if reader.read_se().is_none() { return false; } }
    }

    // Now reader is positioned right before max_num_ref_frames.
    // Record the bit position.
    let ref_byte = reader.byte_pos;
    let ref_bit = reader.bit_pos;

    // Read the current value to know its exp-golomb size.
    let old_refs = match reader.read_ue() { Some(v) => v, None => return false };

    // Exp-golomb encoding of new_refs:
    // value N encodes as: (leading_zeros zeros)(1)(N+1 in binary, leading_zeros bits)
    // For 0: "1" (1 bit)
    // For 1: "010" (3 bits)
    // For 2: "011" (3 bits)
    // For 3: "00100" (5 bits)

    // Calculate bit lengths
    fn ue_bit_len(val: u32) -> u32 {
        if val == 0 { return 1; }
        let n = val + 1;
        let bits = 32 - n.leading_zeros(); // ceil(log2(n+1))
        2 * bits - 1
    }

    let old_len = ue_bit_len(old_refs);
    let new_len = ue_bit_len(new_refs);

    // Only patch if new encoding is same size (avoids shifting the entire
    // bitstream). For refs 3->1: 5 bits -> 3 bits — different size.
    // For refs 3->2: 5 bits -> 3 bits — also different.
    // This is tricky. Let's handle the simple same-size case first,
    // then if sizes differ, we need to rebuild the SPS.
    if old_len != new_len {
        // Rebuild SPS with modified ref count. Copy bits up to the ref
        // field, write new value, copy remaining bits.
        let total_bits = (sps.len() - 4) * 8; // bits after byte 4
        let ref_start_bit = ref_byte * 8 + ref_bit as usize;
        let ref_end_bit = ref_start_bit + old_len as usize;
        let tail_bits = total_bits.saturating_sub(ref_end_bit);

        let new_total_bits = ref_start_bit + new_len as usize + tail_bits;
        let new_total_bytes = (new_total_bits + 7) / 8;
        let mut new_sps = vec![0u8; 4 + new_total_bytes];
        new_sps[..4].copy_from_slice(&sps[..4]); // copy header

        // Copy bits before ref field
        let src = &sps[4..];
        let dst = &mut new_sps[4..];
        for b in 0..ref_start_bit {
            let byte_idx = b / 8;
            let bit_idx = 7 - (b % 8);
            let val = (src[byte_idx] >> bit_idx) & 1;
            let d_byte = b / 8;
            let d_bit = 7 - (b % 8);
            dst[d_byte] = (dst[d_byte] & !(1 << d_bit)) | (val << d_bit);
        }

        // Write new ref value (exp-golomb)
        let ue_val = new_refs + 1;
        let leading = (new_len - 1) / 2;
        let mut pos = ref_start_bit;
        // Write leading zeros
        for _ in 0..leading {
            let d_byte = pos / 8;
            let d_bit = 7 - (pos % 8);
            dst[d_byte] &= !(1 << d_bit);
            pos += 1;
        }
        // Write 1
        {
            let d_byte = pos / 8;
            let d_bit = 7 - (pos % 8);
            dst[d_byte] |= 1 << d_bit;
            pos += 1;
        }
        // Write suffix bits
        for i in (0..leading).rev() {
            let d_byte = pos / 8;
            let d_bit = 7 - (pos % 8);
            let val = ((ue_val >> i) & 1) as u8;
            dst[d_byte] = (dst[d_byte] & !(1 << d_bit)) | (val << d_bit);
            pos += 1;
        }

        // Copy tail bits
        for b in 0..tail_bits {
            let s_pos = ref_end_bit + b;
            let s_byte = s_pos / 8;
            let s_bit = 7 - (s_pos % 8);
            let val = (src[s_byte] >> s_bit) & 1;
            let d_byte = pos / 8;
            let d_bit = 7 - (pos % 8);
            dst[d_byte] = (dst[d_byte] & !(1 << d_bit)) | (val << d_bit);
            pos += 1;
        }

        sps.clear();
        sps.extend_from_slice(&new_sps);
        return true;
    }

    // Same-size: just overwrite the bits in-place
    let ue_val = new_refs + 1;
    let leading = (new_len - 1) / 2;
    let mut pos = ref_byte * 8 + ref_bit as usize;
    let dst = &mut sps[4..];
    for _ in 0..leading {
        let d_byte = pos / 8;
        let d_bit = 7 - (pos % 8);
        dst[d_byte] &= !(1 << d_bit);
        pos += 1;
    }
    {
        let d_byte = pos / 8;
        let d_bit = 7 - (pos % 8);
        dst[d_byte] |= 1 << d_bit;
        pos += 1;
    }
    for i in (0..leading).rev() {
        let d_byte = pos / 8;
        let d_bit = 7 - (pos % 8);
        let val = ((ue_val >> i) & 1) as u8;
        dst[d_byte] = (dst[d_byte] & !(1 << d_bit)) | (val << d_bit);
        pos += 1;
    }
    true
}

// ---------------------------------------------------------------------------
// H.264 SPS parsing (minimal, for extracting width/height)
// ---------------------------------------------------------------------------

/// Convert raw AVCC data to Annex B format (prepend start codes).
/// Simple version for the PSMF path — prepends SPS/PPS on keyframes.
fn avcc_to_annex_b_simple(
    avcc_data: &[u8],
    prefix_size: u8,
    sps: Option<&[u8]>,
    pps: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = Vec::new();

    // Prepend SPS/PPS with start codes if available.
    if let Some(sps) = sps {
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(sps);
    }
    if let Some(pps) = pps {
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(pps);
    }

    // Convert AVCC length-prefixed NALs to Annex B start-coded NALs.
    let ps = prefix_size as usize;
    let mut pos = 0;
    while pos + ps <= avcc_data.len() {
        let mut nal_len = 0u32;
        for i in 0..ps {
            nal_len = (nal_len << 8) | avcc_data[pos + i] as u32;
        }
        pos += ps;
        let end = pos + nal_len as usize;
        if end > avcc_data.len() {
            break;
        }
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&avcc_data[pos..end]);
        pos = end;
    }

    out
}

/// Parsed SPS info.
struct SpsInfo {
    width: u32,
    height: u32,
    max_ref_frames: u32,
}

/// Simplified SPS RBSP parser for Baseline/Main profile.
///
/// Reads exp-golomb coded fields to extract pic_width_in_mbs,
/// pic_height_in_map_units, and max_num_ref_frames.
fn parse_sps_rbsp(sps: &[u8]) -> Option<(u32, u32)> {
    parse_sps_info(sps).map(|info| (info.width, info.height))
}

fn parse_sps_info(sps: &[u8]) -> Option<SpsInfo> {
    if sps.len() < 5 {
        return None;
    }

    // sps[0] = nal header (already checked)
    let profile_idc = sps[1];
    // sps[2] = constraint flags
    // sps[3] = level_idc

    let mut reader = BitReader::new(&sps[4..]);

    // seq_parameter_set_id
    let _sps_id = reader.read_ue()?;

    // High profile has additional fields
    if profile_idc == 100 || profile_idc == 110 || profile_idc == 122
        || profile_idc == 244 || profile_idc == 44 || profile_idc == 83
        || profile_idc == 86 || profile_idc == 118 || profile_idc == 128
    {
        let chroma_format_idc = reader.read_ue()?;
        if chroma_format_idc == 3 {
            reader.skip(1)?; // separate_colour_plane_flag
        }
        let _bit_depth_luma = reader.read_ue()?;
        let _bit_depth_chroma = reader.read_ue()?;
        reader.skip(1)?; // qpprime_y_zero_transform_bypass_flag
        let seq_scaling_matrix_present = reader.read_bit()?;
        if seq_scaling_matrix_present == 1 {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for i in 0..count {
                let present = reader.read_bit()?;
                if present == 1 {
                    // Skip scaling list (first 6 are 4x4=16, rest are 8x8=64)
                    let size = if i < 6 { 16 } else { 64 };
                    let mut last_scale = 8i32;
                    let mut next_scale = 8i32;
                    for _ in 0..size {
                        if next_scale != 0 {
                            let delta = reader.read_se()?;
                            next_scale = (last_scale + delta + 256) % 256;
                        }
                        last_scale = if next_scale == 0 { last_scale } else { next_scale };
                    }
                }
            }
        }
    }

    // log2_max_frame_num_minus4
    let _log2_max_frame_num = reader.read_ue()?;
    // pic_order_cnt_type
    let poc_type = reader.read_ue()?;
    if poc_type == 0 {
        let _log2_max_poc_lsb = reader.read_ue()?;
    } else if poc_type == 1 {
        reader.skip(1)?; // delta_pic_order_always_zero_flag
        let _offset_for_non_ref_pic = reader.read_se()?;
        let _offset_for_top_to_bottom = reader.read_se()?;
        let num_ref_frames_in_poc = reader.read_ue()?;
        for _ in 0..num_ref_frames_in_poc {
            let _offset = reader.read_se()?;
        }
    }

    // max_num_ref_frames
    let max_ref_frames = reader.read_ue()?;
    // gaps_in_frame_num_allowed
    reader.skip(1)?;

    // pic_width_in_mbs_minus1
    let pic_width_mbs = reader.read_ue()? + 1;
    // pic_height_in_map_units_minus1
    let pic_height_map_units = reader.read_ue()? + 1;

    let width = pic_width_mbs * 16;
    let height = pic_height_map_units * 16;

    Some(SpsInfo { width, height, max_ref_frames })
}

/// Minimal bitstream reader for exp-golomb codes in H.264 SPS.
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8, // 0-7, MSB first
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte_pos: 0, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<u32> {
        if self.byte_pos >= self.data.len() {
            return None;
        }
        let bit = ((self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1) as u32;
        self.bit_pos += 1;
        if self.bit_pos >= 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Some(bit)
    }

    fn skip(&mut self, n: u32) -> Option<()> {
        for _ in 0..n {
            self.read_bit()?;
        }
        Some(())
    }

    fn read_bits(&mut self, n: u32) -> Option<u32> {
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | self.read_bit()?;
        }
        Some(val)
    }

    /// Read unsigned exp-golomb code.
    fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zeros = 0u32;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
            if leading_zeros > 31 {
                return None; // overflow protection
            }
        }
        if leading_zeros == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Some((1 << leading_zeros) - 1 + suffix)
    }

    /// Read signed exp-golomb code.
    fn read_se(&mut self) -> Option<i32> {
        let ue = self.read_ue()?;
        let sign = if ue & 1 == 1 { 1 } else { -1 };
        Some(sign * ((ue + 1) / 2) as i32)
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
            if play_stream() {
                break;
            }
        }

        match VIDEO_CMD_QUEUE.pop() {
            Some(VideoCmd::Play { path, seek_secs }) => {
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                if play_mp4(&path, seek_secs) {
                    break;
                }
            },
            Some(VideoCmd::StreamStart) => {
                // Legacy path — prefer STREAM_REQUESTED atomic.
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                if play_stream() {
                    break;
                }
            },
            Some(VideoCmd::Stop) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                while VIDEO_STREAM_QUEUE.pop().is_some() {}
                while VIDEO_FRAME_QUEUE.pop().is_some() {}
                send_audio_cmd(AudioCmd::VideoAudioStop);
            },
            Some(VideoCmd::Shutdown) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                while VIDEO_FRAME_QUEUE.pop().is_some() {}
                break;
            },
            None => {
                // SAFETY: sceKernelDelayThread sleeps the current thread.
                unsafe { psp::sys::sceKernelDelayThread(10_000) };
            },
        }
    }
}

/// Demux an MP4 file, decode H.264 video via ME, and feed audio to the
/// audio thread.
fn play_mp4(path: &str, seek_secs: u64) -> bool {
    use oasis_video::demux_lite::Mp4Lite;

    let reader = match PspFileReader::open(path) {
        Some(r) => r,
        None => {
            psp::dprintln!("video: failed to open {path}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return false;
        },
    };

    let mut mp4 = match Mp4Lite::open(reader) {
        Ok(m) => m,
        Err(e) => {
            psp::dprintln!("video: failed to parse MP4 {path}: {e}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return false;
        },
    };

    // Seek if requested.
    if seek_secs > 0 {
        if let Err(e) = mp4.seek(seek_secs as f64) {
            psp::dprintln!("video: seek to {seek_secs}s failed: {e}");
        }
    }

    psp::dprintln!(
        "video: MP4 opened, video={}, audio={}",
        mp4.video_track_info().is_some(),
        mp4.audio_track_info().is_some(),
    );

    let mut video_count = 0u32;
    let mut audio_count = 0u32;
    let mut decode_count = 0u32;
    let mut audio_done = mp4.audio_track_info().is_none();
    let mut video_done = mp4.video_track_info().is_none();

    // Track playback start time for frame pacing.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;

    loop {
        // Check for stop command.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_FRAME_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    if matches!(cmd, VideoCmd::Shutdown) {
                        return true;
                    }
                    break;
                },
                VideoCmd::Play { .. } | VideoCmd::StreamStart => {
                    // Ignore nested Play/Stream commands during playback.
                },
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            break;
        }

        // Read audio samples and forward raw AAC to the audio thread.
        if !audio_done {
            match mp4.next_audio_sample() {
                Ok(Some(sample)) => {
                    audio_count += 1;
                    send_audio_cmd(AudioCmd::VideoAudioAac { data: sample.data });
                },
                Ok(None) => {
                    audio_done = true;
                },
                Err(oasis_video::demux_lite::LiteError::NoTrack(_)) => {
                    audio_done = true;
                },
                Err(e) => {
                    psp::dprintln!("video: audio read error: {e}");
                    audio_done = true;
                },
            }
        }

        // Skip video samples (local file decode not wired up — use play_stream for H.264).
        if !video_done {
            match mp4.next_video_sample() {
                Ok(Some(_sample)) => {
                    video_count += 1;
                },
                Ok(None) => {
                    video_done = true;
                },
                Err(oasis_video::demux_lite::LiteError::NoTrack(_)) => {
                    video_done = true;
                },
                Err(e) => {
                    psp::dprintln!("video: video read error: {e}");
                    video_done = true;
                },
            }
        }

        if audio_done && video_done {
            break;
        }
    }

    // Cleanup on all exit paths.
    psp::dprintln!(
        "video: stream ended -- {} video samples, {} decoded frames, \
         {} audio samples",
        video_count,
        decode_count,
        audio_count,
    );
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    false
}

/// Push a decoded frame to the output queue, retrying with brief sleeps.
/// Returns true if the frame was pushed, false if dropped.
fn push_frame_with_retry(frame: DecodedFrame) -> bool {
    let mut item = frame;
    for _ in 0..6 {
        match VIDEO_FRAME_QUEUE.push(item) {
            Ok(()) => return true,
            Err(ret) => {
                item = ret;
                // SAFETY: brief sleep to yield to main thread.
                unsafe { psp::sys::sceKernelDelayThread(3_000); }
            }
        }
    }
    // Last-ditch: drop oldest frame, push newer one.
    let _ = VIDEO_FRAME_QUEUE.pop();
    match VIDEO_FRAME_QUEUE.push(item) {
        Ok(()) => true,
        Err(_) => false,
    }
}

/// Streaming playback: receive pre-demuxed H.264 frames from I/O thread
/// and decode them via sceMpeg.
///
/// Returns `true` if Shutdown was received (caller should exit thread).
fn play_stream() -> bool {
    // Wait for previous decoder cleanup to complete before starting
    // a new stream. sceMpegDelete + sceMpegFinish need time for
    // kernel-side ME cleanup; starting sceMpegCreate too soon corrupts state.
    if !DECODER_READY.load(Ordering::Acquire) {
        vlog("[VIDEO] play_stream: waiting for previous decoder cleanup...");
        for _ in 0..100 {
            // 100 × 10ms = 1 second max wait
            if DECODER_READY.load(Ordering::Acquire) {
                break;
            }
            unsafe { psp::sys::sceKernelDelayThread(10_000); }
        }
        if !DECODER_READY.load(Ordering::Acquire) {
            vlog("[VIDEO] play_stream: cleanup timeout, proceeding anyway");
        } else {
            vlog("[VIDEO] play_stream: previous decoder cleanup done");
        }
    }

    vlog("[VIDEO] play_stream: starting streaming decode");
    reset_stats();
    VIDEO_STATE.store(VSTATE_WAITING_KEYFRAME, Ordering::Relaxed);

    // Drain stale commands that may have been queued during moov buffering.
    while let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
        if matches!(cmd, VideoCmd::Shutdown) {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
            vlog("[VIDEO] play_stream: shutdown during drain");
            return true;
        }
        vlog("[VIDEO] play_stream: drained stale command");
    }

    // We need the first keyframe to extract SPS and get video dimensions
    // before initializing the decoder. Wait for it.
    // Drain any stale frames from the previous stream (the I/O thread
    // may have pushed frames between drain_stream_only exiting and
    // this play_stream starting).
    while VIDEO_STREAM_QUEUE.pop().is_some() {}
    vlog("[VIDEO] play_stream: waiting for first keyframe...");
    let mut first_frame: Option<StreamFrame> = None;

    for _ in 0..3000 {
        // ~30 seconds timeout (3000 × 10ms). Needs to be long enough
        // for: old download abort (2s) + TLS handshake (5-10s) +
        // moov buffering (1-3s) + first keyframe extraction.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            if matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown) {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                send_audio_cmd(AudioCmd::VideoAudioStop);
                return matches!(cmd, VideoCmd::Shutdown);
            }
        }

        if let Some(frame) = VIDEO_STREAM_QUEUE.pop() {
            if frame.is_keyframe {
                vlog_force(&format!(
                    "[VIDEO] GOT KEYFRAME: sz={} ts={:.2}",
                    frame.data.len(), frame.timestamp_secs,
                ));
                first_frame = Some(frame);
                break;
            }
            // Skip non-keyframes before decoder init.
            vlog_force(&format!(
                "[VIDEO] skip non-kf: sz={}", frame.data.len(),
            ));
        }

        // Wait for I/O thread to signal a new frame (10ms timeout).
        wait_stream_frame(10_000);
    }

    let first_frame = match first_frame {
        Some(f) => f,
        None => {
            vlog("[VIDEO] play_stream: no keyframe received, audio-only");
            // Continue draining stream queue but don't decode.
            return drain_stream_only();
        },
    };

    // Parse SPS from the first keyframe to get video dimensions.
    let (vid_w, vid_h) = first_frame.avcc_sps.as_ref()
        .and_then(|sps| parse_sps_rbsp(sps))
        .unwrap_or((480, 272));
    VIDEO_WIDTH.store(vid_w, Ordering::Relaxed);
    VIDEO_HEIGHT.store(vid_h, Ordering::Relaxed);
    vlog(&format!(
        "[VIDEO] play_stream: SPS dimensions = {vid_w}x{vid_h}"
    ));

    // Audio-only mode: skip video decode entirely.
    if AUDIO_ONLY.load(Ordering::Relaxed) {
        vlog("[VIDEO] audio-only mode, skipping video decode");
        VIDEO_STATE.store(VSTATE_AUDIO_ONLY, Ordering::Relaxed);
        return drain_stream_only();
    }

    // ME was leaked from a previous deadlock — can't reinit until reboot.
    if ME_LEAKED.load(Ordering::Relaxed) {
        vlog("[VIDEO] ME leaked from prior deadlock, audio-only until reboot");
        VIDEO_STATE.store(VSTATE_ME_LEAKED, Ordering::Relaxed);
        return drain_stream_only();
    }

    // NAL direct path (mpeg_vsh370.prx): works for ≤480p content.
    // >480p content deadlocks sceMpegAvcDecode (mode 5 firmware bug).
    // Skip video decode entirely and use audio-only for >480p.
    let video_oversized = vid_w > 480 || vid_h > 480;
    if video_oversized {
        vlog_force(&format!(
            "[VIDEO] {vid_w}x{vid_h} is >480p — audio-only \
             (mode 5 deadlocks ME firmware)"
        ));
        VIDEO_STATE.store(VSTATE_AUDIO_ONLY, Ordering::Relaxed);
        return drain_stream_only();
    }

    let mut psmf_dec: Option<crate::psmf_decode::PsmfDecoder> = None;
    let mut nal_dec = match NalDecoder::try_init(&first_frame) {
        Ok(dec) => {
            vlog("[VIDEO] NAL decoder initialized OK");
            Some(dec)
        },
        Err(e) => {
            vlog(&format!("[VIDEO] NAL decoder failed: {e}, audio-only"));
            None
        },
    };

    // If no decoder available, go straight to audio-only drain.
    if nal_dec.is_none() && psmf_dec.is_none() {
        return drain_stream_only();
    }

    // Decoder created — mark not ready for new streams until cleanup.
    DECODER_READY.store(false, Ordering::Release);

    VIDEO_STATE.store(VSTATE_DECODING, Ordering::Relaxed);

    // Decode the first keyframe.
    let mut start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
    let mut decode_count = 0u32;
    let mut frames_processed = 0u32;
    let mut error_count = 0u32;
    let mut no_pic_count = 0u32;
    let mut wait_count = 0u32;

    // First frame decode (NAL path only — PSMF feeds via ringbuffer).
    if let Some(ref mut nal) = nal_dec {
        match nal.decode(
            &first_frame.data, first_frame.timestamp_secs,
            first_frame.nal_prefix_size, true,
        ) {
            Ok(Some(decoded)) => {
                decode_count += 1;
                push_frame_with_retry(decoded);
                vlog("[VIDEO] play_stream: first frame decoded!");
            }
            Ok(None) => { no_pic_count += 1; }
            Err(()) => { error_count += 1; }
        }
    } else if let Some(ref mut psmf) = psmf_dec {
        // Convert AVCC to Annex B for PSMF path.
        let annex_b = avcc_to_annex_b_simple(
            &first_frame.data,
            first_frame.nal_prefix_size,
            first_frame.avcc_sps.as_deref(),
            first_frame.avcc_pps.as_deref(),
        );
        if psmf.feed_and_decode(&annex_b, first_frame.timestamp_secs) {
            decode_count += 1;
            vlog("[VIDEO] play_stream: first PSMF frame decoded!");
        }
    }
    frames_processed += 1;

    loop {
        // Check for stop/shutdown commands.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VLOG_ENABLED.store(true, Ordering::Relaxed);
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    while VIDEO_FRAME_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    let is_shutdown = matches!(cmd, VideoCmd::Shutdown);
                    vlog_force(&format!(
                        "[VIDEO] STOP: proc={frames_processed} \
                         dec={decode_count} err={error_count} nopic={no_pic_count}"
                    ));
                    // Wait for I/O thread to fully stop before dropping
                    // the decoder. The I/O thread may be blocked in a
                    // network read; give it up to 2 seconds.
                    vlog_force("[VIDEO] STOP: waiting for I/O thread...");
                    for i in 0..200u32 {
                        if crate::threading::is_download_stopped() {
                            break;
                        }
                        if i % 50 == 49 {
                            vlog_force(&format!(
                                "[VIDEO] STOP: still waiting for I/O ({}ms)",
                                (i + 1) * 10
                            ));
                        }
                        unsafe { psp::sys::sceKernelDelayThread(10_000); }
                    }
                    if !crate::threading::is_download_stopped() {
                        vlog_force("[VIDEO] STOP: I/O timeout, proceeding");
                    } else {
                        vlog_force("[VIDEO] STOP: I/O stopped");
                    }
                    // Drop decoder explicitly now that I/O is quiesced.
                    vlog_force("[VIDEO] STOP: dropping nal_dec...");
                    drop(nal_dec.take());
                    vlog_force("[VIDEO] STOP: nal_dec dropped OK");
                    drop(psmf_dec.take());
                    // Brief ME cooldown after sceMpegDelete/Finish.
                    unsafe { psp::sys::sceKernelDelayThread(100_000); }
                    DECODER_READY.store(true, Ordering::Release);
                    vlog_force("[VIDEO] STOP: cleanup complete");
                    return is_shutdown;
                },
                _ => {},
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            break;
        }

        // Pop next pre-demuxed H.264 frame from stream queue.
        match VIDEO_STREAM_QUEUE.pop() {
            Some(frame) => {
                // Wait for output queue space BEFORE decoding.
                // ME decode takes ~50ms per frame. Without this, we
                // decode frames that will be dropped because the queue
                // is full, wasting ~30% of ME time.
                {
                    let mut waits = 0u32;
                    while VIDEO_FRAME_QUEUE.len()
                        >= VIDEO_FRAME_QUEUE.capacity() as u32
                    {
                        unsafe { psp::sys::sceKernelDelayThread(3_000); }
                        waits += 1;
                        if waits > 20 { break; } // 60ms max wait
                    }
                }


                // PSMF path: convert to Annex B and feed to ringbuffer.
                let decode_result = if let Some(ref mut psmf) = psmf_dec {
                    let annex_b = avcc_to_annex_b_simple(
                        &frame.data,
                        frame.nal_prefix_size,
                        if frame.is_keyframe { frame.avcc_sps.as_deref() } else { None },
                        if frame.is_keyframe { frame.avcc_pps.as_deref() } else { None },
                    );
                    if psmf.feed_and_decode(&annex_b, frame.timestamp_secs) {
                        Ok(Some(DecodedFrame {
                            buf_idx: 0, // PSMF uses its own buffer
                            width: psmf.width,
                            height: psmf.height,
                            stride: psmf.width,
                        }))
                    } else {
                        Ok(None)
                    }
                } else if let Some(ref mut nal) = nal_dec {
                    nal.decode(
                        &frame.data, frame.timestamp_secs,
                        frame.nal_prefix_size, frame.is_keyframe,
                    )
                } else {
                    Err(())
                };

                match decode_result {
                    Ok(Some(decoded)) => {
                        decode_count += 1;
                        VIDEO_DECODE_COUNT.store(decode_count, Ordering::Relaxed);

                        // Suppress verbose logging after first frame.
                        if decode_count == 1 {
                            VLOG_ENABLED.store(false, Ordering::Relaxed);
                        }

                        // Frame pacing via PTS with drift correction.
                        let pts_us =
                            (frame.timestamp_secs * 1_000_000.0) as u64;
                        let now_us = unsafe {
                            psp::sys::sceKernelGetSystemTimeWide()
                        } as u64;
                        let elapsed = now_us.wrapping_sub(start_us);
                        if pts_us > elapsed {
                            let wait = (pts_us - elapsed) as u32;
                            if wait < 100_000 {
                                // SAFETY: Sleep for frame pacing.
                                unsafe {
                                    psp::sys::sceKernelDelayThread(wait);
                                }
                            }
                        } else if elapsed > pts_us + 200_000 {
                            // Decoder >200ms behind — reset baseline to
                            // prevent permanent fast-forward.
                            start_us = now_us.wrapping_sub(pts_us);
                        }

                        // Queue space was checked before decode. Push
                        // should succeed; if not, drop gracefully.
                        if VIDEO_FRAME_QUEUE.push(decoded).is_ok() {
                            VIDEO_FRAMES_PUSHED.fetch_add(
                                1, Ordering::Relaxed,
                            );
                        } else {
                            VIDEO_FRAMES_DROPPED.fetch_add(
                                1, Ordering::Relaxed,
                            );
                        }
                    }
                    Ok(None) => {
                        no_pic_count += 1;
                        VIDEO_NOPIC_COUNT.store(no_pic_count, Ordering::Relaxed);
                    }
                    Err(()) => {
                        error_count += 1;
                        VIDEO_ERROR_COUNT.store(error_count, Ordering::Relaxed);
                        // If the watchdog unblocked a stuck decode, the ME
                        // is in a bad state. Switch to audio-only immediately.
                        if error_count >= 3 && nal_dec.is_some() {
                            vlog_force(&format!(
                                "[VIDEO] {} consecutive errors, ME likely unblocked \
                                 by watchdog. Switching to audio-only \
                                 (dec={decode_count} total={frames_processed})",
                                error_count,
                            ));
                            // LEAK the decoder — do NOT call sceMpegDelete/Finish
                            // while the ME is in a bad state (causes crash).
                            // The ~2MB DDR allocation and mpeg data are lost until
                            // the next cold reboot, but the EBOOT stays alive.
                            if let Some(dec) = nal_dec.take() {
                                core::mem::forget(dec);
                                ME_LEAKED.store(true, Ordering::Release);
                            }
                            VIDEO_STATE.store(VSTATE_ME_LEAKED, Ordering::Relaxed);
                            // ME is leaked — no cleanup needed, mark ready.
                            DECODER_READY.store(true, Ordering::Release);
                            return drain_stream_only();
                        }
                    }
                }
                frames_processed += 1;
                VIDEO_FRAMES_PROCESSED.store(
                    frames_processed, Ordering::Relaxed,
                );

                // Periodic diagnostic (unconditional) — every 50 frames
                // to reduce ~5-20ms per-write I/O overhead.
                if frames_processed % 50 == 0 {
                    vlog_force(&format!(
                        "[VIDEO] #{frames_processed}: dec={decode_count} \
                         err={error_count} nopic={no_pic_count}"
                    ));
                }
            },
            None => {
                wait_count += 1;
                // Log when stuck waiting (every 30 waits ≈ 1 second).
                if wait_count % 30 == 0 {
                    let step = psp::mpeg::DECODE_STEP.load(
                        core::sync::atomic::Ordering::Relaxed,
                    );
                    vlog_force(&format!(
                        "[VIDEO] waiting: w={wait_count} proc={frames_processed} \
                         playing={} step={step}",
                        VIDEO_PLAYING.load(Ordering::Relaxed),
                    ));
                }
                // No frame available — wait for I/O thread signal
                // (33ms timeout matches ~30fps frame interval).
                wait_stream_frame(33_000);
            },
        }
    }

    VLOG_ENABLED.store(true, Ordering::Relaxed);
    VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
    vlog(&format!(
        "[VIDEO] play_stream ended: proc={frames_processed} \
         dec={decode_count} err={error_count} nopic={no_pic_count}"
    ));
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    // Wait for I/O thread to stop before dropping decoders,
    // consistent with the Stop-command path.
    for _i in 0..200u32 {
        if crate::threading::is_download_stopped() {
            break;
        }
        unsafe { psp::sys::sceKernelDelayThread(10_000); }
    }
    // Drop decoders explicitly, then give ME time to clean up.
    drop(nal_dec);
    drop(psmf_dec);
    unsafe { psp::sys::sceKernelDelayThread(50_000); }
    DECODER_READY.store(true, Ordering::Release);
    vlog("[VIDEO] decoder cleanup done");
    false
}

/// Drain the stream queue without decoding (audio-only fallback).
/// Keeps the thread alive to handle Stop/Shutdown commands.
fn drain_stream_only() -> bool {
    loop {
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    while VIDEO_FRAME_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    return matches!(cmd, VideoCmd::Shutdown);
                },
                _ => {},
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            return false;
        }

        // If a new stream was requested, exit drain so the main loop
        // can pick up STREAM_REQUESTED and start a new play_stream().
        if STREAM_REQUESTED.load(Ordering::Relaxed) {
            return false;
        }

        // Drain frames to prevent queue backup.
        while VIDEO_STREAM_QUEUE.pop().is_some() {}

        // SAFETY: sceKernelDelayThread sleeps the current thread.
        unsafe { psp::sys::sceKernelDelayThread(50_000) };
    }
}
