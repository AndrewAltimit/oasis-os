//! Diagnostic: decode an MP3 two ways and compare the PCM output.
//!
//!   A) Fresh `Mp3Decoder`, entire file fed in one `next()`-loop
//!      over the full byte buffer (what `load_track` effectively does).
//!   B) Long-lived `Mp3Decoder`, file fed 4 KB at a time, with
//!      the leftover tail carried across chunks (what streaming does).
//!
//! If the two outputs differ, minimp3 is producing different PCM for the
//! chunked pattern — that's our stutter source. If they match, the
//! stutter is coming from further downstream (resampler state,
//! queuing, timing).
//!
//!   cargo run --release -p oasis-backend-sdl \
//!       --example decode_compare -- /tmp/mp3-compare/stutter2.mp3
//!
//! Prints summary counts and, on mismatch, the first 10 differing
//! frame indices with context.
//!
//! CHUNK_SIZE defaults to 4096 to match ArchiveSource's poll read.

use std::env;
use std::fs;
use std::process::ExitCode;

use oasis_backend_sdl::mp3::{Frame, MAX_SAMPLES_PER_FRAME, Mp3Decoder, Sample};

const CHUNK_SIZE: usize = 4096;

fn decode_whole(data: &[u8]) -> Vec<Sample> {
    let mut decoder = Mp3Decoder::new();
    let mut pcm_out = [Sample::default(); MAX_SAMPLES_PER_FRAME];
    let mut samples = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let remaining = data.len() - offset;
        // Reference baseline: the full file is already in scope, so
        // minimp3's look-ahead never truncates a valid frame — `< 16`
        // just skips the trailing bytes too small to be any MP3 frame
        // header. Intentionally diverges from `decode_chunked`'s
        // `MIN_DECODE_BYTES = 2048` threshold, which mirrors
        // streaming back-pressure.
        if remaining < 16 {
            break;
        }
        match decoder.next(&data[offset..], &mut pcm_out) {
            Some((frame, consumed)) => {
                offset += consumed;
                if let Frame::Audio(audio) = frame {
                    samples.extend_from_slice(audio.samples);
                }
            },
            None => break,
        }
    }
    samples
}

fn decode_chunked(data: &[u8], chunk_size: usize) -> Vec<Sample> {
    let mut decoder = Mp3Decoder::new();
    let mut pcm_out = [Sample::default(); MAX_SAMPLES_PER_FRAME];
    let mut samples = Vec::new();
    // Mirror the streaming backend's mp3_buffer model: accumulate
    // chunks, try to decode frames, leave the partial tail for next
    // chunk.
    let mut buffer: Vec<u8> = Vec::new();
    let mut input_pos = 0;

    // Max MP3 Layer 3 frame is 1440 bytes; minimp3 also wants header
    // lookahead to validate the next frame's sync. ~2 KB is enough for
    // any bitrate/rate to sit comfortably inside the buffer with room
    // for the next sync check.
    const MIN_DECODE_BYTES: usize = 2048;

    while input_pos < data.len() {
        let end = (input_pos + chunk_size).min(data.len());
        buffer.extend_from_slice(&data[input_pos..end]);
        input_pos = end;

        // Decode as many frames as we can from the buffer.
        let mut offset = 0;
        loop {
            let remaining = buffer.len() - offset;
            if remaining < MIN_DECODE_BYTES {
                break;
            }
            match decoder.next(&buffer[offset..], &mut pcm_out) {
                Some((frame, consumed)) => {
                    offset += consumed;
                    if let Frame::Audio(audio) = frame {
                        samples.extend_from_slice(audio.samples);
                    }
                },
                None => break,
            }
        }
        buffer.drain(..offset);
    }

    // Final drain: production now ends each stream with a
    // `finalize_streaming` call that re-runs the decode loop with a
    // 16-byte minimum, recovering the sub-2 KB tail
    // that the mid-stream `MIN_DECODE_BYTES` threshold left behind.
    // Mirror that here in a second pass so this diagnostic actually
    // reflects production behaviour — otherwise the example would
    // report IDENTICAL for an input whose tail prod successfully
    // decodes. (The inner chunk loop above already drained fully
    // consumed bytes via `buffer.drain(..offset)`, so `buffer` now
    // holds only the undecoded tail.)
    let mut offset = 0;
    loop {
        let remaining = buffer.len() - offset;
        if remaining < 16 {
            break;
        }
        match decoder.next(&buffer[offset..], &mut pcm_out) {
            Some((frame, consumed)) => {
                offset += consumed;
                if let Frame::Audio(audio) = frame {
                    samples.extend_from_slice(audio.samples);
                }
            },
            None => break,
        }
    }

    samples
}

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: decode_compare <path-to-mp3> [chunk-size]");
        return ExitCode::from(2);
    };
    let chunk_size: usize = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(CHUNK_SIZE);
    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(1);
        },
    };
    println!("File: {path} ({} bytes)", data.len());

    println!("Decoding whole file…");
    let a = decode_whole(&data);
    println!("  Whole: {} samples", a.len());

    println!("Decoding in {chunk_size}-byte chunks…");
    let b = decode_chunked(&data, chunk_size);
    println!("  Chunked: {} samples", b.len());

    if a.len() != b.len() {
        println!(
            "\nMISMATCH: sample counts differ by {}",
            (a.len() as i64 - b.len() as i64).abs()
        );
    }

    let common = a.len().min(b.len());
    let mut diff_count = 0u64;
    let mut first_diffs: Vec<(usize, i16, i16)> = Vec::new();
    let mut max_diff: i32 = 0;
    for i in 0..common {
        let av = a[i];
        let bv = b[i];
        if av != bv {
            diff_count += 1;
            let d = (av as i32 - bv as i32).abs();
            if d > max_diff {
                max_diff = d;
            }
            if first_diffs.len() < 10 {
                first_diffs.push((i, av, bv));
            }
        }
    }

    if diff_count == 0 && a.len() == b.len() {
        println!("\nIDENTICAL: chunked decode produces byte-equal PCM.");
    } else {
        println!(
            "\n{} / {} samples differ ({:.3}%), max abs difference = {}",
            diff_count,
            common,
            (diff_count as f64 / common.max(1) as f64) * 100.0,
            max_diff
        );
        println!("\nFirst divergences:");
        for (i, av, bv) in first_diffs {
            println!("  sample[{i}] whole={av} chunked={bv}");
        }
    }

    ExitCode::from(0)
}
