//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once -- PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video H.264 frames are decoded via `sceMpeg` (Media Engine) on real PSP
//! hardware, with PSMF container wrapping and direct ABGR pixel output.
//!
//! The sceMpeg API is used instead of the lower-level sceVideocodec because
//! sceVideocodec weak imports fail to resolve on many CFW configurations
//! (error 0x806201fe), while sceMpeg is universally available.
//!
//! The ringbuffer callback feeds PSMF-structured data (2048-byte packets):
//! first packet is the PSMF header, subsequent packets are MPEG-PS packs
//! containing H.264 AUs in PES video packets.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;
use crate::psmf::{PACKET_SIZE, PsmfMuxer};
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
    /// Raw AVCC data (before Annex B conversion) for NAL decoder.
    pub raw_avcc: Option<Vec<u8>>,
    /// AVCC NAL length prefix size (typically 4).
    pub nal_prefix_size: u8,
    /// SPS from MP4 avcC atom (raw NAL, no start codes).
    pub avcc_sps: Option<Vec<u8>>,
    /// PPS from MP4 avcC atom (raw NAL, no start codes).
    pub avcc_pps: Option<Vec<u8>>,
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

/// Pre-initialize the MPEG subsystem before any audio modules load.
///
/// Must be called from the main thread before spawning the audio thread.
pub fn preinit_mpeg() {
    crate::audio::load_av_modules_once_pub();
    load_vsh_mpeg_module();
    vlog("[VIDEO] preinit done");
}

/// Pre-load AV modules during init (AvMp3 only).
///
/// Starting the module during preinit causes a dashboard freeze (the module's
/// `module_start` likely allocates EDRAM or initializes ME hardware in a way
/// that conflicts with GU rendering). Instead, we defer `sceKernelStartModule`
/// to when the video thread actually needs it.
fn load_vsh_mpeg_module() {
    // Do NOT load AvMpegBase — it conflicts with mpeg_vsh370 (exclusive load).
    // mpeg_vsh370 is loaded from the video thread when decode is needed.
    unsafe {
        let r = psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMp3);
        vlog(&format!("[VIDEO] AvMp3 = {r:#x}"));
    }
}

/// Load cooleyesBridge.prx + mpeg_vsh370.prx from the video thread.
/// cooleyesBridge boots the ME core (required for H.264 decode).
/// mpeg_vsh370 provides the sceMpegVsh_library implementation.
fn load_mpeg_vsh_module() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOADED: AtomicBool = AtomicBool::new(false);
    if LOADED.swap(true, Ordering::Relaxed) {
        return;
    }

    // Load cooleyesBridge.prx to boot the ME core.
    // This is a tiny kernel bridge: cooleyesMeBootStart(devkit, type) → sceMeBootStart660.
    // Without ME boot, sceMpegAvcDecode returns 0x80628002 (FATAL).
    let bridge_id = unsafe {
        psp::sys::sceKernelLoadModule(
            b"ms0:/PSP/GAME/OASISOS/cooleyesBridge.prx\0".as_ptr(),
            0, core::ptr::null_mut(),
        )
    };
    vlog(&format!("[VIDEO] cooleyesBridge load = {:#x}", bridge_id.0));
    if bridge_id >= psp::sys::SceUid(0) {
        let mut status: i32 = 0;
        let ret = unsafe {
            psp::sys::sceKernelStartModule(
                bridge_id, 0, core::ptr::null_mut(),
                &mut status, core::ptr::null_mut(),
            )
        };
        vlog(&format!("[VIDEO] cooleyesBridge start = {ret:#x}"));
    }

    vlog("[VIDEO] loading mpeg_vsh370.prx...");
    let id = unsafe {
        psp::sys::sceKernelLoadModule(
            b"ms0:/PSP/GAME/OASISOS/mpeg_vsh370.prx\0".as_ptr(),
            0, core::ptr::null_mut(),
        )
    };
    vlog(&format!("[VIDEO] sceKernelLoadModule = {:#x}", id.0));
    if id < psp::sys::SceUid(0) {
        vlog("[VIDEO] load FAILED");
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
        vlog("[VIDEO] start FAILED");
        unsafe { psp::sys::sceKernelUnloadModule(id); }
        return;
    }
    vlog("[VIDEO] sceMpegVsh_library registered!");

    // Probe which sceMpeg stubs are resolved by reading the stub instructions.
    // Resolved stubs have: jr $ra (0x03E00008) + syscall N
    // Unresolved stubs have: jr $ra (0x03E00008) + nop (0x00000000)
    // OR: some other pattern like two pointer words (Stub struct).
    unsafe {
        // Read stub instructions for key functions to check resolution.
        let fns: &[(&str, unsafe extern "C" fn() -> i32)] = &[
            ("Init", core::mem::transmute(psp::sys::sceMpegInit as *const ())),
            ("QueryMemSize", core::mem::transmute(psp::sys::sceMpegQueryMemSize as *const ())),
            ("MallocEsBuf", core::mem::transmute(psp::sys::sceMpegMallocAvcEsBuf as *const ())),
            ("InitAu", core::mem::transmute(psp::sys::sceMpegInitAu as *const ())),
        ];
        for &(name, f) in fns {
            let addr = f as *const () as *const u32;
            let insn0 = core::ptr::read_volatile(addr);
            let insn1 = core::ptr::read_volatile(addr.add(1));
            let has_syscall = (insn1 & 0x3F) == 0x0C;
            vlog(&format!(
                "[VIDEO] stub {name}: @{addr:?} = {insn0:#010x} {insn1:#010x} {}",
                if has_syscall { "RESOLVED" } else { "unresolved" }
            ));
        }
    }
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

/// Round up to the next power of 2 (minimum 512 for PSP ME alignment).
fn next_power_of_2(v: u32) -> u32 {
    let min = 512;
    let mut n = v.max(min);
    n -= 1;
    n |= n >> 1;
    n |= n >> 2;
    n |= n >> 4;
    n |= n >> 8;
    n |= n >> 16;
    n + 1
}

// ---------------------------------------------------------------------------
// Ringbuffer callback context (static, single video thread)
// ---------------------------------------------------------------------------

/// Context for the ringbuffer callback. Since there's only one video
/// decoder thread, we use static state.
struct RingbufCtx {
    /// Pointer to PSMF packet data to copy into the ringbuffer.
    ptr: *const u8,
    /// Total bytes available.
    len: usize,
    /// Bytes consumed so far.
    offset: usize,
}

// SAFETY: Only accessed from the video thread (single-threaded context).
static mut RINGBUF_CTX: RingbufCtx = RingbufCtx {
    ptr: core::ptr::null(),
    len: 0,
    offset: 0,
};

