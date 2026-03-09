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

    // Compute preview dimensions (match guide.rs header layout).
    let at = &state.active_theme;
    let usable_h = at
        .screen_h
        .saturating_sub(at.statusbar_height + at.bottombar_height);
    let header_h = (usable_h * 20 / 100).max(60);
    let preview_w = (at.screen_w / 5).max(80).saturating_sub(2);
    let preview_h = header_h.saturating_sub(16).saturating_sub(2);
    log::info!("TV: preview {preview_w}x{preview_h}, seek={seek_secs}s");

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

/// Start streaming video decode — downloads in background while decoding
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
        // File missing or too small (failed download) — remove stale entry.
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
        if let Err(e) = stream_download(&url_owned, &tls, &download_buffer, seek_secs)
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

    // Start the decoder — it will block-read from the streaming buffer as
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

// ---------------------------------------------------------------------------
// StreamingBuffer: Read + Seek over a sliding-window in-memory buffer
// ---------------------------------------------------------------------------

/// How much data to retain behind the decoder's read cursor.
/// Allows minor backward seeks without re-downloading.
#[cfg(feature = "_video")]
const RETAIN_BEHIND: usize = 4 * 1024 * 1024; // 4 MB

/// Maximum bytes the download thread may be ahead of the decoder's read
/// cursor before it pauses.  Keeps memory bounded to ~16 MB lookahead.
#[cfg(feature = "_video")]
const MAX_LOOKAHEAD: u64 = 16 * 1024 * 1024; // 16 MB

/// Minimum bytes of body data that must be buffered before the decoder
/// starts reading.  Ensures the decoder doesn't block on CDN latency
/// during initial playback.  The browser's `<video>` element does this
/// automatically; we must do it explicitly for the desktop pipeline.
#[cfg(feature = "_video")]
pub(crate) const MIN_PREBUFFER: u64 = 2 * 1024 * 1024; // 2 MB

/// Maximum buffer size before a warning is logged (during init phase).
/// The demuxer's `read_to_end()` + `seek(0)` pattern requires the full file
/// in memory during init; eviction MUST NOT remove data before seek-back.
/// Actual eviction only begins after the `on_init` callback enables it.
#[cfg(feature = "_video")]
const INIT_WARN_THRESHOLD: usize = 128 * 1024 * 1024; // 128 MB

/// Time without receiving any body bytes before we consider the connection
/// stalled and attempt a reconnect (inspired by ffmpeg's `reconnect_on_http`).
#[cfg(feature = "_video")]
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum number of reconnect attempts on a stalled Range download.
#[cfg(feature = "_video")]
const MAX_RECONNECTS: u32 = 5;

/// Short-seek read-through threshold: if the seek position is within this
/// many bytes of data already downloaded, continue the linear download
/// instead of reconnecting with a new Range request.  Inspired by ffmpeg's
/// `avio.c` short-seek optimization (ffmpeg defaults to ~half buffer size).
/// We use a larger value since HTTP Range reconnects are expensive.
#[cfg(feature = "_video")]
const SHORT_SEEK_THRESHOLD: u64 = 4 * 1024 * 1024; // 4 MB

/// Sliding-window buffer state, protected by a mutex.
#[cfg(feature = "_video")]
struct SlidingState {
    /// Contiguous buffer of the currently-retained byte range.
    /// `buf[0]` corresponds to file offset `base_offset`.
    buf: Vec<u8>,
    /// File offset of `buf[0]`. Increases as old data is evicted.
    base_offset: u64,
    /// Total bytes received from the network so far (monotonically increasing,
    /// never decremented by eviction).
    bytes_received: u64,
    /// Retained moov atom — copied out so it survives eviction.
    /// `(file_offset, data)`.
    moov: Option<(u64, Vec<u8>)>,
    /// Retained file header (ftyp atom, typically 24-32 bytes).
    /// Kept so symphonia can probe the container format after seek restart.
    header: Option<Vec<u8>>,
    /// Parsed top-level atom boundaries: `(offset, size, fourcc)`.
    /// Used to detect moov/mdat locations.
    atoms: Vec<(u64, u64, [u8; 4])>,
    /// How far we have scanned for atom headers.
    atoms_scanned_to: u64,
}

/// Shared inner state for the streaming buffer, fed by the download thread.
#[cfg(feature = "_video")]
pub(crate) struct StreamingInner {
    state: std::sync::Mutex<SlidingState>,
    /// Total content length from HTTP Content-Length header.
    pub(crate) total_size: std::sync::atomic::AtomicU64,
    /// Whether the download is complete (all data received).
    done: std::sync::atomic::AtomicBool,
    /// Whether this session has been cancelled (new channel tuned).
    cancelled: std::sync::atomic::AtomicBool,
    /// Download error message, if any.
    error: std::sync::Mutex<Option<String>>,
    /// Signalled when new data arrives or download completes.
    condvar: std::sync::Condvar,
    /// Decoder's current read position (updated by `StreamingBuffer::read`).
    /// The download thread uses this to throttle when too far ahead.
    decoder_pos: std::sync::atomic::AtomicU64,
    /// When true, reads beyond retained data return zeros instantly
    /// instead of blocking.  Used during symphonia's probe phase so
    /// ignore_bytes() can skip the mdat body without downloading it.
    probe_mode: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "_video")]
impl StreamingInner {
    fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(SlidingState {
                buf: Vec::with_capacity(4 * 1024 * 1024),
                base_offset: 0,
                bytes_received: 0,
                moov: None,
                header: None,
                atoms: Vec::new(),
                atoms_scanned_to: 0,
            }),
            total_size: std::sync::atomic::AtomicU64::new(0),
            done: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            error: std::sync::Mutex::new(None),
            condvar: std::sync::Condvar::new(),
            decoder_pos: std::sync::atomic::AtomicU64::new(0),
            probe_mode: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Append data from the download thread. Scans for top-level MP4 atoms
    /// and retains the moov atom separately.
    fn push(&self, chunk: &[u8]) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.buf.extend_from_slice(chunk);
        s.bytes_received += chunk.len() as u64;

        // Scan for top-level MP4 atom headers in newly arrived data.
        Self::scan_atoms(&mut s);

        // Log progress periodically (every 4MB).
        let received = s.bytes_received;
        let buf_len = s.buf.len();
        if received.is_multiple_of(4 * 1024 * 1024) {
            let total = self.total_size.load(std::sync::atomic::Ordering::Relaxed);
            let pct = if total > 0 {
                format!(" ({}%)", received * 100 / total)
            } else {
                String::new()
            };
            log::info!(
                "TV: download progress: {:.1}MB received{pct}, buffer={:.1}MB",
                received as f64 / (1024.0 * 1024.0),
                buf_len as f64 / (1024.0 * 1024.0),
            );
        }
        // Warn once when buffer is large (init phase holds entire file).
        if buf_len > INIT_WARN_THRESHOLD && (buf_len - chunk.len()) <= INIT_WARN_THRESHOLD {
            log::warn!(
                "TV: buffer reached {:.0}MB during init (file held in memory until demuxer probes)",
                buf_len as f64 / (1024.0 * 1024.0),
            );
        }

