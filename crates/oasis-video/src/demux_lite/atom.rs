//! Atom/box parsing (ftyp, moov, mvhd, trak, stbl subbox parsing).
//!
//! Every box is bounds-checked against its parent before it is parsed or
//! skipped: a child whose declared size runs past its parent ends the
//! scan, box end offsets are overflow-checked, container recursion is
//! depth-limited, and the variable-size config boxes (`avcC`, `esds`) are
//! capped, so a hostile file can neither loop, overflow the stack, nor make
//! the parser allocate more than the bytes it actually holds.

use std::io::{Read, Seek, SeekFrom};

use super::avcc::parse_avcc_record;
use super::sample_table::{
    SampleTable, parse_co64, parse_ctts, parse_stco, parse_stsc, parse_stss, parse_stsz, parse_stts,
};
use super::{AacConfig, AvccConfig, LiteError, TrackInfo, TrackKind};

/// Deepest container nesting the parser follows.  Real files nest
/// `moov/trak/mdia/minf/stbl/stsd` (6); every level costs only 8 bytes, so
/// without a limit a few hundred KB of nested `moov` headers overflow the
/// stack (fatal on the PSP's small thread stacks).
pub(super) const MAX_BOX_DEPTH: u32 = 16;

/// Largest `avcC` / `esds` box parsed (64 KiB).  Real ones are tens of
/// bytes; the parser reads them into memory whole.
const MAX_CONFIG_BOX_BYTES: u64 = 64 * 1024;

/// Parsed box header.
#[derive(Debug, Clone)]
pub(super) struct BoxHeader {
    pub(super) box_type: [u8; 4],
    /// Size of the content (excluding header).
    pub(super) content_size: u64,
    /// Absolute offset where content starts.
    pub(super) content_offset: u64,
    /// Absolute offset one past the box (`content_offset + content_size`,
    /// overflow-checked by [`read_box_header`]).
    pub(super) end: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn read_u32_be<R: Read>(r: &mut R) -> Result<u32, LiteError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

pub(super) fn read_u64_be<R: Read>(r: &mut R) -> Result<u64, LiteError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_be_bytes(buf))
}

// ---------------------------------------------------------------------------
// Box header parsing
// ---------------------------------------------------------------------------

/// Read a box header at the current position.
pub(super) fn read_box_header<R: Read + Seek>(r: &mut R) -> Result<Option<BoxHeader>, LiteError> {
    let offset = r.stream_position()?;
    let mut buf = [0u8; 8];
    match r.read_exact(&mut buf) {
        Ok(()) => {},
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    let size = u64::from(u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]));
    let box_type = [buf[4], buf[5], buf[6], buf[7]];

    let (header_len, total) = if size == 1 {
        // Extended size.
        let ext_size = read_u64_be(r)?;
        if ext_size < 16 {
            return Err(LiteError::Parse("extended box size < 16".into()));
        }
        (16, ext_size)
    } else if size == 0 {
        // Box extends to end of file.
        let file_end = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(offset + 8))?;
        (8, file_end.saturating_sub(offset))
    } else {
        if size < 8 {
            return Err(LiteError::Parse("box size < 8".into()));
        }
        (8, size)
    };

    let end = offset
        .checked_add(total)
        .ok_or_else(|| LiteError::Parse("box end offset overflows".into()))?;
    Ok(Some(BoxHeader {
        box_type,
        content_size: total - header_len,
        content_offset: offset + header_len,
        end,
    }))
}

/// Read the header of the next child box at `pos`, or `None` when the
/// parent (ending at `end`) holds no further complete box.
fn next_child<R: Read + Seek>(
    r: &mut R,
    pos: u64,
    end: u64,
) -> Result<Option<BoxHeader>, LiteError> {
    if pos.saturating_add(8) > end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(pos))?;
    Ok(read_box_header(r)?.filter(|h| h.end <= end))
}

/// Container box types that should be recursed into.
const CONTAINER_BOXES: &[[u8; 4]] = &[
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts",
];

