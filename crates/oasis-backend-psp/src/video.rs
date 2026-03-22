//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once -- PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video H.264 frames are decoded via `sceMpeg` (Media Engine) on real PSP
//! hardware, with VFPU-accelerated YUV420->RGBA color conversion.
//!
//! The sceMpeg API is used instead of the lower-level sceVideocodec because
//! sceVideocodec weak imports fail to resolve on many CFW configurations
//! (error 0x806201fe), while sceMpeg is universally available.

use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;
use crate::threading::{AudioCmd, send_audio_cmd};

/// File-based debug logging (works from video thread, unlike psp::dprintln).
fn vlog(msg: &str) {
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
    pub data: Vec<u8>,
    pub timestamp_secs: f64,
    pub is_keyframe: bool,
}

/// A decoded video frame ready for texture upload.
///
/// Identical to `oasis_video::h264::DecodedFrame` but defined separately
/// because the PSP backend is excluded from the workspace (different target
/// architecture) and cannot depend on oasis-video's h264 module.
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send a command to the video decode thread.
pub fn send_video_cmd(cmd: VideoCmd) {
    let _ = VIDEO_CMD_QUEUE.push(cmd);
}

/// Poll for the next decoded video frame (non-blocking).
pub fn poll_video_frame() -> Option<DecodedFrame> {
    VIDEO_FRAME_QUEUE.pop()
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
    VIDEO_STREAM_QUEUE.push(frame)
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
// H.264 video decoder (sceVideocodec)
// ---------------------------------------------------------------------------
//
// Uses sceVideocodec for direct H.264 decode via the Media Engine.
// The weak import stubs (flags 0x4009) are resolved after loading AV modules
// via sceUtilityLoadModule. The library version was fixed to (0x00, 0x00)
// in the rust-psp SDK to match what avcodec.prx exports on real firmware
// (the previous version 0x11 caused stub resolution to fail with 0x806201fe).

use core::ffi::c_void;

/// 64-byte-aligned codec buffer required by the PSP Media Engine.
#[repr(align(64))]
struct CodecBuf([u32; 96]);

/// Bootstrap the Media Engine by creating a temporary sceMpeg context.
///
/// Ghidra analysis of avcodec.prx reveals its ME submission functions are
/// empty stubs. The real implementations are loaded by mpeg.prx, which
/// patches avcodec.prx when `sceMpegCreate` initializes the ME firmware.
/// We create and delete a minimal MPEG context to trigger this patching.
#[inline(never)]
fn bootstrap_me() {
    use core::ffi::c_void;

    // SAFETY: sceMpegInit is idempotent.
    let ret = unsafe { psp::sys::sceMpegInit() };
    vlog(&format!("[VIDEO] sceMpegInit = {ret:#x}"));

    vlog("[VIDEO] bootstrap: querying mem size...");
    // Query required memory size for MPEG context.
    let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(0) };
    if mem_size <= 0 {
        vlog(&format!("[VIDEO] sceMpegQueryMemSize = {mem_size} (skip)"));
        return;
    }

    // Allocate ringbuffer (minimum viable: 8 packets).
    let rb_size = unsafe { psp::sys::sceMpegRingbufferQueryMemSize(8) };
    if rb_size <= 0 {
        vlog(&format!("[VIDEO] RingbufferQueryMemSize = {rb_size} (skip)"));
        return;
    }

    // Allocate aligned memory for MPEG context.
    let mut mpeg_data = vec![0u8; mem_size as usize + 64];
    let mpeg_data_aligned = {
        let p = mpeg_data.as_mut_ptr();
        let off = p.align_offset(64);
        unsafe { p.add(off) }
    };

    // Allocate ringbuffer data.
    let mut rb_data = vec![0u8; rb_size as usize];

    // Construct ringbuffer (no callback needed — we won't feed data).
    let mut ringbuffer = unsafe {
        core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
    };
    let ret = unsafe {
        psp::sys::sceMpegRingbufferConstruct(
            &mut ringbuffer,
            8,
            rb_data.as_mut_ptr() as *mut c_void,
            rb_size,
            None, // no callback
            core::ptr::null_mut(),
        )
    };
    if ret < 0 {
        vlog(&format!("[VIDEO] RingbufferConstruct = {ret:#x} (skip)"));
        return;
    }

    // Create MPEG handle — this loads ME firmware and patches stubs.
    // SceMpeg is *mut *mut c_void — must point to valid heap storage.
    let mut mpeg_storage: *mut c_void = core::ptr::null_mut();
    let mpeg: psp::sys::SceMpeg = unsafe {
        core::mem::transmute(&mut mpeg_storage as *mut *mut c_void)
    };
    let ret = unsafe {
        psp::sys::sceMpegCreate(
            mpeg,
            mpeg_data_aligned as *mut c_void,
            mem_size as i32,
            &mut ringbuffer,
            512, // frame width
            0,
            0,
        )
    };
    vlog(&format!("[VIDEO] sceMpegCreate = {ret:#x}"));

    if ret >= 0 {
        // Delete context — we only needed the side effect of ME loading.
        unsafe { psp::sys::sceMpegDelete(mpeg) };
        vlog("[VIDEO] sceMpegDelete OK (ME bootstrapped)");
    }

    unsafe { psp::sys::sceMpegRingbufferDestruct(&mut ringbuffer) };
    // Don't call sceMpegFinish — keep ME subsystem alive for sceVideocodec.
}

