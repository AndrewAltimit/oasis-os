//! Diagnostic: play a local MP3 directly through `SdlAudioBackend`.
//!
//! Bypasses radio streaming, the network, and the poll-per-tick feed
//! loop — just loads a file into memory, calls `load_track` + `play`,
//! and waits. If the same audible stutters appear here that we've been
//! hearing in the radio app, the cause is definitively downstream of
//! our streaming code (SDL3 ↔ PipeWire/PulseAudio or the driver). If
//! playback is clean, the stutters are something specific to the
//! streaming path.
//!
//! Run with OASIS_AUDIO_DEBUG=1 to get the same periodic stats the
//! radio uses, for apples-to-apples comparison.
//!
//!   OASIS_AUDIO_DEBUG=1 cargo run --release -p oasis-backend-sdl \
//!       --example play_mp3 -- samples/ambient_dawn.mp3

use std::env;
use std::fs;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use oasis_backend_sdl::SdlAudioBackend;
use oasis_core::backend::AudioBackend;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: play_mp3 <path-to-mp3>");
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

    // load_track decodes the whole file up-front into the SDL stream —
    // this is the simplest possible feed pattern. After `play()` the
    // device drains it on its own; no further work from us. If it
    // stutters here, it isn't our code.
    let track = match backend.load_track(&data) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("load_track failed: {e}");
            return ExitCode::from(1);
        },
    };
    if let Err(e) = backend.play(track) {
        eprintln!("play failed: {e}");
        return ExitCode::from(1);
    }

    log::info!(
        "Playing — duration~{}ms. Listen for stutters; Ctrl-C to stop.",
        backend.duration_ms()
    );

    // Wall-clock based wait rather than polling position_ms, since
    // position calculation drifts on streaming tracks. For a static
    // track it's fine, but a sleep keeps this example dead simple.
    let duration_ms = backend.duration_ms().max(5_000);
    let deadline = std::time::Instant::now() + Duration::from_millis(duration_ms + 2_000);
    while std::time::Instant::now() < deadline && backend.is_playing() {
        thread::sleep(Duration::from_millis(250));
    }

    let _ = backend.stop();
    let _ = backend.shutdown();
    log::info!("Done.");
    ExitCode::from(0)
}
