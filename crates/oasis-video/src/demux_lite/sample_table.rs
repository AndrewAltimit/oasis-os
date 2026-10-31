//! Sample table structures and parsing (stts, stss, ctts, stsz, stco, stsc).
//!
//! The raw tables are parsed as-is, then [`SampleTable::finalize`] builds a
//! small run index (one entry per table run, not per sample) so every
//! per-sample lookup is a binary search instead of a walk over the tables.
//! Players call those lookups once per sample, so the old linear walks made
//! sequential playback quadratic in the track length (painful on the PSP's
//! 333 MHz CPU with a long `ctts`), and a hostile `stsc`/`stts` could make a
//! single lookup spin for billions of iterations.

use std::io::{Read, Seek, SeekFrom};

use super::atom::{current_track_mut, read_u32_be, read_u64_be};
use super::avcc::avcc_to_annex_b;
use super::{LiteError, RawSample, TrackInfo, TrackKind};

/// Maximum number of entries allowed in any sample table (stts, ctts, stsc,
/// stco/co64, stsz, stss). Prevents OOM from malicious MP4 files that
/// declare a count of `0xFFFF_FFFF`.
pub(super) const MAX_TABLE_ENTRIES: u32 = 10_000_000;

/// Largest single sample [`read_sample`] will allocate for (32 MiB).  A
/// 4K H.264 keyframe is a few MiB; anything near the `u32` limit of `stsz`
/// is a hostile file asking for a 4 GiB buffer.
pub(super) const MAX_SAMPLE_BYTES: u32 = 32 * 1024 * 1024;

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

/// Sample sizes from `stsz`.
///
/// A constant-size table (audio, mostly) is stored as its size and count:
/// the box is 20 bytes however many samples it declares, so expanding it
/// to one `u32` per sample would let a 20-byte box allocate 40 MB.
#[derive(Debug, Clone)]
pub(super) enum SampleSizes {
    /// Every sample has the same size.
    Uniform { size: u32, count: u32 },
    /// One size per sample.
    PerSample(Vec<u32>),
}

impl Default for SampleSizes {
    fn default() -> Self {
        Self::PerSample(Vec::new())
    }
}

impl From<Vec<u32>> for SampleSizes {
    fn from(sizes: Vec<u32>) -> Self {
        Self::PerSample(sizes)
    }
}

