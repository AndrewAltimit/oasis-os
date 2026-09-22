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
    /// Video keyframes as `(decode_secs, seek_secs)` pairs, ascending (see
    /// [`demux_lite::TrackInfo::keyframe_times`]).  Symphonia ignores
    /// `stss`, so [`seek`](Self::seek) uses this to land on a keyframe
    /// instead of mid-GOP.  Empty when unknown.
    #[cfg(not(feature = "ffmpeg"))]
    keyframes: Vec<(f64, f64)>,
    /// Discard video packets until the next IDR (after open, seek, or a
    /// decoder resync).  Feeding openh264 P/B-frames whose references it
    /// never saw only produces errors.
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    awaiting_idr: bool,
    /// Pictures drained from the decoder's reorder buffer at end of stream.
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    flushed_frames: VecDeque<VideoFrame>,
    /// Timestamp and spacing of the last video packet (for flushed frames).
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    last_video_ts: (f64, f64),
    /// Whether the demuxer has run out of video packets.
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    video_eof: bool,
    /// Timestamps of packets fed to the decoder whose pictures have not
    /// come out yet.  With B-frames the decoder emits pictures in display
    /// order a few packets after they were fed, so a picture takes the
    /// smallest pending timestamp -- not the timestamp of the packet that
    /// happened to release it (which ran every frame late by the reorder
    /// depth and started post-seek playback one frame past the keyframe).
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    pending_pts: Vec<f64>,
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
            let (demuxer, keyframes) = Mp4Demuxer::open_stream_scanned(source, |data| {
                find_top_level_atom(data, b"moov").map(keyframes_from_moov)
            })?;
            let mut dec = Self::from_demuxer(demuxer)?;
            dec.keyframes = keyframes.unwrap_or_default();
            Ok(dec)
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

    /// Open from a streaming source using a pre-fetched `moov` atom (the
    /// complete atom, header included).
    ///
    /// Like [`open_stream_with_avcc`](Self::open_stream_with_avcc) (no
    /// full-file scan), and additionally indexes the video keyframes so
    /// [`seek`](Self::seek) lands on a keyframe.
    #[cfg(not(feature = "ffmpeg"))]
    pub fn open_stream_with_moov(
        source: Box<dyn VideoSource>,
        moov: &[u8],
    ) -> Result<Self, VideoError> {
        let mut dec = Self::open_stream_with_avcc(source, find_avcc_in_mp4(moov))?;
        dec.keyframes = keyframes_from_moov(moov);
        Ok(dec)
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
            keyframes: Vec::new(),
            #[cfg(feature = "h264")]
            awaiting_idr: true,
            #[cfg(feature = "h264")]
            flushed_frames: VecDeque::new(),
            #[cfg(feature = "h264")]
            last_video_ts: (0.0, 0.0),
            #[cfg(feature = "h264")]
            video_eof: false,
            #[cfg(feature = "h264")]
            pending_pts: Vec::new(),
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
            if let Some(frame) = self.flushed_frames.pop_front() {
                return Ok(Some(frame));
            }
            if self.video_eof {
                return Ok(None);
            }

            // Packets examined without producing a frame before giving up
            // with `SkipLimit` (the caller keeps audio going and retries).
            const MAX_SKIP: u32 = 600;
            // Consecutive decode errors after which the decoder is rebuilt
            // and restarted at the next IDR.  A few isolated errors are
            // tolerated (concealment keeps producing pictures); a streak
            // means the reference chain or the decoder state is gone.
            const RESYNC_ERRORS: u32 = 3;

            let mut skipped = 0u32;
            loop {
                let Some(packet) = self.next_packet_for(TrackKind::Video)? else {
                    return Ok(self.flush_video_at_eof());
                };
                self.note_video_ts(packet.timestamp_secs);

                let is_idr = Self::contains_idr(&packet.data);
                if self.awaiting_idr && !is_idr {
                    skipped += 1;
                    if skipped > MAX_SKIP {
                        return Err(VideoError::SkipLimit);
                    }
                    continue;
                }
                if self.awaiting_idr {
                    self.awaiting_idr = false;
                    if skipped > 0 {
                        log::info!("H264: synced at IDR after skipping {skipped} packets");
                    }
                }

                // Every IDR carries SPS/PPS so a decoder that lost its
                // parameter sets (reinit, internal error reset) recovers at
                // the next keyframe instead of failing every later packet.
                let with_params;
                let data: &[u8] = match self.demuxer.parameter_sets() {
                    Some(ps) if is_idr && !Self::contains_nal_type(&packet.data, NAL_SPS) => {
                        with_params = [ps, packet.data.as_slice()].concat();
                        &with_params
                    },
                    _ => &packet.data,
                };

                self.pending_pts.push(packet.timestamp_secs);
                let h264 = self
                    .h264
                    .as_mut()
                    .expect("h264 decoder verified present above");
                if let Some(frame) = h264.decode(data)? {
                    if frame.width > 0 && frame.height > 0 {
                        self.video_width = frame.width;
                        self.video_height = frame.height;
                    }
                    let ts = pop_min_pts(&mut self.pending_pts).unwrap_or(packet.timestamp_secs);
                    return Ok(Some(VideoFrame {
                        rgba: frame.rgba,
                        width: frame.width,
                        height: frame.height,
                        timestamp_secs: ts,
                    }));
                }

                if h264.last_failed {
                    // The packet produced no picture and never will.
                    self.pending_pts.pop();
                } else if self.pending_pts.len() > MAX_REORDER_DEPTH {
                    // A picture was dropped without an error; don't let its
                    // stale timestamp shift every later frame.
                    pop_min_pts(&mut self.pending_pts);
                }

                if h264.last_failed && h264.error_streak >= RESYNC_ERRORS {
                    log::info!(
                        "H264: {} consecutive decode errors, resyncing at next IDR",
                        h264.error_streak,
                    );
                    h264.reinit()?;
                    self.awaiting_idr = true;
                    self.pending_pts.clear();
                }

                skipped += 1;
                if skipped > MAX_SKIP {
                    return Err(VideoError::SkipLimit);
                }
            }
        }
    }

    /// Record a video packet timestamp (for timing frames flushed at EOF).
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    fn note_video_ts(&mut self, ts: f64) {
        let (last, dur) = self.last_video_ts;
        let step = ts - last;
        let dur = if step > 0.0 && step < 1.0 { step } else { dur };
        self.last_video_ts = (ts, dur);
    }

    /// At end of stream, drain the pictures still in the H.264 reorder
    /// buffer and return the first of them.
    #[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
    fn flush_video_at_eof(&mut self) -> Option<VideoFrame> {
        if !self.video_eof {
            self.video_eof = true;
            let (last_ts, dur) = self.last_video_ts;
            if let Some(h264) = self.h264.as_mut() {
                for (i, frame) in h264.flush().into_iter().enumerate() {
                    let ts = pop_min_pts(&mut self.pending_pts)
                        .unwrap_or(last_ts + dur * (i + 1) as f64);
                    self.flushed_frames.push_back(VideoFrame {
                        rgba: frame.rgba,
                        width: frame.width,
                        height: frame.height,
                        timestamp_secs: ts,
                    });
                }
            }
            self.pending_pts.clear();
        }
        self.flushed_frames.pop_front()
    }

    /// The time to seek the demuxer to for a request of `secs`: the seek
    /// time of the last keyframe starting at or before `secs` (the first
    /// keyframe if `secs` precedes it), or `secs` when no index is known.
    #[cfg(not(feature = "ffmpeg"))]
    pub fn keyframe_seek_target(&self, secs: f64) -> f64 {
        let n = self
            .keyframes
            .partition_point(|&(decode, _)| decode <= secs);
        match n {
            0 => self.keyframes.first().map_or(secs, |&(_, seek)| seek),
            n => self.keyframes[n - 1].1,
        }
    }

    /// Check if an Annex-B bitstream contains an IDR NAL unit (type 5).
    #[cfg(feature = "h264")]
    fn contains_idr(data: &[u8]) -> bool {
        Self::contains_nal_type(data, NAL_IDR)
    }

    /// Check if an Annex-B bitstream contains a NAL unit of type `nal_type`.
    #[cfg(feature = "h264")]
    fn contains_nal_type(data: &[u8], nal_type: u8) -> bool {
        let mut i = 0;
        while i + 4 <= data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
                if i + 4 < data.len() && (data[i + 4] & 0x1F) == nal_type {
                    return true;
                }
                i += 4;
            } else if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                if i + 3 < data.len() && (data[i + 3] & 0x1F) == nal_type {
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
            let target = self.keyframe_seek_target(secs);
            if (target - secs).abs() > 0.05 {
                log::info!("seek {secs:.2}s -> keyframe at {target:.2}s");
            }
            self.demuxer.seek(target)?;
            #[cfg(feature = "h264")]
            {
                // Pictures still in the reorder buffer belong to the old
                // position; restart the decoder at the next IDR.
                if let Some(h264) = self.h264.as_mut() {
                    h264.reinit()?;
                }
                self.awaiting_idr = true;
                self.flushed_frames.clear();
                self.video_eof = false;
                self.last_video_ts = (0.0, 0.0);
                self.pending_pts.clear();
            }
            Ok(())
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

/// Most pictures an H.264 decoder may hold for reordering
/// (`max_dec_frame_buffering` is at most 16).
#[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
const MAX_REORDER_DEPTH: usize = 16;

/// Remove and return the smallest timestamp in `pending`.
#[cfg(all(not(feature = "ffmpeg"), feature = "h264"))]
fn pop_min_pts(pending: &mut Vec<f64>) -> Option<f64> {
    let idx = pending
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)?;
    Some(pending.swap_remove(idx))
}

