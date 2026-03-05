//! MP4 demuxer wrapping symphonia's `FormatReader`.
//!
//! Provides track discovery and per-packet routing so the caller can dispatch
//! H.264 video packets and AAC audio packets to their respective decoders.

use std::io::Cursor;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use symphonia::core::io::MediaSource;

use crate::{VideoError, VideoSource};

/// Wraps a `Box<dyn VideoSource>` to implement symphonia's `MediaSource`.
struct VideoSourceAdapter(Box<dyn VideoSource>);

impl std::io::Read for VideoSourceAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for VideoSourceAdapter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl MediaSource for VideoSourceAdapter {
    fn is_seekable(&self) -> bool {
        self.0.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.0.byte_len()
    }
}

/// Identifies which track a packet belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// A demuxed packet ready for decoding.
pub struct DemuxedPacket {
    pub kind: TrackKind,
    pub data: Vec<u8>,
    pub timestamp_secs: f64,
}

/// MP4 demuxer backed by symphonia.
pub struct Mp4Demuxer {
    reader: Box<dyn FormatReader>,
    video_track_id: Option<u32>,
    audio_track_id: Option<u32>,
    video_timebase: (u64, u64), // (numer, denom)
    audio_timebase: (u64, u64),
    video_codec_params: Option<CodecParameters>,
    audio_codec_params: Option<CodecParameters>,
}

impl Mp4Demuxer {
    /// Open an MP4 from a streaming source.
    ///
    /// Accepts any `Read + Seek + Send` source (file, cursor, etc.) and passes
    /// it directly to symphonia's `MediaSourceStream`.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        let adapter = VideoSourceAdapter(source);
        let mss = MediaSourceStream::new(Box::new(adapter), Default::default());
        Self::open_from_mss(mss)
    }

    /// Open an MP4 from a byte buffer.
    pub fn open(data: Vec<u8>) -> Result<Self, VideoError> {
        Self::open_stream(Box::new(Cursor::new(data)))
    }

    /// Shared probe + track discovery for both `open` and `open_stream`.
    fn open_from_mss(mss: MediaSourceStream) -> Result<Self, VideoError> {
        let mut hint = Hint::new();
        hint.with_extension("mp4");

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| VideoError::Demux(format!("probe failed: {e}")))?;

        let reader = probed.format;

        let mut video_track_id = None;
        let mut audio_track_id = None;
        let mut video_timebase = (1u64, 1u64);
        let mut audio_timebase = (1u64, 1u64);
        let mut video_codec_params = None;
        let mut audio_codec_params = None;

        for track in reader.tracks() {
            let codec = track.codec_params.codec;
            let is_video = codec.to_string().contains("h264")
                || codec == symphonia::core::codecs::CODEC_TYPE_NULL;
            let is_audio = codec.to_string().contains("aac")
                || matches!(codec, symphonia::core::codecs::CODEC_TYPE_AAC);

            if is_video && video_track_id.is_none() {
                video_track_id = Some(track.id);
                if let Some(tb) = track.codec_params.time_base {
                    video_timebase = (tb.numer as u64, tb.denom as u64);
                }
                video_codec_params = Some(track.codec_params.clone());
            } else if is_audio && audio_track_id.is_none() {
                audio_track_id = Some(track.id);
                if let Some(tb) = track.codec_params.time_base {
                    audio_timebase = (tb.numer as u64, tb.denom as u64);
                }
                audio_codec_params = Some(track.codec_params.clone());
            }
        }

        Ok(Self {
            reader,
            video_track_id,
            audio_track_id,
            video_timebase,
            audio_timebase,
            video_codec_params,
            audio_codec_params,
        })
    }

    /// Read the next packet from the container.
    ///
    /// Returns `None` at end-of-stream.
    pub fn next_packet(&mut self) -> Result<Option<DemuxedPacket>, VideoError> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                },
                Err(e) => return Err(VideoError::Demux(format!("read packet: {e}"))),
            };

            let track_id = packet.track_id();

            let (kind, timebase) = if Some(track_id) == self.video_track_id {
                (TrackKind::Video, self.video_timebase)
            } else if Some(track_id) == self.audio_track_id {
                (TrackKind::Audio, self.audio_timebase)
            } else {
                // Unknown track — skip.
                continue;
            };

            let ts = packet.ts();
            let timestamp_secs = if timebase.1 > 0 {
                (ts as f64) * (timebase.0 as f64) / (timebase.1 as f64)
            } else {
                0.0
            };

            return Ok(Some(DemuxedPacket {
                kind,
                data: packet.buf().to_vec(),
                timestamp_secs,
            }));
        }
    }

    /// Seek to a position in seconds.
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        self.reader
            .seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time: Time::new(secs as u64, secs.fract()),
                    track_id: None,
                },
            )
            .map_err(|e| VideoError::Demux(format!("seek: {e}")))?;
        Ok(())
    }

    /// The video track's codec parameters (for initializing the H.264 decoder).
    pub fn video_codec_params(&self) -> Option<&CodecParameters> {
        self.video_codec_params.as_ref()
    }

    /// The audio track's codec parameters (for initializing the AAC decoder).
    pub fn audio_codec_params(&self) -> Option<&CodecParameters> {
        self.audio_codec_params.as_ref()
    }

    /// Whether a video track was found.
    pub fn has_video(&self) -> bool {
        self.video_track_id.is_some()
    }

    /// Whether an audio track was found.
    pub fn has_audio(&self) -> bool {
        self.audio_track_id.is_some()
    }
}
