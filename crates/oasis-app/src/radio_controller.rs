//! Radio subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles VFS-based
//! radio requests, background catalog/track fetching, and state machine ticks.

use crate::app_state::{AppState, CatalogFetchResult, TrackFetchResult};
use oasis_core::backend::NetworkBackend as _;
use oasis_core::vfs::Vfs;

/// Process one frame of radio state: VFS requests, background fetch polling,
/// state machine tick, auto-advance, status publish, and app refresh.
pub fn tick(state: &mut AppState, vfs: &mut dyn Vfs) {
    process_vfs_requests(state, vfs);
    poll_catalog_fetch(state);
    poll_track_fetch(state);
    tick_state_machine(state);
    auto_advance_track(state);

    // Publish radio status periodically (~4 times per second).
    if state.frame_counter.is_multiple_of(15) {
        let _ = state.radio_manager.publish_status(vfs);
    }

    // Refresh radio app display if visible.
    if let Some(ref mut runner) = state.content.app_runner {
        runner.refresh_radio(vfs);
    }
    for (_, runner) in &mut state.content.open_runners {
        runner.refresh_radio(vfs);
    }
}

/// Check the VFS for radio requests and process them.
fn process_vfs_requests(state: &mut AppState, vfs: &mut dyn Vfs) {
    use oasis_audio::RADIO_REQUEST_PATH;

    if !vfs.exists(RADIO_REQUEST_PATH) {
        return;
    }
    let Ok(data) = vfs.read(RADIO_REQUEST_PATH) else {
        return;
    };
    let request = String::from_utf8_lossy(&data).to_string();
    if request.is_empty() {
        return;
    }
    // Clear the request immediately.
    let _ = vfs.write(RADIO_REQUEST_PATH, b"");

    if let Some(target) = request.strip_prefix("tune ") {
        process_tune_request(state, target);
    } else {
        let _ = state
            .radio_manager
            .process_request(&request, &mut state.audio_backend);
        // Clear catalog on stop.
        if state.radio_manager.state() == oasis_audio::radio::RadioState::Stopped {
            state.archive_catalog = None;
            state.pending_catalog_fetch = None;
            state.pending_source_fetch = None;
        }
    }
}

/// Tune to a station by index or name.
fn process_tune_request(state: &mut AppState, target: &str) {
    // Resolve station by index or case-insensitive name.
    let station = if let Ok(idx) = target.parse::<usize>() {
        state.radio_manager.registry.stations.get(idx).cloned()
    } else {
        state
            .radio_manager
            .registry
            .stations
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(target.trim()))
            .cloned()
    };

    let Some(station) = station else {
        state
            .radio_manager
            .set_error(&format!("station not found: {target}"));
        return;
    };

    let _ = state
        .radio_manager
        .tune(&station.name, station.bitrate, &mut state.audio_backend);

    // Clear stale catalog/pending fetches on station change.
    state.archive_catalog = None;
    state.pending_catalog_fetch = None;
    state.pending_source_fetch = None;
    if let Some(mut old) = state.radio_source.take() {
        old.disconnect();
    }

    state
        .radio_manager
        .set_source_info(&station.source_type, &station.collection);

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
        state.pending_catalog_fetch = Some(rx);
    } else if let Some((host, port, path, tls)) = super::parse_stream_url(&station.url) {
        // Icecast: connect to stream (TLS if https).
        let conn_result = state
            .net
            .backend
            .connect(&host, port)
            .map_err(|e| format!("connect: {e}"))
            .and_then(|stream| {
                if tls {
                    use oasis_core::net::TlsProvider;
                    // IcecastSource is an HTTP/1.1 client — force ALPN so
                    // servers don't hand us an h2 stream.
                    state
                        .net
                        .tls_provider
                        .connect_tls_with_alpn(stream, &host, &[b"http/1.1"])
                        .map(|c| c.stream)
                        .map_err(|e| format!("TLS: {e}"))
                } else {
                    Ok(stream)
                }
            });
        match conn_result {
            Ok(stream) => {
                let source = oasis_audio::radio::IcecastSource::new(stream, &host, &path);
                state.radio_source = Some(Box::new(source));
            },
            Err(e) => {
                state.radio_manager.set_error(&e);
            },
        }
    } else {
        state.radio_manager.set_error("invalid stream URL");
    }
}

/// Poll background catalog fetch (non-blocking).
fn poll_catalog_fetch(state: &mut AppState) {
    let Some(ref rx) = state.pending_catalog_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(CatalogFetchResult { catalog, source })) => {
            state.pending_catalog_fetch = None;
            log::info!(
                "Catalog ready: {} tracks in '{}'",
                catalog.tracks.len(),
                catalog.collection
            );
            if let Some(mut old) = state.radio_source.take() {
                old.disconnect();
            }
            state.radio_source = Some(source);
            state.archive_catalog = Some(catalog);
        },
        Ok(Err(e)) => {
            state.pending_catalog_fetch = None;
            log::error!("Catalog fetch failed: {e}");
            state.radio_manager.set_error(&e);
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.pending_catalog_fetch = None;
            log::error!("Catalog fetch thread died unexpectedly");
            state.radio_manager.set_error("catalog fetch failed");
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {},
    }
}

/// Poll background track fetch (non-blocking).
fn poll_track_fetch(state: &mut AppState) {
    let Some(ref rx) = state.pending_source_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(TrackFetchResult { source })) => {
            state.pending_source_fetch = None;
            log::info!("Next track source ready");
            if let Some(mut old) = state.radio_source.take() {
                old.disconnect();
            }
            state.radio_source = Some(source);
        },
        Ok(Err(e)) => {
            state.pending_source_fetch = None;
            log::error!("Track fetch failed: {e}");
            state.radio_manager.set_error(&e);
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.pending_source_fetch = None;
            log::error!("Track fetch thread died unexpectedly");
            state.radio_manager.set_error("track fetch failed");
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {},
    }
}

/// Drive the radio state machine.
fn tick_state_machine(state: &mut AppState) {
    let _ = state
        .radio_manager
        .tick(&mut state.radio_source, &mut state.audio_backend);
}

/// Auto-advance to next track for archive stations (non-blocking).
fn auto_advance_track(state: &mut AppState) {
    if state.radio_manager.needs_next_track()
        && state.pending_source_fetch.is_none()
        && let Some(ref mut catalog) = state.archive_catalog
        && let Some(track) = catalog.next_track().cloned()
    {
        let tls = state.net.tls_provider.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = super::connect_archive_track_sync(&tls, &track);
            let _ = tx.send(result);
        });
        state.pending_source_fetch = Some(rx);
        state.radio_manager.continue_playing();
    }
}