/// Ringbuffer callback invoked by `sceMpegRingbufferPut`.
///
/// Copies PSMF packet data from `RINGBUF_CTX` into the ringbuffer's
/// data buffer. Each packet is 2048 bytes.
///
/// # Safety
///
/// Called by the PSP firmware from within `sceMpegRingbufferPut`.
/// `data` points into the ringbuffer's pre-allocated data buffer.
unsafe extern "C" fn ringbuffer_callback(
    data: *mut c_void,
    num_packets: i32,
    _param: *mut c_void,
) -> i32 {
    // Count callback invocations to distinguish header vs AU data.
    static mut CB_COUNT: u32 = 0;
    let cb_num = unsafe { CB_COUNT };
    unsafe { CB_COUNT += 1; }

    // Log callback invocation.
    {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] ringbuf_cb entered\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    if num_packets <= 0 {
        return 0;
    }

    // SAFETY: Single-threaded access from video thread only.
    // Use raw pointer access (Rust 2024 forbids &mut to mutable statics).
    let ctx = unsafe { &raw mut RINGBUF_CTX };
    let ctx_ptr = unsafe { (*ctx).ptr };
    let ctx_len = unsafe { (*ctx).len };
    let ctx_offset = unsafe { (*ctx).offset };

    if ctx_ptr.is_null() || ctx_offset >= ctx_len {
        return 0;
    }

    let remaining = ctx_len - ctx_offset;
    let bytes_requested = num_packets as usize * PACKET_SIZE;
    let bytes_to_copy = remaining.min(bytes_requested);
    let packets_to_copy = bytes_to_copy / PACKET_SIZE;

    if packets_to_copy == 0 {
        return 0;
    }

    let src = ctx_ptr.add(ctx_offset);
    let dst = data as *mut u8;
    let copy_len = packets_to_copy * PACKET_SIZE;

    // Log pointers and hex-dump src data BEFORE copy (first AU only).
    // Log pointers, hex-dump src, and test dst write (first AU only).
    if cb_num == 1 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            // Format hex dump manually (no alloc in callback).
            let hex = b"0123456789abcdef";
            let mut buf = [0u8; 200];

            // Log pointers.
            let prefix = b"[VIDEO] src=0x";
            buf[..prefix.len()].copy_from_slice(prefix);
            let mut p = prefix.len();
            let src_addr = src as usize;
            for shift in (0..8).rev() {
                let nibble = (src_addr >> (shift * 4)) & 0xF;
                buf[p] = hex[nibble];
                p += 1;
            }
            buf[p..p + 5].copy_from_slice(b" dst=");
            p += 5;
            buf[p..p + 2].copy_from_slice(b"0x");
            p += 2;
            let dst_addr = dst as usize;
            for shift in (0..8).rev() {
                let nibble = (dst_addr >> (shift * 4)) & 0xF;
                buf[p] = hex[nibble];
                p += 1;
            }
            buf[p] = b'\n';
            p += 1;
            psp::sys::sceIoWrite(fd, buf.as_ptr() as *const _, p);

            // Hex dump first 48 bytes of src.
            let mut buf2 = [0u8; 200];
            let prefix2 = b"[VIDEO] pkt: ";
            buf2[..prefix2.len()].copy_from_slice(prefix2);
            let mut p2 = prefix2.len();
            let dump_len = 48usize.min(copy_len);
            for i in 0..dump_len {
                let b = *src.add(i);
                buf2[p2] = hex[(b >> 4) as usize];
                buf2[p2 + 1] = hex[(b & 0xF) as usize];
                buf2[p2 + 2] = b' ';
                p2 += 3;
            }
            buf2[p2] = b'\n';
            p2 += 1;
            psp::sys::sceIoWrite(fd, buf2.as_ptr() as *const _, p2);

            // Test: write a single byte to dst to check if writable.
            let test_msg = b"[VIDEO] testing dst write...\n";
            psp::sys::sceIoWrite(fd, test_msg.as_ptr() as *const _, test_msg.len());
            *dst = 0xAA;
            let ok_msg = b"[VIDEO] dst write OK\n";
            psp::sys::sceIoWrite(fd, ok_msg.as_ptr() as *const _, ok_msg.len());

            psp::sys::sceIoClose(fd);
        }
    }

    let actual_copy = packets_to_copy;
    let actual_len = actual_copy * PACKET_SIZE;

    // Copy data into ringbuffer.
    for i in 0..actual_len {
        unsafe { *dst.add(i) = *src.add(i); }
    }
    unsafe { (*ctx).offset = ctx_offset + actual_len; }

    // Log after copy (first AU only).
    if cb_num <= 2 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] copy done, flushing dcache...\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    // Flush D-cache on the destination buffer.
    unsafe {
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            dst as *const c_void,
            actual_len as u32,
        );
    }

    if cb_num <= 2 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] returning from callback\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    actual_copy as i32
}

// ---------------------------------------------------------------------------
// NAL-based H.264 decoder (cooleyes/PMPlayer approach)
// ---------------------------------------------------------------------------

/// Mp4AvcNalStruct — matches the C struct from PMPlayer.
#[repr(C)]
struct Mp4AvcNalStruct {
    sps_buffer: *const u8,
    sps_size: i32,
    pps_buffer: *const u8,
    pps_size: i32,
    nal_prefix_size: i32,
    nal_buffer: *const u8,
    nal_size: i32,
    mode: i32,  // 3 for first frame, 0 for subsequent
}

/// NAL-based H.264 decoder using sceMpegGetAvcNalAu.
///
/// Feeds raw H.264 NAL units directly to the ME, bypassing the MPEG-PS
/// demuxer and broken avcodec stubs. Based on the cooleyes/PMPlayer
/// approach that's proven to work on real PSP hardware.
struct NalDecoder {
    mpeg_storage: *mut c_void,
    mpeg_data: Vec<u8>,
    ddr_block: psp::sys::SceUid,
    es_buf: *mut c_void,
    au: psp::sys::SceMpegAu,
    sps: Vec<u8>,
    pps: Vec<u8>,
    nal_prefix_size: i32,
    output_buf: Vec<u8>,
    pic_num: i32,
    frame_width: u32,
    width: u32,
    height: u32,
    first_frame: bool,
}

