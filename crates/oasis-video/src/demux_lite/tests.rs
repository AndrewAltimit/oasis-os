use super::*;
use std::io::Cursor;

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/test_320x240_2s.mp4")
}

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(fixture_path()).expect("fixture file missing")
}

#[test]
fn open_fixture_finds_tracks() {
    let data = fixture_bytes();
    let mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
    assert!(mp4.video_track_info().is_some(), "should find video track");
    assert!(mp4.audio_track_info().is_some(), "should find audio track");

    let vt = mp4.video_track_info().unwrap();
    assert!(vt.avcc.is_some(), "video track should have avcC");
    assert!(vt.table.timescale > 0);
    assert!(!vt.table.stsz.is_empty());

    let at = mp4.audio_track_info().unwrap();
    assert!(
        at.aac_config.is_some(),
        "audio track should have AAC config"
    );
}

#[test]
fn read_all_video_samples() {
    let data = fixture_bytes();
    let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
    let mut count = 0;
    let mut last_ts = -1.0f64;
    while let Some(sample) = mp4.next_video_sample().expect("read error") {
        count += 1;
        assert_eq!(sample.kind, TrackKind::Video);
        assert!(!sample.data.is_empty());
        assert!(sample.timestamp_secs >= last_ts, "timestamp went backwards");
        last_ts = sample.timestamp_secs;
    }
    // The fixture is ~2s at ~15fps, so expect at least 10 samples.
    assert!(count >= 10, "expected >=10 video samples, got {count}");
}

#[test]
fn seek_to_midstream() {
    let data = fixture_bytes();
    let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
    mp4.seek(1.0).expect("seek failed");
    let sample = mp4
        .next_video_sample()
        .expect("read error")
        .expect("no sample");
    // Should be near 1.0s (within 0.5s tolerance).
    assert!(
        (sample.timestamp_secs - 1.0).abs() < 0.5,
        "post-seek timestamp {} not near 1.0s",
        sample.timestamp_secs
    );
}

#[test]
fn parse_moov_tracks_too_short() {
    let result = parse_moov_tracks(&[0; 4]);
    assert!(result.is_err());
}

