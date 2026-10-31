//! MP4 demuxer wrapping symphonia's `FormatReader`.
//!
//! Provides track discovery and per-packet routing so the caller can dispatch
//! H.264 video packets and AAC audio packets to their respective decoders.

#[cfg(test)]
use std::io::Cursor;
use std::io::{Read, Seek, SeekFrom};

use symphonia::core::codecs::{CODEC_TYPE_AAC, CODEC_TYPE_NULL, CodecParameters};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::demux_lite::avcc::{ANNEX_B_START, append_annex_b, parse_avcc_record};
use crate::{VideoError, VideoSource};

/// Largest `moov` atom [`read_top_level_atom`] loads (64 MiB).  Even a
/// multi-hour file's sample tables are a few MB; past this the file is
/// hostile or not an MP4.
pub(crate) const MAX_MOOV_BYTES: u64 = 64 * 1024 * 1024;

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

/// Annex B start code (4-byte variant).
const ANNEX_B_START_CODE: [u8; 4] = ANNEX_B_START;

/// H.264 AVCC-to-Annex B conversion state parsed from the avcC box.
pub struct AvccConfig {
    /// Number of bytes in each NAL length prefix (1, 2, 3, or 4).
    nal_length_size: usize,
    /// SPS + PPS NAL units formatted as Annex B (with start codes).
    parameter_sets: Vec<u8>,
}

/// Parse the AVCDecoderConfigurationRecord (avcC box contents).
///
/// Returns the NAL length size and every SPS/PPS as one Annex B bytestream
/// (openh264 takes them all in-band, unlike `demux_lite::AvccConfig`,
/// which keeps one of each for the PSP's ME decoder).
fn parse_avcc(data: &[u8]) -> Result<AvccConfig, VideoError> {
    let record = parse_avcc_record(data).map_err(|e| VideoError::Demux(e.to_string()))?;
    let mut parameter_sets = Vec::new();
    for set in record.sps.iter().chain(&record.pps) {
        parameter_sets.extend_from_slice(&ANNEX_B_START_CODE);
        parameter_sets.extend_from_slice(set);
    }
    Ok(AvccConfig {
        nal_length_size: record.nal_length_size,
        parameter_sets,
    })
}

/// Find the `avcC` box in MP4 bytes (a whole file or just its `moov`) and
/// parse it.
///
/// Symphonia is audio-focused and doesn't extract video codec parameters,
/// so we find the avcC atom ourselves.  The scan is confined to the `moov`
/// atom when the bytes start with top-level boxes that include one, so
/// `avcC`-looking bytes in `mdat` (a moov-at-end file) cannot be mistaken
/// for it.
pub fn find_avcc_in_mp4(mp4_data: &[u8]) -> Option<AvccConfig> {
    let data = find_top_level_atom(mp4_data, b"moov").unwrap_or(mp4_data);
    // [4-byte size]['avcC'][AVCDecoderConfigurationRecord]
    data.windows(8)
        .enumerate()
        .filter(|(_, w)| &w[4..8] == b"avcC")
        .find_map(|(i, w)| {
            let box_size = u32::from_be_bytes([w[0], w[1], w[2], w[3]]) as usize;
            let contents = data.get(i + 8..i.checked_add(box_size)?)?;
            parse_avcc(contents).ok()
        })
}

/// Find a top-level MP4 atom (e.g. `moov`) in a complete file buffer and
/// return it including its header.
pub(crate) fn find_top_level_atom<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
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

