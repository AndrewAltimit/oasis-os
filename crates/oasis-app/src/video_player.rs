//! In-app video player with multiple decode backends.
//!
//! Supports two decode strategies:
//! - **Ffmpeg** (default): spawns two ffmpeg subprocesses (video→RGBA, audio→MP3)
//! - **Software** (feature `video-decode`): uses `oasis-video` for pure-Rust
//!   MP4/H.264+AAC decoding from a local file
//!
//! Background reader/decode threads pipe frames and audio to the main thread
//! via `mpsc` channels. The main loop uploads video frames as SDL textures and
//! feeds audio to the audio backend.

#[cfg(not(feature = "_video"))]
use std::process::{Child, Command, Stdio};
#[cfg(not(feature = "_video"))]
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(not(feature = "_video"))]
use std::time::Duration;
use std::time::Instant;

use oasis_core::backend::{SdiBackend, TextureId};

/// Raw RGBA frame from a decode backend.
struct VideoFrame {
    data: Vec<u8>,
    #[cfg(feature = "_video")]
    width: u32,
    #[cfg(feature = "_video")]
    height: u32,
    /// Presentation timestamp in seconds (software decode only; 0.0 for ffmpeg).
    #[cfg(feature = "_video")]
    timestamp_secs: f64,
}

/// Player lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerState {
    Idle,
    Starting,
    Playing,
    Error,
}

/// Target decode framerate (must match `-r` flag passed to ffmpeg).
#[cfg(not(feature = "_video"))]
const VIDEO_FPS: u32 = 15;

/// Audio output from a single tick.
pub enum AudioOutput {
    /// MP3-encoded chunks (ffmpeg path).
    #[cfg(not(feature = "_video"))]
    Mp3Chunks(Vec<Vec<u8>>),
    /// Decoded PCM f32 samples (software decode path).
    #[cfg(feature = "_video")]
    PcmF32(Vec<SoftwareAudio>),
    /// No audio this tick.
    None,
}

/// A chunk of decoded PCM f32 audio from the software decoder.
#[cfg(feature = "_video")]
pub struct SoftwareAudio {
    pub pcm_f32: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Which decode backend is active.
enum DecodeBackend {
    #[cfg(not(feature = "_video"))]
    Ffmpeg {
        video_process: Child,
        audio_process: Child,
        video_rx: Receiver<VideoFrame>,
        audio_rx: Receiver<Vec<u8>>,
    },
    #[cfg(feature = "_video")]
    Software {
        video_rx: Receiver<VideoFrame>,
        audio_rx: Receiver<SoftwareAudio>,
        stop_tx: std::sync::mpsc::Sender<()>,
    },
}

/// Manages video+audio playback using either ffmpeg or software decoding.
pub struct VideoPlayer {
    state: PlayerState,
    decode: Option<DecodeBackend>,
    current_texture: Option<TextureId>,
    frame_width: u32,
    frame_height: u32,
    error_msg: Option<String>,
    /// Set when the decode thread exits cleanly (EOF or error, not stop signal).
    finished: bool,
    /// When the last video frame was displayed (for pacing).
    last_frame_time: Option<Instant>,
    /// Number of frames displayed (for diagnostics).
    displayed_frames: u64,
    /// Last time a display stats log was emitted.
    last_display_report: Option<Instant>,
    /// Minimum interval between displayed frames (1 / VIDEO_FPS).
    #[cfg(not(feature = "_video"))]
    frame_interval: Duration,
    /// Wall-clock time when the first frame was displayed (PTS sync).
    #[cfg(feature = "_video")]
    playback_start: Option<Instant>,
    /// PTS of the first frame received (base for wall-clock sync).
    #[cfg(feature = "_video")]
    base_pts: f64,
    /// Frame waiting to be displayed (held until wall-clock catches up to its PTS).
    #[cfg(feature = "_video")]
    pending_frame: Option<VideoFrame>,
}

impl VideoPlayer {
    /// Create a new idle video player.
    pub fn new() -> Self {
        Self {
            state: PlayerState::Idle,
            decode: None,
            current_texture: None,
            frame_width: 0,
            frame_height: 0,
            error_msg: None,
            finished: false,
            last_frame_time: None,
            displayed_frames: 0,
            last_display_report: None,
            #[cfg(not(feature = "_video"))]
            frame_interval: Duration::from_nanos(1_000_000_000 / u64::from(VIDEO_FPS)),
            #[cfg(feature = "_video")]
            playback_start: None,
            #[cfg(feature = "_video")]
            base_pts: 0.0,
            #[cfg(feature = "_video")]
            pending_frame: None,
        }
    }

