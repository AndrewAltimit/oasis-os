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

    // ---------------------------------------------------------------
    // StreamingInner: push, bytes_received, finish, cancel, set_error
    // ---------------------------------------------------------------

    #[test]
    fn inner_new_defaults() {
        let inner = StreamingInner::new();
        assert_eq!(inner.bytes_received(), 0);
        assert!(!inner.is_done());
        assert!(!inner.is_cancelled());
        assert!(inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
        let s = inner.state.lock().unwrap();
        assert!(s.buf.is_empty());
        assert_eq!(s.base_offset, 0);
        assert!(s.moov.is_none());
        assert!(s.header.is_none());
        assert!(s.atoms.is_empty());
    }

    #[test]
    fn inner_push_accumulates_data() {
        let inner = StreamingInner::new();
        inner.push(&[1, 2, 3]);
        inner.push(&[4, 5]);
        assert_eq!(inner.bytes_received(), 5);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.buf, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn inner_finish_marks_done() {
        let inner = StreamingInner::new();
        inner.push(&[0; 16]);
        assert!(!inner.is_done());
        inner.finish();
        assert!(inner.is_done());
    }

    #[test]
    fn inner_cancel_marks_done_and_cancelled() {
        let inner = StreamingInner::new();
        assert!(!inner.is_cancelled());
        inner.cancel();
        assert!(inner.is_cancelled());
        assert!(inner.is_done());
    }

    #[test]
    fn inner_set_error_marks_done_and_stores_message() {
        let inner = StreamingInner::new();
        inner.set_error("connection reset".into());
        assert!(inner.is_done());
        let err = inner.error.lock().unwrap();
        assert_eq!(err.as_deref(), Some("connection reset"));
    }

    #[test]
    fn inner_disable_probe_mode() {
        let inner = StreamingInner::new();
        assert!(inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
        inner.disable_probe_mode();
        assert!(!inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
    }

    // ---------------------------------------------------------------
    // Atom scanning: ftyp, mdat, moov detection and retention
    // ---------------------------------------------------------------

    /// Build a minimal MP4 atom with a given fourcc and body.
    fn build_atom(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let size = (8 + body.len()) as u32;
        let mut atom = Vec::new();
        atom.extend_from_slice(&size.to_be_bytes());
        atom.extend_from_slice(fourcc);
        atom.extend_from_slice(body);
        atom
    }

    #[test]
    fn scan_atoms_ftyp_only() {
        let inner = StreamingInner::new();
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert_eq!(s.atoms[0].0, 0); // offset 0
        assert_eq!(s.atoms[0].1, 24); // 8 header + 16 body
    }

    #[test]
    fn scan_atoms_ftyp_mdat() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"mdat", &[0xFF; 32]));
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 2);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert_eq!(s.atoms[1].2, *b"mdat");
        assert_eq!(s.atoms[1].0, 24); // after ftyp
    }

    #[test]
    fn scan_atoms_retains_moov() {
        let inner = StreamingInner::new();
        let moov_body = [0xAB; 64];
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &moov_body));
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov should be retained");
        let (offset, moov_data) = s.moov.as_ref().unwrap();
        assert_eq!(*offset, 24); // after ftyp
        // moov data includes the atom header (8 bytes) + body
        assert_eq!(moov_data.len(), 8 + moov_body.len());
    }

    #[test]
    fn scan_atoms_incomplete_moov_waits() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Write a moov header claiming 100 bytes but only provide 20.
        let moov_size: u32 = 100;
        data.extend_from_slice(&moov_size.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0; 12]); // only 12 of 92 body bytes
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        // ftyp should be scanned, but moov should NOT be in atoms yet
        // (incomplete).
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert!(s.moov.is_none());
    }

    #[test]
    fn scan_atoms_extended_size() {
        let inner = StreamingInner::new();
        // Build an atom with extended size (size32 == 1).
        let body = [0u8; 32];
        let total_size: u64 = 16 + body.len() as u64; // 16-byte header + body
        let mut atom = Vec::new();
        atom.extend_from_slice(&1u32.to_be_bytes()); // size32 = 1 (extended)
        atom.extend_from_slice(b"free");
        atom.extend_from_slice(&total_size.to_be_bytes());
        atom.extend_from_slice(&body);
        inner.push(&atom);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].1, total_size);
        assert_eq!(s.atoms[0].2, *b"free");
    }

    #[test]
    fn scan_atoms_header_retained() {
        let inner = StreamingInner::new();
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        let s = inner.state.lock().unwrap();
        assert!(s.header.is_some(), "header should be retained after ftyp");
        let hdr = s.header.as_ref().unwrap();
        // Header should include at least the ftyp atom.
        assert!(hdr.len() >= ftyp.len());
    }

    #[test]
    fn finish_handles_size_zero_atom() {
        // An atom with size==0 extends to EOF. `finish()` should handle it.
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Append an mdat atom with size=0 (extends to EOF).
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0xFF; 100]);
        inner.push(&data);
        // Before finish, the size-0 atom is not scanned.
        {
            let s = inner.state.lock().unwrap();
            assert_eq!(s.atoms.len(), 1); // only ftyp
        }
        inner.finish();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 2);
        assert_eq!(s.atoms[1].2, *b"mdat");
        // The size should be total - scan_pos (rest of file).
        let expected_size = data.len() as u64 - 24; // after ftyp
        assert_eq!(s.atoms[1].1, expected_size);
    }

    #[test]
    fn finish_retains_moov_at_end() {
        // Moov at end of file (common in non-faststart MP4s).
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        let mdat = build_atom(b"mdat", &[0xFF; 100]);
        data.extend_from_slice(&mdat);
        // Moov at end with size=0 (extends to EOF).
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        // mvhd inside moov.
        let mvhd_body = vec![0u8; 100];
        let mvhd = build_atom(b"mvhd", &mvhd_body);
        data.extend_from_slice(&mvhd);
        inner.push(&data);
        // mdat was scanned but moov (size=0) was not.
        inner.finish();
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov at end should be retained on finish");
    }

    // ---------------------------------------------------------------
    // StreamingBuffer Read: probe_mode returns zeros, no decoder_pos
    // ---------------------------------------------------------------

    #[test]
    fn read_probe_mode_returns_zeros() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push some data and set total_size.
        inner.push(&[0xAA; 1024]);
        inner
            .total_size
            .store(4096, std::sync::atomic::Ordering::Release);
        inner.finish(); // mark done so reads don't block
        // probe_mode is true by default.
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Seek past available data.
        sb.pos = 2048;
        let mut buf = [0xFF; 64];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 64);
        // All zeros (probe mode).
        assert!(buf.iter().all(|&b| b == 0), "probe reads should be zeros");
    }

    #[test]
    fn read_probe_mode_does_not_update_decoder_pos() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0xAA; 1024]);
        inner
            .total_size
            .store(4096, std::sync::atomic::Ordering::Release);
        inner.finish();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 2048;
        let mut buf = [0; 64];
        let _ = sb.read(&mut buf).unwrap();
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 0, "decoder_pos must not update during probe_mode");
    }

    #[test]
    fn read_normal_mode_updates_decoder_pos() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0xBB; 256]);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 64];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 64);
        assert!(buf.iter().all(|&b| b == 0xBB));
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 64, "decoder_pos should advance after normal read");
    }

    #[test]
    fn read_normal_mode_correct_data() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        inner.push(&data);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 200];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 200);
        assert_eq!(&buf[..], &data[..]);
    }

    #[test]
    fn read_partial_then_rest() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[1, 2, 3, 4, 5, 6, 7, 8]);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 4];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(sb.pos, 4);
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(buf, [5, 6, 7, 8]);
        assert_eq!(sb.pos, 8);
    }

    #[test]
    fn read_eof_when_done_and_past_data() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner
            .total_size
            .store(16, std::sync::atomic::Ordering::Release);
        inner.disable_probe_mode();
        inner.finish();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 16; // at EOF
        let mut buf = [0xFF; 8];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 0, "should return EOF at end of data");
    }

    #[test]
    fn read_returns_error_on_cancel() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner.disable_probe_mode();
        inner.cancel();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read after cancel should error");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn read_returns_error_on_set_error() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner.disable_probe_mode();
        inner.set_error("network timeout".into());
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read after set_error should error");
    }

    #[test]
    fn read_from_retained_moov() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build ftyp + moov.
        let moov_body = vec![0xCD; 64];
        let mut data = build_atom(b"ftyp", &[0; 16]);
        let moov_offset = data.len() as u64;
        data.extend_from_slice(&build_atom(b"moov", &moov_body));
        inner.push(&data);
        inner.disable_probe_mode();
        // Verify moov was retained.
        {
            let s = inner.state.lock().unwrap();
            assert!(s.moov.is_some());
        }
        // Now evict the buffer but moov should still be readable.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.clear();
            s.base_offset = data.len() as u64;
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = moov_offset;
        let mut buf = [0; 72]; // 8 header + 64 body
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 72);
        // Verify moov header bytes.
        let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(size, 72);
        assert_eq!(&buf[4..8], b"moov");
    }

    #[test]
    fn read_from_retained_header() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let ftyp = build_atom(b"ftyp", &[0xAA; 16]);
        inner.push(&ftyp);
        inner.disable_probe_mode();
        // Evict the buffer but header should be retained.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.clear();
            s.base_offset = ftyp.len() as u64;
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 0;
        let mut buf = [0; 24];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 24);
        assert_eq!(&buf[4..8], b"ftyp");
    }

    #[test]
    fn read_evicted_region_returns_error() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 1024]);
        inner.disable_probe_mode();
        inner.finish();
        // Manually evict by advancing base_offset.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.drain(..512);
            s.base_offset = 512;
            s.header = None; // clear header so it doesn't serve from there
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 0; // before base_offset
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read from evicted region should error");
    }

    // ---------------------------------------------------------------
    // StreamingBuffer Seek trait
    // ---------------------------------------------------------------

    #[test]
    fn seek_start() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::Start(42)).unwrap();
        assert_eq!(pos, 42);
        assert_eq!(sb.pos, 42);
    }

    #[test]
    fn seek_current_forward() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 100;
        let pos = sb.seek(std::io::SeekFrom::Current(50)).unwrap();
        assert_eq!(pos, 150);
    }

    #[test]
    fn seek_current_backward() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 100;
        let pos = sb.seek(std::io::SeekFrom::Current(-30)).unwrap();
        assert_eq!(pos, 70);
    }

    #[test]
    fn seek_negative_position_errors() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 10;
        let result = sb.seek(std::io::SeekFrom::Current(-20));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn seek_end_with_known_size() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(1000, std::sync::atomic::Ordering::Release);
        inner.finish(); // so it doesn't block waiting for total_size
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::End(-100)).unwrap();
        assert_eq!(pos, 900);
    }

    #[test]
    fn seek_end_at_zero_offset() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(500, std::sync::atomic::Ordering::Release);
        inner.finish();
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::End(0)).unwrap();
        assert_eq!(pos, 500);
    }

    // ---------------------------------------------------------------
    // wait_for_moov
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_moov_immediate() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build a file with moov.
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &[0; 32]));
        inner.push(&data);
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_some(), "moov should be immediately available");
    }

    #[test]
    fn wait_for_moov_cancelled_returns_none() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.cancel();
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_none(), "cancelled session should return None");
    }

    #[test]
    fn wait_for_moov_done_no_moov_returns_none() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push data without a moov atom and finish.
        inner.push(&build_atom(b"ftyp", &[0; 16]));
        inner.push(&build_atom(b"mdat", &[0; 100]));
        inner.finish();
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_none(), "no moov in data should return None");
    }

    #[test]
    fn wait_for_moov_arrives_from_background() {
        use std::sync::Arc;
        let inner = Arc::new(StreamingInner::new());
        let inner2 = Arc::clone(&inner);
        // Spawn a thread that pushes moov after a short delay.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut data = build_atom(b"ftyp", &[0; 16]);
            data.extend_from_slice(&build_atom(b"moov", &[0; 64]));
            inner2.push(&data);
        });
        let result = inner.wait_for_moov(std::time::Duration::from_secs(2));
        assert!(
            result.is_some(),
            "moov pushed from background should be found"
        );
    }

    // ---------------------------------------------------------------
    // wait_for_buffered
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_buffered_immediate() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 4096]);
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(ok);
    }

    #[test]
    fn wait_for_buffered_cancelled() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.cancel();
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        // Returns false because buffer is empty on cancel.
        assert!(!ok);
    }

    #[test]
    fn wait_for_buffered_done_with_partial_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 512]); // less than requested
        inner.finish();
        // Should return true because some data is available (not empty).
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(ok);
    }

    #[test]
    fn wait_for_buffered_done_empty() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.finish();
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(!ok, "empty buffer on done should return false");
    }

    // ---------------------------------------------------------------
    // should_throttle integration (via StreamingInner, not _pure)
    // ---------------------------------------------------------------

    #[test]
    fn should_throttle_integration_no_throttle_initially() {
        let inner = StreamingInner::new();
        inner.push(&[0; 1024]);
        assert!(!inner.should_throttle());
    }

    #[test]
    fn should_throttle_integration_with_moov_and_large_buffer() {
        let inner = StreamingInner::new();
        // Push ftyp + moov to trigger moov retention.
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &[0; 32]));
        // Add enough extra data to exceed MAX_LOOKAHEAD.
        let extra = vec![0u8; MAX_LOOKAHEAD as usize + 1];
        data.extend_from_slice(&extra);
        inner.push(&data);
        // decoder_pos is 0, has_moov is true, buf > MAX_LOOKAHEAD.
        assert!(
            inner.should_throttle(),
            "should throttle with moov and large buffer"
        );
    }

    #[test]
    fn should_throttle_integration_decoder_active() {
        let inner = StreamingInner::new();
        let data_size = MAX_LOOKAHEAD as usize + 1_000_000;
        inner.push(&vec![0; data_size]);
        // Set decoder_pos to somewhere in the stream.
        inner
            .decoder_pos
            .store(1000, std::sync::atomic::Ordering::Release);
        // received (data_size) > decoder_pos (1000) + MAX_LOOKAHEAD
        assert!(inner.should_throttle());
    }

    // ---------------------------------------------------------------
    // StreamingBuffer: Read + Seek round-trip (probe -> disable -> read)
    // ---------------------------------------------------------------

    #[test]
    fn probe_then_normal_read_roundtrip() {
        use std::io::{Read, Seek};
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Simulate a small MP4 file.
        let mut mp4 = build_atom(b"ftyp", &[0x11; 16]);
        mp4.extend_from_slice(&build_atom(b"mdat", &[0x22; 200]));
        mp4.extend_from_slice(&build_atom(b"moov", &[0x33; 64]));
        let file_size = mp4.len() as u64;
        inner.push(&mp4);
        inner
            .total_size
            .store(file_size, std::sync::atomic::Ordering::Release);
        inner.finish();

        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));

        // Phase 1: probe mode -- skip ahead, reads return zeros.
        sb.pos = 100;
        let mut buf = [0xFF; 16];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 16);
        // In probe mode, data beyond the sliding buffer returns zeros.
        // But pos=100 is within the buffer, so it returns real data.
        // Let's check decoder_pos was NOT updated in probe mode.
        // Actually pos=100 is within the buffer so it returns real data,
        // but decoder_pos should still not be updated.
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 0, "decoder_pos must not update during probe");

        // Phase 2: disable probe, seek back, read real data.
        inner.disable_probe_mode();
        sb.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut header = [0; 8];
        let n = sb.read(&mut header).unwrap();
        assert_eq!(n, 8);
        assert_eq!(&header[4..8], b"ftyp");
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 8, "decoder_pos should update after probe disabled");
    }

    // ---------------------------------------------------------------
    // Multiple push + incremental atom scanning
    // ---------------------------------------------------------------

    #[test]
    fn incremental_atom_scanning() {
        let inner = StreamingInner::new();
        // Push ftyp in full.
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        {
            let s = inner.state.lock().unwrap();
            assert_eq!(s.atoms.len(), 1);
            assert_eq!(s.atoms_scanned_to, 24);
        }
        // Push first part of mdat header (only 4 bytes).
        inner.push(&100u32.to_be_bytes());
        {
            let s = inner.state.lock().unwrap();
            // Still only 1 atom -- header incomplete.
            assert_eq!(s.atoms.len(), 1);
        }
        // Push the rest of mdat header + some body.
        let mut rest = Vec::new();
        rest.extend_from_slice(b"mdat");
        rest.extend_from_slice(&[0xFF; 88]); // 100 - 8 = 92 body, gave 88
        inner.push(&rest);
        {
            let s = inner.state.lock().unwrap();
            // mdat header is now complete (24 + 100 = 124 bytes total,
            // we have 24 + 4 + 4 + 88 = 120; 92 body bytes needed,
            // 88 provided -- atom scanned because we have the header).
            assert_eq!(s.atoms.len(), 2);
            assert_eq!(s.atoms[1].2, *b"mdat");
        }
    }

    // ---------------------------------------------------------------
    // VideoSource trait
    // ---------------------------------------------------------------

    #[test]
    fn video_source_byte_len_unknown() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let sb = StreamingBuffer::new(inner);
        assert_eq!(sb.byte_len(), None);
    }

    #[test]
    fn video_source_byte_len_known() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(12345, std::sync::atomic::Ordering::Release);
        let sb = StreamingBuffer::new(inner);
        assert_eq!(sb.byte_len(), Some(12345));
    }

    #[test]
    fn video_source_is_seekable() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let sb = StreamingBuffer::new(inner);
        assert!(sb.is_seekable());
    }
}