/// Pre-initialize codec buffer scratch pointers for sceVideocodecOpen.
#[inline(never)]
fn init_codec_scratch(p: *mut u32) {
    // Allocate 64-byte aligned scratch for ME DMA compatibility.
    #[repr(align(64))]
    struct Scratch([u8; 256]);
    let s = Box::new(Scratch([0u8; 256]));
    let a = Box::into_raw(s) as u32;
    unsafe { *p.add(4) = a; *p.add(21) = a; }
}

/// 64-byte aligned AU buffer for ME DMA compatibility.
/// sceVideocodecDecode validates buf[9] (AU pointer) via k1 and
/// requires aligned memory. 128KB covers typical TV Guide H.264 AUs.
#[repr(align(64))]
struct AuBuf([u8; 128 * 1024]);

/// PSP hardware H.264 video decoder using sceVideocodec (Media Engine).
struct PspVideoDecoder {
    buf: Box<CodecBuf>,
    au_buf: Box<AuBuf>,
    initialized: bool,
    width: u32,
    height: u32,
}

impl PspVideoDecoder {
    /// Attempt to initialize the H.264 hardware decoder.
    ///
    /// Loads AV codec modules first (idempotent), then calls sceVideocodec.
    /// Returns `Err` on PPSSPP (ME not emulated) or if the codec modules
    /// are unavailable. The caller should fall back to audio-only mode.
    fn try_init() -> Result<Self, String> {
        vlog("[VIDEO] try_init: loading AV modules...");
        crate::audio::load_av_modules_once_pub();
        vlog("[VIDEO] try_init: AV modules loaded");

        // Initialize MPEG subsystem and create a temporary MPEG context.
        // Ghidra RE of avcodec.prx revealed that the ME submission functions
        // are EMPTY STUBS — the real codec implementation is in mpeg.prx,
        // which patches avcodec.prx's stubs when sceMpegCreate loads the
        // ME firmware binary. We create and immediately delete an MPEG
        // context just to trigger this patching.
        bootstrap_me();

        let mut buf = Box::new(CodecBuf([0u32; 96]));
        let ptr = buf.0.as_mut_ptr();

        // Pre-initialize codec buffer fields required by sceVideocodecOpen.
        // Ghidra analysis: sceVideocodecSetMemory validates buf[4] and
        // buf[21] as pointers to scratch memory.
        init_codec_scratch(ptr);

        vlog("[VIDEO] try_init: calling sceVideocodecOpen...");
        // SAFETY: sceVideocodecOpen with 64-byte aligned buffer, type 0 = AVC.
        let ret = unsafe { psp::sys::sceVideocodecOpen(ptr, 0) };
        if ret < 0 {
            let msg = format!(
                "sceVideocodecOpen failed: {:#010x}",
                ret as u32
            );
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] sceVideocodecOpen OK");

        // SAFETY: ptr is the same aligned buffer passed to Open.
        let ret = unsafe { psp::sys::sceVideocodecGetEDRAM(ptr, 0) };
        if ret < 0 {
            let msg = format!(
                "sceVideocodecGetEDRAM failed: {:#010x}",
                ret as u32
            );
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] sceVideocodecGetEDRAM OK");

        // SAFETY: ptr is the same aligned buffer.
        let ret = unsafe { psp::sys::sceVideocodecInit(ptr, 0) };
        if ret < 0 {
            unsafe { psp::sys::sceVideocodecReleaseEDRAM(ptr) };
            let msg = format!(
                "sceVideocodecInit failed: {:#010x}",
                ret as u32
            );
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] sceVideocodecInit OK -- decoder ready");

        Ok(Self {
            buf,
            au_buf: Box::new(AuBuf([0u8; 128 * 1024])),
            initialized: true,
            width: 0,
            height: 0,
        })
    }

