//! MP4 demuxer wrapping symphonia's `FormatReader`.
//!
//! Provides track discovery and per-packet routing so the caller can dispatch
//! H.264 video packets and AAC audio packets to their respective decoders.

use std::io::Cursor;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

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

/// Annex B start code (4-byte variant).
const ANNEX_B_START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// H.264 AVCC-to-Annex B conversion state parsed from the avcC box.
pub struct AvccConfig {
    /// Number of bytes in each NAL length prefix (1, 2, 3, or 4).
    nal_length_size: usize,
    /// SPS + PPS NAL units formatted as Annex B (with start codes).
    parameter_sets: Vec<u8>,
}

/// Parse the AVCDecoderConfigurationRecord (avcC box contents).
///
/// Returns the NAL length size and SPS/PPS parameter sets as Annex B bytestream.
fn parse_avcc(data: &[u8]) -> Result<AvccConfig, VideoError> {
    if data.len() < 7 {
        return Err(VideoError::Demux("avcC too short".into()));
    }

    let nal_length_size = (usize::from(data[4]) & 0x03) + 1;
    let num_sps = usize::from(data[5]) & 0x1F;

    let mut offset = 6;
    let mut parameter_sets = Vec::new();

    // Parse SPS entries.
    for _ in 0..num_sps {
        if offset + 2 > data.len() {
            return Err(VideoError::Demux("avcC SPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        if offset + len > data.len() {
            return Err(VideoError::Demux("avcC SPS data truncated".into()));
        }
        parameter_sets.extend_from_slice(&ANNEX_B_START_CODE);
        parameter_sets.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }

    // Parse PPS entries.
    if offset >= data.len() {
        return Err(VideoError::Demux("avcC PPS count missing".into()));
    }
    let num_pps = usize::from(data[offset]);
    offset += 1;

    for _ in 0..num_pps {
        if offset + 2 > data.len() {
            return Err(VideoError::Demux("avcC PPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        if offset + len > data.len() {
            return Err(VideoError::Demux("avcC PPS data truncated".into()));
        }
        parameter_sets.extend_from_slice(&ANNEX_B_START_CODE);
        parameter_sets.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }

    Ok(AvccConfig {
        nal_length_size,
        parameter_sets,
    })
}

/// Scan raw MP4 bytes for the `avcC` box and parse it.
///
/// Symphonia is audio-focused and doesn't extract video codec parameters,
/// so we find the avcC atom ourselves by scanning for the `avcC` fourcc.
/// Works on the full MP4 or just the moov atom data.
pub fn find_avcc_in_mp4(mp4_data: &[u8]) -> Option<AvccConfig> {
    // The avcC box is a child of the avc1 sample entry inside stsd.
    // Structure: [4-byte size][4-byte type='avcC'][AVCDecoderConfigurationRecord]
    // We scan for the 'avcC' fourcc and parse the box contents.
    let fourcc = b"avcC";
    let data = mp4_data;

    for i in 0..data.len().saturating_sub(8) {
        if &data[i + 4..i + 8] == fourcc {
            let box_size =
                u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            if box_size < 8 || i + box_size > data.len() {
                continue;
            }
            // Box contents start after the 8-byte header.
            let contents = &data[i + 8..i + box_size];
            if let Ok(config) = parse_avcc(contents) {
                return Some(config);
            }
        }
    }
    None
}

/// Convert an AVCC-formatted packet to Annex B by replacing length prefixes
/// with start codes.
fn avcc_to_annex_b(data: &[u8], nal_length_size: usize) -> Result<Vec<u8>, VideoError> {
    let mut out = Vec::with_capacity(data.len() + 32);
    avcc_to_annex_b_into(data, nal_length_size, &mut out)?;
    Ok(out)
}

/// Convert AVCC to Annex B, reusing the provided output buffer.
///
/// Clears `out` before writing.  The caller retains the buffer across calls
/// so its capacity grows to the largest packet and stays stable, eliminating
/// per-packet allocation.
fn avcc_to_annex_b_into(
    data: &[u8],
    nal_length_size: usize,
    out: &mut Vec<u8>,
) -> Result<(), VideoError> {
    out.clear();
    let mut offset = 0;

    while offset + nal_length_size <= data.len() {
        let nal_len = match nal_length_size {
            1 => usize::from(data[offset]),
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => {
                ((data[offset] as usize) << 16)
                    | ((data[offset + 1] as usize) << 8)
                    | (data[offset + 2] as usize)
            },
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => return Err(VideoError::Demux("invalid NAL length size".into())),
        };
        offset += nal_length_size;

        if offset
            .checked_add(nal_len)
            .is_none_or(|end| end > data.len())
        {
            return Err(VideoError::Demux("NAL unit exceeds packet bounds".into()));
        }

        out.extend_from_slice(&ANNEX_B_START_CODE);
        out.extend_from_slice(&data[offset..offset + nal_len]);
        offset += nal_len;
    }

    Ok(())
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
    /// Accepts any `Read + Seek + Send` source (file, cursor, etc.). Reads
    /// the full source to scan for the avcC box (required for AVCC→Annex B
    /// conversion), then seeks back to the start before handing off to
    /// symphonia.
    pub fn open_stream(mut source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        // Read the entire source to scan for the avcC box.
        let mut buf = Vec::new();
        source
            .read_to_end(&mut buf)
            .map_err(|e| VideoError::Demux(format!("read source: {e}")))?;
        let avcc = find_avcc_in_mp4(&buf);
        // Seek back to the start so symphonia can read from the beginning.
        source
            .seek(std::io::SeekFrom::Start(0))
            .map_err(|e| VideoError::Demux(format!("seek to start: {e}")))?;
        drop(buf);

        let adapter = VideoSourceAdapter(source);
        let mss = MediaSourceStream::new(Box::new(adapter), Default::default());
        Self::open_from_mss(mss, avcc)
    }

    /// Open an MP4 from a streaming source with pre-extracted avcC config.
    ///
    /// Skips the expensive `read_to_end()` scan — the caller has already
    /// extracted avcC from the moov atom (e.g. via a Range request).
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

            let data = if kind == TrackKind::Video {
                if let Some(avcc) = &self.avcc {
                    // Convert AVCC length-prefixed NALs to Annex B start codes.
                    let mut annex_b = avcc_to_annex_b(packet.buf(), avcc.nal_length_size)?;
                    // Prepend SPS/PPS before the first video packet.
                    if !self.sent_params {
                        let mut with_params =
                            Vec::with_capacity(avcc.parameter_sets.len() + annex_b.len());
                        with_params.extend_from_slice(&avcc.parameter_sets);
                        with_params.append(&mut annex_b);
                        annex_b = with_params;
                        self.sent_params = true;
                    }
                    annex_b
                } else {
                    packet.buf().to_vec()
                }
            } else {
                packet.buf().to_vec()
            };

            return Ok(Some(DemuxedPacket {
                kind,
                data,
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
}
