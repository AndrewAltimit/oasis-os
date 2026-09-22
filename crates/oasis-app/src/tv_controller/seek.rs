//! Seek estimation and moov-atom parsing.
//!
//! When the user seeks into a partially-downloaded video, we need to estimate
//! the byte offset that corresponds to a given timestamp.  Two paths are used:
//!
//! 1. `parse_moov_duration` reads the `mvhd` child of an MP4 `moov` atom to get
//!    the file duration in seconds.
//! 2. `linear_seek_interpolation` uses that duration plus the `mdat` byte
//!    range to estimate `(seek_secs / duration) * mdat_size`.
//!
//! Both are only fallbacks: `estimate_seek_byte` prefers the exact byte from
//! the moov sample tables (`oasis_video::demux_lite::seek_point_from_moov`),
//! which is where the demuxer will actually start reading.  The decoder
//! seeks to the same keyframe (`SoftwareVideoDecoder::seek` snaps to it), so
//! the Range restart and the first packet read agree.  A linear estimate can
//! be off by many MB on variable-bitrate video; restarting past the real
//! position made every read hit the evicted region below the window.
//!
//! `parse_tail_for_moov` handles moov-at-end MP4 files by scanning a
//! tail-fetched buffer for the `moov` fourcc and validating the atom header.
//! `check_moov_at_start_restart` decides whether a Range restart is worth
//! issuing when moov is found near the start of the file.

#[cfg(feature = "_video")]
use super::streaming_buffer::{SlidingState, StreamingInner};

/// Short-seek read-through threshold: if the seek position is within this
/// many bytes of data already downloaded, continue the linear download
/// instead of reconnecting with a new Range request.  Inspired by ffmpeg's
/// `avio.c` short-seek optimization (ffmpeg defaults to ~half buffer size).
/// We use a larger value since HTTP Range reconnects are expensive.
#[cfg(feature = "_video")]
pub(crate) const SHORT_SEEK_THRESHOLD: u64 = 4 * 1024 * 1024; // 4 MB

/// Pure logic for linear seek interpolation within mdat.
///
/// Returns estimated byte offset = `mdat_offset + (seek_secs / duration) * mdat_size`.
#[cfg(feature = "_video")]
pub(crate) fn linear_seek_interpolation(
    seek_secs: f64,
    duration: f64,
    mdat_offset: u64,
    mdat_size: u64,
) -> u64 {
    if duration <= 0.0 {
        return mdat_offset;
    }
    let frac = (seek_secs / duration).clamp(0.0, 1.0);
    mdat_offset.saturating_add((frac * mdat_size as f64) as u64)
}

/// Parse the duration (in seconds) from an MP4 `moov` atom by reading its
/// `mvhd` child.  Supports both `mvhd` v0 (32-bit duration) and v1 (64-bit
/// duration).
///
/// Returns `None` if the moov data is malformed, mvhd is missing, or
/// timescale is zero.
#[cfg(feature = "_video")]
pub(crate) fn parse_moov_duration(moov_data: &[u8]) -> Option<f64> {
    // moov is a container atom. Scan its children for mvhd.
    let mut pos = 8usize; // skip moov header (size + fourcc)
    while pos + 8 <= moov_data.len() {
        let size = u32::from_be_bytes([
            moov_data[pos],
            moov_data[pos + 1],
            moov_data[pos + 2],
            moov_data[pos + 3],
        ]) as usize;
        if size < 8 || pos + size > moov_data.len() {
            break;
        }
        let fourcc = &moov_data[pos + 4..pos + 8];
        if fourcc == b"mvhd" {
            // mvhd: version(1) + flags(3) + ...
            let data = &moov_data[pos + 8..pos + size];
            if data.is_empty() {
                return None;
            }
            let version = data[0];
            if version == 0 && data.len() >= 20 {
                // v0 layout after version(1)+flags(3): create(4) + mod(4) + timescale(4) +
                // duration(4) timescale starts at byte 12, duration at byte 16
                let timescale = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
                let duration = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            } else if version == 1 && data.len() >= 32 {
                // v1 layout after version(1)+flags(3): create(8) + mod(8) + timescale(4) +
                // duration(8) timescale starts at byte 20, duration at byte 24
                let timescale = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                let duration = u64::from_be_bytes([
                    data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
                ]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            }
            return None;
        }
        pos += size;
    }
    None
}

/// How far before the exact seek byte (from the moov sample tables) a Range
/// restart begins.  The exact byte is where the demuxer's first read lands,
/// so this is only a safety margin; keeping it small means the prebuffer
/// (`MIN_PREBUFFER`, counted from the restart point) is mostly data the
/// decoder will actually consume.
#[cfg(feature = "_video")]
pub(crate) const EXACT_SEEK_MARGIN: u64 = 256 * 1024;

/// Margin before a *linear* (duration-proportional) seek estimate, which can
/// land well away from the real sample on variable-bitrate video.
#[cfg(feature = "_video")]
pub(crate) const LINEAR_SEEK_MARGIN: u64 = 2 * 1024 * 1024;

/// A byte position the demuxer will read from after seeking.
#[cfg(feature = "_video")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeekEstimate {
    /// Estimated first byte the demuxer reads.
    pub(crate) byte: u64,
    /// `true` when computed from the moov sample tables (exact), `false`
    /// for the linear fallback.
    pub(crate) exact: bool,
}

