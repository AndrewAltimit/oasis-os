//! Background worker threads (audio + I/O on separate threads).
//!
//! Uses `psp::thread::ThreadBuilder` for native PSP kernel threads with
//! priority tuning. Communication uses lock-free `SpscQueue` for commands
//! and bare atomics for shared state (lock-free to avoid priority inversion
//! on single-core PSP where a high-priority audio thread could starve the
//! main thread if both contend on a spinlock).

mod audio;
mod io_handlers;
mod radio;
mod tls_http;
mod video_dl_http;
mod video_dl_parse;
mod video_download;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;

use crate::sfx::SfxId;

// ---------------------------------------------------------------------------
// Lock-free command and response queues (SPSC: main thread -> workers)
// ---------------------------------------------------------------------------

/// Audio command queue: main thread pushes, audio thread pops.
/// 64 slots buffer ~1.5 seconds of AAC at 44.1kHz (23ms per frame),
/// absorbing network I/O jitter that causes audio stuttering.
static AUDIO_QUEUE: SpscQueue<AudioCmd, 64> = SpscQueue::new();
/// I/O command queue: main thread pushes, I/O thread pops.
static IO_CMD_QUEUE: SpscQueue<IoCmd, 16> = SpscQueue::new();
/// I/O response queue: I/O thread pushes, main thread pops.
static IO_RESP_QUEUE: SpscQueue<IoResponse, 16> = SpscQueue::new();

// ---------------------------------------------------------------------------
// Shared audio state (lock-free atomics -- no priority inversion)
// ---------------------------------------------------------------------------

static AUDIO_PLAYING: AtomicBool = AtomicBool::new(false);
static AUDIO_PAUSED: AtomicBool = AtomicBool::new(false);
static AUDIO_SAMPLE_RATE: AtomicU32 = AtomicU32::new(0);
static AUDIO_BITRATE: AtomicU32 = AtomicU32::new(0);
static AUDIO_CHANNELS: AtomicU32 = AtomicU32::new(0);
// Position/duration stored as u32 milliseconds (max ~49 days, plenty).
static AUDIO_POSITION_MS: AtomicU32 = AtomicU32::new(0);
static AUDIO_DURATION_MS: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Radio streaming state (atomics: audio thread -> main thread)
// ---------------------------------------------------------------------------

static RADIO_STREAMING: AtomicBool = AtomicBool::new(false);
static RADIO_BUFFERING: AtomicBool = AtomicBool::new(false);
/// ICY metadata titles from audio thread -> main thread.
static RADIO_META_QUEUE: SpscQueue<String, 4> = SpscQueue::new();

/// Raw MP3 byte chunks: I/O thread pushes (after fetching from archive.org
/// over HTTPS), audio thread pops (feeds `RadioStreamer`'s decoder).
/// 8 slots × ~32 KB = ~256 KB ring buffer, enough to absorb HTTPS jitter
/// while a 192 kbps MP3 plays at ~24 KB/s.
pub(crate) static RADIO_DATA_QUEUE: SpscQueue<Vec<u8>, 8> = SpscQueue::new();

/// Set by the main thread (Stop press) to signal the I/O thread to stop
/// streaming the archive MP3. Checked between chunks.
pub(crate) static RADIO_CANCEL: AtomicBool = AtomicBool::new(false);

/// Set by the I/O thread when its archive streaming exits (success, EOF,
/// error, or cancel), so a new RadioArchive command can safely begin.
pub(crate) static RADIO_STOPPED: AtomicBool = AtomicBool::new(true);

/// True when the audio thread is NOT currently popping `RADIO_DATA_QUEUE`.
/// `RADIO_DATA_QUEUE` is an SPSC queue (single-consumer contract): if the
/// I/O thread drains it for a new stream while the audio thread is still
/// popping the previous stream, two concurrent `pop()` calls on a
/// lock-free SPSC is UB. The I/O thread waits for this flag before
/// draining, and the audio thread clears/sets it around its consumer
/// lifetime.
pub(crate) static RADIO_AUDIO_IDLE: AtomicBool = AtomicBool::new(true);

