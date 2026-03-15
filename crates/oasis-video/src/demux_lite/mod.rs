//! Lightweight MP4 demuxer for `no_std`-friendly environments (e.g. PSP).
//!
//! Parses ISO BMFF boxes manually using only `std::io::{Read, Seek}`,
//! avoiding symphonia's `lazy_static`/`sync::Once` which panics in PPSSPP HLE.
//!
//! Supports:
//! - H.264 video (avc1) and AAC audio (mp4a) tracks
//! - AVCC->Annex B NAL conversion
//! - Sample-accurate seeking via stss (sync sample table)
//! - stts/ctts-based PTS calculation

pub mod avcc;

mod atom;
mod sample_table;

use std::io::{Read, Seek, SeekFrom};

use atom::parse_boxes;
use sample_table::{
    SampleTable, find_keyframe_before, find_sample_at, is_keyframe, read_sample,
    sample_file_offset, sample_pts,
};

// Re-export public API items.
pub use avcc::avcc_to_annex_b;

/// Errors from the lightweight demuxer.
#[derive(Debug, thiserror::Error)]
pub enum LiteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("no track: {0}")]
    NoTrack(String),
}

/// Track kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// AVCC configuration extracted from the avcC box.
#[derive(Debug, Clone)]
pub struct AvccConfig {
    pub nal_length_size: usize,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

/// AAC configuration extracted from the esds box.
#[derive(Debug, Clone)]
pub struct AacConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Raw AudioSpecificConfig bytes.
    pub config_data: Vec<u8>,
}

/// Information about a discovered track.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub kind: TrackKind,
    pub(super) table: SampleTable,
    pub avcc: Option<AvccConfig>,
    pub aac_config: Option<AacConfig>,
}

impl TrackInfo {
    /// Number of samples in this track.
    pub fn sample_count(&self) -> usize {
        self.table.stsz.len()
    }

    /// Get the absolute file offset and size (bytes) for a sample.
    pub fn sample_offset_size(&self, idx: usize) -> Option<(u64, u32)> {
        sample_file_offset(&self.table, idx)
    }

    /// Presentation timestamp in seconds for a sample.
    pub fn sample_timestamp(&self, idx: usize) -> f64 {
        let pts = sample_pts(&self.table, idx);
        let ts = self.table.timescale;
        if ts > 0 { pts as f64 / ts as f64 } else { 0.0 }
    }

    /// Whether a sample is a sync (key) frame.
    pub fn sample_is_keyframe(&self, idx: usize) -> bool {
        is_keyframe(&self.table, idx)
    }
}

/// A raw sample read from the file.
#[derive(Debug)]
pub struct RawSample {
    pub data: Vec<u8>,
    pub timestamp_secs: f64,
    pub is_keyframe: bool,
    pub kind: TrackKind,
}

/// Lightweight MP4 parser.
pub struct Mp4Lite<R: Read + Seek> {
    reader: R,
    video_track: Option<TrackInfo>,
    audio_track: Option<TrackInfo>,
    video_sample_index: usize,
    audio_sample_index: usize,
}

impl<R: Read + Seek> Mp4Lite<R> {
    /// Open and parse an MP4 container.
    pub fn open(mut reader: R) -> Result<Self, LiteError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut video_track = None;
        let mut audio_track = None;

        // Parse top-level boxes.
        parse_boxes(
            &mut reader,
            0,
            file_size,
            &mut video_track,
            &mut audio_track,
        )?;

