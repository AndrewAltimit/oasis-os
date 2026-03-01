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

use demux::{Mp4Demuxer, TrackKind};

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
}

impl SoftwareVideoDecoder {
    /// Open an MP4 from a byte buffer.
    ///
    /// Parses the container and initializes decoders for any H.264 video and
    /// AAC audio tracks found.
    pub fn open(mp4_data: Vec<u8>) -> Result<Self, VideoError> {
        let demuxer = Mp4Demuxer::open(mp4_data)?;

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
        })
    }

    /// Decode the next video frame.
    ///
    /// Skips audio packets internally. Returns `None` at end-of-stream.
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
            let h264 = match &mut self.h264 {
                Some(d) => d,
                None => return Err(VideoError::NoTrack("no video track".into())),
            };

            loop {
                let packet = match self.demuxer.next_packet()? {
                    Some(p) => p,
                    None => return Ok(None),
                };

                if packet.kind != TrackKind::Video {
                    continue;
                }

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
    /// Skips video packets internally. Returns `None` at end-of-stream.
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        let aac = match &mut self.aac {
            Some(d) => d,
            None => return Err(VideoError::NoTrack("no audio track".into())),
        };

        loop {
            let packet = match self.demuxer.next_packet()? {
                Some(p) => p,
                None => return Ok(None),
            };

            if packet.kind != TrackKind::Audio {
                continue;
            }

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
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
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
