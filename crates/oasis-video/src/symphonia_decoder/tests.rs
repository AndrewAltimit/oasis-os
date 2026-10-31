use super::*;
use crate::demux::find_top_level_atom;

fn fixture_bytes() -> Vec<u8> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read(manifest.join("../../tests/fixtures/test_320x240_2s.mp4"))
        .expect("fixture file missing")
}

#[test]
fn find_top_level_atom_walks_boxes() {
    let data = fixture_bytes();
    let moov = find_top_level_atom(&data, b"moov").expect("moov");
    assert_eq!(&moov[4..8], b"moov");
    assert_eq!(moov.len(), 2235);
    assert!(find_top_level_atom(&data, b"zzzz").is_none());
    assert!(find_top_level_atom(&[0, 0, 0, 3, b'b', b'a', b'd', b'!'], b"moov").is_none());
}

#[test]
fn other_stream_queue_is_bounded() {
    // Regression: reading only audio from a file with video buffered
    // every video packet of the file.
    let mut dec = SymphoniaDecoder::open_stream(Box::new(std::io::Cursor::new(fixture_bytes())))
        .expect("open");
    for i in 0..MAX_QUEUED_PACKETS + 100 {
        for kind in [TrackKind::Video, TrackKind::Audio] {
            dec.buffer_packet(DemuxedPacket {
                kind,
                data: vec![0; 4],
                timestamp_secs: i as f64,
            });
        }
    }
    assert_eq!(dec.video_queue.len(), MAX_QUEUED_PACKETS);
    assert_eq!(dec.audio_queue.len(), MAX_QUEUED_PACKETS);
    // The oldest packets went first.
    assert_eq!(
        dec.audio_queue.front().map(|p| p.timestamp_secs),
        Some(100.0)
    );
}

#[test]
fn open_stream_reads_keyframes_without_buffering_the_file() {
    // `open_stream` walks top-level boxes and reads only `moov`.
    let dec = SymphoniaDecoder::open_stream(Box::new(std::io::Cursor::new(fixture_bytes())))
        .expect("open");
    assert_eq!(dec.keyframes.len(), 2);
}

// ---------------------------------------------------------------
// Item 76: H.264 NAL unit / SPS/PPS / IDR detection tests
// (contains_idr is defined here, gated on feature = "h264")
// ---------------------------------------------------------------

#[cfg(feature = "h264")]
mod nal_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn contains_idr_4byte_start_code() {
        // IDR NAL type = 5 (0x65 = 0b01100101, type = 5)
        let data = [0x00, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB];
        assert!(SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_3byte_start_code() {
        // 3-byte start code with IDR NAL type 5
        let data = [0x00, 0x00, 0x01, 0x65, 0xAA];
        assert!(SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_non_idr_nal() {
        // NAL type 1 (non-IDR coded slice, 0x41 & 0x1F = 1)
        let data = [0x00, 0x00, 0x00, 0x01, 0x41, 0xAA];
        assert!(!SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_sps_not_idr() {
        // SPS NAL type = 7 (0x67 & 0x1F = 7)
        let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0];
        assert!(!SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_pps_not_idr() {
        // PPS NAL type = 8 (0x68 & 0x1F = 8)
        let data = [0x00, 0x00, 0x00, 0x01, 0x68, 0xCE];
        assert!(!SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_empty_data() {
        assert!(!SymphoniaDecoder::contains_idr(&[]));
    }

    #[test]
    fn contains_idr_too_short_for_start_code() {
        assert!(!SymphoniaDecoder::contains_idr(&[0x00, 0x00]));
        assert!(!SymphoniaDecoder::contains_idr(&[0x00, 0x00, 0x01]));
    }

    #[test]
    fn contains_idr_multiple_nals_idr_second() {
        // SPS then IDR
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]); // SPS
        data.extend_from_slice(&[0x42, 0xC0, 0x1E]); // SPS data
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x65]); // IDR
        data.extend_from_slice(&[0xAA, 0xBB]); // IDR data
        assert!(SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_idr_with_nal_ref_idc() {
        // NAL byte = 0x25: nal_ref_idc=1, type=5 (IDR)
        let data = [0x00, 0x00, 0x00, 0x01, 0x25, 0xAA];
        assert!(SymphoniaDecoder::contains_idr(&data));
    }

    #[test]
    fn contains_idr_start_code_at_end_no_nal_byte() {
        // Start code is the last bytes, no NAL type byte follows.
        let data = [0x00, 0x00, 0x00, 0x01];
        assert!(!SymphoniaDecoder::contains_idr(&data));
    }
}
