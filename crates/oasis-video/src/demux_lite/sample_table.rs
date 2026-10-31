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

use super::avcc::avcc_to_annex_b;
use super::{LiteError, RawSample, TrackInfo, TrackKind};

mod parse;

pub(super) use parse::{
    parse_co64, parse_ctts, parse_stco, parse_stsc, parse_stss, parse_stsz, parse_stts,
};

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
mod tests;
