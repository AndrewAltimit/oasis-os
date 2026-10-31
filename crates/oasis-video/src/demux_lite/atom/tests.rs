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
