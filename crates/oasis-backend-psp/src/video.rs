//! Video decode thread for TV Guide playback.
//!
//! Provides command queues and a background thread for future video decoding.
//! Currently audio-only: the downloaded MP4 is acknowledged but frame decode
//! requires a PSP-compatible demuxer (symphonia pulls in `std::sync::Once`
//! which uses syscalls that PPSSPP's HLE doesn't implement).

use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;

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
// Thread function
// ---------------------------------------------------------------------------

fn video_thread_fn() {
    loop {
        match VIDEO_CMD_QUEUE.pop() {
            Some(VideoCmd::Play { path, seek_secs: _ }) => {
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                // TODO: Implement PSP-native MP4 demux + decode.
                // symphonia/oasis-video cannot be used here because it pulls in
                // std::sync::Once (via lazy_static) which triggers unimplemented
                // PSP syscalls in PPSSPP. A future approach:
                //   1. Minimal no_std MP4 parser (moov/mdat extraction)
                //   2. PSP Media Engine hardware decode (sceVideocodec/sceAudiocodec)
                //   3. Or: pre-transcode to raw PCM on download
                //
                // For now, log that we received the play command.
                let _ = &path; // suppress unused warning
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
            },
            Some(VideoCmd::Stop) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
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
