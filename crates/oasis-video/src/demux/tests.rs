use super::*;

/// One AVCC packet as Annex B (the conversion `next_packet` applies).
fn avcc_to_annex_b(data: &[u8], nal_length_size: usize) -> Result<Vec<u8>, VideoError> {
    let mut out = Vec::new();
    append_annex_b(data, nal_length_size, &mut out)
        .map_err(|e| VideoError::Demux(e.to_string()))?;
    Ok(out)
}

#[test]
fn parse_avcc_extracts_sps_pps() {
    // Minimal avcC: version=1, profile=66(baseline), compat=0xC0, level=30,
    // nal_length_size=4 (0xFF & 0x03 = 3, +1=4),
    // 1 SPS of 4 bytes, 1 PPS of 3 bytes.
    #[rustfmt::skip]
    let avcc_data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
        0xFF,                     // 6 reserved bits + nal_length_size_minus_one=3
        0xE1,                     // 3 reserved bits + num_sps=1
        0x00, 0x04,               // SPS length = 4
        0x67, 0x42, 0xC0, 0x1E,  // SPS NAL data
        0x01,                     // num_pps = 1
        0x00, 0x03,               // PPS length = 3
        0x68, 0xCE, 0x38,        // PPS NAL data
    ];
    let config = parse_avcc(avcc_data).unwrap();
    assert_eq!(config.nal_length_size, 4);
    // parameter_sets should be: [start_code][SPS][start_code][PPS]
    let expected: Vec<u8> = [
        &ANNEX_B_START_CODE[..],
        &[0x67, 0x42, 0xC0, 0x1E],
        &ANNEX_B_START_CODE[..],
        &[0x68, 0xCE, 0x38],
    ]
    .concat();
    assert_eq!(config.parameter_sets, expected);
}

#[test]
fn parse_avcc_rejects_short_data() {
    assert!(parse_avcc(&[0x01, 0x42, 0xC0]).is_err());
}

#[test]
fn avcc_to_annex_b_converts_4byte_lengths() {
    // Two NALs: length=3 + data, length=2 + data.
    #[rustfmt::skip]
    let avcc_packet: &[u8] = &[
        0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC, // NAL 1
        0x00, 0x00, 0x00, 0x02, 0xDD, 0xEE,        // NAL 2
    ];
    let annex_b = avcc_to_annex_b(avcc_packet, 4).unwrap();
    let expected: Vec<u8> = [
        &ANNEX_B_START_CODE[..],
        &[0xAA, 0xBB, 0xCC],
        &ANNEX_B_START_CODE[..],
        &[0xDD, 0xEE],
    ]
    .concat();
    assert_eq!(annex_b, expected);
}

#[test]
fn avcc_to_annex_b_truncated_nal() {
    // Length says 10 bytes but only 2 available.
    let bad = &[0x00, 0x00, 0x00, 0x0A, 0xAA, 0xBB];
    assert!(avcc_to_annex_b(bad, 4).is_err());
}

#[test]
fn find_avcc_in_mp4_locates_box() {
    // Fake a minimal MP4 with an avcC box embedded.
    let avcc_contents: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, 0x00, 0x02, 0x67, 0x42, 0x01, 0x00, 0x01, 0x68,
    ];
    let box_size = (8 + avcc_contents.len()) as u32;
    let mut mp4 = Vec::new();
    mp4.extend_from_slice(b"JUNK"); // leading garbage
    mp4.extend_from_slice(&box_size.to_be_bytes());
    mp4.extend_from_slice(b"avcC");
    mp4.extend_from_slice(avcc_contents);
    mp4.extend_from_slice(b"TAIL"); // trailing data

    let config = find_avcc_in_mp4(&mp4).expect("should find avcC box");
    assert_eq!(config.nal_length_size, 4);
    assert!(!config.parameter_sets.is_empty());
}

#[test]
fn find_avcc_returns_none_for_no_avcc() {
    let garbage = b"this is not an mp4 file at all";
    assert!(find_avcc_in_mp4(garbage).is_none());
}

#[test]
fn avcc_to_annex_b_huge_nal_length_no_overflow() {
    // Craft a packet where the 4-byte NAL length declares 0xFFFFFFFF bytes.
    // The bounds check must not panic from integer overflow in offset+nal_len.
    let bad = &[0xFF, 0xFF, 0xFF, 0xFF, 0xAA, 0xBB];
    assert!(avcc_to_annex_b(bad, 4).is_err());
}

#[test]
fn avcc_to_annex_b_empty_input() {
    let empty: &[u8] = &[];
    let result = avcc_to_annex_b(empty, 4).unwrap();
    assert!(result.is_empty());
}

// ---------------------------------------------------------------
// Item 71: Video demux error path tests (truncated headers,
// missing atoms, edge cases)
// ---------------------------------------------------------------

#[test]
fn parse_avcc_exactly_7_bytes_minimal() {
    // 7 bytes is the minimum length check. Build a valid-ish header
    // with 0 SPS entries and 0 PPS entries.
    let data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
        0xFF, // nal_length_size_minus_one = 3 -> size = 4
        0xE0, // num_sps = 0
        0x00, // num_pps = 0
    ];
    let config = parse_avcc(data).expect("should parse minimal avcC");
    assert_eq!(config.nal_length_size, 4);
    assert!(config.parameter_sets.is_empty());
}