    /// Start playing a video from a URL using ffmpeg subprocesses.
    ///
    /// If `seek_secs > 0`, seeks into the stream before decoding.
    #[cfg(not(feature = "_video"))]
    pub fn start(&mut self, url: &str, seek_secs: u64, width: u32, height: u32) {
        self.stop_internal();

        // Check that ffmpeg is available.
        let ffmpeg_ok = Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !ffmpeg_ok {
            log::warn!("VideoPlayer: ffmpeg not found, cannot play video");
            self.state = PlayerState::Error;
            self.error_msg = Some("ffmpeg not installed".to_string());
            return;
        }

        self.frame_width = width;
        self.frame_height = height;

        // Build seek args.
        let mut seek_args: Vec<String> = Vec::new();
        if seek_secs > 0 {
            seek_args.push("-ss".to_string());
            seek_args.push(seek_secs.to_string());
        }

        // Spawn video ffmpeg: decode to raw RGBA frames at 15fps.
        let video_result = Command::new("ffmpeg")
            .args(&seek_args)
            .args([
                "-i",
                url,
                "-vf",
                &format!("scale={width}:{height}"),
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "-r",
                "15",
                "-v",
                "quiet",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn();

        let mut video_child = match video_result {
            Ok(child) => child,
            Err(e) => {
                log::error!("VideoPlayer: failed to spawn video ffmpeg: {e}");
                self.state = PlayerState::Error;
                self.error_msg = Some(format!("ffmpeg spawn: {e}"));
                return;
            },
        };

        // Spawn audio ffmpeg: decode to MP3 stream.
        let audio_result = Command::new("ffmpeg")
            .args(&seek_args)
            .args([
                "-i", url, "-vn", "-ar", "44100", "-f", "mp3", "-b:a", "128k", "-v", "quiet",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn();

        let mut audio_child = match audio_result {
            Ok(child) => child,
            Err(e) => {
                log::error!("VideoPlayer: failed to spawn audio ffmpeg: {e}");
                let _ = video_child.kill();
                let _ = video_child.wait();
                self.state = PlayerState::Error;
                self.error_msg = Some(format!("ffmpeg audio spawn: {e}"));
                return;
            },
        };

        // Take stdout handles before moving children.
        let video_stdout = video_child
            .stdout
            .take()
            .expect("video child stdout must be piped");
        let audio_stdout = audio_child
            .stdout
            .take()
            .expect("audio child stdout must be piped");

        // Video reader thread: reads exact frame-sized chunks.
        let frame_size = (width * height * 4) as usize;
        let (video_tx, video_rx): (SyncSender<VideoFrame>, Receiver<VideoFrame>) =
            mpsc::sync_channel(2);
        std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = std::io::BufReader::new(video_stdout);
            loop {
                let mut buf = vec![0u8; frame_size];
                match reader.read_exact(&mut buf) {
                    Ok(()) => {
                        let frame = VideoFrame {
                            data: buf,
                            #[cfg(feature = "_video")]
                            width: 0,
                            #[cfg(feature = "_video")]
                            height: 0,
                            #[cfg(feature = "_video")]
                            timestamp_secs: 0.0,
                        };
                        if video_tx.send(frame).is_err() {
                            break; // Receiver dropped.
                        }
                    },
                    Err(_) => break, // EOF or error.
                }
            }
            log::debug!("VideoPlayer: video reader thread exited");
        });

        // Audio reader thread: reads variable-size chunks.
        let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = std::io::BufReader::new(audio_stdout);
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF.
                    Ok(n) => {
                        if audio_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    },
                    Err(_) => break,
                }
            }
            log::debug!("VideoPlayer: audio reader thread exited");
        });

