//! Radio subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles VFS-based
//! radio requests, background catalog/track fetching, and state machine ticks.

use std::time::{Duration, Instant};

use crate::app_state::{AppState, CatalogFetchResult, TrackFetchResult};
use oasis_core::backend::NetworkBackend as _;
use oasis_core::vfs::Vfs;

/// How often the radio status file is republished (4 Hz of wall time, at
/// any frame rate).
pub const STATUS_INTERVAL: Duration = Duration::from_millis(250);

/// Process one frame of radio state at `now`: VFS requests, background
/// fetch polling, state machine tick, auto-advance and status publish.
pub fn tick(state: &mut AppState, vfs: &mut dyn Vfs, now: Instant) {
    process_vfs_requests(state, vfs);
    poll_catalog_fetch(state);
    poll_track_fetch(state);
    tick_state_machine(state);
    auto_advance_track(state);

    let due = state
        .radio
        .last_status_publish
        .is_none_or(|at| now.saturating_duration_since(at) >= STATUS_INTERVAL);
    if due {
        state.radio.last_status_publish = Some(now);
        let _ = state.radio.manager.publish_status(vfs);
    }
    // The Internet Radio app polls the published status in its own `tick`.
}

/// Check the VFS for radio requests and process them.
fn process_vfs_requests(state: &mut AppState, vfs: &mut dyn Vfs) {
    use oasis_audio::RADIO_REQUEST_PATH;

    // Trimmed: a request written with a trailing newline (`tune 2\n`)
    // must still resolve the station index.
    let Some(request) = crate::commands::take_request(vfs, RADIO_REQUEST_PATH) else {
        return;
    };

    if let Some(target) = request.strip_prefix("tune ") {
        process_tune_request(state, target.trim());
    } else {
        let _ = state
            .radio
            .manager
            .process_request(&request, state.audio_backend.as_mut());
        // A stop releases the catalog, the pending fetches and the stream.
        if state.radio.manager.state() == oasis_audio::radio::RadioState::Stopped {
            state.radio.release_playback();
        }
    }
}

/// Tune to a station by index or name.
fn process_tune_request(state: &mut AppState, target: &str) {
    // Resolve station by index or case-insensitive name.
    let station = if let Ok(idx) = target.parse::<usize>() {
        state.radio.manager.registry.stations.get(idx).cloned()
    } else {
        state
            .radio
            .manager
            .registry
            .stations
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(target.trim()))
            .cloned()
    };

    let Some(station) = station else {
        state
            .radio
            .manager
            .set_error(&format!("station not found: {target}"));
        return;
    };

    // Radio takes over the audio output: release the music track first
    // (as the WASM host does), so the music controller doesn't mistake
    // the radio's `stop` of the shared stream for the end of its track
    // and auto-advance into the radio.
    if state.media_track.is_some() {
        crate::media_controller::shutdown(state);
    }
    let _ = state
        .radio
        .manager
        .tune(&station.name, station.bitrate, state.audio_backend.as_mut());

    // Clear stale catalog/pending fetches on station change.
    state.radio.release_playback();

    state
        .radio
        .manager
        .set_source_info(&station.source_type, &station.collection);

    if state.offline {
        state.radio.manager.set_error("offline: network disabled");
        return;
    }

    if station.source_type == "archive" && !station.collection.is_empty() {
        // Internet Archive: spawn background thread to fetch catalog.
        let collection = station.collection.clone();
        let seed = state.frame_counter;
        let tls = state.net.tls_provider.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = super::fetch_catalog_blocking(&collection, seed, &tls);
            let _ = tx.send(result);
        });
        state.radio.pending_catalog_fetch = Some(rx);
    } else if oasis_audio::radio::StreamUrl::parse(&station.url).is_some() {
        // Icecast: connect on the pump thread (TLS if https) so the TCP
        // connect + TLS handshake never stall the frame loop, follow any
        // redirects (same-scheme or http -> https only), then stream
        // through the ThreadedSource readahead queue.
        let tls_provider = state.net.tls_provider.clone();
        let url = station.url.clone();
        let source = oasis_audio::radio::ThreadedSource::spawn_connect(move || {
            let src = oasis_audio::radio::connect_icecast(&url, |target| {
                use oasis_core::net::TlsProvider;
                let mut net = oasis_core::net::StdNetworkBackend::new();
                let stream = net
                    .connect(&target.host, target.port)
                    .map_err(|e| format!("connect: {e}"))?;
                if target.https {
                    // IcecastSource is an HTTP/1.1 client — force ALPN so
                    // servers don't hand us an h2 stream.
                    tls_provider
                        .connect_tls_with_alpn(stream, &target.host, &[b"http/1.1"])
                        .map(|c| c.stream)
                        .map_err(|e| format!("TLS: {e}"))
                } else {
                    Ok(stream)
                }
            })?;
            Ok(Box::new(src) as Box<dyn oasis_audio::radio::RadioSource + Send>)
        });
        state.radio.source = Some(Box::new(source));
    } else {
        state.radio.manager.set_error("invalid stream URL");
    }
}

