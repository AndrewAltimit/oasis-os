//! Background worker threads (audio + I/O on separate threads).
//!
//! Uses `psp::thread::ThreadBuilder` for native PSP kernel threads with
//! priority tuning. Communication uses lock-free `SpscQueue` for commands
//! and bare atomics for shared state (lock-free to avoid priority inversion
//! on single-core PSP where a high-priority audio thread could starve the
//! main thread if both contend on a spinlock).

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;

use crate::audio::{AudioPlayer, RadioStreamer};
use crate::filesystem::decode_jpeg;
use crate::sfx::{SfxEngine, SfxId};


// ---------------------------------------------------------------------------
// Lock-free command and response queues (SPSC: main thread -> workers)
// ---------------------------------------------------------------------------

/// Audio command queue: main thread pushes, audio thread pops.
static AUDIO_QUEUE: SpscQueue<AudioCmd, 16> = SpscQueue::new();
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

// ---------------------------------------------------------------------------
// Video download cancellation flag
// ---------------------------------------------------------------------------

/// Set by main thread when the user cancels a download (Circle press).
/// Checked by I/O thread during moov buffering and streaming to abort early.
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

/// Request cancellation of the current video download.
pub fn cancel_video_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::Release);
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
    VideoAudioAac { data: Vec<u8> },
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
            audio_thread_fn();
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
            io_thread_fn();
            0
        })
    {
        core::mem::forget(handle);
    }

    (AudioHandle, IoHandle)
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

/// Raw PSP AAC hardware decoder using `sceAudiocodec*` syscalls directly.
///
/// Unlike the generic `AudiocodecDecoder`, this sets `buf[10] = sample_rate`
/// before `sceAudiocodecInit` (required for AAC) and does NOT overwrite it
/// during decode (which would break AAC by replacing the sample rate with
/// the source buffer length — an MP3-specific quirk).
struct PspAacDecoder {
    buf: Box<AacCodecBuf>,
    edram_allocated: bool,
}

/// 64-byte-aligned codec buffer for sceAudiocodec (65 words).
#[repr(C, align(64))]
struct AacCodecBuf {
    words: [u32; 65],
}

impl PspAacDecoder {
    /// Initialize the AAC hardware decoder with the given sample rate.
    fn init(sample_rate: u32) -> Result<Self, i32> {
        use psp::sys;

        crate::audio::load_av_modules_once_pub();

        let mut buf = Box::new(AacCodecBuf { words: [0u32; 65] });
        let ptr = buf.words.as_mut_ptr();
        let codec_type = 0x1003; // AAC

        // SAFETY: sceAudiocodec operates on the 64-byte-aligned buffer.
        // Flush cache before each codec call (DMA coherency).
        unsafe {
            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecCheckNeedMem(ptr, codec_type);
            if ret < 0 {
                io_log(&format!(
                    "[AUDIO] CheckNeedMem failed: {ret:#010x}"
                ));
                return Err(ret);
            }

            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecGetEDRAM(ptr, codec_type);
            if ret < 0 {
                io_log(&format!(
                    "[AUDIO] GetEDRAM failed: {ret:#010x}"
                ));
                return Err(ret);
            }

            // Set sample rate BEFORE init — required for AAC.
            buf.words[10] = sample_rate;

            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecInit(ptr, codec_type);
            if ret < 0 {
                io_log(&format!(
                    "[AUDIO] AudiocodecInit failed: {ret:#010x}"
                ));
                sys::sceAudiocodecReleaseEDRAM(ptr);
                return Err(ret);
            }
        }

        Ok(Self {
            buf,
            edram_allocated: true,
        })
    }

    /// Decode one raw AAC frame into PCM. Returns number of bytes consumed.
    fn decode(&mut self, src: &[u8], dst: &mut [i16]) -> Result<usize, i32> {
        use psp::sys;

        let words = &mut self.buf.words;

        // Set source and destination pointers/sizes.
        words[6] = src.as_ptr() as u32;
        words[7] = src.len() as u32;
        words[8] = dst.as_mut_ptr() as u32;
        words[9] = (dst.len() * 2) as u32; // bytes
        // Do NOT touch words[10] — it holds the sample rate set during init.

        // Flush D-cache before DMA-based codec decode to ensure the
        // hardware reads coherent data from the source buffer.
        // SAFETY: sceKernelDcacheWritebackInvalidateAll flushes all
        // cached data back to main memory.
        unsafe {
            sys::sceKernelDcacheWritebackInvalidateAll();
        }

        // SAFETY: sceAudiocodecDecode operates on the aligned buffer.
        let ret = unsafe {
            sys::sceAudiocodecDecode(words.as_mut_ptr(), 0x1003)
        };
        if ret < 0 {
            return Err(ret);
        }

        Ok(words[7] as usize)
    }
}

impl Drop for PspAacDecoder {
    fn drop(&mut self) {
        if self.edram_allocated {
            // SAFETY: Release EDRAM allocated by sceAudiocodecGetEDRAM.
            unsafe {
                psp::sys::sceAudiocodecReleaseEDRAM(self.buf.words.as_mut_ptr());
            }
        }
    }
}

/// Decode a raw AAC frame via PSP hardware codec and output PCM.
fn decode_aac_frame(
    data: &[u8],
    player: &mut AudioPlayer,
    aac_decoder: &mut Option<PspAacDecoder>,
    aac_sample_rate: u32,
) {
    use psp::audio::{AudioChannel, AudioFormat};

    // AAC: 1024 samples per frame, stereo = 2048 i16.
    const AAC_FRAME_SAMPLES: i32 = 1024;

    if aac_sample_rate == 0 {
        // Config not received yet — drop frame silently.
        return;
    }

    // Lazily create AAC decoder (only once, don't retry on failure).
    if aac_decoder.is_none() {
        io_log(&format!(
            "[AUDIO] creating AAC decoder (rate={aac_sample_rate})..."
        ));
        match PspAacDecoder::init(aac_sample_rate) {
            Ok(dec) => {
                io_log("[AUDIO] AAC decoder init OK");
                *aac_decoder = Some(dec);
            },
            Err(e) => {
                io_log(&format!(
                    "[AUDIO] AAC decoder init failed: {e:#010x}"
                ));
                return;
            },
        }
    }

    // Ensure audio channel exists.
    if player.channel.is_none() {
        io_log("[AUDIO] reserving audio channel...");
        player.channel =
            AudioChannel::reserve(AAC_FRAME_SAMPLES, AudioFormat::Stereo).ok();
        if player.channel.is_some() {
            io_log("[AUDIO] audio channel reserved OK");
        } else {
            io_log("[AUDIO] audio channel reserve FAILED");
        }
    }

    let decoder = aac_decoder.as_mut().unwrap();

    let mut pcm = vec![0i16; AAC_FRAME_SAMPLES as usize * 2];

    static DECODE_COUNT: core::sync::atomic::AtomicU32 =
        core::sync::atomic::AtomicU32::new(0);
    let count = DECODE_COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 3 {
        io_log(&format!(
            "[AUDIO] decode #{count} src_len={} src_ptr={:#x}",
            data.len(),
            data.as_ptr() as u32,
        ));
    }

    match decoder.decode(data, &mut pcm) {
        Ok(consumed) => {
            if count < 3 {
                io_log(&format!(
                    "[AUDIO] decode #{count} OK, consumed={consumed}"
                ));
            }
            if consumed == 0 {
                return;
            }
            if let Some(channel) = &player.channel {
                let _ = channel.output_blocking(0x8000, &pcm);
            }
        },
        Err(e) => {
            static ERR_COUNT: core::sync::atomic::AtomicU32 =
                core::sync::atomic::AtomicU32::new(0);
            let c = ERR_COUNT.fetch_add(1, Ordering::Relaxed);
            if c < 10 {
                io_log(&format!(
                    "[AUDIO] AAC decode #{count} error: {e:#010x}"
                ));
            }
        },
    }
}

