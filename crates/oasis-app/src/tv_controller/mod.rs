//! TV Guide subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles TV catalog
//! fetching, video player ticking, tune/untune requests, and audio streaming.

mod download;
mod streaming_buffer;

// Re-export public items so existing `use crate::tv_controller::*` paths work.
#[cfg(feature = "_video")]
pub(crate) use streaming_buffer::{MIN_PREBUFFER, StreamingBuffer, StreamingInner};

use crate::app_state::AppState;
use oasis_core::apps::AppRunner;
use oasis_core::backend::{AudioBackend, SdiBackend};
use oasis_core::vfs::Vfs;

/// Process one frame of TV state: catalog fetching, tune requests, video
/// player ticking, and untune detection.
pub fn tick(state: &mut AppState, backend: &mut impl SdiBackend, vfs: &mut dyn Vfs) {
    poll_catalog_fetch(state);
    start_catalog_fetch_if_needed(state);
    handle_tune_requests(state, backend, vfs);
    tick_video_player(state, backend);
    detect_untune(state, backend);
    auto_advance_episode(state, backend);
}

/// Poll pending TV catalog fetch (non-blocking).
fn poll_catalog_fetch(state: &mut AppState) {
    let Some(ref rx) = state.pending_tv_catalog_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(catalogs)) => {
            let loaded = catalogs.iter().filter(|c| c.is_some()).count();
            let total = catalogs.len();
            log::info!("TV catalog fetch result: {loaded}/{total} channels have episodes");
            state.pending_tv_catalog_fetch = None;
            let runner = find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    let all_none = catalogs.iter().all(|c| c.is_none());
                    for (i, cat) in catalogs.into_iter().enumerate() {
                        if let Some(c) = cat
                            && i < guide.catalogs.len()
                        {
                            guide.catalogs[i] = Some(c);
                            guide.rebuild_cached_schedule(i);
                        }
                    }
                    if all_none {
                        log::warn!("TV: all channel catalogs empty");
                        guide.fetch_error = Some("No episodes found for any channel".into());
                    }

                    // Auto-tune if OASIS_TV_CHANNEL is set (for automated testing).
                    if let Ok(ch_str) = std::env::var("OASIS_TV_CHANNEL")
                        && let Ok(ch_num) = ch_str.parse::<u32>()
                    {
                        if let Some(idx) = guide.channels.iter().position(|c| c.number == ch_num) {
                            guide.selected_channel = idx;
                            if let Some(req) = guide.tune() {
                                use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
                                let url = oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(&req.episode);
                                let seek_secs = std::env::var("OASIS_TV_SEEK")
                                    .ok()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .unwrap_or(req.seek_secs);
                                let data = format!("tune_url {url} {seek_secs}");
                                log::info!("TV: auto-tune CH{} -> {}", ch_num, req.episode.title,);
                                runner.set_pending_request(TV_REQUEST_PATH.to_string(), data);
                            } else if let Some(catalog) =
                                guide.catalogs.get(idx).and_then(|c| c.as_ref())
                                && let Some(ep) = catalog.episodes.first()
                            {
                                // Force-tune to first episode (schedule has no slot
                                // at current time, but env var testing needs it).
                                use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
                                let url = oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(ep);
                                let seek_secs = std::env::var("OASIS_TV_SEEK")
                                    .ok()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .unwrap_or(0);
                                let data = format!("tune_url {url} {seek_secs}");
                                log::info!(
                                    "TV: force-tune CH{} -> {} (no schedule slot)",
                                    ch_num,
                                    ep.title,
                                );
                                guide.tuned_channel = Some(idx);
                                runner.set_pending_request(TV_REQUEST_PATH.to_string(), data);
                            } else {
                                log::warn!("TV: auto-tune CH{ch_num} failed (no episodes)");
                            }
                        } else {
                            log::warn!("TV: OASIS_TV_CHANNEL={ch_num} not found in channels");
                        }
                    }
                }
                runner.refresh_tv_text();
            } else {
                log::warn!("TV: catalogs arrived but no TV Guide runner found");
            }
        },
        Ok(Err(e)) => {
            state.pending_tv_catalog_fetch = None;
            log::error!("TV catalog fetch failed: {e}");
            let runner = find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    guide.fetch_error = Some(e);
                }
                runner.refresh_tv_text();
            } else {
                log::warn!("TV: error arrived but no TV Guide runner found");
            }
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.pending_tv_catalog_fetch = None;
            log::error!("TV catalog fetch thread died");
            let runner = find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    guide.fetch_error = Some("catalog fetch failed".into());
                }
                runner.refresh_tv_text();
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            // Timeout after 2 minutes.
            if let Some(start) = state.tv_fetch_start
                && start.elapsed().as_secs() >= 120
            {
                log::warn!("TV: catalog fetch timed out after 120s");
                state.pending_tv_catalog_fetch = None;
                state.tv_fetch_start = None;
                let runner = find_tv_guide_runner(
                    &mut state.content.app_runner,
                    &mut state.content.open_runners,
                );
                if let Some(runner) = runner {
                    if let Some(guide) = runner.tv_guide_state() {
                        guide.fetch_in_progress = false;
                        guide.fetch_error = Some("Fetch timed out (2 min)".into());
                    }
                    runner.refresh_tv_text();
                }
            }
        },
    }
}

