//! End-to-end decode tests: real MP4 fixtures through `SoftwareVideoDecoder`,
//! driven the way the desktop player's decode thread drives it
//! (`next_video_frame` + `next_buffered_audio` after every frame), over
//! sources that behave like a network stream (short reads, slow reads,
//! truncation) as well as plain in-memory cursors.
//!
//! Gated on the openh264 (`h264`) backend: the exact frame/timestamp
//! expectations below are for that decoder. The ffmpeg backend (CI's
//! workspace build) has its own decode path and is not covered here.
#![cfg(feature = "h264")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use oasis_video::{SoftwareVideoDecoder, VideoError, VideoSource};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Expected properties of a fixture (from `ffprobe`).
struct Fixture {
    file: &'static str,
    width: u32,
    height: u32,
    fps: f64,
    frames: usize,
    duration: f64,
    /// `(sample_rate, channels)`, or `None` for a video-only file.
    audio: Option<(u32, u16)>,
    /// Whether every frame shows a different picture (testsrc2 fixtures);
    /// the original fixture is a still image.
    distinct_frames: bool,
}

const BASELINE_2S: Fixture = Fixture {
    file: "test_320x240_2s.mp4",
    width: 320,
    height: 240,
    fps: 15.0,
    frames: 30,
    duration: 2.0,
    audio: Some((22050, 1)),
    distinct_frames: false,
};

/// moov after mdat (not "faststart"), 1 s GOP, 44.1 kHz stereo.
const MOOV_END_4S: Fixture = Fixture {
    file: "streaming/moov_end_4s.mp4",
    width: 320,
    height: 240,
    fps: 24.0,
    frames: 96,
    duration: 4.0,
    audio: Some((44100, 2)),
    distinct_frames: true,
};

/// Main profile with 2 B-frames (reordered output), 48 kHz stereo.
const BFRAMES_4S: Fixture = Fixture {
    file: "streaming/bframes_4s.mp4",
    width: 320,
    height: 240,
    fps: 24.0,
    frames: 96,
    duration: 4.0,
    audio: Some((48000, 2)),
    distinct_frames: true,
};

const NOAUDIO_2S: Fixture = Fixture {
    file: "streaming/noaudio_2s.mp4",
    width: 320,
    height: 240,
    fps: 15.0,
    frames: 30,
    duration: 2.0,
    audio: None,
    distinct_frames: true,
};

/// Width/height not multiples of 16 (encoder crops the coded picture).
const ODD_2S: Fixture = Fixture {
    file: "streaming/odd_250x138_2s.mp4",
    width: 250,
    height: 138,
    fps: 20.0,
    frames: 40,
    duration: 2.0,
    audio: Some((22050, 1)),
    distinct_frames: true,
};

fn fixture_bytes(f: &Fixture) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(f.file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Run `f` on a worker thread and fail the test if it doesn't finish in
/// `secs` -- no test in this file may hang.
fn with_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => {
            let _ = handle.join();
            v
        },
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("test exceeded hard timeout of {secs}s (hang)")
        },
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("worker exited without a result"),
        },
    }
}

/// One decoded video frame.
#[derive(Debug, Clone)]
struct FrameInfo {
    pts: f64,
    width: u32,
    height: u32,
    hash: u64,
}

/// One decoded audio chunk.
#[derive(Debug, Clone)]
struct AudioInfo {
    ts: f64,
    /// Samples per channel.
    samples: usize,
    rate: u32,
    channels: u16,
}

#[derive(Debug, Default)]
struct Decoded {
    frames: Vec<FrameInfo>,
    audio: Vec<AudioInfo>,
    /// The error that ended decoding, if any (rather than clean EOF).
    error: Option<String>,
}

