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

use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use oasis_core::backend::{SdiBackend, TextureId};

/// Raw RGBA frame from a decode backend.
struct VideoFrame {
    data: Vec<u8>,
    #[cfg(feature = "video-decode")]
    width: u32,
    #[cfg(feature = "video-decode")]
    height: u32,
    /// Presentation timestamp in seconds (software decode only; 0.0 for ffmpeg).
    #[cfg(feature = "video-decode")]
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
const VIDEO_FPS: u32 = 15;

/// Audio output from a single tick.
pub enum AudioOutput {
    /// MP3-encoded chunks (ffmpeg path).
    Mp3Chunks(Vec<Vec<u8>>),
    /// Decoded PCM f32 samples (software decode path).
    #[cfg(feature = "video-decode")]
    PcmF32(Vec<SoftwareAudio>),
    /// No audio this tick.
    None,
}

/// A chunk of decoded PCM f32 audio from the software decoder.
#[cfg(feature = "video-decode")]
pub struct SoftwareAudio {
    pub pcm_f32: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Which decode backend is active.
enum DecodeBackend {
    Ffmpeg {
        video_process: Child,
        audio_process: Child,
        video_rx: Receiver<VideoFrame>,
        audio_rx: Receiver<Vec<u8>>,
    },
    #[cfg(feature = "video-decode")]
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
    /// When the last video frame was displayed (for pacing).
    last_frame_time: Option<Instant>,
    /// Minimum interval between displayed frames (1 / VIDEO_FPS).
    frame_interval: Duration,
    /// Wall-clock time when the first frame was displayed (PTS sync).
    #[cfg(feature = "video-decode")]
    playback_start: Option<Instant>,
    /// PTS of the first frame received (base for wall-clock sync).
    #[cfg(feature = "video-decode")]
    base_pts: f64,
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
            last_frame_time: None,
            frame_interval: Duration::from_nanos(1_000_000_000 / u64::from(VIDEO_FPS)),
            #[cfg(feature = "video-decode")]
            playback_start: None,
            #[cfg(feature = "video-decode")]
            base_pts: 0.0,
        }
    }

    /// Start playing a video from a URL using ffmpeg subprocesses.
    ///
    /// If `seek_secs > 0`, seeks into the stream before decoding.
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
                            #[cfg(feature = "video-decode")]
                            width: 0,
                            #[cfg(feature = "video-decode")]
                            height: 0,
                            #[cfg(feature = "video-decode")]
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
    #[cfg(feature = "video-decode")]
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

        let mut decoder = match oasis_video::SoftwareVideoDecoder::open_stream(Box::new(file)) {
            Ok(d) => d,
            Err(e) => {
                log::error!("VideoPlayer: failed to open decoder: {e}");
                self.state = PlayerState::Error;
                self.error_msg = Some(format!("decoder: {e}"));
                return;
            },
        };

        // Seek if requested.
        if seek_secs > 0
            && let Err(e) = decoder.seek(seek_secs as f64)
        {
            log::warn!("VideoPlayer: seek to {seek_secs}s failed: {e}");
        }

        let (video_tx, video_rx) = mpsc::sync_channel::<VideoFrame>(2);
        let (audio_tx, audio_rx) = mpsc::sync_channel::<SoftwareAudio>(8);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let target_w = width;
        let target_h = height;

        std::thread::spawn(move || {
            log::info!("VideoPlayer: software decode thread started");
            loop {
                // Check stop signal.
                if stop_rx.try_recv().is_ok() {
                    break;
                }

                // Alternate: try video frame first, then audio.
                match decoder.next_video_frame() {
                    Ok(Some(frame)) => {
                        // Scale to target if dimensions differ.
                        let ts = frame.timestamp_secs;
                        let (data, w, h) = if frame.width == target_w && frame.height == target_h {
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
                            break;
                        }
                    },
                    Ok(None) => {
                        log::info!("VideoPlayer: software decode: end of video stream");
                        break;
                    },
                    Err(oasis_video::VideoError::NoTrack(_)) => {
                        // No video track — still try audio.
                    },
                    Err(e) => {
                        log::error!("VideoPlayer: video decode error: {e}");
                        break;
                    },
                }

                // Try audio.
                match decoder.next_audio_samples() {
                    Ok(Some(chunk)) => {
                        let _ = audio_tx.send(SoftwareAudio {
                            pcm_f32: chunk.pcm_f32,
                            channels: chunk.channels,
                            sample_rate: chunk.sample_rate,
                        });
                    },
                    Ok(None) => {},
                    Err(oasis_video::VideoError::NoTrack(_)) => {},
                    Err(e) => {
                        log::warn!("VideoPlayer: audio decode error: {e}");
                    },
                }
            }
            log::info!("VideoPlayer: software decode thread exited");
        });