#[test]
fn parse_avcc_sps_length_exceeds_data() {
    // num_sps = 1, SPS length = 100 but only 2 bytes of data follow.
    let data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
        0x00, 0x64, // SPS length = 100
        0x67, 0x42, // only 2 bytes
    ];
    assert!(parse_avcc(data).is_err());
}

#[test]
fn parse_avcc_pps_length_exceeds_data() {
    // Valid SPS of 2 bytes, then PPS claims 50 bytes but only 1 available.
    let data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
        0x00, 0x02, // SPS length = 2
        0x67, 0x42, // SPS data
        0x01, // num_pps = 1
        0x00, 0x32, // PPS length = 50
        0x68, // only 1 byte of PPS
    ];
    assert!(parse_avcc(data).is_err());
}

#[test]
fn parse_avcc_pps_count_missing() {
    // Valid SPS but data ends before PPS count byte.
    let data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, // num_sps=1
        0x00, 0x02, // SPS length = 2
        0x67, 0x42, // SPS data — ends here, no PPS count
    ];
    assert!(parse_avcc(data).is_err());
}

#[test]
fn parse_avcc_multiple_sps_pps() {
    // 2 SPS entries and 2 PPS entries.
    #[rustfmt::skip]
    let data: &[u8] = &[
        0x01, 0x42, 0xC0, 0x1E, // version, profile, compat, level
        0xFF,                     // nal_length_size = 4
        0xE2,                     // num_sps = 2
        0x00, 0x02, 0x67, 0x42,  // SPS 1 (2 bytes)
        0x00, 0x01, 0x68,        // SPS 2 (1 byte)
        0x02,                     // num_pps = 2
        0x00, 0x01, 0xAA,        // PPS 1 (1 byte)
        0x00, 0x02, 0xBB, 0xCC,  // PPS 2 (2 bytes)
    ];
    let config = parse_avcc(data).expect("should parse multi-SPS/PPS");
    assert_eq!(config.nal_length_size, 4);
    // 2 SPS + 2 PPS = 4 start codes (4 bytes each) + data
    let expected_len = 4 * 4 + 2 + 1 + 1 + 2;
    assert_eq!(config.parameter_sets.len(), expected_len);
}

#[test]
fn avcc_to_annex_b_1byte_nal_length() {
    // Single NAL with 1-byte length prefix: length=3, data=0xAA,0xBB,0xCC
    let data: &[u8] = &[0x03, 0xAA, 0xBB, 0xCC];
    let result = avcc_to_annex_b(data, 1).expect("should convert");
    let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xAA, 0xBB, 0xCC]].concat();
    assert_eq!(result, expected);
}

#[test]
fn avcc_to_annex_b_2byte_nal_length() {
    // Single NAL with 2-byte length prefix: length=2, data=0xDD,0xEE
    let data: &[u8] = &[0x00, 0x02, 0xDD, 0xEE];
    let result = avcc_to_annex_b(data, 2).expect("should convert");
    let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xDD, 0xEE]].concat();
    assert_eq!(result, expected);
}

#[test]
fn avcc_to_annex_b_3byte_nal_length() {
    // Single NAL with 3-byte length prefix: length=1, data=0xFF
    let data: &[u8] = &[0x00, 0x00, 0x01, 0xFF];
    let result = avcc_to_annex_b(data, 3).expect("should convert");
    let expected: Vec<u8> = [&ANNEX_B_START_CODE[..], &[0xFF]].concat();
    assert_eq!(result, expected);
}

#[test]
fn avcc_to_annex_b_invalid_nal_length_size() {
    let data: &[u8] = &[0x00; 8];
    assert!(avcc_to_annex_b(data, 5).is_err());
    assert!(avcc_to_annex_b(data, 0).is_err());
}

#[test]
fn avcc_to_annex_b_zero_length_nal() {
    // NAL length = 0 should produce just a start code with empty data.
    let data: &[u8] = &[0x00, 0x00, 0x00, 0x00];
    let result = avcc_to_annex_b(data, 4).expect("should handle zero-len");
    assert_eq!(result, ANNEX_B_START_CODE);
}

#[test]
fn avcc_to_annex_b_trailing_partial_length() {
    // Data has 3 bytes but nal_length_size = 4, so the trailing bytes
    // are too short for another NAL length. Should produce empty output
    // (loop condition `offset + nal_length_size <= data.len()` fails).
    let data: &[u8] = &[0x00, 0x00, 0x03];
    let result = avcc_to_annex_b(data, 4).expect("should succeed");
    assert!(result.is_empty());
}

#[test]
fn find_avcc_in_mp4_with_zero_size_box() {
    // avcC box with size = 0 (invalid for sub-box). Should skip.
    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&0u32.to_be_bytes());
    mp4.extend_from_slice(b"avcC");
    mp4.extend_from_slice(&[0; 20]);
    assert!(find_avcc_in_mp4(&mp4).is_none());
}