impl Decoded {
    fn audio_samples(&self) -> usize {
        self.audio.iter().map(|a| a.samples).sum()
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    // Sample every 7th byte: plenty to tell frames apart, cheap in debug.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data.iter().step_by(7) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn push_audio(out: &mut Decoded, chunk: oasis_video::AudioChunk) {
    let ch = chunk.channels.max(1) as usize;
    out.audio.push(AudioInfo {
        ts: chunk.timestamp_secs,
        samples: chunk.pcm_f32.len() / ch,
        rate: chunk.sample_rate,
        channels: chunk.channels,
    });
}

/// Decode to the end the way `VideoPlayer::decode_loop` does: one video
/// frame, then drain buffered audio; at video EOF drain the rest.  For a
/// file without video, pull audio with `next_audio_samples`.
fn decode_all(dec: &mut SoftwareVideoDecoder) -> Decoded {
    let mut out = Decoded::default();
    let mut skip_limits = 0;
    loop {
        match dec.next_video_frame() {
            Ok(Some(f)) => out.frames.push(FrameInfo {
                pts: f.timestamp_secs,
                width: f.width,
                height: f.height,
                hash: fnv1a(&f.rgba),
            }),
            Ok(None) => break,
            Err(VideoError::SkipLimit) if skip_limits < 3 => skip_limits += 1,
            Err(VideoError::NoTrack(_)) => {
                // Audio-only: advance the demuxer through audio.
                loop {
                    match dec.next_audio_samples() {
                        Ok(Some(c)) => push_audio(&mut out, c),
                        Ok(None) => break,
                        Err(e) => {
                            out.error = Some(e.to_string());
                            break;
                        },
                    }
                }
                return out;
            },
            Err(e) => {
                out.error = Some(e.to_string());
                break;
            },
        }
        while let Some(c) = dec.next_buffered_audio() {
            push_audio(&mut out, c);
        }
    }
    while let Some(c) = dec.next_buffered_audio() {
        push_audio(&mut out, c);
    }
    out
}

/// Full-file assertions shared by every clean decode.
fn assert_complete(fx: &Fixture, d: &Decoded) {
    assert!(d.error.is_none(), "{}: decode error {:?}", fx.file, d.error);
    assert_eq!(
        d.frames.len(),
        fx.frames,
        "{}: decoded frame count (pts: {:?})",
        fx.file,
        d.frames.iter().map(|f| f.pts).collect::<Vec<_>>()
    );
    assert_frames_sane(fx, &d.frames);

    // PTS start at zero and span the whole file.
    let frame_dur = 1.0 / fx.fps;
    let first = d.frames.first().unwrap().pts;
    let last = d.frames.last().unwrap().pts;
    assert!(first.abs() < 1e-3, "{}: first pts {first}", fx.file);
    let expected_last = fx.duration - frame_dur;
    assert!(
        (last - expected_last).abs() < frame_dur / 2.0,
        "{}: last pts {last}, expected ~{expected_last}",
        fx.file
    );

    match fx.audio {
        None => assert!(d.audio.is_empty(), "{}: unexpected audio", fx.file),
        Some((rate, channels)) => {
            assert!(!d.audio.is_empty(), "{}: no audio decoded", fx.file);
            for a in &d.audio {
                assert_eq!((a.rate, a.channels), (rate, channels), "{}", fx.file);
            }
            // Duration x rate, within +/- 3 AAC frames (encoder priming /
            // padding and the final partial frame).
            let expected = fx.duration * f64::from(rate);
            let got = d.audio_samples() as f64;
            assert!(
                (got - expected).abs() <= 3.0 * 1024.0,
                "{}: {got} audio samples, expected ~{expected}",
                fx.file
            );
            assert_audio_contiguous(fx, &d.audio);
        },
    }
}

/// Dimensions, strictly increasing PTS at the nominal frame spacing (no
/// drops, no duplicates), and distinct pictures frame to frame.
fn assert_frames_sane(fx: &Fixture, frames: &[FrameInfo]) {
    let frame_dur = 1.0 / fx.fps;
    for f in frames {
        assert_eq!((f.width, f.height), (fx.width, fx.height), "{}", fx.file);
    }
    for w in frames.windows(2) {
        let step = w[1].pts - w[0].pts;
        assert!(
            (step - frame_dur).abs() < frame_dur * 0.25,
            "{}: pts step {:.4} -> {:.4} (expected {frame_dur:.4}): dropped, duplicated or \
             mis-ordered frame",
            fx.file,
            w[0].pts,
            w[1].pts,
        );
        // testsrc2 changes every frame, so identical pictures in a row mean
        // a frame was emitted twice.
        assert!(
            !fx.distinct_frames || w[0].hash != w[1].hash,
            "{}: duplicate picture at pts {:.3} / {:.3}",
            fx.file,
            w[0].pts,
            w[1].pts
        );
    }
}

/// Audio chunks are gap-free: each starts where the previous one ended.
fn assert_audio_contiguous(fx: &Fixture, audio: &[AudioInfo]) {
    for w in audio.windows(2) {
        let end = w[0].ts + w[0].samples as f64 / f64::from(w[0].rate);
        assert!(
            (w[1].ts - end).abs() < 0.005,
            "{}: audio gap/overlap: chunk at {:.4} ends {:.4}, next at {:.4}",
            fx.file,
            w[0].ts,
            end,
            w[1].ts
        );
    }
}

// ---------------------------------------------------------------------------
// Network-like sources
// ---------------------------------------------------------------------------

/// A seekable source that serves at most `max_read` bytes per call and
/// optionally sleeps before each read (a slow, trickling connection).
struct TrickleSource {
    inner: Cursor<Vec<u8>>,
    max_read: usize,
    delay: Duration,
    reads: u64,
}

impl TrickleSource {
    fn new(data: Vec<u8>, max_read: usize, delay: Duration) -> Self {
        Self {
            inner: Cursor::new(data),
            max_read,
            delay,
            reads: 0,
        }
    }
}

impl Read for TrickleSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reads += 1;
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        // Vary the read size so boundaries land everywhere.
        let n = 1 + (self.reads as usize * 7919) % self.max_read;
        let n = n.min(buf.len());
        self.inner.read(&mut buf[..n])
    }
}