/// H.264 NAL unit type of an IDR slice.
#[cfg(feature = "h264")]
const NAL_IDR: u8 = 5;
/// H.264 NAL unit type of a sequence parameter set.
#[cfg(feature = "h264")]
const NAL_SPS: u8 = 7;

/// Find a top-level MP4 atom (e.g. `moov`) in a complete file buffer and
/// return it including its header.
#[cfg(not(feature = "ffmpeg"))]
fn find_top_level_atom<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut pos = 0usize;
    while pos.checked_add(8)? <= data.len() {
        let size32 = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let size = match size32 {
            0 => data.len() - pos,
            1 => {
                let b = data.get(pos + 8..pos + 16)?;
                usize::try_from(u64::from_be_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ]))
                .ok()?
            },
            n => n as usize,
        };
        if size < 8 {
            return None;
        }
        let end = pos.checked_add(size)?;
        if &data[pos + 4..pos + 8] == fourcc {
            return data.get(pos..end);
        }
        pos = end;
    }
    None
}

/// Video keyframe `(decode_secs, seek_secs)` pairs from raw moov bytes.
#[cfg(not(feature = "ffmpeg"))]
fn keyframes_from_moov(moov: &[u8]) -> Vec<(f64, f64)> {
    demux_lite::parse_moov_tracks(moov)
        .ok()
        .and_then(|(video, _)| video)
        .map(|v| v.keyframe_times())
        .unwrap_or_default()
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

    #[test]
    #[cfg(feature = "h264")]
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
    #[cfg(not(feature = "ffmpeg"))]
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
    #[cfg(not(feature = "ffmpeg"))]
    fn open_stream_with_moov_builds_keyframe_index() {
        let data = fixture_bytes();
        let moov = find_top_level_atom(&data, b"moov").expect("moov").to_vec();
        let dec = SoftwareVideoDecoder::open_stream_with_moov(Box::new(Cursor::new(data)), &moov)
            .expect("open failed");
        let second = (15.0 * 1024.0 + 512.0) / 15360.0;
        assert!((dec.keyframe_seek_target(1.5) - second).abs() < 1e-9);
    }

    #[test]
    #[cfg(feature = "h264")]
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

    #[test]
    #[cfg(not(feature = "ffmpeg"))]
    fn find_top_level_atom_walks_boxes() {
        let data = fixture_bytes();
        let moov = find_top_level_atom(&data, b"moov").expect("moov");
        assert_eq!(&moov[4..8], b"moov");
        assert_eq!(moov.len(), 2235);
        assert!(find_top_level_atom(&data, b"zzzz").is_none());
        assert!(find_top_level_atom(&[0, 0, 0, 3, b'b', b'a', b'd', b'!'], b"moov").is_none());
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

    // ---------------------------------------------------------------
    // Item 76: H.264 NAL unit / SPS/PPS / IDR detection tests
    // (contains_idr is defined here, gated on feature = "h264")
    // ---------------------------------------------------------------

    #[cfg(feature = "h264")]
    mod nal_tests {
        #![allow(clippy::unwrap_used)]

        use super::*;

        #[test]
        fn contains_idr_4byte_start_code() {
            // IDR NAL type = 5 (0x65 = 0b01100101, type = 5)
            let data = [0x00, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB];
            assert!(SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_3byte_start_code() {
            // 3-byte start code with IDR NAL type 5
            let data = [0x00, 0x00, 0x01, 0x65, 0xAA];
            assert!(SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_non_idr_nal() {
            // NAL type 1 (non-IDR coded slice, 0x41 & 0x1F = 1)
            let data = [0x00, 0x00, 0x00, 0x01, 0x41, 0xAA];
            assert!(!SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_sps_not_idr() {
            // SPS NAL type = 7 (0x67 & 0x1F = 7)
            let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0];
            assert!(!SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_pps_not_idr() {
            // PPS NAL type = 8 (0x68 & 0x1F = 8)
            let data = [0x00, 0x00, 0x00, 0x01, 0x68, 0xCE];
            assert!(!SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_empty_data() {
            assert!(!SoftwareVideoDecoder::contains_idr(&[]));
        }

        #[test]
        fn contains_idr_too_short_for_start_code() {
            assert!(!SoftwareVideoDecoder::contains_idr(&[0x00, 0x00]));
            assert!(!SoftwareVideoDecoder::contains_idr(&[0x00, 0x00, 0x01]));
        }

        #[test]
        fn contains_idr_multiple_nals_idr_second() {
            // SPS then IDR
            let mut data = Vec::new();
            data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]); // SPS
            data.extend_from_slice(&[0x42, 0xC0, 0x1E]); // SPS data
            data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x65]); // IDR
            data.extend_from_slice(&[0xAA, 0xBB]); // IDR data
            assert!(SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_idr_with_nal_ref_idc() {
            // NAL byte = 0x25: nal_ref_idc=1, type=5 (IDR)
            let data = [0x00, 0x00, 0x00, 0x01, 0x25, 0xAA];
            assert!(SoftwareVideoDecoder::contains_idr(&data));
        }

        #[test]
        fn contains_idr_start_code_at_end_no_nal_byte() {
            // Start code is the last bytes, no NAL type byte follows.
            let data = [0x00, 0x00, 0x00, 0x01];
            assert!(!SoftwareVideoDecoder::contains_idr(&data));
        }
    }
}
