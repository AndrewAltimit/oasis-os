//! Ignored soak test for diagnosing radio streaming stutters.
//!
//! Drives the exact production pipeline — `fetch_catalog_blocking` →
//! `connect_archive_source` (ThreadedSource-wrapped) → `RadioManager::tick`
//! at 60 Hz → real SDL audio at volume 0 — and logs queue depths every
//! second. A "STARVED" line means the SDL stream held under 1 s of PCM
//! while Playing (designed floor is ~3.6 s), i.e. an audible stutter.
//!
//! Run manually (needs network + an audio device):
//!   RUST_LOG=info cargo test -p oasis-app --release -- --ignored \
//!       radio_soak_archive --nocapture
//!
//! `OASIS_SOAK_SECS` overrides the default 180 s duration.
//! `OASIS_SOAK_TICK_MS` overrides the 16 ms tick interval — set it to
//! e.g. 500 to simulate a collapsed frame rate (occluded window, heavy
//! scene) and verify ingest keeps up regardless.

use std::time::{Duration, Instant};

use oasis_audio::radio::source::RadioSource;
use oasis_audio::radio::{RadioManager, RadioState};
use oasis_core::backend::AudioBackend;

#[test]
#[ignore = "network + audio-device soak; run manually with --ignored"]
fn radio_soak_archive() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let soak_secs: u64 = std::env::var("OASIS_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);
    let tick_ms: u64 = std::env::var("OASIS_SOAK_TICK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    let tls = oasis_core::net::RustlsTlsProvider::new();
    let mut backend = oasis_backend_sdl::SdlAudioBackend::new();
    backend.init().expect("SDL audio init");

    let mut mgr = RadioManager::new();
    mgr.set_volume(0, &mut backend).expect("set volume");
    mgr.tune("Old Time Radio", 0, &mut backend).expect("tune");

    log::info!("soak: fetching catalog for 'oldtimeradio'...");
    let res = crate::fetch_catalog_blocking("oldtimeradio", 12345, &tls).expect("catalog fetch");
    let mut catalog = res.catalog;
    let mut source: Option<Box<dyn RadioSource>> = Some(res.source);
    log::info!(
        "soak: connected, {} tracks in catalog",
        catalog.tracks.len()
    );

    let start = Instant::now();
    let mut last_log = Instant::now();
    let mut underruns = 0u32;
    let mut playing_since: Option<Instant> = None;
    // Per-second aggregates so short dips/stalls between 1 Hz reports
    // aren't aliased away.
    let mut min_q = u32::MAX;
    let mut max_tick_ms = 0u128;

    while start.elapsed() < Duration::from_secs(soak_secs) {
        let tick_start = Instant::now();
        mgr.tick(&mut source, &mut backend).expect("tick");
        max_tick_ms = max_tick_ms.max(tick_start.elapsed().as_millis());
        if mgr.state() == RadioState::Playing {
            if playing_since.is_none() {
                playing_since = Some(Instant::now());
            }
            // Only meaningful while playing — the queue is legitimately
            // near-empty during Connecting/Buffering.
            min_q = min_q.min(backend.music_queued_bytes());
        }

        // Auto-advance like radio_controller does (synchronously here).
        if mgr.needs_next_track() {
            log::info!("soak: track ended, advancing");
            if let Some(track) = catalog.next_track().cloned() {
                match crate::connect_archive_source(&tls, &track) {
                    Ok(s) => {
                        source = Some(s);
                        mgr.continue_playing();
                    },
                    Err(e) => {
                        log::error!("soak: advance failed: {e}");
                        break;
                    },
                }
            } else {
                break;
            }
        }

        if mgr.state() == RadioState::Error {
            log::error!("soak: radio error: {}", mgr.error_msg());
            break;
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            last_log = Instant::now();
            let q = backend.music_queued_bytes();
            // Grace period: the queue legitimately ramps for the first
            // couple of seconds after playback starts.
            let playing_settled = playing_since
                .map(|t| t.elapsed() >= Duration::from_secs(2))
                .unwrap_or(false);
            // The queue's designed floor is REFILL_QUEUE_BYTES (~3.6 s of
            // PCM); below 1 s the device is draining dry between feeds —
            // that is the audible stutter state. `min_q` is sampled every
            // tick so brief dips between 1 Hz reports are caught too.
            if playing_settled && min_q != u32::MAX && min_q < 192_000 {
                underruns += 1;
                log::warn!(
                    "soak: STARVED at t={}s (min queued {}KB)",
                    start.elapsed().as_secs(),
                    min_q / 1024,
                );
            }
            log::info!(
                "soak t={:>3}s state={} sdl_queued={:>4}KB (~{:.1}s) min={:>4}KB max_tick={}ms",
                start.elapsed().as_secs(),
                mgr.state(),
                q / 1024,
                // 48 kHz stereo i16 playout = 192 000 bytes/s.
                q as f64 / 192_000.0,
                if min_q == u32::MAX { 0 } else { min_q / 1024 },
                max_tick_ms,
            );
            min_q = u32::MAX;
            max_tick_ms = 0;
        }

        std::thread::sleep(Duration::from_millis(tick_ms));
    }

    log::info!(
        "soak done after {}s: underruns={underruns}",
        start.elapsed().as_secs(),
    );
    assert!(playing_since.is_some(), "never reached Playing state");
    assert_eq!(underruns, 0, "audio underruns detected");
}