/// Parse boxes within a range, recursing into container boxes.
pub(super) fn parse_boxes<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    parse_boxes_at(r, start, end, video, audio, 0)
}

fn parse_boxes_at<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
    depth: u32,
) -> Result<(), LiteError> {
    if depth > MAX_BOX_DEPTH {
        return Err(LiteError::Parse("boxes nested too deeply".into()));
    }
    let mut pos = start;
    while let Some(header) = next_child(r, pos, end)? {
        let (offset, size) = (header.content_offset, header.content_size);
        match &header.box_type {
            b"trak" => parse_trak(r, offset, header.end, video, audio, depth + 1)?,
            bt if CONTAINER_BOXES.contains(bt) => {
                parse_boxes_at(r, offset, header.end, video, audio, depth + 1)?;
            },
            b"stsd" => parse_stsd(r, offset, size, video, audio)?,
            b"stts" => parse_stts(r, offset, size, video, audio)?,
            b"ctts" => parse_ctts(r, offset, size, video, audio)?,
            b"stsc" => parse_stsc(r, offset, size, video, audio)?,
            b"stsz" => parse_stsz(r, offset, size, video, audio)?,
            b"stco" => parse_stco(r, offset, size, video, audio)?,
            b"co64" => parse_co64(r, offset, size, video, audio)?,
            b"stss" => parse_stss(r, offset, size, video, audio)?,
            // `mdhd` is handled per-track by `parse_trak`: it precedes `stsd`
            // (which creates the track), so it cannot be routed through
            // `current_track_mut` like the sample-table boxes.
            _ => {},
        }
        pos = header.end;
    }
    Ok(())
}

/// Parse one `trak` box in isolation.
///
/// Each track is parsed into fresh slots so its boxes can only land on the
/// track it creates: `mdhd` (which precedes `stsd`) gets its timescale
/// applied to the right track, and the sample tables of tracks this demuxer
/// does not keep (a second video track, text/timecode tracks) are dropped
/// instead of being merged into an already-parsed track.
fn parse_trak<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
    depth: u32,
) -> Result<(), LiteError> {
    let mut trak_video = None;
    let mut trak_audio = None;
    parse_boxes_at(r, start, end, &mut trak_video, &mut trak_audio, depth)?;
    let timescale = find_mdhd_timescale(r, start, end, depth)?;

    for (slot, parsed) in [(video, trak_video), (audio, trak_audio)] {
        if let Some(mut track) = parsed
            && slot.is_none()
        {
            if let Some(ts) = timescale {
                track.table.timescale = ts;
            }
            *slot = Some(track);
        }
    }
    Ok(())
}

/// Find `trak/mdia/mdhd` within `[start, end)` and return its timescale.
fn find_mdhd_timescale<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    depth: u32,
) -> Result<Option<u32>, LiteError> {
    if depth > MAX_BOX_DEPTH {
        return Err(LiteError::Parse("boxes nested too deeply".into()));
    }
    let mut pos = start;
    while let Some(header) = next_child(r, pos, end)? {
        if header.box_type == *b"mdia" {
            return find_mdhd_timescale(r, header.content_offset, header.end, depth + 1);
        }
        if header.box_type == *b"mdhd" {
            return read_mdhd_timescale(r, header.content_offset).map(Some);
        }
        pos = header.end;
    }
    Ok(None)
}

/// Get a mutable reference to the "current" (last-added) track being built.
///
/// The most recently created track with an empty sample table is the one
/// currently being parsed. If both have data, prefer audio (added last).
pub(super) fn current_track_mut<'a>(
    video: &'a mut Option<TrackInfo>,
    audio: &'a mut Option<TrackInfo>,
) -> Option<&'a mut TrackInfo> {
    fn tables_empty(t: &SampleTable) -> bool {
        t.stsz.is_empty() && t.stts.is_empty() && t.stco.is_empty() && t.stsc.is_empty()
    }
    if audio.as_ref().is_some_and(|a| tables_empty(&a.table)) {
        return audio.as_mut();
    }
    if video.as_ref().is_some_and(|v| tables_empty(&v.table)) {
        return video.as_mut();
    }
    // Both have data -- return audio (most recently created), else video.
    if audio.is_some() {
        audio.as_mut()
    } else {
        video.as_mut()
    }
}