/// Dedicated audio thread: MP3 playback + SFX mixing + radio streaming.
fn audio_thread_fn() {
    let mut player = AudioPlayer::new();
    player.init();

    let mut sfx = SfxEngine::new();
    let mut radio: Option<RadioStreamer> = None;
    let mut aac_decoder: Option<PspAacDecoder> = None;
    let mut aac_sample_rate: u32 = 0;

    loop {
        match AUDIO_QUEUE.pop() {
            Some(AudioCmd::LoadAndPlay(path)) => {
                // Stop radio if active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.load_and_play(&path) {
                    publish_audio_state(&player);
                } else {
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::LoadAndPlayData(data)) => {
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.load_and_play_owned(data) {
                    publish_audio_state(&player);
                } else {
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Pause) => {
                if player.is_playing() && !player.is_paused() {
                    player.toggle_pause();
                    AUDIO_PAUSED.store(true, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Resume) => {
                if player.is_playing() && player.is_paused() {
                    player.toggle_pause();
                    AUDIO_PAUSED.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Stop) => {
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                AUDIO_PAUSED.store(false, Ordering::Relaxed);
                // Also stop radio if active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::SetVolume(v)) => {
                player.set_volume(v);
                if let Some(r) = &mut radio {
                    r.set_volume(v);
                }
            },
            Some(AudioCmd::PlaySfx(id)) => {
                if let Some(sfx) = &sfx {
                    sfx.play(id);
                }
            },
            Some(AudioCmd::RadioStreamFromFd {
                fd,
                icy_metaint,
                initial_data,
            }) => {
                // Stop file player first.
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                AUDIO_PAUSED.store(false, Ordering::Relaxed);
                // Stop any existing radio stream.
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                // Create new radio streamer with any leftover header data.
                let mut streamer = RadioStreamer::new(fd, icy_metaint);
                if !initial_data.is_empty() {
                    streamer.seed_buffer(&initial_data);
                }
                RADIO_BUFFERING.store(true, Ordering::Relaxed);
                RADIO_STREAMING.store(true, Ordering::Relaxed);
                radio = Some(streamer);
            },
            Some(AudioCmd::RadioStop) => {
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            },
            Some(AudioCmd::VideoAudioData {
                pcm_i16,
                sample_rate: _,
                channels: _,
            }) => {
                // Stop radio/file playback if still active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.is_playing() {
                    player.stop();
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                    AUDIO_PAUSED.store(false, Ordering::Relaxed);
                }
                // Output PCM directly to the hardware audio channel.
                player.output_video_pcm(&pcm_i16);
            },
            Some(AudioCmd::VideoAudioAacConfig {
                sample_rate,
                channels: _,
            }) => {
                // Store config for lazy decoder init.
                aac_sample_rate = sample_rate;
                // Reset decoder if sample rate changed.
                aac_decoder = None;
                io_log(&format!(
                    "[AUDIO] AAC config: rate={sample_rate}"
                ));
            },
            Some(AudioCmd::VideoAudioAac { data }) => {
                // Decode raw AAC frame via sceAudiocodec and output PCM.
                decode_aac_frame(
                    &data,
                    &mut player,
                    &mut aac_decoder,
                    aac_sample_rate,
                );
            },
            Some(AudioCmd::VideoAudioStop) => {
                // Video playback ended -- flush AAC decoder state.
                aac_decoder = None;
                aac_sample_rate = 0;
            },
            Some(AudioCmd::Shutdown) => {
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
                break;
            },
            None => {},
        }

        if player.is_playing() && !player.is_paused() {
            player.update();
            AUDIO_POSITION_MS.store(player.position_ms() as u32, Ordering::Relaxed);
            AUDIO_DURATION_MS.store(player.duration_ms() as u32, Ordering::Relaxed);
            if !player.is_playing() {
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
            }
        } else if let Some(r) = &mut radio {
            // Radio streaming: recv data and decode.
            r.recv_data();
            if r.buffering && r.buf_valid >= RadioStreamer::BUFFER_THRESHOLD {
                r.buffering = false;
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            }
            if !r.buffering {
                r.update(&mut player);
                // Push ICY metadata to main thread.
                if let Some(title) = r.take_meta() {
                    let _ = RADIO_META_QUEUE.push(title);
                }
            }
            if r.is_error() {
                let _ = RADIO_META_QUEUE.push(String::from("[Stream error]"));
                radio = None;
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            }
        } else {
            // Sleep when idle. During AAC playback the audio thread must
            // wake frequently to pop frames, but a short sleep (1ms)
            // prevents a CPU-burning busy loop that can crash the PSP.
            let sleep_us = if aac_sample_rate > 0 { 1_000 } else { 10_000 };
            // SAFETY: sceKernelDelayThread sleeps the current thread.
            unsafe { psp::sys::sceKernelDelayThread(sleep_us) };
        }

        // Pump SFX mixer (separate hardware channel, short blocking).
        if let Some(sfx) = &mut sfx {
            sfx.pump();
        }
    }
}

/// Publish audio player state to shared atomics after a load_and_play.
fn publish_audio_state(player: &AudioPlayer) {
    AUDIO_SAMPLE_RATE.store(player.sample_rate, Ordering::Relaxed);
    AUDIO_BITRATE.store(player.bitrate, Ordering::Relaxed);
    AUDIO_CHANNELS.store(player.channels, Ordering::Relaxed);
    AUDIO_POSITION_MS.store(0, Ordering::Relaxed);
    AUDIO_DURATION_MS.store(0, Ordering::Relaxed);
    AUDIO_PAUSED.store(false, Ordering::Relaxed);
    // Set playing LAST so readers see consistent metadata first.
    AUDIO_PLAYING.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// I/O thread
// ---------------------------------------------------------------------------

/// Dedicated I/O thread: file reads, JPEG decoding, and radio connections.
fn io_thread_fn() {
    loop {
        match IO_CMD_QUEUE.pop() {
            Some(IoCmd::LoadTexture { path, max_w, max_h }) => {
                handle_load_texture(path, max_w, max_h);
            },
            Some(IoCmd::ReadFile { path }) => {
                handle_read_file(path);
            },
            Some(IoCmd::HttpGet { url, tag }) => {
                handle_http_get(url, tag);
            },
            Some(IoCmd::RadioConnect { url }) => {
                handle_radio_connect(url);
            },
            Some(IoCmd::TvCatalogFetchBatch { requests }) => {
                handle_tv_catalog_batch(requests);
            },
            Some(IoCmd::VideoDownload { url, dest, tag }) => {
                handle_video_download(url, dest, tag);
            },
            Some(IoCmd::Shutdown) => break,
            None => {
                // Sleep when idle to avoid spinning.
                psp::thread::sleep_ms(10);
            },
        }
    }
}

fn handle_load_texture(path: String, max_w: i32, max_h: i32) {
    match psp::io::read_to_vec(&path) {
        Ok(data) => match decode_jpeg(&data, max_w, max_h) {
            Some((w, h, rgba)) => {
                let _ = IO_RESP_QUEUE.push(IoResponse::TextureReady {
                    path,
                    width: w,
                    height: h,
                    rgba,
                });
            },
            None => {
                let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                    path,
                    msg: "JPEG decode failed".into(),
                });
            },
        },
        Err(_) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: "file read failed".into(),
            });
        },
    }
}

fn handle_read_file(path: String) {
    match psp::io::read_to_vec(&path) {
        Ok(data) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::FileReady { path, data });
        },
        Err(_) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: "file not found".into(),
            });
        },
    }
}

fn handle_http_get(url: String, tag: u32) {
    // Network must be initialized before HTTP.
    if let Err(e) = crate::network::ensure_net_init_pub() {
        let _ = IO_RESP_QUEUE.push(IoResponse::Error {
            path: url,
            msg: format!("net init: {e}"),
        });
        return;
    }

    let mut url_bytes: Vec<u8> = url.as_bytes().to_vec();
    url_bytes.push(0);

    match psp::http::HttpClient::new() {
        Ok(client) => {
            match client.request(psp::sys::HttpMethod::Get, &url_bytes)
                .timeout(15_000) // 15 second timeout
                .send()
            {
                Ok(resp) => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::HttpDone {
                        tag,
                        status_code: resp.status_code,
                        body: resp.body,
                    });
                },
                Err(e) => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                        path: url,
                        msg: format!("HTTP GET: {e}"),
                    });
                },
            }
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path: url,
                msg: format!("HTTP init: {e}"),
            });
        },
    }
}

// ---------------------------------------------------------------------------
// TV catalog handler (I/O thread -- JSON parse off main thread)
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

fn handle_tv_catalog_batch(requests: Vec<TvCatalogRequest>) {
    io_log(&format!("[IO-TV] batch: {} requests", requests.len()));

    if let Err(e) = crate::network::ensure_net_init_pub() {
        io_log(&format!("[IO-TV] net init failed: {e}"));
        return;
    }

    let client = match psp::http::HttpClient::new() {
        Ok(c) => c,
        Err(e) => {
            io_log(&format!("[IO-TV] HTTP init failed: {e}"));
            return;
        },
    };

    for req in &requests {
        io_log(&format!("[IO-TV] fetching ch={} {}", req.ch_idx, req.url));

        let mut url_bytes: Vec<u8> = req.url.as_bytes().to_vec();
        url_bytes.push(0);

        let resp = match client.request(psp::sys::HttpMethod::Get, &url_bytes)
            .timeout(15_000)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                io_log(&format!("[IO-TV] GET failed ch={}: {e}", req.ch_idx));
                continue;
            },
        };

        io_log(&format!(
            "[IO-TV] ch={} status={} len={}",
            req.ch_idx, resp.status_code, resp.body.len()
        ));

        if resp.status_code < 200 || resp.status_code >= 300 {
            continue;
        }

        if resp.body.len() < 256 {
            let preview = String::from_utf8_lossy(&resp.body);
            io_log(&format!("[IO-TV] body: {preview}"));
        }

        // Convert to String and drop the original body to reduce peak memory.
        let body_len = resp.body.len();
        let json = String::from_utf8_lossy(&resp.body).into_owned();
        drop(resp);
        io_log(&format!("[IO-TV] parsing ch={} ({body_len} bytes)...", req.ch_idx));
        let episodes = parse_files_lightweight(
            &json,
            &req.item_id,
            req.subfolder.as_deref(),
        );
        io_log(&format!("[IO-TV] ch={} parsed {} episodes", req.ch_idx, episodes.len()));

        let _ = IO_RESP_QUEUE.push(IoResponse::TvCatalogReady {
            ch_idx: req.ch_idx,
            episodes,
        });
    }

    io_log("[IO-TV] batch complete");
}

