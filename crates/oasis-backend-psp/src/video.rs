//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once -- PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video H.264 frames are decoded via `sceVideocodec` (Media Engine) on real
//! PSP hardware, with VFPU-accelerated YUV420->RGBA color conversion.

use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;
use psp::vfpu_asm;

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
// sceVideocodec is declared with weak import flags (0x4009) in the rust-psp
// SDK, so the EBOOT module loads even when codec libraries aren't present at
// boot. Before first use, we call `load_av_modules_once()` which triggers
// `sceUtilityLoadModule(AvCodec)` + `sceUtilityLoadModule(AvMpegBase)`.
// The kernel then re-links the weak stubs with the real syscall entries.

/// 64-byte-aligned codec buffer required by the PSP Media Engine.
///
/// `sceVideocodec*` APIs require a 64-byte-aligned buffer of at least
/// 96 `u32` words. `#[repr(align(64))]` lets the Rust compiler guarantee
/// alignment without manual pointer arithmetic.
#[repr(align(64))]
struct CodecBuf([u32; 96]);

/// PSP hardware H.264 video decoder using the Media Engine.
///
/// Uses `sceVideocodec*` APIs after lazy-loading the AV modules.
/// The ME is not emulated in PPSSPP, so `try_init()` returns `Err`
/// when running under emulation (falls back to audio-only mode).
struct PspVideoDecoder {
    /// Codec buffer (96 words, 64-byte aligned via `CodecBuf`).
    buf: Box<CodecBuf>,
    initialized: bool,
    /// Cached video dimensions from first successful decode.
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
        // Ensure AV modules are loaded so the weak sceVideocodec stubs
        // get re-linked by the kernel to the real syscall entries.
        crate::audio::load_av_modules_once_pub();
        vlog("[VIDEO] try_init: AV modules loaded");

        let mut buf = Box::new(CodecBuf([0u32; 96]));
        let ptr = buf.0.as_mut_ptr();

        vlog("[VIDEO] try_init: calling sceVideocodecOpen...");
        // SAFETY: sceVideocodecOpen initializes the codec buffer.
        // Type 0 = H.264 / AVC. ptr is 64-byte aligned via CodecBuf.
        let ret = unsafe { psp::sys::sceVideocodecOpen(ptr, 0) };
        if ret < 0 {
            let msg = format!(
                "sceVideocodecOpen failed: {:#010x} (ME not available?)",
                ret as u32
            );
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] try_init: sceVideocodecOpen OK");

        vlog("[VIDEO] try_init: calling sceVideocodecGetEDRAM...");
        // SAFETY: ptr is the same aligned buffer passed to Open.
        let ret = unsafe { psp::sys::sceVideocodecGetEDRAM(ptr, 0) };
        if ret < 0 {
            // Note: sceVideocodecOpen has no corresponding Close/Stop API, so
            // the codec handle from Open cannot be explicitly released. Only
            // EDRAM (from GetEDRAM) has a release function.
            let msg = format!("sceVideocodecGetEDRAM failed: {:#010x}", ret as u32);
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] try_init: sceVideocodecGetEDRAM OK");

        vlog("[VIDEO] try_init: calling sceVideocodecInit...");
        // SAFETY: ptr is the same aligned buffer passed to Open/GetEDRAM.
        let ret = unsafe { psp::sys::sceVideocodecInit(ptr, 0) };
        if ret < 0 {
            // SAFETY: Release EDRAM on init failure.
            unsafe { psp::sys::sceVideocodecReleaseEDRAM(ptr) };
            let msg = format!("sceVideocodecInit failed: {:#010x}", ret as u32);
            vlog(&format!("[VIDEO] {msg}"));
            return Err(msg);
        }
        vlog("[VIDEO] try_init: sceVideocodecInit OK -- decoder ready");