#[test]
fn parse_moov_tracks_empty_moov() {
    // 8-byte moov header, no children.
    let mut data = Vec::new();
    data.extend_from_slice(&8u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    let (video, audio) = parse_moov_tracks(&data).unwrap();
    assert!(video.is_none());
    assert!(audio.is_none());
}

#[test]
fn seek_point_from_moov_empty_returns_none() {
    let mut data = Vec::new();
    data.extend_from_slice(&8u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    assert!(seek_point_from_moov(&data, 5.0).is_none());
}

#[test]
fn truncated_moov_atom() {
    // A moov atom whose declared size extends beyond the available data.
    // The moov header says 200 bytes total, but we only provide 20.
    let mut data = Vec::new();
    // ftyp box (valid, 24 bytes)
    data.extend_from_slice(&24u32.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(&[0u8; 16]);
    // moov box header claiming 200 bytes, but content is truncated
    data.extend_from_slice(&200u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0u8; 4]); // only 4 bytes of "content"

    // Should not panic. parse_boxes skips boxes whose end exceeds
    // the parent boundary, so moov is skipped and no tracks found.
    let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
    assert!(mp4.video_track_info().is_none());
    assert!(mp4.audio_track_info().is_none());
}

#[test]
fn truncated_moov_atom_via_parse_moov_tracks() {
    // A moov header that claims 200 bytes but only 20 bytes are present.
    // parse_moov_tracks should handle gracefully (no panic).
    let mut data = Vec::new();
    data.extend_from_slice(&200u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0u8; 12]); // truncated content
    let result = parse_moov_tracks(&data);
    // Should succeed but find no tracks (children are truncated/skipped).
    match result {
        Ok((v, a)) => {
            assert!(v.is_none());
            assert!(a.is_none());
        },
        Err(_) => {
            // An error is also acceptable -- just must not panic.
        },
    }
}

#[test]
fn zero_size_atom_parsing() {
    // An atom with size=0 means "extends to end of file".
    // Build: ftyp(24 bytes) + zero-size mdat.
    let mut data = Vec::new();
    // ftyp
    data.extend_from_slice(&24u32.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(&[0u8; 16]);
    // mdat with size=0 (extends to EOF)
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(b"mdat");
    data.extend_from_slice(&[0xAA; 50]); // payload

    // Should parse without panic. No moov, so no tracks.
    let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
    assert!(mp4.video_track_info().is_none());
    assert!(mp4.audio_track_info().is_none());
}

#[test]
fn empty_input_mp4lite() {
    // Parsing an empty byte slice should return Ok with no tracks,
    // not panic.
    let mp4 = Mp4Lite::open(Cursor::new(Vec::<u8>::new())).unwrap();
    assert!(mp4.video_track_info().is_none());
    assert!(mp4.audio_track_info().is_none());
}

#[test]
fn ftyp_only_no_moov() {
    // Just an ftyp atom with no moov. Should return no track info.
    let ftyp_content = b"isom\x00\x00\x00\x00isomavc1";
    let mut data = Vec::new();
    let size = (8 + ftyp_content.len()) as u32;
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(ftyp_content);

    let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
    assert!(
        mp4.video_track_info().is_none(),
        "ftyp-only file should have no video track"
    );
    assert!(
        mp4.audio_track_info().is_none(),
        "ftyp-only file should have no audio track"
    );
}

// ---------------------------------------------------------------
// AVCC conversion tests (delegated to avcc module but tested here
// for backward compatibility with the original test locations)
// ---------------------------------------------------------------

#[test]
fn avcc_to_annex_b_empty_data() {
    let avcc = AvccConfig {
        nal_length_size: 4,
        sps: vec![0x67, 0x42],
        pps: vec![0x68, 0xCE],
    };
    // Empty data, keyframe -> should produce SPS + PPS only.
    let result = avcc_to_annex_b(&[], &avcc, true).unwrap();
    // [0,0,0,1, 0x67,0x42, 0,0,0,1, 0x68,0xCE]
    assert_eq!(result.len(), 12);
    assert_eq!(&result[0..4], &[0, 0, 0, 1]);
    assert_eq!(&result[4..6], &[0x67, 0x42]);
    assert_eq!(&result[6..10], &[0, 0, 0, 1]);
    assert_eq!(&result[10..12], &[0x68, 0xCE]);
}

#[test]
fn avcc_to_annex_b_nal_exceeds_bounds() {
    let avcc = AvccConfig {
        nal_length_size: 4,
        sps: Vec::new(),
        pps: Vec::new(),
    };
    // NAL length says 1000 bytes but only 4 bytes of data follow.
    let mut data = Vec::new();
    data.extend_from_slice(&1000u32.to_be_bytes());
    data.extend_from_slice(&[0xAA; 4]);
    let result = avcc_to_annex_b(&data, &avcc, false);
    assert!(result.is_err());
}

#[test]
fn avcc_to_annex_b_invalid_nal_length_size() {
    let avcc = AvccConfig {
        nal_length_size: 5, // invalid
        sps: Vec::new(),
        pps: Vec::new(),
    };
    let data = vec![0; 8];
    let result = avcc_to_annex_b(&data, &avcc, false);
    assert!(result.is_err());
}

// ---------------------------------------------------------------
// Property-based tests (proptest)
// ---------------------------------------------------------------

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn avcc_to_annex_b_no_panic(
            data in proptest::collection::vec(any::<u8>(), 0..4096),
            nls in 1usize..=4,
        ) {
            let avcc = AvccConfig {
                nal_length_size: nls,
                sps: vec![],
                pps: vec![],
            };
            let _ = avcc_to_annex_b(&data, &avcc, true);
            let _ = avcc_to_annex_b(&data, &avcc, false);
        }
    }
}

