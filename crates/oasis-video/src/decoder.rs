//! [`SoftwareVideoDecoder`] and the backend contract it forwards to.

use std::io::Cursor;

use crate::{AudioChunk, VideoError, VideoFrame, VideoSource};

/// What a decode backend provides. [`SoftwareVideoDecoder`] forwards every
/// call to the backend selected at compile time ([`Backend`]); the
/// defaults cover backends that read the container themselves (ffmpeg).
pub(crate) trait DecoderBackend: Sized {
    /// Open an MP4 from a streaming source.
    fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError>;

    /// Open using a pre-fetched `moov` atom (header included). Defaults to
    /// [`open_stream`](Self::open_stream).
    fn open_stream_with_moov(
        source: Box<dyn VideoSource>,
        moov: &[u8],
    ) -> Result<Self, VideoError> {
        let _ = moov;
        Self::open_stream(source)
    }

    /// The next video frame; `Ok(None)` at end of stream.
    fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError>;

    /// The next audio chunk; `Ok(None)` at end of stream.
    fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError>;

    /// A buffered audio chunk, without reading from the stream.
    fn next_buffered_audio(&mut self) -> Option<AudioChunk>;

    /// Seek to `secs`, dropping buffered packets.
    fn seek(&mut self, secs: f64) -> Result<(), VideoError>;

    /// Where a seek to `secs` lands. Defaults to `secs` (backends that
    /// snap to keyframes themselves).
    fn keyframe_seek_target(&self, secs: f64) -> f64 {
        secs
    }

    /// Video dimensions (0x0 without a video track).
    fn video_size(&self) -> (u32, u32);

    /// Audio sample rate and channel count.
    fn audio_format(&self) -> (u32, u16);
}

/// The decoder backend [`SoftwareVideoDecoder`] wraps: ffmpeg when the
/// `ffmpeg` feature is on (it wins over `symphonia`/`h264`), otherwise
/// symphonia + openh264.
#[cfg(feature = "ffmpeg")]
type Backend = crate::ffmpeg_decoder::FfmpegDecoder;
#[cfg(all(feature = "symphonia", not(feature = "ffmpeg")))]
type Backend = crate::symphonia_decoder::SymphoniaDecoder;

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
pub struct SoftwareVideoDecoder {
    inner: Backend,
}

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
        Ok(Self {
            inner: Backend::open_stream_with_moov(source, moov)?,
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
        self.inner.keyframe_seek_target(secs)
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