    /// Decode a single H.264 access unit (Annex B format).
    fn decode(&mut self, au_data: &[u8]) -> Option<DecodedFrame> {
        if !self.initialized || au_data.is_empty() {
            return None;
        }

        let ptr = self.buf.0.as_mut_ptr();

        // Copy AU data into aligned buffer for ME DMA compatibility.
        let au_len = au_data.len();
        if au_len > self.au_buf.0.len() {
            return None; // AU too large
        }
        self.au_buf.0[..au_len].copy_from_slice(au_data);
        let au_ptr = self.au_buf.0.as_ptr() as u32;

        // Set AU pointer and size in codec buffer.
        // Also write the AU pointer into the scratch metadata at buf[4]+0
        // and AU size at buf[4]+4, in case the ME reads from scratch.
        unsafe {
            // Standard AU pointer fields
            *ptr.add(9) = au_ptr;
            *ptr.add(10) = au_len as u32;
            // Also set buf[3] area (offset 0x0C) with AU pointer — some
            // PSP homebrew documentation puts AU info here instead of buf[9]
            // Write AU pointer into scratch metadata structure
            let scratch = *ptr.add(4) as *mut u32;
            if !scratch.is_null() {
                *scratch = au_ptr;             // scratch[0] = AU pointer
                *scratch.add(1) = au_len as u32; // scratch[1] = AU size
            }
        }

        // SAFETY: Flush codec buffer + AU data for ME DMA coherency.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                ptr as *const c_void,
                core::mem::size_of::<CodecBuf>() as u32,
            );
            psp::sys::sceKernelDcacheWritebackRange(
                au_data.as_ptr() as *const c_void,
                au_data.len() as u32,
            );
        }

        // Scan the AU header first — this submits data to the ME and must
        // be called before Decode. Ghidra shows ScanHeader flushes buf[4]
        // metadata, submits to ME via FUN_00004424, and Decode reads the
        // result via a jump table.
        let scan_ret = unsafe { psp::sys::sceVideocodecScanHeader(ptr, 0) };

        // SAFETY: sceVideocodecDecode reads the ME result.
        let ret = unsafe { psp::sys::sceVideocodecDecode(ptr, 0) };
        if ret < 0 || scan_ret < 0 {
            static ERR_COUNT: core::sync::atomic::AtomicU32 =
                core::sync::atomic::AtomicU32::new(0);
            let c = ERR_COUNT.fetch_add(1, Ordering::Relaxed);
            if c < 5 {
                vlog(&format!(
                    "[VIDEO] scan={:#010x} decode={:#010x}",
                    scan_ret as u32, ret as u32,
                ));
            }
            return None;
        }

        // SAFETY: Read decoded frame info from codec buffer.
        let (width, height, y_ptr, y_stride, cb_ptr, cr_ptr) = unsafe {
            (
                *ptr.add(32),
                *ptr.add(33),
                *ptr.add(38),
                *ptr.add(39),
                *ptr.add(40),
                *ptr.add(41),
            )
        };

        if width == 0 || height == 0 {
            return None;
        }

        if self.width == 0 {
            self.width = width;
            self.height = height;
            vlog(&format!(
                "[VIDEO] first frame: {width}x{height}, \
                 Y={y_ptr:#010x} stride={y_stride} \
                 Cb={cb_ptr:#010x} Cr={cr_ptr:#010x}"
            ));
        }

        // Validate EDRAM range.
        if y_ptr < 0x0400_0000 || y_ptr >= 0x0420_0000 {
            return None;
        }

        // SAFETY: EDRAM pointers via uncached address for DMA coherency.
        let frame = unsafe {
            yuv420_to_rgba_vfpu(
                (y_ptr | 0x4000_0000) as *const u8,
                y_stride as usize,
                (cb_ptr | 0x4000_0000) as *const u8,
                (cr_ptr | 0x4000_0000) as *const u8,
                width,
                height,
            )
        };

        Some(frame)
    }
}

