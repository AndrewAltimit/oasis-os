//! Sample table structures and parsing (stts, stss, ctts, stsz, stco, stsc).

use std::io::{Read, Seek, SeekFrom};

use super::atom::{current_track_mut, read_u32_be, read_u64_be};
use super::avcc::avcc_to_annex_b;
use super::{LiteError, RawSample, TrackInfo, TrackKind};

/// Maximum number of entries allowed in any sample table (stts, ctts, stsc,
/// stco/co64, stsz, stss). Prevents OOM from malicious MP4 files that
/// declare a count of `0xFFFF_FFFF`.
pub(super) const MAX_TABLE_ENTRIES: u32 = 10_000_000;

/// A time-to-sample (stts) entry.
#[derive(Debug, Clone)]
pub(super) struct SttsEntry {
    pub(super) sample_count: u32,
    pub(super) sample_delta: u32,
}

/// A composition offset (ctts) entry.
#[derive(Debug, Clone)]
pub(super) struct CttsEntry {
    pub(super) sample_count: u32,
    pub(super) sample_offset: i32,
}

/// A sample-to-chunk (stsc) entry.
#[derive(Debug, Clone)]
pub(super) struct StscEntry {
    pub(super) first_chunk: u32,
    pub(super) samples_per_chunk: u32,
}

/// Per-track sample table.
#[derive(Debug, Clone, Default)]
pub(crate) struct SampleTable {
    pub(super) stsc: Vec<StscEntry>,
    pub(super) stsz: Vec<u32>,
    /// Chunk offsets (stco or co64).
    pub(super) stco: Vec<u64>,
    /// Sync sample indices (1-based).
    pub(super) stss: Vec<u32>,
    pub(super) stts: Vec<SttsEntry>,
    pub(super) ctts: Vec<CttsEntry>,
    pub(super) timescale: u32,
}

// ---------------------------------------------------------------------------
// Sample table box parsers
// ---------------------------------------------------------------------------

/// Parse stts (time-to-sample).
pub(super) fn parse_stts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("stts table too large".into()));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let sample_count = read_u32_be(r)?;
        let sample_delta = read_u32_be(r)?;
        entries.push(SttsEntry {
            sample_count,
            sample_delta,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stts = entries;
    }
    Ok(())
}

/// Parse ctts (composition time offsets).
pub(super) fn parse_ctts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("ctts table too large".into()));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let sample_count = read_u32_be(r)?;
        // ISO 14496-12: ctts version 1 uses signed offsets. The u32->i32
        // reinterpretation is intentional -- the bit pattern is preserved
        // and negative composition offsets (B-frames before their reference)
        // are represented correctly.
        let sample_offset = read_u32_be(r)? as i32;
        entries.push(CttsEntry {
            sample_count,
            sample_offset,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.ctts = entries;
    }
    Ok(())
}

/// Parse stsc (sample-to-chunk).
pub(super) fn parse_stsc<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("stsc table too large".into()));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let first_chunk = read_u32_be(r)?;
        let samples_per_chunk = read_u32_be(r)?;
        let _sample_desc_idx = read_u32_be(r)?;
        entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stsc = entries;
    }
    Ok(())
}

/// Parse stsz (sample sizes).
pub(super) fn parse_stsz<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let default_size = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("stsz table too large".into()));
    }
    let sizes = if default_size != 0 {
        vec![default_size; count as usize]
    } else {
        let mut v = Vec::with_capacity(count as usize);
        for _ in 0..count {
            v.push(read_u32_be(r)?);
        }
        v
    };
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stsz = sizes;
    }
    Ok(())
}

/// Parse stco (chunk offsets, 32-bit).
pub(super) fn parse_stco<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("stco table too large".into()));
    }
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offsets.push(read_u32_be(r)? as u64);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stco = offsets;
    }
    Ok(())
}

/// Parse co64 (chunk offsets, 64-bit).
pub(super) fn parse_co64<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("co64 table too large".into()));
    }
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offsets.push(read_u64_be(r)?);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stco = offsets;
    }
    Ok(())
}