        Ok(Self {
            buf,
            initialized: true,
            width: 0,
            height: 0,
        })
    }

    /// Decode a single H.264 access unit (Annex B format).
    ///
    /// The AU data should contain NAL units separated by Annex B start codes
    /// (00 00 00 01). On keyframes, SPS + PPS NALs are prepended by demux_lite.
    ///
    /// Returns the decoded RGBA frame on success, or `None` if the codec
    /// needs more data (buffering reference frames) or decode fails.
    fn decode(&mut self, au_data: &[u8]) -> Option<DecodedFrame> {
        if !self.initialized || au_data.is_empty() {
            return None;
        }

        let ptr = self.buf.0.as_mut_ptr();

        // SAFETY: Set source AU pointer and size in the codec buffer.
        // buf[9] = pointer to H.264 access unit data (Annex B)
        // buf[10] = size of AU data in bytes
        // These offsets are standard for sceVideocodec type 0 (H.264).
        unsafe {
            *ptr.add(9) = au_data.as_ptr() as u32;
            *ptr.add(10) = au_data.len() as u32;
        }

        // SAFETY: The Media Engine reads AU data via DMA, which bypasses
        // the CPU data cache. Flush only the relevant ranges so the ME
        // sees the latest writes without thrashing the entire 16KB D-cache.
        unsafe {
            // Flush the codec buffer (384 bytes = 96 u32 words).
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                ptr as *const core::ffi::c_void,
                core::mem::size_of::<CodecBuf>() as u32,
            );
            // Flush the H.264 access unit data.
            psp::sys::sceKernelDcacheWritebackRange(
                au_data.as_ptr() as *const core::ffi::c_void,
                au_data.len() as u32,
            );
        }

        // SAFETY: sceVideocodecDecode processes one AU through the Media
        // Engine. The codec buffer and AU data must remain valid.
        let ret = unsafe { psp::sys::sceVideocodecDecode(ptr, 0) };
        if ret < 0 {
            vlog(&format!(
                "[VIDEO] decode: sceVideocodecDecode failed: {:#010x}",
                ret as u32
            ));
            return None;
        }

        // Read decoded frame info from codec buffer.
        //
        // After a successful decode, the ME writes YCbCr 4:2:0 planar data
        // to EDRAM and populates these codec buffer fields:
        //   buf[32] = decoded picture width
        //   buf[33] = decoded picture height
        //   buf[38] = Y plane pointer (in EDRAM)
        //   buf[39] = Y plane stride (bytes per row)
        //   buf[40] = Cb plane pointer
        //   buf[41] = Cr plane pointer
        //
        // NOTE: These offsets are based on PSP homebrew documentation (PMP
        // Mod, cooleyes MP4 player). If decode succeeds but dimensions are
        // zero, we log the full buffer for diagnosis on real hardware.
        let width;
        let height;
        let y_ptr;
        let y_stride;
        let cb_ptr;
        let cr_ptr;

        // SAFETY: Reading codec buffer fields populated by sceVideocodecDecode.
        unsafe {
            width = *ptr.add(32);
            height = *ptr.add(33);
            y_ptr = *ptr.add(38);
            y_stride = *ptr.add(39);
            cb_ptr = *ptr.add(40);
            cr_ptr = *ptr.add(41);
        }

        // No output frame yet (codec buffering reference frames).
        if width == 0 || height == 0 {
            // On first decode, log the buffer to help diagnose offset issues.
            if self.width == 0 {
                psp::dprintln!("video: no frame yet, probing buffer...");
                self.log_codec_buffer();
            }
            return None;
        }

        // Cache dimensions on first successful decode.
        if self.width == 0 {
            self.width = width;
            self.height = height;
            psp::dprintln!(
                "video: first frame decoded: {}x{}, Y={:#010x} stride={} \
                 Cb={:#010x} Cr={:#010x}",
                width,
                height,
                y_ptr,
                y_stride,
                cb_ptr,
                cr_ptr
            );
        }

        // Validate pointers -- EDRAM is at 0x04000000..0x04200000 on PSP.
        if y_ptr < 0x0400_0000 || y_ptr >= 0x0420_0000 {
            psp::dprintln!(
                "video: Y pointer {:#010x} outside EDRAM, dumping buffer",
                y_ptr
            );
            self.log_codec_buffer();
            return None;
        }

        // Convert YCbCr 4:2:0 to RGBA using VFPU.
        // SAFETY: EDRAM pointers are valid after successful decode.
        // We access via uncached address (| 0x40000000) to bypass data cache
        // since the ME writes directly to EDRAM.
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

    /// Log all 96 words of the codec buffer for debugging on real hardware.
    fn log_codec_buffer(&self) {
        let ptr = self.buf.0.as_ptr();
        for row in 0..12 {
            let base = row * 8;
            // SAFETY: Reading within the 96-word buffer.
            let vals: [u32; 8] = unsafe {
                [
                    *ptr.add(base),
                    *ptr.add(base + 1),
                    *ptr.add(base + 2),
                    *ptr.add(base + 3),
                    *ptr.add(base + 4),
                    *ptr.add(base + 5),
                    *ptr.add(base + 6),
                    *ptr.add(base + 7),
                ]
            };
            psp::dprintln!(
                "  buf[{:02}..{:02}]: {:08x} {:08x} {:08x} {:08x} \
                 {:08x} {:08x} {:08x} {:08x}",
                base,
                base + 8,
                vals[0],
                vals[1],
                vals[2],
                vals[3],
                vals[4],
                vals[5],
                vals[6],
                vals[7],
            );
        }
    }
}