impl Seek for TrickleSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl VideoSource for TrickleSource {
    fn byte_len(&self) -> Option<u64> {
        Some(self.inner.get_ref().len() as u64)
    }
}

/// Top-level atom `(offset, size)` in a complete MP4.
fn find_atom(data: &[u8], fourcc: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        if size < 8 {
            return None;
        }
        if &data[pos + 4..pos + 8] == fourcc {
            return Some((pos, size));
        }
        pos += size;
    }
    None
}

// ---------------------------------------------------------------------------
// Clean decodes
// ---------------------------------------------------------------------------

#[test]
fn baseline_fixture_decodes_completely() {
    with_timeout(60, || {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes(&BASELINE_2S)).unwrap();
        assert_complete(&BASELINE_2S, &decode_all(&mut dec));
    });
}

#[test]
fn moov_at_end_decodes_completely() {
    with_timeout(60, || {
        let data = fixture_bytes(&MOOV_END_4S);
        let (moov_off, _) = find_atom(&data, b"moov").unwrap();
        let (mdat_off, _) = find_atom(&data, b"mdat").unwrap();
        assert!(moov_off > mdat_off, "fixture must have moov after mdat");
        let mut dec = SoftwareVideoDecoder::open(data).unwrap();
        assert_complete(&MOOV_END_4S, &decode_all(&mut dec));
    });
}

#[test]
fn bframe_stream_decodes_every_frame_in_presentation_order() {
    with_timeout(60, || {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes(&BFRAMES_4S)).unwrap();
        assert_complete(&BFRAMES_4S, &decode_all(&mut dec));
    });
}