impl Drop for PspVideoDecoder {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                psp::sys::sceVideocodecReleaseEDRAM(self.buf.0.as_mut_ptr());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VFPU-accelerated YUV420P -> RGBA conversion
// ---------------------------------------------------------------------------
//
// BT.601 standard conversion (used by most H.264 content):
//   R = 1.164 * (Y - 16) + 1.596 * (Cr - 128)
//   G = 1.164 * (Y - 16) - 0.392 * (Cb - 128) - 0.813 * (Cr - 128)
//   B = 1.164 * (Y - 16) + 2.017 * (Cb - 128)
//   A = 255
//
// The VFPU processes this as a 4x4 matrix-vector multiply per pixel:
//   [R]   [1.164   0.0   1.596  0.0] [Y  - 16 ]
//   [G] = [1.164  -0.392 -0.813 0.0] [Cb - 128]
//   [B]   [1.164   2.017  0.0   0.0] [Cr - 128]
//   [A]   [0.0     0.0    0.0   1.0] [255     ]
//
// The conversion matrix is loaded into VFPU M000 once, then `vtfm4.q`
// computes all 4 output channels in a single instruction (~4 cycles).
// `vmax`/`vmin` clamp to [0, 255], and `vf2iz` converts to integer.
//
// At ~20 VFPU cycles per pixel, a 480x272 frame converts in ~2.6M cycles
// = ~8ms at 333MHz. This is well within the 33ms budget for 30fps video.

/// Convert a YUV420P frame to RGBA using BT.601 integer fixed-point math.
///
/// Uses scalar integer arithmetic instead of VFPU inline asm to avoid
/// LLVM MIPS register allocation issues with `vfpu_asm!` that trigger
/// "expected relocatable expression" errors on nightly.
///
/// # Safety
///
/// - `y_ptr`, `cb_ptr`, `cr_ptr` must point to valid YUV420P plane data.
/// - Pointers should use uncached addresses for EDRAM data.
#[inline(never)]
unsafe fn yuv420_to_rgba_vfpu(
    y_ptr: *const u8,
    y_stride: usize,
    cb_ptr: *const u8,
    cr_ptr: *const u8,
    width: u32,
    height: u32,
) -> DecodedFrame {
    let w = width as usize;
    let h = height as usize;
    let chroma_stride = y_stride / 2;
    let mut rgba = vec![0u8; w * h * 4];

    for row in 0..h {
        let chroma_row = row / 2;
        for col in 0..w {
            let chroma_col = col / 2;
            let y_val = unsafe { *y_ptr.add(row * y_stride + col) } as u32;
            let cb_val =
                unsafe { *cb_ptr.add(chroma_row * chroma_stride + chroma_col) } as u32;
            let cr_val =
                unsafe { *cr_ptr.add(chroma_row * chroma_stride + chroma_col) } as u32;
            // BT.601 YUV→RGB conversion (integer fixed-point).
            // Uses 16.16 fixed point to avoid VFPU inline asm which
            // triggers LLVM MIPS register allocation issues.
            let y_adj = y_val as i32 - 16;
            let cb_adj = cb_val as i32 - 128;
            let cr_adj = cr_val as i32 - 128;

            // Fixed-point coefficients (×256):
            // R = 1.164*Y + 1.596*Cr = (298*Y + 409*Cr) >> 8
            // G = 1.164*Y - 0.392*Cb - 0.813*Cr = (298*Y - 100*Cb - 208*Cr) >> 8
            // B = 1.164*Y + 2.017*Cb = (298*Y + 516*Cb) >> 8
            let r = ((298 * y_adj + 409 * cr_adj + 128) >> 8).clamp(0, 255);
            let g = ((298 * y_adj - 100 * cb_adj - 208 * cr_adj + 128) >> 8)
                .clamp(0, 255);
            let b = ((298 * y_adj + 516 * cb_adj + 128) >> 8).clamp(0, 255);

            let pix = (row * w + col) * 4;
            rgba[pix] = r as u8;
            rgba[pix + 1] = g as u8;
            rgba[pix + 2] = b as u8;
            rgba[pix + 3] = 255;
        }
    }

    DecodedFrame {
        rgba,
        width,
        height,
    }
}

// ---------------------------------------------------------------------------
// Thread function
// ---------------------------------------------------------------------------

fn video_thread_fn() {
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
                send_audio_cmd(AudioCmd::VideoAudioStop);
            },
            Some(VideoCmd::Shutdown) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
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

    // Attempt H.264 hardware decoder init.
    vlog("[VIDEO] play_mp4: attempting H.264 init...");
    let mut h264 = match PspVideoDecoder::try_init() {
        Ok(dec) => {
            vlog("[VIDEO] play_mp4: H.264 hardware decoder initialized");
            Some(dec)
        },
        Err(e) => {
            vlog(&format!(
                "[VIDEO] play_mp4: H.264 disabled ({e}), audio-only"
            ));
            None
        },
    };

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
    // Use u64 (sceKernelGetSystemTimeWide) to avoid overflow at ~71 minutes.
    // SAFETY: sceKernelGetSystemTimeWide is a read-only kernel syscall that
    // returns the 64-bit system timer in microseconds. No preconditions.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;