        self.decode = Some(DecodeBackend::Ffmpeg {
            video_process: video_child,
            audio_process: audio_child,
            video_rx,
            audio_rx,
        });
        self.state = PlayerState::Starting;

        log::info!("VideoPlayer: started {width}x{height} seek={seek_secs}s url={url}");
    }

    /// Start playing a video from a local file using the software decoder.
    ///
    /// Requires the `video-decode` feature. Opens the file as a streaming
    /// source and spawns a background decode thread.
    #[cfg(feature = "_video")]
    pub fn start_software(
        &mut self,
        path: std::path::PathBuf,
        seek_secs: u64,
        width: u32,
        height: u32,
    ) {
        self.stop_internal();

        self.frame_width = width;
        self.frame_height = height;

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                log::error!("VideoPlayer: failed to open {}: {e}", path.display());
                self.state = PlayerState::Error;
                self.error_msg = Some(format!("open file: {e}"));
                return;
            },
        };

        // For local files, open the decoder on the main thread (fast I/O).
        let mut decoder = match oasis_video::SoftwareVideoDecoder::open_stream(Box::new(file)) {
            Ok(d) => d,
            Err(e) => {
                log::error!("VideoPlayer: failed to open decoder: {e}");
                self.state = PlayerState::Error;
                self.error_msg = Some(format!("decoder: {e}"));
                return;
            },
        };

        if seek_secs > 0
            && let Err(e) = decoder.seek(seek_secs as f64)
        {
            log::warn!("VideoPlayer: seek to {seek_secs}s failed: {e}");
        }

        self.spawn_decode_thread(decoder, width, height);
        log::info!(
            "VideoPlayer: software decode started {}x{} seek={seek_secs}s file={}",
            width,
            height,
            path.display(),
        );
    }

    /// Start playing a video from any `VideoSource` using the software decoder.
    ///
    /// Both decoder initialization and decoding run on a background thread
    /// so the main/UI thread is never blocked. The optional `on_init`
    /// callback runs after the decoder's initial scan completes (used to
    /// enable sliding-window eviction on streaming buffers).
    #[cfg(feature = "_video")]
    pub fn start_software_source(
        &mut self,
        source: Box<dyn oasis_video::VideoSource>,
        seek_secs: u64,
        width: u32,
        height: u32,
        on_init: Option<Box<dyn FnOnce() + Send>>,
    ) {
        self.stop_internal();

        self.frame_width = width;
        self.frame_height = height;

        let (video_tx, video_rx) = mpsc::sync_channel::<VideoFrame>(4);
        let (audio_tx, audio_rx) = mpsc::sync_channel::<SoftwareAudio>(64);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let target_w = width;
        let target_h = height;

        std::thread::spawn(move || {
            // Open decoder on background thread to avoid blocking the UI.
            log::info!("VideoPlayer: opening decoder (streaming source)...");
            let t0 = std::time::Instant::now();
            let mut decoder = match oasis_video::SoftwareVideoDecoder::open_stream(source) {
                Ok(d) => {
                    log::info!(
                        "VideoPlayer: decoder opened in {:.1}s",
                        t0.elapsed().as_secs_f64(),
                    );
                    d
                },
                Err(e) => {
                    log::error!(
                        "VideoPlayer: failed to open decoder after {:.1}s: {e}",
                        t0.elapsed().as_secs_f64(),
                    );
                    return;
                },
            };

            // Decoder initialized — invoke callback (e.g. enable eviction).
            if let Some(cb) = on_init {
                cb();
            }

            if seek_secs > 0
                && let Err(e) = decoder.seek(seek_secs as f64)
            {
                log::warn!("VideoPlayer: seek to {seek_secs}s failed: {e}");
            }

            Self::decode_loop(decoder, video_tx, audio_tx, stop_rx, target_w, target_h);
        });

        self.decode = Some(DecodeBackend::Software {
            video_rx,
            audio_rx,
            stop_tx,
        });
        self.state = PlayerState::Starting;
    }

    /// Spawn the decode loop for an already-opened decoder.
    #[cfg(feature = "_video")]
    fn spawn_decode_thread(
        &mut self,
        decoder: oasis_video::SoftwareVideoDecoder,
        width: u32,
        height: u32,
    ) {
        let (video_tx, video_rx) = mpsc::sync_channel::<VideoFrame>(4);
        let (audio_tx, audio_rx) = mpsc::sync_channel::<SoftwareAudio>(64);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        std::thread::spawn(move || {
            Self::decode_loop(decoder, video_tx, audio_tx, stop_rx, width, height);
        });

        self.decode = Some(DecodeBackend::Software {
            video_rx,
            audio_rx,
            stop_tx,
        });
        self.state = PlayerState::Starting;
    }

    /// Drain all buffered audio from the decoder and send it.
    ///
    /// Uses `next_buffered_audio()` to avoid reading new packets from the
    /// stream, which would advance the read position and potentially cause
    /// the video decoder to miss data or hit premature EOF.
    /// Returns the number of chunks sent and dropped.
    #[cfg(feature = "_video")]
    fn drain_audio(
        decoder: &mut oasis_video::SoftwareVideoDecoder,
        audio_tx: &mpsc::SyncSender<SoftwareAudio>,
        has_audio: bool,
    ) -> (u64, u64) {
        if !has_audio {
            return (0, 0);
        }
        let mut sent = 0u64;
        let mut dropped = 0u64;
        while let Some(chunk) = decoder.next_buffered_audio() {
            match audio_tx.try_send(SoftwareAudio {
                pcm_f32: chunk.pcm_f32,
                channels: chunk.channels,
                sample_rate: chunk.sample_rate,
            }) {
                Ok(()) => sent += 1,
                Err(_) => dropped += 1,
            }
        }
        (sent, dropped)
    }

    /// Run the decode loop (called on a background thread).
    ///
    /// Decodes video and audio from the same decoder, draining all available
    /// audio after each video frame to prevent audio starvation. Falls back
    /// to audio-only mode if H.264 decoding fails repeatedly.
    #[cfg(feature = "_video")]
    fn decode_loop(
        mut decoder: oasis_video::SoftwareVideoDecoder,
        video_tx: mpsc::SyncSender<VideoFrame>,
        audio_tx: mpsc::SyncSender<SoftwareAudio>,
        stop_rx: mpsc::Receiver<()>,
        target_w: u32,
        target_h: u32,
    ) {
        log::info!("VideoPlayer: software decode thread started");

        // Detect whether audio track exists to avoid repeated NoTrack errors.
        let (audio_rate, audio_ch) = decoder.audio_format();
        let has_audio = audio_rate > 0 && audio_ch > 0;
        if has_audio {
            log::info!(
                "VideoPlayer: audio track: {audio_rate}Hz, {audio_ch}ch",
            );
        } else {
            log::warn!("VideoPlayer: no audio track found in video");
        }

        let mut frame_count = 0u64;
        let mut audio_sent = 0u64;
        let mut audio_dropped = 0u64;
        let decode_start = Instant::now();
        let mut last_report = Instant::now();
        let mut video_failed = false;
        let mut skip_limit_count = 0u32;

        loop {
            if stop_rx.try_recv().is_ok() {
                log::info!("VideoPlayer: stop signal received");
                break;
            }

            // --- Video decode (unless in audio-only fallback) ---
            if !video_failed {
                let t0 = Instant::now();
                match decoder.next_video_frame() {
                    Ok(Some(frame)) => {
                        let decode_ms = t0.elapsed().as_millis();
                        frame_count += 1;
                        if frame_count <= 5 || frame_count.is_multiple_of(200) {
                            log::info!(
                                "VideoPlayer: frame {frame_count}: {}x{} ts={:.2}s \
                                 decode={decode_ms}ms",
                                frame.width,
                                frame.height,
                                frame.timestamp_secs,
                            );
                        }
                        let ts = frame.timestamp_secs;
                        let (data, w, h) =
                            if frame.width == target_w && frame.height == target_h {
                                (frame.rgba, frame.width, frame.height)
                            } else {
                                (
                                    simple_scale(
                                        &frame.rgba,
                                        frame.width,
                                        frame.height,
                                        target_w,
                                        target_h,
                                    ),
                                    target_w,
                                    target_h,
                                )
                            };
                        if video_tx
                            .send(VideoFrame {
                                data,
                                width: w,
                                height: h,
                                timestamp_secs: ts,
                            })
                            .is_err()
                        {
                            log::info!("VideoPlayer: video receiver dropped");
                            break;
                        }
                    },
                    Ok(None) => {
                        log::info!(
                            "VideoPlayer: video EOF after {frame_count} frames in {:.1}s",
                            decode_start.elapsed().as_secs_f64(),
                        );
                        // Drain remaining audio before exiting.
                        let (s, d) = Self::drain_audio(&mut decoder, &audio_tx, has_audio);
                        audio_sent += s;
                        audio_dropped += d;
                        break;
                    },
                    Err(oasis_video::VideoError::NoTrack(_)) => {
                        log::warn!("VideoPlayer: no video track, switching to audio-only");
                        video_failed = true;
                    },
                    Err(oasis_video::VideoError::SkipLimit) => {
                        skip_limit_count += 1;
                        if skip_limit_count >= 3 {
                            log::warn!(
                                "VideoPlayer: H.264 skip limit x{skip_limit_count} after \
                                 {frame_count} frames, switching to audio-only mode"
                            );
                            video_failed = true;
                        } else {
                            log::warn!(
                                "VideoPlayer: H.264 skip limit ({skip_limit_count}/3) after \
                                 {frame_count} frames, will retry"
                            );
                        }
                    },
                    Err(e) => {
                        log::error!(
                            "VideoPlayer: video decode error after {frame_count} frames: {e}",
                        );
                        video_failed = true;
                    },
                }
            }

            // --- Audio: drain ALL available packets after each video frame ---
            let (s, d) = Self::drain_audio(&mut decoder, &audio_tx, has_audio);
            audio_sent += s;
            audio_dropped += d;

            // In audio-only mode, sleep briefly to avoid busy-spinning.
            if video_failed && s == 0 {
                // No audio produced — we're at EOF or stream is stalled.
                if !has_audio {
                    log::info!("VideoPlayer: no video and no audio, exiting");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            // --- Periodic diagnostics ---
            if last_report.elapsed().as_millis() >= 2000 {
                let elapsed = decode_start.elapsed().as_secs_f64();
                let vfps = frame_count as f64 / elapsed.max(0.001);
                log::info!(
                    "VideoPlayer: {elapsed:.1}s: {frame_count} video ({vfps:.1} fps), \
                     {audio_sent} audio sent, {audio_dropped} dropped{}",
                    if video_failed { " [audio-only]" } else { "" },
                );
                last_report = Instant::now();
            }
        }
        log::info!(
            "VideoPlayer: decode thread exited: {frame_count} video frames, \
             {audio_sent} audio chunks sent, {audio_dropped} dropped",
        );
    }

    /// Tick the video player: drain channels, upload latest frame, collect audio.
    ///
    /// Returns `(current_texture, audio_output)`. The caller should feed
    /// audio to the audio backend and assign the texture to the guide.
    pub fn tick(&mut self, backend: &mut impl SdiBackend) -> (Option<TextureId>, AudioOutput) {
        if self.state != PlayerState::Starting && self.state != PlayerState::Playing {
            return (self.current_texture, AudioOutput::None);
        }

        let Some(ref decode) = self.decode else {
            return (self.current_texture, AudioOutput::None);
        };

        // Take at most one video frame, paced appropriately per backend.
        let mut latest_frame: Option<VideoFrame> = None;
        let mut video_disconnected = false;

        match decode {
            #[cfg(not(feature = "_video"))]
            DecodeBackend::Ffmpeg { video_rx, .. } => {
                // Fixed framerate pacing for ffmpeg (no timestamps).
                let should_take = self
                    .last_frame_time
                    .is_none_or(|t| t.elapsed() >= self.frame_interval);
                if should_take {
                    match video_rx.try_recv() {
                        Ok(frame) => latest_frame = Some(frame),
                        Err(TryRecvError::Empty) => {},
                        Err(TryRecvError::Disconnected) => video_disconnected = true,
                    }
                }
            },
            #[cfg(feature = "_video")]
            DecodeBackend::Software { video_rx, .. } => {
                // PTS-based frame pacing: only display when wall-clock
                // time has caught up to the frame's presentation timestamp.

                // Try to fill pending_frame from the channel if empty.
                if self.pending_frame.is_none() {
                    match video_rx.try_recv() {
                        Ok(frame) => self.pending_frame = Some(frame),
                        Err(TryRecvError::Empty) => {},
                        Err(TryRecvError::Disconnected) => video_disconnected = true,
                    }
                }

                if let Some(ref pending) = self.pending_frame {
                    let should_display = match self.playback_start {
                        Some(start) => {
                            let wall_elapsed = start.elapsed().as_secs_f64();
                            let frame_pts = pending.timestamp_secs - self.base_pts;
                            frame_pts <= wall_elapsed
                        },
                        // First frame — always display immediately.
                        None => true,
                    };

                    if should_display {
                        latest_frame = self.pending_frame.take();

                        // Skip frames that are already behind schedule
                        // (catch up if decode is ahead of display).
                        loop {
                            match video_rx.try_recv() {
                                Ok(next) => {
                                    if let Some(start) = self.playback_start {
                                        let wall_elapsed = start.elapsed().as_secs_f64();
                                        let next_pts = next.timestamp_secs - self.base_pts;
                                        if next_pts <= wall_elapsed {
                                            // This frame is also due — skip to it.
                                            latest_frame = Some(next);
                                            continue;
                                        }
                                    }
                                    // Next frame is in the future — hold it.
                                    self.pending_frame = Some(next);
                                    break;
                                },
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    video_disconnected = true;
                                    break;
                                },
                            }
                        }
                    }
                }
            },
        }

        // Upload new frame as texture and record display time.
        if let Some(frame) = latest_frame {
            if let Some(old_tex) = self.current_texture.take() {
                let _ = backend.destroy_texture(old_tex);
            }

            // For software decode, frame may carry its own dimensions.
            #[cfg(feature = "_video")]
            let (fw, fh) = if frame.width > 0 && frame.height > 0 {
                (frame.width, frame.height)
            } else {
                (self.frame_width, self.frame_height)
            };
            #[cfg(not(feature = "_video"))]
            let (fw, fh) = (self.frame_width, self.frame_height);

            // Save timestamp before consuming frame data.
            #[cfg(feature = "_video")]
            let frame_ts = frame.timestamp_secs;

            match backend.load_texture(fw, fh, &frame.data) {
                Ok(tex) => {
                    self.current_texture = Some(tex);
                    self.last_frame_time = Some(Instant::now());
                    self.displayed_frames += 1;
                    if self.state == PlayerState::Starting {
                        self.state = PlayerState::Playing;
                        #[cfg(feature = "_video")]
                        {
                            self.playback_start = Some(Instant::now());
                            self.base_pts = frame_ts;
                        }
                        self.last_display_report = Some(Instant::now());
                        log::info!("VideoPlayer: first frame received, now playing");
                    }
                    if let Some(ref mut t) = self.last_display_report
                        && t.elapsed().as_millis() >= 500
                    {
                        let elapsed = self
                            .playback_start
                            .map(|s| s.elapsed().as_secs_f64())
                            .unwrap_or(0.0);
                        let fps = self.displayed_frames as f64 / elapsed.max(0.001);
                        log::info!(
                            "VideoPlayer: displayed {} frames in {elapsed:.1}s ({fps:.1} display fps)",
                            self.displayed_frames,
                        );
                        *t = Instant::now();
                    }
                },
                Err(e) => {
                    log::error!("VideoPlayer: failed to upload frame texture: {e}");
                },
            }
        }

        // Drain audio channel.
        let mut audio_disconnected = false;
        let audio = match decode {
            #[cfg(not(feature = "_video"))]
            DecodeBackend::Ffmpeg { audio_rx, .. } => {
                let mut chunks = Vec::new();
                loop {
                    match audio_rx.try_recv() {
                        Ok(chunk) => chunks.push(chunk),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            audio_disconnected = true;
                            break;
                        },
                    }
                }
                AudioOutput::Mp3Chunks(chunks)
            },
            #[cfg(feature = "_video")]
            DecodeBackend::Software { audio_rx, .. } => {
                let mut chunks = Vec::new();
                loop {
                    match audio_rx.try_recv() {
                        Ok(chunk) => chunks.push(chunk),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            audio_disconnected = true;
                            break;
                        },
                    }
                }
                AudioOutput::PcmF32(chunks)
            },
        };

        // Detect decode exit (both channels disconnected).
        if video_disconnected && audio_disconnected {
            log::info!("VideoPlayer: decode exited (both channels disconnected)");
            self.decode = None;
            self.finished = true;
            // Keep current_texture visible (last frame stays).
        }

        (self.current_texture, audio)
    }

    /// Stop playback and clean up all resources.
    pub fn stop(&mut self, backend: &mut impl SdiBackend) {
        if let Some(tex) = self.current_texture.take() {
            let _ = backend.destroy_texture(tex);
        }
        self.stop_internal();
    }

    /// Whether the player is actively starting or playing.
    pub fn is_active(&self) -> bool {
        self.state == PlayerState::Starting || self.state == PlayerState::Playing
    }

    /// Whether playback finished (decode thread exited cleanly).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Number of frames displayed so far (for diagnostics).
    pub fn displayed_frames(&self) -> u64 {
        self.displayed_frames
    }

    /// Internal cleanup: kill processes / signal threads, clear channels.
    fn stop_internal(&mut self) {
        if let Some(decode) = self.decode.take() {
            match decode {
                #[cfg(not(feature = "_video"))]
                DecodeBackend::Ffmpeg {
                    mut video_process,
                    mut audio_process,
                    ..
                } => {
                    let _ = video_process.kill();
                    let _ = video_process.wait();
                    let _ = audio_process.kill();
                    let _ = audio_process.wait();
                },
                #[cfg(feature = "_video")]
                DecodeBackend::Software { stop_tx, .. } => {
                    let _ = stop_tx.send(());
                },
            }
        }
        self.current_texture = None;
        self.last_frame_time = None;
        self.displayed_frames = 0;
        self.last_display_report = None;
        self.finished = false;
        #[cfg(feature = "_video")]
        {
            self.playback_start = None;
            self.base_pts = 0.0;
            self.pending_frame = None;
        }
        self.state = PlayerState::Idle;
        self.error_msg = None;
    }
}