/// Poll background catalog fetch (non-blocking).
fn poll_catalog_fetch(state: &mut AppState) {
    let Some(ref rx) = state.radio.pending_catalog_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(CatalogFetchResult { catalog, source })) => {
            state.radio.pending_catalog_fetch = None;
            log::info!(
                "Catalog ready: {} tracks in '{}'",
                catalog.tracks.len(),
                catalog.collection
            );
            if let Some(mut old) = state.radio.source.take() {
                old.disconnect();
            }
            state.radio.source = Some(source);
            state.radio.archive_catalog = Some(catalog);
        },
        Ok(Err(e)) => {
            state.radio.pending_catalog_fetch = None;
            log::error!("Catalog fetch failed: {e}");
            state.radio.manager.set_error(&e);
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.radio.pending_catalog_fetch = None;
            log::error!("Catalog fetch thread died unexpectedly");
            state.radio.manager.set_error("catalog fetch failed");
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {},
    }
}

/// Poll background track fetch (non-blocking).
fn poll_track_fetch(state: &mut AppState) {
    let Some(ref rx) = state.radio.pending_source_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(TrackFetchResult { source })) => {
            state.radio.pending_source_fetch = None;
            log::info!("Next track source ready");
            if let Some(mut old) = state.radio.source.take() {
                old.disconnect();
            }
            state.radio.source = Some(source);
        },
        Ok(Err(e)) => {
            state.radio.pending_source_fetch = None;
            log::error!("Track fetch failed: {e}");
            state.radio.manager.set_error(&e);
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.radio.pending_source_fetch = None;
            log::error!("Track fetch thread died unexpectedly");
            state.radio.manager.set_error("track fetch failed");
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {},
    }
}

/// Drive the radio state machine.
fn tick_state_machine(state: &mut AppState) {
    let _ = state
        .radio
        .manager
        .tick(&mut state.radio.source, state.audio_backend.as_mut());
}