#[cfg(feature = "_video")]
impl SeekEstimate {
    /// Where a Range restart should begin to cover this seek.
    pub(crate) fn restart_from(self) -> u64 {
        let margin = if self.exact {
            EXACT_SEEK_MARGIN
        } else {
            LINEAR_SEEK_MARGIN
        };
        self.byte.saturating_sub(margin)
    }
}

/// Estimate the first byte the demuxer reads after seeking to `seek_secs`.
///
/// Uses the exact position from the moov sample tables when available
/// (the lower of the video keyframe at or before the target and the audio
/// sample at that time -- the decoder seeks to that keyframe).  Falls back
/// to linear interpolation over `mdat` (`(offset, size)`) when the tables
/// can't be parsed.
#[cfg(feature = "_video")]
pub(crate) fn estimate_seek_byte(
    moov_data: &[u8],
    seek_secs: u64,
    mdat: Option<(u64, u64)>,
) -> Option<SeekEstimate> {
    let exact = oasis_video::demux_lite::seek_point_from_moov(moov_data, seek_secs as f64);
    let linear = mdat.and_then(|(mdat_off, mdat_size)| {
        parse_moov_duration(moov_data)
            .map(|dur| linear_seek_interpolation(seek_secs as f64, dur, mdat_off, mdat_size))
    });
    match (exact, linear) {
        (Some(point), linear) => {
            log::info!(
                "TV: seek estimate: {seek_secs}s -> keyframe {:.2}s at byte {:.1}MB \
                 (linear estimate {})",
                point.keyframe_secs,
                point.min_byte as f64 / (1024.0 * 1024.0),
                linear.map_or_else(
                    || "n/a".to_string(),
                    |l| format!("{:.1}MB", l as f64 / (1024.0 * 1024.0))
                ),
            );
            Some(SeekEstimate {
                byte: point.min_byte,
                exact: true,
            })
        },
        (None, Some(linear)) => {
            log::info!(
                "TV: seek estimate: {seek_secs}s -> ~{:.1}MB (linear; no sample tables)",
                linear as f64 / (1024.0 * 1024.0),
            );
            Some(SeekEstimate {
                byte: linear,
                exact: false,
            })
        },
        (None, None) => None,
    }
}

/// Check if a moov-at-start file should restart download from a seek position.
/// Returns `Some(byte_offset)` if restart is worthwhile.
#[cfg(feature = "_video")]
pub(crate) fn check_moov_at_start_restart(
    s: &SlidingState,
    seek_secs: u64,
    bytes_received: u64,
) -> Option<u64> {
    let moov_data = s.moov.as_ref().map(|(_, d)| d)?;
    let mdat = s
        .atoms
        .iter()
        .find(|(_, size, cc)| cc == b"mdat" && *size > 1024)
        .map(|(off, size, _)| (*off, *size));
    let estimate = estimate_seek_byte(moov_data, seek_secs, mdat)?;

    // Clamp seek byte to file boundaries.
    let total = bytes_received.max(
        s.atoms
            .iter()
            .map(|(off, sz, _)| off + sz)
            .max()
            .unwrap_or(0),
    );
    let estimate = SeekEstimate {
        byte: estimate.byte.min(total),
        ..estimate
    };
    let start_from = estimate.restart_from();
    let downloaded = bytes_received;
    if start_from > downloaded + SHORT_SEEK_THRESHOLD {
        log::info!(
            "TV: moov-at-start: seek={seek_secs}s -> byte ~{:.1}MB \
             (downloaded {:.1}MB), restarting from {:.1}MB",
            estimate.byte as f64 / (1024.0 * 1024.0),
            downloaded as f64 / (1024.0 * 1024.0),
            start_from as f64 / (1024.0 * 1024.0),
        );
        Some(start_from)
    } else {
        None
    }
}