        self.condvar.notify_all();
    }

    /// Scan top-level MP4 atoms starting from `atoms_scanned_to`.
    /// Each atom: 4-byte big-endian size + 4-byte fourcc. If size==1,
    /// the next 8 bytes are a 64-bit extended size.
    fn scan_atoms(s: &mut SlidingState) {
        let total = s.base_offset + s.buf.len() as u64;
        loop {
            let scan_pos = s.atoms_scanned_to;
            // Need at least 8 bytes for the atom header.
            if scan_pos + 8 > total {
                break;
            }
            let buf_off = (scan_pos - s.base_offset) as usize;
            let size32 = u32::from_be_bytes([
                s.buf[buf_off],
                s.buf[buf_off + 1],
                s.buf[buf_off + 2],
                s.buf[buf_off + 3],
            ]);
            let fourcc: [u8; 4] = [
                s.buf[buf_off + 4],
                s.buf[buf_off + 5],
                s.buf[buf_off + 6],
                s.buf[buf_off + 7],
            ];

            let atom_size = if size32 == 1 {
                // Extended size — need 16 bytes total header.
                if scan_pos + 16 > total {
                    break;
                }
                u64::from_be_bytes([
                    s.buf[buf_off + 8],
                    s.buf[buf_off + 9],
                    s.buf[buf_off + 10],
                    s.buf[buf_off + 11],
                    s.buf[buf_off + 12],
                    s.buf[buf_off + 13],
                    s.buf[buf_off + 14],
                    s.buf[buf_off + 15],
                ])
            } else if size32 == 0 {
                // Atom extends to EOF — we don't know total yet.
                // Use bytes_received as a conservative estimate; we'll
                // re-scan when more data arrives.
                break;
            } else {
                size32 as u64
            };

            if atom_size < 8 {
                break; // Invalid atom, stop scanning.
            }

            log::debug!(
                "TV: MP4 atom '{}' at offset {scan_pos}, size {atom_size}",
                String::from_utf8_lossy(&fourcc),
            );

            // For moov: don't advance past it until the full atom is
            // available and retained.  Otherwise we'd skip over moov and
            // never revisit it on subsequent push() calls.
            if &fourcc == b"moov" && s.moov.is_none() && scan_pos + atom_size > total {
                // moov found but incomplete — wait for more data.
                break;
            }

            s.atoms.push((scan_pos, atom_size, fourcc));

            // Retain the full moov atom for the decoder.
            if &fourcc == b"moov" && scan_pos + atom_size <= total && s.moov.is_none() {
                let start = buf_off;
                let end = start + atom_size as usize;
                let moov_data = s.buf[start..end].to_vec();
                log::info!(
                    "TV: retained moov atom ({} bytes) at offset {scan_pos}",
                    moov_data.len(),
                );
                s.moov = Some((scan_pos, moov_data));
            }

            s.atoms_scanned_to = scan_pos + atom_size;
        }

        // Update retained file header — includes all scanned atom
        // headers so symphonia can discover the file structure after
        // a seek restart.  We save up to atoms_scanned_to + 16
        // (for the next atom's header), capped to avoid saving large
        // moov bodies (moov is retained separately).
        if s.base_offset == 0 && s.atoms_scanned_to > 0 {
            let scan_end = (s.atoms_scanned_to - s.base_offset) as usize;
            let keep = (scan_end + 16).min(s.buf.len());
            // Don't replace an existing larger header.
            let current_len = s.header.as_ref().map_or(0, |h| h.len());
            if keep > current_len {
                s.header = Some(s.buf[..keep].to_vec());
            }
        }
    }

    /// Mark download as complete. Final atom scan + moov retention.
    fn finish(&self) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // Final scan — handle size==0 atoms (extends to EOF).
            let total = s.base_offset + s.buf.len() as u64;
            if s.atoms_scanned_to < total {
                let scan_pos = s.atoms_scanned_to;
                let buf_off = (scan_pos - s.base_offset) as usize;
                if buf_off + 8 <= s.buf.len() {
                    let size32 = u32::from_be_bytes([
                        s.buf[buf_off],
                        s.buf[buf_off + 1],
                        s.buf[buf_off + 2],
                        s.buf[buf_off + 3],
                    ]);
                    let fourcc: [u8; 4] = [
                        s.buf[buf_off + 4],
                        s.buf[buf_off + 5],
                        s.buf[buf_off + 6],
                        s.buf[buf_off + 7],
                    ];
                    let atom_size = if size32 == 0 {
                        total - scan_pos
                    } else if size32 == 1 && buf_off + 16 <= s.buf.len() {
                        u64::from_be_bytes([
                            s.buf[buf_off + 8],
                            s.buf[buf_off + 9],
                            s.buf[buf_off + 10],
                            s.buf[buf_off + 11],
                            s.buf[buf_off + 12],
                            s.buf[buf_off + 13],
                            s.buf[buf_off + 14],
                            s.buf[buf_off + 15],
                        ])
                    } else {
                        size32 as u64
                    };

                    s.atoms.push((scan_pos, atom_size, fourcc));
                    s.atoms_scanned_to = scan_pos + atom_size;

                    // Retain moov if found at end of file (only if complete).
                    if &fourcc == b"moov" && s.moov.is_none() {
                        let expected_end = buf_off + atom_size as usize;
                        if expected_end <= s.buf.len() {
                            let moov_data = s.buf[buf_off..expected_end].to_vec();
                            log::info!(
                                "TV: retained moov atom ({} bytes) at offset \
                                 {scan_pos} (end of file)",
                                moov_data.len(),
                            );
                            s.moov = Some((scan_pos, moov_data));
                        } else {
                            log::warn!(
                                "TV: moov at offset {scan_pos} truncated \
                                 ({} of {} bytes available)",
                                s.buf.len() - buf_off,
                                atom_size,
                            );
                        }
                    }
                }
            }
        }
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Set an error and mark as done.
    fn set_error(&self, msg: String) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Total bytes received (for progress/logging).
    pub(crate) fn bytes_received(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes_received
    }

    /// Whether download is complete.
    fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Cancel the download and unblock any waiting readers.
    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Whether this session has been cancelled.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Wait until moov data is available (or download finishes/cancels).
    /// Returns a clone of the moov data if found.
    pub(crate) fn wait_for_moov(&self, timeout: std::time::Duration) -> Option<Vec<u8>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some((_, ref data)) = s.moov {
                return Some(data.clone());
            }
            if self.is_cancelled() {
                return None;
            }
            // After download finishes, `finish()` does a final atom scan
            // that may store moov under the lock.  Re-check moov one last
            // time (we already hold the lock via `s`) before giving up.
            if self.is_done() {
                // `s` is the current MutexGuard — moov was just checked
                // above and was None, so the download truly has no moov.
                // Drop and re-acquire to pick up any store that happened
                // between `finish()`'s unlock and `done=true`.
                drop(s);
                let s2 = self.state.lock().unwrap_or_else(|e| e.into_inner());
                return s2.moov.as_ref().map(|(_, data)| data.clone());
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let wait = remaining.min(std::time::Duration::from_millis(200));
            let result = self
                .condvar
                .wait_timeout(s, wait)
                .unwrap_or_else(|e| e.into_inner());
            s = result.0;
        }
    }

    /// Block until at least `min_bytes` of body data have been buffered,
    /// or until the download finishes/is cancelled.  Returns `true` if
    /// the minimum was reached, `false` on timeout/cancel/done.
    pub(crate) fn wait_for_buffered(&self, min_bytes: u64, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if s.buf.len() as u64 >= min_bytes {
                log::info!(
                    "TV: prebuffer ready: {:.1}MB buffered",
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                );
                return true;
            }
            if self.is_cancelled() || self.is_done() {
                log::info!(
                    "TV: prebuffer ended early: {:.1}MB buffered (done={}, cancelled={})",
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                    self.is_done(),
                    self.is_cancelled(),
                );
                // If some data arrived, proceed anyway.
                return !s.buf.is_empty();
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                log::warn!(
                    "TV: prebuffer timeout: {:.1}MB buffered (wanted {:.1}MB)",
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                    min_bytes as f64 / (1024.0 * 1024.0),
                );
                // Proceed with whatever we have.
                return !s.buf.is_empty();
            }
            let wait = remaining.min(std::time::Duration::from_millis(200));
            let result = self
                .condvar
                .wait_timeout(s, wait)
                .unwrap_or_else(|e| e.into_inner());
            s = result.0;
        }
    }

    /// Disable probe mode so reads block on real data instead of
    /// returning zeros.  Called after the decoder's probe phase completes.
    pub(crate) fn disable_probe_mode(&self) {
        log::info!("TV: StreamingBuffer probe_mode disabled — reads will now block");
        self.probe_mode
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns `true` if the download is far enough ahead of the decoder
    /// that it should pause to avoid unbounded memory growth.
    fn should_throttle(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let buf_size = s.buf.len() as u64;
        let has_moov = s.moov.is_some();
        let received = s.bytes_received;
        drop(s);

        let decoder = self.decoder_pos.load(std::sync::atomic::Ordering::Relaxed);
        should_throttle_pure(decoder, received, has_moov, buf_size)
    }
}

