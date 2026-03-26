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

use core::ffi::c_void;
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

/// Pre-load AV modules during init.
fn load_vsh_mpeg_module() {
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
    decoder: psp::mpeg::AvcDecoder,
    sps: Vec<u8>,
    pps: Vec<u8>,
    nal_prefix_size: i32,
    first_frame: bool,
}

impl NalDecoder {
    /// Initialize the NAL decoder from the first keyframe.
    fn try_init(first_frame: &StreamFrame) -> Result<Self, String> {
        vlog("[VIDEO] NalDecoder::try_init");
        crate::audio::load_av_modules_once_pub();
        load_mpeg_vsh_module();

        // Prefer SPS/PPS from MP4 avcC atom (exact match for ME expectations).
        let (sps, pps) = if let (Some(s), Some(p)) =
            (&first_frame.avcc_sps, &first_frame.avcc_pps)
        {
            (s.clone(), p.clone())
        } else {
            extract_sps_pps(&first_frame.data)
                .ok_or_else(|| "no SPS/PPS found".to_string())?
        };
        if sps.len() >= 4 {
            vlog(&format!(
                "[VIDEO] NAL: SPS={} PPS={} profile={:#x} level={:#x}",
                sps.len(), pps.len(), sps[1], sps[3]
            ));
        }

        let (width, height) = parse_sps_dimensions(&first_frame.data)
            .unwrap_or((480, 272));
        vlog(&format!("[VIDEO] NAL: {width}x{height}"));

        let decoder = psp::mpeg::AvcDecoder::new(width, height)
            .map_err(|e| format!("AvcDecoder::new: {e}"))?;
        vlog(&format!(
            "[VIDEO] NAL: decoder ready, ddr={:#x}",
            decoder.ddr_top()
        ));

        Ok(Self {
            decoder,
            sps,
            pps,
            nal_prefix_size: first_frame.nal_prefix_size as i32,
            first_frame: true,
        })
    }

    /// Decode one H.264 access unit.
    fn decode(
        &mut self, au_data: &[u8], _pts_secs: f64,
        raw_avcc: Option<&[u8]>, avcc_prefix: u8,
    ) -> Option<DecodedFrame> {
        if au_data.is_empty() {
            return None;
        }

        static DECODE_COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);
        let call_num = DECODE_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let verbose = call_num < 10;

        let (data, prefix) = if let Some(avcc) = raw_avcc {
            (avcc, avcc_prefix as i32)
        } else {
            (au_data, 0)
        };

        let is_first = self.first_frame;
        self.first_frame = false;

        let nal = psp::mpeg::AvcNal {
            sps: &self.sps,
            pps: &self.pps,
            data,
            prefix_size: prefix,
            is_first_frame: is_first,
        };

        match self.decoder.decode(&nal) {
            Ok(Some(frame)) => {
                if verbose {
                    vlog("[VIDEO] NAL: FRAME DECODED!");
                }
                Some(DecodedFrame {
                    rgba: frame.pixels,
                    width: frame.width,
                    height: frame.height,
                })
            }
            Ok(None) => {
                if verbose {
                    vlog("[VIDEO] NAL: no picture yet (reordering)");
                }
                None
            }
            Err(e) => {
                if verbose || call_num < 50 {
                    vlog(&format!("[VIDEO] NAL: decode error: {e}"));
                }
                None
            }
        }
    }
}

/// Extract SPS and PPS NAL units from an Annex B H.264 stream.
fn extract_sps_pps(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut sps = None;
    let mut pps = None;
    let mut i = 0;
    while i + 4 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            let (sc_len, nal_start) = if data[i + 2] == 1 {
                (3, i + 3)
            } else if data[i + 2] == 0 && i + 3 < data.len() && data[i + 3] == 1 {
                (4, i + 4)
            } else {
                i += 1;
                continue;
            };
            if nal_start >= data.len() { break; }
            let nal_type = data[nal_start] & 0x1F;
            let mut end = nal_start + 1;
            while end + 3 < data.len() {
                if data[end] == 0 && data[end + 1] == 0
                    && (data[end + 2] == 1
                        || (data[end + 2] == 0 && end + 3 < data.len() && data[end + 3] == 1))
                {
                    break;
                }
                end += 1;
            }
            if end + 3 >= data.len() { end = data.len(); }
            let nal_data = &data[nal_start..end];
            match nal_type {
                7 => sps = Some(nal_data.to_vec()),
                8 => pps = Some(nal_data.to_vec()),
                _ => {}
            }
            if sps.is_some() && pps.is_some() { break; }
            i = end;
        } else {
            i += 1;
        }
    }
    match (sps, pps) {
        (Some(s), Some(p)) => Some((s, p)),
        _ => None,
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
