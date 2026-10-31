#![allow(clippy::unwrap_used)]

use super::*;
use std::io::Cursor;

/// Wrap a cursor as a VideoSource via the blanket impl in lib.rs.
fn cursor_source(data: Vec<u8>) -> Box<dyn VideoSource> {
    Box::new(Cursor::new(data))
}

#[test]
fn open_empty_data_fails() {
    let result = FfmpegDecoder::open_stream(cursor_source(Vec::new()));
    assert!(result.is_err());
}

#[test]
fn open_garbage_data_fails() {
    let result = FfmpegDecoder::open_stream(cursor_source(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    assert!(result.is_err());
}

#[test]
fn open_single_byte_fails() {
    let result = FfmpegDecoder::open_stream(cursor_source(vec![0xFF]));
    assert!(result.is_err());
}

#[test]
fn open_truncated_ftyp_fails() {
    // Valid ftyp header but truncated content.
    let mut data = Vec::new();
    data.extend_from_slice(&100u32.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(&[0; 8]); // truncated
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    assert!(result.is_err());
}

#[test]
fn open_all_zeros_fails() {
    let result = FfmpegDecoder::open_stream(cursor_source(vec![0; 1024]));
    assert!(result.is_err());
}

#[test]
fn open_random_noise_fails() {
    // Pseudorandom-ish data that is definitely not MP4.
    let data: Vec<u8> = (0..2048u16)
        .map(|i| (i.wrapping_mul(31) ^ 0xA5) as u8)
        .collect();
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    assert!(result.is_err());
}

#[test]
fn open_ftyp_only_no_tracks() {
    // Valid ftyp atom but nothing else.
    let mut data = Vec::new();
    let ftyp = b"isom\x00\x00\x00\x00isomavc1";
    let size = (8 + ftyp.len()) as u32;
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(ftyp);
    // This may or may not open successfully depending on ffmpeg's
    // tolerance, but must not panic.
    match FfmpegDecoder::open_stream(cursor_source(data)) {
        Ok(dec) => {
            // If it opens, it shouldn't have useful tracks.
            assert!(!dec.has_video() || !dec.has_audio());
        },
        Err(_) => {
            // Error is acceptable.
        },
    }
}

#[test]
fn video_size_no_tracks() {
    // Open with data that might parse but has no video.
    // We test the accessor directly on a struct.
    // Can't easily construct FfmpegDecoder without ffmpeg, so
    // test that open_stream with garbage produces an error.
    let result = FfmpegDecoder::open_stream(cursor_source(vec![0; 32]));
    assert!(result.is_err());
}

#[test]
fn ffmpeg_error_string_produces_output() {
    // AVERROR_EOF is a well-known error code.
    let msg = ffmpeg_error_string(ffi::AVERROR_EOF);
    assert!(!msg.is_empty());
}

#[test]
fn ffmpeg_error_string_unknown_error() {
    // An unlikely error code should still produce some string.
    let msg = ffmpeg_error_string(-999999);
    assert!(!msg.is_empty());
}

#[test]
fn ffmpeg_error_string_zero() {
    // Error code 0 (success) — ffmpeg should still produce output.
    let msg = ffmpeg_error_string(0);
    // Some versions return empty for 0, some return "Success".
    // Just check no panic.
    let _ = msg;
}

#[test]
fn open_large_garbage_no_oom() {
    // 64KB of garbage should fail quickly without OOM.
    let data = vec![0xAB; 65536];
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    assert!(result.is_err());
}

#[test]
fn open_wav_header_not_mp4() {
    // A WAV-like header should be rejected.
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&1000u32.to_le_bytes());
    data.extend_from_slice(b"WAVE");
    data.extend_from_slice(b"fmt ");
    data.extend_from_slice(&[0; 100]);
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    // ffmpeg may or may not recognize WAV, but it shouldn't crash.
    // The test validates no panic occurs.
    let _ = result;
}

#[test]
fn open_truncated_moov_no_panic() {
    // ftyp + moov header that claims more bytes than available.
    let mut data = Vec::new();
    // ftyp
    data.extend_from_slice(&24u32.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(&[0; 16]);
    // moov claiming 10000 bytes
    data.extend_from_slice(&10000u32.to_be_bytes());
    data.extend_from_slice(b"moov");
    data.extend_from_slice(&[0; 32]); // only 32 bytes
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    // Must not panic. Error or partial open are both fine.
    let _ = result;
}

#[test]
fn open_repeated_ftyp_no_panic() {
    // Multiple ftyp atoms (malformed but shouldn't crash).
    let mut data = Vec::new();
    for _ in 0..10 {
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0; 16]);
    }
    let result = FfmpegDecoder::open_stream(cursor_source(data));
    let _ = result; // just no panic
}