impl Drop for PspVideoDecoder {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: Release EDRAM allocated by sceVideocodecGetEDRAM.
            // buf.0.as_mut_ptr() is the same 64-byte-aligned pointer.
            unsafe { psp::sys::sceVideocodecReleaseEDRAM(self.buf.0.as_mut_ptr()) };
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

/// BT.601 YUV->RGB conversion matrix, 16-byte aligned for VFPU loads.
/// Row-major: each row contains coefficients for one output channel.
#[repr(C, align(16))]
struct Bt601Matrix([[f32; 4]; 4]);

/// The BT.601 conversion matrix (constant).
static BT601: Bt601Matrix = Bt601Matrix([
    [1.164, 0.0, 1.596, 0.0],     // R = 1.164*(Y-16) + 1.596*(Cr-128)
    [1.164, -0.392, -0.813, 0.0], // G = 1.164*(Y-16) - 0.392*(Cb-128) - 0.813*(Cr-128)
    [1.164, 2.017, 0.0, 0.0],     // B = 1.164*(Y-16) + 2.017*(Cb-128)
    [0.0, 0.0, 0.0, 1.0],         // A = passthrough (input[3] = 255)
]);

/// Bias vector subtracted from input: [16, 128, 128, 0].
/// After subtraction: [Y-16, Cb-128, Cr-128, 255-0=255].
#[repr(C, align(16))]
struct BiasVec([f32; 4]);

static YUV_BIAS: BiasVec = BiasVec([16.0, 128.0, 128.0, 0.0]);

/// Convert a YUV420P frame to RGBA using VFPU matrix multiply.
///
/// Processes 2 horizontally adjacent pixels per iteration: since YUV420
/// subsamples chroma 2:1 horizontally, each Cb/Cr value covers two luma
/// pixels, so we share the chroma read. RGBA output is packed as u32 and
/// written with a single word store instead of 4 byte stores per pixel.
///
/// Performance: ~12 VFPU instructions per pixel (vs ~18 in the 1-pixel
/// version), plus halved chroma reads and 75% fewer memory stores.
/// 480x272 frame converts in ~4-5ms at 333MHz.
///
/// # Safety
///
/// - `y_ptr`, `cb_ptr`, `cr_ptr` must point to valid YUV420P plane data
///   with the given dimensions.
/// - Y plane: `height` rows of `y_stride` bytes (only first `width` used).
/// - Cb/Cr planes: `height/2` rows of `y_stride/2` bytes each.
/// - Pointers should use uncached addresses for EDRAM data.
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
    // Allocate as u32 slice for word-aligned writes.
    let mut rgba_u32 = vec![0u32; w * h];

    let mat_ptr = BT601.0.as_ptr() as *const u8;
    let bias_ptr = YUV_BIAS.0.as_ptr() as *const u8;
    let clamp_255: u32 = 255.0f32.to_bits();

    // SAFETY: Load the BT.601 matrix into VFPU M000 and bias into C130.
    // These registers persist across the pixel loop since Rust's scalar
    // code doesn't touch VFPU registers.
    //
    // Register allocation:
    //   M000 (C000-C030): BT.601 conversion matrix (persistent)
    //   C130:             Bias vector [16, 128, 128, 0] (persistent)
    //   S200:             255.0 constant for clamping (persistent)
    //   C100:             Pixel 0 input [Y0, Cb, Cr, 255]
    //   C110:             Pixel 0 output [R0, G0, B0, A0]
    //   C120:             Temp for clamping (shared)
    //   C210:             Pixel 1 input [Y1, Cb, Cr, 255]
    //   C220:             Pixel 1 output [R1, G1, B1, A1]
    vfpu_asm!(
        "lv.q C000, 0({m})",
        "lv.q C010, 16({m})",
        "lv.q C020, 32({m})",
        "lv.q C030, 48({m})",
        "lv.q C130, 0({b})",
        "mtv {c255}, S200",
        m = in(reg) mat_ptr,
        b = in(reg) bias_ptr,
        c255 = in(reg) clamp_255,
        options(nostack),
    );

    // Process width in pairs of 2 (shared chroma).
    let w_pairs = w / 2;
    let w_tail = w & 1; // 1 if odd width, 0 if even

