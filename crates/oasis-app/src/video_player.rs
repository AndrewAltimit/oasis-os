//! In-app video player using ffmpeg as a subprocess.
//!
//! Spawns two ffmpeg processes (video + audio) to decode Internet Archive
//! URLs. Background reader threads pipe decoded RGBA frames and MP3 audio
//! chunks to the main thread via `mpsc` channels. The main loop uploads
//! video frames as SDL textures and feeds audio to `SdlAudioBackend`.

use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use oasis_core::backend::{SdiBackend, TextureId};

/// Raw RGBA frame from the ffmpeg video decoder.
struct VideoFrame {
    data: Vec<u8>,
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

/// Manages ffmpeg subprocesses for in-app video+audio playback.
pub struct VideoPlayer {
    state: PlayerState,
    video_process: Option<Child>,
    audio_process: Option<Child>,
    video_rx: Option<Receiver<VideoFrame>>,
    audio_rx: Option<Receiver<Vec<u8>>>,
    current_texture: Option<TextureId>,
    frame_width: u32,
    frame_height: u32,
    error_msg: Option<String>,
    /// When the last video frame was displayed (for pacing).
    last_frame_time: Option<Instant>,
    /// Minimum interval between displayed frames (1 / VIDEO_FPS).
    frame_interval: Duration,
}

impl VideoPlayer {
    /// Create a new idle video player.
    pub fn new() -> Self {
        Self {
            state: PlayerState::Idle,
            video_process: None,
            audio_process: None,
            video_rx: None,
            audio_rx: None,
            current_texture: None,
            frame_width: 0,
            frame_height: 0,
            error_msg: None,
            last_frame_time: None,
            frame_interval: Duration::from_nanos(1_000_000_000 / u64::from(VIDEO_FPS)),
        }
    }

    /// Start playing a video from the given URL.
    ///
    /// Spawns ffmpeg video and audio decoder subprocesses with reader threads.
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
        let video_stdout = video_child.stdout.take().unwrap();
        let audio_stdout = audio_child.stdout.take().unwrap();

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
                        if video_tx.send(VideoFrame { data: buf }).is_err() {
                            break; // Receiver dropped.
                        }
                    },
                    Err(_) => break, // EOF or error.
                }
            }
            log::debug!("VideoPlayer: video reader thread exited");
        });

        // Audio reader thread: reads variable-size chunks.
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>();
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

        self.video_process = Some(video_child);
        self.audio_process = Some(audio_child);
        self.video_rx = Some(video_rx);
        self.audio_rx = Some(audio_rx);
        self.state = PlayerState::Starting;

        log::info!("VideoPlayer: started {width}x{height} seek={seek_secs}s url={url}");
    }

    /// Tick the video player: drain channels, upload latest frame, collect audio.
    ///
    /// Returns `(current_texture, audio_chunks)`. The caller should feed
    /// audio chunks to the audio backend and assign the texture to the guide.
    pub fn tick(&mut self, backend: &mut impl SdiBackend) -> (Option<TextureId>, Vec<Vec<u8>>) {
        if self.state != PlayerState::Starting && self.state != PlayerState::Playing {
            return (self.current_texture, Vec::new());
        }

        // Take at most one video frame, paced to VIDEO_FPS.
        // The sync_channel(2) backpressure keeps ffmpeg from racing too far
        // ahead once we stop consuming frames.
        let mut latest_frame: Option<VideoFrame> = None;
        let mut video_disconnected = false;
        let should_take_frame = self
            .last_frame_time
            .is_none_or(|t| t.elapsed() >= self.frame_interval);
        if should_take_frame && let Some(ref rx) = self.video_rx {
            match rx.try_recv() {
                Ok(frame) => latest_frame = Some(frame),
                Err(TryRecvError::Empty) => {},
                Err(TryRecvError::Disconnected) => video_disconnected = true,
            }
        }

        // Upload new frame as texture and record display time.
        if let Some(frame) = latest_frame {
            if let Some(old_tex) = self.current_texture.take() {
                let _ = backend.destroy_texture(old_tex);
            }
            match backend.load_texture(self.frame_width, self.frame_height, &frame.data) {
                Ok(tex) => {
                    self.current_texture = Some(tex);
                    self.last_frame_time = Some(Instant::now());
                    if self.state == PlayerState::Starting {
                        self.state = PlayerState::Playing;
                        log::info!("VideoPlayer: first frame received, now playing");
                    }
                },
                Err(e) => {
                    log::error!("VideoPlayer: failed to upload frame texture: {e}");
                },
            }
        }

        // Drain audio channel.
        let mut audio_chunks = Vec::new();
        let mut audio_disconnected = false;
        if let Some(ref rx) = self.audio_rx {
            loop {
                match rx.try_recv() {
                    Ok(chunk) => audio_chunks.push(chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        audio_disconnected = true;
                        break;
                    },
                }
            }
        }

        // Detect ffmpeg exit (both channels disconnected).
        if video_disconnected && audio_disconnected {
            log::info!("VideoPlayer: ffmpeg exited (both channels disconnected)");
            self.video_rx = None;
            self.audio_rx = None;
            // Keep current_texture visible (last frame stays).
        }

        (self.current_texture, audio_chunks)
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

    /// Internal cleanup: kill processes, clear channels.
    fn stop_internal(&mut self) {
        if let Some(ref mut child) = self.video_process {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(ref mut child) = self.audio_process {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.video_process = None;
        self.audio_process = None;
        self.video_rx = None;
        self.audio_rx = None;
        self.current_texture = None;
        self.last_frame_time = None;
        self.state = PlayerState::Idle;
        self.error_msg = None;
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::backend::{Color, SdiBackend, TextureId};
    use oasis_core::error::Result;

    /// Minimal mock backend for video_player tests.
    struct MockBackend;

    impl SdiBackend for MockBackend {
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
        assert!(audio.is_empty());
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
}