/// Pure logic for throttle decision, extracted for testability.
///
/// - `decoder_pos > 0`: throttle if `received > decoder_pos + MAX_LOOKAHEAD`
/// - `decoder_pos == 0`: throttle if moov found AND `buf_size > MAX_LOOKAHEAD`
#[cfg(feature = "_video")]
fn should_throttle_pure(
    decoder_pos: u64,
    bytes_received: u64,
    has_moov: bool,
    buf_size: u64,
) -> bool {
    if decoder_pos > 0 {
        bytes_received > decoder_pos + MAX_LOOKAHEAD
    } else {
        has_moov && buf_size > MAX_LOOKAHEAD
    }
}

/// Pure logic for linear seek interpolation within mdat.
///
/// Returns estimated byte offset = `mdat_offset + (seek_secs / duration) * mdat_size`.
#[cfg(feature = "_video")]
fn linear_seek_interpolation(
    seek_secs: f64,
    duration: f64,
    mdat_offset: u64,
    mdat_size: u64,
) -> u64 {
    if duration <= 0.0 {
        return mdat_offset;
    }
    let frac = (seek_secs / duration).clamp(0.0, 1.0);
    mdat_offset + (frac * mdat_size as f64) as u64
}

/// A reader cursor over a `StreamingInner` sliding-window buffer.
///
/// Implements `Read + Seek` with blocking semantics. Reads block until data
/// is available at the current position or the download completes/errors.
/// After each read, data far behind the cursor is evicted to bound memory
/// usage. The moov atom is retained separately and never evicted.
///
/// Eviction is disabled by default and must be enabled by setting the
/// `eviction_enabled` flag after the demuxer has finished its initial
/// full-file scan (avcC + probe). This prevents evicting data that the
/// demuxer needs to seek back to during initialization.
#[cfg(feature = "_video")]
struct StreamingBuffer {
    inner: std::sync::Arc<StreamingInner>,
    pos: u64,
    /// Whether sliding-window eviction is active. Starts `false` to allow
    /// the demuxer to `read_to_end` + `seek(Start(0))` without data loss.
    eviction_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Whether we've logged a "waiting for data" message (avoids log spam).
    logged_wait: bool,
    /// Whether we've logged a "gap-fill zeros" warning (avoids log spam).
    logged_gap: bool,
}

#[cfg(feature = "_video")]
impl StreamingBuffer {
    fn new(inner: std::sync::Arc<StreamingInner>) -> Self {
        Self {
            inner,
            pos: 0,
            eviction_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            logged_wait: false,
            logged_gap: false,
        }
    }

    /// Evict data that is far behind the read cursor, except moov.
    ///
    /// Eviction is ONLY active after `on_init` enables it (post-demuxer probe).
    /// During init the demuxer does `read_to_end()` + `seek(0)`, so ALL data
    /// must remain in the buffer — evicting early breaks format probing.
    fn maybe_evict(&self) {
        if !self
            .eviction_enabled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }

        let mut s = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let cursor_in_buf = self.pos.saturating_sub(s.base_offset) as usize;
        if cursor_in_buf > RETAIN_BEHIND {
            let evict = cursor_in_buf - RETAIN_BEHIND;
            let evict = evict.min(s.buf.len());
            if evict > 0 {
                s.buf.drain(..evict);
                s.base_offset += evict as u64;
                log::debug!(
                    "TV: evicted {:.1}MB, window now {:.1}MB ({}-{})",
                    evict as f64 / (1024.0 * 1024.0),
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                    s.base_offset,
                    s.base_offset + s.buf.len() as u64,
                );
            }
        }
    }
}