/// Nearest-neighbor RGBA scale.
#[cfg(feature = "_video")]
fn simple_scale(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..dst_h {
        let sy = (y * src_h / dst_h).min(src_h - 1);
        for x in 0..dst_w {
            let sx = (x * src_w / dst_w).min(src_w - 1);
            let si = (sy * src_w + sx) as usize * 4;
            let di = (y * dst_w + x) as usize * 4;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::backend::{Color, SdiBackend, SdiCore, TextureId};
    use oasis_core::error::Result;

    /// Minimal mock backend for video_player tests.
    struct MockBackend;

    impl SdiCore for MockBackend {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, _c: Color) -> Result<()> {
            Ok(())
        }
        fn blit(&mut self, _t: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _c: Color) -> Result<()> {
            Ok(())
        }
        fn draw_text(&mut self, _t: &str, _x: i32, _y: i32, _s: u16, _c: Color) -> Result<()> {
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _d: &[u8]) -> Result<TextureId> {
            Ok(TextureId(1))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, _t: &str, _s: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl SdiBackend for MockBackend {}

    #[test]
    fn new_is_idle() {
        let player = VideoPlayer::new();
        assert_eq!(player.state, PlayerState::Idle);
        assert!(!player.is_active());
        assert!(player.current_texture.is_none());
        assert!(player.error_msg.is_none());
    }

    #[test]
    fn stop_when_idle_is_noop() {
        let mut player = VideoPlayer::new();
        let mut backend = MockBackend;
        player.stop(&mut backend);
        assert_eq!(player.state, PlayerState::Idle);
    }

    #[test]
    fn double_stop_is_safe() {
        let mut player = VideoPlayer::new();
        let mut backend = MockBackend;
        player.stop(&mut backend);
        player.stop(&mut backend);
        assert_eq!(player.state, PlayerState::Idle);
    }

    #[test]
    fn tick_when_idle_returns_none() {
        let mut player = VideoPlayer::new();
        let mut backend = MockBackend;
        let (tex, audio) = player.tick(&mut backend);
        assert!(tex.is_none());
        assert!(matches!(audio, AudioOutput::None));
    }

    #[cfg(not(feature = "_video"))]
    #[test]
    fn start_with_bogus_command_sets_error() {
        let mut player = VideoPlayer::new();
        // ffmpeg check will fail if ffmpeg is not installed in CI,
        // which is fine — it exercises the error path.
        let ffmpeg_available = Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !ffmpeg_available {
            // ffmpeg not installed — start() should set Error state.
            player.start("http://example.com/video.mp4", 0, 160, 104);
            assert_eq!(player.state, PlayerState::Error);
            assert!(player.error_msg.is_some());
        }
        // If ffmpeg IS available, we can't easily test error without
        // a network call, so just verify construction is valid.
    }

    #[cfg(feature = "_video")]
    #[test]
    fn start_software_nonexistent_file_sets_error() {
        let mut player = VideoPlayer::new();
        player.start_software("/tmp/nonexistent_video.mp4".into(), 0, 160, 104);
        assert_eq!(player.state, PlayerState::Error);
        assert!(player.error_msg.is_some());
    }

    #[cfg(feature = "_video")]
    #[test]
    fn start_software_corrupt_file_sets_error() {
        use std::io::Write;
        let path = std::env::temp_dir().join("oasis_test_corrupt.mp4");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"this is not an mp4").unwrap();
        }
        let mut player = VideoPlayer::new();
        player.start_software(path.clone(), 0, 160, 104);
        assert_eq!(player.state, PlayerState::Error);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn audio_output_variants() {
        #[cfg(not(feature = "_video"))]
        {
            let mp3 = AudioOutput::Mp3Chunks(vec![vec![1, 2, 3]]);
            assert!(matches!(mp3, AudioOutput::Mp3Chunks(ref v) if v.len() == 1));
        }

        let none = AudioOutput::None;
        assert!(matches!(none, AudioOutput::None));
    }

    #[cfg(feature = "_video")]
    #[test]
    fn simple_scale_identity() {
        let src = vec![255u8; 4 * 4 * 4]; // 4x4 white
        let dst = simple_scale(&src, 4, 4, 4, 4);
        assert_eq!(dst, src);
    }

    #[cfg(feature = "_video")]
    #[test]
    fn simple_scale_downscale() {
        // 4x4 → 2x2
        let mut src = vec![0u8; 4 * 4 * 4];
        // Set top-left pixel to red.
        src[0] = 255;
        src[3] = 255;
        let dst = simple_scale(&src, 4, 4, 2, 2);
        assert_eq!(dst.len(), 2 * 2 * 4);
        // Top-left of downscaled should sample top-left of source.
        assert_eq!(dst[0], 255); // R
    }
}