impl NalDecoder {
    /// Initialize the NAL decoder from the first keyframe.
    ///
    /// Uses SPS/PPS from the MP4 avcC atom if available (preferred),
    /// otherwise extracts from the Annex B stream.
    fn try_init(first_frame: &StreamFrame) -> Result<Self, String> {
        vlog("[VIDEO] NalDecoder::try_init");
        crate::audio::load_av_modules_once_pub();
        load_mpeg_vsh_module();

        // Prefer SPS/PPS from MP4 avcC atom (exact match for ME expectations).
        let (sps, pps) = if let (Some(s), Some(p)) = (&first_frame.avcc_sps, &first_frame.avcc_pps) {
            (s.clone(), p.clone())
        } else {
            extract_sps_pps(&first_frame.data)
                .ok_or_else(|| "no SPS/PPS found".to_string())?
        };
        // Log SPS profile/level (first 3 bytes: profile_idc, flags, level_idc).
        let prof = if sps.len() >= 4 {
            format!("profile={:#x} level={:#x}", sps[1], sps[3])
        } else {
            "short".to_string()
        };
        vlog(&format!(
            "[VIDEO] NAL: SPS={} PPS={} {prof}", sps.len(), pps.len()
        ));

        // Parse dimensions from SPS.
        let (width, height) = parse_sps_dimensions(&first_frame.data)
            .unwrap_or((480, 272));
        let frame_width = if width > 480 { 768 } else { 512 };
        vlog(&format!("[VIDEO] NAL: {width}x{height}, stride={frame_width}"));

        // Use standard linked sceMpeg functions. The VSH addresses
        // are in a different process space and can't be called.
        // The kernel PRX handles ME boot + avcodec stub patching.
        vlog("[VIDEO] NAL: using linked sceMpeg");

        let ret = unsafe { psp::sys::sceMpegInit() };
        vlog(&format!("[VIDEO] NAL: sceMpegInit = {ret:#x}"));
        if ret < 0 && ret != 0x80618003_u32 as i32 && ret != 0x80618005_u32 as i32 {
            return Err(format!("sceMpegInit: {ret:#x}"));
        }

        // Mode 5 for Main Profile or >480x272 (cooleyes pattern).
        let mpeg_mode = if width > 480 || height > 272 { 5 } else { 4 };
        let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(mpeg_mode) };
        if mem_size <= 0 {
            return Err(format!("QueryMemSize({mpeg_mode}): {mem_size}"));
        }
        vlog(&format!("[VIDEO] NAL: mem_size={mem_size}, mode={mpeg_mode}"));

        let mut mpeg_data = vec![0u8; mem_size as usize + 64];
        let mpeg_data_aligned = {
            let p = mpeg_data.as_mut_ptr();
            unsafe { p.add(p.align_offset(64)) }
        };

        // DDR-top: 2MB for ME decode output, 4MB aligned (cooleyes pattern).
        // Allocate 2MB + 4MB for alignment overhead.
        let ddr_block = unsafe {
            psp::sys::sceKernelAllocPartitionMemory(
                psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
                b"MeDdrTop\0".as_ptr(),
                psp::sys::SceSysMemBlockTypes::Low,
                0x20_0000 + 0x40_0000, // 2MB + 4MB alignment
                core::ptr::null_mut(),
            )
        };
        if ddr_block < psp::sys::SceUid(0) {
            return Err(format!("DDR-top alloc: {:#x}", ddr_block.0));
        }
        let ddr_raw = unsafe { psp::sys::sceKernelGetBlockHeadAddr(ddr_block) };
        // Align to 4MB boundary.
        let ddr_aligned = ((ddr_raw as u32) + 0x3F_FFFF) & !0x3F_FFFF;
        vlog(&format!("[VIDEO] NAL: DDR raw={ddr_raw:?} aligned={ddr_aligned:#x}"));