/// Extract a JSON string value for the given key from a JSON object substring.
/// Returns the unescaped value or empty string if not found.
fn extract_json_str<'a>(obj: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{}\":\"", key);
    if let Some(start) = obj.find(&needle) {
        let val_start = start + needle.len();
        if let Some(end) = obj[val_start..].find('"') {
            return &obj[val_start..val_start + end];
        }
    }
    ""
}

/// Lightweight archive.org `/metadata/ITEM/files` parser.
///
/// Scans the JSON for file objects without building a full DOM tree.
/// Extracts only MP4/h.264 video entries, matching the same filtering
/// as `ChannelCatalog::parse_files_response` but with O(1) heap overhead.
fn parse_files_lightweight(
    json: &str,
    item_id: &str,
    subfolder: Option<&str>,
) -> Vec<oasis_core::apps::tv_guide::VideoEpisode> {
    // Find the "result" array.
    let result_start = match json.find("\"result\":[") {
        Some(pos) => pos + "\"result\":[".len(),
        None => {
            match json.find("\"result\": [") {
                Some(pos) => pos + "\"result\": [".len(),
                None => return Vec::new(),
            }
        },
    };

    let mut episodes = Vec::new();
    let rest = &json[result_start..];

    // Pre-compute subfolder prefix outside the loop.
    let sf_prefix: Option<String> = subfolder.map(|sf| format!("{sf}/"));

    // Iterate over objects in the array by finding matched { }.
    let mut pos = 0;
    while pos < rest.len() {
        let obj_start = match rest[pos..].find('{') {
            Some(p) => pos + p,
            None => break,
        };
        // Find the matching closing brace. Skip nested braces by tracking
        // depth, ignoring braces inside JSON string literals.
        let mut depth = 0i32;
        let mut obj_end = obj_start;
        let mut in_string = false;
        let mut escape = false;
        for (i, b) in rest[obj_start..].bytes().enumerate() {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' if in_string => escape = true,
                b'"' => in_string = !in_string,
                b'{' if !in_string => depth += 1,
                b'}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        obj_end = obj_start + i + 1;
                        break;
                    }
                },
                _ => {},
            }
        }
        if depth != 0 {
            break; // Malformed JSON.
        }
        let obj = &rest[obj_start..obj_end];
        pos = obj_end;

        let name = extract_json_str(obj, "name");
        if name.is_empty() {
            continue;
        }

        // Quick filter: skip non-video files early (before extracting other fields).
        let format_str = extract_json_str(obj, "format");
        let is_video = format_str.eq_ignore_ascii_case("h.264")
            || format_str.eq_ignore_ascii_case("mpeg4")
            || format_str.eq_ignore_ascii_case("h.264 ia")
            || name.ends_with(".mp4");
        if !is_video {
            continue;
        }

        // Subfolder filter.
        if let Some(ref prefix) = sf_prefix {
            if !name.starts_with(prefix.as_str()) {
                continue;
            }
        }

        // Parse duration — skip files without one.
        let length_str = extract_json_str(obj, "length");
        let duration: f64 = length_str.parse().unwrap_or(0.0);
        if duration <= 0.0 {
            continue;
        }

        let width: u32 = extract_json_str(obj, "width").parse().unwrap_or(0);
        let height: u32 = extract_json_str(obj, "height").parse().unwrap_or(0);
        let size_bytes: u64 = extract_json_str(obj, "size").parse().unwrap_or(0);
        let original = extract_json_str(obj, "original");

        // Derive title from filename.
        let display_name = if let Some(ref prefix) = sf_prefix {
            name.strip_prefix(prefix.as_str()).unwrap_or(name)
        } else {
            name
        };
        let title = display_name
            .strip_suffix(".mp4")
            .or_else(|| display_name.strip_suffix(".MP4"))
            .unwrap_or(display_name)
            .replace('_', " ");

        episodes.push(oasis_core::apps::tv_guide::VideoEpisode {
            item_id: item_id.to_string(),
            filename: name.to_string(),
            title,
            duration_secs: duration,
            width,
            height,
            size_bytes,
            format: format_str.to_string(),
            original: if original.is_empty() { None } else { Some(original.to_string()) },
        });

        // Cap at 50 episodes per channel to limit memory.
        if episodes.len() >= 50 {
            break;
        }
    }

    episodes
}

// ---------------------------------------------------------------------------
// Video download handler (I/O thread)
// ---------------------------------------------------------------------------

/// Download a video file via HTTP and write it to Memory Stick.
///
/// Uses `psp::http::HttpClient` which buffers the entire response in RAM.
/// The `select_smallest_for()` caller ensures files are capped (default 20MB)
/// so this fits within PSP memory constraints.
/// Parse MP4 box headers from the first bytes of a download to find where
/// the moov atom ends.  Returns `Some(moov_offset + moov_size)` for
/// faststart files (moov before mdat), or `None` if moov wasn't found.
fn find_moov_end(header_bytes: &[u8]) -> Option<u64> {
    let mut pos = 0usize;
    while pos + 8 <= header_bytes.len() {
        let size = u32::from_be_bytes([
            header_bytes[pos],
            header_bytes[pos + 1],
            header_bytes[pos + 2],
            header_bytes[pos + 3],
        ]) as u64;
        let box_type = &header_bytes[pos + 4..pos + 8];

        if box_type == b"moov" {
            if size == 0 {
                return None; // extends to EOF, can't determine end
            }
            return Some(pos as u64 + size);
        }

        // 64-bit extended size
        if size == 1 {
            if pos + 16 > header_bytes.len() {
                break;
            }
            let big = u64::from_be_bytes([
                header_bytes[pos + 8],
                header_bytes[pos + 9],
                header_bytes[pos + 10],
                header_bytes[pos + 11],
                header_bytes[pos + 12],
                header_bytes[pos + 13],
                header_bytes[pos + 14],
                header_bytes[pos + 15],
            ]);
            pos += big as usize;
        } else if size == 0 {
            break; // box extends to EOF
        } else {
            pos += size as usize;
        }
    }
    None
}

/// Persistent sceHttp template ID.  Initialized once, never torn down.
/// Mirrors how `psp::http::HttpClient` works (one template, many requests).
/// SAFETY: Only accessed from the I/O thread (single producer).
static mut DL_TEMPLATE_ID: i32 = -1;

/// Ensure sceHttp is initialized and return the persistent template ID.
///
/// On first call: `sceHttpInit` + `sceHttpCreateTemplate`.
/// On subsequent calls: returns the cached template ID immediately.
unsafe fn ensure_dl_template() -> Result<i32, String> {
    use psp::sys;

    if DL_TEMPLATE_ID >= 0 {
        return Ok(DL_TEMPLATE_ID);
    }

    let ret = sys::sceHttpInit(0x20000);
    // Accept "already initialized" (0x80431020) in case IO-TV already
    // initialized it via psp::http::HttpClient.
    if ret < 0 && ret != -0x7FBCEFE0_i32 {
        io_log(&format!("[IO-DL] sceHttpInit failed: {ret:#x}"));
        return Err(format!("sceHttpInit failed: {ret:#x}"));
    }
    io_log(&format!("[IO-DL] sceHttpInit: {ret:#x}"));

    let tid = sys::sceHttpCreateTemplate(
        b"oasis-psp/1.0\0".as_ptr() as *mut u8,
        1, 0,
    );
    if tid < 0 {
        return Err(format!("template: {tid:#x}"));
    }
    // Disable keep-alive so each request gets a fresh TCP connection.
    sys::sceHttpDisableKeepAlive(tid);
    // Disable auto-redirect. archive.org redirects some items'
    // HTTP URLs to HTTPS, and PSP's built-in SSL (2008 root CAs)
    // can't connect. We handle redirects manually, rewriting
    // HTTPS→HTTP in the Location header.
    sys::sceHttpDisableRedirect(tid);
    io_log(&format!(
        "[IO-DL] template created: {tid} (keep-alive off, redirect off)"
    ));
    DL_TEMPLATE_ID = tid;
    Ok(tid)
}