/// Auto-advance to next track for archive stations (non-blocking).
fn auto_advance_track(state: &mut AppState) {
    if state.radio.manager.needs_next_track()
        && !state.offline
        && state.radio.pending_source_fetch.is_none()
        && let Some(ref mut catalog) = state.radio.archive_catalog
        && let Some(track) = catalog.next_track().cloned()
    {
        let tls = state.net.tls_provider.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = super::connect_archive_track_sync(&tls, &track);
            let _ = tx.send(result);
        });
        state.radio.pending_source_fetch = Some(rx);
        state.radio.manager.continue_playing();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use oasis_audio::RADIO_REQUEST_PATH;
    use oasis_audio::radio::{AudioChunk, RadioSource, SourceState, StationRegistry};
    use oasis_core::vfs::MemoryVfs;

    use super::*;

    /// A stream that records whether it was disconnected.
    struct FakeSource(Rc<Cell<bool>>);

    impl RadioSource for FakeSource {
        fn poll(&mut self) -> oasis_core::error::Result<Option<AudioChunk>> {
            Ok(None)
        }
        fn disconnect(&mut self) {
            self.0.set(true);
        }
        fn state(&self) -> SourceState {
            SourceState::Active
        }
        fn source_type(&self) -> &str {
            "fake"
        }
    }

    fn post(vfs: &mut MemoryVfs, request: &str) {
        if !vfs.exists("/var/radio") {
            vfs.mkdir("/var").ok();
            vfs.mkdir("/var/radio").expect("radio dir");
        }
        vfs.write(RADIO_REQUEST_PATH, request.as_bytes())
            .expect("request");
    }

    /// The app's `stop` request closes the stream: a source left behind
    /// kept its connection open and the shell redrawing every frame.
    #[test]
    fn stop_request_releases_the_stream_source() {
        let mut state = AppState::new_for_test();
        let mut vfs = MemoryVfs::new();
        let closed = Rc::new(Cell::new(false));
        state.radio.source = Some(Box::new(FakeSource(Rc::clone(&closed))));
        post(&mut vfs, "stop");
        process_vfs_requests(&mut state, &mut vfs);
        assert!(state.radio.source.is_none(), "source dropped");
        assert!(closed.get(), "source disconnected");
    }

    /// `tune 0` written with a trailing newline still tunes station 0.
    #[test]
    fn tune_request_is_trimmed() {
        let mut state = AppState::new_for_test();
        state.radio.manager.registry = StationRegistry::from_toml(
            "[[station]]\nname = \"Test FM\"\nurl = \"http://example.invalid/s\"\n\
             genre = \"test\"\n",
        )
        .expect("registry");
        let mut vfs = MemoryVfs::new();
        post(&mut vfs, "tune 0\n");
        process_vfs_requests(&mut state, &mut vfs);
        // Offline fixture: the tune resolved the station, then stopped
        // short of the network.
        let err = state.radio.manager.error_msg();
        assert!(err.contains("offline"), "{err}");
    }

    /// A `Send` stream (what the fetch threads hand back) recording its
    /// disconnect.
    struct SendSource(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl RadioSource for SendSource {
        fn poll(&mut self) -> oasis_core::error::Result<Option<AudioChunk>> {
            Ok(None)
        }
        fn disconnect(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn state(&self) -> SourceState {
            SourceState::Active
        }
        fn source_type(&self) -> &str {
            "fetched"
        }
    }

    /// A catalog arriving from the fetch thread replaces (and closes) the
    /// previous stream and installs the catalog; a track fetch likewise.
    #[test]
    fn arriving_catalog_and_track_replace_the_old_stream() {
        let mut state = AppState::new_for_test();
        let old_closed = Rc::new(Cell::new(false));
        state.radio.source = Some(Box::new(FakeSource(Rc::clone(&old_closed))));
        let (tx, rx) = std::sync::mpsc::channel();
        state.radio.pending_catalog_fetch = Some(rx);
        // Nothing yet: the fetch stays pending.
        poll_catalog_fetch(&mut state);
        assert!(state.radio.pending_catalog_fetch.is_some());
        let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        tx.send(Ok(CatalogFetchResult {
            catalog: oasis_audio::radio::ArchiveCatalog::new("coll"),
            source: Box::new(SendSource(std::sync::Arc::clone(&first))),
        }))
        .expect("send");
        poll_catalog_fetch(&mut state);
        assert!(state.radio.pending_catalog_fetch.is_none());
        assert!(old_closed.get(), "previous stream disconnected");
        assert_eq!(
            state
                .radio
                .archive_catalog
                .as_ref()
                .map(|c| c.collection.as_str()),
            Some("coll")
        );
        assert_eq!(
            state.radio.source.as_ref().map(|s| s.source_type()),
            Some("fetched")
        );

        let (tx, rx) = std::sync::mpsc::channel();
        state.radio.pending_source_fetch = Some(rx);
        tx.send(Ok(TrackFetchResult {
            source: Box::new(SendSource(std::sync::Arc::new(Default::default()))),
        }))
        .expect("send");
        poll_track_fetch(&mut state);
        assert!(state.radio.pending_source_fetch.is_none());
        assert!(
            first.load(std::sync::atomic::Ordering::SeqCst),
            "the catalog's first stream closed for the next track"
        );
    }

    /// Fetch failures and dead fetch threads surface as the radio error
    /// (the app shows it) and clear the pending fetch, so a retry can start.
    #[test]
    fn fetch_failures_surface_as_radio_errors() {
        let mut state = AppState::new_for_test();
        let (tx, rx) = std::sync::mpsc::channel();
        state.radio.pending_catalog_fetch = Some(rx);
        tx.send(Err("HTTP 503".into())).expect("send");
        poll_catalog_fetch(&mut state);
        assert!(state.radio.pending_catalog_fetch.is_none());
        assert_eq!(state.radio.manager.error_msg(), "HTTP 503");

        let (tx, rx) = std::sync::mpsc::channel::<Result<CatalogFetchResult, String>>();
        state.radio.pending_catalog_fetch = Some(rx);
        drop(tx);
        poll_catalog_fetch(&mut state);
        assert!(state.radio.pending_catalog_fetch.is_none());
        assert_eq!(state.radio.manager.error_msg(), "catalog fetch failed");

        let (tx, rx) = std::sync::mpsc::channel();
        state.radio.pending_source_fetch = Some(rx);
        tx.send(Err("track 404".into())).expect("send");
        poll_track_fetch(&mut state);
        assert!(state.radio.pending_source_fetch.is_none());
        assert_eq!(state.radio.manager.error_msg(), "track 404");

        let (tx, rx) = std::sync::mpsc::channel::<Result<TrackFetchResult, String>>();
        state.radio.pending_source_fetch = Some(rx);
        drop(tx);
        poll_track_fetch(&mut state);
        assert!(state.radio.pending_source_fetch.is_none());
        assert_eq!(state.radio.manager.error_msg(), "track fetch failed");
    }
}