#[cfg(feature = "_video")]
impl std::io::Read for StreamingBuffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = loop {
            // Check for cancellation first.
            if self.inner.is_cancelled() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "streaming session cancelled",
                ));
            }
            // Check for errors.
            if let Some(ref e) = *self.inner.error.lock().unwrap_or_else(|e| e.into_inner()) {
                return Err(std::io::Error::other(e.clone()));
            }

            let s = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());

            // Try serving from the retained file header (ftyp).
            if let Some(ref hdr) = s.header
                && (self.pos as usize) < hdr.len()
            {
                let local = self.pos as usize;
                let n = buf.len().min(hdr.len() - local);
                buf[..n].copy_from_slice(&hdr[local..local + n]);
                self.pos += n as u64;
                break n;
            }

            // Try serving from the retained moov atom.
            if let Some((moov_off, ref moov_data)) = s.moov {
                let moov_end = moov_off + moov_data.len() as u64;
                if self.pos >= moov_off && self.pos < moov_end {
                    let local = (self.pos - moov_off) as usize;
                    let n = buf.len().min(moov_data.len() - local);
                    buf[..n].copy_from_slice(&moov_data[local..local + n]);
                    self.pos += n as u64;
                    break n;
                }
            }

            // Try serving from the sliding buffer.
            let buf_end = s.base_offset + s.buf.len() as u64;
            if self.pos >= s.base_offset && self.pos < buf_end {
                let local = (self.pos - s.base_offset) as usize;
                let n = buf.len().min(s.buf.len() - local);
                buf[..n].copy_from_slice(&s.buf[local..local + n]);
                self.pos += n as u64;
                break n;
            }

            // Position is at or beyond file end — EOF.
            let total = self
                .inner
                .total_size
                .load(std::sync::atomic::Ordering::Relaxed);
            if total > 0 && self.pos >= total {
                break 0; // EOF
            }

            // In probe mode: return zeros for any position not covered
            // by retained data or the sliding buffer.  This lets
            // symphonia's ignore_bytes() skip the mdat body instantly
            // without downloading it.
            let in_probe = self
                .inner
                .probe_mode
                .load(std::sync::atomic::Ordering::Relaxed);
            if in_probe {
                // Fill with zeros up to base_offset, total_size, or
                // buf.len() — whichever comes first.
                let limit = if total > 0 { total } else { u64::MAX };
                let remaining = (limit - self.pos) as usize;
                let n = buf.len().min(remaining);
                if n == 0 {
                    break 0; // EOF
                }
                buf[..n].fill(0);
                self.pos += n as u64;
                break n;
            }

            // Position is in the gap (evicted data) — return zeros.
            // WARNING: this feeds zeros to the demuxer which may corrupt
            // the stream.  Log once per gap encounter so we can diagnose.
            if self.pos < s.base_offset {
                if !self.logged_gap {
                    log::warn!(
                        "TV: StreamingBuffer gap-fill zeros at {:.1}MB \
                         (base_offset={:.1}MB, gap={:.0}KB)",
                        self.pos as f64 / (1024.0 * 1024.0),
                        s.base_offset as f64 / (1024.0 * 1024.0),
                        (s.base_offset - self.pos) as f64 / 1024.0,
                    );
                    self.logged_gap = true;
                }
                let gap_remaining = (s.base_offset - self.pos) as usize;
                let n = buf.len().min(gap_remaining);
                buf[..n].fill(0);
                self.pos += n as u64;
                break n;
            }

            // Position is beyond available data.
            if self.inner.is_done() {
                break 0; // EOF
            }

            // Log once when first entering a wait state.
            if !self.logged_wait {
                log::info!(
                    "TV: StreamingBuffer waiting for data at {:.1}MB \
                     (buffer: {:.1}MB..{:.1}MB, {:.1}MB available)",
                    self.pos as f64 / (1024.0 * 1024.0),
                    s.base_offset as f64 / (1024.0 * 1024.0),
                    buf_end as f64 / (1024.0 * 1024.0),
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                );
                self.logged_wait = true;
            }

            // Block until more data arrives.
            let _guard = self
                .inner
                .condvar
                .wait_timeout(s, std::time::Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
        };

        // Update decoder position and evict old data after successful read.
        // Skip during probe_mode — probe reads return zeros and don't
        // represent real decoder progress.  Updating decoder_pos during
        // probe would race with the download thread's seek-restart logic
        // that resets decoder_pos to the Range start offset.
        if n > 0
            && !self
                .inner
                .probe_mode
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.inner
                .decoder_pos
                .store(self.pos, std::sync::atomic::Ordering::Relaxed);
            self.maybe_evict();
        }

        Ok(n)
    }
}

#[cfg(feature = "_video")]
impl std::io::Seek for StreamingBuffer {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::sync::atomic::Ordering;

        let new_pos = match pos {
            std::io::SeekFrom::Start(p) => p as i64,
            std::io::SeekFrom::Current(off) => (self.pos as i64).saturating_add(off),
            std::io::SeekFrom::End(off) => {
                // Wait for total_size from Content-Length header.
                let total = self.inner.total_size.load(Ordering::Acquire);
                if total == 0 && !self.inner.is_done() {
                    let mut attempts = 0;
                    loop {
                        let t = self.inner.total_size.load(Ordering::Acquire);
                        if t > 0 || self.inner.is_done() {
                            break;
                        }
                        attempts += 1;
                        if attempts > 300 {
                            return Err(std::io::Error::other(
                                "timeout waiting for content length",
                            ));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                let total = self.inner.total_size.load(Ordering::Acquire);
                if total == 0 {
                    (self.inner.bytes_received() as i64).saturating_add(off)
                } else {
                    (total as i64).saturating_add(off)
                }
            },
        };

        if new_pos < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek to negative position",
            ));
        }

        let old_pos = self.pos;
        self.pos = new_pos as u64;
        self.logged_wait = false; // reset so next wait/gap is logged
        // Log significant seeks (> 1MB jump) for debugging streaming issues.
        let jump = self.pos.abs_diff(old_pos);
        if jump > 1024 * 1024 {
            log::info!(
                "TV: StreamingBuffer seek {:.1}MB -> {:.1}MB (jump {:.1}MB)",
                old_pos as f64 / (1024.0 * 1024.0),
                self.pos as f64 / (1024.0 * 1024.0),
                jump as f64 / (1024.0 * 1024.0),
            );
        }
        Ok(self.pos)
    }
}

// SAFETY: StreamingBuffer is Send + Sync because StreamingInner uses
// Arc<Mutex<..>> + atomics for all shared state.
#[cfg(feature = "_video")]
unsafe impl Send for StreamingBuffer {}
#[cfg(feature = "_video")]
unsafe impl Sync for StreamingBuffer {}

#[cfg(feature = "_video")]
impl oasis_video::VideoSource for StreamingBuffer {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        let total = self
            .inner
            .total_size
            .load(std::sync::atomic::Ordering::Acquire);
        if total > 0 { Some(total) } else { None }
    }
}

// ---------------------------------------------------------------------------
// HTTP streaming download
// ---------------------------------------------------------------------------

/// Fetch a byte range from a URL via HTTP Range request.
/// Returns the raw body bytes on success.
#[cfg(feature = "_video")]
fn fetch_range(
    tls: &oasis_core::net::RustlsTlsProvider,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    fetch_range_inner(tls, url, start, end, 3)
}