impl SampleSizes {
    /// Number of samples.
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Uniform { count, .. } => *count as usize,
            Self::PerSample(v) => v.len(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Size of sample `idx`, if it exists.
    pub(super) fn get(&self, idx: usize) -> Option<u32> {
        match self {
            Self::Uniform { size, count } => (idx < *count as usize).then_some(*size),
            Self::PerSample(v) => v.get(idx).copied(),
        }
    }

    /// Largest sample size (0 for an empty table).
    pub(super) fn max(&self) -> u32 {
        match self {
            Self::Uniform { size, count } => {
                if *count > 0 {
                    *size
                } else {
                    0
                }
            },
            Self::PerSample(v) => v.iter().copied().max().unwrap_or(0),
        }
    }

    /// Total bytes of samples `start..end` (clamped to the table), by a
    /// linear walk: the reference [`SampleTable::size_sum`] is checked
    /// against.
    #[cfg(test)]
    fn sum(&self, start: usize, end: usize) -> u64 {
        let end = end.min(self.len());
        let start = start.min(end);
        match self {
            Self::Uniform { size, .. } => u64::from(*size).saturating_mul((end - start) as u64),
            Self::PerSample(v) => v[start..end].iter().map(|&s| u64::from(s)).sum(),
        }
    }
}

/// One `stsc` run resolved against `stco`: chunks `first_chunk..end_chunk`
/// (0-based) each hold `samples_per_chunk` samples, the first of which is
/// sample `first_sample`.
#[derive(Debug, Clone)]
struct ChunkRun {
    first_chunk: u64,
    end_chunk: u64,
    first_sample: u64,
    samples_per_chunk: u64,
}

/// Run index built by [`SampleTable::finalize`].
#[derive(Debug, Clone, Default)]
struct TableIndex {
    /// `(first_sample, first_dts)` of each `stts` entry.
    stts: Vec<(u64, u64)>,
    /// Samples and ticks covered by the whole `stts` table.
    stts_total: (u64, u64),
    /// First sample of each `ctts` entry.
    ctts: Vec<u64>,
    /// Non-empty `stsc` runs, ascending by `first_sample`.
    chunks: Vec<ChunkRun>,
    /// For a per-sample `stsz`: the total size of samples
    /// `0..k * SIZE_BLOCK`, for `k` in `0..=len / SIZE_BLOCK`.
    size_blocks: Vec<u64>,
}

/// Samples per [`TableIndex::size_blocks`] entry: a size prefix sum is
/// one block lookup plus at most this many adds, for 8 bytes of index per
/// `SIZE_BLOCK` samples (1/32 of the `stsz` table itself).
const SIZE_BLOCK: usize = 64;

/// Per-track sample table.
#[derive(Debug, Clone, Default)]
pub(crate) struct SampleTable {
    pub(super) stsc: Vec<StscEntry>,
    pub(super) stsz: SampleSizes,
    /// Chunk offsets (stco or co64).
    pub(super) stco: Vec<u64>,
    /// Sync sample indices (1-based, ascending after `finalize`).
    pub(super) stss: Vec<u32>,
    pub(super) stts: Vec<SttsEntry>,
    pub(super) ctts: Vec<CttsEntry>,
    pub(super) timescale: u32,
    index: TableIndex,
}

impl SampleTable {
    /// Build the run index the lookups below rely on.  Called once the
    /// track's boxes are all parsed (`Mp4Lite::open`, `parse_moov_tracks`);
    /// a table that is not finalized answers every lookup as if empty.
    pub(super) fn finalize(&mut self) {
        // `is_keyframe` and the keyframe searches binary-search `stss`.
        if !self.stss.is_sorted() {
            self.stss.sort_unstable();
        }
        self.stss.dedup();

        let (mut sample, mut dts) = (0u64, 0u64);
        self.index.stts = self
            .stts
            .iter()
            .map(|e| {
                let start = (sample, dts);
                sample = sample.saturating_add(u64::from(e.sample_count));
                dts = dts.saturating_add(
                    u64::from(e.sample_count).saturating_mul(u64::from(e.sample_delta)),
                );
                start
            })
            .collect();
        self.index.stts_total = (sample, dts);

        let mut sample = 0u64;
        self.index.ctts = self
            .ctts
            .iter()
            .map(|e| {
                let start = sample;
                sample = sample.saturating_add(u64::from(e.sample_count));
                start
            })
            .collect();

        self.index.size_blocks.clear();
        if let SampleSizes::PerSample(v) = &self.stsz {
            let mut total = 0u64;
            self.index.size_blocks.reserve(v.len() / SIZE_BLOCK + 1);
            self.index.size_blocks.push(0);
            for block in v.chunks(SIZE_BLOCK) {
                total += block.iter().map(|&s| u64::from(s)).sum::<u64>();
                if block.len() == SIZE_BLOCK {
                    self.index.size_blocks.push(total);
                }
            }
        }

        let num_chunks = self.stco.len() as u64;
        let mut sample = 0u64;
        self.index.chunks.clear();
        for (i, entry) in self.stsc.iter().enumerate() {
            // 1-based -> 0-based, clamped to the chunks that exist.
            let first = u64::from(entry.first_chunk)
                .saturating_sub(1)
                .min(num_chunks);
            let end = self.stsc.get(i + 1).map_or(num_chunks, |next| {
                u64::from(next.first_chunk)
                    .saturating_sub(1)
                    .min(num_chunks)
            });
            if end <= first || entry.samples_per_chunk == 0 {
                continue;
            }
            let spc = u64::from(entry.samples_per_chunk);
            self.index.chunks.push(ChunkRun {
                first_chunk: first,
                end_chunk: end,
                first_sample: sample,
                samples_per_chunk: spc,
            });
            sample = sample.saturating_add((end - first).saturating_mul(spc));
        }
    }