// ---------------------------------------------------------------------------
// Individual box parsers
// ---------------------------------------------------------------------------

/// Parse stsd (sample description) to discover track codecs.
fn parse_stsd<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let entry_count = read_u32_be(r)?;

    let entries_end = offset.saturating_add(size);
    let mut pos = offset + 8;
    for _ in 0..entry_count {
        let Some(header) = next_child(r, pos, entries_end)? else {
            break;
        };

        let codec = header.box_type;
        if codec == *b"avc1" || codec == *b"avc3" {
            // H.264 video track.
            let avcc = parse_avc_sample_entry(r, header.content_offset, header.end)?;
            if video.is_none() {
                *video = Some(TrackInfo {
                    kind: TrackKind::Video,
                    table: SampleTable::default(),
                    avcc,
                    aac_config: None,
                });
            }
        } else if codec == *b"mp4a" {
            // AAC audio track.
            let aac = parse_mp4a_sample_entry(r, header.content_offset, header.end)?;
            if audio.is_none() {
                *audio = Some(TrackInfo {
                    kind: TrackKind::Audio,
                    table: SampleTable::default(),
                    avcc: None,
                    aac_config: aac,
                });
            }
        }
        pos = header.end;
    }

    Ok(())
}

/// Find the first child box of type `fourcc` in `[start, end)`.
fn find_child<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    fourcc: &[u8; 4],
) -> Result<Option<BoxHeader>, LiteError> {
    let mut pos = start;
    while let Some(header) = next_child(r, pos, end)? {
        if header.box_type == *fourcc {
            return Ok(Some(header));
        }
        pos = header.end;
    }
    Ok(None)
}

/// Read a small config box's content into memory.
fn read_config_box<R: Read + Seek>(r: &mut R, header: &BoxHeader) -> Result<Vec<u8>, LiteError> {
    if header.content_size > MAX_CONFIG_BOX_BYTES {
        return Err(LiteError::Parse(format!(
            "{} box of {} bytes exceeds {MAX_CONFIG_BOX_BYTES}",
            String::from_utf8_lossy(&header.box_type),
            header.content_size
        )));
    }
    r.seek(SeekFrom::Start(header.content_offset))?;
    let mut data = vec![0u8; header.content_size as usize];
    r.read_exact(&mut data)?;
    Ok(data)
}

/// Parse an avc1/avc3 sample entry (content `[offset, end)`) for its avcC.
fn parse_avc_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    end: u64,
) -> Result<Option<AvccConfig>, LiteError> {
    // The visual sample entry has a fixed 78-byte header (reserved, data
    // reference index, dimensions, resolution, compressor name, depth)
    // before its child boxes.
    match find_child(r, offset.saturating_add(78), end, b"avcC")? {
        Some(header) => parse_avcc_bytes(&read_config_box(r, &header)?).map(Some),
        None => Ok(None),
    }
}

/// Parse the avcC box contents (AVCDecoderConfigurationRecord).
///
/// Keeps the first SPS and the first PPS: [`AvccConfig`] holds one NAL of
/// each, and concatenating several into one buffer (as this used to) yields
/// a single corrupt NAL.  Multi-parameter-set streams also carry their
/// extra sets in-band.
pub(super) fn parse_avcc_bytes(data: &[u8]) -> Result<AvccConfig, LiteError> {
    let record = parse_avcc_record(data)?;
    let first = |sets: &[&[u8]]| sets.first().map(|s| s.to_vec()).unwrap_or_default();
    Ok(AvccConfig {
        nal_length_size: record.nal_length_size,
        sps: first(&record.sps),
        pps: first(&record.pps),
    })
}

