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

use crate::VideoError;

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
struct AvccConfig {
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
fn find_avcc_in_mp4(mp4_data: &[u8]) -> Option<AvccConfig> {
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

        if offset + nal_len > data.len() {
            return Err(VideoError::Demux("NAL unit exceeds packet bounds".into()));
        }

        out.extend_from_slice(&ANNEX_B_START_CODE);
        out.extend_from_slice(&data[offset..offset + nal_len]);
        offset += nal_len;
    }

    Ok(out)
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
    /// Open an MP4 from a byte buffer.
    pub fn open(data: Vec<u8>) -> Result<Self, VideoError> {
        // Scan for avcC box before symphonia consumes the data, since
        // symphonia doesn't extract video codec parameters.
        let avcc = find_avcc_in_mp4(&data);

        let cursor = Cursor::new(data);
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

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
}