        Ok(Self {
            reader,
            video_track,
            audio_track,
            video_sample_index: 0,
            audio_sample_index: 0,
        })
    }

    /// Read the next video sample (with AVCC->Annex B conversion).
    pub fn next_video_sample(&mut self) -> Result<Option<RawSample>, LiteError> {
        let track = match &self.video_track {
            Some(t) => t,
            None => return Err(LiteError::NoTrack("no video track".into())),
        };
        let idx = self.video_sample_index;
        if idx >= track.table.stsz.len() {
            return Ok(None);
        }
        let sample = read_sample(&mut self.reader, track, idx)?;
        self.video_sample_index += 1;
        Ok(Some(sample))
    }

    /// Read the next audio sample.
    pub fn next_audio_sample(&mut self) -> Result<Option<RawSample>, LiteError> {
        let track = match &self.audio_track {
            Some(t) => t,
            None => return Err(LiteError::NoTrack("no audio track".into())),
        };
        let idx = self.audio_sample_index;
        if idx >= track.table.stsz.len() {
            return Ok(None);
        }
        let sample = read_sample(&mut self.reader, track, idx)?;
        self.audio_sample_index += 1;
        Ok(Some(sample))
    }

    /// Seek to the nearest keyframe at or before `secs`.
    pub fn seek(&mut self, secs: f64) -> Result<(), LiteError> {
        // Seek video track.
        if let Some(track) = &self.video_track {
            let target_ts = (secs * track.table.timescale as f64) as u64;
            let sample_idx = find_keyframe_before(track, target_ts);
            self.video_sample_index = sample_idx;
        }
        // Seek audio track to the nearest sample.
        if let Some(track) = &self.audio_track {
            let target_ts = (secs * track.table.timescale as f64) as u64;
            let sample_idx = find_sample_at(track, target_ts);
            self.audio_sample_index = sample_idx;
        }
        Ok(())
    }

    /// Video track info, if present.
    pub fn video_track_info(&self) -> Option<&TrackInfo> {
        self.video_track.as_ref()
    }

    /// Audio track info, if present.
    pub fn audio_track_info(&self) -> Option<&TrackInfo> {
        self.audio_track.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Public moov-parsing API for streaming seek estimation
// ---------------------------------------------------------------------------

/// Parse raw moov atom bytes and return track information.
///
/// The input should be the complete moov atom including the 8-byte header
/// (size + `moov` fourcc).  Returns `(video_track, audio_track)`.
///
/// This is useful for streaming players that have pre-fetched the moov atom
/// and need to compute exact sample byte offsets for seeking without having
/// the full file available.
pub fn parse_moov_tracks(
    moov_data: &[u8],
) -> Result<(Option<TrackInfo>, Option<TrackInfo>), LiteError> {
    if moov_data.len() < 8 {
        return Err(LiteError::Parse("moov too short".into()));
    }
    let mut cursor = std::io::Cursor::new(moov_data);
    let moov_len = moov_data.len() as u64;

    // Skip the moov atom header (8 bytes: size + fourcc).
    // parse_boxes will recursively parse trak->mdia->minf->stbl children.
    let mut video = None;
    let mut audio = None;
    parse_boxes(&mut cursor, 8, moov_len, &mut video, &mut audio)?;
    Ok((video, audio))
}

/// Find the byte offset of the keyframe nearest to `seek_secs` using
/// sample tables parsed from moov data.
///
/// Returns `Some(byte_offset)` of the keyframe's file position, or `None`
/// if the moov doesn't contain enough information.
pub fn seek_byte_from_moov(moov_data: &[u8], seek_secs: f64) -> Option<u64> {
    let (video, _audio) = parse_moov_tracks(moov_data).ok()?;
    let track = video?;
    let count = track.sample_count();
    if count == 0 {
        return None;
    }

    // Find sample nearest to seek_secs via binary search on timestamp.
    let target_sample = {
        let mut lo = 0usize;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let ts = track.sample_timestamp(mid);
            if ts < seek_secs {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 { lo - 1 } else { 0 }
    };

    // Find the nearest keyframe at or before target_sample.
    let keyframe_sample = if track.table.stss.is_empty() {
        // All frames are keyframes.
        target_sample
    } else {
        // stss is 1-based and sorted. Find largest entry <= target_sample+1.
        let one_based = (target_sample + 1) as u32;
        match track.table.stss.binary_search(&one_based) {
            Ok(_) => target_sample,
            Err(0) => {
                // Before first keyframe -- use first keyframe.
                (track.table.stss[0] as usize).saturating_sub(1)
            },
            Err(i) => {
                // stss[i-1] is the largest keyframe <= target.
                (track.table.stss[i - 1] as usize).saturating_sub(1)
            },
        }
    };

    let (offset, _size) = track.sample_offset_size(keyframe_sample)?;
    Some(offset)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/test_320x240_2s.mp4")
    }

    fn fixture_bytes() -> Vec<u8> {
        std::fs::read(fixture_path()).expect("fixture file missing")
    }

    #[test]
    fn open_fixture_finds_tracks() {
        let data = fixture_bytes();
        let mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        assert!(mp4.video_track_info().is_some(), "should find video track");
        assert!(mp4.audio_track_info().is_some(), "should find audio track");

        let vt = mp4.video_track_info().unwrap();
        assert!(vt.avcc.is_some(), "video track should have avcC");
        assert!(vt.table.timescale > 0);
        assert!(!vt.table.stsz.is_empty());

        let at = mp4.audio_track_info().unwrap();
        assert!(
            at.aac_config.is_some(),
            "audio track should have AAC config"
        );
    }

    #[test]
    fn read_all_video_samples() {
        let data = fixture_bytes();
        let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        let mut count = 0;
        let mut last_ts = -1.0f64;
        while let Some(sample) = mp4.next_video_sample().expect("read error") {
            count += 1;
            assert_eq!(sample.kind, TrackKind::Video);
            assert!(!sample.data.is_empty());
            assert!(sample.timestamp_secs >= last_ts, "timestamp went backwards");
            last_ts = sample.timestamp_secs;
        }
        // The fixture is ~2s at ~15fps, so expect at least 10 samples.
        assert!(count >= 10, "expected >=10 video samples, got {count}");
    }

    #[test]
    fn seek_to_midstream() {
        let data = fixture_bytes();
        let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        mp4.seek(1.0).expect("seek failed");
        let sample = mp4
            .next_video_sample()
            .expect("read error")
            .expect("no sample");
        // Should be near 1.0s (within 0.5s tolerance).
        assert!(
            (sample.timestamp_secs - 1.0).abs() < 0.5,
            "post-seek timestamp {} not near 1.0s",
            sample.timestamp_secs
        );
    }

    #[test]
    fn parse_moov_tracks_too_short() {
        let result = parse_moov_tracks(&[0; 4]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_moov_tracks_empty_moov() {
        // 8-byte moov header, no children.
        let mut data = Vec::new();
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        let (video, audio) = parse_moov_tracks(&data).unwrap();
        assert!(video.is_none());
        assert!(audio.is_none());
    }

    #[test]
    fn seek_byte_from_moov_empty_returns_none() {
        let mut data = Vec::new();
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        assert!(seek_byte_from_moov(&data, 5.0).is_none());
    }

    #[test]
    fn truncated_moov_atom() {
        // A moov atom whose declared size extends beyond the available data.
        // The moov header says 200 bytes total, but we only provide 20.
        let mut data = Vec::new();
        // ftyp box (valid, 24 bytes)
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 16]);
        // moov box header claiming 200 bytes, but content is truncated
        data.extend_from_slice(&200u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 4]); // only 4 bytes of "content"

        // Should not panic. parse_boxes skips boxes whose end exceeds
        // the parent boundary, so moov is skipped and no tracks found.
        let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
        assert!(mp4.video_track_info().is_none());
        assert!(mp4.audio_track_info().is_none());
    }

    #[test]
    fn truncated_moov_atom_via_parse_moov_tracks() {
        // A moov header that claims 200 bytes but only 20 bytes are present.
        // parse_moov_tracks should handle gracefully (no panic).
        let mut data = Vec::new();
        data.extend_from_slice(&200u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 12]); // truncated content
        let result = parse_moov_tracks(&data);
        // Should succeed but find no tracks (children are truncated/skipped).
        match result {
            Ok((v, a)) => {
                assert!(v.is_none());
                assert!(a.is_none());
            },
            Err(_) => {
                // An error is also acceptable -- just must not panic.
            },
        }
    }

    #[test]
    fn zero_size_atom_parsing() {
        // An atom with size=0 means "extends to end of file".
        // Build: ftyp(24 bytes) + zero-size mdat.
        let mut data = Vec::new();
        // ftyp
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 16]);
        // mdat with size=0 (extends to EOF)
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0xAA; 50]); // payload

        // Should parse without panic. No moov, so no tracks.
        let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
        assert!(mp4.video_track_info().is_none());
        assert!(mp4.audio_track_info().is_none());
    }

    #[test]
    fn empty_input_mp4lite() {
        // Parsing an empty byte slice should return Ok with no tracks,
        // not panic.
        let mp4 = Mp4Lite::open(Cursor::new(Vec::<u8>::new())).unwrap();
        assert!(mp4.video_track_info().is_none());
        assert!(mp4.audio_track_info().is_none());
    }

    #[test]
    fn ftyp_only_no_moov() {
        // Just an ftyp atom with no moov. Should return no track info.
        let ftyp_content = b"isom\x00\x00\x00\x00isomavc1";
        let mut data = Vec::new();
        let size = (8 + ftyp_content.len()) as u32;
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(ftyp_content);

        let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
        assert!(
            mp4.video_track_info().is_none(),
            "ftyp-only file should have no video track"
        );
        assert!(
            mp4.audio_track_info().is_none(),
            "ftyp-only file should have no audio track"
        );
    }

    // ---------------------------------------------------------------
    // AVCC conversion tests (delegated to avcc module but tested here
    // for backward compatibility with the original test locations)
    // ---------------------------------------------------------------

    #[test]
    fn avcc_to_annex_b_empty_data() {
        let avcc = AvccConfig {
            nal_length_size: 4,
            sps: vec![0x67, 0x42],
            pps: vec![0x68, 0xCE],
        };
        // Empty data, keyframe -> should produce SPS + PPS only.
        let result = avcc_to_annex_b(&[], &avcc, true).unwrap();
        // [0,0,0,1, 0x67,0x42, 0,0,0,1, 0x68,0xCE]
        assert_eq!(result.len(), 12);
        assert_eq!(&result[0..4], &[0, 0, 0, 1]);
        assert_eq!(&result[4..6], &[0x67, 0x42]);
        assert_eq!(&result[6..10], &[0, 0, 0, 1]);
        assert_eq!(&result[10..12], &[0x68, 0xCE]);
    }

    #[test]
    fn avcc_to_annex_b_nal_exceeds_bounds() {
        let avcc = AvccConfig {
            nal_length_size: 4,
            sps: Vec::new(),
            pps: Vec::new(),
        };
        // NAL length says 1000 bytes but only 4 bytes of data follow.
        let mut data = Vec::new();
        data.extend_from_slice(&1000u32.to_be_bytes());
        data.extend_from_slice(&[0xAA; 4]);
        let result = avcc_to_annex_b(&data, &avcc, false);
        assert!(result.is_err());
    }

    #[test]
    fn avcc_to_annex_b_invalid_nal_length_size() {
        let avcc = AvccConfig {
            nal_length_size: 5, // invalid
            sps: Vec::new(),
            pps: Vec::new(),
        };
        let data = vec![0; 8];
        let result = avcc_to_annex_b(&data, &avcc, false);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Property-based tests (proptest)
    // ---------------------------------------------------------------

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn avcc_to_annex_b_no_panic(
                data in proptest::collection::vec(any::<u8>(), 0..4096),
                nls in 1usize..=4,
            ) {
                let avcc = AvccConfig {
                    nal_length_size: nls,
                    sps: vec![],
                    pps: vec![],
                };
                let _ = avcc_to_annex_b(&data, &avcc, true);
                let _ = avcc_to_annex_b(&data, &avcc, false);
            }
        }
    }

    // ---------------------------------------------------------------
    // Item 71: Demux_lite error path tests (truncated/missing atoms)
    // ---------------------------------------------------------------

    #[test]
    fn mp4lite_garbage_data_no_panic() {
        let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
        let mp4 = Mp4Lite::open(Cursor::new(garbage)).unwrap();
        assert!(mp4.video_track_info().is_none());
        assert!(mp4.audio_track_info().is_none());
    }

    #[test]
    fn mp4lite_single_byte_no_panic() {
        let mp4 = Mp4Lite::open(Cursor::new(vec![0xFF])).unwrap();
        assert!(mp4.video_track_info().is_none());
    }

    #[test]
    fn mp4lite_next_video_without_track_errors() {
        let mp4_data = vec![0u8; 0];
        let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
        let result = mp4.next_video_sample();
        assert!(result.is_err());
    }

    #[test]
    fn mp4lite_next_audio_without_track_errors() {
        let mp4_data = vec![0u8; 0];
        let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
        let result = mp4.next_audio_sample();
        assert!(result.is_err());
    }

    #[test]
    fn mp4lite_seek_no_tracks_ok() {
        let mp4_data = vec![0u8; 0];
        let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
        // Seeking with no tracks should succeed (no-op).
        mp4.seek(5.0).unwrap();
    }

    #[test]
    fn parse_moov_tracks_garbage_content() {
        // moov header + random bytes inside.
        let mut data = Vec::new();
        data.extend_from_slice(&64u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0xDE; 56]);
        let (v, a) = parse_moov_tracks(&data).unwrap();
        assert!(v.is_none());
        assert!(a.is_none());
    }

    #[test]
    fn seek_byte_from_moov_garbage_returns_none() {
        let mut data = Vec::new();
        data.extend_from_slice(&64u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0xFF; 56]);
        assert!(seek_byte_from_moov(&data, 1.0).is_none());
    }

    #[test]
    fn mp4lite_truncated_mdat_header_only() {
        // mdat atom with valid header but size extends past EOF.
        let mut data = Vec::new();
        data.extend_from_slice(&1000u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0xAA; 8]); // only 8 bytes, not 992
        let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
        assert!(mp4.video_track_info().is_none());
    }

    #[test]
    fn mp4lite_multiple_unknown_atoms_skipped() {
        // Several unknown atom types should be silently skipped.
        let mut data = Vec::new();
        for fourcc in [b"free", b"skip", b"udta", b"meta"] {
            data.extend_from_slice(&16u32.to_be_bytes());
            data.extend_from_slice(fourcc);
            data.extend_from_slice(&[0; 8]);
        }
        let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
        assert!(mp4.video_track_info().is_none());
        assert!(mp4.audio_track_info().is_none());
    }
}
