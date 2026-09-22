//! TV Guide tune/untune logic: handling tune requests and starting video playback.

use crate::app_state::AppState;
use oasis_core::backend::SdiBackend;
use oasis_core::vfs::Vfs;

/// Resolve a `tune_ch:<number>` request at [`TV_REQUEST_PATH`] (written by
/// the `tv tune` terminal command and the MCP `tune` tool) against the open
/// TV Guide: select that channel and tune it exactly like pressing Confirm
/// in the guide, which queues the guide's own `tune_url` request for
/// [`handle_tune_requests`]. Nothing consumed these requests before, so
/// both entry points silently did nothing.
///
/// [`TV_REQUEST_PATH`]: oasis_core::apps::tv_guide::TV_REQUEST_PATH
fn tune_channel_from_vfs(state: &mut AppState, vfs: &mut dyn Vfs) {
    use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
    use oasis_core::apps::tv_guide::catalog::ChannelCatalog;

    let Ok(data) = vfs.read(TV_REQUEST_PATH) else {
        return;
    };
    let Some(number) = std::str::from_utf8(&data)
        .ok()
        .and_then(|s| s.trim().strip_prefix("tune_ch:"))
        .map(str::to_string)
    else {
        return;
    };
    let _ = vfs.write(TV_REQUEST_PATH, b"");
    let Ok(number) = number.parse::<u32>() else {
        log::warn!("TV: bad channel in tune request: {number:?}");
        return;
    };
    let Some(runner) = super::find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    ) else {
        log::warn!("TV: tune_ch:{number} ignored, the TV Guide is not open");
        return;
    };
    let Some(guide) = runner.tv_guide_state() else {
        return;
    };
    let Some(index) = guide.channels.iter().position(|c| c.number == number) else {
        log::warn!("TV: tune_ch:{number}: no such channel");
        return;
    };
    // Move the selection like the arrow keys do (keeps the scroll window
    // consistent), then tune.
    while guide.selected_channel > index {
        guide.select_up();
    }
    while guide.selected_channel < index {
        guide.select_down();
    }
    let Some(req) = guide.tune() else {
        log::info!("TV: tune_ch:{number}: already tuned or no schedule yet");
        runner.refresh_tv_text();
        return;
    };
    let url = ChannelCatalog::download_url(&req.episode);
    runner.set_pending_request(
        TV_REQUEST_PATH.to_string(),
        format!("tune_url {url} {}", req.seek_secs),
    );
    runner.refresh_tv_text();
}

/// Handle TV Guide tune requests -- start in-app video player.
pub(super) fn handle_tune_requests(
    state: &mut AppState,
    backend: &mut impl SdiBackend,
    vfs: &mut dyn Vfs,
) {
    tune_channel_from_vfs(state, vfs);

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

    // Offline (headless harness): no download. Start an injected decode
    // session so the scenario can feed frames/audio through the real
    // player -> audio-backend path.
    if state.offline {
        #[cfg(feature = "_video")]
        {
            state.tv_current_url = Some(url.to_string());
            state.video_player.start_injected(preview_w, preview_h);
            setup_streaming_audio(state);
        }
        #[cfg(not(feature = "_video"))]
        log::info!("TV: offline, not starting ffmpeg playback for {url}");
        return;
    }

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

    let tls = state.net.tls_provider.clone();
    let session = start_stream_session(&mut state.video_player, url, tls, seek_secs, width, height);

    // Store session for cancellation on re-tune, and URL for dedup.
    state.tv_stream_session = Some(session);
    state.tv_current_url = Some(url.to_string());
    setup_streaming_audio(state);

    // Clear download-related state (no longer used for streaming).
    state.pending_video_download = None;
    state.tv_download_progress = None;
    state.pending_video_params = None;
}

/// Start a streaming session: a download thread feeding a
/// [`StreamingInner`](super::streaming_buffer::StreamingInner) and the
/// player's decoder reading from it.  Returns the shared buffer (the
/// session handle, cancelled on re-tune).
#[cfg(feature = "_video")]
pub(super) fn start_stream_session(
    player: &mut crate::video_player::VideoPlayer,
    url: &str,
    tls: oasis_core::net::RustlsTlsProvider,
    seek_secs: u64,
    width: u32,
    height: u32,
) -> std::sync::Arc<super::streaming_buffer::StreamingInner> {
    use std::sync::Arc;

    use super::streaming_buffer::{StreamingBuffer, StreamingInner};

    // Create a streaming buffer shared between the download thread and decoder.
    let buffer = Arc::new(StreamingInner::new());
    let reader = StreamingBuffer::new(Arc::clone(&buffer));
    let eviction_buffer = Arc::clone(&buffer);

    let url_owned = url.to_string();

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
        eviction_buffer.enable_eviction();
    });

    // Start the decoder -- it will block-read from the streaming buffer as
    // data arrives from the HTTP download.  Moov data is fetched from the
    // shared buffer on the decoder thread (not the UI thread).
    player.start_software_source(
        Box::new(reader),
        seek_secs,
        width,
        height,
        Some(on_init),
        moov_buffer,
    );
    buffer
}
