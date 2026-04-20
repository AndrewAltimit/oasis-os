//! Diagnostic: play a local MP3 through the same *streaming* path
//! that radio uses — `load_streaming` + chunked `feed_data` calls —
//! instead of `load_track`. This removes the network and radio code
//! from the loop entirely, so if the same audible stutter shows up
//! here as in radio, we've definitively proved the cause is in the
//! streaming feed pattern itself.
//!
//! Feeds a 4 KB chunk (matching `ArchiveSource::poll()` read size) on
//! each call. A wall-clock sleep between calls keeps the push rate
//! comparable to what the radio controller does at 60 Hz.
//!
//!   OASIS_AUDIO_DEBUG=1 cargo run --release -p oasis-backend-sdl \
//!       --example stream_mp3 -- /tmp/mp3-compare/stutter2.mp3

use std::env;
use std::fs;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use oasis_backend_sdl::SdlAudioBackend;
use oasis_core::backend::AudioBackend;

const CHUNK_SIZE: usize = 4096;
/// Tick-ish cadence: one chunk per ~16 ms so total throughput is
/// ~240 KB/s, which is way more than any MP3 needs — the hysteresis
/// inside the SDL backend will ratelimit the actual feed to match
/// playback, same as in the radio app.
const TICK_MS: u64 = 16;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: stream_mp3 <path-to-mp3>");
        return ExitCode::from(2);
    };

    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(1);
        },
    };
    log::info!("Loaded {path} ({} bytes)", data.len());

    let mut backend = SdlAudioBackend::new();
    if let Err(e) = backend.init() {
        eprintln!("audio init failed: {e}");
        return ExitCode::from(1);
    }

    let track = match backend.load_streaming() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("load_streaming failed: {e}");
            return ExitCode::from(1);
        },
    };
    if let Err(e) = backend.play(track) {
        eprintln!("play failed: {e}");
        return ExitCode::from(1);
    }

    log::info!("Streaming {} bytes as 4 KB chunks…", data.len());
    let start = Instant::now();
    let mut offset = 0;
    while offset < data.len() {
        // Only feed when the backend says there's room — mirrors
        // RadioManager::tick.
        if backend.streaming_can_accept(track) {
            let end = (offset + CHUNK_SIZE).min(data.len());
            if let Err(e) = backend.feed_data(track, &data[offset..end]) {
                log::warn!("feed_data error at offset {offset}: {e}");
            }
            offset = end;
        }
        thread::sleep(Duration::from_millis(TICK_MS));
    }
    log::info!(
        "All {} bytes fed in {:.1}s; waiting for queue to drain…",
        data.len(),
        start.elapsed().as_secs_f64()
    );

    // Drain: sleep until playback finishes. Use duration_ms if known,
    // otherwise cap at an hour.
    let total_wait_ms = 60 * 60 * 1000;
    let deadline = Instant::now() + Duration::from_millis(total_wait_ms);
    while Instant::now() < deadline && backend.is_playing() {
        thread::sleep(Duration::from_millis(500));
    }

    let _ = backend.stop();
    let _ = backend.shutdown();
    log::info!("Done.");
    ExitCode::from(0)
}
