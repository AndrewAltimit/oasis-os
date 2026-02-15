//! Background worker threads (audio + I/O on separate threads).
//!
//! Uses `psp::thread::ThreadBuilder` for native PSP kernel threads with
//! priority tuning. Communication uses lock-free `SpscQueue` for commands
//! and bare atomics for shared state (lock-free to avoid priority inversion
//! on single-core PSP where a high-priority audio thread could starve the
//! main thread if both contend on a spinlock).

use std::sync::Arc;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;

use crate::audio::AudioPlayer;
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
// Audio commands
// ---------------------------------------------------------------------------

/// Commands for the dedicated audio thread.
pub enum AudioCmd {
    LoadAndPlay(String),
    LoadAndPlayData(Arc<Vec<u8>>),
    Pause,
    Resume,
    Stop,
    SetVolume(u8),
    PlaySfx(SfxId),
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
}

/// Send an audio command from any context.
pub fn send_audio_cmd(cmd: AudioCmd) {
    let _ = AUDIO_QUEUE.push(cmd);
}

// ---------------------------------------------------------------------------
// I/O commands and responses
// ---------------------------------------------------------------------------

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
/// Returns handles for audio state and I/O responses.
pub fn spawn_workers() -> (AudioHandle, IoHandle) {
    // Audio thread: high priority (16) for low-latency playback.
    let audio_result = ThreadBuilder::new(b"oasis_audio\0")
        .priority(16)
        .spawn(move || {
            audio_thread_fn();
            0
        });
    if let Err(e) = &audio_result {
        psp::dprintln!("OASIS_OS: Failed to spawn audio thread: {:?}", e);
    }

    // I/O thread: normal priority (32) for file operations.
    let io_result = ThreadBuilder::new(b"oasis_io\0")
        .priority(32)
        .spawn(move || {
            io_thread_fn();
            0
        });
    if let Err(e) = &io_result {
        psp::dprintln!("OASIS_OS: Failed to spawn I/O thread: {:?}", e);
    }

    (AudioHandle, IoHandle)
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

/// Dedicated audio thread: MP3 playback + SFX mixing.
fn audio_thread_fn() {
    let mut player = AudioPlayer::new();
    if !player.init() {
        psp::dprintln!("OASIS_OS: Audio thread init failed");
    }

    let mut sfx = SfxEngine::new();
    if sfx.is_none() {
        psp::dprintln!("OASIS_OS: SFX engine init failed (non-fatal)");
    }

    loop {
        match AUDIO_QUEUE.pop() {
            Some(AudioCmd::LoadAndPlay(path)) => {
                if player.load_and_play(&path) {
                    publish_audio_state(&player);
                } else {
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::LoadAndPlayData(data)) => {
                if player.load_and_play_data(&data) {
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
            },
            Some(AudioCmd::SetVolume(v)) => {
                player.set_volume(v);
            },
            Some(AudioCmd::PlaySfx(id)) => {
                if let Some(sfx) = &sfx {
                    sfx.play(id);
                }
            },
            Some(AudioCmd::Shutdown) => {
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                break;
            },
            None => {},
        }

        if player.is_playing() && !player.is_paused() {
            // update() contains the blocking sceAudioOutputBlocking call.
            player.update();
            // Publish position each frame (lock-free atomic stores).
            AUDIO_POSITION_MS.store(player.position_ms() as u32, Ordering::Relaxed);
            AUDIO_DURATION_MS.store(player.duration_ms() as u32, Ordering::Relaxed);
            if !player.is_playing() {
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
            }
        } else {
            // Sleep when idle to avoid spinning.
            psp::thread::sleep_ms(10);
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

/// Dedicated I/O thread: file reads and JPEG decoding.
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
        Ok(client) => match client.get(&url_bytes) {
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
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path: url,
                msg: format!("HTTP init: {e}"),
            });
        },
    }
}