    loop {
        // Check for stop command.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
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

        // Read and decode video samples.
        if !video_done {
            match mp4.next_video_sample() {
                Ok(Some(sample)) => {
                    video_count += 1;

                    // Decode H.264 via Media Engine if available.
                    if let Some(ref mut decoder) = h264 {
                        if let Some(frame) = decoder.decode(&sample.data) {
                            decode_count += 1;

                            // Frame pacing: wait until the frame's PTS.
                            // This prevents dumping frames faster than
                            // the display can consume them.
                            let pts_us = (sample.timestamp_secs * 1_000_000.0) as u64;
                            // SAFETY: Read-only kernel syscall returning 64-bit system timer.
                            let now_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
                            let elapsed = now_us.wrapping_sub(start_us);
                            if pts_us > elapsed {
                                let wait = (pts_us - elapsed) as u32;
                                // Cap wait to 100ms to avoid stalls.
                                if wait < 100_000 {
                                    // SAFETY: Sleep for frame pacing.
                                    unsafe {
                                        psp::sys::sceKernelDelayThread(wait);
                                    }
                                }
                            }

                            // Push to frame queue (drops if full).
                            let _ = VIDEO_FRAME_QUEUE.push(frame);
                        }
                    }
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

/// Streaming playback: receive pre-demuxed H.264 frames from I/O thread
/// and decode them via the Media Engine.
///
/// Returns `true` if Shutdown was received (caller should exit thread).
fn play_stream() -> bool {
    vlog("[VIDEO] play_stream: starting streaming decode");

    // Drain stale commands that may have been queued during moov buffering
    // (e.g., user pressed Cancel while I/O thread was still downloading).
    // Without this, a stale Stop command would immediately exit the loop.
    while let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
        if matches!(cmd, VideoCmd::Shutdown) {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            vlog("[VIDEO] play_stream: shutdown during drain");
            return true;
        }
        vlog("[VIDEO] play_stream: drained stale command");
    }

    // Attempt H.264 hardware decoder init for streaming mode.
    // On real PSP hardware the ME is available and frames are decoded.
    // On PPSSPP the ME is not emulated — try_init() returns Err and
    // playback continues as audio-only (the I/O thread still pushes
    // video samples, but they are drained without decode).
    vlog("[VIDEO] play_stream: attempting H.264 init...");
    let mut h264 = match PspVideoDecoder::try_init() {
        Ok(dec) => {
            vlog("[VIDEO] play_stream: H.264 hardware decoder initialized");
            Some(dec)
        },
        Err(e) => {
            vlog(&format!(
                "[VIDEO] play_stream: H.264 disabled ({e}), audio-only"
            ));
            None
        },
    };

    // Use u64 (sceKernelGetSystemTimeWide) to avoid overflow at ~71 minutes.
    // SAFETY: Read-only kernel syscall returning 64-bit system timer.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
    let mut decode_count = 0u32;

    loop {
        // Check for stop/shutdown commands.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    if matches!(cmd, VideoCmd::Shutdown) {
                        return true;
                    }
                    vlog(&format!(
                        "[VIDEO] play_stream stopped, {decode_count} frames decoded"
                    ));
                    return false;
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
                if let Some(ref mut decoder) = h264 {
                    if let Some(decoded) = decoder.decode(&frame.data) {
                        decode_count += 1;

                        // Frame pacing via PTS.
                        let pts_us = (frame.timestamp_secs * 1_000_000.0) as u64;
                        // SAFETY: Read-only kernel syscall returning 64-bit system timer.
                        let now_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
                        let elapsed = now_us.wrapping_sub(start_us);
                        if pts_us > elapsed {
                            let wait = (pts_us - elapsed) as u32;
                            if wait < 100_000 {
                                // SAFETY: Sleep for frame pacing.
                                unsafe {
                                    psp::sys::sceKernelDelayThread(wait);
                                }
                            }
                        }

                        let _ = VIDEO_FRAME_QUEUE.push(decoded);
                    }
                }
            },
            None => {
                // No frame available yet, sleep briefly.
                // SAFETY: sceKernelDelayThread sleeps the current thread.
                unsafe { psp::sys::sceKernelDelayThread(5_000) };
            },
        }
    }

    vlog(&format!(
        "[VIDEO] play_stream ended, {decode_count} frames decoded"
    ));
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    false
}