/// Open an HTTP connection with manual redirect handling.
///
/// PSP's `sceHttpEnableRedirect` follows HTTP→HTTPS redirects which fail
/// because the firmware's root CAs are from 2008. Instead, we handle
/// 301/302/307/308 manually, rewriting `https://` → `http://` in the
/// Location header.
///
/// Returns `(req_id, conn_id, content_length)` on success.
/// Uses a persistent template — caller must only clean up req_id and conn_id.
///
/// On redirect-loop failure (CDN requires HTTPS), returns the HTTPS
/// redirect URL as the second element so the caller can try TLS.
unsafe fn http_open_with_redirect(
    url: &str,
) -> Result<(i32, i32, u64), (String, Option<String>)> {
    use psp::sys;

    let template_id = ensure_dl_template()
        .map_err(|e| (e, None))?;
    let mut current_url = url.to_string();
    // Track the last HTTPS redirect URL for TLS fallback.
    let mut last_https_redirect: Option<String> = None;

    for attempt in 0..5 {
        let mut url_bytes: Vec<u8> = current_url.as_bytes().to_vec();
        url_bytes.push(0);

        let conn_id = sys::sceHttpCreateConnectionWithURL(
            template_id,
            url_bytes.as_ptr(),
            0,
        );
        if conn_id < 0 {
            return Err((format!("connect: {conn_id:#x}"), None));
        }

        let req_id = sys::sceHttpCreateRequestWithURL(
            conn_id,
            sys::HttpMethod::Get,
            url_bytes.as_ptr() as *mut u8,
            0,
        );
        if req_id < 0 {
            sys::sceHttpDeleteConnection(conn_id);
            return Err((format!("request: {req_id:#x}"), None));
        }

        sys::sceHttpSetConnectTimeOut(req_id, 30_000_000);
        sys::sceHttpSetRecvTimeOut(req_id, 30_000_000);

        let ret = sys::sceHttpSendRequest(req_id, core::ptr::null_mut(), 0);
        if ret < 0 {
            io_log(&format!("[IO-DL] send failed: {ret:#x}"));
            sys::sceHttpDeleteRequest(req_id);
            sys::sceHttpDeleteConnection(conn_id);
            return Err((
                format!("send: {ret:#x}"),
                last_https_redirect.clone(),
            ));
        }

        let mut status_code: i32 = 0;
        sys::sceHttpGetStatusCode(req_id, &mut status_code);
        io_log(&format!("[IO-DL] status={status_code} (attempt {attempt})"));

        // Handle redirects manually.
        if matches!(status_code, 301 | 302 | 303 | 307 | 308) {
            // Read all headers to find Location — must copy BEFORE
            // deleting the request, since the pointer is into its buffer.
            let mut hdr_ptr: *mut u8 = core::ptr::null_mut();
            let mut hdr_len: u32 = 0;
            let ret = sys::sceHttpGetAllHeader(req_id, &mut hdr_ptr, &mut hdr_len);

            let location_url = if ret >= 0
                && !hdr_ptr.is_null()
                && hdr_len > 0
            {
                // SAFETY: pointer valid while request alive.
                let hdrs = core::slice::from_raw_parts(hdr_ptr, hdr_len as usize);
                let hdr_str = core::str::from_utf8(hdrs).unwrap_or("");
                hdr_str
                    .lines()
                    .find(|l| {
                        l.len() > 9
                            && l[..9].eq_ignore_ascii_case("location:")
                    })
                    .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            } else {
                None
            };

            // Now safe to delete the request+connection (template persists).
            sys::sceHttpDeleteRequest(req_id);
            sys::sceHttpDeleteConnection(conn_id);

            if let Some(loc) = location_url {
                // Save HTTPS URL for TLS fallback before rewriting.
                if loc.starts_with("https://") {
                    last_https_redirect = Some(loc.clone());
                }
                // Rewrite HTTPS → HTTP so PSP can follow it.
                let new_url = loc.replacen("https://", "http://", 1);
                // Detect redirect loop: same URL after rewrite.
                if new_url == current_url {
                    io_log(&format!(
                        "[IO-DL] redirect loop detected → {new_url}"
                    ));
                    return Err((
                        String::from("redirect loop (CDN requires HTTPS)"),
                        last_https_redirect,
                    ));
                }
                io_log(&format!("[IO-DL] redirect → {new_url}"));
                current_url = new_url;
                continue;
            } else {
                return Err((
                    format!("redirect {status_code}, no Location"),
                    None,
                ));
            }
        }

        if status_code < 200 || status_code >= 300 {
            sys::sceHttpDeleteRequest(req_id);
            sys::sceHttpDeleteConnection(conn_id);
            return Err((format!("HTTP {status_code}"), None));
        }

        let mut content_length: u64 = 0;
        sys::sceHttpGetContentLength(req_id, &mut content_length);

        return Ok((req_id, conn_id, content_length));
    }

    Err((
        String::from("too many redirects"),
        last_https_redirect,
    ))
}

/// Raw TCP HTTP reader — bypasses sceHttp entirely using BSD sockets.
///
/// sceHttp's internal state corrupts after the first download session,
/// causing `0x80431079` on subsequent `sceHttpSendRequest` calls.
/// Raw sockets have no such state — each connection is independent.
struct RawHttpReader {
    fd: i32,
    /// Leftover body data read during header parsing.
    leftover: Vec<u8>,
}

impl RawHttpReader {
    /// Open an HTTP connection via raw TCP: DNS → connect → GET → parse headers.
    ///
    /// Returns the reader and content length (0 if unknown).
    /// Follows up to 5 redirects.
    fn open(url: &str) -> Result<(Self, u64), String> {
        Self::open_with_redirects(url, 5)
    }

    fn open_with_redirects(
        url: &str,
        max_redirects: u32,
    ) -> Result<(Self, u64), String> {
        let (host, port, path, _) =
            parse_url(url).ok_or_else(|| format!("bad URL: {url}"))?;

        io_log(&format!("[IO-RAW] resolving {host}..."));

        let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
        host_bytes.push(0);
        let addr = psp::net::resolve_hostname(&host_bytes)
            .map_err(|e| format!("DNS {host}: {e}"))?;

        io_log(&format!(
            "[IO-RAW] resolved {host} → {}.{}.{}.{}",
            addr.0[0], addr.0[1], addr.0[2], addr.0[3]
        ));

        // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
        let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            return Err("socket() failed".into());
        }

        // Set recv/send timeouts (30s) before connect.
        // SAFETY: Valid socket options on PSP BSD stack.
        unsafe {
            #[repr(C)]
            struct Timeval { tv_sec: i32, tv_usec: i32 }
            let timeout = Timeval { tv_sec: 30, tv_usec: 0 };
            let timeout_ptr =
                &timeout as *const Timeval as *const core::ffi::c_void;
            let timeout_len = core::mem::size_of::<Timeval>() as u32;
            psp::sys::sceNetInetSetsockopt(
                fd, 0xFFFF, 0x1005, timeout_ptr, timeout_len,
            );
            psp::sys::sceNetInetSetsockopt(
                fd, 0xFFFF, 0x1006, timeout_ptr, timeout_len,
            );
        }

        io_log(&format!("[IO-RAW] connecting to {host}:{port}..."));

