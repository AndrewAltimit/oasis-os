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

pub(crate) mod avcc;

mod atom;
mod sample_table;

use std::io::{Read, Seek, SeekFrom};

use atom::parse_boxes;
use sample_table::{
    SampleTable, find_keyframe_before, find_sample_at, is_keyframe, read_sample,
    sample_containing_dts, sample_dts_delta, sample_file_offset, sample_pts,
};

// Re-export public API items.
pub use avcc::avcc_to_annex_b;

/// Errors from the lightweight demuxer.
#[derive(Debug, thiserror::Error)]
pub enum LiteError {
    /// Reading or seeking the source failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not a well-formed MP4 (or exceeds a parser limit).
    #[error("parse: {0}")]
    Parse(String),
    /// The requested track is absent.
    #[error("no track: {0}")]
    NoTrack(String),
}

/// Track kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// An H.264 (`avc1`) video track.
    Video,
    /// An AAC (`mp4a`) audio track.
    Audio,
}

/// AVCC configuration extracted from the avcC box.
#[derive(Debug, Clone)]
pub struct AvccConfig {
    /// Bytes in each NAL unit's length prefix (1, 2 or 4).
    pub nal_length_size: usize,
    /// The first sequence parameter set (raw NAL, no start code).
    pub sps: Vec<u8>,
    /// The first picture parameter set (raw NAL, no start code).
    pub pps: Vec<u8>,
}

/// AAC configuration extracted from the esds box.
#[derive(Debug, Clone)]
pub struct AacConfig {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Raw AudioSpecificConfig bytes.
    pub config_data: Vec<u8>,
}

/// Information about a discovered track.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    /// Whether this is the video or the audio track.
    pub kind: TrackKind,
    pub(super) table: SampleTable,
    /// The avcC decoder configuration (video tracks).
    pub avcc: Option<AvccConfig>,
    /// The esds AudioSpecificConfig (audio tracks).
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

    /// Timescale (ticks per second) from this track's `mdhd`.
    pub fn timescale(&self) -> u32 {
        self.table.timescale
    }

    /// Index of the sample whose decode interval contains `secs`.
    ///
    /// Decode (not presentation) time, which is how sample-accurate
    /// demuxers such as symphonia's isomp4 resolve a seek.
    pub fn sample_at_decode_time(&self, secs: f64) -> usize {
        let ts = self.table.timescale;
        if ts == 0 || self.table.stsz.is_empty() {
            return 0;
        }
        let target = (secs.max(0.0) * ts as f64) as u64;
        sample_containing_dts(&self.table, target).min(self.table.stsz.len() - 1)
    }

    /// The sync sample at or before `idx` (decode order): the first sync
    /// sample if `idx` precedes all of them, `idx` itself when the track has
    /// no `stss` (every sample is a sync sample).
    pub fn keyframe_at_or_before(&self, idx: usize) -> usize {
        let stss = &self.table.stss;
        if stss.is_empty() {
            return idx;
        }
        let one_based = u32::try_from(idx.saturating_add(1)).unwrap_or(u32::MAX);
        match stss.binary_search(&one_based) {
            Ok(_) => idx,
            Err(0) => (stss[0] as usize).saturating_sub(1),
            Err(i) => (stss[i - 1] as usize).saturating_sub(1),
        }
    }

    /// A time (seconds) that seeks a sample-accurate demuxer exactly onto
    /// sample `idx`: the middle of its decode interval, so rounding in the
    /// demuxer's seconds-to-ticks conversion cannot land on a neighbour.
    pub fn sample_seek_time(&self, idx: usize) -> f64 {
        let ts = self.table.timescale;
        if ts == 0 {
            return 0.0;
        }
        let (dts, delta) = sample_dts_delta(&self.table, idx);
        (dts as f64 + delta as f64 / 2.0) / ts as f64
    }

    /// Every sync sample as `(decode_secs, seek_secs)`, ascending:
    /// `decode_secs` is where its decode interval starts, `seek_secs` the
    /// [`sample_seek_time`](Self::sample_seek_time) that lands on it.
    /// Empty when the track has no `stss` (every sample is a sync sample).
    pub fn keyframe_times(&self) -> Vec<(f64, f64)> {
        let ts = self.table.timescale;
        if ts == 0 {
            return Vec::new();
        }
        self.table
            .stss
            .iter()
            .map(|&one_based| {
                let idx = (one_based as usize).saturating_sub(1);
                let (dts, _) = sample_dts_delta(&self.table, idx);
                (dts as f64 / ts as f64, self.sample_seek_time(idx))
            })
            .collect()
    }

    /// Largest sample size in the track (bytes).
    ///
    /// Useful for pre-allocating buffers on memory-constrained platforms.
    pub fn max_sample_size(&self) -> u32 {
        self.table.stsz.max()
    }
}

/// A raw sample read from the file.
#[derive(Debug)]
pub struct RawSample {
    /// The sample bytes exactly as stored (AVCC length-prefixed NALs for
    /// video; see [`avcc_to_annex_b`]).
    pub data: Vec<u8>,
    /// Presentation time in seconds.
    pub timestamp_secs: f64,
    /// Whether the sample is a sync sample (`stss`).
    pub is_keyframe: bool,
    /// The track it came from.
    pub kind: TrackKind,
}

/// Build the lookup index of every parsed track (see
/// [`SampleTable::finalize`]).
fn finalize_tracks(video: &mut Option<TrackInfo>, audio: &mut Option<TrackInfo>) {
    for track in [video, audio].into_iter().flatten() {
        track.table.finalize();
    }
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
        finalize_tracks(&mut video_track, &mut audio_track);

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
    finalize_tracks(&mut video, &mut audio);
    Ok((video, audio))
}

/// Where a seek lands in an MP4 file, computed from its `moov` atom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoovSeekPoint {
    /// Time (seconds) to hand a sample-accurate demuxer so that the video
    /// track lands exactly on the keyframe at or before the requested time.
    /// Symphonia's isomp4 demuxer ignores `stss`, so seeking it to the raw
    /// request lands mid-GOP and the decoder has to discard everything up
    /// to the next keyframe (seconds of frozen video, audio running ahead).
    pub keyframe_secs: f64,
    /// Lowest file offset the demuxer reads after seeking to
    /// `keyframe_secs`: the minimum of the video keyframe's offset and the
    /// audio sample at that time (tracks are interleaved independently, so
    /// either may come first).  A streaming download must start at or
    /// before this byte or the first packet read falls outside the data.
    pub min_byte: u64,
}

/// Compute the [`MoovSeekPoint`] for `seek_secs` from raw moov bytes
/// (the complete atom including its 8-byte header).
///
/// Returns `None` if the moov has no usable video track.
pub fn seek_point_from_moov(moov_data: &[u8], seek_secs: f64) -> Option<MoovSeekPoint> {
    let (video, audio) = parse_moov_tracks(moov_data).ok()?;
    let video = video?;
    if video.sample_count() == 0 || video.timescale() == 0 {
        return None;
    }
    let target = video.sample_at_decode_time(seek_secs);
    let key = video.keyframe_at_or_before(target);
    let keyframe_secs = video.sample_seek_time(key);
    let (video_off, _) = video.sample_offset_size(key)?;

    let audio_off = audio
        .filter(|a| a.sample_count() > 0 && a.timescale() > 0)
        .and_then(|a| {
            let idx = a.sample_at_decode_time(keyframe_secs);
            a.sample_offset_size(idx).map(|(off, _)| off)
        });
    let min_byte = audio_off.map_or(video_off, |a| a.min(video_off));
    Some(MoovSeekPoint {
        keyframe_secs,
        min_byte,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