        let mpeg_storage = Box::into_raw(Box::new(core::ptr::null_mut::<c_void>()));
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(mpeg_storage as *mut *mut c_void)
        };

        // Construct a real ringbuffer (mpeg_vsh370 validates it during Create).
        let rb_packets = 8; // Minimal — NAL feeding bypasses the ringbuffer.
        let rb_size = unsafe { psp::sys::sceMpegRingbufferQueryMemSize(rb_packets) };
        vlog(&format!("[VIDEO] NAL: rb_size={rb_size}"));
        let mut rb_data = vec![0u8; if rb_size > 0 { rb_size as usize } else { 16384 }];
        let mut ringbuffer = Box::new(unsafe {
            core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
        });
        if rb_size > 0 {
            let ret = unsafe {
                psp::sys::sceMpegRingbufferConstruct(
                    &mut *ringbuffer, rb_packets,
                    rb_data.as_mut_ptr() as *mut c_void,
                    rb_size, None, core::ptr::null_mut(),
                )
            };
            vlog(&format!("[VIDEO] NAL: RingbufferConstruct = {ret:#x}"));
        }

        let ret = unsafe {
            psp::sys::sceMpegCreate(
                mpeg,
                mpeg_data_aligned as *mut c_void,
                mem_size as i32,
                &mut *ringbuffer,
                512, // frame_width
                0,   // unk1 (standard)
                0,   // unk2 (standard)
            )
        };
        vlog(&format!("[VIDEO] NAL: sceMpegCreate = {ret:#x}"));
        if ret < 0 {
            unsafe { let _ = Box::from_raw(mpeg_storage); }
            return Err(format!("sceMpegCreate: {ret:#x}"));
        }
        vlog("[VIDEO] NAL: sceMpegCreate OK");

        // Register video stream (required before MallocAvcEsBuf/AvcDecode).
        let stream = unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };
        vlog(&format!("[VIDEO] NAL: RegistStream = {:?}", stream));

        // Allocate ES buffer and init AU.
        let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
        vlog(&format!("[VIDEO] NAL: MallocAvcEsBuf = {:?}", es_buf));
        if es_buf.is_null() {
            // Try without ringbuffer — some firmware versions need different init order.
            return Err("MallocAvcEsBuf returned null".to_string());
        }
        vlog("[VIDEO] NAL: ES buffer allocated");

        let mut au = unsafe { core::mem::zeroed::<psp::sys::SceMpegAu>() };
        let ret = unsafe { psp::sys::sceMpegInitAu(mpeg, es_buf, &mut au) };
        if ret < 0 {
            unsafe {
                psp::sys::sceMpegDelete(mpeg);
                let _ = Box::from_raw(mpeg_storage);
            }
            return Err(format!("sceMpegInitAu: {ret:#x}"));
        }
        vlog("[VIDEO] NAL: sceMpegInitAu OK");

        // Skip AvcDecodeMode for NAL approach — cooleyes doesn't call it.
        // The 0x80628002 error from AvcDecode may be about something else.

        // Output pixel buffer.
        let out_h = ((height + 15) / 16) * 16;
        let output_buf = vec![0u8; frame_width as usize * out_h as usize * 4];

        Ok(Self {
            mpeg_storage: mpeg_storage as *mut c_void,
            mpeg_data,
            ddr_block,
            es_buf,
            au,
            sps,
            pps,
            nal_prefix_size: first_frame.nal_prefix_size as i32,
            output_buf,
            pic_num: 0,
            frame_width,
            width,
            height,
            first_frame: true,
        })
    }

    fn mpeg(&self) -> psp::sys::SceMpeg {
        unsafe {
            core::mem::transmute(self.mpeg_storage as *mut *mut c_void)
        }
    }

    /// Decode one H.264 access unit using NAL-based approach.
    fn decode(&mut self, au_data: &[u8], _pts_secs: f64,
              raw_avcc: Option<&[u8]>, avcc_prefix: u8) -> Option<DecodedFrame> {
        if au_data.is_empty() {
            return None;
        }

        static DECODE_COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);
        let call_num = DECODE_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let verbose = call_num < 10;

        let mpeg = self.mpeg();

        // Build NAL struct.
        let mode = if self.first_frame { 3 } else { 0 };
        self.first_frame = false;

        // Use raw AVCC data (length-prefixed NALs) if available.
        // sceMpegGetAvcNalAu expects AVCC format, not Annex B.
        let (nal_buf, nal_len, prefix) = if let Some(avcc) = raw_avcc {
            (avcc.as_ptr(), avcc.len() as i32, avcc_prefix as i32)
        } else {
            (au_data.as_ptr(), au_data.len() as i32, 0)
        };

        let mut nal = Mp4AvcNalStruct {
            sps_buffer: self.sps.as_ptr(),
            sps_size: self.sps.len() as i32,
            pps_buffer: self.pps.as_ptr(),
            pps_size: self.pps.len() as i32,
            nal_prefix_size: prefix,
            nal_buffer: nal_buf,
            nal_size: nal_len,
            mode,
        };

        // Always log first frame details.
        if call_num == 0 {
            vlog(&format!(
                "[VIDEO] NAL: sps={} pps={} nal={} prefix={} mode={}",
                self.sps.len(), self.pps.len(), nal_len, prefix, mode
            ));
            // Hex dump first 16 bytes.
            let dump_len = (nal_len as usize).min(16);
            let slice = unsafe { core::slice::from_raw_parts(nal_buf, dump_len) };
            let hex_chars = b"0123456789abcdef";
            let mut hex_buf = [0u8; 64];
            let mut hp = 0;
            for &b in slice {
                if hp + 3 > hex_buf.len() { break; }
                hex_buf[hp] = hex_chars[(b >> 4) as usize];
                hex_buf[hp + 1] = hex_chars[(b & 0xF) as usize];
                hex_buf[hp + 2] = b' ';
                hp += 3;
            }
            let hex_str = core::str::from_utf8(&hex_buf[..hp]).unwrap_or("?");
            vlog(&format!("[VIDEO] NAL: data={hex_str}"));
            // Also log SPS first bytes.
            let sps_dump = self.sps.len().min(8);
            let mut sbuf = [0u8; 32];
            let mut sp = 0;
            for &b in &self.sps[..sps_dump] {
                if sp + 3 > sbuf.len() { break; }
                sbuf[sp] = hex_chars[(b >> 4) as usize];
                sbuf[sp + 1] = hex_chars[(b & 0xF) as usize];
                sbuf[sp + 2] = b' ';
                sp += 3;
            }
            let sps_str = core::str::from_utf8(&sbuf[..sp]).unwrap_or("?");
            vlog(&format!("[VIDEO] NAL: sps={sps_str}"));
        }

        // Flush cache on NAL data + struct.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                au_data.as_ptr() as *const c_void, au_data.len() as u32,
            );
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                &nal as *const _ as *const c_void,
                core::mem::size_of::<Mp4AvcNalStruct>() as u32,
            );
        }

        // Feed NAL to ME.
        let ret = unsafe {
            psp::sys::sceMpegGetAvcNalAu(
                mpeg, &mut nal as *mut _ as *mut c_void, &mut self.au,
            )
        };
        if ret < 0 {
            if verbose {
                vlog(&format!(
                    "[VIDEO] NAL: GetAvcNalAu = {ret:#x} prefix={prefix} nalsz={nal_len} mode={mode}"
                ));
            }
            return None;
        }
        if verbose {
            vlog("[VIDEO] NAL: GetAvcNalAu OK");
        }

        // Decode.
        let mut output_ptr = self.output_buf.as_mut_ptr() as *mut c_void;
        let buf_arg = &mut output_ptr as *mut *mut c_void as *mut c_void;
        let ret = unsafe {
            psp::sys::sceMpegAvcDecode(
                mpeg, &mut self.au, 512, buf_arg, &mut self.pic_num,
            )
        };
        if ret < 0 {
            if verbose || call_num < 50 {
                vlog(&format!("[VIDEO] NAL: AvcDecode = {ret:#x} pic={}",
                    self.pic_num));
            }
            return None;
        }
        if verbose {
            vlog(&format!("[VIDEO] NAL: AvcDecode OK, pic_num={}", self.pic_num));
        }

        if self.pic_num <= 0 {
            return None; // No picture produced yet (B-frame reordering).
        }

        // Get YCbCr buffer pointers.
        let mut detail2: *mut c_void = core::ptr::null_mut();
        let ret = unsafe {
            psp::sys::sceMpegAvcDecodeDetail2(mpeg, &mut detail2)
        };
        if ret < 0 || detail2.is_null() {
            if verbose {
                vlog(&format!("[VIDEO] NAL: DecodeDetail2 = {ret:#x}"));
            }
            return None;
        }

        // CSC: YCbCr → ABGR via hardware.
        // Build CSC struct from detail2.
        // detail2 is Mp4AvcDetail2Struct; CSC needs info_buffer and yuv_buffer.
        // info_buffer is at offset 16 (4th u32), yuv_buffer at offset 44 (11th u32).
        let detail_ptr = detail2 as *const u32;
        let info_ptr = unsafe { *detail_ptr.add(4) } as *const u32; // info_buffer
        let yuv_ptr = unsafe { *detail_ptr.add(11) } as *const u32; // yuv_buffer

        if info_ptr.is_null() || yuv_ptr.is_null() {
            if verbose {
                vlog("[VIDEO] NAL: null info/yuv pointers");
            }
            return None;
        }

        // Build CscStruct on stack.
        // Format: height_mbs, width_mbs, mode0, mode1, buffer0..buffer7
        let info_w = unsafe { *info_ptr.add(2) } as u32; // width at offset 8
        let info_h = unsafe { *info_ptr.add(3) } as u32; // height at offset 12
        let csc_width = if info_w > 480 { 768i32 } else { 512 };

        #[repr(C)]
        struct Mp4AvcCscStruct {
            height: i32,
            width: i32,
            mode0: i32,
            mode1: i32,
            buffers: [*const c_void; 8],
        }

        let csc = Mp4AvcCscStruct {
            height: ((info_h + 15) / 16) as i32,
            width: ((info_w + 15) / 16) as i32,
            mode0: 0,
            mode1: 0,
            buffers: [
                unsafe { *yuv_ptr.add(0) } as *const c_void,
                unsafe { *yuv_ptr.add(1) } as *const c_void,
                unsafe { *yuv_ptr.add(2) } as *const c_void,
                unsafe { *yuv_ptr.add(3) } as *const c_void,
                unsafe { *yuv_ptr.add(4) } as *const c_void,
                unsafe { *yuv_ptr.add(5) } as *const c_void,
                unsafe { *yuv_ptr.add(6) } as *const c_void,
                unsafe { *yuv_ptr.add(7) } as *const c_void,
            ],
        };

        let ret = unsafe {
            psp::sys::sceMpegBaseCscAvc(
                self.output_buf.as_mut_ptr() as *mut c_void,
                0,
                csc_width,
                &csc as *const _ as *mut c_void,
            )
        };
        if ret < 0 {
            if verbose {
                vlog(&format!("[VIDEO] NAL: CscAvc = {ret:#x}"));
            }
            return None;
        }

        if verbose {
            vlog("[VIDEO] NAL: FRAME DECODED!");
        }

        // Copy from output buffer (stride=frame_width) to output (stride=width).
        let w = self.width as usize;
        let h = self.height as usize;
        let stride = self.frame_width as usize;
        let mut rgba = vec![0u8; w * h * 4];
        for row in 0..h {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.output_buf.as_ptr().add(row * stride * 4),
                    rgba.as_mut_ptr().add(row * w * 4),
                    w * 4,
                );
            }
        }

        Some(DecodedFrame {
            rgba,
            width: self.width,
            height: self.height,
        })
    }
}

