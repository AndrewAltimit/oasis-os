//! Software MP4/H.264+AAC video decode pipeline.
//!
//! Provides a pure-software path for decoding MP4 videos when hardware or
//! browser-native codecs are unavailable.  Two backends:
//!
//! - **symphonia + openh264** (default): Pure Rust MP4 demuxing and AAC-LC
//!   decoding via symphonia, with optional H.264 via openh264 (`h264` feature).
//! - **ffmpeg** (`ffmpeg` feature): Statically-linked ffmpeg for demuxing,
//!   H.264, and AAC. Full codec support, SIMD-optimized, no runtime deps.
//!
//! The `h264` and `ffmpeg` features are mutually exclusive.

#[cfg(all(feature = "h264", feature = "ffmpeg"))]
compile_error!("features `h264` and `ffmpeg` are mutually exclusive — use one or the other");

pub mod aac;
pub mod demux;
pub mod demux_lite;
#[cfg(feature = "ffmpeg")]
#[allow(clippy::unnecessary_cast)]
pub mod ffmpeg_decoder;
#[cfg(feature = "h264")]
pub mod h264;
pub mod yuv;

use std::io::{Cursor, Read, Seek};

#[cfg(not(feature = "ffmpeg"))]
use demux::{DemuxedPacket, Mp4Demuxer, TrackKind};
#[cfg(not(feature = "ffmpeg"))]
use std::collections::VecDeque;

// Re-export avcC helpers so callers can pre-extract from moov data.
#[cfg(not(feature = "ffmpeg"))]
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
#[derive(Debug)]
pub enum VideoError {
    /// MP4 container / demuxing error.
    Demux(String),
    /// Codec decode error (H.264 or AAC).
    Decode(String),
    /// No suitable track found.
    NoTrack(String),
    /// Decoder couldn't produce a frame within the packet skip limit.
    /// Not a fatal error — caller should continue with audio and retry.
    SkipLimit,
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Demux(s) => write!(f, "demux error: {s}"),
            Self::Decode(s) => write!(f, "decode error: {s}"),
            Self::NoTrack(s) => write!(f, "no track: {s}"),
            Self::SkipLimit => write!(f, "decoder skip limit reached"),
        }
    }
}

impl std::error::Error for VideoError {}

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

/// Software MP4 video decoder.
///
/// Opens an MP4 from a byte buffer and provides frame-by-frame video (RGBA)
/// and audio (PCM f32) output.
///
/// When the `ffmpeg` feature is enabled, uses statically-linked ffmpeg for
/// full H.264+AAC decode with SIMD optimization.
///
/// Otherwise, H.264 video decoding requires the `h264` feature.
/// Without it, [`next_video_frame`](Self::next_video_frame) returns
/// [`VideoError::NoTrack`].
pub struct SoftwareVideoDecoder {
    #[cfg(feature = "ffmpeg")]
    inner: ffmpeg_decoder::FfmpegDecoder,

    #[cfg(not(feature = "ffmpeg"))]
    demuxer: Mp4Demuxer,
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    h264: Option<h264::H264Decoder>,
    #[cfg(not(feature = "ffmpeg"))]
    aac: Option<aac::AacDecoder>,
    #[cfg(not(feature = "ffmpeg"))]
    video_width: u32,
    #[cfg(not(feature = "ffmpeg"))]
    video_height: u32,
    #[cfg(not(feature = "ffmpeg"))]
    audio_sample_rate: u32,
    #[cfg(not(feature = "ffmpeg"))]
    audio_channels: u16,
    /// Buffered video packets encountered while reading audio.
    #[cfg(not(feature = "ffmpeg"))]
    video_queue: VecDeque<DemuxedPacket>,
    /// Buffered audio packets encountered while reading video.
    #[cfg(not(feature = "ffmpeg"))]
    audio_queue: VecDeque<DemuxedPacket>,
}

