//! Atom/box parsing (ftyp, moov, mvhd, trak, stbl subbox parsing).

use std::io::{Read, Seek, SeekFrom};

use super::sample_table::{
    SampleTable, parse_co64, parse_ctts, parse_stco, parse_stsc, parse_stss, parse_stsz, parse_stts,
};
use super::{AacConfig, AvccConfig, LiteError, TrackInfo, TrackKind};

/// Parsed box header.
#[derive(Debug, Clone)]
pub(super) struct BoxHeader {
    pub(super) box_type: [u8; 4],
    /// Size of the content (excluding header).
    pub(super) content_size: u64,
    /// Absolute offset where content starts.
    pub(super) content_offset: u64,
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

    let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64;
    let box_type = [buf[4], buf[5], buf[6], buf[7]];

    let (content_offset, content_size) = if size == 1 {
        // Extended size.
        let mut ext = [0u8; 8];
        r.read_exact(&mut ext)?;
        let ext_size = u64::from_be_bytes(ext);
        if ext_size < 16 {
            return Err(LiteError::Parse("extended box size < 16".into()));
        }
        (offset + 16, ext_size - 16)
    } else if size == 0 {
        // Box extends to end of file.
        let end = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(offset + 8))?;
        (offset + 8, end - offset - 8)
    } else {
        if size < 8 {
            return Err(LiteError::Parse("box size < 8".into()));
        }
        (offset + 8, size - 8)
    };

    Ok(Some(BoxHeader {
        box_type,
        content_size,
        content_offset,
    }))
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
    r.seek(SeekFrom::Start(start))?;
    let mut pos = start;

    while pos < end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };

        let box_end = header.content_offset + header.content_size;
        if box_end > end {
            break;
        }

        let bt = header.box_type;

        if CONTAINER_BOXES.contains(&bt) {
            parse_boxes(r, header.content_offset, box_end, video, audio)?;
        } else if bt == *b"stsd" {
            // Skip 8-byte fullbox header (version + flags + entry_count).
            parse_stsd(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stts" {
            parse_stts(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"ctts" {
            parse_ctts(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stsc" {
            parse_stsc(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stsz" {
            parse_stsz(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stco" {
            parse_stco(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"co64" {
            parse_co64(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stss" {
            parse_stss(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"mdhd" {
            parse_mdhd(r, header.content_offset, header.content_size, video, audio)?;
        }

        pos = box_end;
    }

    Ok(())
}

/// Get a mutable reference to the "current" (last-added) track being built.
///
/// The most recently created track with an empty sample table is the one
/// currently being parsed. If both have data, prefer audio (added last).
pub(super) fn current_track_mut<'a>(
    video: &'a mut Option<TrackInfo>,
    audio: &'a mut Option<TrackInfo>,
) -> Option<&'a mut TrackInfo> {
    let audio_empty = audio.as_ref().is_some_and(|a| {
        a.table.stsz.is_empty()
            && a.table.stts.is_empty()
            && a.table.stco.is_empty()
            && a.table.stsc.is_empty()
    });
    if audio_empty {
        return audio.as_mut();
    }

    let video_empty = video.as_ref().is_some_and(|v| {
        v.table.stsz.is_empty()
            && v.table.stts.is_empty()
            && v.table.stco.is_empty()
            && v.table.stsc.is_empty()
    });
    if video_empty {
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

    let entries_end = offset + size;

    for _ in 0..entry_count {
        let pos = r.stream_position()?;
        if pos >= entries_end {
            break;
        }
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };

        let codec = header.box_type;
        if codec == *b"avc1" || codec == *b"avc3" {
            // H.264 video track.
            let avcc = parse_avc_sample_entry(r, header.content_offset, header.content_size)?;
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
            let aac = parse_mp4a_sample_entry(r, header.content_offset, header.content_size)?;
            if audio.is_none() {
                *audio = Some(TrackInfo {
                    kind: TrackKind::Audio,
                    table: SampleTable::default(),
                    avcc: None,
                    aac_config: aac,
                });
            }
        }

        let box_end = header.content_offset + header.content_size;
        r.seek(SeekFrom::Start(box_end))?;
    }

    Ok(())
}

/// Parse avc1/avc3 sample entry to extract the avcC box.
fn parse_avc_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Option<AvccConfig>, LiteError> {
    // avc1 sample entry: 6 reserved + 2 data_ref_idx + 16 pre-defined + 2 w + 2 h + ...
    // The avcC sub-box is somewhere inside. Scan for it.
    let end = offset + size;
    // Skip the fixed 78-byte visual sample entry header to find sub-boxes.
    let sub_start = offset + 78;
    if sub_start >= end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(sub_start))?;

    let mut pos = sub_start;
    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if header.box_type == *b"avcC" {
            return parse_avcc_box(r, header.content_offset, header.content_size).map(Some);
        }
        pos = header.content_offset + header.content_size;
    }
    Ok(None)
}

/// Parse the avcC box contents.
fn parse_avcc_box<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<AvccConfig, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    if data.len() < 7 {
        return Err(LiteError::Parse("avcC too short".into()));
    }

    let nal_length_size = (usize::from(data[4]) & 0x03) + 1;
    let num_sps = usize::from(data[5]) & 0x1F;
    let mut pos = 6;
    let mut sps = Vec::new();
    let mut pps = Vec::new();

    for _ in 0..num_sps {
        if pos + 2 > data.len() {
            return Err(LiteError::Parse("avcC SPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + len > data.len() {
            return Err(LiteError::Parse("avcC SPS data truncated".into()));
        }
        sps.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }

    if pos >= data.len() {
        return Err(LiteError::Parse("avcC PPS count missing".into()));
    }
    let num_pps = usize::from(data[pos]);
    pos += 1;

    for _ in 0..num_pps {
        if pos + 2 > data.len() {
            return Err(LiteError::Parse("avcC PPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + len > data.len() {
            return Err(LiteError::Parse("avcC PPS data truncated".into()));
        }
        pps.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }

    Ok(AvccConfig {
        nal_length_size,
        sps,
        pps,
    })
}

/// Parse mp4a sample entry to extract AAC config from esds.
fn parse_mp4a_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Option<AacConfig>, LiteError> {
    // mp4a: 6 reserved + 2 data_ref_idx + 8 reserved + 2 channels + 2 sample_size
    //       + 2 compression_id + 2 packet_size + 4 sample_rate(fixed16.16)
    // Total fixed header: 28 bytes, then sub-boxes (esds).
    let end = offset + size;
    if offset + 28 > end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(offset + 8))?; // skip reserved + data_ref_idx
    let mut hdr = [0u8; 20];
    r.read_exact(&mut hdr)?;
    let channels = u16::from_be_bytes([hdr[0], hdr[1]]); // offset 8 in entry
    // sample_rate is at offset 16 in the 20-byte header, as fixed-point 16.16
    let sr_fixed = u32::from_be_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    let sample_rate = sr_fixed >> 16;

    // Now look for esds sub-box.
    let sub_start = offset + 28;
    let mut pos = sub_start;
    let mut config_data = Vec::new();

    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if header.box_type == *b"esds" {
            config_data = parse_esds_for_asc(r, header.content_offset, header.content_size)?;
            break;
        }
        pos = header.content_offset + header.content_size;
    }

    Ok(Some(AacConfig {
        sample_rate,
        channels,
        config_data,
    }))
}

/// Extract AudioSpecificConfig from esds box.
fn parse_esds_for_asc<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Vec<u8>, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    // Skip version+flags (4 bytes), then scan for DecoderConfigDescriptor
    // tag (0x04) -> DecoderSpecificInfo tag (0x05) -> ASC bytes.
    // This is a simplified scan -- real esds parsing would walk descriptors.
    let mut i = 4; // skip version+flags
    while i < data.len() {
        if data[i] == 0x05 {
            // DecoderSpecificInfo tag.
            i += 1;
            // Read variable-length size.
            let (len, consumed) = read_desc_len(&data[i..]);
            i += consumed;
            if i + len <= data.len() {
                return Ok(data[i..i + len].to_vec());
            }
        }
        i += 1;
    }
    Ok(Vec::new())
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

/// Parse mdhd box for timescale.
fn parse_mdhd<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let version_flags = read_u32_be(r)?;
    let version = version_flags >> 24;

    let timescale = if version == 0 {
        // Skip creation_time(4) + modification_time(4).
        let mut skip = [0u8; 8];
        r.read_exact(&mut skip)?;
        read_u32_be(r)?
    } else {
        // v1: skip creation_time(8) + modification_time(8).
        let mut skip = [0u8; 16];
        r.read_exact(&mut skip)?;
        read_u32_be(r)?
    };

    if let Some(track) = current_track_mut(video, audio) {
        track.table.timescale = timescale;
    }
    Ok(())
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
}