#[test]
fn video_only_stream_decodes_without_audio() {
    with_timeout(60, || {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes(&NOAUDIO_2S)).unwrap();
        assert_eq!(dec.audio_format(), (0, 0));
        assert_complete(&NOAUDIO_2S, &decode_all(&mut dec));
    });
}

#[test]
fn non_macroblock_aligned_resolution_is_cropped() {
    with_timeout(60, || {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes(&ODD_2S)).unwrap();
        let d = decode_all(&mut dec);
        assert_complete(&ODD_2S, &d);
        assert_eq!(dec.video_size(), (250, 138));
    });
}

// ---------------------------------------------------------------------------
// Streaming-shaped sources
// ---------------------------------------------------------------------------

#[test]
fn trickled_short_reads_decode_identically_to_in_memory() {
    with_timeout(120, || {
        for fx in [&BASELINE_2S, &MOOV_END_4S, &BFRAMES_4S] {
            let data = fixture_bytes(fx);
            let reference = decode_all(&mut SoftwareVideoDecoder::open(data.clone()).unwrap());
            let src = TrickleSource::new(data, 97, Duration::ZERO);
            let mut dec = SoftwareVideoDecoder::open_stream(Box::new(src)).unwrap();
            let streamed = decode_all(&mut dec);
            assert_complete(fx, &streamed);
            let hashes = |d: &Decoded| d.frames.iter().map(|f| f.hash).collect::<Vec<_>>();
            assert_eq!(hashes(&reference), hashes(&streamed), "{}", fx.file);
            assert_eq!(reference.audio_samples(), streamed.audio_samples());
        }
    });
}

#[test]
fn slow_source_decodes_without_hanging() {
    // ~1 ms per <=4 KB read: a slow link. Must finish, not stall.
    with_timeout(120, || {
        let src = TrickleSource::new(fixture_bytes(&BASELINE_2S), 4096, Duration::from_millis(1));
        let mut dec = SoftwareVideoDecoder::open_stream(Box::new(src)).unwrap();
        assert_complete(&BASELINE_2S, &decode_all(&mut dec));
    });
}

#[test]
fn open_with_prefetched_moov_matches_full_scan() {
    // The streaming player opens with the moov it retained from the
    // download (`open_stream_with_moov`) instead of a full-file scan.
    with_timeout(60, || {
        for fx in [
            &BASELINE_2S,
            &MOOV_END_4S,
            &BFRAMES_4S,
            &ODD_2S,
            &NOAUDIO_2S,
        ] {
            let data = fixture_bytes(fx);
            let (off, size) = find_atom(&data, b"moov").unwrap();
            let moov = data[off..off + size].to_vec();
            let mut dec =
                SoftwareVideoDecoder::open_stream_with_moov(Box::new(Cursor::new(data)), &moov)
                    .unwrap();
            assert_complete(fx, &decode_all(&mut dec));
        }
    });
}

// ---------------------------------------------------------------------------
// Seek / restart
// ---------------------------------------------------------------------------

/// After seeking to `target`, decoding starts at the keyframe at or before
/// it, every later frame follows without gaps, and audio starts with video.
fn assert_seek(fx: &Fixture, target: f64, gop_secs: f64) {
    let mut dec = SoftwareVideoDecoder::open(fixture_bytes(fx)).unwrap();
    dec.seek(target).unwrap();
    let d = decode_all(&mut dec);
    assert!(d.error.is_none(), "{}: {:?}", fx.file, d.error);
    let keyframe = (target / gop_secs).floor() * gop_secs;
    let first = d.frames.first().expect("frames after seek").pts;
    assert!(
        (first - keyframe).abs() < 1e-3,
        "{}: seek {target}s started at {first}, expected keyframe {keyframe}",
        fx.file
    );
    assert_frames_sane(fx, &d.frames);
    let expected = ((fx.duration - keyframe) * fx.fps).round() as usize;
    assert_eq!(d.frames.len(), expected, "{}: frames after seek", fx.file);
    if fx.audio.is_some() {
        let a0 = d.audio.first().expect("audio after seek").ts;
        assert!(
            (a0 - first).abs() <= 0.05,
            "{}: A/V start mismatch after seek: audio {a0:.3}s vs video {first:.3}s",
            fx.file
        );
        assert_audio_contiguous(fx, &d.audio);
    }
}