/// Request cancellation of the current radio archive stream.
pub fn cancel_radio_stream() {
    RADIO_CANCEL.store(true, Ordering::Release);
}

/// Check if the I/O radio archive worker has fully stopped.
pub fn is_radio_stopped() -> bool {
    RADIO_STOPPED.load(Ordering::Acquire)
}

/// Set `RADIO_STREAMING` and `RADIO_BUFFERING` atomically as soon as the
/// main thread forwards a `RadioConnected` response to the audio thread.
/// Without this pre-set, the main loop's `if Buffering && !is_streaming →
/// Stopped` check can race the audio thread's `AudioCmd` handler and
/// bounce status back to Stopped before any audio plays.
pub fn mark_radio_starting() {
    RADIO_STREAMING.store(true, Ordering::Release);
    RADIO_BUFFERING.store(true, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Video download cancellation flag
// ---------------------------------------------------------------------------

/// Set by main thread when the user cancels a download (Circle press).
/// Checked by I/O thread during moov buffering and streaming to abort early.
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

/// Set by the I/O thread when it has fully exited handle_video_download()
/// and cleaned up all HTTP/TLS resources. The video thread waits for this
/// before dropping the decoder to avoid race conditions.
static DOWNLOAD_STOPPED: AtomicBool = AtomicBool::new(true);

/// Request cancellation of the current video download.
pub fn cancel_video_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::Release);
}