#[test]
fn find_avcc_in_mp4_truncated_box_size() {
    // avcC fourcc found but box extends beyond data.
    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&500u32.to_be_bytes()); // claims 500 bytes
    mp4.extend_from_slice(b"avcC");
    mp4.extend_from_slice(&[0; 10]); // only 10 bytes of content
    assert!(find_avcc_in_mp4(&mp4).is_none());
}

#[test]
fn find_avcc_in_mp4_data_too_short_for_scan() {
    // Less than 8 bytes — loop never executes.
    assert!(find_avcc_in_mp4(b"short").is_none());
    assert!(find_avcc_in_mp4(b"").is_none());
}

#[test]
fn demux_open_single_byte_fails() {
    let result = Mp4Demuxer::open(vec![0xFF]);
    assert!(result.is_err());
}

#[test]
fn demux_open_ftyp_only_no_audio_no_video() {
    // Valid ftyp atom but no tracks.
    let mut data = Vec::new();
    let ftyp = b"isom\x00\x00\x00\x00isomavc1";
    let size = (8 + ftyp.len()) as u32;
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(ftyp);
    // Symphonia should either error or find no tracks.
    match Mp4Demuxer::open(data) {
        Ok(demuxer) => {
            assert!(
                !demuxer.has_video() || !demuxer.has_audio(),
                "ftyp-only should not have both tracks"
            );
        },
        Err(_) => {
            // Error on open is acceptable for data with no mdat/moov.
        },
    }
}

fn streaming_fixture(name: &str) -> Vec<u8> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read(manifest.join("../../tests/fixtures/streaming").join(name))
        .expect("fixture file missing")
}

#[test]
fn read_top_level_atom_finds_moov_at_end() {
    let data = streaming_fixture("moov_end_4s.mp4");
    let expected = find_top_level_atom(&data, b"moov").expect("moov").to_vec();
    let mut cursor = Cursor::new(&data);
    let moov = read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap();
    assert_eq!(moov, Some(expected));
    // Over the size cap: not loaded.
    let mut cursor = Cursor::new(&data);
    assert_eq!(read_top_level_atom(&mut cursor, b"moov", 64).unwrap(), None);
    // Garbage / absent.
    let mut cursor = Cursor::new(vec![0u8; 3]);
    assert_eq!(
        read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap(),
        None
    );
}

#[test]
fn read_top_level_atom_rejects_a_moov_larger_than_the_file() {
    // A 16-byte file declaring a 60 MiB `moov`: an error, and no
    // up-front allocation of the declared size.
    let mut data = (60u32 << 20).to_be_bytes().to_vec();
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0; 8]);
    let mut cursor = Cursor::new(data);
    let err = read_top_level_atom(&mut cursor, b"moov", MAX_MOOV_BYTES).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn parse_avcc_keeps_every_parameter_set_in_order() {
    #[rustfmt::skip]
    let data: &[u8] = &[
        0x01, 0x64, 0x00, 0x1F, 0xFF,
        0xE2, 0, 1, 0x67, 0, 2, 0x67, 0x01, // 2 SPS
        0x01, 0, 1, 0x68,                   // 1 PPS
    ];
    let cfg = parse_avcc(data).unwrap();
    assert_eq!(
        cfg.parameter_sets,
        [0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x67, 0x01, 0, 0, 0, 1, 0x68]
    );
    for len in 0..data.len() {
        assert!(parse_avcc(&data[..len]).is_err(), "prefix {len}");
    }
}

#[test]
fn find_avcc_ignores_decoys_outside_moov() {
    // Regression: the whole-file byte scan could take `avcC`-looking
    // bytes in `mdat` (before a moov-at-end) for the real config.
    #[rustfmt::skip]
    let decoy: &[u8] = &[
        0, 0, 0, 0x12, b'a', b'v', b'c', b'C',
        0x01, 0x42, 0xC0, 0x1E, 0xFC, 0xE1, 0x00, 0x01, 0x67, 0x00,
    ];
    let real = streaming_fixture("moov_end_4s.mp4");
    let moov = find_top_level_atom(&real, b"moov").expect("moov");
    let mut mdat = vec![0, 0, 0, 0, b'm', b'd', b'a', b't'];
    mdat.extend_from_slice(decoy);
    let mdat_len = mdat.len() as u32;
    mdat[..4].copy_from_slice(&mdat_len.to_be_bytes());
    let mut file = mdat;
    file.extend_from_slice(moov);
    let cfg = find_avcc_in_mp4(&file).expect("avcC");
    let real_cfg = find_avcc_in_mp4(moov).expect("avcC");
    assert_eq!(cfg.parameter_sets, real_cfg.parameter_sets);
    assert_eq!(cfg.nal_length_size, 4);
}

#[test]
fn seek_with_hostile_times_does_not_panic() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read(manifest.join("../../tests/fixtures/test_320x240_2s.mp4"))
        .expect("fixture file missing");
    let mut demuxer = Mp4Demuxer::open(data).expect("open");
    for secs in [-3.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.25, 1e30] {
        let _ = demuxer.seek(secs);
        let _ = demuxer.next_packet();
    }
}