#[test]
fn seek_lands_on_keyframe_with_audio_aligned() {
    with_timeout(120, || {
        for target in [1.0, 1.5, 2.9] {
            assert_seek(&MOOV_END_4S, target, 1.0);
            assert_seek(&BFRAMES_4S, target, 1.0);
        }
    });
}

#[test]
fn restart_by_reopening_and_seeking_repeatedly() {
    // Channel change / retune: open, play a bit, drop, reopen at a later
    // position -- repeatedly on the same bytes. Every session must start
    // cleanly.
    with_timeout(120, || {
        let data = fixture_bytes(&BFRAMES_4S);
        for (i, target) in [0.0, 2.0, 1.0, 3.0, 0.0].into_iter().enumerate() {
            let mut dec = SoftwareVideoDecoder::open(data.clone()).unwrap();
            if target > 0.0 {
                dec.seek(target).unwrap();
            }
            let f = dec.next_video_frame().unwrap().expect("first frame");
            assert!(
                (f.timestamp_secs - target).abs() < 1e-3,
                "session {i}: seek {target} started at {}",
                f.timestamp_secs
            );
            for _ in 0..5 {
                assert!(dec.next_video_frame().unwrap().is_some());
            }
        }
    });
}

#[test]
fn backward_seek_after_playing_restarts_cleanly() {
    with_timeout(60, || {
        let mut dec = SoftwareVideoDecoder::open(fixture_bytes(&BFRAMES_4S)).unwrap();
        for _ in 0..60 {
            dec.next_video_frame().unwrap().unwrap();
        }
        dec.seek(1.0).unwrap();
        let d = decode_all(&mut dec);
        assert_eq!(d.frames.first().unwrap().pts, 1.0);
        assert_eq!(d.frames.len(), 72);
        assert_frames_sane(&BFRAMES_4S, &d.frames);
    });
}

// ---------------------------------------------------------------------------
// Damaged input
// ---------------------------------------------------------------------------

#[test]
fn truncated_stream_ends_cleanly_with_the_frames_it_has() {
    with_timeout(120, || {
        for fx in [&BASELINE_2S, &BFRAMES_4S] {
            let data = fixture_bytes(fx);
            let (moov_off, moov_size) = find_atom(&data, b"moov").unwrap();
            let body_start = moov_off + moov_size;
            for pct in [30usize, 60, 90] {
                let cut = body_start + (data.len() - body_start) * pct / 100;
                let src = TrickleSource::new(data[..cut].to_vec(), 4096, Duration::ZERO);
                // A truncated file may fail to open or end early, but must
                // not panic or hang, and what it does decode must be sane.
                let Ok(mut dec) = SoftwareVideoDecoder::open_stream(Box::new(src)) else {
                    continue;
                };
                let d = decode_all(&mut dec);
                assert!(
                    d.frames.len() < fx.frames,
                    "{} cut at {pct}%: decoded all {} frames from a truncated file",
                    fx.file,
                    d.frames.len()
                );
                assert_frames_sane(fx, &d.frames);
            }
        }
    });
}

#[test]
fn moov_at_end_truncated_before_moov_fails_to_open() {
    with_timeout(30, || {
        let data = fixture_bytes(&MOOV_END_4S);
        let (moov_off, _) = find_atom(&data, b"moov").unwrap();
        let res = SoftwareVideoDecoder::open(data[..moov_off].to_vec());
        assert!(res.is_err(), "opened a file with no moov");
    });
}
