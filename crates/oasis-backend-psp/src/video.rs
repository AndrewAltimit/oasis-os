//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once — PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video frames are logged (no H.264 decode yet — see Step 6 stubs).

use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;

use crate::threading::{AudioCmd, send_audio_cmd};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Commands for the video decode thread.
pub enum VideoCmd {
    /// Start decoding a downloaded MP4 file.
    Play { path: String, seek_secs: u64 },
    /// Stop current playback.
    Stop,
    /// Shut down the thread.
    Shutdown,
}

/// A decoded video frame ready for texture upload.
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
/// Whether video is currently playing.
static VIDEO_PLAYING: AtomicBool = AtomicBool::new(false);

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
        let fd = unsafe {
            psp::sys::sceIoOpen(path_bytes.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
        };
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
        let n = unsafe {
            psp::sys::sceIoRead(self.fd, buf.as_mut_ptr() as *mut _, buf.len() as u32)
        };
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
// H.264 video decoder stub (sceVideocodec)
// ---------------------------------------------------------------------------

/// 64-byte-aligned codec buffer required by the PSP Media Engine.
///
/// `sceVideocodec*` APIs require a 64-byte-aligned buffer of at least
/// 65 `u32` words. `#[repr(align(64))]` lets the Rust compiler guarantee
/// alignment without manual pointer arithmetic.
#[repr(align(64))]
struct CodecBuf([u32; 65]);

/// PSP hardware H.264 video decoder using the Media Engine.
///
/// Uses `sceVideocodec*` raw APIs. The ME is not emulated in PPSSPP, so
/// `try_init()` returns `Err` when running under emulation (audio-only mode).
struct PspVideoDecoder {
    /// Codec buffer (65 words, 64-byte aligned via `CodecBuf`).
    buf: Box<CodecBuf>,
    initialized: bool,
}

impl PspVideoDecoder {
    /// Attempt to initialize the H.264 hardware decoder.
    ///
    /// Returns `Err` on PPSSPP (ME not emulated) or if the codec modules
    /// are unavailable. The caller should fall back to audio-only mode.
    fn try_init() -> Result<Self, String> {
        let mut buf = Box::new(CodecBuf([0u32; 65]));
        let ptr = buf.0.as_mut_ptr();

        // SAFETY: sceVideocodecOpen initializes the codec buffer.
        // Type 0 = H.264 / AVC. ptr is 64-byte aligned via CodecBuf.
        let ret = unsafe { psp::sys::sceVideocodecOpen(ptr, 0) };
        if ret < 0 {
            return Err(format!(
                "sceVideocodecOpen failed: {:#010x} (ME not available?)",
                ret as u32
            ));
        }

        // SAFETY: ptr is the same aligned buffer passed to Open.
        let ret = unsafe { psp::sys::sceVideocodecGetEDRAM(ptr, 0) };
        if ret < 0 {
            return Err(format!(
                "sceVideocodecGetEDRAM failed: {:#010x}",
                ret as u32
            ));
        }

        // SAFETY: ptr is the same aligned buffer passed to Open/GetEDRAM.
        let ret = unsafe { psp::sys::sceVideocodecInit(ptr, 0) };
        if ret < 0 {
            // SAFETY: Release EDRAM on init failure.
            unsafe { psp::sys::sceVideocodecReleaseEDRAM(ptr) };
            return Err(format!(
                "sceVideocodecInit failed: {:#010x}",
                ret as u32
            ));
        }

        Ok(Self {
            buf,
            initialized: true,
        })
    }

    /// Decode a single H.264 NAL unit (Annex B format).
    ///
    /// Returns the decoded YUV420 frame data on success, or `None` if the
    /// codec needs more data (buffering/reference frames).
    #[allow(dead_code)]
    fn decode(&mut self, _nal_data: &[u8]) -> Option<DecodedFrame> {
        if !self.initialized {
            return None;
        }
        // TODO: Wire up sceVideocodecDecode when running on real hardware.
        // The codec buffer fields [6]/[7]/[8]/[9]/[10] need to be set
        // with source NAL pointer/len and destination YUV buffer.
        // After decode, convert YUV420→RGBA and push to VIDEO_FRAME_QUEUE.
        None
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
// Thread function
// ---------------------------------------------------------------------------

fn video_thread_fn() {
    loop {
        match VIDEO_CMD_QUEUE.pop() {
            Some(VideoCmd::Play { path, seek_secs }) => {
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                play_mp4(&path, seek_secs);
            },
            Some(VideoCmd::Stop) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
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

/// Demux an MP4 file and feed audio samples to the audio thread.
fn play_mp4(path: &str, seek_secs: u64) {
    use oasis_video::demux_lite::Mp4Lite;

    let reader = match PspFileReader::open(path) {
        Some(r) => r,
        None => {
            psp::dprintln!("video: failed to open {path}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return;
        },
    };

    let mut mp4 = match Mp4Lite::open(reader) {
        Ok(m) => m,
        Err(e) => {
            psp::dprintln!("video: failed to parse MP4 {path}: {e}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return;
        },
    };

    // Seek if requested.
    if seek_secs > 0 {
        if let Err(e) = mp4.seek(seek_secs as f64) {
            psp::dprintln!("video: seek to {seek_secs}s failed: {e}");
        }
    }

    // Attempt H.264 hardware decoder init.
    let mut _h264 = match PspVideoDecoder::try_init() {
        Ok(dec) => {
            psp::dprintln!("video: H.264 hardware decoder initialized");
            Some(dec)
        },
        Err(e) => {
            psp::dprintln!("video: H.264 disabled ({e}), audio-only mode");
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
    let mut audio_done = mp4.audio_track_info().is_none();
    let mut video_done = mp4.video_track_info().is_none();

    loop {
        // Check for stop command.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    if matches!(cmd, VideoCmd::Shutdown) {
                        return;
                    }
                    break;
                },
                VideoCmd::Play { .. } => {
                    // Ignore nested Play commands during playback.
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

        // Read video samples (log count, no H.264 decode yet).
        if !video_done {
            match mp4.next_video_sample() {
                Ok(Some(_sample)) => {
                    video_count += 1;
                    // TODO: decode H.264 NALs via sceVideocodec.
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
        "video: stream ended — {video_count} video, {audio_count} audio samples"
    );
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
}
