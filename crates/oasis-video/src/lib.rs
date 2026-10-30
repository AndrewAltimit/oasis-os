//! Software MP4/H.264+AAC video decode pipeline.
//!
//! Provides a pure-software path for decoding MP4 videos when hardware or
//! browser-native codecs are unavailable.
//!
//! # Features
//!
//! All features are additive; none is on by default.
//!
//! - `demux-lite`: `demux_lite`, a dependency-free MP4 parser (PSP,
//!   seek-point lookup, fuzzing).
//! - `symphonia`: `demux` + `aac` (symphonia MP4 demux and AAC-LC) and an
//!   audio-only `SoftwareVideoDecoder`. Implies `demux-lite`.
//! - `h264`: `h264` (openh264, compiled from C source), so the symphonia
//!   backend decodes video too. Implies `symphonia`.
//! - `ffmpeg`: statically-linked ffmpeg for demux, H.264 and AAC
//!   (Main/High profile, SIMD-optimized).
//!
//! [`SoftwareVideoDecoder`] exists when `symphonia` or `ffmpeg` is on. When
//! both backends are compiled in (e.g. `--all-features`, or one binary
//! enabling `h264` through one crate and `ffmpeg` through another), the
//! ffmpeg backend wins; the symphonia modules stay available as libraries.
//! The [`yuv`] converters and the error/frame types are always available.

#[cfg(feature = "symphonia")]
pub mod aac;
#[cfg(feature = "symphonia")]
pub mod demux;
#[cfg(feature = "demux-lite")]
pub mod demux_lite;
#[cfg(feature = "ffmpeg")]
#[allow(clippy::unnecessary_cast)]
pub mod ffmpeg_decoder;
#[cfg(feature = "h264")]
pub mod h264;
#[cfg(feature = "symphonia")]
// Compiled (and linted) whenever its dependencies are, even when ffmpeg is
// the selected backend.
#[cfg_attr(feature = "ffmpeg", allow(dead_code))]
mod symphonia_decoder;
pub mod yuv;

use std::io::{Cursor, Read, Seek};

// Re-export avcC helpers so callers can pre-extract from moov data.
#[cfg(feature = "symphonia")]
pub use demux::{AvccConfig, find_avcc_in_mp4};

/// A streaming video source: anything that is `Read + Seek + Send + Sync`.
///
/// Provides optional length and seekability hints for the demuxer.
/// This is a blanket trait — `File`, `Cursor<Vec<u8>>`, and any other
/// `Read + Seek + Send + Sync + 'static` type implements it automatically.
pub trait VideoSource: Read + Seek + Send + Sync {
    /// Whether the source supports seeking. Defaults to `true`.
    fn is_seekable(&self) -> bool {
        true
    }

    /// Total length in bytes, if known.
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl VideoSource for std::fs::File {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.metadata().ok().map(|m| m.len())
    }
}

impl<T: AsRef<[u8]> + Send + Sync> VideoSource for Cursor<T> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.get_ref().as_ref().len() as u64)
    }
}

/// Errors from the video pipeline.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// MP4 container / demuxing error.
    #[error("demux error: {0}")]
    Demux(String),
    /// Codec decode error (H.264 or AAC).
    #[error("decode error: {0}")]
    Decode(String),
    /// No suitable track found.
    #[error("no track: {0}")]
    NoTrack(String),
    /// Decoder couldn't produce a frame within the packet skip limit.
    /// Not a fatal error — caller should continue with audio and retry.
    #[error("decoder skip limit reached")]
    SkipLimit,
}

/// A decoded video frame in RGBA format.
pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp_secs: f64,
}

/// A chunk of decoded audio as interleaved f32 PCM.
pub struct AudioChunk {
    pub pcm_f32: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub timestamp_secs: f64,
}

/// The decoder backend [`SoftwareVideoDecoder`] wraps: ffmpeg when the
/// `ffmpeg` feature is on (it wins over `symphonia`/`h264`), otherwise
/// symphonia + openh264.
#[cfg(feature = "ffmpeg")]
type Backend = ffmpeg_decoder::FfmpegDecoder;
#[cfg(all(feature = "symphonia", not(feature = "ffmpeg")))]
type Backend = symphonia_decoder::SymphoniaDecoder;

/// Software MP4 video decoder.
///
/// Opens an MP4 from a byte buffer or a stream and provides frame-by-frame
/// video (RGBA) and audio (PCM f32) output. The decode backend is picked
/// at compile time (see the crate docs): ffmpeg (Main/High profile,
/// SIMD-optimized) with the `ffmpeg` feature, otherwise symphonia demux +
/// AAC with openh264 H.264 (Baseline profile, reinit-on-error recovery).
///
/// With only the `symphonia` feature (no `h264`, no `ffmpeg`) the decoder
/// is audio-only and [`next_video_frame`](Self::next_video_frame) returns
/// [`VideoError::NoTrack`].
#[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
pub struct SoftwareVideoDecoder {
    inner: Backend,
}