impl Drop for NalDecoder {
    fn drop(&mut self) {
        let mpeg = self.mpeg();
        unsafe {
            if !self.es_buf.is_null() {
                psp::sys::sceMpegFreeAvcEsBuf(mpeg, self.es_buf);
            }
            psp::sys::sceMpegDelete(mpeg);
            let _ = Box::from_raw(self.mpeg_storage as *mut *mut c_void);
            if self.ddr_block >= psp::sys::SceUid(0) {
                psp::sys::sceKernelFreePartitionMemory(self.ddr_block);
            }
            psp::sys::sceMpegFinish();
        }
    }
}

/// Extract SPS and PPS NAL units from an Annex B H.264 stream.
fn extract_sps_pps(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut sps = None;
    let mut pps = None;
    let mut i = 0;
    while i + 4 < data.len() {
        // Find start code (00 00 00 01 or 00 00 01).
        if data[i] == 0 && data[i + 1] == 0 {
            let (sc_len, nal_start) = if data[i + 2] == 1 {
                (3, i + 3)
            } else if data[i + 2] == 0 && i + 3 < data.len() && data[i + 3] == 1 {
                (4, i + 4)
            } else {
                i += 1;
                continue;
            };

            if nal_start >= data.len() {
                break;
            }

            let nal_type = data[nal_start] & 0x1F;

            // Find end of this NAL (next start code or end of data).
            let mut end = nal_start + 1;
            while end + 2 < data.len() {
                if data[end] == 0 && data[end + 1] == 0
                    && (data[end + 2] == 1
                        || (end + 3 < data.len() && data[end + 2] == 0 && data[end + 3] == 1))
                {
                    break;
                }
                end += 1;
            }
            if end + 2 >= data.len() {
                end = data.len();
            }

            let nal_data = &data[nal_start..end];

            if nal_type == 7 && sps.is_none() {
                sps = Some(nal_data.to_vec());
            } else if nal_type == 8 && pps.is_none() {
                pps = Some(nal_data.to_vec());
            }

            if sps.is_some() && pps.is_some() {
                break;
            }

            i = end;
        } else {
            i += 1;
        }
    }
    Some((sps?, pps?))
}

// ---------------------------------------------------------------------------
// SceMpeg H.264 video decoder (ringbuffer-based, disabled — kept for reference)
// ---------------------------------------------------------------------------

/// PSP hardware H.264 video decoder using sceMpeg (Media Engine).
///
/// Uses the PSMF container format to feed H.264 AUs through the sceMpeg
/// ringbuffer pipeline. The ME decodes H.264 and outputs ABGR pixels
/// directly (via `sceMpegAvcDecodeMode` with `Psm8888`).
struct SceMpegDecoder {
    /// sceMpeg handle (pointer-to-pointer).
    mpeg_storage: *mut c_void,
    /// Ringbuffer for feeding PSMF data to the ME (heap-allocated for
    /// stable address — sceMpegCreate stores an internal pointer to it).
    ringbuffer: Box<psp::sys::SceMpegRingbuffer>,
    /// Working memory for sceMpeg (64-byte aligned).
    mpeg_data: Vec<u8>,
    /// Ringbuffer packet data memory.
    ringbuf_data: Vec<u8>,
    /// Registered video stream handle.
    video_stream: psp::sys::SceMpegStream,
    /// ES buffer handle from sceMpegMallocAvcEsBuf.
    es_buf: *mut c_void,
    /// Access unit descriptor.
    au: psp::sys::SceMpegAu,
    /// PSMF muxer for wrapping H.264 AUs.
    muxer: PsmfMuxer,
    /// Output pixel buffer (512 × height × 4 ABGR).
    output_buf: Vec<u8>,
    /// Decode init flag (passed to sceMpegAvcDecode).
    decode_init: i32,
    /// Video dimensions.
    width: u32,
    height: u32,
    /// Output stride (power-of-2 >= width).
    frame_width: u32,
}

impl SceMpegDecoder {
    /// Number of ringbuffer packets. 256 packets = 512KB, enough for
    /// TV Guide H.264 AUs which are typically 10-50KB.
    const RB_PACKETS: i32 = 256;

    /// Attempt to initialize the sceMpeg H.264 hardware decoder.
    ///
    /// Returns `Err` on PPSSPP (sceMpeg not fully emulated for streaming)
    /// or if the MPEG modules are unavailable.
    fn try_init(width: u32, height: u32) -> Result<Self, String> {
        vlog("[VIDEO] SceMpegDecoder::try_init start");
        crate::audio::load_av_modules_once_pub();
        load_mpeg_vsh_module();
        vlog("[VIDEO] AV modules loaded");

        // Step 1: Init MPEG subsystem (may already be done by preinit_mpeg).
        // SAFETY: sceMpegInit is idempotent; ignore "already init" errors.
        // Known "already initialized" codes:
        //   0x80618003 = SCE_MPEG_ERROR_ALREADY_INIT
        //   0x80618005 = firmware-specific "already init" variant (PSP-3001)
        let ret = unsafe { psp::sys::sceMpegInit() };
        if ret < 0
            && ret != 0x80618003_u32 as i32
            && ret != 0x80618005_u32 as i32
        {
            return Err(format!("sceMpegInit failed: {ret:#x}"));
        }
        vlog(&format!("[VIDEO] sceMpegInit = {ret:#x}"));

        // Step 2: Query and allocate working memory.
        let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(0) };
        if mem_size <= 0 {
            return Err(format!("sceMpegQueryMemSize = {mem_size}"));
        }
        vlog(&format!("[VIDEO] mpeg mem size = {mem_size}"));

        // Allocate 64-byte aligned working memory.
        let mut mpeg_data = vec![0u8; mem_size as usize + 64];
        let mpeg_data_aligned = {
            let p = mpeg_data.as_mut_ptr();
            let off = p.align_offset(64);
            // SAFETY: offset is within the over-allocated buffer.
            unsafe { p.add(off) }
        };

        // Step 3: Query and allocate ringbuffer memory.
        let rb_size = unsafe { psp::sys::sceMpegRingbufferQueryMemSize(Self::RB_PACKETS) };
        if rb_size <= 0 {
            return Err(format!("sceMpegRingbufferQueryMemSize = {rb_size}"));
        }
        vlog(&format!("[VIDEO] ringbuf mem size = {rb_size}"));
        let mut ringbuf_data = vec![0u8; rb_size as usize];