/// Parse tail data (fetched via Range) looking for the moov atom.
/// If found, retains it in the buffer and notifies waiters.
///
/// The tail data typically starts in the middle of an mdat atom (raw
/// video/audio data), so we cannot walk atom boundaries from offset 0.
/// Instead, scan for the `moov` fourcc and validate the atom header.
#[cfg(feature = "_video")]
pub(crate) fn parse_tail_for_moov(
    buffer: &StreamingInner,
    tail_data: &[u8],
    tail_offset: u64,
    content_length: u64,
    seek_secs: u64,
) {
    // Scan for the 'moov' fourcc.  In an MP4 atom header the layout is
    // [4-byte big-endian size][4-byte fourcc], so 'moov' appears at
    // offset+4 of the atom header.  We look for the fourcc and then
    // validate the preceding size field.
    let needle = b"moov";
    let mut search_from = 4usize; // need >=4 bytes before fourcc for size
    let found = loop {
        if search_from + 4 > tail_data.len() {
            break None;
        }
        let haystack = &tail_data[search_from..];
        let pos = haystack.windows(4).position(|w| w == needle);
        let Some(rel) = pos else { break None };
        let fourcc_off = search_from + rel;
        let atom_start = fourcc_off - 4; // size field is 4 bytes before fourcc
        let size32 = u32::from_be_bytes([
            tail_data[atom_start],
            tail_data[atom_start + 1],
            tail_data[atom_start + 2],
            tail_data[atom_start + 3],
        ]);
        let atom_size = if size32 == 1 && atom_start + 16 <= tail_data.len() {
            let b = &tail_data[atom_start + 8..atom_start + 16];
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
        } else if size32 == 0 {
            tail_data.len() - atom_start
        } else {
            size32 as usize
        };
        // Validate: atom must be >=8 bytes and fit within the tail data.
        if atom_size >= 8 && atom_start + atom_size <= tail_data.len() {
            break Some((atom_start, atom_size));
        }
        // False positive -- keep scanning past this occurrence.
        search_from = fourcc_off + 4;
    };

    let Some((atom_start, atom_size)) = found else {
        log::info!(
            "TV: tail probe: no moov found in last {:.1}MB of file",
            tail_data.len() as f64 / (1024.0 * 1024.0)
        );
        return;
    };

    let file_off = tail_offset + atom_start as u64;
    let moov_data = tail_data[atom_start..atom_start + atom_size].to_vec();
    log::info!(
        "TV: pre-fetched moov atom ({} bytes) at file offset {}",
        moov_data.len(),
        file_off,
    );

    // If seeking, compute byte offset and set base_offset so the
    // main download thread can restart from the seek position.
    if seek_secs > 0 {
        // mdat for the linear fallback: moov-at-end files put mdat before
        // moov, so it spans (roughly) [0, file_off).
        let Some(estimate) = estimate_seek_byte(&moov_data, seek_secs, Some((0, file_off))) else {
            // Cannot estimate -- retain moov and let decoder seek.
            let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
            s.moov = Some((file_off, std::sync::Arc::new(moov_data)));
            buffer.condvar.notify_all();
            return;
        };
        // Clamp to file size to avoid requesting bytes beyond EOF.
        let estimate = SeekEstimate {
            byte: estimate.byte.min(content_length.saturating_sub(1)),
            ..estimate
        };
        let seek_byte = estimate.byte;
        let start_from = estimate.restart_from();
        log::info!(
            "TV: tail probe: seek={seek_secs}s -> byte ~{:.1}MB, \
             need download from {:.1}MB",
            seek_byte as f64 / (1024.0 * 1024.0),
            start_from as f64 / (1024.0 * 1024.0),
        );
        let moov_arc = std::sync::Arc::new(moov_data);
        let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
        s.moov = Some((file_off, std::sync::Arc::clone(&moov_arc)));
        // Set base_offset so the main download loop knows where
        // to restart from (checked via `restart_offset`).
        let received = buffer.bytes_received();
        if start_from > received + SHORT_SEEK_THRESHOLD {
            // Retain file header (ftyp + mdat header) for symphonia
            // probe.  Upgrade if current header is smaller.
            if !s.buf.is_empty() {
                let current_len = s.header.as_ref().map_or(0, |h| h.len());
                let keep = s.buf.len().min(4096);
                if keep > current_len {
                    s.header = Some(s.buf[..keep].to_vec());
                }
            }
            s.base_offset = start_from;
            buffer
                .bytes_received
                .store(start_from, std::sync::atomic::Ordering::Release);
            s.buf.clear();
        }
        drop(s);
        buffer.condvar.notify_all();
        // Signal content_length for seek-based range download.
        buffer
            .total_size
            .store(content_length, std::sync::atomic::Ordering::Release);
        return;
    }

    let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
    s.moov = Some((file_off, std::sync::Arc::new(moov_data)));
    buffer.condvar.notify_all();
}