/// Check if the I/O download thread has fully stopped.
pub fn is_download_stopped() -> bool {
    DOWNLOAD_STOPPED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Streaming log suppression
// ---------------------------------------------------------------------------

/// Set to `true` during active audio/video streaming. When active,
/// `io_log_verbose()` calls are suppressed to avoid Memory Stick I/O
/// stalls (each `sceIoOpen+Write+Close` costs ~5-20ms).
static STREAMING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mark streaming as active (suppress verbose I/O logging).
pub fn set_streaming_active(active: bool) {
    STREAMING_ACTIVE.store(active, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Audio commands
// ---------------------------------------------------------------------------

/// Commands for the dedicated audio thread.
pub enum AudioCmd {
    LoadAndPlay(String),
    LoadAndPlayData(Vec<u8>),
    Pause,
    Resume,
    Stop,
    SetVolume(u8),
    PlaySfx(SfxId),
    /// Start radio streaming from a connected socket fd.
    RadioStreamFromFd {
        fd: i32,
        icy_metaint: usize,
        initial_data: Vec<u8>,
    },
    /// Stop radio streaming and close the socket.
    RadioStop,
    /// Video audio PCM data from the video decode thread.
    VideoAudioData {
        pcm_i16: Vec<i16>,
        sample_rate: u32,
        channels: u16,
    },
    /// Configure AAC decoder with track parameters (send before first frame).
    VideoAudioAacConfig {
        sample_rate: u32,
        channels: u16,
    },
    /// Raw AAC frame data from demux_lite for hardware decode.
    VideoAudioAac {
        data: Vec<u8>,
    },
    /// Stop video audio playback.
    VideoAudioStop,
    Shutdown,
}

/// Handle to the background audio thread (reads shared atomics).
pub struct AudioHandle;

impl AudioHandle {
    /// Send a command to the audio thread.
    pub fn send(&self, cmd: AudioCmd) {
        let _ = AUDIO_QUEUE.push(cmd);
    }

    pub fn is_playing(&self) -> bool {
        AUDIO_PLAYING.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        AUDIO_PAUSED.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        AUDIO_SAMPLE_RATE.load(Ordering::Relaxed)
    }

    pub fn bitrate(&self) -> u32 {
        AUDIO_BITRATE.load(Ordering::Relaxed)
    }

    pub fn channels(&self) -> u32 {
        AUDIO_CHANNELS.load(Ordering::Relaxed)
    }

    pub fn position_ms(&self) -> u64 {
        AUDIO_POSITION_MS.load(Ordering::Relaxed) as u64
    }

    pub fn duration_ms(&self) -> u64 {
        AUDIO_DURATION_MS.load(Ordering::Relaxed) as u64
    }

    pub fn is_radio_streaming(&self) -> bool {
        RADIO_STREAMING.load(Ordering::Relaxed)
    }

    pub fn is_radio_buffering(&self) -> bool {
        RADIO_BUFFERING.load(Ordering::Relaxed)
    }

    /// Poll for the latest ICY metadata title (non-blocking).
    pub fn poll_radio_meta(&self) -> Option<String> {
        RADIO_META_QUEUE.pop()
    }
}

/// Send an audio command from any context.
pub fn send_audio_cmd(cmd: AudioCmd) {
    let _ = AUDIO_QUEUE.push(cmd);
}

// ---------------------------------------------------------------------------
// I/O commands and responses
// ---------------------------------------------------------------------------

/// A single TV catalog fetch request (part of a batch).
pub struct TvCatalogRequest {
    pub url: String,
    pub ch_idx: usize,
    pub item_id: String,
    pub subfolder: Option<String>,
}

/// Commands for the dedicated I/O thread.
pub enum IoCmd {
    LoadTexture {
        path: String,
        max_w: i32,
        max_h: i32,
    },
    ReadFile {
        path: String,
    },
    HttpGet {
        url: String,
        tag: u32,
    },
    /// Connect to an internet radio stream (raw TCP + HTTP).
    RadioConnect {
        url: String,
    },
    /// Resolve and stream an Internet Archive collection over HTTPS.
    /// Mirrors the desktop archive flow: search → first item → first MP3 →
    /// streaming HTTPS GET. Pushes raw MP3 bytes into `RADIO_DATA_QUEUE`.
    RadioArchive {
        collection: String,
    },
    /// Fetch and parse TV Guide catalogs from archive.org (I/O thread).
    /// Batched to reuse a single HttpClient for all requests.
    TvCatalogFetchBatch {
        requests: Vec<TvCatalogRequest>,
    },
    /// Download a video file to Memory Stick for TV Guide playback.
    VideoDownload {
        url: String,
        dest: String,
        tag: u32,
    },
    Shutdown,
}

/// Responses from the I/O thread.
pub enum IoResponse {
    TextureReady {
        path: String,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    FileReady {
        path: String,
        data: Vec<u8>,
    },
    HttpDone {
        tag: u32,
        status_code: u16,
        body: Vec<u8>,
    },
    /// Radio socket connected and HTTP headers parsed.
    RadioConnected {
        fd: i32,
        icy_metaint: usize,
        initial_data: Vec<u8>,
    },
    /// Radio connection failed.
    RadioError {
        msg: String,
    },
    /// Video download progress update.
    VideoProgress {
        tag: u32,
        bytes: u64,
        total: Option<u64>,
    },
    /// Video file downloaded successfully.
    VideoReady {
        tag: u32,
        path: String,
    },
    /// Video stream moov atom downloaded — playback can begin while download
    /// continues in the background.
    VideoStreamReady {
        tag: u32,
        path: String,
        content_length: u32,
    },
    /// Video download failed.
    VideoError {
        tag: u32,
        msg: String,
    },
    /// TV Guide catalog parsed on the I/O thread.
    TvCatalogReady {
        ch_idx: usize,
        episodes: Vec<oasis_core::apps::tv_guide::VideoEpisode>,
    },
    Error {
        path: String,
        msg: String,
    },
}

/// Handle to the I/O thread's response queue.
pub struct IoHandle;

impl IoHandle {
    /// Send a command to the I/O thread.
    pub fn send(&self, cmd: IoCmd) {
        let _ = IO_CMD_QUEUE.push(cmd);
    }

    /// Try to receive an I/O response (non-blocking).
    pub fn try_recv(&self) -> Option<IoResponse> {
        IO_RESP_QUEUE.pop()
    }
}

// ---------------------------------------------------------------------------
// Thread spawning
// ---------------------------------------------------------------------------

/// Spawn the background audio and I/O threads.
///
/// Returns handles for audio state and I/O responses. The `JoinHandle`s
/// are leaked intentionally — the worker threads run for the lifetime of
/// the process, and dropping a `JoinHandle` terminates its thread.
pub fn spawn_workers() -> (AudioHandle, IoHandle) {
    // Audio thread: high priority (16) for low-latency playback.
    if let Ok(handle) = ThreadBuilder::new(b"oasis_audio\0")
        .priority(16)
        .spawn(move || {
            audio::audio_thread_fn();
            0
        })
    {
        // Leak the JoinHandle so the thread isn't killed on drop.
        core::mem::forget(handle);
    }

    // I/O thread: normal priority (32) for file operations.
    if let Ok(handle) = ThreadBuilder::new(b"oasis_io\0")
        .priority(32)
        .stack_size(512 * 1024) // 512KB for moov parsing + TLS handshake
        .spawn(move || {
            io_handlers::io_thread_fn();
            0
        })
    {
        core::mem::forget(handle);
    }

    (AudioHandle, IoHandle)
}

// ---------------------------------------------------------------------------
// Shared utility functions
// ---------------------------------------------------------------------------

/// Log from I/O thread (raw sceIo — safe from any thread).
fn io_log(msg: &str) {
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

/// Verbose log — suppressed during active streaming to avoid Memory Stick
/// I/O stalls (~5-20ms per `sceIoOpen+Write+Close`). Errors should still
/// use `io_log()` directly.
fn io_log_verbose(msg: &str) {
    if !STREAMING_ACTIVE.load(Ordering::Relaxed) {
        io_log(msg);
    }
}

/// Find `\r\n\r\n` in a byte slice, return offset past it.
fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == b'\r' && data[i + 1] == b'\n' && data[i + 2] == b'\r' && data[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

/// Parse an HTTP URL into (host, port, path).
fn parse_radio_url(url: &str) -> Option<(String, u16, String)> {
    let stripped = url.strip_prefix("http://")?;
    let (host_port, path) = match stripped.find('/') {
        Some(i) => (&stripped[..i], &stripped[i..]),
        None => (stripped, "/"),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i + 1..].parse::<u16>().ok()?),
        None => (host_port, 80),
    };
    Some((host.to_string(), port, path.to_string()))
}

/// Parse a URL into (host, port, path, is_https).
fn parse_url(url: &str) -> Option<(String, u16, String, bool)> {
    let (stripped, is_https) = if let Some(s) = url.strip_prefix("https://") {
        (s, true)
    } else if let Some(s) = url.strip_prefix("http://") {
        (s, false)
    } else {
        return None;
    };
    let (host_port, path) = match stripped.find('/') {
        Some(i) => (&stripped[..i], &stripped[i..]),
        None => (stripped, "/"),
    };
    let default_port = if is_https { 443 } else { 80 };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i + 1..].parse::<u16>().ok()?),
        None => (host_port, default_port),
    };
    Some((host.to_string(), port, path.to_string(), is_https))
}

/// Parse `icy-metaint:` value from HTTP response headers.
fn parse_icy_metaint(headers: &str) -> usize {
    for line in headers.split('\n') {
        let lower: String = line.chars().map(|c| c.to_ascii_lowercase()).collect();
        if let Some(rest) = lower.strip_prefix("icy-metaint:") {
            if let Ok(v) = rest.trim().parse::<usize>() {
                return v;
            }
        }
    }
    0
}