/// Start TV catalog fetch if a TV Guide app needs it.
fn start_catalog_fetch_if_needed(state: &mut AppState) {
    if state.pending_tv_catalog_fetch.is_some() {
        return;
    }
    let runner = find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    if let Some(runner) = runner
        && let Some(guide) = runner.tv_guide_state()
        && !guide.fetch_attempted
        && guide.catalogs.iter().all(|c| c.is_none())
    {
        log::info!(
            "TV: starting catalog fetch for {} channels",
            guide.channels.len(),
        );
        guide.fetch_attempted = true;
        guide.fetch_in_progress = true;
        let channels = guide.channels.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let tls = state.net.tls_provider.clone();
        std::thread::spawn(move || {
            log::info!("TV: background fetch thread started");
            let result = super::fetch_tv_catalogs_blocking(&channels, &tls);
            log::info!(
                "TV: background fetch thread finished (ok={})",
                result.is_ok(),
            );
            let _ = tx.send(result);
        });
        state.pending_tv_catalog_fetch = Some(rx);
        state.tv_fetch_start = Some(std::time::Instant::now());
    }
}

/// Handle TV Guide tune requests -- start in-app video player.
fn handle_tune_requests(state: &mut AppState, backend: &mut impl SdiBackend, vfs: &mut dyn Vfs) {
    let runner = find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    let Some(runner) = runner else { return };
    let Some((path, data)) = runner.take_pending_request() else {
        return;
    };

    if path != oasis_core::apps::tv_guide::TV_REQUEST_PATH || !data.starts_with("tune_url ") {
        let _ = vfs.write(&path, data.as_bytes());
        return;
    }

    let rest = &data["tune_url ".len()..];
    // Parse "url seek_secs" from IPC data.
    let (url, seek_secs) = if let Some(space_idx) = rest.rfind(' ') {
        let seek: u64 = rest[space_idx + 1..].parse().unwrap_or(0);
        (&rest[..space_idx], seek)
    } else {
        (rest, 0u64)
    };
    // Allow test override of seek position.
    let seek_secs = std::env::var("OASIS_TV_SEEK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(seek_secs);
    log::info!("TV: tune request: seek={seek_secs}s url={url}");

    // Deduplicate: ignore tune requests for the URL already playing.
    #[cfg(feature = "_video")]
    if state.tv_current_url.as_deref() == Some(url) && state.video_player.is_active() {
        log::info!("TV: ignoring duplicate tune request for same URL");
        return;
    }

    // Cancel any orphaned streaming session (download + decoder threads).
    #[cfg(feature = "_video")]
    if let Some(ref session) = state.tv_stream_session.take() {
        session.cancel();
    }

    // Stop any existing video session.
    state.video_player.stop(backend);
    if let Some(track) = state.tv_audio_track.take() {
        let _ = state.audio_backend.unload_track(track);
    }
    // Reset diagnostics for the new session.
    state.tv_audio_chunks_fed = 0;
    state.tv_audio_samples_fed = 0;

    // Decode at full screen resolution so the video looks sharp in both
    // PIP and expanded (fullscreen) modes. The backend handles downscaling
    // when blitting to the smaller PIP area.
    let at = &state.active_theme;
    let usable_h = at
        .screen_h
        .saturating_sub(at.statusbar_height + at.bottombar_height);
    let preview_w = at.screen_w;
    let preview_h = usable_h;
    log::info!("TV: video decode {preview_w}x{preview_h}, seek={seek_secs}s");

    #[cfg(feature = "_video")]
    start_video_download(state, url, seek_secs, preview_w, preview_h);

    #[cfg(not(feature = "_video"))]
    start_ffmpeg_playback(state, url, seek_secs, preview_w, preview_h);
}

/// Start ffmpeg-based playback (the legacy path, used when video-decode is disabled).
#[cfg(not(feature = "_video"))]
fn start_ffmpeg_playback(state: &mut AppState, url: &str, seek_secs: u64, width: u32, height: u32) {
    state.video_player.start(url, seek_secs, width, height);
    setup_streaming_audio(state);
}

/// Set up a streaming audio track for the video player.
fn setup_streaming_audio(state: &mut AppState) {
    match state.audio_backend.load_streaming() {
        Ok(track) => {
            let _ = state.audio_backend.play(track);
            state.tv_audio_track = Some(track);
        },
        Err(e) => {
            log::warn!("TV: failed to start audio stream: {e}");
        },
    }
}

/// Start streaming video decode -- downloads in background while decoding
/// starts immediately. No "Downloading..." wait state.
#[cfg(feature = "_video")]
fn start_video_download(state: &mut AppState, url: &str, seek_secs: u64, width: u32, height: u32) {
    use std::sync::Arc;

    // Check cache: if URL exists and file is on disk with valid size, play from file.
    if let Some(pos) = state.tv_video_cache.iter().position(|(u, _)| u == url) {
        let (_, ref path) = state.tv_video_cache[pos];
        let valid = path.metadata().map(|m| m.len() > 8192).unwrap_or(false);
        if valid {
            log::info!("TV: cache hit for {url}, starting software decode");
            state.tv_video_cache_path = Some(path.clone());
            state
                .video_player
                .start_software(path.clone(), seek_secs, width, height);
            setup_streaming_audio(state);
            return;
        }
        // File missing or too small (failed download) -- remove stale entry.
        state.tv_video_cache.remove(pos);
    }

    // Create a streaming buffer shared between the download thread and decoder.
    let buffer = Arc::new(StreamingInner::new());
    let reader = StreamingBuffer::new(Arc::clone(&buffer));
    let eviction_flag = Arc::clone(&reader.eviction_enabled);

    // Store session for cancellation on re-tune, and URL for dedup.
    state.tv_stream_session = Some(Arc::clone(&buffer));
    state.tv_current_url = Some(url.to_string());

    let url_owned = url.to_string();
    let tls = state.net.tls_provider.clone();

    // Clone for the decoder thread to wait on moov data.
    let moov_buffer = Arc::clone(&buffer);
    let download_buffer = Arc::clone(&buffer);

    std::thread::spawn(move || {
        log::info!("TV: streaming download thread started: {url_owned}");
        if let Err(e) = download::stream_download(&url_owned, &tls, &download_buffer, seek_secs)
            && !download_buffer.is_cancelled()
        {
            log::error!("TV: streaming download failed: {e}");
            download_buffer.set_error(e);
        }
    });

    // Enable sliding-window eviction after the decoder finishes its initial
    // probe. With pre-extracted avcC, there is no full-file scan, so eviction
    // can be enabled immediately.
    let on_init: Box<dyn FnOnce() + Send> = Box::new(move || {
        log::info!("TV: decoder initialized, enabling sliding-window eviction");
        eviction_flag.store(true, std::sync::atomic::Ordering::Release);
    });

    // Start the decoder -- it will block-read from the streaming buffer as
    // data arrives from the HTTP download.  Moov data is fetched from the
    // shared buffer on the decoder thread (not the UI thread).
    state.video_player.start_software_source(
        Box::new(reader),
        seek_secs,
        width,
        height,
        Some(on_init),
        moov_buffer,
    );
    setup_streaming_audio(state);

    // Clear download-related state (no longer used for streaming).
    state.pending_video_download = None;
    state.tv_download_progress = None;
    state.pending_video_params = None;
}

/// Tick video player: upload frames, collect audio chunks.
fn tick_video_player(state: &mut AppState, backend: &mut impl SdiBackend) {
    let (texture, audio_output) = state.video_player.tick(backend);

    // Feed audio to the streaming track.
    if let Some(track) = state.tv_audio_track {
        let mut audio_chunks_fed = 0u32;
        let mut audio_samples_fed = 0u64;
        match &audio_output {
            #[cfg(not(feature = "_video"))]
            crate::video_player::AudioOutput::Mp3Chunks(chunks) => {
                for chunk in chunks {
                    let _ = state.audio_backend.feed_data(track, chunk);
                    audio_chunks_fed += 1;
                }
            },
            #[cfg(feature = "_video")]
            crate::video_player::AudioOutput::PcmF32(chunks) => {
                for chunk in chunks {
                    audio_samples_fed += chunk.pcm_f32.len() as u64;
                    if let Err(e) = state.audio_backend.feed_pcm_f32(
                        track,
                        &chunk.pcm_f32,
                        chunk.channels,
                        chunk.sample_rate,
                    ) {
                        log::warn!("TV: audio feed error: {e}");
                    }
                    audio_chunks_fed += 1;
                }
            },
            crate::video_player::AudioOutput::None => {},
        }
        if audio_chunks_fed > 0 {
            state.tv_audio_chunks_fed += u64::from(audio_chunks_fed);
            state.tv_audio_samples_fed += audio_samples_fed;
        }
    }

    // Periodic diagnostics (every ~5 seconds at 60fps).
    if state.video_player.is_active() && state.frame_counter.is_multiple_of(300) {
        log::info!(
            "TV: main thread: {} display frames, {} audio chunks fed ({:.1}M samples)",
            state.video_player.displayed_frames(),
            state.tv_audio_chunks_fed,
            state.tv_audio_samples_fed as f64 / 1_000_000.0,
        );
    }

    // Update the guide's preview texture and download status.
    let download_status = {
        #[cfg(feature = "_video")]
        {
            state.tv_stream_session.as_ref().and_then(|session| {
                let received = session.bytes_received();
                let total = session
                    .total_size
                    .load(std::sync::atomic::Ordering::Relaxed);
                if total > 0 {
                    let pct = (received * 100).checked_div(total).unwrap_or(0);
                    Some(format!("{}% ({}/{}KB)", pct, received / 1024, total / 1024,))
                } else if received > 0 {
                    Some(format!("{}KB", received / 1024))
                } else {
                    None
                }
            })
        }
        #[cfg(not(feature = "_video"))]
        {
            None::<String>
        }
    };
    let runner = find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    if let Some(runner) = runner
        && let Some(guide) = runner.tv_guide_state()
    {
        guide.preview_texture = texture;
        guide.download_status = download_status;
    }
}

/// Detect untune: video is active but guide has no tuned channel.
fn detect_untune(state: &mut AppState, backend: &mut impl SdiBackend) {
    if !state.video_player.is_active() {
        return;
    }
    let should_stop = {
        let runner = find_tv_guide_runner(
            &mut state.content.app_runner,
            &mut state.content.open_runners,
        );
        match runner {
            Some(runner) => runner
                .tv_guide_state()
                .is_none_or(|g| g.tuned_channel.is_none()),
            None => true, // TV Guide closed.
        }
    };
    if should_stop {
        log::info!("TV: untuned or guide closed, stopping video");
        state.video_player.stop(backend);
        if let Some(track) = state.tv_audio_track.take() {
            let _ = state.audio_backend.unload_track(track);
        }
        #[cfg(feature = "_video")]
        {
            if let Some(ref session) = state.tv_stream_session.take() {
                session.cancel();
            }
            state.tv_current_url = None;
            state.pending_video_download = None;
            state.pending_video_params = None;
            state.tv_download_progress = None;
            // Keep the file in cache (don't delete) -- it can be reused on re-tune.
            state.tv_video_cache_path = None;
        }
    }
}

/// Auto-advance to the next episode when the current video reaches EOF.
///
/// Re-tunes to whatever the schedule says should be playing *now*,
/// which will be the next episode since the previous one just ended.
fn auto_advance_episode(state: &mut AppState, backend: &mut impl SdiBackend) {
    if !state.video_player.is_finished() {
        return;
    }

    let runner = find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    let Some(runner) = runner else { return };
    let Some(guide) = runner.tv_guide_state() else {
        return;
    };

    // Only auto-advance if we're currently tuned to a channel.
    let Some(channel_idx) = guide.tuned_channel else {
        return;
    };

    // Stop the finished player immediately to reset the `finished` flag.
    // This prevents auto_advance from firing again on the next frame.
    state.video_player.stop(backend);
    // Clear the guide's preview texture so SDI doesn't reference the
    // destroyed texture before the next video starts.
    guide.preview_texture = None;
    if let Some(track) = state.tv_audio_track.take() {
        let _ = state.audio_backend.unload_track(track);
    }

    // Update the guide's clock so schedule_at returns the current episode.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    guide.current_time = now;

    // Re-tune to whatever should be playing now. If the current slot has
    // very little time left (<5s), skip ahead to the next episode to avoid
    // an infinite re-tune loop (video finishes instantly, triggers another
    // auto-advance to the same nearly-finished episode).
    let catalog = guide.catalogs.get(channel_idx).and_then(|c| c.as_ref());
    let Some(catalog) = catalog else { return };
    let query_time = {
        let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, now) else {
            return;
        };
        if slot.remaining_secs < 5 {
            // Jump past current slot end to get the next episode.
            now + slot.remaining_secs + 1
        } else {
            now
        }
    };
    let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, query_time) else {
        return;
    };

    let url = oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(&slot.episode);
    let seek_secs = slot.elapsed_secs;
    let data = format!("tune_url {url} {seek_secs}");
    log::info!(
        "TV: auto-advance -> {} (seek={seek_secs}s, remaining={}s)",
        slot.episode.title,
        slot.remaining_secs,
    );

    use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
    runner.set_pending_request(TV_REQUEST_PATH.to_string(), data);
}

