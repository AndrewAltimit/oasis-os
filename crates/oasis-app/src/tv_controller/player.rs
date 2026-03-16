//! Video player ticking: frame upload, audio feed, untune detection, episode auto-advance.

use crate::app_state::AppState;
use oasis_core::backend::{AudioBackend, SdiBackend};

/// Tick video player: upload frames, collect audio chunks.
pub(super) fn tick_video_player(state: &mut AppState, backend: &mut impl SdiBackend) {
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
    let runner = super::find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    if let Some(runner) = runner
        && let Some(guide) = runner.tv_guide_state()
    {
        // Destroy the guide's old texture if the video player is providing
        // a new one (prevents texture leaks from auto-advance preservation).
        if texture.is_some()
            && guide.preview_texture.is_some()
            && guide.preview_texture != texture
            && let Some(old) = guide.preview_texture.take()
        {
            let _ = backend.destroy_texture(old);
        }
        guide.preview_texture = texture;
        guide.download_status = download_status;
    }
}

/// Detect untune: video is active but guide has no tuned channel.
pub(super) fn detect_untune(state: &mut AppState, backend: &mut impl SdiBackend) {
    if !state.video_player.is_active() {
        return;
    }
    let should_stop = {
        let runner = super::find_tv_guide_runner(
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
/// Detects rapid-retune loops (playback < 3s) and skips to the next
/// episode to avoid infinite buffer-play-buffer cycles.
pub(super) fn auto_advance_episode(state: &mut AppState, backend: &mut impl SdiBackend) {
    if !state.video_player.is_finished() {
        return;
    }

    let runner = super::find_tv_guide_runner(
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

    // Detect rapid-retune: if playback lasted < 3 seconds, the video
    // is likely near EOF or has decode issues (H.264 profile mismatch).
    // Skip ahead to the next episode instead of re-tuning the same video
    // which would create an infinite buffer→play→buffer loop.
    #[cfg(feature = "_video")]
    let was_rapid = state.video_player.playback_duration_secs() < 3.0
        && state.video_player.displayed_frames() < 60;
    #[cfg(not(feature = "_video"))]
    let was_rapid = false;

    // Stop the finished player — but preserve the last frame texture so
    // the user sees a still image instead of the "Loading..." screen
    // during the brief retune/rebuffer period.
    let last_texture = state.video_player.take_texture();
    state.video_player.stop(backend);
    // Assign the preserved texture back to the guide. It will be
    // replaced by the new video's first frame once it arrives, or
    // destroyed if the channel is untuned.
    guide.preview_texture = last_texture;
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
    // very little time left (<5s), or if we detected a rapid-retune loop,
    // skip ahead to the next episode to avoid infinite re-tune cycles.
    let catalog = guide.catalogs.get(channel_idx).and_then(|c| c.as_ref());
    let Some(catalog) = catalog else { return };
    let query_time = {
        let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, now) else {
            return;
        };
        if slot.remaining_secs < 5 || was_rapid {
            if was_rapid {
                log::info!("TV: rapid-retune detected (playback < 3s), skipping to next episode",);
            }
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