// ---------------------------------------------------------------
// Item 71: Demux_lite error path tests (truncated/missing atoms)
// ---------------------------------------------------------------

#[test]
fn mp4lite_garbage_data_no_panic() {
    let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
    let mp4 = Mp4Lite::open(Cursor::new(garbage)).unwrap();
    assert!(mp4.video_track_info().is_none());
    assert!(mp4.audio_track_info().is_none());
}

#[test]
fn mp4lite_single_byte_no_panic() {
    let mp4 = Mp4Lite::open(Cursor::new(vec![0xFF])).unwrap();
    assert!(mp4.video_track_info().is_none());
}

#[test]
fn mp4lite_next_video_without_track_errors() {
    let mp4_data = vec![0u8; 0];
    let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
    let result = mp4.next_video_sample();
    assert!(result.is_err());
}

#[test]
fn mp4lite_next_audio_without_track_errors() {
    let mp4_data = vec![0u8; 0];
    let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
    let result = mp4.next_audio_sample();
    assert!(result.is_err());
}

#[test]
fn mp4lite_seek_no_tracks_ok() {
    let mp4_data = vec![0u8; 0];
    let mut mp4 = Mp4Lite::open(Cursor::new(mp4_data)).unwrap();
    // Seeking with no tracks should succeed (no-op).
    mp4.seek(5.0).unwrap();
}

#[test]
fn parse_moov_tracks_garbage_content() {
    // moov header + random bytes inside.
    let mut data = Vec::new();
    data.extend_from_slice(&64u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0xDE; 56]);
    let (v, a) = parse_moov_tracks(&data).unwrap();
    assert!(v.is_none());
    assert!(a.is_none());
}

#[test]
fn seek_point_from_moov_garbage_returns_none() {
    let mut data = Vec::new();
    data.extend_from_slice(&64u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0xFF; 56]);
    assert!(seek_point_from_moov(&data, 1.0).is_none());
}

#[test]
fn mp4lite_truncated_mdat_header_only() {
    // mdat atom with valid header but size extends past EOF.
    let mut data = Vec::new();
    data.extend_from_slice(&1000u32.to_be_bytes());
    data.extend_from_slice(b"mdat");
    data.extend_from_slice(&[0xAA; 8]); // only 8 bytes, not 992
    let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
    assert!(mp4.video_track_info().is_none());
}

#[test]
fn mp4lite_multiple_unknown_atoms_skipped() {
    // Several unknown atom types should be silently skipped.
    let mut data = Vec::new();
    for fourcc in [b"free", b"skip", b"udta", b"meta"] {
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(fourcc);
        data.extend_from_slice(&[0; 8]);
    }
    let mp4 = Mp4Lite::open(Cursor::new(data)).unwrap();
    assert!(mp4.video_track_info().is_none());
    assert!(mp4.audio_track_info().is_none());
}

// ---------------------------------------------------------------
// Per-track timescale + seek points (streaming seek regression)
// ---------------------------------------------------------------

/// The fixture's complete moov atom (header included).
fn fixture_moov() -> Vec<u8> {
    let data = fixture_bytes();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if &data[pos + 4..pos + 8] == b"moov" {
            return data[pos..pos + size].to_vec();
        }
        pos += size;
    }
    panic!("fixture has no moov");
}

#[test]
fn each_track_gets_its_own_mdhd_timescale() {
    // Regression: mdhd precedes stsd (which creates the track), so the
    // video trak's mdhd used to be dropped and the audio trak's mdhd
    // (44.1/48 kHz) was written onto the video track -- every video
    // timestamp and seek-byte estimate came out ~3x too small.
    let (video, audio) = parse_moov_tracks(&fixture_moov()).unwrap();
    assert_eq!(video.unwrap().timescale(), 15360);
    assert_eq!(audio.unwrap().timescale(), 22050);

    let mp4 = Mp4Lite::open(Cursor::new(fixture_bytes())).unwrap();
    assert_eq!(mp4.video_track_info().unwrap().timescale(), 15360);
    assert_eq!(mp4.audio_track_info().unwrap().timescale(), 22050);
}

