//! Background worker threads (audio + I/O on separate threads).
//!
//! Uses `psp::thread::ThreadBuilder` for native PSP kernel threads with
//! priority tuning. Communication uses lock-free `SpscQueue` for commands
//! and `SpinMutex` for shared state readable from the main thread.

use psp::sync::{SpinMutex, SpscQueue};
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
// Shared audio state (SpinMutex for richer state than bare atomics)
// ---------------------------------------------------------------------------

/// Shared audio state protected by a spinlock.
///
/// Readable from the main thread and written by the audio thread.
/// PSP is single-core so SpinMutex has near-zero overhead for short
/// critical sections.
static SHARED_AUDIO: SpinMutex<SharedAudioState> =
    SpinMutex::new(SharedAudioState::new());

/// Audio state shared between the audio thread and main thread.
#[derive(Clone)]
pub struct SharedAudioState {
    pub playing: bool,
    pub paused: bool,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub channels: u32,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub track_name: [u8; 64],
    pub track_name_len: usize,
}

impl SharedAudioState {
    const fn new() -> Self {
        Self {
            playing: false,
            paused: false,
            sample_rate: 0,
            bitrate: 0,
            channels: 0,
            position_ms: 0,
            duration_ms: 0,
            track_name: [0u8; 64],
            track_name_len: 0,
        }
    }
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
    PlaySfx(SfxId),
    Shutdown,
}

/// Handle to the background audio thread (reads from SHARED_AUDIO).
pub struct AudioHandle;

impl AudioHandle {
    /// Send a command to the audio thread.
    pub fn send(&self, cmd: AudioCmd) {
        let _ = AUDIO_QUEUE.push(cmd);
    }

    /// Snapshot the current audio state (short spinlock hold).
    pub fn state(&self) -> SharedAudioState {
        SHARED_AUDIO.lock().clone()
    }

    pub fn is_playing(&self) -> bool {
        SHARED_AUDIO.lock().playing
    }

    pub fn is_paused(&self) -> bool {
        SHARED_AUDIO.lock().paused
    }

    pub fn sample_rate(&self) -> u32 {
        SHARED_AUDIO.lock().sample_rate
    }

    pub fn bitrate(&self) -> u32 {
        SHARED_AUDIO.lock().bitrate
    }

    pub fn channels(&self) -> u32 {
        SHARED_AUDIO.lock().channels
    }

    pub fn position_ms(&self) -> u64 {
        SHARED_AUDIO.lock().position_ms
    }

    pub fn duration_ms(&self) -> u64 {
        SHARED_AUDIO.lock().duration_ms
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
    ReadFile { path: String },
    HttpGet { url: String, tag: u32 },
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
    FileReady { path: String, data: Vec<u8> },
    HttpDone {
        tag: u32,
        status_code: u16,
        body: Vec<u8>,
    },
    Error { path: String, msg: String },
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
                    SHARED_AUDIO.lock().playing = false;
                }
            }
            Some(AudioCmd::LoadAndPlayData(data)) => {
                if player.load_and_play_data(&data) {
                    publish_audio_state(&player);
                } else {
                    SHARED_AUDIO.lock().playing = false;
                }
            }
            Some(AudioCmd::Pause) => {
                if player.is_playing() && !player.is_paused() {
                    player.toggle_pause();
                    SHARED_AUDIO.lock().paused = true;
                }
            }
            Some(AudioCmd::Resume) => {
                if player.is_playing() && player.is_paused() {
                    player.toggle_pause();
                    SHARED_AUDIO.lock().paused = false;
                }
            }
            Some(AudioCmd::Stop) => {
                player.stop();
                let mut state = SHARED_AUDIO.lock();
                state.playing = false;
                state.paused = false;
            }
            Some(AudioCmd::PlaySfx(id)) => {
                if let Some(sfx) = &sfx {
                    sfx.play(id);
                }
            }
            Some(AudioCmd::Shutdown) => {
                player.stop();
                SHARED_AUDIO.lock().playing = false;
                break;
            }
            None => {}
        }

        if player.is_playing() && !player.is_paused() {
            // update() contains the blocking sceAudioOutputBlocking call.
            player.update();
            // Publish position each frame.
            {
                let mut state = SHARED_AUDIO.lock();
                state.position_ms = player.position_ms();
                state.duration_ms = player.duration_ms();
            }
            if !player.is_playing() {
                SHARED_AUDIO.lock().playing = false;
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

/// Publish audio player state to the shared spinlock after a load_and_play.
fn publish_audio_state(player: &AudioPlayer) {
    let mut state = SHARED_AUDIO.lock();
    state.playing = true;
    state.paused = false;
    state.sample_rate = player.sample_rate;
    state.bitrate = player.bitrate;
    state.channels = player.channels;
    state.position_ms = 0;
    state.duration_ms = 0;
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
            }
            Some(IoCmd::ReadFile { path }) => {
                handle_read_file(path);
            }
            Some(IoCmd::HttpGet { url, tag }) => {
                handle_http_get(url, tag);
            }
            Some(IoCmd::Shutdown) => break,
            None => {
                // Sleep when idle to avoid spinning.
                psp::thread::sleep_ms(10);
            }
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
            }
            None => {
                let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                    path,
                    msg: "JPEG decode failed".into(),
                });
            }
        },
        Err(_) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: "file read failed".into(),
            });
        }
    }
}

fn handle_read_file(path: String) {
    match psp::io::read_to_vec(&path) {
        Ok(data) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::FileReady { path, data });
        }
        Err(_) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: "file not found".into(),
            });
        }
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
            }
            Err(e) => {
                let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                    path: url,
                    msg: format!("HTTP GET: {e}"),
                });
            }
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path: url,
                msg: format!("HTTP init: {e}"),
            });
        }
    }
}
