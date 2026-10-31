//! Parsers for the sample table boxes (`stts`, `ctts`, `stsc`, `stsz`,
//! `stco` / `co64`, `stss`): each reads one box's entries into the
//! [`SampleTable`] of the track being parsed, bounded by
//! [`MAX_TABLE_ENTRIES`].

use std::io::{Read, Seek, SeekFrom};

use super::super::atom::{current_track_mut, read_u32_be, read_u64_be};
use super::super::{LiteError, TrackInfo};
use super::*;

/// Read a full box's `version_flags` + `entry_count` header and check that
/// `count` entries of `entry_bytes` each fit in the box's `size` content
/// bytes, so the table allocation is bounded by bytes actually in the file.
fn read_table_header<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    extra_header: u64,
    entry_bytes: u64,
    name: &str,
) -> Result<u32, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    for _ in 0..extra_header / 4 {
        read_u32_be(r)?;
    }
    let count = read_u32_be(r)?;
    let fits = u64::from(count)
        .checked_mul(entry_bytes)
        .and_then(|b| b.checked_add(8 + extra_header))
        .is_some_and(|b| b <= size);
    if count > MAX_TABLE_ENTRIES || !fits {
        return Err(LiteError::Parse(format!(
            "{name} declares {count} entries, more than the box holds"
        )));
    }
    Ok(count)
}

/// Parse stts (time-to-sample).
pub(in crate::demux_lite) fn parse_stts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 8, "stts")?;
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
pub(in crate::demux_lite) fn parse_ctts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 8, "ctts")?;
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
pub(in crate::demux_lite) fn parse_stsc<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 12, "stsc")?;
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
pub(in crate::demux_lite) fn parse_stsz<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
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
        SampleSizes::Uniform {
            size: default_size,
            count,
        }
    } else {
        if u64::from(count) * 4 + 12 > size {
            return Err(LiteError::Parse(format!(
                "stsz declares {count} entries, more than the box holds"
            )));
        }
        let mut v = Vec::with_capacity(count as usize);
        for _ in 0..count {
            v.push(read_u32_be(r)?);
        }
        SampleSizes::PerSample(v)
    };
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stsz = sizes;
    }
    Ok(())
}

/// Parse stco (chunk offsets, 32-bit).
pub(in crate::demux_lite) fn parse_stco<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 4, "stco")?;
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offsets.push(u64::from(read_u32_be(r)?));
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stco = offsets;
    }
    Ok(())
}

/// Parse co64 (chunk offsets, 64-bit).
pub(in crate::demux_lite) fn parse_co64<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 8, "co64")?;
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
pub(in crate::demux_lite) fn parse_stss<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    let count = read_table_header(r, offset, size, 0, 4, "stss")?;
    let mut samples = Vec::with_capacity(count as usize);
    for _ in 0..count {
        samples.push(read_u32_be(r)?);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stss = samples;
    }
    Ok(())
}