#[test]
fn video_timestamps_span_the_file_duration() {
    let mp4 = Mp4Lite::open(Cursor::new(fixture_bytes())).unwrap();
    let vt = mp4.video_track_info().unwrap();
    let last = vt.sample_timestamp(vt.sample_count() - 1);
    // 30 frames at 15 fps: last frame starts at ~1.93s of a 2s clip.
    assert!((1.8..2.0).contains(&last), "last video ts {last}");
}

#[test]
fn keyframe_times_are_decode_start_and_mid_sample() {
    let (video, _) = parse_moov_tracks(&fixture_moov()).unwrap();
    let kf = video.unwrap().keyframe_times();
    // stss = [1, 16]; 1024-tick samples at 15360 Hz.
    assert_eq!(kf.len(), 2);
    assert!((kf[0].0 - 0.0).abs() < 1e-9);
    assert!((kf[1].0 - 1.0).abs() < 1e-9);
    assert!((kf[1].1 - (15.0 * 1024.0 + 512.0) / 15360.0).abs() < 1e-9);
}

#[test]
fn seek_point_lands_on_keyframe_at_or_before_target() {
    let moov = fixture_moov();
    let (video, audio) = parse_moov_tracks(&moov).unwrap();
    let (video, audio) = (video.unwrap(), audio.unwrap());

    // Mid-GOP request snaps back to the keyframe at 1.0s (sample 15).
    let p = seek_point_from_moov(&moov, 1.6).unwrap();
    assert_eq!(video.sample_at_decode_time(p.keyframe_secs), 15);
    assert!(video.sample_is_keyframe(15));
    let (key_off, _) = video.sample_offset_size(15).unwrap();
    let audio_idx = audio.sample_at_decode_time(p.keyframe_secs);
    let (audio_off, _) = audio.sample_offset_size(audio_idx).unwrap();
    assert_eq!(p.min_byte, key_off.min(audio_off));

    // Before the second keyframe -> the first one.
    let p = seek_point_from_moov(&moov, 0.9).unwrap();
    assert_eq!(video.sample_at_decode_time(p.keyframe_secs), 0);

    // Exactly on a keyframe stays on it.
    let p = seek_point_from_moov(&moov, 1.0).unwrap();
    assert_eq!(video.sample_at_decode_time(p.keyframe_secs), 15);
}

#[test]
fn seek_byte_never_past_the_first_packet_read() {
    // The streaming download restarts at this byte: every packet the
    // demuxer reads after seeking must be at or beyond it.
    let moov = fixture_moov();
    let (video, audio) = parse_moov_tracks(&moov).unwrap();
    let (video, audio) = (video.unwrap(), audio.unwrap());
    for tenths in 0..20 {
        let secs = f64::from(tenths) / 10.0;
        let p = seek_point_from_moov(&moov, secs).unwrap();
        let v = video.sample_at_decode_time(p.keyframe_secs);
        let a = audio.sample_at_decode_time(p.keyframe_secs);
        let (voff, _) = video.sample_offset_size(v).unwrap();
        let (aoff, _) = audio.sample_offset_size(a).unwrap();
        assert!(p.min_byte <= voff && p.min_byte <= aoff, "secs={secs}");
        assert_eq!(
            seek_point_from_moov(&moov, secs).map(|q| q.min_byte),
            Some(p.min_byte)
        );
    }
}

// ---------------------------------------------------------------
// Fuzz-like robustness: truncated and bit-flipped real files
// ---------------------------------------------------------------