impl SoftwareVideoDecoder {
    /// Open an MP4 from a streaming source.
    ///
    /// Accepts any `Read + Seek + Send` source (e.g. `File`, network stream)
    /// and initializes decoders for any H.264 video and AAC audio tracks.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        #[cfg(feature = "ffmpeg")]
        {
            let inner = ffmpeg_decoder::FfmpegDecoder::open_stream(source)?;
            Ok(Self { inner })
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            let demuxer = Mp4Demuxer::open_stream(source)?;
            Self::from_demuxer(demuxer)
        }
    }

    /// Open from a streaming source with pre-extracted avcC config.
    ///
    /// Skips the full-file `read_to_end()` scan that `open_stream` performs.
    /// Use this when avcC has already been extracted from the moov atom
    /// (e.g. fetched via HTTP Range request).
    #[cfg(not(feature = "ffmpeg"))]
    pub fn open_stream_with_avcc(
        source: Box<dyn VideoSource>,
        avcc: Option<AvccConfig>,
    ) -> Result<Self, VideoError> {
        let demuxer = Mp4Demuxer::open_stream_with_avcc(source, avcc)?;
        Self::from_demuxer(demuxer)
    }

    /// Open an MP4 from a byte buffer.
    ///
    /// Parses the container and initializes decoders for any H.264 video and
    /// AAC audio tracks found.
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError> {
        Self::open_stream(Box::new(Cursor::new(mp4_data)))
    }

    /// Initialize decoders from a probed demuxer (symphonia path only).
    #[cfg(not(feature = "ffmpeg"))]
    fn from_demuxer(demuxer: Mp4Demuxer) -> Result<Self, VideoError> {
        #[cfg(feature = "h264")]
        let h264 = if demuxer.has_video() {
            Some(h264::H264Decoder::new()?)
        } else {
            None
        };

        let aac = if let Some(params) = demuxer.audio_codec_params() {
            Some(aac::AacDecoder::new(params)?)
        } else {
            None
        };

        // Dimensions are discovered from the first decoded H.264 frame.
        let video_width = 0;
        let video_height = 0;

        let audio_sample_rate = aac.as_ref().map(|a| a.sample_rate()).unwrap_or(0);
        let audio_channels = aac.as_ref().map(|a| a.channels()).unwrap_or(0);

        Ok(Self {
            demuxer,
            #[cfg(feature = "h264")]
            h264,
            aac,
            video_width,
            video_height,
            audio_sample_rate,
            audio_channels,
            video_queue: VecDeque::new(),
            audio_queue: VecDeque::new(),
        })
    }

    /// Read the next packet for a given track kind, buffering packets for the
    /// other stream so they aren't lost.
    #[cfg(not(feature = "ffmpeg"))]
    fn next_packet_for(&mut self, kind: TrackKind) -> Result<Option<DemuxedPacket>, VideoError> {
        // Check the dedicated queue first.
        let queue = match kind {
            TrackKind::Video => &mut self.video_queue,
            TrackKind::Audio => &mut self.audio_queue,
        };
        if let Some(pkt) = queue.pop_front() {
            return Ok(Some(pkt));
        }
        // Read from the demuxer, buffering packets for the other stream.
        loop {
            let packet = match self.demuxer.next_packet()? {
                Some(p) => p,
                None => return Ok(None),
            };
            if packet.kind == kind {
                return Ok(Some(packet));
            }
            // Buffer the other stream's packet.
            match packet.kind {
                TrackKind::Video => self.video_queue.push_back(packet),
                TrackKind::Audio => self.audio_queue.push_back(packet),
            }
        }
    }

    /// Decode the next video frame.
    ///
    /// Buffers audio packets internally. Returns `Ok(None)` at end-of-stream,
    /// `Err(VideoError::SkipLimit)` if the decoder couldn't produce a frame
    /// within the packet limit (caller should continue with audio).
    ///
    /// With the `ffmpeg` feature, uses ffmpeg's full H.264 decoder (Main/High
    /// profile support, SIMD-optimized). Otherwise falls back to openh264
    /// (Baseline profile only, with reinit-on-error recovery).
    #[allow(clippy::needless_return)]
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.next_video_frame();
        }

        #[cfg(not(any(feature = "h264", feature = "ffmpeg")))]
        {
            Err(VideoError::NoTrack(
                "H.264 decoding unavailable (oasis-video built without 'h264' feature)".into(),
            ))
        }

        #[cfg(feature = "h264")]
        {
            if self.h264.is_none() {
                return Err(VideoError::NoTrack("no video track".into()));
            }

            // Try up to 3 reinit cycles per call. Each cycle: reinit decoder,
            // skip to next IDR, decode with SPS/PPS. If that doesn't produce
            // a frame, fall through to the normal decode loop which may trigger
            // another reinit cycle.
            for reinit_attempt in 0u32..3 {
                let h264 = self
                    .h264
                    .as_mut()
                    .expect("h264 decoder verified present above");

                if h264.error_streak > 5 {
                    log::info!(
                        "H264: reinit attempt {} (error_streak={})",
                        reinit_attempt + 1,
                        h264.error_streak,
                    );
                    h264.reinit()?;
                    self.demuxer.reset_params();
                    let params = self.demuxer.parameter_sets().map(|p| p.to_vec());
                    let mut skipped_to_idr = 0u32;
                    loop {
                        let packet = match self.next_packet_for(TrackKind::Video)? {
                            Some(p) => p,
                            None => return Ok(None),
                        };
                        skipped_to_idr += 1;
                        if Self::contains_idr(&packet.data) {
                            log::info!("H264: found IDR after skipping {skipped_to_idr} packets");
                            // Prepend SPS/PPS to IDR for decoder reinitialization.
                            let decode_data = if let Some(ref ps) = params {
                                let mut buf = Vec::with_capacity(ps.len() + packet.data.len());
                                buf.extend_from_slice(ps);
                                buf.extend_from_slice(&packet.data);
                                buf
                            } else {
                                packet.data.clone()
                            };
                            let h264 = self
                                .h264
                                .as_mut()
                                .expect("h264 decoder verified present above");
                            if let Some(frame) = h264.decode(&decode_data)? {
                                if frame.width > 0 && frame.height > 0 {
                                    self.video_width = frame.width;
                                    self.video_height = frame.height;
                                }
                                return Ok(Some(VideoFrame {
                                    rgba: frame.rgba,
                                    width: frame.width,
                                    height: frame.height,
                                    timestamp_secs: packet.timestamp_secs,
                                }));
                            }
                            break; // IDR didn't produce frame — fall through
                            // to normal decode loop for subsequent frames.
                        }
                        if skipped_to_idr > 2000 {
                            return Err(VideoError::SkipLimit);
                        }
                    }
                }

                // Normal decode: read packets until we produce a frame.
                let mut skipped = 0u32;
                loop {
                    let packet = match self.next_packet_for(TrackKind::Video)? {
                        Some(p) => p,
                        None => return Ok(None),
                    };

                    let h264 = self
                        .h264
                        .as_mut()
                        .expect("h264 decoder verified present above");

                    if let Some(frame) = h264.decode(&packet.data)? {
                        if frame.width > 0 && frame.height > 0 {
                            self.video_width = frame.width;
                            self.video_height = frame.height;
                        }
                        return Ok(Some(VideoFrame {
                            rgba: frame.rgba,
                            width: frame.width,
                            height: frame.height,
                            timestamp_secs: packet.timestamp_secs,
                        }));
                    }

                    // When error_streak crosses threshold, break to outer
                    // loop for reinit instead of returning SkipLimit.
                    if h264.error_streak > 5 {
                        break;
                    }

                    skipped += 1;
                    if skipped > 500 {
                        return Err(VideoError::SkipLimit);
                    }
                }
            }

            // All reinit attempts exhausted.
            Err(VideoError::SkipLimit)
        }
    }

    /// Check if an Annex-B bitstream contains an IDR NAL unit (type 5).
    #[cfg(feature = "h264")]
    fn contains_idr(data: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 <= data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
                if i + 4 < data.len() && (data[i + 4] & 0x1F) == 5 {
                    return true;
                }
                i += 4;
            } else if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                if i + 3 < data.len() && (data[i + 3] & 0x1F) == 5 {
                    return true;
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        false
    }

    /// Decode the next chunk of audio.
    ///
    /// Buffers video packets internally. Returns `None` at end-of-stream.
    #[allow(clippy::needless_return)]
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.next_audio_samples();
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            if self.aac.is_none() {
                return Err(VideoError::NoTrack("no audio track".into()));
            }

            loop {
                let packet = match self.next_packet_for(TrackKind::Audio)? {
                    Some(p) => p,
                    None => return Ok(None),
                };

                let aac = self
                    .aac
                    .as_mut()
                    .expect("aac decoder verified present above");
                if let Some(audio) = aac.decode(&packet.data, 0)? {
                    return Ok(Some(AudioChunk {
                        pcm_f32: audio.pcm_f32,
                        channels: audio.channels,
                        sample_rate: audio.sample_rate,
                        timestamp_secs: packet.timestamp_secs,
                    }));
                }
            }
        }
    }

    /// Return a buffered audio chunk without reading new packets from the stream.
    ///
    /// With the `ffmpeg` backend, audio packets are automatically buffered during
    /// `next_video_frame()`. This method drains those buffers without advancing
    /// the stream, preventing EOF from being triggered prematurely.
    ///
    /// With the symphonia backend, this drains from the internal audio queue
    /// (packets already read from the demuxer during video decoding).
    #[allow(clippy::needless_return)]
    pub fn next_buffered_audio(&mut self) -> Option<AudioChunk> {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.next_buffered_audio();
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            // Drain from the audio queue (packets buffered during video reads).
            while let Some(pkt) = self.audio_queue.pop_front() {
                let Some(aac) = self.aac.as_mut() else {
                    continue;
                };
                match aac.decode(&pkt.data, 0) {
                    Ok(Some(audio)) => {
                        return Some(AudioChunk {
                            pcm_f32: audio.pcm_f32,
                            channels: audio.channels,
                            sample_rate: audio.sample_rate,
                            timestamp_secs: pkt.timestamp_secs,
                        });
                    },
                    Ok(None) => {
                        // Decoder consumed packet but produced no output
                        // (e.g. priming frame). Continue to next packet.
                    },
                    Err(e) => {
                        // Log once per burst to avoid spam (AAC errors are
                        // common after seeking into mid-stream).
                        log::debug!("AAC decode error (skipping frame): {e}");
                    },
                }
            }
            None
        }
    }

    /// Seek to a position in seconds.
    ///
    /// Clears any buffered packets so post-seek reads don't return stale data.
    #[allow(clippy::needless_return)]
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.seek(secs);
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            self.video_queue.clear();
            self.audio_queue.clear();
            self.demuxer.seek(secs)
        }
    }

    /// Video dimensions (may be 0x0 if no video track or not yet decoded).
    #[allow(clippy::needless_return)]
    pub fn video_size(&self) -> (u32, u32) {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.video_size();
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            (self.video_width, self.video_height)
        }
    }

    /// Audio sample rate and channel count.
    #[allow(clippy::needless_return)]
    pub fn audio_format(&self) -> (u32, u16) {
        #[cfg(feature = "ffmpeg")]
        {
            return self.inner.audio_format();
        }

        #[cfg(not(feature = "ffmpeg"))]
        {
            (self.audio_sample_rate, self.audio_channels)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to the shared test fixture (320x240, ~2s, H.264+AAC).
    fn fixture_path() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.join("../../tests/fixtures/test_320x240_2s.mp4")
    }

    /// Read the fixture into a `Vec<u8>`.
    fn fixture_bytes() -> Vec<u8> {
        std::fs::read(fixture_path()).expect("fixture file missing")
    }

    /// Open the fixture as a `File`.
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

    // ------------------------------------------------------------------
    // Existing unit tests
    // ------------------------------------------------------------------

    #[test]
    fn open_empty_data_fails() {
        let result = SoftwareVideoDecoder::open(Vec::new());
        assert!(result.is_err());
    }

    #[test]
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
    fn open_stream_with_cursor_empty_fails() {
        let cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = SoftwareVideoDecoder::open_stream(Box::new(cursor));
        assert!(result.is_err());
    }

    #[test]
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