        // Step 4: Construct ringbuffer (heap-allocated for stable address).
        // sceMpegCreate stores an internal pointer to the ringbuffer, so it
        // must not move after creation.
        // SAFETY: ringbuf_data is valid for rb_size bytes.
        let mut ringbuffer = Box::new(unsafe {
            core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
        });
        let ret = unsafe {
            psp::sys::sceMpegRingbufferConstruct(
                &mut *ringbuffer,
                Self::RB_PACKETS,
                ringbuf_data.as_mut_ptr() as *mut c_void,
                rb_size,
                Some(ringbuffer_callback),
                core::ptr::null_mut(),
            )
        };
        if ret < 0 {
            return Err(format!("sceMpegRingbufferConstruct = {ret:#x}"));
        }
        vlog("[VIDEO] ringbuffer constructed");

        // Step 5: Create MPEG handle.
        // SceMpeg is *mut *mut c_void — must point to valid heap storage.
        let mpeg_storage_box = Box::into_raw(Box::new(core::ptr::null_mut::<c_void>()));
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(mpeg_storage_box as *mut *mut c_void)
        };
        let mpeg_storage = mpeg_storage_box as *mut c_void;

        // Frame width must be >= video width, rounded up to next power of 2.
        // The ME uses this as the output stride for decoded frames.
        let frame_width = next_power_of_2(width);
        vlog(&format!(
            "[VIDEO] frame_width = {frame_width} (video = {width}x{height})"
        ));

        let ret = unsafe {
            psp::sys::sceMpegCreate(
                mpeg,
                mpeg_data_aligned as *mut c_void,
                mem_size as i32,
                &mut *ringbuffer,
                frame_width as i32,
                0,
                0,
            )
        };
        if ret < 0 {
            // SAFETY: Clean up allocated storage.
            unsafe {
                let _ = Box::from_raw(mpeg_storage_box);
                psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
            }
            return Err(format!("sceMpegCreate = {ret:#x}"));
        }
        vlog("[VIDEO] sceMpegCreate OK");

        // Step 6: Register video stream (stream_id=0 for video).
        let video_stream = unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };
        vlog("[VIDEO] stream registered");

        // Step 7: Set decode mode to ABGR 8888 for direct pixel output.
        let mut mode = psp::sys::SceMpegAvcMode {
            unk0: -1,
            pixel_format: psp::sys::DisplayPixelFormat::Psm8888,
        };
        let ret = unsafe { psp::sys::sceMpegAvcDecodeMode(mpeg, &mut mode) };
        vlog(&format!("[VIDEO] sceMpegAvcDecodeMode = {ret:#x}"));

        // Step 8: Allocate ES buffer.
        let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
        if es_buf.is_null() {
            unsafe {
                psp::sys::sceMpegUnRegistStream(mpeg, video_stream);
                psp::sys::sceMpegDelete(mpeg);
                let _ = Box::from_raw(mpeg_storage_box);
                psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
            }
            return Err("sceMpegMallocAvcEsBuf returned null".to_string());
        }
        vlog("[VIDEO] ES buffer allocated");

        // Step 9: Init AU descriptor.
        let mut au = unsafe { core::mem::zeroed::<psp::sys::SceMpegAu>() };
        let ret = unsafe { psp::sys::sceMpegInitAu(mpeg, es_buf, &mut au) };
        vlog(&format!("[VIDEO] sceMpegInitAu = {ret:#x}"));

        // Step 10: Init hardware CSC (color space converter).
        let ret = unsafe { psp::sys::sceMpegBaseCscInit(frame_width as i32) };
        vlog(&format!("[VIDEO] sceMpegBaseCscInit = {ret:#x}"));

        // Allocate output pixel buffer (frame_width stride × height × 4 ABGR).
        let out_h = ((height + 15) / 16) * 16; // round up to 16-pixel boundary
        let output_buf = vec![0u8; frame_width as usize * out_h as usize * 4];

        let muxer = PsmfMuxer::new(width as u16, height as u16);

        // Validate PSMF header via sceMpegQueryStreamOffset.
        // This confirms the firmware can parse our generated header.
        let header = muxer.peek_header();
        let mut stream_offset: i32 = 0;
        let qso_ret = unsafe {
            psp::sys::sceMpegQueryStreamOffset(
                mpeg,
                header.as_ptr() as *mut c_void,
                &mut stream_offset,
            )
        };
        vlog(&format!(
            "[VIDEO] sceMpegQueryStreamOffset = {qso_ret:#x}, offset = {stream_offset}"
        ));

        let mut stream_size: i32 = 0;
        let qss_ret = unsafe {
            psp::sys::sceMpegQueryStreamSize(
                header.as_ptr() as *mut c_void,
                &mut stream_size,
            )
        };
        vlog(&format!(
            "[VIDEO] sceMpegQueryStreamSize = {qss_ret:#x}, size = {stream_size}"
        ));

        vlog(&format!(
            "[VIDEO] SceMpegDecoder ready: {width}x{height}, \
             output buf = {} bytes",
            output_buf.len()
        ));

        Ok(Self {
            mpeg_storage,
            ringbuffer,
            mpeg_data,
            ringbuf_data,
            video_stream,
            es_buf,
            au,
            muxer,
            output_buf,
            decode_init: 0,
            width,
            height,
            frame_width,
        })
    }

    /// Get the sceMpeg handle from the storage pointer.
    fn mpeg(&self) -> psp::sys::SceMpeg {
        // SAFETY: mpeg_storage was allocated by Box and cast to *mut c_void;
        // it originally points to a *mut *mut c_void (SceMpeg layout).
        unsafe { core::mem::transmute(self.mpeg_storage) }
    }

    /// Feed PSMF packet data to the ringbuffer via the callback.
    ///
    /// Sets up the static `RINGBUF_CTX` and calls `sceMpegRingbufferPut`.
    /// Returns the number of packets actually consumed.
    fn feed_packets(&mut self, data: &[u8]) -> i32 {
        let num_packets = (data.len() / PACKET_SIZE) as i32;
        if num_packets == 0 {
            return 0;
        }

        // SAFETY: Single-threaded access; set context before Put triggers callback.
        unsafe {
            RINGBUF_CTX.ptr = data.as_ptr();
            RINGBUF_CTX.len = data.len();
            RINGBUF_CTX.offset = 0;
        }

        // Check available space in ringbuffer.
        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(&mut *self.ringbuffer)
        };
        if avail <= 0 {
            // SAFETY: Clear context.
            unsafe {
                RINGBUF_CTX.ptr = core::ptr::null();
                RINGBUF_CTX.len = 0;
            }
            return 0;
        }

        let packets_to_put = num_packets.min(avail);

        // Flush D-cache on the data to ensure DMA coherency.
        // SAFETY: data pointer and length are valid.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                data.as_ptr() as *const c_void,
                data.len() as u32,
            );
        }

        // Log immediately before Put (last chance before potential crash).
        vlog(&format!(
            "[VIDEO] sceMpegRingbufferPut: put={packets_to_put}, avail={avail}"
        ));

        // SAFETY: sceMpegRingbufferPut invokes our callback which copies data.
        let ret = unsafe {
            psp::sys::sceMpegRingbufferPut(
                &mut *self.ringbuffer,
                packets_to_put,
                avail,
            )
        };

        // SAFETY: Clear context after Put returns.
        unsafe {
            RINGBUF_CTX.ptr = core::ptr::null();
            RINGBUF_CTX.len = 0;
        }

        ret
    }

    /// Decode a single H.264 access unit (Annex B format).
    ///
    /// Feeds PSMF header (first call only), then spawns a helper thread to
    /// call `sceMpegRingbufferPut` (which blocks until the ME consumes data),
    /// while the main thread calls `sceMpegGetAvcAu` + `sceMpegAvcDecode`
    /// to kick the ME into consuming.
    fn decode(&mut self, au_data: &[u8], pts_secs: f64) -> Option<DecodedFrame> {
        if au_data.is_empty() {
            return None;
        }

        static DECODE_CALL_COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);
        let call_num = DECODE_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        let verbose = call_num < 10;

        let pts_90khz = (pts_secs * 90_000.0) as u64;
        let mpeg = self.mpeg();

        // PSMF header was already parsed by sceMpegQueryStreamOffset during
        // init. Do NOT feed it through the ringbuffer — the ringbuffer should
        // only receive MPEG-PS data (pack headers + PES packets).
        // Just consume the header packet so the muxer knows it's been "sent".
        let _ = self.muxer.take_header_packet();

        // Wrap the AU in MPEG-PS PES packets.
        let packets = self.muxer.wrap_au(au_data, pts_90khz);
        if packets.is_empty() {
            return None;
        }

        if verbose {
            vlog(&format!(
                "[VIDEO] decode#{call_num}: au={} bytes, pts={pts_secs:.2}s, \
                 wrapped into {} packets",
                au_data.len(),
                packets.len(),
            ));
        }

        // Flatten packets into a contiguous buffer for the callback.
        let total_bytes = packets.len() * PACKET_SIZE;
        let mut flat = vec![0u8; total_bytes];
        for (i, pkt) in packets.iter().enumerate() {
            flat[i * PACKET_SIZE..(i + 1) * PACKET_SIZE].copy_from_slice(pkt);
        }

        // Set up static context for the ringbuffer callback.
        unsafe {
            let ctx = &raw mut RINGBUF_CTX;
            (*ctx).ptr = flat.as_ptr();
            (*ctx).len = flat.len();
            (*ctx).offset = 0;
        }

        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(&mut *self.ringbuffer)
        };
        if avail <= 0 {
            unsafe {
                let ctx = &raw mut RINGBUF_CTX;
                (*ctx).ptr = core::ptr::null();
                (*ctx).len = 0;
            }
            return None;
        }

        let packets_to_put = (packets.len() as i32).min(avail);

        if verbose {
            vlog(&format!(
                "[VIDEO] feeding {packets_to_put} packets (avail={avail})..."
            ));
        }

        // Flush D-cache on the data.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                flat.as_ptr() as *const c_void,
                flat.len() as u32,
            );
        }

        // Call sceMpegRingbufferPut directly (no feeder thread).
        // This blocks until the callback copies all data, but since we're
        // not trying to call GetAvcAu concurrently, it should complete.
        let put_ret = unsafe {
            psp::sys::sceMpegRingbufferPut(
                &mut *self.ringbuffer,
                packets_to_put,
                avail,
            )
        };
        if verbose {
            vlog(&format!("[VIDEO] RingbufferPut ret={put_ret}"));
        }

        // Clean up callback context.
        unsafe {
            let ctx = &raw mut RINGBUF_CTX;
            (*ctx).ptr = core::ptr::null();
            (*ctx).len = 0;
        }

        // Now try GetAvcAu — data should be in the ringbuffer.
        let mut au_ret = -1i32;
        let mpeg = self.mpeg();
        for attempt in 0..10 {
            let ret = unsafe {
                psp::sys::sceMpegGetAvcAu(
                    mpeg,
                    self.video_stream,
                    &mut self.au,
                    core::ptr::null_mut(),
                )
            };
            if ret >= 0 {
                au_ret = ret;
                if verbose {
                    vlog(&format!("[VIDEO] GetAvcAu OK on attempt {attempt}"));
                }
                break;
            }
            if verbose && attempt < 3 {
                vlog(&format!("[VIDEO] GetAvcAu attempt {attempt} = {ret:#x}"));
            }
            unsafe { psp::sys::sceKernelDelayThread(1_000) };
        }

        if au_ret < 0 {
            if verbose {
                vlog("[VIDEO] GetAvcAu failed after all attempts");
            }
            return None;
        }

        // Decode the AU.
        let mut output_buf_ptr: *mut c_void = self.output_buf.as_mut_ptr() as *mut c_void;
        let buffer_addr = &mut output_buf_ptr as *mut *mut c_void as *mut c_void;
        let dec_ret = unsafe {
            psp::sys::sceMpegAvcDecode(
                mpeg,
                &mut self.au,
                self.frame_width as i32,
                buffer_addr,
                &mut self.decode_init,
            )
        };
        if verbose {
            vlog(&format!(
                "[VIDEO] AvcDecode ret={dec_ret:#x} init={} out={output_buf_ptr:?}",
                self.decode_init
            ));
        }

        if dec_ret < 0 {
            return None;
        }

        // If output is available, convert YCbCr to RGBA.
        if !output_buf_ptr.is_null() && self.decode_init != 0 {
            vlog("[VIDEO] FRAME DECODED! Converting...");
            // TODO: Read YCbCr from output_buf_ptr and convert to RGBA
            // For now, return a placeholder to prove decode works.
            return Some(DecodedFrame {
                rgba: vec![0x80; self.frame_width as usize * self.height as usize * 4],
                width: self.width,
                height: self.height,
            });
        }

        None
    }

}