    /// Total size of samples `0..i` (clamped to the table).
    fn size_prefix(&self, i: usize) -> u64 {
        let i = i.min(self.stsz.len());
        match &self.stsz {
            SampleSizes::Uniform { size, .. } => u64::from(*size).saturating_mul(i as u64),
            SampleSizes::PerSample(v) => {
                // Unfinalized tables have no index: walk from the start.
                let block = (i / SIZE_BLOCK).min(self.index.size_blocks.len().saturating_sub(1));
                let base = self.index.size_blocks.get(block).copied().unwrap_or(0);
                let from = if self.index.size_blocks.is_empty() {
                    0
                } else {
                    block * SIZE_BLOCK
                };
                base + v[from..i].iter().map(|&s| u64::from(s)).sum::<u64>()
            },
        }
    }

    /// Total size of samples `start..end` (clamped to the table) in
    /// O(`SIZE_BLOCK`). Per-sample offset lookups used to sum every
    /// preceding sample of the chunk, so a hostile file with one chunk of
    /// millions of samples made sequential playback quadratic.
    fn size_sum(&self, start: usize, end: usize) -> u64 {
        let end = end.min(self.stsz.len());
        self.size_prefix(end) - self.size_prefix(start.min(end))
    }
}

// ---------------------------------------------------------------------------
// Sample table box parsers
// ---------------------------------------------------------------------------

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
pub(super) fn parse_stts<R: Read + Seek>(
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
pub(super) fn parse_ctts<R: Read + Seek>(
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
pub(super) fn parse_stsc<R: Read + Seek>(
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
pub(super) fn parse_stsz<R: Read + Seek>(
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
pub(super) fn parse_stco<R: Read + Seek>(
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
pub(super) fn parse_co64<R: Read + Seek>(
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
pub(super) fn parse_stss<R: Read + Seek>(
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

// ---------------------------------------------------------------------------
// Sample reading and timestamp calculation
// ---------------------------------------------------------------------------

/// Locate a sample's file offset and size using the sample table.
///
/// `None` if the sample is outside the tables or its offset would overflow.
pub(super) fn sample_file_offset(table: &SampleTable, sample_idx: usize) -> Option<(u64, u32)> {
    let size = table.stsz.get(sample_idx)?;
    let (chunk_idx, first_in_chunk) = sample_to_chunk(table, sample_idx)?;
    let chunk_offset = *table.stco.get(chunk_idx)?;
    // Sizes of the samples that precede this one in its chunk.
    let preceding = table.size_sum(first_in_chunk, sample_idx);
    Some((chunk_offset.checked_add(preceding)?, size))
}

/// The chunk holding `sample_idx` and the first sample of that chunk.
fn sample_to_chunk(table: &SampleTable, sample_idx: usize) -> Option<(usize, usize)> {
    let idx = sample_idx as u64;
    let runs = &table.index.chunks;
    let run = &runs[runs
        .partition_point(|r| r.first_sample <= idx)
        .checked_sub(1)?];
    let chunk_in_run = (idx - run.first_sample) / run.samples_per_chunk;
    let chunk = run.first_chunk.checked_add(chunk_in_run)?;
    if chunk >= run.end_chunk {
        return None;
    }
    let first_in_chunk = run.first_sample + chunk_in_run * run.samples_per_chunk;
    Some((
        usize::try_from(chunk).ok()?,
        usize::try_from(first_in_chunk).ok()?,
    ))
}

/// The `stts` entry covering `idx` as `(entry, first_sample, first_dts)`.
/// `None` when `idx` is past the table (or the table is empty).
fn stts_entry(table: &SampleTable, idx: u64) -> Option<(&SttsEntry, u64, u64)> {
    let i = table
        .index
        .stts
        .partition_point(|&(s, _)| s <= idx)
        .checked_sub(1)?;
    let (start, dts) = table.index.stts[i];
    let entry = &table.stts[i];
    (idx - start < u64::from(entry.sample_count)).then_some((entry, start, dts))
}

/// Calculate the PTS in timescale units for a given sample index.
///
/// Past the end of `stts` the DTS stays at the table's total duration.
pub(super) fn sample_pts(table: &SampleTable, sample_idx: usize) -> u64 {
    let idx = sample_idx as u64;
    let dts = match stts_entry(table, idx) {
        Some((entry, start, dts)) => {
            dts.saturating_add((idx - start).saturating_mul(u64::from(entry.sample_delta)))
        },
        None => table.index.stts_total.1,
    };

    // CTS offset from ctts (0 outside the table).
    let ctts = &table.index.ctts;
    let cts_offset = ctts
        .partition_point(|&s| s <= idx)
        .checked_sub(1)
        .map(|i| (ctts[i], &table.ctts[i]))
        .filter(|(start, e)| idx - start < u64::from(e.sample_count))
        .map_or(0, |(_, e)| i64::from(e.sample_offset));

    dts.saturating_add_signed(cts_offset)
}

/// Decode timestamp (stts only, no ctts) and duration of a sample, in
/// timescale units.  Decode order is what demuxers iterate and seek in, so
/// unlike [`sample_pts`] this is monotonic in `sample_idx`.
pub(super) fn sample_dts_delta(table: &SampleTable, sample_idx: usize) -> (u64, u32) {
    let idx = sample_idx as u64;
    if let Some((entry, start, dts)) = stts_entry(table, idx) {
        let dts = dts.saturating_add((idx - start).saturating_mul(u64::from(entry.sample_delta)));
        return (dts, entry.sample_delta);
    }
    // Beyond the stts range: extrapolate with the last delta.
    let last_delta = table.stts.last().map_or(0, |e| e.sample_delta);
    let (total_samples, total_dts) = table.index.stts_total;
    let extra = idx
        .saturating_sub(total_samples)
        .saturating_mul(u64::from(last_delta));
    (total_dts.saturating_add(extra), last_delta)
}

/// Index of the sample whose decode interval contains `target_dts` (the
/// last sample starting at or before it).  Mirrors how symphonia's isomp4
/// demuxer resolves a seek timestamp to a sample.
pub(super) fn sample_containing_dts(table: &SampleTable, target_dts: u64) -> usize {
    let index = &table.index.stts;
    if let Some(i) = index
        .partition_point(|&(_, d)| d <= target_dts)
        .checked_sub(1)
    {
        let (start, dts) = index[i];
        let entry = &table.stts[i];
        let delta = u64::from(entry.sample_delta);
        let span = u64::from(entry.sample_count).saturating_mul(delta);
        if delta > 0 && target_dts < dts.saturating_add(span) {
            return to_usize(start + (target_dts - dts) / delta);
        }
    }
    to_usize(table.index.stts_total.0.saturating_sub(1))
}

/// Check if a sample is a keyframe.
pub(super) fn is_keyframe(table: &SampleTable, sample_idx: usize) -> bool {
    if table.stss.is_empty() {
        // No stss = all samples are sync samples.
        return true;
    }
    u32::try_from(sample_idx)
        .ok()
        .and_then(|i| i.checked_add(1))
        .is_some_and(|one_based| table.stss.binary_search(&one_based).is_ok())
}

/// Read a sample from the file and apply AVCC->Annex B if needed.
pub(super) fn read_sample<R: Read + Seek>(
    r: &mut R,
    track: &TrackInfo,
    sample_idx: usize,
) -> Result<RawSample, LiteError> {
    let (offset, size) = sample_file_offset(&track.table, sample_idx)
        .ok_or_else(|| LiteError::Parse(format!("sample {sample_idx} not found in table")))?;
    if size > MAX_SAMPLE_BYTES {
        return Err(LiteError::Parse(format!(
            "sample {sample_idx} is {size} bytes (limit {MAX_SAMPLE_BYTES})"
        )));
    }

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

/// The first sample whose DTS is at or after `target_ts`, or the last
/// sample when every sample starts before it.
pub(super) fn find_sample_at(track: &TrackInfo, target_ts: u64) -> usize {
    let table = &track.table;
    let index = &table.index.stts;
    let total = table.index.stts_total.0;
    // Entries that start strictly before the target.
    let before = index.partition_point(|&(_, d)| d < target_ts);
    if let Some(i) = before.checked_sub(1) {
        let (start, dts) = index[i];
        let entry = &table.stts[i];
        let delta = u64::from(entry.sample_delta);
        if delta > 0 {
            let k = (target_ts - dts).div_ceil(delta);
            if k < u64::from(entry.sample_count) {
                return to_usize(start + k);
            }
        }
    }
    // The first sample of the next entry starts at or after the target.
    match index.get(before) {
        Some(&(start, _)) if start < total => to_usize(start),
        _ => to_usize(total.saturating_sub(1)),
    }
}

/// A sample index as `usize`, saturating on 32-bit targets (the PSP).
fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stts(runs: &[(u32, u32)]) -> Vec<SttsEntry> {
        runs.iter()
            .map(|&(sample_count, sample_delta)| SttsEntry {
                sample_count,
                sample_delta,
            })
            .collect()
    }

    fn finalized(mut table: SampleTable) -> SampleTable {
        table.finalize();
        table
    }

    fn track(table: SampleTable) -> TrackInfo {
        TrackInfo {
            kind: TrackKind::Video,
            table: finalized(table),
            avcc: None,
            aac_config: None,
        }
    }

    #[test]
    fn sample_pts_from_stts() {
        let table = finalized(SampleTable {
            stts: stts(&[(3, 1000), (2, 2000)]),
            timescale: 30000,
            ..Default::default()
        });
        assert_eq!(sample_pts(&table, 0), 0);
        assert_eq!(sample_pts(&table, 1), 1000);
        assert_eq!(sample_pts(&table, 2), 2000);
        assert_eq!(sample_pts(&table, 3), 3000); // first in 2nd run
        assert_eq!(sample_pts(&table, 4), 3000 + 2000);
    }

    #[test]
    fn sample_pts_with_ctts() {
        let table = finalized(SampleTable {
            stts: stts(&[(3, 1000)]),
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
        });
        assert_eq!(sample_pts(&table, 0), 500); // DTS=0 + CTS=500
        assert_eq!(sample_pts(&table, 1), 800); // DTS=1000 + CTS=-200
        assert_eq!(sample_pts(&table, 2), 1800);
        assert_eq!(sample_pts(&table, 3), 3000); // past ctts: no offset
    }

    #[test]
    fn keyframe_lookup_stss() {
        let table = finalized(SampleTable {
            stss: vec![1, 5, 10], // 1-based
            stsz: vec![100; 15].into(),
            stts: stts(&[(15, 1000)]),
            timescale: 1000,
            ..Default::default()
        });
        assert!(is_keyframe(&table, 0)); // sample 1 (0-based=0)
        assert!(!is_keyframe(&table, 1));
        assert!(is_keyframe(&table, 4)); // sample 5 (0-based=4)
        assert!(is_keyframe(&table, 9)); // sample 10 (0-based=9)
        assert!(!is_keyframe(&table, usize::MAX));
    }

    #[test]
    fn unsorted_stss_is_sorted_by_finalize() {
        // Regression: a hostile/odd muxer's unsorted stss made the binary
        // search in `is_keyframe` miss real sync samples.
        let table = finalized(SampleTable {
            stss: vec![10, 1, 5, 5],
            ..Default::default()
        });
        assert_eq!(table.stss, vec![1, 5, 10]);
        assert!(is_keyframe(&table, 9));
    }

    #[test]
    fn empty_stts_table_pts_is_zero() {
        let table = finalized(SampleTable::default());
        assert_eq!(sample_pts(&table, 0), 0);
        assert_eq!(sample_pts(&table, 100), 0);
    }

    #[test]
    fn empty_stsz_table_sample_offset_returns_none() {
        let table = finalized(SampleTable::default());
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn empty_stsc_table_sample_to_chunk_returns_none() {
        let table = finalized(SampleTable {
            stsz: vec![100; 5].into(),
            stco: vec![0],
            ..Default::default()
        });
        // stsc is empty, so sample_to_chunk returns None.
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn empty_stco_table_sample_offset_returns_none() {
        let table = finalized(SampleTable {
            stsz: vec![100; 5].into(),
            stsc: vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 5,
            }],
            // stco is empty.
            ..Default::default()
        });
        assert!(sample_file_offset(&table, 0).is_none());
    }

    #[test]
    fn sample_offsets_across_stsc_runs() {
        // Chunks 1-2 hold 2 samples each, chunk 3 holds 3.
        let table = finalized(SampleTable {
            stsz: vec![10, 11, 12, 13, 14, 15, 16].into(),
            stsc: vec![
                StscEntry {
                    first_chunk: 1,
                    samples_per_chunk: 2,
                },
                StscEntry {
                    first_chunk: 3,
                    samples_per_chunk: 3,
                },
            ],
            stco: vec![1000, 2000, 3000],
            ..Default::default()
        });
        assert_eq!(sample_file_offset(&table, 0), Some((1000, 10)));
        assert_eq!(sample_file_offset(&table, 1), Some((1010, 11)));
        assert_eq!(sample_file_offset(&table, 2), Some((2000, 12)));
        assert_eq!(sample_file_offset(&table, 3), Some((2012, 13)));
        assert_eq!(sample_file_offset(&table, 4), Some((3000, 14)));
        assert_eq!(sample_file_offset(&table, 6), Some((3000 + 14 + 15, 16)));
        assert_eq!(sample_file_offset(&table, 7), None);
    }

    #[test]
    fn hostile_stsc_first_chunk_does_not_spin() {
        // Regression: a second stsc entry starting at chunk 0xFFFF_FFFF made
        // the old per-chunk walk iterate ~4 billion times per lookup.
        let table = finalized(SampleTable {
            stsz: SampleSizes::Uniform {
                size: 1,
                count: MAX_TABLE_ENTRIES,
            },
            stsc: vec![
                StscEntry {
                    first_chunk: 1,
                    samples_per_chunk: 0,
                },
                StscEntry {
                    first_chunk: u32::MAX,
                    samples_per_chunk: 1,
                },
            ],
            stco: vec![0; 4],
            ..Default::default()
        });
        assert_eq!(sample_file_offset(&table, 5_000_000), None);
    }

    #[test]
    fn chunk_offset_overflow_is_none() {
        // Regression: co64 offset near u64::MAX + preceding sample sizes
        // overflowed (panic in debug builds).
        let table = finalized(SampleTable {
            stsz: vec![u32::MAX; 2].into(),
            stsc: vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 2,
            }],
            stco: vec![u64::MAX - 10],
            ..Default::default()
        });
        assert_eq!(
            sample_file_offset(&table, 0),
            Some((u64::MAX - 10, u32::MAX))
        );
        assert_eq!(sample_file_offset(&table, 1), None);
    }

    #[test]
    fn uniform_sizes_behave_like_per_sample() {
        let uniform = SampleSizes::Uniform { size: 7, count: 4 };
        let per: SampleSizes = vec![7; 4].into();
        for s in [&uniform, &per] {
            assert_eq!(s.len(), 4);
            assert_eq!(s.get(3), Some(7));
            assert_eq!(s.get(4), None);
            assert_eq!(s.max(), 7);
            assert_eq!(s.sum(1, 3), 14);
            assert_eq!(s.sum(3, 100), 7);
        }
        assert_eq!(SampleSizes::Uniform { size: 7, count: 0 }.max(), 0);
    }

    #[test]
    fn indexed_size_sums_match_the_linear_walk() {
        let sizes: Vec<u32> = (0..1000u32).map(|i| i * 7 % 113).collect();
        let reference: SampleSizes = sizes.clone().into();
        let table = finalized(SampleTable {
            stsz: sizes.into(),
            ..Default::default()
        });
        for (start, end) in [
            (0, 0),
            (0, 64),
            (63, 65),
            (64, 128),
            (5, 999),
            (0, 1000),
            (900, 5000),
        ] {
            assert_eq!(
                table.size_sum(start, end),
                reference.sum(start, end),
                "{start}..{end}"
            );
        }
    }

    #[test]
    fn offset_lookup_in_a_huge_chunk_is_not_linear() {
        // One chunk of 2M samples: the last sample's offset used to sum
        // all 2M preceding sizes on every lookup.
        let n = 2_000_000usize;
        let table = finalized(SampleTable {
            stsc: vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: n as u32,
            }],
            stsz: vec![3u32; n].into(),
            stco: vec![1000],
            ..Default::default()
        });
        let start = std::time::Instant::now();
        for i in (n - 1000)..n {
            assert_eq!(
                sample_file_offset(&table, i),
                Some((1000 + 3 * i as u64, 3))
            );
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn no_stss_means_all_keyframes() {
        let table = finalized(SampleTable {
            stsz: vec![100; 10].into(),
            ..Default::default()
        });
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
        let table = finalized(SampleTable {
            stts: stts(&[(2, 1000)]),
            timescale: 1000,
            ..Default::default()
        });
        // Sample 5 is beyond the table: the DTS stays at its total, 2000.
        assert_eq!(sample_pts(&table, 5), 2000);
    }

    #[test]
    fn ctts_negative_offset_clamps_to_zero() {
        let table = finalized(SampleTable {
            stts: stts(&[(1, 100)]),
            ctts: vec![CttsEntry {
                sample_count: 1,
                sample_offset: -500, // larger than DTS=0
            }],
            timescale: 1000,
            ..Default::default()
        });
        // DTS=0 + CTS=-500 = -500, clamped to 0.
        assert_eq!(sample_pts(&table, 0), 0);
    }

    #[test]
    fn dts_delta_extrapolates_past_table() {
        let table = finalized(SampleTable {
            stts: stts(&[(2, 100), (0, 5), (3, 10)]),
            ..Default::default()
        });
        assert_eq!(sample_dts_delta(&table, 1), (100, 100));
        assert_eq!(sample_dts_delta(&table, 2), (200, 10));
        assert_eq!(sample_dts_delta(&table, 4), (220, 10));
        // Past the 5 samples (230 ticks): extrapolate by the last delta.
        assert_eq!(sample_dts_delta(&table, 6), (240, 10));
    }

    #[test]
    fn containing_dts_skips_zero_delta_runs() {
        let table = finalized(SampleTable {
            stts: stts(&[(2, 100), (3, 0), (2, 50)]),
            ..Default::default()
        });
        assert_eq!(sample_containing_dts(&table, 0), 0);
        assert_eq!(sample_containing_dts(&table, 199), 1);
        // The zero-delta run occupies no time: 200 is the third run's first.
        assert_eq!(sample_containing_dts(&table, 200), 5);
        assert_eq!(sample_containing_dts(&table, 260), 6);
        assert_eq!(sample_containing_dts(&table, 10_000), 6);
    }

    #[test]
    fn find_keyframe_before_no_stss() {
        let track = track(SampleTable {
            stts: stts(&[(10, 1000)]),
            stsz: vec![100; 10].into(),
            timescale: 1000,
            ..Default::default()
        });
        // With no stss, delegates to find_sample_at.
        assert_eq!(find_keyframe_before(&track, 5000), 5);
    }

    #[test]
    fn find_sample_at_rounds_up_to_next_sample() {
        let track = track(SampleTable {
            stts: stts(&[(3, 1000), (0, 7), (2, 500)]),
            ..Default::default()
        });
        assert_eq!(find_sample_at(&track, 0), 0);
        assert_eq!(find_sample_at(&track, 1), 1);
        assert_eq!(find_sample_at(&track, 2000), 2);
        assert_eq!(find_sample_at(&track, 2001), 3);
        assert_eq!(find_sample_at(&track, 3400), 4);
        assert_eq!(find_sample_at(&track, 99_999), 4);
    }

    #[test]
    fn find_sample_at_huge_run_is_constant_time() {
        // Regression: the old per-sample walk took ~4 billion steps here.
        let track = track(SampleTable {
            stts: stts(&[(u32::MAX, 1)]),
            ..Default::default()
        });
        assert_eq!(find_sample_at(&track, 4_000_000_000), 4_000_000_000);
    }

    #[test]
    fn find_sample_at_empty_stts() {
        let track = track(SampleTable::default());
        assert_eq!(find_sample_at(&track, 5000), 0);
    }

    // ---------------------------------------------------------------
    // Property-based tests (proptest)
    // ---------------------------------------------------------------

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        /// Reference implementations: the original per-sample walks.
        fn naive_find_sample_at(table: &SampleTable, target: u64) -> usize {
            let mut dts = 0u64;
            let mut sample = 0usize;
            for e in &table.stts {
                for _ in 0..e.sample_count {
                    if dts >= target {
                        return sample;
                    }
                    dts += u64::from(e.sample_delta);
                    sample += 1;
                }
            }
            sample.saturating_sub(1)
        }

        fn naive_pts(table: &SampleTable, idx: usize) -> u64 {
            let mut dts = 0u64;
            let mut sample = 0usize;
            for e in &table.stts {
                let c = e.sample_count as usize;
                if sample + c > idx {
                    dts += (idx - sample) as u64 * u64::from(e.sample_delta);
                    break;
                }
                dts += c as u64 * u64::from(e.sample_delta);
                sample += c;
            }
            let mut s = 0usize;
            let mut off = 0i64;
            for e in &table.ctts {
                if s + e.sample_count as usize > idx {
                    off = i64::from(e.sample_offset);
                    break;
                }
                s += e.sample_count as usize;
            }
            (dts as i64 + off).max(0) as u64
        }

        fn naive_offset(table: &SampleTable, idx: usize) -> Option<(u64, u32)> {
            let size = table.stsz.get(idx)?;
            let mut accum = 0usize;
            for (i, e) in table.stsc.iter().enumerate() {
                let first = (e.first_chunk as usize).saturating_sub(1);
                let next = table.stsc.get(i + 1).map_or(table.stco.len(), |n| {
                    (n.first_chunk as usize).saturating_sub(1)
                });
                for chunk in first..next.min(table.stco.len()) {
                    let spc = e.samples_per_chunk as usize;
                    if idx < accum + spc {
                        let base = *table.stco.get(chunk)?;
                        return Some((base + table.stsz.sum(accum, idx), size));
                    }
                    accum += spc;
                }
            }
            None
        }

        fn arb_table() -> impl Strategy<Value = SampleTable> {
            (
                proptest::collection::vec((0u32..20, 0u32..1000), 0..8),
                proptest::collection::vec((0u32..20, -500i32..500), 0..8),
                proptest::collection::vec((1u32..6, 0u32..5), 0..5),
                proptest::collection::vec(0u64..1_000_000, 0..10),
                proptest::collection::vec(0u32..5000, 0..60),
            )
                .prop_map(|(t, c, mut sc, stco, sizes)| {
                    sc.sort_by_key(|&(first, _)| first);
                    finalized(SampleTable {
                        stts: stts(&t),
                        ctts: c
                            .into_iter()
                            .map(|(sample_count, sample_offset)| CttsEntry {
                                sample_count,
                                sample_offset,
                            })
                            .collect(),
                        stsc: sc
                            .into_iter()
                            .map(|(first_chunk, samples_per_chunk)| StscEntry {
                                first_chunk,
                                samples_per_chunk,
                            })
                            .collect(),
                        stco,
                        stsz: sizes.into(),
                        timescale: 1000,
                        ..Default::default()
                    })
                })
        }

        proptest! {
            #[test]
            fn indexed_lookups_match_naive_walks(
                table in arb_table(),
                idx in 0usize..200,
                target in 0u64..50_000,
            ) {
                prop_assert_eq!(sample_pts(&table, idx), naive_pts(&table, idx));
                prop_assert_eq!(
                    sample_file_offset(&table, idx),
                    naive_offset(&table, idx)
                );
                let t = TrackInfo {
                    kind: TrackKind::Audio,
                    table: table.clone(),
                    avcc: None,
                    aac_config: None,
                };
                prop_assert_eq!(
                    find_sample_at(&t, target),
                    naive_find_sample_at(&table, target)
                );
            }

            #[test]
            fn sample_pts_no_panic(
                counts in proptest::collection::vec(any::<u32>(), 1..20),
                deltas in proptest::collection::vec(any::<u32>(), 1..20),
                sample_idx in any::<usize>(),
            ) {
                let runs: Vec<_> = counts.into_iter().zip(deltas).collect();
                let table = finalized(SampleTable {
                    stts: stts(&runs),
                    timescale: 90000,
                    ..Default::default()
                });
                let _ = sample_pts(&table, sample_idx);
                let _ = sample_dts_delta(&table, sample_idx);
                let _ = sample_containing_dts(&table, sample_idx as u64);
            }

            #[test]
            fn find_sample_at_no_panic(
                counts in proptest::collection::vec(any::<u32>(), 1..10),
                deltas in proptest::collection::vec(any::<u32>(), 1..10),
                target_ts in any::<u64>(),
            ) {
                let runs: Vec<_> = counts.into_iter().zip(deltas).collect();
                let t = track(SampleTable {
                    stts: stts(&runs),
                    timescale: 90000,
                    ..Default::default()
                });
                let _ = find_sample_at(&t, target_ts);
            }
        }
    }
}