/// Find a TV Guide runner in either the full-screen runner or open windowed runners.
fn find_tv_guide_runner<'a>(
    app_runner: &'a mut Option<AppRunner>,
    open_runners: &'a mut [(String, AppRunner)],
) -> Option<&'a mut AppRunner> {
    if let Some(ref mut runner) = *app_runner
        && runner.title == "TV Guide"
    {
        log::trace!("TV: found TV Guide in app_runner (full-screen)");
        return Some(runner);
    }
    let found = open_runners
        .iter_mut()
        .map(|(_, runner)| runner)
        .find(|runner| runner.title == "TV Guide");
    if found.is_some() {
        log::trace!("TV: found TV Guide in open_runners (windowed)");
    }
    found
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "_video")]
mod tests {
    use super::download::parse_moov_duration;
    use super::streaming_buffer::*;

    // ---------------------------------------------------------------
    // StreamingBuffer must be Send + Sync (compile-time assertion)
    // ---------------------------------------------------------------

    const _: () = {
        fn assert_send_sync<T: Send + Sync>() {}
        fn check() {
            assert_send_sync::<StreamingBuffer>();
        }
    };

    // ---------------------------------------------------------------
    // should_throttle_pure tests
    // ---------------------------------------------------------------

    #[test]
    fn throttle_decoder_zero_no_moov_no_throttle() {
        assert!(!should_throttle_pure(0, 0, false, 0));
    }

