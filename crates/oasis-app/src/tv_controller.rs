//! TV Guide subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles TV catalog
//! fetching, video player ticking, tune/untune requests, and audio streaming.

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
    #[cfg(feature = "video-decode")]
    poll_video_download(state, backend);
    tick_video_player(state, backend);
    detect_untune(state, backend);
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
    log::info!("TV: starting video player: {url} seek={seek_secs}s");

    // Stop any existing video session.
    state.video_player.stop(backend);
    if let Some(track) = state.tv_audio_track.take() {
        let _ = state.audio_backend.unload_track(track);
    }

    // Compute preview dimensions (match guide.rs header layout).
    let at = &state.active_theme;
    let usable_h = at
        .screen_h
        .saturating_sub(at.statusbar_height + at.bottombar_height);
    let header_h = (usable_h * 20 / 100).max(60);
    let preview_w = (at.screen_w / 5).max(80).saturating_sub(2);
    let preview_h = header_h.saturating_sub(16).saturating_sub(2);

    #[cfg(feature = "video-decode")]
    start_video_download(state, url, seek_secs, preview_w, preview_h);

    #[cfg(not(feature = "video-decode"))]
    start_ffmpeg_playback(state, url, seek_secs, preview_w, preview_h);
}

/// Start ffmpeg-based playback (the legacy path).
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

/// Start downloading the video to a temp file for software decode.
#[cfg(feature = "video-decode")]
fn start_video_download(state: &mut AppState, url: &str, seek_secs: u64, width: u32, height: u32) {
    use crate::app_state::PendingVideoParams;

    let url_owned = url.to_string();
    let tls = state.net.tls_provider.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        log::info!("TV: video download thread started: {url_owned}");
        let result = download_to_temp(&url_owned, &tls);
        log::info!("TV: video download finished (ok={})", result.is_ok(),);
        let _ = tx.send(result);
    });

    state.pending_video_download = Some(rx);
    state.pending_video_params = Some(PendingVideoParams {
        url: url.to_string(),
        seek_secs,
        width,
        height,
    });
}

/// Download a URL to a temporary file. Returns the path on success.
#[cfg(feature = "video-decode")]
fn download_to_temp(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
) -> Result<std::path::PathBuf, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;
    use std::io::Write;

    // Parse URL into host + path.
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("unsupported URL scheme: {url}"))?;
    let (host, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((stripped, "/".to_string()));

    let tmp_path = std::env::temp_dir().join(format!(
        "oasis_tv_{}.mp4",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let tcp = net
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;

    let mut stream = tls
        .connect_tls(tcp, host)
        .map_err(|e| format!("TLS: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Connection: close\r\nAccept: */*\r\n\r\n"
    );
    let req_bytes = request.as_bytes();
    let mut written = 0;
    while written < req_bytes.len() {
        match stream.write(&req_bytes[written..]) {
            Ok(n) => written += n,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("write: {e}"));
            },
        }
    }

    // Read full response.
    let mut buf = [0u8; 8192];
    let mut response = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout downloading video".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if !response.is_empty() {
                    break;
                }
                return Err(format!("read: {e}"));
            },
        }
    }

    // Strip HTTP headers to get body.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no header/body separator in response".to_string())?;
    let body = &response[header_end + 4..];

    let mut file =
        std::fs::File::create(&tmp_path).map_err(|e| format!("create temp file: {e}"))?;
    file.write_all(body)
        .map_err(|e| format!("write temp file: {e}"))?;

    log::info!(
        "TV: downloaded {} bytes to {}",
        body.len(),
        tmp_path.display()
    );
    Ok(tmp_path)
}

/// Poll pending video download and start software decode when ready.
#[cfg(feature = "video-decode")]
fn poll_video_download(state: &mut AppState, backend: &mut impl SdiBackend) {
    let Some(ref rx) = state.pending_video_download else {
        return;
    };

    match rx.try_recv() {
        Ok(Ok(path)) => {
            state.pending_video_download = None;
            let params = state.pending_video_params.take();
            if let Some(params) = params {
                log::info!("TV: download complete, starting software decode");
                state.tv_video_cache_path = Some(path.clone());
                state.video_player.start_software(
                    path,
                    params.seek_secs,
                    params.width,
                    params.height,
                );
                setup_streaming_audio(state);
            }
        },
        Ok(Err(e)) => {
            state.pending_video_download = None;
            let params = state.pending_video_params.take();
            log::warn!("TV: video download failed: {e}, falling back to ffmpeg");
            // Fallback to ffmpeg.
            if let Some(params) = params {
                start_ffmpeg_playback(
                    state,
                    &params.url,
                    params.seek_secs,
                    params.width,
                    params.height,
                );
            }
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.pending_video_download = None;
            let params = state.pending_video_params.take();
            log::error!("TV: video download thread died, falling back to ffmpeg");
            if let Some(params) = params {
                start_ffmpeg_playback(
                    state,
                    &params.url,
                    params.seek_secs,
                    params.width,
                    params.height,
                );
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            // Still downloading, show progress indicator on guide if needed.
            let runner = find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner
                && let Some(guide) = runner.tv_guide_state()
            {
                // Clear texture while downloading (shows "Loading..." in guide).
                guide.preview_texture = None;
            }
            let _ = backend; // suppress unused warning
        },
    }
}

/// Tick video player: upload frames, collect audio chunks.
fn tick_video_player(state: &mut AppState, backend: &mut impl SdiBackend) {
    let (texture, audio_output) = state.video_player.tick(backend);

    // Feed audio to the streaming track.
    if let Some(track) = state.tv_audio_track {
        match &audio_output {
            crate::video_player::AudioOutput::Mp3Chunks(chunks) => {
                for chunk in chunks {
                    let _ = state.audio_backend.feed_data(track, chunk);
                }
            },
            #[cfg(feature = "video-decode")]
            crate::video_player::AudioOutput::PcmF32(chunks) => {
                for chunk in chunks {
                    let _ = state.audio_backend.feed_pcm_f32(
                        track,
                        &chunk.pcm_f32,
                        chunk.channels,
                        chunk.sample_rate,
                    );
                }
            },
            crate::video_player::AudioOutput::None => {},
        }
    }

    // Update the guide's preview texture.
    let runner = find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    if let Some(runner) = runner
        && let Some(guide) = runner.tv_guide_state()
    {
        guide.preview_texture = texture;
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
        #[cfg(feature = "video-decode")]
        {
            state.pending_video_download = None;
            state.pending_video_params = None;
            if let Some(path) = state.tv_video_cache_path.take()
                && let Err(e) = std::fs::remove_file(&path)
            {
                log::warn!("TV: failed to remove temp file {}: {e}", path.display());
            }
        }
    }
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