/// Parse an mp4a sample entry (content `[offset, end)`) for its AAC config.
fn parse_mp4a_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    end: u64,
) -> Result<Option<AacConfig>, LiteError> {
    // AudioSampleEntry: 6 reserved + 2 data_ref_idx, then (QuickTime names
    // in brackets) 2 reserved [version] + 6 reserved [revision, vendor],
    // 2 channelcount, 2 samplesize, 2 pre_defined [compression id],
    // 2 reserved [packet size], 4 samplerate (16.16): 28 bytes.
    if offset.saturating_add(28) > end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(offset + 8))?; // skip reserved + data_ref_idx
    let mut hdr = [0u8; 20];
    r.read_exact(&mut hdr)?;
    let version = u16::from_be_bytes([hdr[0], hdr[1]]);
    let channels = u16::from_be_bytes([hdr[8], hdr[9]]);
    let sample_rate = u32::from_be_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]) >> 16;

    // QuickTime sound sample description v1 / v2 append 16 / 36 bytes of
    // extra fields before the child boxes.
    let extra = match version {
        1 => 16,
        2 => 36,
        _ => 0,
    };
    let config_data = match find_child(r, offset + 28 + extra, end, b"esds")? {
        Some(header) => parse_esds_for_asc(&read_config_box(r, &header)?),
        None => Vec::new(),
    };

    Ok(Some(AacConfig {
        sample_rate,
        channels,
        config_data,
    }))
}

/// Extract the AudioSpecificConfig from an `esds` box's content.
///
/// Walks ES_Descriptor (0x03) -> DecoderConfigDescriptor (0x04) ->
/// DecoderSpecificInfo (0x05) as ISO 14496-1 lays them out.  Empty when
/// the structure is missing or truncated.
pub(super) fn parse_esds_for_asc(data: &[u8]) -> Vec<u8> {
    // Skip the full-box version+flags.
    let Some(mut rest) = data.get(4..) else {
        return Vec::new();
    };
    let Some(es) = descriptor(&mut rest, 0x03) else {
        return Vec::new();
    };
    // ES_ID (2), then flags: streamDependence, URL, OCRstream.
    let Some(&flags) = es.get(2) else {
        return Vec::new();
    };
    let mut skip = 3;
    if flags & 0x80 != 0 {
        skip += 2; // dependsOn_ES_ID
    }
    if flags & 0x40 != 0 {
        skip += 1 + es.get(skip).map_or(0, |&n| usize::from(n)); // URL
    }
    if flags & 0x20 != 0 {
        skip += 2; // OCR_ES_Id
    }
    let Some(mut rest) = es.get(skip..) else {
        return Vec::new();
    };
    let Some(dcd) = descriptor(&mut rest, 0x04) else {
        return Vec::new();
    };
    // objectTypeIndication (1), streamType (1), bufferSizeDB (3),
    // maxBitrate (4), avgBitrate (4).
    let Some(mut rest) = dcd.get(13..) else {
        return Vec::new();
    };
    descriptor(&mut rest, 0x05)
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}

/// Find the first descriptor tagged `tag` in `*data`, skipping others, and
/// return its body; `*data` is advanced past it.
fn descriptor<'a>(data: &mut &'a [u8], tag: u8) -> Option<&'a [u8]> {
    loop {
        let (&t, rest) = data.split_first()?;
        let (len, consumed) = read_desc_len(rest);
        let body = rest.get(consumed..consumed.checked_add(len)?)?;
        *data = &rest[consumed + len..];
        if t == tag {
            return Some(body);
        }
    }
}

/// Read an ISO base media descriptor variable-length size.
///
/// Uses saturating arithmetic to prevent overflow on malicious inputs
/// (max valid value is 0x0FFFFFFF = 268 MB which fits in `usize`).
pub(super) fn read_desc_len(data: &[u8]) -> (usize, usize) {
    let mut len = 0usize;
    let mut consumed = 0;
    for &b in data.iter().take(4) {
        consumed += 1;
        len = len.saturating_mul(128).saturating_add(b as usize & 0x7F);
        if b & 0x80 == 0 {
            break;
        }
    }
    (len, consumed)
}