        let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
        // SAFETY: Blocking connect — will return when connected or on
        // TCP timeout. Port 80 is not blocked so this completes quickly.
        let ret = unsafe {
            psp::sys::sceNetInetConnect(
                fd, &sa,
                core::mem::size_of::<psp::sys::sockaddr>() as u32,
            )
        };
        if ret < 0 {
            io_log(&format!("[IO-RAW] connect failed: {ret:#x}"));
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("connect failed {host}:{port}: {ret:#x}"));
        }

        io_log("[IO-RAW] connected, sending HTTP GET...");

        // Send HTTP/1.1 GET request.
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             User-Agent: oasis-psp/1.0\r\n\
             Accept: */*\r\n\
             Connection: close\r\n\r\n"
        );
        let req_bytes = request.as_bytes();
        let mut sent = 0usize;
        while sent < req_bytes.len() {
            // SAFETY: fd is valid, buf points to request data.
            let n = unsafe {
                psp::sys::sceNetInetSend(
                    fd,
                    req_bytes[sent..].as_ptr() as *const core::ffi::c_void,
                    req_bytes.len() - sent,
                    0,
                )
            };
            if n <= 0 {
                unsafe { psp::sys::sceNetInetClose(fd) };
                return Err("send failed".into());
            }
            sent += n as usize;
        }

        // Read response headers (up to 8KB).
        let mut hdr_buf = vec![0u8; 8192];
        let mut hdr_len = 0usize;
        loop {
            if hdr_len >= hdr_buf.len() {
                break;
            }
            // SAFETY: fd is valid, buffer is valid.
            let n = unsafe {
                psp::sys::sceNetInetRecv(
                    fd,
                    hdr_buf[hdr_len..].as_mut_ptr() as *mut core::ffi::c_void,
                    hdr_buf.len() - hdr_len,
                    0,
                )
            };
            if n <= 0 {
                break;
            }
            hdr_len += n as usize;
            if find_header_end(&hdr_buf[..hdr_len]).is_some() {
                break;
            }
        }

        let header_end = find_header_end(&hdr_buf[..hdr_len])
            .ok_or_else(|| "incomplete HTTP headers".to_string())?;

        let hdr_str =
            core::str::from_utf8(&hdr_buf[..header_end]).unwrap_or("");
        io_log(&format!(
            "[IO-RAW] response: {}",
            hdr_str.lines().next().unwrap_or("?")
        ));

        let status = hdr_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // Handle redirects.
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            unsafe { psp::sys::sceNetInetClose(fd) };

            if max_redirects == 0 {
                return Err("too many redirects".into());
            }

            let location = hdr_str.lines().find_map(|l| {
                if l.len() > 9
                    && l[..9].eq_ignore_ascii_case("location:")
                {
                    l.split_once(':').map(|(_, v)| v.trim().to_string())
                } else {
                    None
                }
            });

            if let Some(loc) = location {
                // Rewrite HTTPS → HTTP for PSP.
                let new_url = loc.replacen("https://", "http://", 1);
                io_log(&format!("[IO-RAW] redirect → {new_url}"));
                return Self::open_with_redirects(
                    &new_url,
                    max_redirects - 1,
                );
            }
            return Err(format!("redirect {status}, no Location"));
        }

        if status < 200 || status >= 300 {
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("HTTP {status}"));
        }

        let content_length: u64 = hdr_str
            .lines()
            .find_map(|l| {
                if l.len() > 15
                    && l[..15].eq_ignore_ascii_case("content-length:")
                {
                    l[15..].trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        io_log(&format!(
            "[IO-RAW] status={status} content-length={content_length}"
        ));

        let leftover = hdr_buf[header_end..hdr_len].to_vec();

        Ok((Self { fd, leftover }, content_length))
    }

    /// Read body data. Returns leftover first, then reads from socket.
    fn read_data(&mut self, buf: &mut [u8]) -> i32 {
        if !self.leftover.is_empty() {
            let take = core::cmp::min(self.leftover.len(), buf.len());
            buf[..take].copy_from_slice(&self.leftover[..take]);
            self.leftover.drain(..take);
            return take as i32;
        }
        // SAFETY: fd is valid, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                self.fd,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 { 0 } else { n as i32 }
    }

    /// Close the socket.
    fn cleanup(self) {
        // SAFETY: fd is a valid open socket.
        unsafe { psp::sys::sceNetInetClose(self.fd) };
    }
}

/// Abstraction over HTTP data sources.
enum HttpDataSource {
    /// PSP's built-in HTTP library (for `http://` URLs).
    SceHttp {
        req_id: i32,
        conn_id: i32,
    },
    /// Raw TCP + TLS 1.3 via embedded-tls (for `https://` URLs).
    Tls(TlsHttpReader),
}

impl HttpDataSource {
    /// Read data into `buf`. Returns bytes read, 0 on EOF, negative on error.
    fn read_data(&mut self, buf: &mut [u8]) -> i32 {
        match self {
            HttpDataSource::SceHttp { req_id, .. } => {
                // SAFETY: req_id is a valid HTTP request handle.
                unsafe {
                    psp::sys::sceHttpReadData(
                        *req_id,
                        buf.as_mut_ptr() as *mut core::ffi::c_void,
                        buf.len() as u32,
                    )
                }
            },
            HttpDataSource::Tls(reader) => {
                reader.read_data(buf).unwrap_or(0)
            },
        }
    }

    /// Clean up the connection — abort the in-flight request, delete
    /// request+connection handles. The persistent template stays alive.
    fn cleanup(self) {
        match self {
            HttpDataSource::SceHttp {
                req_id,
                conn_id,
            } => {
                // SAFETY: IDs are valid sceHttp handles.
                unsafe {
                    psp::sys::sceHttpAbortRequest(req_id);
                    psp::sys::sceHttpDeleteRequest(req_id);
                    psp::sys::sceHttpDeleteConnection(conn_id);
                }
                io_log("[IO-DL] cleanup: abort+delete done");
            },
            HttpDataSource::Tls(reader) => reader.cleanup(),
        }
    }
}

/// Streaming video download: buffers moov atom in memory, parses MP4 track
/// tables, then extracts and pushes demuxed samples directly to the video
/// and audio threads as HTTP data arrives. No disk I/O.
///
/// Supports both HTTP (via sceHttp) and HTTPS (via raw TCP + embedded-tls).
fn handle_video_download(url: String, _dest: String, tag: u32) {
    use oasis_video::demux_lite::Mp4Lite;

    io_log(&format!("[IO-DL] starting stream: {url}"));

    // Clear any previous cancellation flag.
    DOWNLOAD_CANCEL.store(false, Ordering::Release);

    if let Err(e) = crate::network::ensure_net_init_pub() {
        let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
            tag,
            msg: format!("net init: {e}"),
        });
        return;
    }

    // Try sceHttp first (HTTP port 80), fall back to raw TCP + TLS 1.3
    // (HTTPS port 443) if sceHttp fails. sceHttp's internal connection
    // pool corrupts after an aborted partial download, causing 0x80431079
    // on subsequent requests. The TLS path uses independent raw sockets.
    let http_url = if url.starts_with("https://") {
        url.replacen("https://", "http://", 1)
    } else {
        url.clone()
    };

    // SAFETY: All sceHttp calls use IDs returned by prior creation.
    let (mut source, content_length) =
        match unsafe { http_open_with_redirect(&http_url) } {
            Ok((req_id, conn_id, cl)) => {
                io_log("[IO-DL] sceHttp OK");
                (
                    HttpDataSource::SceHttp {
                        req_id,
                        conn_id,
                    },
                    cl,
                )
            },
            Err((msg, https_redirect)) => {
                // Try TLS fallback. First try archive.org HTTPS
                // (may redirect to a different, reachable CDN node),
                // then try CDN HTTPS directly as a last resort.
                let origin_tls = if url.starts_with("http://") {
                    url.replacen("http://", "https://", 1)
                } else {
                    url.clone()
                };

                // Build candidate list: origin first, then CDN if different.
                let mut tls_candidates: Vec<String> = vec![origin_tls];
                if let Some(cdn_url) = &https_redirect {
                    if !tls_candidates.contains(cdn_url) {
                        tls_candidates.push(cdn_url.clone());
                    }
                }

                let mut last_err = msg.clone();
                let mut found = None;
                for (i, tls_url) in tls_candidates.iter().enumerate() {
                    io_log(&format!(
                        "[IO-DL] sceHttp failed ({msg}), trying TLS \
                         #{} to {tls_url}...",
                        i + 1
                    ));
                    match TlsHttpReader::open(tls_url) {
                        Ok((reader, cl)) => {
                            io_log(&format!(
                                "[IO-DL] TLS fallback #{} OK, len={cl}",
                                i + 1
                            ));
                            found = Some((reader, cl));
                            break;
                        },
                        Err(e) => {
                            io_log(&format!(
                                "[IO-DL] TLS fallback #{} failed: {e}",
                                i + 1
                            ));
                            last_err = e;
                        },
                    }
                }

                match found {
                    Some((reader, cl)) => {
                        (HttpDataSource::Tls(reader), cl)
                    },
                    None => {
                        let _ =
                            IO_RESP_QUEUE.push(IoResponse::VideoError {
                                tag,
                                msg: format!(
                                    "HTTP: {msg}; TLS: {last_err}"
                                ),
                            });
                        return;
                    },
                }
            },
        };

    let total = if content_length > 0 {
        Some(content_length)
    } else {
        None
    };
    io_log(&format!("[IO-DL] content-length={content_length}"));

    // Phase 1: buffer data until moov atom is fully received.
    let mut moov_buf: Vec<u8> = Vec::new();
    let mut moov_end: Option<u64> = None;
    let mut buf = [0u8; 8192];
    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    loop {
        // Check for cancellation (user pressed Circle during download).
        if DOWNLOAD_CANCEL.load(Ordering::Acquire) {
            io_log("[IO-DL] cancelled during moov buffering");
            source.cleanup();
            return;
        }

        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase1): {n:#x}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("read: {n:#x}"),
            });
            return;
        }
        if n == 0 {
            break; // EOF during moov buffering
        }

        moov_buf.extend_from_slice(&buf[..n as usize]);
        downloaded += n as u64;

        // Report progress.
        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }

        // Try to find moov end from headers once we have enough.
        if moov_end.is_none() && moov_buf.len() >= 32 {
            moov_end = find_moov_end(&moov_buf);
            if let Some(end) = moov_end {
                io_log(&format!("[IO-DL] moov ends at byte {end}"));
            }
        }

        // Check if we've buffered past moov end.
        if let Some(end) = moov_end {
            if downloaded >= end {
                io_log(&format!(
                    "[IO-DL] moov fully buffered ({downloaded} bytes, \
                     moov_end={end})"
                ));
                break;
            }
        }

        // Safety limit: if moov hasn't been found after 8MB, abort.
        if moov_buf.len() > 8 * 1024 * 1024 {
            io_log("[IO-DL] moov not found in first 8MB, aborting");
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: String::from("moov atom not found (non-faststart?)"),
            });
            return;
        }
    }

    // Parse moov using Mp4Lite with a Cursor over the buffered data.
    io_log(&format!(
        "[IO-DL] parsing moov ({} bytes buffered)...",
        moov_buf.len()
    ));

    let cursor = std::io::Cursor::new(&moov_buf);
    let mp4 = match Mp4Lite::open(cursor) {
        Ok(m) => m,
        Err(e) => {
            io_log(&format!("[IO-DL] Mp4Lite parse failed: {e}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("MP4 parse: {e}"),
            });
            return;
        },
    };

    let video_track = mp4.video_track_info().cloned();
    let audio_track = mp4.audio_track_info().cloned();
    drop(mp4);

    let v_count =
        video_track.as_ref().map_or(0, |t| t.sample_count());
    let a_count =
        audio_track.as_ref().map_or(0, |t| t.sample_count());
    io_log(&format!(
        "[IO-DL] parsed: {v_count} video, {a_count} audio samples"
    ));

    // Send AAC config to audio thread before any frames arrive.
    if let Some(ref at) = audio_track {
        if let Some(ref aac) = at.aac_config {
            send_audio_cmd(AudioCmd::VideoAudioAacConfig {
                sample_rate: aac.sample_rate,
                channels: aac.channels,
            });
            io_log(&format!(
                "[IO-DL] AAC config: rate={}, ch={}",
                aac.sample_rate, aac.channels
            ));
        }
    }

    io_log("[IO-DL] setting video_playing...");

    // Pre-arm the playing flag BEFORE sending StreamStart to avoid a
    // race: the I/O thread checks is_video_playing() in the phase-2
    // loop, but the video thread may not have processed the command yet.
    crate::video::set_video_playing(true);
    crate::video::request_stream_start();

    io_log("[IO-DL] sending VideoStreamReady...");

    // Notify main thread that streaming playback has begun.
    let _ = IO_RESP_QUEUE.push(IoResponse::VideoStreamReady {
        tag,
        path: String::new(),
        content_length: content_length as u32,
    });

    // Phase 2: stream mdat samples from HTTP(S).
    let mut v_idx = 0usize;
    let mut a_idx = 0usize;
    let mut http_pos: u64;

    let mut sample_data: Vec<u8> = Vec::new();
    let mut sample_offset: u64 = 0;
    let mut sample_size: u32 = 0;
    let mut sample_is_video = false;
    let mut have_target = false;

    let moov_end_off = moov_end.unwrap_or(downloaded);
    let leftover_start = moov_end_off as usize;
    let leftover = if leftover_start < moov_buf.len() {
        &moov_buf[leftover_start..]
    } else {
        &[]
    };
    http_pos = moov_end_off;

    io_log(&format!(
        "[IO-DL] leftover={} bytes, moov_end_off={moov_end_off}",
        leftover.len()
    ));

    if !leftover.is_empty() {
        process_stream_chunk(
            leftover,
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );
        io_log(&format!(
            "[IO-DL] leftover processed: v={v_idx} a={a_idx}"
        ));
    }

    io_log("[IO-DL] dropping moov_buf...");
    drop(moov_buf);
    io_log("[IO-DL] entering phase 2 loop");

    let mut loop_iter = 0u32;
    loop {
        if !crate::video::is_video_playing()
            || DOWNLOAD_CANCEL.load(Ordering::Acquire)
        {
            io_log("[IO-DL] playback stopped, ending stream");
            break;
        }

        if loop_iter < 3 {
            io_log(&format!("[IO-DL] phase2 read #{loop_iter}..."));
        }
        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase2): {n:#x}"));
            break;
        }
        if n == 0 {
            break; // EOF
        }
        if loop_iter < 3 {
            io_log(&format!("[IO-DL] phase2 read #{loop_iter}: {n} bytes"));
        }

        downloaded += n as u64;
        loop_iter += 1;

        process_stream_chunk(
            &buf[..n as usize],
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );

        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }
    }

    source.cleanup();

    io_log(&format!(
        "[IO-DL] stream complete: {downloaded} bytes, \
         {v_idx}/{v_count} video, {a_idx}/{a_count} audio"
    ));

    crate::video::set_video_playing(false);
    send_audio_cmd(AudioCmd::VideoAudioStop);
}

/// Determine the next sample to extract (lowest file offset among pending
/// video and audio samples).
fn next_sample_target(
    v_idx: usize,
    a_idx: usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) -> Option<(u64, u32, bool)> {
    let v_next = video_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(v_idx));
    let a_next = audio_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(a_idx));

    match (v_next, a_next) {
        (Some((vo, vs)), Some((ao, a_s))) => {
            if vo <= ao {
                Some((vo, vs, true))
            } else {
                Some((ao, a_s, false))
            }
        },
        (Some((vo, vs)), None) => Some((vo, vs, true)),
        (None, Some((ao, a_s))) => Some((ao, a_s, false)),
        (None, None) => None,
    }
}

/// Process a chunk of HTTP data, extracting complete samples and pushing
/// them to the video/audio decode threads.
#[allow(clippy::too_many_arguments)]
fn process_stream_chunk(
    chunk: &[u8],
    http_pos: &mut u64,
    have_target: &mut bool,
    sample_offset: &mut u64,
    sample_size: &mut u32,
    sample_is_video: &mut bool,
    sample_data: &mut Vec<u8>,
    v_idx: &mut usize,
    a_idx: &mut usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) {
    let mut chunk_pos = 0usize;

    while chunk_pos < chunk.len() {
        // Find next sample target if we don't have one.
        if !*have_target {
            match next_sample_target(*v_idx, *a_idx, video_track, audio_track) {
                Some((off, sz, is_v)) => {
                    *sample_offset = off;
                    *sample_size = sz;
                    *sample_is_video = is_v;
                    *have_target = true;
                    sample_data.clear();
                },
                None => {
                    // All samples extracted; skip remaining data.
                    *http_pos += (chunk.len() - chunk_pos) as u64;
                    return;
                },
            }
        }

        // Skip bytes before sample start.
        if *http_pos < *sample_offset {
            let skip = core::cmp::min(
                (*sample_offset - *http_pos) as usize,
                chunk.len() - chunk_pos,
            );
            chunk_pos += skip;
            *http_pos += skip as u64;
            if *http_pos < *sample_offset {
                return; // need more data to reach sample
            }
        }

        if *sample_is_video {
            // Skip video sample data — just advance stream position.
            // sample_data is unused for video; track progress via offset.
            let sample_end = *sample_offset + *sample_size as u64;
            let available = chunk.len() - chunk_pos;
            let remaining = (sample_end - *http_pos) as usize;
            let skip = core::cmp::min(remaining, available);
            chunk_pos += skip;
            *http_pos += skip as u64;

            if *http_pos >= sample_end {
                *v_idx += 1;
                *have_target = false;
            }
        } else {
            // Buffer audio sample data.
            let remaining = *sample_size as usize - sample_data.len();
            let available = chunk.len() - chunk_pos;
            let take = core::cmp::min(remaining, available);
            sample_data.extend_from_slice(&chunk[chunk_pos..chunk_pos + take]);
            chunk_pos += take;
            *http_pos += take as u64;

            if sample_data.len() == *sample_size as usize {
                let data = core::mem::take(sample_data);
                // Blocking push with backpressure: retry until the audio
                // queue has space, sleeping 2ms between attempts. This
                // throttles the I/O thread to match the audio decode rate,
                // preventing frame drops and choppy playback.
                let mut cmd = AudioCmd::VideoAudioAac { data };
                loop {
                    match AUDIO_QUEUE.push(cmd) {
                        Ok(()) => break,
                        Err(returned) => {
                            cmd = returned;
                            // Check if playback was stopped to avoid
                            // deadlocking the I/O thread.
                            if !crate::video::is_video_playing() {
                                break;
                            }
                            // SAFETY: sceKernelDelayThread sleeps thread.
                            unsafe {
                                psp::sys::sceKernelDelayThread(2_000);
                            }
                        },
                    }
                }
                *a_idx += 1;
                *have_target = false;
                sample_data.clear();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Radio connection handler (I/O thread)
// ---------------------------------------------------------------------------

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
        Some(i) => (
            &host_port[..i],
            host_port[i + 1..].parse::<u16>().ok()?,
        ),
        None => (host_port, default_port),
    };
    Some((host.to_string(), port, path.to_string(), is_https))
}

// ---------------------------------------------------------------------------
// HTTPS support via raw TCP + embedded-tls (TLS 1.3)
// ---------------------------------------------------------------------------
//
// PSP's sceHttp SSL stack uses firmware root CAs from 2008 and SSL 3.0,
// which can't connect to modern HTTPS servers. Instead, we use raw TCP
// sockets wrapped with embedded-tls for TLS 1.3 with UnsecureProvider
// (no certificate validation -- acceptable for PSP media streaming).

/// Wraps a raw PSP socket fd for `embedded_io::Read + Write`.
struct PspSocketIo {
    fd: i32,
}

impl embedded_io::ErrorType for PspSocketIo {
    type Error = embedded_io::ErrorKind;
}

impl embedded_io::Read for PspSocketIo {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        // SAFETY: fd is a valid socket descriptor, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                self.fd,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            Err(embedded_io::ErrorKind::Other)
        } else {
            Ok(n as usize)
        }
    }
}

impl embedded_io::Write for PspSocketIo {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        // SAFETY: fd is a valid socket descriptor, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetSend(
                self.fd,
                buf.as_ptr() as *const core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            Err(embedded_io::ErrorKind::Other)
        } else {
            Ok(n as usize)
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

type PspTlsConn<'a> = embedded_tls::blocking::TlsConnection<
    'a,
    PspSocketIo,
    embedded_tls::blocking::Aes128GcmSha256,
>;

/// HTTPS reader: raw TCP socket + TLS 1.3 + HTTP/1.1.
///
/// Buffers are heap-allocated via `Box::leak` to get 'static lifetime
/// for the TLS connection (same pattern as `tls.rs`).
struct TlsHttpReader {
    tls: PspTlsConn<'static>,
    fd: i32,
    read_buf_ptr: *mut [u8],
    write_buf_ptr: *mut [u8],
    /// Leftover body data read during header parsing.
    leftover: Vec<u8>,
}

/// RNG for TLS handshake using PSP's MT19937 PRNG.
struct IoRng {
    ctx: psp::sys::SceKernelUtilsMt19937Context,
}

impl IoRng {
    fn new() -> Self {
        // SAFETY: MT19937 context is initialized before use.
        // Seed from system timer (user-mode safe). mfc0 $9 (COP0 Count)
        // is privileged on PSP Allegrex and crashes in user mode.
        unsafe {
            let mut ctx = core::mem::MaybeUninit::uninit();
            let seed = psp::sys::sceKernelGetSystemTimeLow() as u32;
            psp::sys::sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
            Self {
                ctx: ctx.assume_init(),
            }
        }
    }
}

impl rand_core::RngCore for IoRng {
    fn next_u32(&mut self) -> u32 {
        // SAFETY: ctx was initialized in new().
        unsafe { psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx) }
    }

    fn next_u64(&mut self) -> u64 {
        let lo = self.next_u32() as u64;
        let hi = self.next_u32() as u64;
        (hi << 32) | lo
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // SAFETY: ctx was initialized in new().
        unsafe {
            for byte in dest.iter_mut() {
                *byte =
                    (psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx)
                        & 0xFF) as u8;
            }
        }
    }

    fn try_fill_bytes(
        &mut self,
        dest: &mut [u8],
    ) -> core::result::Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

// SAFETY: MT19937 is the best PRNG available on PSP hardware.
impl rand_core::CryptoRng for IoRng {}

impl TlsHttpReader {
    /// Open an HTTPS connection: TCP connect → TLS handshake → HTTP GET.
    ///
    /// Returns the reader and content length (0 if unknown).
    fn open(url: &str) -> Result<(Self, u64), String> {
        Self::open_with_redirects(url, 5)
    }

    /// Open with redirect depth limit to prevent infinite recursion.
    fn open_with_redirects(
        url: &str,
        redirects_left: u8,
    ) -> Result<(Self, u64), String> {
        let (host, port, path, _) =
            parse_url(url).ok_or_else(|| format!("bad URL: {url}"))?;

        io_log(&format!("[IO-TLS] resolving {host}..."));

        // DNS resolve.
        let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
        host_bytes.push(0);
        let addr = psp::net::resolve_hostname(&host_bytes)
            .map_err(|e| format!("DNS {host}: {e}"))?;

        io_log(&format!(
            "[IO-TLS] resolved {host} → {}.{}.{}.{}",
            addr.0[0], addr.0[1], addr.0[2], addr.0[3]
        ));

        // TCP connect with non-blocking + polling timeout.
        // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
        let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            return Err("socket() failed".into());
        }

        // Set non-blocking mode for connect with timeout.
        // SAFETY: SO_NONBLOCK=0x1009 is a PSP-specific socket option.
        unsafe {
            let nb: u32 = 1;
            psp::sys::sceNetInetSetsockopt(
                fd,
                0xFFFF,
                0x1009,
                &nb as *const u32 as *const core::ffi::c_void,
                4,
            );
        }

        io_log(&format!("[IO-TLS] TCP connecting to {host}:{port}..."));

        let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
        // SAFETY: Non-blocking connect returns immediately.
        unsafe {
            psp::sys::sceNetInetConnect(
                fd,
                &sa,
                core::mem::size_of::<psp::sys::sockaddr>() as u32,
            );
        }

        // Poll for connection (up to 10 seconds, 100ms intervals).
        // SAFETY: getpeername succeeds only when socket is connected.
        let mut connected = false;
        for tick in 0..100u32 {
            // Check for download cancellation during connect wait.
            if DOWNLOAD_CANCEL.load(Ordering::Acquire) {
                io_log("[IO-TLS] cancelled during TCP connect");
                // SAFETY: Close socket on cancellation.
                unsafe { psp::sys::sceNetInetClose(fd) };
                return Err("cancelled".into());
            }

            let mut sa_out: psp::sys::sockaddr =
                unsafe { core::mem::zeroed() };
            let mut sa_len: u32 =
                core::mem::size_of::<psp::sys::sockaddr>() as u32;
            let ret = unsafe {
                psp::sys::sceNetInetGetpeername(
                    fd,
                    &mut sa_out,
                    &mut sa_len,
                )
            };
            if ret == 0 {
                connected = true;
                break;
            }
            if tick == 0 {
                io_log("[IO-TLS] waiting for TCP connect...");
            }
            // SAFETY: Sleep 100ms between polls.
            unsafe {
                psp::sys::sceKernelDelayThread(100_000);
            }
        }

        if !connected {
            let errno = unsafe { psp::sys::sceNetInetGetErrno() };
            // SAFETY: Close socket on connect timeout.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!(
                "connect timeout {host}:{port} (10s, errno={errno})"
            ));
        }

        // Set back to blocking mode for TLS I/O + timeouts.
        // SAFETY: Valid socket options on PSP BSD stack.
        #[repr(C)]
        struct Timeval {
            tv_sec: i32,
            tv_usec: i32,
        }
        unsafe {
            let nb: u32 = 0;
            psp::sys::sceNetInetSetsockopt(
                fd,
                0xFFFF,
                0x1009,
                &nb as *const u32 as *const core::ffi::c_void,
                4,
            );
            // SOL_SOCKET=0xFFFF, SO_SNDTIMEO=0x1005, SO_RCVTIMEO=0x1006
            let timeout = Timeval {
                tv_sec: 30,
                tv_usec: 0,
            };
            let timeout_ptr =
                &timeout as *const Timeval as *const core::ffi::c_void;
            let timeout_len =
                core::mem::size_of::<Timeval>() as u32;
            psp::sys::sceNetInetSetsockopt(
                fd, 0xFFFF, 0x1005, timeout_ptr, timeout_len,
            );
            psp::sys::sceNetInetSetsockopt(
                fd, 0xFFFF, 0x1006, timeout_ptr, timeout_len,
            );
        }

        io_log("[IO-TLS] TCP connected, starting TLS...");

        // TLS 1.3 handshake via embedded-tls.
        let socket_io = PspSocketIo { fd };

        const RECORD_BUF: usize = 16384 + 256;
        let read_buf =
            Box::leak(vec![0u8; RECORD_BUF].into_boxed_slice());
        let write_buf =
            Box::leak(vec![0u8; RECORD_BUF].into_boxed_slice());
        let read_buf_ptr: *mut [u8] = read_buf;
        let write_buf_ptr: *mut [u8] = write_buf;

        let config = embedded_tls::blocking::TlsConfig::new()
            .with_server_name(&host);

        let mut tls: PspTlsConn<'static> =
            embedded_tls::blocking::TlsConnection::new(
                socket_io, read_buf, write_buf,
            );

        let provider = embedded_tls::UnsecureProvider::new::<
            embedded_tls::blocking::Aes128GcmSha256,
        >(IoRng::new());
        let context = embedded_tls::blocking::TlsContext::new(
            &config,
            provider,
        );
        io_log("[IO-TLS] starting TLS 1.3 handshake...");
        if let Err(e) = tls.open(context) {
            drop(tls);
            // SAFETY: Reclaim leaked buffers after TLS is dropped.
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
            }
            // SAFETY: Close socket on handshake failure.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("TLS handshake: {e:?}"));
        }

        io_log("[IO-TLS] TLS 1.3 handshake OK");

        // Send HTTP/1.1 GET request.
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             User-Agent: oasis-psp/1.0\r\n\
             Accept: */*\r\n\
             Connection: keep-alive\r\n\r\n"
        );
        if let Err(e) =
            embedded_io::Write::write_all(&mut tls, request.as_bytes())
                .and_then(|_| embedded_io::Write::flush(&mut tls))
        {
            drop(tls);
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }
            return Err(format!("TLS write: {e:?}"));
        }

        io_log("[IO-TLS] HTTP GET sent, reading response headers...");

        // Read response headers (up to 8KB).
        let mut hdr_buf = vec![0u8; 8192];
        let mut hdr_len = 0usize;
        loop {
            if hdr_len >= hdr_buf.len() {
                break;
            }
            match embedded_io::Read::read(
                &mut tls,
                &mut hdr_buf[hdr_len..],
            ) {
                Ok(0) => break,
                Ok(n) => {
                    hdr_len += n;
                    if let Some(_end) =
                        find_header_end(&hdr_buf[..hdr_len])
                    {
                        break;
                    }
                },
                Err(e) => {
                    drop(tls);
                    unsafe {
                        let _ = Box::from_raw(read_buf_ptr);
                        let _ = Box::from_raw(write_buf_ptr);
                        psp::sys::sceNetInetClose(fd);
                    }
                    return Err(format!("TLS read headers: {e:?}"));
                },
            }
        }

        let header_end = find_header_end(&hdr_buf[..hdr_len])
            .ok_or_else(|| "incomplete HTTP headers".to_string())?;

        let hdr_str =
            core::str::from_utf8(&hdr_buf[..header_end]).unwrap_or("");
        io_log(&format!(
            "[IO-TLS] response: {}",
            hdr_str.lines().next().unwrap_or("?")
        ));

        // Check status code (first line: "HTTP/1.1 200 OK").
        let status = hdr_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // Handle redirects (follow up to 5).
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = hdr_str.lines().find_map(|l| {
                if l.len() > 9
                    && l[..9].eq_ignore_ascii_case("location:")
                {
                    l.split_once(':').map(|(_, v)| v.trim().to_string())
                } else {
                    None
                }
            });

            // Clean up current connection.
            drop(tls);
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }

            if let Some(loc) = location {
                if redirects_left == 0 {
                    return Err("too many TLS redirects".into());
                }
                io_log(&format!("[IO-TLS] redirect → {loc}"));
                return Self::open_with_redirects(
                    &loc,
                    redirects_left - 1,
                );
            }
            return Err(format!("redirect {status}, no Location"));
        }

        if status < 200 || status >= 300 {
            drop(tls);
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }
            return Err(format!("HTTP {status}"));
        }

        // Parse Content-Length.
        let content_length: u64 = hdr_str
            .lines()
            .find_map(|l| {
                if l.len() > 15
                    && l[..15].eq_ignore_ascii_case("content-length:")
                {
                    l[15..].trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        io_log(&format!(
            "[IO-TLS] status={status} content-length={content_length}"
        ));

        // Any leftover body data after headers.
        let leftover = hdr_buf[header_end..hdr_len].to_vec();

        Ok((
            Self {
                tls,
                fd,
                read_buf_ptr,
                write_buf_ptr,
                leftover,
            },
            content_length,
        ))
    }

    /// Read body data. Returns leftover data first, then reads from TLS.
    fn read_data(&mut self, buf: &mut [u8]) -> Result<i32, String> {
        if !self.leftover.is_empty() {
            let take = core::cmp::min(self.leftover.len(), buf.len());
            buf[..take].copy_from_slice(&self.leftover[..take]);
            self.leftover.drain(..take);
            return Ok(take as i32);
        }

        match embedded_io::Read::read(&mut self.tls, buf) {
            Ok(n) => Ok(n as i32),
            Err(_) => Ok(0), // treat errors as EOF
        }
    }

    /// Clean up: drop TLS, free buffers, close socket.
    fn cleanup(self) {
        let Self {
            tls,
            fd,
            read_buf_ptr,
            write_buf_ptr,
            ..
        } = self;
        drop(tls);
        // SAFETY: Buffers were created via Box::leak and are freed
        // exactly once here. Socket fd is valid and open.
        unsafe {
            let _ = Box::from_raw(read_buf_ptr);
            let _ = Box::from_raw(write_buf_ptr);
            psp::sys::sceNetInetClose(fd);
        }
    }
}

/// Connect to an internet radio stream via raw TCP + HTTP.
///
/// Sends an HTTP GET with `Icy-MetaData: 1`, reads headers to extract
/// `icy-metaint`, then passes the connected socket fd to the audio thread.
fn handle_radio_connect(url: String) {
    use std::ffi::c_void;

    // Initialize network.
    if let Err(e) = crate::network::ensure_net_init_pub() {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: format!("net init: {e}"),
        });
        return;
    }

    // Parse URL.
    let (host, port, path) = match parse_radio_url(&url) {
        Some(v) => v,
        None => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("bad URL: {url}"),
            });
            return;
        },
    };

    // DNS resolve.
    let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
    host_bytes.push(0);
    let addr = match psp::net::resolve_hostname(&host_bytes) {
        Ok(a) => a,
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("DNS: {e}"),
            });
            return;
        },
    };

    // Create TCP socket.
    // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
    let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
    if fd < 0 {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "socket() failed".into(),
        });
        return;
    }

    // Connect.
    let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
    // SAFETY: Connect to the resolved address.
    let ret = unsafe {
        psp::sys::sceNetInetConnect(fd, &sa, core::mem::size_of::<psp::sys::sockaddr>() as u32)
    };
    if ret < 0 {
        // SAFETY: fd is a valid socket descriptor; closing on connect failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: format!("connect {}:{} failed", host, port),
        });
        return;
    }

    // Send HTTP GET with ICY metadata request.
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nIcy-MetaData: 1\r\n\
         User-Agent: OASIS_OS/1.0\r\nAccept: */*\r\n\r\n",
        path, host,
    );
    let req_bytes = request.as_bytes();
    // SAFETY: Send the HTTP request over the connected socket.
    let sent = unsafe {
        psp::sys::sceNetInetSend(fd, req_bytes.as_ptr() as *const c_void, req_bytes.len(), 0)
    };
    if sent <= 0 {
        // SAFETY: fd is a valid socket descriptor; closing on send failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "send failed".into(),
        });
        return;
    }

    // Read response headers (up to 4KB).
    let mut hdr_buf = vec![0u8; 4096];
    let mut hdr_len = 0usize;
    let mut attempts = 0;
    while hdr_len < hdr_buf.len() && attempts < 200 {
        // SAFETY: Blocking recv for header data.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                fd,
                hdr_buf.as_mut_ptr().add(hdr_len) as *mut c_void,
                (hdr_buf.len() - hdr_len).min(512),
                0,
            )
        };
        if n > 0 {
            hdr_len += n as usize;
            // Check for end of headers.
            if hdr_len >= 4 {
                let search_start = if hdr_len > n as usize + 3 {
                    hdr_len - n as usize - 3
                } else {
                    0
                };
                let haystack = &hdr_buf[search_start..hdr_len];
                if find_header_end(haystack).is_some() {
                    break;
                }
            }
        } else if n == 0 {
            break; // Connection closed.
        } else {
            attempts += 1;
            psp::thread::sleep_ms(20);
        }
    }

    // Validate that we received a complete header (with \r\n\r\n terminator).
    let header_end = if hdr_len > 0 {
        find_header_end(&hdr_buf[..hdr_len])
    } else {
        None
    };

    if header_end.is_none() {
        // SAFETY: fd is a valid socket descriptor; closing on incomplete headers.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "incomplete headers".into(),
        });
        return;
    }

    let header_end = header_end.unwrap();

    // Parse icy-metaint from headers.
    let hdr_str = String::from_utf8_lossy(&hdr_buf[..hdr_len]);
    let icy_metaint = parse_icy_metaint(&hdr_str);

    // Extract any leftover audio data after the header boundary.
    let initial_data = hdr_buf[header_end..hdr_len].to_vec();

    // Set non-blocking for streaming.
    let nb: i32 = 1;
    // SAFETY: SO_NONBLOCK is a PSP-specific socket option.
    unsafe {
        psp::sys::sceNetInetSetsockopt(
            fd,
            0xFFFF, // SOL_SOCKET
            0x0080, // SO_NONBLOCK
            &nb as *const i32 as *const c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }

    let _ = IO_RESP_QUEUE.push(IoResponse::RadioConnected {
        fd,
        icy_metaint,
        initial_data,
    });
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