impl Drop for SceMpegDecoder {
    fn drop(&mut self) {
        let mpeg = self.mpeg();
        // SAFETY: Cleanup sceMpeg resources in reverse order.
        unsafe {
            if !self.es_buf.is_null() {
                psp::sys::sceMpegFreeAvcEsBuf(mpeg, self.es_buf);
            }
            psp::sys::sceMpegUnRegistStream(mpeg, self.video_stream);
            psp::sys::sceMpegDelete(mpeg);
            psp::sys::sceMpegRingbufferDestruct(&mut *self.ringbuffer);
            // Reclaim the Box we leaked for mpeg_storage.
            let _ = Box::from_raw(self.mpeg_storage as *mut *mut c_void);
            // Don't call sceMpegFinish — allow reuse on channel switch.
        }
        vlog("[VIDEO] SceMpegDecoder dropped");
    }
}

// ---------------------------------------------------------------------------
// H.264 SPS parsing (minimal, for extracting width/height)
// ---------------------------------------------------------------------------

/// Parse width and height from an H.264 SPS NAL unit (Annex B format).
///
/// Scans the AU for a NAL type 7 (SPS) and extracts dimensions from the
/// fixed-offset fields. Returns `(width, height)` or `None`.
fn parse_sps_dimensions(au_data: &[u8]) -> Option<(u32, u32)> {
    // Find SPS NAL: 00 00 00 01 67 or 00 00 01 67 (nal_type & 0x1F == 7)
    let mut i = 0;
    while i + 4 < au_data.len() {
        let is_start = (au_data[i] == 0 && au_data[i + 1] == 0 && au_data[i + 2] == 1)
            || (au_data[i] == 0
                && au_data[i + 1] == 0
                && au_data[i + 2] == 0
                && au_data[i + 3] == 1);
        if is_start {
            let nal_offset = if au_data[i + 2] == 1 { i + 3 } else { i + 4 };
            if nal_offset < au_data.len() {
                let nal_type = au_data[nal_offset] & 0x1F;
                if nal_type == 7 {
                    // Found SPS. For TV Guide content (Baseline/Main profile,
                    // no cropping, standard resolution), dimensions are at
                    // predictable bit offsets. Use a simplified parser.
                    return parse_sps_rbsp(&au_data[nal_offset..]);
                }
            }
        }
        i += 1;
    }
    None
}