/// Read the timescale from an mdhd box whose content starts at `offset`.
fn read_mdhd_timescale<R: Read + Seek>(r: &mut R, offset: u64) -> Result<u32, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let version_flags = read_u32_be(r)?;
    let version = version_flags >> 24;
    // Skip creation_time + modification_time (4 bytes each in v0, 8 in v1).
    let skip = if version == 0 { 8 } else { 16 };
    r.seek(SeekFrom::Current(skip))?;
    read_u32_be(r)
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

    /// Build a minimal MP4-like byte stream from a list of boxes.
    /// Each entry is (fourcc, content_bytes). Size header is computed.
    fn build_boxes(boxes: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (fourcc, content) in boxes {
            let size = (8 + content.len()) as u32;
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(*fourcc);
            out.extend_from_slice(content);
        }
        out
    }

    #[test]
    fn box_header_ftyp() {
        let data = fixture_bytes();
        let mut cursor = Cursor::new(&data);
        let header = read_box_header(&mut cursor).unwrap().unwrap();
        // First box in a valid MP4 is usually ftyp.
        assert_eq!(&header.box_type, b"ftyp");
        assert!(header.content_size > 0);
        assert_eq!(header.content_offset, 8);
    }

    #[test]
    fn box_header_extended_size() {
        // Fake an extended-size box: size=1, type='test', extended=24.
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // size=1 -> extended
        data.extend_from_slice(b"test");
        data.extend_from_slice(&24u64.to_be_bytes()); // ext size=24 -> content=8
        data.extend_from_slice(&[0xAA; 8]); // content

        let mut cursor = Cursor::new(&data);
        let header = read_box_header(&mut cursor).unwrap().unwrap();
        assert_eq!(&header.box_type, b"test");
        assert_eq!(header.content_offset, 16);
        assert_eq!(header.content_size, 8);
    }

    #[test]
    fn truncated_box_header_too_short() {
        // Only 4 bytes -- not enough for an 8-byte box header.
        let data = vec![0x00, 0x00, 0x00, 0x10];
        let mut cursor = Cursor::new(&data);
        let result = read_box_header(&mut cursor).unwrap();
        // Should return None (UnexpectedEof).
        assert!(result.is_none());
    }

    #[test]
    fn empty_input_returns_none() {
        let data: Vec<u8> = Vec::new();
        let mut cursor = Cursor::new(&data);
        let result = read_box_header(&mut cursor).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn box_size_too_small_errors() {
        // size=4 is invalid (minimum is 8).
        let mut data = Vec::new();
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(b"test");
        let mut cursor = Cursor::new(&data);
        let result = read_box_header(&mut cursor);
        assert!(result.is_err(), "box size < 8 should error");
    }

    #[test]
    fn extended_size_too_small_errors() {
        // size=1 (extended), extended_size=12 (< 16).
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(b"test");
        data.extend_from_slice(&12u64.to_be_bytes());
        let mut cursor = Cursor::new(&data);
        let result = read_box_header(&mut cursor);
        assert!(result.is_err(), "extended size < 16 should error");
    }

    #[test]
    fn zero_size_box_extends_to_eof() {
        // size=0 means "extends to end of file".
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0xAA; 100]); // content
        let mut cursor = Cursor::new(&data);
        let header = read_box_header(&mut cursor).unwrap().unwrap();
        assert_eq!(&header.box_type, b"mdat");
        assert_eq!(header.content_offset, 8);
        assert_eq!(header.content_size, 100);
    }

    #[test]
    fn box_content_exceeds_parent_is_skipped() {
        // A box whose size exceeds the parent boundary is skipped
        // by parse_boxes (box_end > end check).
        let mut data = Vec::new();
        // A box that claims 1000 bytes but file is only 16.
        data.extend_from_slice(&1000u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0; 8]); // some content

        let mut video = None;
        let mut audio = None;
        let mut cursor = Cursor::new(&data);
        let result = parse_boxes(&mut cursor, 0, data.len() as u64, &mut video, &mut audio);
        // Should succeed (box is skipped, not an error).
        assert!(result.is_ok());
        // No tracks discovered.
        assert!(video.is_none());
        assert!(audio.is_none());
    }

    #[test]
    fn open_with_no_moov_finds_no_tracks() {
        // A valid ftyp + mdat but no moov.
        let data = build_boxes(&[(b"ftyp", &[0; 16]), (b"mdat", &[0; 64])]);
        let mut video = None;
        let mut audio = None;
        let mut cursor = Cursor::new(&data);
        parse_boxes(&mut cursor, 0, data.len() as u64, &mut video, &mut audio).unwrap();
        assert!(video.is_none());
        assert!(audio.is_none());
    }

    #[test]
    fn open_with_empty_moov_finds_no_tracks() {
        // A moov box with no children (no trak).
        let data = build_boxes(&[(b"ftyp", &[0; 16]), (b"moov", &[])]);
        let mut video = None;
        let mut audio = None;
        let mut cursor = Cursor::new(&data);
        parse_boxes(&mut cursor, 0, data.len() as u64, &mut video, &mut audio).unwrap();
        assert!(video.is_none());
        assert!(audio.is_none());
    }

    #[test]
    fn read_desc_len_single_byte() {
        let data = [0x10]; // 0x10 = 16, no continuation bit
        let (len, consumed) = read_desc_len(&data);
        assert_eq!(len, 16);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn read_desc_len_multi_byte() {
        // 0x81, 0x00 -> (1 << 7) | 0 = 128
        let data = [0x81, 0x00];
        let (len, consumed) = read_desc_len(&data);
        assert_eq!(len, 128);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn read_desc_len_empty() {
        let data: [u8; 0] = [];
        let (len, consumed) = read_desc_len(&data);
        assert_eq!(len, 0);
        assert_eq!(consumed, 0);
    }

    #[test]
    fn zero_size_moov_atom() {
        // A moov atom with size=0 (extends to EOF) but no valid children.
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 30]); // garbage children

        let mut video = None;
        let mut audio = None;
        let mut cursor = Cursor::new(&data);
        parse_boxes(&mut cursor, 0, data.len() as u64, &mut video, &mut audio).unwrap();
        assert!(video.is_none());
        assert!(audio.is_none());
    }

    fn parse(data: &[u8]) -> Result<(Option<TrackInfo>, Option<TrackInfo>), LiteError> {
        let (mut video, mut audio) = (None, None);
        parse_boxes(
            &mut Cursor::new(data),
            0,
            data.len() as u64,
            &mut video,
            &mut audio,
        )?;
        Ok((video, audio))
    }

    /// Wrap `content` in a box header.
    fn bx(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
        build_boxes(&[(fourcc, content)])
    }

    #[test]
    fn extended_size_end_overflow_is_an_error() {
        // Regression: a box at a non-zero offset with ext_size near
        // u64::MAX overflowed `content_offset + content_size` (a panic in
        // debug builds; in release the wrapped end sent the scan backwards).
        let mut data = bx(b"free", &[0; 8]);
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&u64::MAX.to_be_bytes());
        let mut cursor = Cursor::new(&data);
        cursor.set_position(16);
        assert!(read_box_header(&mut cursor).is_err());
        assert!(parse(&data).is_err());
    }

    #[test]
    fn deep_nesting_is_rejected_not_a_stack_overflow() {
        // Regression: 100k nested 8-byte `moov` headers recursed 100k deep.
        let depth = 100_000u32;
        let mut data = Vec::with_capacity(depth as usize * 8);
        for level in 0..depth {
            data.extend_from_slice(&((depth - level) * 8).to_be_bytes());
            data.extend_from_slice(b"moov");
        }
        assert!(parse(&data).is_err());
        // Real-world depth still parses.
        let mut nested = Vec::new();
        for fourcc in [b"stbl", b"minf", b"mdia", b"moov"] {
            nested = bx(fourcc, &nested);
        }
        assert!(parse(&nested).is_ok());
    }

    #[test]
    fn table_count_larger_than_box_is_rejected() {
        // Regression: a 16-byte stts declaring 10M entries reserved 80 MB
        // before the first read failed.
        let mut stts = vec![0; 4];
        stts.extend_from_slice(&10_000_000u32.to_be_bytes());
        stts.extend_from_slice(&[0; 8]);
        let err = parse(&bx(b"stbl", &bx(b"stts", &stts))).unwrap_err();
        assert!(err.to_string().contains("stts"), "{err}");
    }

    #[test]
    fn oversized_avcc_is_rejected() {
        let mut entry = vec![0u8; 78];
        entry.extend(bx(b"avcC", &vec![0u8; 70_000]));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(bx(b"avc1", &entry));
        assert!(parse(&bx(b"stsd", &stsd)).is_err());
    }

    #[test]
    fn stsd_entry_past_its_parent_is_ignored() {
        // The avc1 entry claims 4 KB inside a ~100-byte stsd.
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend_from_slice(&4096u32.to_be_bytes());
        stsd.extend_from_slice(b"avc1");
        stsd.extend_from_slice(&[0; 90]);
        let (video, audio) = parse(&bx(b"stsd", &stsd)).unwrap();
        assert!(video.is_none() && audio.is_none());
    }

    #[test]
    fn avcc_keeps_first_parameter_sets() {
        // Regression: two SPS used to be concatenated into one corrupt NAL.
        let mut avcc = vec![1, 0x42, 0, 0x1e, 0xFF, 0xE2];
        for sps in [&[0x67, 1][..], &[0x67, 2, 2]] {
            avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
            avcc.extend_from_slice(sps);
        }
        avcc.push(1);
        avcc.extend_from_slice(&[0, 2, 0x68, 9]);
        let cfg = parse_avcc_bytes(&avcc).unwrap();
        assert_eq!(cfg.nal_length_size, 4);
        assert_eq!(cfg.sps, vec![0x67, 1]);
        assert_eq!(cfg.pps, vec![0x68, 9]);
        // Truncated PPS data is an error.
        assert!(parse_avcc_bytes(&avcc[..avcc.len() - 1]).is_err());
    }

    /// An esds body: ES_Descriptor(ES_ID, flags) > DecoderConfigDescriptor
    /// > DecoderSpecificInfo(asc).
    fn esds(es_id: u16, asc: &[u8]) -> Vec<u8> {
        let mut dsi = vec![0x05, asc.len() as u8];
        dsi.extend_from_slice(asc);
        let mut dcd = vec![0x04, (13 + dsi.len()) as u8, 0x40, 0x15];
        dcd.extend_from_slice(&[0; 11]);
        dcd.extend(dsi);
        let mut es = vec![0x03, (3 + dcd.len() + 3) as u8];
        es.extend_from_slice(&es_id.to_be_bytes());
        es.push(0);
        es.extend(dcd);
        es.extend_from_slice(&[0x06, 1, 2]); // SLConfigDescriptor
        let mut out = vec![0; 4];
        out.extend(es);
        out
    }

    #[test]
    fn esds_walks_descriptors() {
        assert_eq!(
            parse_esds_for_asc(&esds(1, &[0x12, 0x10])),
            vec![0x12, 0x10]
        );
        // Regression: the old byte scan took the 0x05 in ES_ID 0x0502 for
        // the DecoderSpecificInfo tag and returned garbage.
        assert_eq!(
            parse_esds_for_asc(&esds(0x0502, &[0x11, 0x90])),
            vec![0x11, 0x90]
        );
        // Truncations never panic and yield no config.
        let full = esds(7, &[0x12, 0x10]);
        for len in 0..full.len() {
            let _ = parse_esds_for_asc(&full[..len]);
        }
        assert!(parse_esds_for_asc(&full[..10]).is_empty());
    }

    #[test]
    fn mp4a_channels_read_from_channelcount() {
        // Regression: `channels` was read from the reserved/version field
        // at entry offset 8 and came out 0 for every ISO file.
        let data = fixture_bytes();
        let (_, audio) = parse(&data).unwrap();
        let aac = audio.unwrap().aac_config.unwrap();
        assert!((1..=2).contains(&aac.channels), "channels {}", aac.channels);
        assert_eq!(aac.sample_rate, 22050);
        assert!(!aac.config_data.is_empty());
    }
}