fn streaming_fixtures() -> Vec<(String, Vec<u8>)> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/streaming");
    let mut out = vec![("test_320x240_2s.mp4".to_string(), fixture_bytes())];
    for entry in std::fs::read_dir(dir).expect("streaming fixtures missing") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "mp4") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&path).unwrap()));
        }
    }
    out
}

/// Drive every public entry point over `data`; must never panic, hang
/// or allocate wildly (errors are fine).
fn exercise(data: &[u8]) {
    if let Ok(mut mp4) = Mp4Lite::open(Cursor::new(data)) {
        for _ in 0..64 {
            if !matches!(mp4.next_video_sample(), Ok(Some(_))) {
                break;
            }
        }
        for _ in 0..64 {
            if !matches!(mp4.next_audio_sample(), Ok(Some(_))) {
                break;
            }
        }
        for secs in [-1.0, 0.0, 0.7, 1e12, f64::NAN, f64::INFINITY] {
            mp4.seek(secs).unwrap();
            let _ = mp4.next_video_sample();
            let _ = mp4.next_audio_sample();
        }
        for track in [mp4.video_track_info(), mp4.audio_track_info()]
            .into_iter()
            .flatten()
        {
            let n = track.sample_count();
            for idx in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
                let _ = track.sample_offset_size(idx);
                let _ = track.sample_timestamp(idx);
                let _ = track.keyframe_at_or_before(idx);
                let _ = track.sample_seek_time(idx);
            }
            let _ = track.keyframe_times();
            let _ = track.sample_at_decode_time(f64::MAX);
        }
    }
    let _ = seek_point_from_moov(data, 1.0);
}

/// The bytes of the moov atom (header included), if `data` has one.
fn moov_of(data: &[u8]) -> Option<&[u8]> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if size < 8 {
            return None;
        }
        if &data[pos + 4..pos + 8] == b"moov" {
            return data.get(pos..pos + size);
        }
        pos += size;
    }
    None
}

#[test]
fn every_fixture_reads_all_samples() {
    for (name, data) in streaming_fixtures() {
        let mut mp4 = Mp4Lite::open(Cursor::new(&data)).unwrap();
        let counts = (
            mp4.video_track_info().map_or(0, TrackInfo::sample_count),
            mp4.audio_track_info().map_or(0, TrackInfo::sample_count),
        );
        assert!(counts.0 + counts.1 > 0, "{name}: no samples");
        let mut read = (0, 0);
        if counts.0 > 0 {
            while mp4.next_video_sample().unwrap().is_some() {
                read.0 += 1;
            }
        }
        if counts.1 > 0 {
            while mp4.next_audio_sample().unwrap().is_some() {
                read.1 += 1;
            }
        }
        assert_eq!(read, counts, "{name}");
    }
}

#[test]
fn truncated_fixtures_never_panic() {
    for (_, data) in streaming_fixtures() {
        // Every prefix of the moov (the dense part), then coarser steps
        // over the whole file.
        if let Some(moov) = moov_of(&data) {
            for len in 0..moov.len() {
                let _ = parse_moov_tracks(&moov[..len]);
            }
        }
        for len in (0..data.len()).step_by(97) {
            exercise(&data[..len]);
        }
    }
}

#[test]
fn bit_flipped_fixtures_never_panic() {
    // Deterministic xorshift so failures reproduce.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for (_, data) in streaming_fixtures() {
        let Some(moov) = moov_of(&data) else { continue };
        let moov_start = moov.as_ptr() as usize - data.as_ptr() as usize;
        let moov_len = moov.len();
        for _ in 0..300 {
            let mut mutated = data.clone();
            for _ in 0..1 + next() % 4 {
                // Mostly hit the moov, where every byte is structure.
                let pos = if next() % 4 == 0 {
                    next() as usize % mutated.len()
                } else {
                    moov_start + next() as usize % moov_len
                };
                mutated[pos] ^= 1 << (next() % 8);
            }
            exercise(&mutated);
        }
    }
}