#[cfg(feature = "_video")]
fn fetch_range_inner(
    tls: &oasis_core::net::RustlsTlsProvider,
    url: &str,
    start: u64,
    end: u64,
    redirects_left: u8,
) -> Result<Vec<u8>, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("unsupported URL: {url}"))?;
    let (host, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((stripped, "/".to_string()));

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let tcp = net
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;
    let mut stream = tls
        .connect_tls(tcp, host)
        .map_err(|e| format!("TLS: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Range: bytes={start}-{}\r\nConnection: close\r\n\r\n",
        end.saturating_sub(1),
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

    // Read response.
    let mut response = Vec::new();
    let mut buf = [0u8; 8192];
    let mut deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout".into());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            },
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

    // Split headers from body.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header terminator")?;
    let header_str = String::from_utf8_lossy(&response[..header_end]);
    let status = header_str
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if redirects_left == 0 {
            return Err("too many redirects".into());
        }
        let location = header_str
            .lines()
            .find(|l| l.len() > 9 && l[..9].eq_ignore_ascii_case("location:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()));
        if let Some(loc) = location {
            return fetch_range_inner(tls, &loc, start, end, redirects_left - 1);
        }
        return Err(format!("HTTP {status} no Location"));
    }

    if status != 206 && status != 200 {
        return Err(format!("HTTP {status}"));
    }

    let body = response[header_end + 4..].to_vec();
    log::info!("TV: Range response: HTTP {status}, {} bytes", body.len(),);
    Ok(body)
}

/// Check if a moov-at-start file should restart download from a seek position.
/// Returns `Some(byte_offset)` if restart is worthwhile.
#[cfg(feature = "_video")]
fn check_moov_at_start_restart(s: &SlidingState, seek_secs: u64) -> Option<u64> {
    let moov_data = s.moov.as_ref().map(|(_, d)| d)?;

    // Compute seek position two ways and take the minimum.
    // Our exact seek-byte from MP4 sample tables only considers the video
    // track, but symphonia's own seek considers both audio and video tracks
    // and may land at a significantly earlier byte position.  Using the
    // minimum of both estimates ensures the Range download covers wherever
    // symphonia will actually seek to.
    let exact_byte = oasis_video::demux_lite::seek_byte_from_moov(moov_data, seek_secs as f64);

    let linear_byte = parse_moov_duration(moov_data).and_then(|dur| {
        let (mdat_off, mdat_size) = s
            .atoms
            .iter()
            .find(|(_, size, cc)| cc == b"mdat" && *size > 1024)
            .map(|(off, size, _)| (*off, *size))?;
        Some(linear_seek_interpolation(
            seek_secs as f64,
            dur,
            mdat_off,
            mdat_size,
        ))
    });

    // Use the LINEAR estimate as start_from (it tracks where symphonia
    // actually seeks, since symphonia uses time-based coarse seek which
    // maps roughly linearly within the mdat).  The exact seek-byte from
    // our sample tables may differ significantly because it only
    // considers the video track's stco/stsz tables.
    let seek_byte = match (linear_byte, exact_byte) {
        (Some(linear), Some(exact)) => {
            log::info!(
                "TV: seek estimates: linear={:.1}MB, exact={:.1}MB, using linear",
                linear as f64 / (1024.0 * 1024.0),
                exact as f64 / (1024.0 * 1024.0),
            );
            linear
        },
        (Some(linear), None) => linear,
        (None, Some(exact)) => {
            log::info!(
                "TV: exact seek-byte from sample tables: {:.1}MB",
                exact as f64 / (1024.0 * 1024.0),
            );
            exact
        },
        (None, None) => return None,
    };

    // Clamp seek byte to file boundaries.
    let total = s.bytes_received.max(
        s.atoms
            .iter()
            .map(|(off, sz, _)| off + sz)
            .max()
            .unwrap_or(0),
    );
    let seek_byte = seek_byte.min(total);
    // Back up 2MB before the estimated position to give symphonia room
    // to find sync points — its internal seek may land somewhat before
    // our estimate.
    let start_from = seek_byte.saturating_sub(2 * 1024 * 1024);
    let downloaded = s.bytes_received;
    if start_from > downloaded + SHORT_SEEK_THRESHOLD {
        log::info!(
            "TV: moov-at-start: seek={seek_secs}s -> byte ~{:.1}MB \
             (downloaded {:.1}MB), restarting from {:.1}MB",
            seek_byte as f64 / (1024.0 * 1024.0),
            downloaded as f64 / (1024.0 * 1024.0),
            start_from as f64 / (1024.0 * 1024.0),
        );
        Some(start_from)
    } else {
        None
    }
}

/// Parse tail data (fetched via Range) looking for the moov atom.
/// If found, retains it in the buffer and notifies waiters.
///
/// The tail data typically starts in the middle of an mdat atom (raw
/// video/audio data), so we cannot walk atom boundaries from offset 0.
/// Instead, scan for the `moov` fourcc and validate the atom header.
#[cfg(feature = "_video")]
fn parse_tail_for_moov(
    buffer: &StreamingInner,
    tail_data: &[u8],
    tail_offset: u64,
    content_length: u64,
    seek_secs: u64,
) {
    // Scan for the 'moov' fourcc.  In an MP4 atom header the layout is
    // [4-byte big-endian size][4-byte fourcc], so 'moov' appears at
    // offset+4 of the atom header.  We look for the fourcc and then
    // validate the preceding size field.
    let needle = b"moov";
    let mut search_from = 4usize; // need ≥4 bytes before fourcc for size
    let found = loop {
        if search_from + 4 > tail_data.len() {
            break None;
        }
        let haystack = &tail_data[search_from..];
        let pos = haystack.windows(4).position(|w| w == needle);
        let Some(rel) = pos else { break None };
        let fourcc_off = search_from + rel;
        let atom_start = fourcc_off - 4; // size field is 4 bytes before fourcc
        let size32 = u32::from_be_bytes([
            tail_data[atom_start],
            tail_data[atom_start + 1],
            tail_data[atom_start + 2],
            tail_data[atom_start + 3],
        ]);
        let atom_size = if size32 == 1 && atom_start + 16 <= tail_data.len() {
            let b = &tail_data[atom_start + 8..atom_start + 16];
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
        } else if size32 == 0 {
            tail_data.len() - atom_start
        } else {
            size32 as usize
        };
        // Validate: atom must be ≥8 bytes and fit within the tail data.
        if atom_size >= 8 && atom_start + atom_size <= tail_data.len() {
            break Some((atom_start, atom_size));
        }
        // False positive — keep scanning past this occurrence.
        search_from = fourcc_off + 4;
    };

    let Some((atom_start, atom_size)) = found else {
        log::info!(
            "TV: tail probe: no moov found in last {:.1}MB of file",
            tail_data.len() as f64 / (1024.0 * 1024.0)
        );
        return;
    };

    let file_off = tail_offset + atom_start as u64;
    let moov_data = tail_data[atom_start..atom_start + atom_size].to_vec();
    log::info!(
        "TV: pre-fetched moov atom ({} bytes) at file offset {}",
        moov_data.len(),
        file_off,
    );

    // If seeking, compute byte offset and set base_offset so the
    // main download thread can restart from the seek position.
    if seek_secs > 0 {
        // Compute seek position two ways and take the minimum.
        // Our exact seek-byte only considers video track, but symphonia
        // may seek to an earlier position when considering both tracks.
        let exact_byte = oasis_video::demux_lite::seek_byte_from_moov(&moov_data, seek_secs as f64);

        let linear_byte = parse_moov_duration(&moov_data)
            .map(|dur| linear_seek_interpolation(seek_secs as f64, dur, 0, file_off));

        let seek_byte = match (linear_byte, exact_byte) {
            (Some(linear), Some(exact)) => {
                log::info!(
                    "TV: tail seek estimates: linear={:.1}MB, exact={:.1}MB, using linear",
                    linear as f64 / (1024.0 * 1024.0),
                    exact as f64 / (1024.0 * 1024.0),
                );
                linear
            },
            (Some(linear), None) => linear,
            (None, Some(exact)) => exact,
            (None, None) => {
                // Cannot estimate — retain moov and let decoder seek.
                let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                s.moov = Some((file_off, moov_data));
                buffer.condvar.notify_all();
                return;
            },
        };
        // Clamp to file size to avoid requesting bytes beyond EOF.
        let seek_byte = seek_byte.min(content_length.saturating_sub(1));
        // Back up 2MB for symphonia's seek margin.
        let start_from = seek_byte.saturating_sub(2 * 1024 * 1024);
        log::info!(
            "TV: tail probe: seek={seek_secs}s -> byte ~{:.1}MB, \
             need download from {:.1}MB",
            seek_byte as f64 / (1024.0 * 1024.0),
            start_from as f64 / (1024.0 * 1024.0),
        );
        let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
        s.moov = Some((file_off, moov_data));
        // Set base_offset so the main download loop knows where
        // to restart from (checked via `restart_offset`).
        if start_from > s.bytes_received + SHORT_SEEK_THRESHOLD {
            // Retain file header (ftyp + mdat header) for symphonia
            // probe.  Upgrade if current header is smaller.
            if !s.buf.is_empty() {
                let current_len = s.header.as_ref().map_or(0, |h| h.len());
                let keep = s.buf.len().min(4096);
                if keep > current_len {
                    s.header = Some(s.buf[..keep].to_vec());
                }
            }
            s.base_offset = start_from;
            s.bytes_received = start_from;
            s.buf.clear();
        }
        drop(s);
        buffer.condvar.notify_all();
        // Signal content_length for seek-based range download.
        buffer
            .total_size
            .store(content_length, std::sync::atomic::Ordering::Release);
        return;
    }

    let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
    s.moov = Some((file_off, moov_data));
    buffer.condvar.notify_all();
}

fn parse_moov_duration(moov_data: &[u8]) -> Option<f64> {
    // moov is a container atom. Scan its children for mvhd.
    let mut pos = 8usize; // skip moov header (size + fourcc)
    while pos + 8 <= moov_data.len() {
        let size = u32::from_be_bytes([
            moov_data[pos],
            moov_data[pos + 1],
            moov_data[pos + 2],
            moov_data[pos + 3],
        ]) as usize;
        if size < 8 || pos + size > moov_data.len() {
            break;
        }
        let fourcc = &moov_data[pos + 4..pos + 8];
        if fourcc == b"mvhd" {
            // mvhd: version(1) + flags(3) + ...
            let data = &moov_data[pos + 8..pos + size];
            if data.is_empty() {
                return None;
            }
            let version = data[0];
            if version == 0 && data.len() >= 20 {
                // v0 layout after version(1)+flags(3): create(4) + mod(4) + timescale(4) + duration(4)
                // timescale starts at byte 12, duration at byte 16
                let timescale = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
                let duration = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            } else if version == 1 && data.len() >= 32 {
                // v1 layout after version(1)+flags(3): create(8) + mod(8) + timescale(4) + duration(8)
                // timescale starts at byte 20, duration at byte 24
                let timescale = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                let duration = u64::from_be_bytes([
                    data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
                ]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            }
            return None;
        }
        pos += size;
    }
    None
}

/// Open a TLS connection, send an HTTP Range request, and return the stream
/// plus any leftover body bytes from the header read.
///
/// Returns `(stream, leftover_body_bytes)` on success.
#[cfg(feature = "_video")]
fn open_range_connection(
    host: &str,
    path: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    range_start: u64,
    range_end: u64,
) -> Result<(Box<dyn oasis_core::backend::NetworkStream>, Vec<u8>), String> {
    open_range_connection_inner(host, path, tls, range_start, range_end, 5)
}

#[cfg(feature = "_video")]
fn open_range_connection_inner(
    host: &str,
    path: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    range_start: u64,
    range_end: u64,
    redirects_left: u8,
) -> Result<(Box<dyn oasis_core::backend::NetworkStream>, Vec<u8>), String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let tcp = net
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;
    let mut stream = tls
        .connect_tls(tcp, host)
        .map_err(|e| format!("TLS: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Range: bytes={range_start}-{range_end}\r\nConnection: close\r\n\r\n",
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

    // Read headers.
    let mut header_buf = Vec::with_capacity(4096);
    let mut buf = [0u8; 8192];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

    let leftover_start = loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout reading Range headers".into());
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                return Err("connection closed before headers".into());
            },
            Ok(n) => {
                header_buf.extend_from_slice(&buf[..n]);
                if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            },
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("read headers: {e}"));
            },
        }
    };

    let header_str = String::from_utf8_lossy(&header_buf[..leftover_start]);
    let status_line = header_str.lines().next().unwrap_or("");
    log::info!("TV: Range response: {status_line}");

    let status = header_str
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    // Follow redirects (archive.org 302 → CDN node).
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if redirects_left == 0 {
            return Err("too many redirects on Range request".into());
        }
        let location = header_str
            .lines()
            .find(|l| l.len() > 9 && l[..9].eq_ignore_ascii_case("location:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()));
        if let Some(loc) = location {
            log::info!("TV: Range redirect {status} -> {loc}");
            let redir = loc
                .strip_prefix("https://")
                .or_else(|| loc.strip_prefix("http://"))
                .ok_or_else(|| format!("bad redirect: {loc}"))?;
            let (redir_host, redir_path) = redir
                .split_once('/')
                .map(|(h, p)| (h, format!("/{p}")))
                .unwrap_or((redir, "/".to_string()));
            drop(stream);
            return open_range_connection_inner(
                redir_host,
                &redir_path,
                tls,
                range_start,
                range_end,
                redirects_left - 1,
            );
        }
        return Err(format!("HTTP {status} with no Location header"));
    }

    match status {
        206 => {
            // Partial Content — server honoured the Range request.
        },
        200 => {
            // Server ignored Range header and is sending the full file
            // from byte 0.  Pushing this data at `range_start` would
            // corrupt the stream with misaligned data.
            return Err("HTTP 200 (server ignored Range header) — cannot resume".into());
        },
        416 => {
            return Err("HTTP 416 Range Not Satisfiable".into());
        },
        _ if (200..300).contains(&status) => {
            // Other 2xx — unexpected but not fatal.
            log::warn!("TV: unexpected HTTP {status} for Range request");
        },
        _ => {
            return Err(format!("HTTP {status} on Range request"));
        },
    }

    let leftover = header_buf[leftover_start..].to_vec();
    Ok((stream, leftover))
}

/// Download from a specific byte offset using HTTP Range request.
/// Pushes data into the buffer starting at `start_offset`.
///
/// Implements stall detection with automatic reconnect (inspired by ffmpeg's
/// `reconnect_on_http`): if no data arrives for `STALL_TIMEOUT` seconds,
/// the connection is dropped and a new Range request resumes from the last
/// byte received.  Up to `MAX_RECONNECTS` reconnect attempts are made.
#[cfg(feature = "_video")]
fn stream_download_range(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &StreamingInner,
    start_offset: u64,
    total_size: u64,
) -> Result<(), String> {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("unsupported URL: {url}"))?;
    let (host, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((stripped, "/".to_string()));

    let range_end = total_size.saturating_sub(1);
    let mut current_offset = start_offset;
    let mut reconnects = 0u32;

    'outer: loop {
        if buffer.is_cancelled() {
            log::info!("TV: range download cancelled before connect");
            return Ok(());
        }

        let (mut stream, leftover) =
            match open_range_connection(host, &path, tls, current_offset, range_end) {
                Ok(pair) => pair,
                Err(e) => {
                    if reconnects < MAX_RECONNECTS {
                        reconnects += 1;
                        log::warn!(
                            "TV: Range connect failed ({e}), reconnect {reconnects}/\
                         {MAX_RECONNECTS} from {:.1}MB",
                            current_offset as f64 / (1024.0 * 1024.0),
                        );
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                    buffer.finish();
                    return Err(e);
                },
            };

        if !leftover.is_empty() {
            buffer.push(&leftover);
            current_offset += leftover.len() as u64;
        }

        // Stream body with stall detection.
        log::info!(
            "TV: range body loop starting at {:.1}MB (reconnect {reconnects})",
            current_offset as f64 / (1024.0 * 1024.0),
        );
        let mut buf = [0u8; 8192];
        let mut last_data_time = std::time::Instant::now();
        let mut was_throttled = false;
        let mut first_data_logged = current_offset > start_offset;

        loop {
            if buffer.is_cancelled() {
                log::info!(
                    "TV: range download cancelled ({:.1}MB received)",
                    (current_offset - start_offset) as f64 / (1024.0 * 1024.0),
                );
                return Ok(());
            }
            if buffer.should_throttle() {
                std::thread::sleep(std::time::Duration::from_millis(100));
                was_throttled = true;
                continue;
            }
            // After exiting throttle, reset stall timer since the pause
            // was intentional (decoder was lagging, not the network).
            if was_throttled {
                last_data_time = std::time::Instant::now();
                was_throttled = false;
            }
            match stream.read(&mut buf) {
                Ok(0) => {
                    log::info!(
                        "TV: range body EOF at {:.1}MB (received {:.1}MB from {:.1}MB)",
                        current_offset as f64 / (1024.0 * 1024.0),
                        (current_offset - start_offset) as f64 / (1024.0 * 1024.0),
                        start_offset as f64 / (1024.0 * 1024.0),
                    );
                    break 'outer;
                }, // Clean EOF
                Ok(n) => {
                    if !first_data_logged {
                        log::info!(
                            "TV: range download first data: {n} bytes \
                             (buffer at {:.1}MB)",
                            buffer.bytes_received() as f64 / (1024.0 * 1024.0),
                        );
                        first_data_logged = true;
                    }
                    current_offset += n as u64;
                    buffer.push(&buf[..n]);
                    last_data_time = std::time::Instant::now();
                },
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("WouldBlock") || msg.contains("would block") {
                        // Check for stall.
                        if last_data_time.elapsed() > STALL_TIMEOUT {
                            if reconnects >= MAX_RECONNECTS {
                                log::warn!(
                                    "TV: stall detected, max reconnects \
                                     ({MAX_RECONNECTS}) exhausted"
                                );
                                if current_offset > start_offset {
                                    break 'outer; // partial success
                                }
                                buffer.finish();
                                return Err("stalled, max reconnects exhausted".into());
                            }
                            reconnects += 1;
                            log::info!(
                                "TV: stall detected ({:.0}s no data), \
                                 reconnect {reconnects}/{MAX_RECONNECTS} \
                                 from {:.1}MB",
                                last_data_time.elapsed().as_secs_f64(),
                                current_offset as f64 / (1024.0 * 1024.0),
                            );
                            drop(stream);
                            continue 'outer;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    // Hard error — try reconnect or finish.
                    if reconnects < MAX_RECONNECTS && current_offset < total_size {
                        reconnects += 1;
                        log::warn!(
                            "TV: read error ({msg}), reconnect \
                             {reconnects}/{MAX_RECONNECTS} from {:.1}MB",
                            current_offset as f64 / (1024.0 * 1024.0),
                        );
                        drop(stream);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue 'outer;
                    }
                    if current_offset > start_offset {
                        break 'outer; // partial success
                    }
                    buffer.finish();
                    return Err(format!("read: {msg}"));
                },
            }
        }
    }

    let received = buffer.bytes_received();
    log::info!(
        "TV: range download complete: {:.1}MB received (from offset {:.1}MB, \
         {reconnects} reconnects)",
        (received - start_offset) as f64 / (1024.0 * 1024.0),
        start_offset as f64 / (1024.0 * 1024.0),
    );
    buffer.finish();
    Ok(())
}

/// Download URL data into a `StreamingInner` buffer (follows redirects).
#[cfg(feature = "_video")]
fn stream_download(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &std::sync::Arc<StreamingInner>,
    seek_secs: u64,
) -> Result<(), String> {
    let original_url = url.to_string();
    stream_download_inner(url, tls, buffer, 5, seek_secs, &original_url)
}

#[cfg(feature = "_video")]
fn stream_download_inner(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &std::sync::Arc<StreamingInner>,
    redirects_left: u8,
    seek_secs: u64,
    original_url: &str,
) -> Result<(), String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;
    use std::sync::atomic::Ordering;

    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("unsupported URL scheme: {url}"))?;
    let (host, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((stripped, "/".to_string()));

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

    // Read HTTP headers.
    let mut header_buf = Vec::with_capacity(4096);
    let mut buf = [0u8; 8192];
    let mut deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    let (header_end, leftover_start) = loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout reading headers".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err("connection closed before headers complete".to_string()),
            Ok(n) => {
                header_buf.extend_from_slice(&buf[..n]);
                if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (pos, pos + 4);
                }
            },
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("read headers: {e}"));
            },
        }
    };

    let header_str = String::from_utf8_lossy(&header_buf[..header_end]);
    let status = header_str
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    // Follow redirects.
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if redirects_left == 0 {
            return Err("too many redirects".to_string());
        }
        let location = header_str
            .lines()
            .find(|l| l.len() > 9 && l[..9].eq_ignore_ascii_case("location:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()));
        if let Some(loc) = location {
            log::info!("TV: stream redirect {status} -> {loc}");
            drop(stream);
            return stream_download_inner(
                &loc,
                tls,
                buffer,
                redirects_left - 1,
                seek_secs,
                original_url,
            );
        }
        return Err(format!("HTTP {status} with no Location header"));
    }

    if status != 200 {
        return Err(format!("HTTP {status}"));
    }

    let content_length: u64 = header_str
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(0);

    if content_length > 0 {
        buffer.total_size.store(content_length, Ordering::Release);
    }

    // Push leftover body bytes from the header read.
    let leftover = &header_buf[leftover_start..];
    if !leftover.is_empty() {
        buffer.push(leftover);
    }

    // Track whether we've already checked for moov-based seek restart.
    let mut checked_seek_restart = false;

    // Deferred tail probe: only launch after we've received >2MB without
    // finding moov.  For moov-at-start files (moov within first ~1.2MB),
    // the linear stream discovers moov before the threshold, so no tail
    // probe is launched.  This avoids concurrent HTTPS connections to the
    // CDN which causes connection throttling (0 body bytes on the Range
    // download while the tail probe consumes bandwidth).
    let mut tail_probe_launched = content_length <= 10 * 1024 * 1024; // skip for small files
    // Threshold must exceed the largest plausible moov-at-start atom
    // (observed up to ~4MB) plus margin, so moov is fully retained
    // before we decide whether to launch the tail probe.
    const TAIL_PROBE_THRESHOLD: u64 = 8 * 1024 * 1024;

    // Stream remaining body into the shared buffer.
    loop {
        if buffer.is_cancelled() {
            log::info!("TV: download cancelled");
            return Ok(());
        }

        // Check for moov-based seek restart BEFORE throttle check so that
        // when the tail probe finds moov (and clears the buffer), we
        // immediately issue the Range restart instead of sleeping.
        if !checked_seek_restart && seek_secs > 0 && content_length > 10 * 1024 * 1024 {
            let restart_from = {
                let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.moov.is_some() {
                    checked_seek_restart = true;
                    // If base_offset was moved far ahead by the tail probe
                    // (moov-at-end case), restart from there.
                    if s.base_offset > 4 * 1024 * 1024 && s.buf.len() < 1024 * 1024 {
                        Some(s.base_offset)
                    } else {
                        // moov found at start — check if seek position is far ahead.
                        self::check_moov_at_start_restart(&s, seek_secs)
                    }
                } else {
                    None
                }
            };
            if let Some(start_from) = restart_from {
                // Clear buffer and set base_offset for the restart.
                {
                    let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                    // For moov-at-start: retain the entire current buffer
                    // (ftyp + moov + mdat header) so symphonia can probe
                    // the full atom structure after restart.  At this point
                    // the buffer is typically only ~1-2MB.
                    // For moov-at-end: the tail probe already stored moov
                    // separately; only retain if base_offset is still 0
                    // (moov-at-start case).
                    if s.base_offset == 0 && s.moov.is_some() && !s.buf.is_empty() {
                        let full_header = s.buf.clone();
                        s.moov = Some((s.base_offset, full_header));
                    }
                    s.base_offset = start_from;
                    s.bytes_received = start_from;
                    s.buf.clear();
                }
                // Reset decoder_pos so throttle doesn't think we're
                // far ahead of the decoder (decoder_pos was left at the
                // probe position ~1MB while bytes_received jumps to
                // the seek restart offset).
                buffer
                    .decoder_pos
                    .store(start_from, std::sync::atomic::Ordering::Relaxed);
                log::info!(
                    "TV: restarting download from byte {:.1}MB via Range",
                    start_from as f64 / (1024.0 * 1024.0),
                );
                // Use original archive.org URL for Range requests —
                // open_range_connection follows the 302 redirect to the CDN.
                // This avoids 401 errors from CDN nodes that reject direct
                // Range requests without a fresh redirect.
                return stream_download_range(
                    original_url,
                    tls,
                    buffer,
                    start_from,
                    content_length,
                );
            }
        }

        // Deferred tail probe: if we've downloaded >8MB and moov still not
        // found, launch the tail probe now.  This ensures moov-at-start files
        // never launch a competing concurrent connection.  Also skip if moov
        // was already found and a seek restart is pending/done.
        if !tail_probe_launched
            && !checked_seek_restart
            && buffer.bytes_received() > TAIL_PROBE_THRESHOLD
        {
            let has_moov = buffer
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .moov
                .is_some();
            if has_moov {
                // moov already found in linear stream — no tail probe needed.
                tail_probe_launched = true;
                log::info!("TV: moov found at start, skipping tail probe");
            } else {
                tail_probe_launched = true;
                let tail_buffer = std::sync::Arc::clone(buffer);
                let tail_url = original_url.to_string();
                let tail_tls = tls.clone();
                std::thread::spawn(move || {
                    let tail_size = (8 * 1024 * 1024u64).min(content_length / 4);
                    let tail_offset = content_length.saturating_sub(tail_size);
                    log::info!(
                        "TV: probing tail {:.1}MB of {:.0}MB file \
                         for moov via Range",
                        tail_size as f64 / (1024.0 * 1024.0),
                        content_length as f64 / (1024.0 * 1024.0),
                    );
                    match fetch_range(&tail_tls, &tail_url, tail_offset, content_length) {
                        Ok(tail_data) => {
                            parse_tail_for_moov(
                                &tail_buffer,
                                &tail_data,
                                tail_offset,
                                content_length,
                                seek_secs,
                            );
                        },
                        Err(e) => {
                            log::warn!("TV: Range request for moov failed: {e}");
                        },
                    }
                });
            }
        }

        // Backpressure: pause downloading when buffer is far ahead of decoder.
        if buffer.should_throttle() {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }

        if std::time::Instant::now() > deadline {
            buffer.finish();
            return Err("timeout downloading video".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                buffer.push(&buf[..n]);
                // Reset deadline on successful data receipt so long
                // videos (and intentional throttle pauses) don't hit the
                // timeout.
                deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            },
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if buffer.bytes_received() > 0 {
                    break;
                }
                buffer.finish();
                return Err(format!("read: {e}"));
            },
        }
    }

    let received = buffer.bytes_received();
    let total = buffer.total_size.load(Ordering::Relaxed);
    log::info!(
        "TV: streaming download complete: {:.1}MB received (content-length={:.1}MB)",
        received as f64 / (1024.0 * 1024.0),
        total as f64 / (1024.0 * 1024.0),
    );
    buffer.finish();
    Ok(())
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
            // Keep the file in cache (don't delete) — it can be reused on re-tune.
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
    use super::*;

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
        // Exactly at MAX_LOOKAHEAD — not over, so no throttle.
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
        // Nothing evicted — cursor is at 0, not past RETAIN_BEHIND.
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
