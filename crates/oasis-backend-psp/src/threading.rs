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

/// Dedicated audio thread: MP3 playback + SFX mixing + radio streaming.
fn audio_thread_fn() {
    let mut player = AudioPlayer::new();
    player.init();

    let mut sfx = SfxEngine::new();
    let mut radio: Option<RadioStreamer> = None;

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
            // SAFETY: sceKernelDelayThread sleeps the current thread.
            unsafe { psp::sys::sceKernelDelayThread(10_000) };
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
