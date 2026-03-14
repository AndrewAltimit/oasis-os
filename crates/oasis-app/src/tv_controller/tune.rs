//! TV Guide tune/untune logic: handling tune requests and starting video playback.

use crate::app_state::AppState;
use oasis_core::backend::{AudioBackend, SdiBackend};
use oasis_core::vfs::Vfs;

/// Handle TV Guide tune requests -- start in-app video player.
pub(super) fn handle_tune_requests(
    state: &mut AppState,
    backend: &mut impl SdiBackend,
    vfs: &mut dyn Vfs,
) {
    let runner = super::find_tv_guide_runner(
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

    use super::streaming_buffer::{StreamingBuffer, StreamingInner};

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
        if let Err(e) =
            super::download::stream_download(&url_owned, &tls, &download_buffer, seek_secs)
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
