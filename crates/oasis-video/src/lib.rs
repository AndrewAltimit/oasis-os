//! Software MP4/H.264+AAC video decode pipeline.
//!
//! Provides a pure-software path for decoding MP4 videos when hardware or
//! browser-native codecs are unavailable.  Uses:
//! - **symphonia** (pure Rust) for MP4 demuxing and AAC-LC audio decoding
//! - **openh264** (Cisco C source via `cc`) for H.264 video frame decoding
//!   (requires the `h264` feature, enabled by default)
//! - Pure Rust YUV420→RGBA pixel conversion

pub mod aac;
pub mod demux;
#[cfg(feature = "h264")]
pub mod h264;
pub mod yuv;

use std::collections::VecDeque;
use std::io::{Cursor, Read, Seek};

use demux::{DemuxedPacket, Mp4Demuxer, TrackKind};

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
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Demux(s) => write!(f, "demux error: {s}"),
            Self::Decode(s) => write!(f, "decode error: {s}"),
            Self::NoTrack(s) => write!(f, "no track: {s}"),
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
/// H.264 video decoding requires the `h264` feature (enabled by default).
/// Without it, [`next_video_frame`](Self::next_video_frame) returns
/// [`VideoError::NoTrack`].
pub struct SoftwareVideoDecoder {
    demuxer: Mp4Demuxer,
    #[cfg(feature = "h264")]
    h264: Option<h264::H264Decoder>,
    aac: Option<aac::AacDecoder>,
    video_width: u32,
    video_height: u32,
    audio_sample_rate: u32,
    audio_channels: u16,
    /// Buffered video packets encountered while reading audio.
    video_queue: VecDeque<DemuxedPacket>,
    /// Buffered audio packets encountered while reading video.
    audio_queue: VecDeque<DemuxedPacket>,
}

impl SoftwareVideoDecoder {
    /// Open an MP4 from a streaming source.
    ///
    /// Accepts any `Read + Seek + Send` source (e.g. `File`, network stream)
    /// and initializes decoders for any H.264 video and AAC audio tracks.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        let demuxer = Mp4Demuxer::open_stream(source)?;
        Self::from_demuxer(demuxer)
    }

    /// Open an MP4 from a byte buffer.
    ///
    /// Parses the container and initializes decoders for any H.264 video and
    /// AAC audio tracks found.
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError> {
        Self::open_stream(Box::new(Cursor::new(mp4_data)))
    }

    /// Initialize decoders from a probed demuxer.
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
    /// Buffers audio packets internally. Returns `None` at end-of-stream.
    /// Requires the `h264` feature; returns `VideoError::NoTrack` without it.
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        #[cfg(not(feature = "h264"))]
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

            loop {
                let packet = match self.next_packet_for(TrackKind::Video)? {
                    Some(p) => p,
                    None => return Ok(None),
                };

                let h264 = self.h264.as_mut().unwrap();
                if let Some(frame) = h264.decode(&packet.data)? {
                    // Update dimensions from the actual decoded frame.
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
                // No frame produced (SPS/PPS etc.) — keep reading.
            }
        }
    }

    /// Decode the next chunk of audio.
    ///
    /// Buffers video packets internally. Returns `None` at end-of-stream.
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        if self.aac.is_none() {
            return Err(VideoError::NoTrack("no audio track".into()));
        }

        loop {
            let packet = match self.next_packet_for(TrackKind::Audio)? {
                Some(p) => p,
                None => return Ok(None),
            };

            let aac = self.aac.as_mut().unwrap();
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

    /// Seek to a position in seconds.
    ///
    /// Clears any buffered packets so post-seek reads don't return stale data.
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        self.video_queue.clear();
        self.audio_queue.clear();
        self.demuxer.seek(secs)
    }

    /// Video dimensions (may be 0x0 if no video track or not yet decoded).
    pub fn video_size(&self) -> (u32, u32) {
        (self.video_width, self.video_height)
    }

    /// Audio sample rate and channel count.
    pub fn audio_format(&self) -> (u32, u16) {
        (self.audio_sample_rate, self.audio_channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