/// Simplified SPS RBSP parser for Baseline/Main profile.
///
/// Reads exp-golomb coded fields to extract pic_width_in_mbs and
/// pic_height_in_map_units, then computes pixel dimensions.
fn parse_sps_rbsp(sps: &[u8]) -> Option<(u32, u32)> {
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
            for _ in 0..count {
                let present = reader.read_bit()?;
                if present == 1 {
                    // Skip scaling list
                    let size = if count < 6 { 16 } else { 64 };
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
    let _max_ref_frames = reader.read_ue()?;
    // gaps_in_frame_num_allowed
    reader.skip(1)?;

    // pic_width_in_mbs_minus1
    let pic_width_mbs = reader.read_ue()? + 1;
    // pic_height_in_map_units_minus1
    let pic_height_map_units = reader.read_ue()? + 1;

    let width = pic_width_mbs * 16;
    let height = pic_height_map_units * 16;

    Some((width, height))
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

    // Get video dimensions from the first video sample's SPS.
    // Default to 480x272 if SPS parsing fails.
    let (vid_w, vid_h) = (480u32, 272u32);

    // Attempt H.264 hardware decoder init.
    vlog("[VIDEO] play_mp4: attempting sceMpeg init...");
    let mut h264 = match SceMpegDecoder::try_init(vid_w, vid_h) {
        Ok(dec) => {
            vlog("[VIDEO] play_mp4: sceMpeg decoder initialized");
            Some(dec)
        },
        Err(e) => {
            vlog(&format!(
                "[VIDEO] play_mp4: sceMpeg disabled ({e}), audio-only"
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

                    // Decode H.264 via sceMpeg if available.
                    if let Some(ref mut decoder) = h264 {
                        if let Some(frame) = decoder.decode(
                            &sample.data,
                            sample.timestamp_secs,
                        ) {
                            decode_count += 1;

                            // Frame pacing: wait until the frame's PTS.
                            let pts_us = (sample.timestamp_secs * 1_000_000.0) as u64;
                            let now_us =
                                unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
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
/// and decode them via sceMpeg.
///
/// Returns `true` if Shutdown was received (caller should exit thread).
fn play_stream() -> bool {
    vlog("[VIDEO] play_stream: starting streaming decode");

    // Drain stale commands that may have been queued during moov buffering.
    while let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
        if matches!(cmd, VideoCmd::Shutdown) {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            vlog("[VIDEO] play_stream: shutdown during drain");
            return true;
        }
        vlog("[VIDEO] play_stream: drained stale command");
    }

    // We need the first keyframe to extract SPS and get video dimensions
    // before initializing the decoder. Wait for it.
    vlog("[VIDEO] play_stream: waiting for first keyframe...");
    let mut first_frame: Option<StreamFrame> = None;

    for _ in 0..500 {
        // ~5 seconds timeout (500 × 10ms)
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            if matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown) {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                send_audio_cmd(AudioCmd::VideoAudioStop);
                return matches!(cmd, VideoCmd::Shutdown);
            }
        }

        if let Some(frame) = VIDEO_STREAM_QUEUE.pop() {
            if frame.is_keyframe {
                first_frame = Some(frame);
                break;
            }
            // Skip non-keyframes before decoder init.
        }

        // SAFETY: Sleep while waiting for data.
        unsafe { psp::sys::sceKernelDelayThread(10_000) };
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
    let (vid_w, vid_h) = parse_sps_dimensions(&first_frame.data)
        .unwrap_or((480, 272));
    vlog(&format!(
        "[VIDEO] play_stream: SPS dimensions = {vid_w}x{vid_h}"
    ));

    // NAL-based decode (cooleyes/PMPlayer approach).
    let mut nal_dec = match NalDecoder::try_init(&first_frame) {
        Ok(dec) => {
            vlog("[VIDEO] NAL decoder initialized OK");
            dec
        },
        Err(e) => {
            vlog(&format!("[VIDEO] NAL decoder failed: {e}, audio-only"));
            return drain_stream_only();
        },
    };

    // Decode the first keyframe.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
    let mut decode_count = 0u32;

    if let Some(decoded) = nal_dec.decode(
        &first_frame.data, first_frame.timestamp_secs,
        first_frame.raw_avcc.as_deref(), first_frame.nal_prefix_size,
    ) {
        decode_count += 1;
        let _ = VIDEO_FRAME_QUEUE.push(decoded);
        vlog("[VIDEO] play_stream: first frame decoded!");
    }

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
                if let Some(decoded) = nal_dec.decode(
                    &frame.data, frame.timestamp_secs,
                    frame.raw_avcc.as_deref(), frame.nal_prefix_size,
                ) {
                    decode_count += 1;

                    // Frame pacing via PTS.
                    let pts_us = (frame.timestamp_secs * 1_000_000.0) as u64;
                    let now_us =
                        unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
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

/// Drain the stream queue without decoding (audio-only fallback).
/// Keeps the thread alive to handle Stop/Shutdown commands.
fn drain_stream_only() -> bool {
    loop {
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    return matches!(cmd, VideoCmd::Shutdown);
                },
                _ => {},
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            return false;
        }

        // Drain frames to prevent queue backup.
        while VIDEO_STREAM_QUEUE.pop().is_some() {}

        // SAFETY: sceKernelDelayThread sleeps the current thread.
        unsafe { psp::sys::sceKernelDelayThread(50_000) };
    }
}