/// Parse stss (sync sample table).
pub(super) fn parse_stss<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    if count > MAX_TABLE_ENTRIES {
        return Err(LiteError::Parse("stss table too large".into()));
    }
    let mut samples = Vec::with_capacity(count as usize);
    for _ in 0..count {
        samples.push(read_u32_be(r)?);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stss = samples;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sample reading and timestamp calculation
// ---------------------------------------------------------------------------

/// Locate a sample's file offset and size using the sample table.
pub(super) fn sample_file_offset(table: &SampleTable, sample_idx: usize) -> Option<(u64, u32)> {
    if sample_idx >= table.stsz.len() {
        return None;
    }
    let size = table.stsz[sample_idx];

    // Walk stsc to find which chunk this sample is in and the offset within it.
    let (chunk_idx, intra_offset) = sample_to_chunk(table, sample_idx)?;

    if chunk_idx >= table.stco.len() {
        return None;
    }
    let chunk_offset = table.stco[chunk_idx];

    // Sum sizes of preceding samples in this chunk.
    let mut offset = chunk_offset;
    let first_sample_in_chunk = intra_offset.0;
    for i in first_sample_in_chunk..sample_idx {
        if i < table.stsz.len() {
            offset += table.stsz[i] as u64;
        }
    }

    Some((offset, size))
}

/// Find the chunk index and first-sample-in-chunk for a given sample index.
fn sample_to_chunk(table: &SampleTable, sample_idx: usize) -> Option<(usize, (usize, u32))> {
    let mut sample_accum = 0usize;
    let num_chunks = table.stco.len();

    for (i, entry) in table.stsc.iter().enumerate() {
        let first_chunk = (entry.first_chunk as usize).saturating_sub(1); // 1-based -> 0-based
        let next_first = if i + 1 < table.stsc.len() {
            (table.stsc[i + 1].first_chunk as usize).saturating_sub(1)
        } else {
            num_chunks
        };

        for chunk in first_chunk..next_first {
            let samples_in_chunk = entry.samples_per_chunk as usize;
            if sample_idx < sample_accum.checked_add(samples_in_chunk)? {
                return Some((chunk, (sample_accum, entry.samples_per_chunk)));
            }
            sample_accum = sample_accum.checked_add(samples_in_chunk)?;
        }
    }
    None
}

/// Calculate the PTS in timescale units for a given sample index.
pub(super) fn sample_pts(table: &SampleTable, sample_idx: usize) -> u64 {
    // DTS from stts.
    let mut dts = 0u64;
    let mut sample = 0usize;
    for entry in &table.stts {
        let count = entry.sample_count as usize;
        if sample + count > sample_idx {
            dts = dts.saturating_add(
                ((sample_idx - sample) as u64).saturating_mul(entry.sample_delta as u64),
            );
            break;
        }
        dts = dts.saturating_add((count as u64).saturating_mul(entry.sample_delta as u64));
        sample += count;
    }

    // CTS offset from ctts.
    let cts_offset = if table.ctts.is_empty() {
        0i64
    } else {
        let mut s = 0usize;
        let mut offset = 0i64;
        for entry in &table.ctts {
            let count = entry.sample_count as usize;
            if s + count > sample_idx {
                offset = entry.sample_offset as i64;
                break;
            }
            s += count;
        }
        offset
    };

    (dts as i64 + cts_offset).max(0) as u64
}

/// Check if a sample is a keyframe.
pub(super) fn is_keyframe(table: &SampleTable, sample_idx: usize) -> bool {
    if table.stss.is_empty() {
        // No stss = all samples are sync samples.
        return true;
    }
    let one_based = (sample_idx + 1) as u32;
    table.stss.binary_search(&one_based).is_ok()
}

/// Read a sample from the file and apply AVCC->Annex B if needed.
pub(super) fn read_sample<R: Read + Seek>(
    r: &mut R,
    track: &TrackInfo,
    sample_idx: usize,
) -> Result<RawSample, LiteError> {
    let (offset, size) = sample_file_offset(&track.table, sample_idx)
        .ok_or_else(|| LiteError::Parse(format!("sample {sample_idx} not found in table")))?;

    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    let keyframe = is_keyframe(&track.table, sample_idx);

    // AVCC->Annex B for video.
    if track.kind == TrackKind::Video
        && let Some(avcc) = &track.avcc
    {
        data = avcc_to_annex_b(&data, avcc, keyframe)?;
    }

    let pts = sample_pts(&track.table, sample_idx);
    let ts = track.table.timescale;
    let timestamp_secs = if ts > 0 { pts as f64 / ts as f64 } else { 0.0 };

    Ok(RawSample {
        data,
        timestamp_secs,
        is_keyframe: keyframe,
        kind: track.kind,
    })
}

/// Find the keyframe at or before a target timestamp (in timescale units).
pub(super) fn find_keyframe_before(track: &TrackInfo, target_ts: u64) -> usize {
    if track.table.stss.is_empty() {
        // No stss -- find the sample nearest the target time.
        return find_sample_at(track, target_ts);
    }

    let mut best = 0usize;
    for &sync_sample in &track.table.stss {
        let idx = (sync_sample as usize).saturating_sub(1);
        let pts = sample_pts(&track.table, idx);
        if pts <= target_ts {
            best = idx;
        } else {
            break;
        }
    }
    best
}

/// Find the sample nearest a target timestamp (linear scan of stts).
pub(super) fn find_sample_at(track: &TrackInfo, target_ts: u64) -> usize {
    let mut dts = 0u64;
    let mut sample = 0usize;
    for entry in &track.table.stts {
        for _ in 0..entry.sample_count {
            if dts >= target_ts {
                return sample;
            }
            dts = dts.saturating_add(entry.sample_delta as u64);
            sample += 1;
        }
    }
    sample.saturating_sub(1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_pts_from_stts() {
        let table = SampleTable {
            stts: vec![
                SttsEntry {
                    sample_count: 3,
                    sample_delta: 1000,
                },
                SttsEntry {
                    sample_count: 2,
                    sample_delta: 2000,
                },
            ],
            timescale: 30000,
            ..Default::default()
        };
        assert_eq!(sample_pts(&table, 0), 0);
        assert_eq!(sample_pts(&table, 1), 1000);
        assert_eq!(sample_pts(&table, 2), 2000);
        assert_eq!(sample_pts(&table, 3), 3000 + 0); // first in 2nd run
        assert_eq!(sample_pts(&table, 4), 3000 + 2000);
    }

    #[test]
    fn sample_pts_with_ctts() {
        let table = SampleTable {
            stts: vec![SttsEntry {
                sample_count: 3,
                sample_delta: 1000,
            }],
            ctts: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 500,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: -200,
                },
            ],
            timescale: 1000,
            ..Default::default()
        };
        assert_eq!(sample_pts(&table, 0), 500); // DTS=0 + CTS=500
        assert_eq!(sample_pts(&table, 1), 800); // DTS=1000 + CTS=-200
    }

    #[test]
    fn keyframe_lookup_stss() {
        let table = SampleTable {
            stss: vec![1, 5, 10], // 1-based
            stsz: vec![100; 15],
            stts: vec![SttsEntry {
                sample_count: 15,
                sample_delta: 1000,
            }],
            timescale: 1000,
            ..Default::default()
        };
        assert!(is_keyframe(&table, 0)); // sample 1 (0-based=0)
        assert!(!is_keyframe(&table, 1));
        assert!(is_keyframe(&table, 4)); // sample 5 (0-based=4)
        assert!(is_keyframe(&table, 9)); // sample 10 (0-based=9)
    }

    #[test]
    fn empty_stts_table_pts_is_zero() {
        let table = SampleTable::default();
        assert_eq!(sample_pts(&table, 0), 0);
        assert_eq!(sample_pts(&table, 100), 0);
    }

    #[test]
    fn empty_stsz_table_sample_offset_returns_none() {
        let table = SampleTable::default();
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn empty_stsc_table_sample_to_chunk_returns_none() {
        let table = SampleTable {
            stsz: vec![100; 5],
            stco: vec![0],
            ..Default::default()
        };
        // stsc is empty, so sample_to_chunk returns None.
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn empty_stco_table_sample_offset_returns_none() {
        let table = SampleTable {
            stsz: vec![100; 5],
            stsc: vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 5,
            }],
            // stco is empty.
            ..Default::default()
        };
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn no_stss_means_all_keyframes() {
        let table = SampleTable {
            stsz: vec![100; 10],
            ..Default::default()
        };
        for i in 0..10 {
            assert!(
                is_keyframe(&table, i),
                "sample {i} should be keyframe with empty stss"
            );
        }
    }

    #[test]
    fn sample_pts_beyond_stts_range() {
        // Ask for sample_idx beyond the range covered by stts.
        let table = SampleTable {
            stts: vec![SttsEntry {
                sample_count: 2,
                sample_delta: 1000,
            }],
            timescale: 1000,
            ..Default::default()
        };
        // Sample 0 and 1 are within range. Sample 5 is beyond.
        // The function accumulates and breaks, returning accumulated DTS.
        let pts = sample_pts(&table, 5);
        // Should be 2*1000 = 2000 (the total from the single stts entry).
        assert_eq!(pts, 2000);
    }

    #[test]
    fn ctts_negative_offset_clamps_to_zero() {
        let table = SampleTable {
            stts: vec![SttsEntry {
                sample_count: 1,
                sample_delta: 100,
            }],
            ctts: vec![CttsEntry {
                sample_count: 1,
                sample_offset: -500, // larger than DTS=0
            }],
            timescale: 1000,
            ..Default::default()
        };
        // DTS=0 + CTS=-500 = -500, clamped to 0.
        assert_eq!(sample_pts(&table, 0), 0);
    }

    #[test]
    fn find_keyframe_before_no_stss() {
        let track = TrackInfo {
            kind: TrackKind::Video,
            table: SampleTable {
                stts: vec![SttsEntry {
                    sample_count: 10,
                    sample_delta: 1000,
                }],
                stsz: vec![100; 10],
                timescale: 1000,
                ..Default::default()
            },
            avcc: None,
            aac_config: None,
        };
        // With no stss, delegates to find_sample_at.
        let idx = find_keyframe_before(&track, 5000);
        assert_eq!(idx, 5);
    }

    #[test]
    fn find_sample_at_empty_stts() {
        let track = TrackInfo {
            kind: TrackKind::Audio,
            table: SampleTable::default(),
            avcc: None,
            aac_config: None,
        };
        // Empty stts -> loop doesn't execute, returns 0.saturating_sub(1) = 0.
        let idx = find_sample_at(&track, 5000);
        assert_eq!(idx, 0);
    }

    // ---------------------------------------------------------------
    // Property-based tests (proptest)
    // ---------------------------------------------------------------

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn sample_pts_no_panic(
                counts in proptest::collection::vec(1u32..10000, 1..20),
                deltas in proptest::collection::vec(1u32..100000, 1..20),
                sample_idx in 0usize..50000,
            ) {
                let len = counts.len().min(deltas.len());
                let stts: Vec<SttsEntry> = counts[..len].iter()
                    .zip(deltas[..len].iter())
                    .map(|(&c, &d)| SttsEntry {
                        sample_count: c,
                        sample_delta: d,
                    })
                    .collect();
                let table = SampleTable {
                    stts,
                    timescale: 90000,
                    ..Default::default()
                };
                let _ = sample_pts(&table, sample_idx);
            }

            #[test]
            fn find_sample_at_no_panic(
                counts in proptest::collection::vec(1u32..10000, 1..10),
                deltas in proptest::collection::vec(1u32..100000, 1..10),
                target_ts in any::<u64>(),
            ) {
                let len = counts.len().min(deltas.len());
                let stts: Vec<SttsEntry> = counts[..len].iter()
                    .zip(deltas[..len].iter())
                    .map(|(&c, &d)| SttsEntry {
                        sample_count: c,
                        sample_delta: d,
                    })
                    .collect();
                let table = SampleTable {
                    stts,
                    timescale: 90000,
                    ..Default::default()
                };
                let track = TrackInfo {
                    kind: TrackKind::Audio,
                    table,
                    avcc: None,
                    aac_config: None,
                };
                let _ = find_sample_at(&track, target_ts);
            }
        }
    }
}