#[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
impl SoftwareVideoDecoder {
    /// Open an MP4 from a streaming source.
    ///
    /// Accepts any `Read + Seek + Send` source (e.g. `File`, network stream)
    /// and initializes decoders for any H.264 video and AAC audio tracks.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        Ok(Self {
            inner: Backend::open_stream(source)?,
        })
    }

    /// Open from a streaming source using a pre-fetched `moov` atom (the
    /// complete atom, header included).
    ///
    /// The symphonia backend skips the full-file scan that
    /// [`open_stream`](Self::open_stream) performs and indexes the video
    /// keyframes from `moov` so [`seek`](Self::seek) lands on a keyframe.
    /// ffmpeg reads the container itself, so there this is
    /// [`open_stream`](Self::open_stream).
    pub fn open_stream_with_moov(
        source: Box<dyn VideoSource>,
        moov: &[u8],
    ) -> Result<Self, VideoError> {
        #[cfg(feature = "ffmpeg")]
        let inner = {
            let _ = moov;
            Backend::open_stream(source)?
        };
        #[cfg(not(feature = "ffmpeg"))]
        let inner = Backend::open_stream_with_moov(source, moov)?;
        Ok(Self { inner })
    }

    /// Open from a streaming source with a pre-extracted avcC config
    /// (symphonia backend only: skips the full-file scan).
    #[cfg(all(feature = "symphonia", not(feature = "ffmpeg")))]
    pub fn open_stream_with_avcc(
        source: Box<dyn VideoSource>,
        avcc: Option<AvccConfig>,
    ) -> Result<Self, VideoError> {
        Ok(Self {
            inner: Backend::open_stream_with_avcc(source, avcc)?,
        })
    }

    /// Open an MP4 from a byte buffer.
    ///
    /// Parses the container and initializes decoders for any H.264 video and
    /// AAC audio tracks found.
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError> {
        Self::open_stream(Box::new(Cursor::new(mp4_data)))
    }

    /// Decode the next video frame.
    ///
    /// Buffers audio packets internally. Returns `Ok(None)` at end-of-stream,
    /// `Err(VideoError::SkipLimit)` if the decoder couldn't produce a frame
    /// within the packet limit (caller should continue with audio).
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        self.inner.next_video_frame()
    }

    /// Decode the next chunk of audio.
    ///
    /// Buffers video packets internally. Returns `None` at end-of-stream.
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        self.inner.next_audio_samples()
    }

    /// Return a buffered audio chunk without reading new packets from the
    /// stream.
    ///
    /// Audio packets read while decoding video are buffered; this drains
    /// them without advancing the stream, so EOF is not triggered
    /// prematurely.
    pub fn next_buffered_audio(&mut self) -> Option<AudioChunk> {
        self.inner.next_buffered_audio()
    }

    /// Seek to a position in seconds.
    ///
    /// Clears any buffered packets so post-seek reads don't return stale data.
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        self.inner.seek(secs)
    }

    /// The time the decoder seeks to for a request of `secs`.
    ///
    /// The symphonia backend snaps to the last keyframe at or before `secs`
    /// (symphonia ignores `stss`); ffmpeg seeks to keyframes itself, so
    /// there this returns `secs`.
    pub fn keyframe_seek_target(&self, secs: f64) -> f64 {
        #[cfg(feature = "ffmpeg")]
        {
            secs
        }
        #[cfg(not(feature = "ffmpeg"))]
        {
            self.inner.keyframe_seek_target(secs)
        }
    }

    /// Video dimensions (may be 0x0 if no video track or not yet decoded).
    pub fn video_size(&self) -> (u32, u32) {
        self.inner.video_size()
    }

    /// Audio sample rate and channel count.
    pub fn audio_format(&self) -> (u32, u16) {
        self.inner.audio_format()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to the shared test fixture (320x240, ~2s, H.264+AAC).
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn fixture_path() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.join("../../tests/fixtures/test_320x240_2s.mp4")
    }

    /// Read the fixture into a `Vec<u8>`.
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn fixture_bytes() -> Vec<u8> {
        std::fs::read(fixture_path()).expect("fixture file missing")
    }

    /// Open the fixture as a `File`.
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn fixture_file() -> std::fs::File {
        std::fs::File::open(fixture_path()).expect("fixture file missing")
    }

    // ------------------------------------------------------------------
    // Integration tests (require the test fixture)
    // ------------------------------------------------------------------

    #[test]
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn open_stream_with_file_decodes() {
        let file = fixture_file();
        let mut dec =
            SoftwareVideoDecoder::open_stream(Box::new(file)).expect("open_stream failed");
        let frame = dec
            .next_video_frame()
            .expect("decode error")
            .expect("no frame");
        assert_eq!(frame.width, 320);
        assert_eq!(frame.height, 240);
        assert_eq!(frame.rgba.len(), (320 * 240 * 4) as usize);
    }

    #[test]
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn open_stream_with_cursor_decodes() {
        let data = fixture_bytes();
        let cursor = Cursor::new(data);
        let mut dec =
            SoftwareVideoDecoder::open_stream(Box::new(cursor)).expect("open_stream failed");
        let frame = dec
            .next_video_frame()
            .expect("decode error")
            .expect("no frame");
        assert!(frame.width > 0);
        assert!(frame.height > 0);
    }

    #[test]
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn full_decode_pipeline() {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        let mut count = 0u32;
        let mut last_ts = -1.0f64;
        let mut dims = (0u32, 0u32);
        while let Some(frame) = dec.next_video_frame().expect("decode error") {
            count += 1;
            assert!(
                frame.timestamp_secs >= last_ts,
                "timestamp went backwards: {} < {}",
                frame.timestamp_secs,
                last_ts
            );
            last_ts = frame.timestamp_secs;
            dims = (frame.width, frame.height);
        }
        assert!(count >= 10, "expected >=10 frames, got {count}");
        assert_eq!(dims, (320, 240));
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn audio_decode_format() {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        let chunk = dec
            .next_audio_samples()
            .expect("audio decode error")
            .expect("no audio");
        assert!(chunk.sample_rate > 0, "sample_rate should be > 0");
        assert!(chunk.channels > 0, "channels should be > 0");
        assert!(!chunk.pcm_f32.is_empty(), "PCM buffer should not be empty");
    }

    #[test]
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn seek_to_midstream() {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        // Decode one frame to prime the decoder.
        let _ = dec.next_video_frame().expect("decode error");
        dec.seek(1.0).expect("seek failed");
        let frame = dec
            .next_video_frame()
            .expect("decode error")
            .expect("no frame after seek");
        // Timestamp should be within ±0.5s of the seek target.
        assert!(
            (frame.timestamp_secs - 1.0).abs() < 0.5,
            "post-seek timestamp {} not near 1.0s",
            frame.timestamp_secs
        );
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn truncated_file_no_panic() {
        let full = fixture_bytes();
        let half = &full[..full.len() / 2];
        // Opening or decoding a truncated file should not panic.
        match SoftwareVideoDecoder::open(half.to_vec()) {
            Ok(mut dec) => {
                // If it opens, try decoding — it may error, but must not panic.
                let _ = dec.next_video_frame();
                let _ = dec.next_audio_samples();
            },
            Err(_) => {
                // Error on open is acceptable.
            },
        }
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn video_size_before_decode() {
        let dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        // Without ffmpeg, dimensions come from first decoded frame (0,0 initially).
        // With ffmpeg, dimensions are known from stream headers at open time.
        #[cfg(not(feature = "ffmpeg"))]
        assert_eq!(dec.video_size(), (0, 0));
        #[cfg(feature = "ffmpeg")]
        {
            let (w, h) = dec.video_size();
            assert!(w > 0 && h > 0, "ffmpeg should know dimensions at open");
        }
        // Audio format should already be known from track headers.
        let (sr, ch) = dec.audio_format();
        assert!(sr > 0, "sample_rate should be discoverable before decode");
        assert!(ch > 0, "channels should be discoverable before decode");
    }

    #[test]
    #[cfg(any(feature = "h264", feature = "ffmpeg"))]
    fn timestamp_monotonicity() {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        let mut last_video_ts = -1.0f64;
        let mut last_audio_ts = -1.0f64;
        for _ in 0..60 {
            // Alternate video and audio to exercise interleaved buffering.
            if let Ok(Some(frame)) = dec.next_video_frame() {
                assert!(
                    frame.timestamp_secs >= last_video_ts,
                    "video ts went backwards: {} < {}",
                    frame.timestamp_secs,
                    last_video_ts
                );
                last_video_ts = frame.timestamp_secs;
            }
            if let Ok(Some(chunk)) = dec.next_audio_samples() {
                assert!(
                    chunk.timestamp_secs >= last_audio_ts,
                    "audio ts went backwards: {} < {}",
                    chunk.timestamp_secs,
                    last_audio_ts
                );
                last_audio_ts = chunk.timestamp_secs;
            }
        }
    }

    #[test]
    #[cfg(all(feature = "h264", not(feature = "ffmpeg")))]
    fn decodes_every_frame_including_reorder_tail() {
        // The decoder runs without per-packet flushing (required for
        // B-frame streams); the pictures still buffered at end of stream
        // must be drained so no frame is lost.
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        let mut count = 0u32;
        while let Some(_frame) = dec.next_video_frame().expect("decode error") {
            count += 1;
        }
        assert_eq!(count, 30, "fixture has 30 video samples");
    }

    #[test]
    #[cfg(all(feature = "symphonia", not(feature = "ffmpeg")))]
    fn open_builds_keyframe_index() {
        let dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        // Keyframes at 0.0s and 1.0s: requests snap back to the keyframe
        // at or before them (mid-sample seek times).
        let first = 512.0 / 15360.0;
        let second = (15.0 * 1024.0 + 512.0) / 15360.0;
        assert!((dec.keyframe_seek_target(0.5) - first).abs() < 1e-9);
        assert!((dec.keyframe_seek_target(1.0) - second).abs() < 1e-9);
        assert!((dec.keyframe_seek_target(1.7) - second).abs() < 1e-9);
    }

    #[test]
    #[cfg(all(feature = "symphonia", not(feature = "ffmpeg")))]
    fn open_stream_with_moov_builds_keyframe_index() {
        let data = fixture_bytes();
        let moov = symphonia_decoder::find_top_level_atom(&data, b"moov")
            .expect("moov")
            .to_vec();
        let dec = SoftwareVideoDecoder::open_stream_with_moov(Box::new(Cursor::new(data)), &moov)
            .expect("open failed");
        let second = (15.0 * 1024.0 + 512.0) / 15360.0;
        assert!((dec.keyframe_seek_target(1.5) - second).abs() < 1e-9);
    }

    #[test]
    #[cfg(all(feature = "h264", not(feature = "ffmpeg")))]
    fn seek_mid_gop_starts_at_keyframe_with_aligned_audio() {
        // Symphonia ignores stss; without snapping, a seek to 1.5s lands on
        // a P-frame and video only resumes at the next keyframe while
        // audio runs from 1.5s. With snapping both start at the 1.0s IDR.
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes()).expect("open failed");
        dec.seek(1.5).expect("seek failed");
        let frame = dec
            .next_video_frame()
            .expect("decode error")
            .expect("no frame after seek");
        assert!(
            (0.95..1.2).contains(&frame.timestamp_secs),
            "first frame after seek at {}",
            frame.timestamp_secs
        );
        let audio = dec
            .next_buffered_audio()
            .or_else(|| dec.next_audio_samples().expect("audio decode error"));
        let audio = audio.expect("no audio after seek");
        assert!(
            (0.9..1.1).contains(&audio.timestamp_secs),
            "first audio after seek at {}",
            audio.timestamp_secs
        );
    }

    // ------------------------------------------------------------------
    // Existing unit tests
    // ------------------------------------------------------------------

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn open_empty_data_fails() {
        let result = SoftwareVideoDecoder::open(Vec::new());
        assert!(result.is_err());
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn open_garbage_data_fails() {
        let result = SoftwareVideoDecoder::open(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11]);
        assert!(result.is_err());
    }

    #[test]
    fn video_error_display() {
        let e = VideoError::Demux("bad container".into());
        assert_eq!(format!("{e}"), "demux error: bad container");

        let e = VideoError::Decode("codec failure".into());
        assert_eq!(format!("{e}"), "decode error: codec failure");

        let e = VideoError::NoTrack("no video".into());
        assert_eq!(format!("{e}"), "no track: no video");
    }

    #[test]
    fn video_error_is_error_trait() {
        let e: Box<dyn std::error::Error> = Box::new(VideoError::Demux("test".into()));
        assert!(!e.to_string().is_empty());
    }

    #[test]
    fn video_frame_fields() {
        let frame = VideoFrame {
            rgba: vec![255; 16],
            width: 2,
            height: 2,
            timestamp_secs: 1.5,
        };
        assert_eq!(frame.rgba.len(), 16);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert!((frame.timestamp_secs - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn audio_chunk_fields() {
        let chunk = AudioChunk {
            pcm_f32: vec![0.0; 1024],
            channels: 2,
            sample_rate: 44100,
            timestamp_secs: 0.0,
        };
        assert_eq!(chunk.pcm_f32.len(), 1024);
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 44100);
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn open_stream_with_cursor_empty_fails() {
        let cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = SoftwareVideoDecoder::open_stream(Box::new(cursor));
        assert!(result.is_err());
    }

    #[test]
    #[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
    fn open_stream_equivalent_to_open_on_garbage() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11];
        let r1 = SoftwareVideoDecoder::open(data.clone());
        let r2 = SoftwareVideoDecoder::open_stream(Box::new(std::io::Cursor::new(data)));
        // Both should fail with the same kind of error.
        assert!(r1.is_err());
        assert!(r2.is_err());
    }

    #[test]
    fn video_source_impls_compile() {
        // Compile-time check: File and Cursor implement VideoSource.
        fn _assert_video_source<T: VideoSource>() {}
        _assert_video_source::<std::fs::File>();
        _assert_video_source::<std::io::Cursor<Vec<u8>>>();
    }
}