    #[test]
    fn throttle_decoder_zero_no_moov_large_buf_no_throttle() {
        // Without moov, never throttle even with huge buffer.
        assert!(!should_throttle_pure(0, 100_000_000, false, 100_000_000));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_small_buf_no_throttle() {
        // moov found but buffer under threshold.
        assert!(!should_throttle_pure(0, 1_000_000, true, 1_000_000));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_at_threshold_no_throttle() {
        // Exactly at MAX_LOOKAHEAD -- not over, so no throttle.
        assert!(!should_throttle_pure(0, MAX_LOOKAHEAD, true, MAX_LOOKAHEAD));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_over_threshold_throttle() {
        assert!(should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 1,
            true,
            MAX_LOOKAHEAD + 1,
        ));
    }

    #[test]
    fn throttle_decoder_active_under_lookahead_no_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD - 1;
        assert!(!should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_at_boundary_no_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD;
        // received == decoder + MAX_LOOKAHEAD, not >, so no throttle.
        assert!(!should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_over_lookahead_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD + 1;
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_ignores_moov_flag() {
        // When decoder_pos > 0, moov doesn't matter.
        let decoder = 5_000_000u64;
        let received = decoder + MAX_LOOKAHEAD + 100;
        assert!(should_throttle_pure(decoder, received, false, received));
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_received_less_than_decoder() {
        // Edge: received < decoder (shouldn't happen, but shouldn't panic).
        assert!(!should_throttle_pure(100, 50, true, 50));
    }

    #[test]
    fn throttle_large_values() {
        // Multi-GB file scenario.
        let decoder = 2_000_000_000u64; // 2 GB
        let received = decoder + MAX_LOOKAHEAD + 1;
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    // ---------------------------------------------------------------
    // linear_seek_interpolation tests
    // ---------------------------------------------------------------

    #[test]
    fn seek_interpolation_zero_secs() {
        let offset = linear_seek_interpolation(0.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_at_duration() {
        let offset = linear_seek_interpolation(100.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 50_000);
    }

    #[test]
    fn seek_interpolation_half_duration() {
        let offset = linear_seek_interpolation(50.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 25_000);
    }

    #[test]
    fn seek_interpolation_beyond_duration_clamps() {
        // seek_secs > duration -> frac clamped to 1.0
        let offset = linear_seek_interpolation(200.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 50_000);
    }

    #[test]
    fn seek_interpolation_duration_zero() {
        // Edge: duration=0 -> returns mdat_offset (no division).
        let offset = linear_seek_interpolation(50.0, 0.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_negative_duration() {
        // Edge: negative duration -> returns mdat_offset.
        let offset = linear_seek_interpolation(50.0, -10.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_small_file() {
        let offset = linear_seek_interpolation(1.0, 2.0, 0, 100);
        assert_eq!(offset, 50);
    }

    #[test]
    fn seek_interpolation_large_file() {
        // 4 GB file at quarter duration.
        let file_size = 4_000_000_000u64;
        let offset = linear_seek_interpolation(25.0, 100.0, 0, file_size);
        assert_eq!(offset, 1_000_000_000);
    }

    #[test]
    fn seek_interpolation_saturates_on_large_offset() {
        // mdat_offset near u64::MAX -- addition must saturate, not wrap.
        let offset = linear_seek_interpolation(50.0, 100.0, u64::MAX - 100, 1000);
        assert_eq!(offset, u64::MAX);
    }

    // ---------------------------------------------------------------
    // parse_moov_duration tests
    // ---------------------------------------------------------------

    /// Build a minimal moov atom containing an mvhd v0 child.
    fn build_moov_v0(timescale: u32, duration: u32) -> Vec<u8> {
        // mvhd v0: version(1) + flags(3) + create(4) + mod(4)
        //          + timescale(4) + duration(4) = 20 bytes
        let mut mvhd_body = Vec::new();
        mvhd_body.push(0); // version 0
        mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
        mvhd_body.extend_from_slice(&[0; 4]); // creation_time
        mvhd_body.extend_from_slice(&[0; 4]); // modification_time
        mvhd_body.extend_from_slice(&timescale.to_be_bytes());
        mvhd_body.extend_from_slice(&duration.to_be_bytes());
        // Pad to plausible size (real mvhd has more fields).
        mvhd_body.extend_from_slice(&[0; 80]);

        let mvhd_size = (8 + mvhd_body.len()) as u32;
        let moov_size = (8 + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);
        moov
    }

    #[test]
    fn parse_moov_duration_v0() {
        let moov = build_moov_v0(1000, 60000);
        let dur = parse_moov_duration(&moov);
        assert_eq!(dur, Some(60.0));
    }

    #[test]
    fn parse_moov_duration_zero_timescale() {
        let moov = build_moov_v0(0, 60000);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_no_mvhd() {
        // moov with only a trak child, no mvhd.
        let trak_body = [0u8; 16];
        let trak_size = (8 + trak_body.len()) as u32;
        let moov_size = (8 + trak_size as usize) as u32;
        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&trak_size.to_be_bytes());
        moov.extend_from_slice(b"trak");
        moov.extend_from_slice(&trak_body);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_too_short() {
        assert_eq!(parse_moov_duration(&[0; 4]), None);
    }

    // ---------------------------------------------------------------
    // maybe_evict tests (via StreamingBuffer)
    // ---------------------------------------------------------------

    #[test]
    fn evict_small_buffer_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push less than RETAIN_BEHIND bytes.
        inner.push(&vec![0xAA; 1024]);
        let sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Enable eviction.
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        // Nothing evicted -- cursor is at 0, not past RETAIN_BEHIND.
        assert_eq!(s.base_offset, 0);
        assert_eq!(s.buf.len(), 1024);
    }

    #[test]
    fn evict_large_buffer_evicts_old_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data_size = RETAIN_BEHIND + 2 * 1024 * 1024;
        inner.push(&vec![0xBB; data_size]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // Move cursor past RETAIN_BEHIND.
        sb.pos = data_size as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        // Some data should have been evicted.
        assert!(s.base_offset > 0, "expected eviction");
        // Remaining buffer should be approximately RETAIN_BEHIND.
        assert!(
            s.buf.len() <= RETAIN_BEHIND + 1,
            "expected buf <= RETAIN_BEHIND after eviction"
        );
    }

    #[test]
    fn evict_disabled_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data_size = RETAIN_BEHIND + 2 * 1024 * 1024;
        inner.push(&vec![0xCC; data_size]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // eviction_enabled defaults to false.
        sb.pos = data_size as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, 0, "eviction should be disabled");
        assert_eq!(s.buf.len(), data_size);
    }

    #[test]
    fn evict_cursor_at_start_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&vec![0xDD; RETAIN_BEHIND + 1024]);
        let sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // pos=0 means cursor_in_buf=0, not > RETAIN_BEHIND.
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, 0);
    }

    #[test]
    fn evict_preserves_data_near_cursor() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let total = RETAIN_BEHIND * 3;
        inner.push(&vec![0xEE; total]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // Cursor at 2*RETAIN_BEHIND: evicts first RETAIN_BEHIND.
        sb.pos = (RETAIN_BEHIND * 2) as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, RETAIN_BEHIND as u64);
        assert_eq!(s.buf.len(), RETAIN_BEHIND * 2);
    }
}