/// Read the top-level atom `fourcc` (header included) from a seekable
/// stream by walking the top-level box headers, seeking over every other
/// box (`mdat` is never read).  Leaves the stream position unspecified.
///
/// `Ok(None)` when the stream has no such atom, or it is larger than
/// `max_bytes`.
pub(crate) fn read_top_level_atom<R: Read + Seek + ?Sized>(
    r: &mut R,
    fourcc: &[u8; 4],
    max_bytes: u64,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut pos = r.seek(SeekFrom::Start(0))?;
    loop {
        let mut header = [0u8; 16];
        match r.read_exact(&mut header[..8]) {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let size = match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
            0 => r.seek(SeekFrom::End(0))?.saturating_sub(pos),
            1 => {
                r.read_exact(&mut header[8..16])?;
                u64::from_be_bytes([
                    header[8], header[9], header[10], header[11], header[12], header[13],
                    header[14], header[15],
                ])
            },
            n => u64::from(n),
        };
        if size < 8 {
            return Ok(None);
        }
        if &header[4..8] == fourcc {
            if size > max_bytes {
                return Ok(None);
            }
            r.seek(SeekFrom::Start(pos))?;
            // Grow with the bytes actually read rather than allocating the
            // declared size up front: a truncated file (or a hostile header)
            // claiming a 64 MiB `moov` must not cost 64 MiB.
            let mut atom = Vec::new();
            r.take(size).read_to_end(&mut atom)?;
            if atom.len() as u64 != size {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            return Ok(Some(atom));
        }
        let Some(next) = pos.checked_add(size) else {
            return Ok(None);
        };
        pos = r.seek(SeekFrom::Start(next))?;
    }
}

/// MP4 demuxer backed by symphonia.
pub struct Mp4Demuxer {
    reader: Box<dyn FormatReader>,
    video_track_id: Option<u32>,
    audio_track_id: Option<u32>,
    video_timebase: (u64, u64), // (numer, denom)
    audio_timebase: (u64, u64),
    audio_codec_params: Option<CodecParameters>,
    /// AVCC config parsed from the avcC box (None if no video or not AVCC).
    avcc: Option<AvccConfig>,
    /// Whether the SPS/PPS parameter sets have been prepended to the first
    /// video packet.
    sent_params: bool,
}

impl Mp4Demuxer {
    /// Open an MP4 from a streaming source, also returning the `moov`
    /// bytes (empty if none was found) for callers that index them further.
    ///
    /// Locates the `moov` atom by walking the top-level box headers
    /// (seeking over `mdat`, so only `moov` is read), takes the avcC
    /// config from it, then rewinds and hands the source to symphonia.
    pub(crate) fn open_stream_and_moov(
        mut source: Box<dyn VideoSource>,
    ) -> Result<(Self, Vec<u8>), VideoError> {
        let moov = read_top_level_atom(&mut source, b"moov", MAX_MOOV_BYTES)
            .map_err(|e| VideoError::Demux(format!("read moov: {e}")))?
            .unwrap_or_default();
        source
            .seek(SeekFrom::Start(0))
            .map_err(|e| VideoError::Demux(format!("seek to start: {e}")))?;
        let avcc = find_avcc_in_mp4(&moov);
        Ok((Self::open_stream_with_avcc(source, avcc)?, moov))
    }

    /// Open an MP4 from a streaming source with pre-extracted avcC config.
    ///
    /// Skips the `moov` lookup -- the caller has already extracted avcC
    /// from the moov atom (e.g. via a Range request).
    pub fn open_stream_with_avcc(
        source: Box<dyn VideoSource>,
        avcc: Option<AvccConfig>,
    ) -> Result<Self, VideoError> {
        let adapter = VideoSourceAdapter(source);
        let mss = MediaSourceStream::new(Box::new(adapter), Default::default());
        Self::open_from_mss(mss, avcc)
    }

    /// Open an MP4 from a byte buffer.
    #[cfg(test)]
    pub fn open(data: Vec<u8>) -> Result<Self, VideoError> {
        // Scan for avcC box before symphonia consumes the data, since
        // symphonia doesn't extract video codec parameters.
        let avcc = find_avcc_in_mp4(&data);
        let cursor = Cursor::new(data);
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
        Self::open_from_mss(mss, avcc)
    }

    /// Shared probe + track discovery for both `open` and `open_stream`.
    fn open_from_mss(mss: MediaSourceStream, avcc: Option<AvccConfig>) -> Result<Self, VideoError> {
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
        let mut audio_codec_params = None;

        for track in reader.tracks() {
            let codec = track.codec_params.codec;
            // symphonia's isomp4 reader has no video codec types: it reports
            // every sample entry it does not decode (avc1/avc3 included) as
            // CODEC_TYPE_NULL, so the first such track is taken as video.
            if codec == CODEC_TYPE_NULL && video_track_id.is_none() {
                video_track_id = Some(track.id);
                if let Some(tb) = track.codec_params.time_base {
                    video_timebase = (u64::from(tb.numer), u64::from(tb.denom));
                }
            } else if codec == CODEC_TYPE_AAC && audio_track_id.is_none() {
                audio_track_id = Some(track.id);
                if let Some(tb) = track.codec_params.time_base {
                    audio_timebase = (u64::from(tb.numer), u64::from(tb.denom));
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
            audio_codec_params,
            avcc,
            sent_params: false,
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

            let data = match (&self.avcc, kind) {
                (Some(avcc), TrackKind::Video) => {
                    // Convert AVCC length-prefixed NALs to Annex B start
                    // codes, with SPS/PPS before the first video packet.
                    let mut annex_b =
                        Vec::with_capacity(avcc.parameter_sets.len() + packet.buf().len() + 32);
                    if !self.sent_params {
                        annex_b.extend_from_slice(&avcc.parameter_sets);
                        self.sent_params = true;
                    }
                    append_annex_b(packet.buf(), avcc.nal_length_size, &mut annex_b)
                        .map_err(|e| VideoError::Demux(e.to_string()))?;
                    annex_b
                },
                _ => packet.buf().to_vec(),
            };

            return Ok(Some(DemuxedPacket {
                kind,
                data,
                timestamp_secs,
            }));
        }
    }

    /// Seek to a position in seconds (negative / non-finite = the start).
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        // `Time` wants whole seconds plus a fraction in [0, 1).
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        self.reader
            .seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time: Time::new(secs.trunc() as u64, secs.fract()),
                    track_id: None,
                },
            )
            .map_err(|e| VideoError::Demux(format!("seek: {e}")))?;
        // Re-send SPS/PPS after seek so the decoder can reinitialize.
        self.sent_params = false;
        Ok(())
    }

    /// Get a copy of the SPS/PPS parameter sets as Annex-B data.
    /// Returns `None` if no AVCC config was found.
    #[cfg(feature = "h264")]
    pub fn parameter_sets(&self) -> Option<&[u8]> {
        self.avcc.as_ref().map(|a| a.parameter_sets.as_slice())
    }

    /// The audio track's codec parameters (for initializing the AAC decoder).
    pub fn audio_codec_params(&self) -> Option<&CodecParameters> {
        self.audio_codec_params.as_ref()
    }

    /// Whether a video track was found.
    #[cfg(any(feature = "h264", test))]
    pub fn has_video(&self) -> bool {
        self.video_track_id.is_some()
    }

    /// Whether an audio track was found.
    #[cfg(test)]
    pub fn has_audio(&self) -> bool {
        self.audio_track_id.is_some()
    }
}

#[cfg(test)]
mod tests;
