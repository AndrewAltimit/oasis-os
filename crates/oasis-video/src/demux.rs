//! MP4 demuxer wrapping symphonia's `FormatReader`.
//!
//! Provides track discovery and per-packet routing so the caller can dispatch
//! H.264 video packets and AAC audio packets to their respective decoders.

use std::io::{Cursor, Read, Seek, SeekFrom};

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
    video_codec_params: Option<CodecParameters>,
    audio_codec_params: Option<CodecParameters>,
    /// AVCC config parsed from the avcC box (None if no video or not AVCC).
    avcc: Option<AvccConfig>,
    /// Whether the SPS/PPS parameter sets have been prepended to the first
    /// video packet.
    sent_params: bool,
}

impl Mp4Demuxer {
    /// Open an MP4 from a streaming source.
    ///
    /// Locates the `moov` atom by walking the top-level box headers
    /// (seeking over `mdat`, so only `moov` is read), takes the avcC
    /// config from it, then rewinds and hands the source to symphonia.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        Self::open_stream_and_moov(source).map(|(demuxer, _)| demuxer)
    }

    /// [`open_stream`](Self::open_stream), also returning the `moov` bytes
    /// (empty if none was found) for callers that index them further.
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
        let mut video_codec_params = None;
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
                video_codec_params = Some(track.codec_params.clone());
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
            video_codec_params,
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

    /// Reset the SPS/PPS sent flag so parameter sets are re-prepended
    /// to the next video packet. Used after decoder error recovery.
    pub fn reset_params(&mut self) {
        self.sent_params = false;
    }

    /// Get a copy of the SPS/PPS parameter sets as Annex-B data.
    /// Returns `None` if no AVCC config was found.
    pub fn parameter_sets(&self) -> Option<&[u8]> {
        self.avcc.as_ref().map(|a| a.parameter_sets.as_slice())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One AVCC packet as Annex B (the conversion `next_packet` applies).
    fn avcc_to_annex_b(data: &[u8], nal_length_size: usize) -> Result<Vec<u8>, VideoError> {
        let mut out = Vec::new();
        append_annex_b(data, nal_length_size, &mut out)
            .map_err(|e| VideoError::Demux(e.to_string()))?;
        Ok(out)
    }

    #[test]
    fn parse_avcc_extracts_sps_pps() {
        // Minimal avcC: version=1, profile=66(baseline), compat=0xC0, level=30,
        // nal_length_size=4 (0xFF & 0x03 = 3, +1=4),
        // 1 SPS of 4 bytes, 1 PPS of 3 bytes.
        #[rustfmt::skip]
        let avcc_data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
            0xFF,                     // 6 reserved bits + nal_length_size_minus_one=3
            0xE1,                     // 3 reserved bits + num_sps=1
            0x00, 0x04,               // SPS length = 4
            0x67, 0x42, 0xC0, 0x1E,  // SPS NAL data
            0x01,                     // num_pps = 1
            0x00, 0x03,               // PPS length = 3
            0x68, 0xCE, 0x38,        // PPS NAL data
        ];
        let config = parse_avcc(avcc_data).unwrap();
        assert_eq!(config.nal_length_size, 4);
        // parameter_sets should be: [start_code][SPS][start_code][PPS]
        let expected: Vec<u8> = [
            &ANNEX_B_START_CODE[..],
            &[0x67, 0x42, 0xC0, 0x1E],
            &ANNEX_B_START_CODE[..],
            &[0x68, 0xCE, 0x38],
        ]
        .concat();
        assert_eq!(config.parameter_sets, expected);
    }

    #[test]
    fn parse_avcc_rejects_short_data() {
        assert!(parse_avcc(&[0x01, 0x42, 0xC0]).is_err());
    }

    #[test]
    fn avcc_to_annex_b_converts_4byte_lengths() {
        // Two NALs: length=3 + data, length=2 + data.
        #[rustfmt::skip]
        let avcc_packet: &[u8] = &[
            0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC, // NAL 1
            0x00, 0x00, 0x00, 0x02, 0xDD, 0xEE,        // NAL 2
        ];
        let annex_b = avcc_to_annex_b(avcc_packet, 4).unwrap();
        let expected: Vec<u8> = [
            &ANNEX_B_START_CODE[..],
            &[0xAA, 0xBB, 0xCC],
            &ANNEX_B_START_CODE[..],
            &[0xDD, 0xEE],
        ]
        .concat();
        assert_eq!(annex_b, expected);
    }

    #[test]
    fn avcc_to_annex_b_truncated_nal() {
        // Length says 10 bytes but only 2 available.
        let bad = &[0x00, 0x00, 0x00, 0x0A, 0xAA, 0xBB];
        assert!(avcc_to_annex_b(bad, 4).is_err());
    }

    #[test]
    fn find_avcc_in_mp4_locates_box() {
        // Fake a minimal MP4 with an avcC box embedded.
        let avcc_contents: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, 0x00, 0x02, 0x67, 0x42, 0x01, 0x00, 0x01, 0x68,
        ];
        let box_size = (8 + avcc_contents.len()) as u32;
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(b"JUNK"); // leading garbage
        mp4.extend_from_slice(&box_size.to_be_bytes());
        mp4.extend_from_slice(b"avcC");
        mp4.extend_from_slice(avcc_contents);
        mp4.extend_from_slice(b"TAIL"); // trailing data

        let config = find_avcc_in_mp4(&mp4).expect("should find avcC box");
        assert_eq!(config.nal_length_size, 4);
        assert!(!config.parameter_sets.is_empty());
    }

    #[test]
    fn find_avcc_returns_none_for_no_avcc() {
        let garbage = b"this is not an mp4 file at all";
        assert!(find_avcc_in_mp4(garbage).is_none());
    }

    #[test]
    fn avcc_to_annex_b_huge_nal_length_no_overflow() {
        // Craft a packet where the 4-byte NAL length declares 0xFFFFFFFF bytes.
        // The bounds check must not panic from integer overflow in offset+nal_len.
        let bad = &[0xFF, 0xFF, 0xFF, 0xFF, 0xAA, 0xBB];
        assert!(avcc_to_annex_b(bad, 4).is_err());
    }

    #[test]
    fn avcc_to_annex_b_empty_input() {
        let empty: &[u8] = &[];
        let result = avcc_to_annex_b(empty, 4).unwrap();
        assert!(result.is_empty());
    }

    // ---------------------------------------------------------------
    // Item 71: Video demux error path tests (truncated headers,
    // missing atoms, edge cases)
    // ---------------------------------------------------------------

    #[test]
    fn parse_avcc_exactly_7_bytes_minimal() {
        // 7 bytes is the minimum length check. Build a valid-ish header
        // with 0 SPS entries and 0 PPS entries.
        let data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
            0xFF, // nal_length_size_minus_one = 3 -> size = 4
            0xE0, // num_sps = 0
            0x00, // num_pps = 0
        ];
        let config = parse_avcc(data).expect("should parse minimal avcC");
        assert_eq!(config.nal_length_size, 4);
        assert!(config.parameter_sets.is_empty());
    }

    #[test]
    fn parse_avcc_sps_length_exceeds_data() {
        // num_sps = 1, SPS length = 100 but only 2 bytes of data follow.
        let data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
            0x00, 0x64, // SPS length = 100
            0x67, 0x42, // only 2 bytes
        ];
        assert!(parse_avcc(data).is_err());
    }

    #[test]
    fn parse_avcc_pps_length_exceeds_data() {
        // Valid SPS of 2 bytes, then PPS claims 50 bytes but only 1 available.
        let data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
            0x00, 0x02, // SPS length = 2
            0x67, 0x42, // SPS data
            0x01, // num_pps = 1
            0x00, 0x32, // PPS length = 50
            0x68, // only 1 byte of PPS
        ];
        assert!(parse_avcc(data).is_err());
    }

    #[test]
    fn parse_avcc_pps_count_missing() {
        // Valid SPS but data ends before PPS count byte.
        let data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
            0x00, 0x02, // SPS length = 2
            0x67, 0x42, // SPS data — ends here, no PPS count
        ];
        assert!(parse_avcc(data).is_err());
    }

    #[test]
    fn parse_avcc_multiple_sps_pps() {
        // 2 SPS entries and 2 PPS entries.
        #[rustfmt::skip]
        let data: &[u8] = &[
            0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
            0xFF,                     // nal_length_size = 4
            0xE2,                     // num_sps = 2
            0x00, 0x02, 0x67, 0x42,  // SPS 1 (2 bytes)
            0x00, 0x01, 0x68,        // SPS 2 (1 byte)
            0x02,                     // num_pps = 2
            0x00, 0x01, 0xAA,        // PPS 1 (1 byte)
            0x00, 0x02, 0xBB, 0xCC,  // PPS 2 (2 bytes)
        ];
        let config = parse_avcc(data).expect("should parse multi-SPS/PPS");
        assert_eq!(config.nal_length_size, 4);
        // 2 SPS + 2 PPS = 4 start codes (4 bytes each) + data
        let expected_len = 4 * 4 + 2 + 1 + 1 + 2;
        assert_eq!(config.parameter_sets.len(), expected_len);
    }

    #[test]
    fn avcc_to_annex_b_1byte_nal_length() {
        // Single NAL with 1-byte length prefix: length=3, data=0xAA,0xBB,0xCC
        let data: &[u8] = &[0x03, 0xAA, 0xBB, 0xCC];
        let result = avcc_to_annex_b(data, 1).expect("should convert");
        let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xAA, 0xBB, 0xCC]].concat();
        assert_eq!(result, expected);
    }

    #[test]
    fn avcc_to_annex_b_2byte_nal_length() {
        // Single NAL with 2-byte length prefix: length=2, data=0xDD,0xEE
        let data: &[u8] = &[0x00, 0x02, 0xDD, 0xEE];
        let result = avcc_to_annex_b(data, 2).expect("should convert");
        let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xDD, 0xEE]].concat();
        assert_eq!(result, expected);
    }

    #[test]
    fn avcc_to_annex_b_3byte_nal_length() {
        // Single NAL with 3-byte length prefix: length=1, data=0xFF
        let data: &[u8] = &[0x00, 0x00, 0x01, 0xFF];
        let result = avcc_to_annex_b(data, 3).expect("should convert");
        let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xFF]].concat();
        assert_eq!(result, expected);
    }

    #[test]
    fn avcc_to_annex_b_invalid_nal_length_size() {
        let data: &[u8] = &[0x00; 8];
        assert!(avcc_to_annex_b(data, 5).is_err());
        assert!(avcc_to_annex_b(data, 0).is_err());
    }

    #[test]
    fn avcc_to_annex_b_zero_length_nal() {
        // NAL length = 0 should produce just a start code with empty data.
        let data: &[u8] = &[0x00, 0x00, 0x00, 0x00];
        let result = avcc_to_annex_b(data, 4).expect("should handle zero-len");
        assert_eq!(result, ANNEX_B_START_CODE);
    }

    #[test]
    fn avcc_to_annex_b_trailing_partial_length() {
        // Data has 3 bytes but nal_length_size = 4, so the trailing bytes
        // are too short for another NAL length. Should produce empty output
        // (loop condition `offset + nal_length_size <= data.len()` fails).
        let data: &[u8] = &[0x00, 0x00, 0x03];
        let result = avcc_to_annex_b(data, 4).expect("should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn find_avcc_in_mp4_with_zero_size_box() {
        // avcC box with size = 0 (invalid for sub-box). Should skip.
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&0u32.to_be_bytes());
        mp4.extend_from_slice(b"avcC");
        mp4.extend_from_slice(&[0; 20]);
        assert!(find_avcc_in_mp4(&mp4).is_none());
    }

    #[test]
    fn find_avcc_in_mp4_truncated_box_size() {
        // avcC fourcc found but box extends beyond data.
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&500u32.to_be_bytes()); // claims 500 bytes
        mp4.extend_from_slice(b"avcC");
        mp4.extend_from_slice(&[0; 10]); // only 10 bytes of content
        assert!(find_avcc_in_mp4(&mp4).is_none());
    }

    #[test]
    fn find_avcc_in_mp4_data_too_short_for_scan() {
        // Less than 8 bytes — loop never executes.
        assert!(find_avcc_in_mp4(b"short").is_none());
        assert!(find_avcc_in_mp4(b"").is_none());
    }

    #[test]
    fn demux_open_single_byte_fails() {
        let result = Mp4Demuxer::open(vec![0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn demux_open_ftyp_only_no_audio_no_video() {
        // Valid ftyp atom but no tracks.
        let mut data = Vec::new();
        let ftyp = b"isom\x00\x00\x00\x00isomavc1";
        let size = (8 + ftyp.len()) as u32;
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(ftyp);
        // Symphonia should either error or find no tracks.
        match Mp4Demuxer::open(data) {
            Ok(demuxer) => {
                assert!(
                    !demuxer.has_video() || !demuxer.has_audio(),
                    "ftyp-only should not have both tracks"
                );
            },
            Err(_) => {
                // Error on open is acceptable for data with no mdat/moov.
            },
        }
    }

    fn streaming_fixture(name: &str) -> Vec<u8> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read(manifest.join("../../tests/fixtures/streaming").join(name))
            .expect("fixture file missing")
    }

    #[test]
    fn read_top_level_atom_finds_moov_at_end() {
        let data = streaming_fixture("moov_end_4s.mp4");
        let expected = find_top_level_atom(&data, b"moov").expect("moov").to_vec();
        let mut cursor = Cursor::new(&data);
        let moov = read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap();
        assert_eq!(moov, Some(expected));
        // Over the size cap: not loaded.
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_top_level_atom(&mut cursor, b"moov", 64).unwrap(), None);
        // Garbage / absent.
        let mut cursor = Cursor::new(vec![0u8; 3]);
        assert_eq!(
            read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap(),
            None
        );
    }

    #[test]
    fn read_top_level_atom_rejects_a_moov_larger_than_the_file() {
        // A 16-byte file declaring a 60 MiB `moov`: an error, and no
        // up-front allocation of the declared size.
        let mut data = (60u32 << 20).to_be_bytes().to_vec();
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0; 8]);
        let mut cursor = Cursor::new(data);
        let err = read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn parse_avcc_keeps_every_parameter_set_in_order() {
        #[rustfmt::skip]
        let data: &[u8] = &[
            0x01, 0x64, 0x00, 0x1F, 0xFF,
            0xE2, 0, 1, 0x67, 0, 2, 0x67, 0x01, // 2 SPS
            0x01, 0, 1, 0x68,                   // 1 PPS
        ];
        let cfg = parse_avcc(data).unwrap();
        assert_eq!(
            cfg.parameter_sets,
            [0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x67, 0x01, 0, 0, 0, 1, 0x68]
        );
        for len in 0..data.len() {
            assert!(parse_avcc(&data[..len]).is_err(), "prefix {len}");
        }
    }

    #[test]
    fn find_avcc_ignores_decoys_outside_moov() {
        // Regression: the whole-file byte scan could take `avcC`-looking
        // bytes in `mdat` (before a moov-at-end) for the real config.
        #[rustfmt::skip]
        let decoy: &[u8] = &[
            0, 0, 0, 0x12, b'a', b'v', b'c', b'C',
            0x01, 0x42, 0xC0, 0x1E, 0xFC, 0xE1, 0x00, 0x01, 0x67, 0x00,
        ];
        let real = streaming_fixture("moov_end_4s.mp4");
        let moov = find_top_level_atom(&real, b"moov").expect("moov");
        let mut mdat = vec![0, 0, 0, 0, b'm', b'd', b'a', b't'];
        mdat.extend_from_slice(decoy);
        let mdat_len = mdat.len() as u32;
        mdat[..4].copy_from_slice(&mdat_len.to_be_bytes());
        let mut file = mdat;
        file.extend_from_slice(moov);
        let cfg = find_avcc_in_mp4(&file).expect("avcC");
        let real_cfg = find_avcc_in_mp4(moov).expect("avcC");
        assert_eq!(cfg.parameter_sets, real_cfg.parameter_sets);
        assert_eq!(cfg.nal_length_size, 4);
    }

    #[test]
    fn seek_with_hostile_times_does_not_panic() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let data = std::fs::read(manifest.join("../../tests/fixtures/test_320x240_2s.mp4"))
            .expect("fixture file missing");
        let mut demuxer = Mp4Demuxer::open(data).expect("open");
        for secs in [-3.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.25, 1e30] {
            let _ = demuxer.seek(secs);
            let _ = demuxer.next_packet();
        }
    }
}