    for row in 0..h {
        let chroma_row = row / 2;
        let y_row_base = row * y_stride;
        let c_row_base = chroma_row * chroma_stride;
        let out_row_base = row * w;

        for pair in 0..w_pairs {
            let col = pair * 2;
            let chroma_col = pair; // col/2 == pair

            // SAFETY: Pointers are valid EDRAM (uncached) addresses for the
            // decoded frame. Indices are within plane dimensions.
            let y0 = unsafe { *y_ptr.add(y_row_base + col) } as u32;
            let y1 = unsafe { *y_ptr.add(y_row_base + col + 1) } as u32;
            let cb = unsafe { *cb_ptr.add(c_row_base + chroma_col) } as u32;
            let cr = unsafe { *cr_ptr.add(c_row_base + chroma_col) } as u32;
            let alpha: u32 = 255;

            let r0: u32;
            let g0: u32;
            let b0: u32;
            let r1: u32;
            let g1: u32;
            let b1: u32;

            // SAFETY: VFPU conversion for 2 pixels sharing the same
            // Cb/Cr. Each pixel is a separate vfpu_asm! block to stay
            // within MIPS register pressure limits (7 operands each).
            // The chroma read is shared in Rust, halving EDRAM loads.
            vfpu_asm!(
                "mtv {y}, S100",
                "mtv {cb}, S101",
                "mtv {cr}, S102",
                "mtv {a}, S103",
                "vi2f.q C100, C100, 0",
                "vsub.q C100, C100, C130",
                "vtfm4.q C110, M000, C100",
                "vzero.q C120",
                "vmax.q C110, C110, C120",
                "vone.q C120",
                "vscl.q C120, C120, S200",
                "vmin.q C110, C110, C120",
                "vf2iz.q C110, C110, 0",
                "mfv {ro}, S110",
                "mfv {go}, S111",
                "mfv {bo}, S112",
                y = in(reg) y0,
                cb = in(reg) cb,
                cr = in(reg) cr,
                a = in(reg) alpha,
                ro = out(reg) r0,
                go = out(reg) g0,
                bo = out(reg) b0,
                options(nostack),
            );

            vfpu_asm!(
                "mtv {y}, S100",
                "mtv {cb}, S101",
                "mtv {cr}, S102",
                "mtv {a}, S103",
                "vi2f.q C100, C100, 0",
                "vsub.q C100, C100, C130",
                "vtfm4.q C110, M000, C100",
                "vzero.q C120",
                "vmax.q C110, C110, C120",
                "vone.q C120",
                "vscl.q C120, C120, S200",
                "vmin.q C110, C110, C120",
                "vf2iz.q C110, C110, 0",
                "mfv {ro}, S110",
                "mfv {go}, S111",
                "mfv {bo}, S112",
                y = in(reg) y1,
                cb = in(reg) cb,
                cr = in(reg) cr,
                a = in(reg) alpha,
                ro = out(reg) r1,
                go = out(reg) g1,
                bo = out(reg) b1,
                options(nostack),
            );

            // Pack RGBA as u32 (little-endian: byte order R, G, B, A).
            let pix0 = r0 | (g0 << 8) | (b0 << 16) | 0xFF00_0000;
            let pix1 = r1 | (g1 << 8) | (b1 << 16) | 0xFF00_0000;

            let idx = out_row_base + col;
            rgba_u32[idx] = pix0;
            rgba_u32[idx + 1] = pix1;
        }

        // Handle odd-width tail pixel (rare for video, but correct).
        if w_tail != 0 {
            let col = w - 1;
            let chroma_col = col / 2;

            let y_val = unsafe { *y_ptr.add(y_row_base + col) } as u32;
            let cb_val =
                unsafe { *cb_ptr.add(c_row_base + chroma_col) } as u32;
            let cr_val =
                unsafe { *cr_ptr.add(c_row_base + chroma_col) } as u32;
            let alpha: u32 = 255;

            let r: u32;
            let g: u32;
            let b: u32;

            vfpu_asm!(
                "mtv {y}, S100",
                "mtv {cb}, S101",
                "mtv {cr}, S102",
                "mtv {a}, S103",
                "vi2f.q C100, C100, 0",
                "vsub.q C100, C100, C130",
                "vtfm4.q C110, M000, C100",
                "vzero.q C120",
                "vmax.q C110, C110, C120",
                "vone.q C120",
                "vscl.q C120, C120, S200",
                "vmin.q C110, C110, C120",
                "vf2iz.q C110, C110, 0",
                "mfv {ro}, S110",
                "mfv {go}, S111",
                "mfv {bo}, S112",
                y = in(reg) y_val,
                cb = in(reg) cb_val,
                cr = in(reg) cr_val,
                a = in(reg) alpha,
                ro = out(reg) r,
                go = out(reg) g,
                bo = out(reg) b,
                options(nostack),
            );

            let pix = r | (g << 8) | (b << 16) | 0xFF00_0000;
            rgba_u32[out_row_base + col] = pix;
        }
    }

    // Reinterpret Vec<u32> as Vec<u8> without copying.
    let rgba = {
        let ptr = rgba_u32.as_mut_ptr() as *mut u8;
        let len = rgba_u32.len() * 4;
        let cap = rgba_u32.capacity() * 4;
        core::mem::forget(rgba_u32);
        // SAFETY: u32 and u8 have compatible layouts when multiplied.
        // The pointer, length, and capacity are correctly scaled.
        unsafe { Vec::from_raw_parts(ptr, len, cap) }
    };

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