        self.decode = Some(DecodeBackend::Software {
            video_rx,
            audio_rx,
            stop_tx,
        });
        self.state = PlayerState::Starting;

        log::info!(
            "VideoPlayer: software decode started {}x{} seek={seek_secs}s file={}",
            width,
            height,
            path.display(),
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
            #[cfg(feature = "video-decode")]
            DecodeBackend::Software { video_rx, .. } => {
                // PTS-based pacing: display the latest frame whose PTS <= wall clock.
                let wall = self
                    .playback_start
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
                let target_pts = self.base_pts + wall;

                loop {
                    match video_rx.try_recv() {
                        Ok(frame) => {
                            if frame.timestamp_secs <= target_pts {
                                // This frame is due or late — keep it, try next.
                                latest_frame = Some(frame);
                            } else {
                                // Frame is early — display it anyway (we can't
                                // put it back), but stop draining.
                                latest_frame = Some(frame);
                                break;
                            }
                        },
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            video_disconnected = true;
                            break;
                        },
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
            #[cfg(feature = "video-decode")]
            let (fw, fh) = if frame.width > 0 && frame.height > 0 {
                (frame.width, frame.height)
            } else {
                (self.frame_width, self.frame_height)
            };
            #[cfg(not(feature = "video-decode"))]
            let (fw, fh) = (self.frame_width, self.frame_height);

            // Save timestamp before consuming frame data.
            #[cfg(feature = "video-decode")]
            let frame_ts = frame.timestamp_secs;

            match backend.load_texture(fw, fh, &frame.data) {
                Ok(tex) => {
                    self.current_texture = Some(tex);
                    self.last_frame_time = Some(Instant::now());
                    if self.state == PlayerState::Starting {
                        self.state = PlayerState::Playing;
                        // Record PTS sync reference on first frame.
                        #[cfg(feature = "video-decode")]
                        {
                            self.playback_start = Some(Instant::now());
                            self.base_pts = frame_ts;
                        }
                        log::info!("VideoPlayer: first frame received, now playing");
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
            #[cfg(feature = "video-decode")]
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

    /// Internal cleanup: kill processes / signal threads, clear channels.
    fn stop_internal(&mut self) {
        if let Some(decode) = self.decode.take() {
            match decode {
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
                #[cfg(feature = "video-decode")]
                DecodeBackend::Software { stop_tx, .. } => {
                    let _ = stop_tx.send(());
                },
            }
        }
        self.current_texture = None;
        self.last_frame_time = None;
        #[cfg(feature = "video-decode")]
        {
            self.playback_start = None;
            self.base_pts = 0.0;
        }
        self.state = PlayerState::Idle;
        self.error_msg = None;
    }
}

/// Nearest-neighbor RGBA scale.
#[cfg(feature = "video-decode")]
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

    #[cfg(feature = "video-decode")]
    #[test]
    fn start_software_nonexistent_file_sets_error() {
        let mut player = VideoPlayer::new();
        player.start_software("/tmp/nonexistent_video.mp4".into(), 0, 160, 104);
        assert_eq!(player.state, PlayerState::Error);
        assert!(player.error_msg.is_some());
    }

    #[cfg(feature = "video-decode")]
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
        let mp3 = AudioOutput::Mp3Chunks(vec![vec![1, 2, 3]]);
        assert!(matches!(mp3, AudioOutput::Mp3Chunks(ref v) if v.len() == 1));

        let none = AudioOutput::None;
        assert!(matches!(none, AudioOutput::None));
    }

    #[cfg(feature = "video-decode")]
    #[test]
    fn simple_scale_identity() {
        let src = vec![255u8; 4 * 4 * 4]; // 4x4 white
        let dst = simple_scale(&src, 4, 4, 4, 4);
        assert_eq!(dst, src);
    }

    #[cfg(feature = "video-decode")]
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
