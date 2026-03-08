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
                            } else if let Some(catalog) = guide.catalogs.get(idx).and_then(|c| c.as_ref())
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

    std::thread::spawn(move || {
        log::info!("TV: streaming download thread started: {url_owned}");
        if let Err(e) = stream_download(&url_owned, &tls, &buffer, seek_secs)
            && !buffer.is_cancelled()
        {
            log::error!("TV: streaming download failed: {e}");
            buffer.set_error(e);
        }
    });

    // Enable sliding-window eviction after the decoder finishes its initial
    // full-file scan (read_to_end + seek back to start). Before that, all
    // data must remain in the buffer.
    let on_init: Box<dyn FnOnce() + Send> = Box::new(move || {
        log::info!("TV: decoder initialized, enabling sliding-window eviction");
        eviction_flag.store(true, std::sync::atomic::Ordering::Release);
    });

    // Start the decoder immediately — it will block-read from the streaming
    // buffer as data arrives from the HTTP download.
    state.video_player.start_software_source(
        Box::new(reader),
        seek_secs,
        width,
        height,
        Some(on_init),
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

/// Maximum buffer size before a warning is logged (during init phase).
/// The demuxer's `read_to_end()` + `seek(0)` pattern requires the full file
/// in memory during init; eviction MUST NOT remove data before seek-back.
/// Actual eviction only begins after the `on_init` callback enables it.
#[cfg(feature = "_video")]
const INIT_WARN_THRESHOLD: usize = 128 * 1024 * 1024; // 128 MB

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
    total_size: std::sync::atomic::AtomicU64,
    /// Whether the download is complete (all data received).
    done: std::sync::atomic::AtomicBool,
    /// Whether this session has been cancelled (new channel tuned).
    cancelled: std::sync::atomic::AtomicBool,
    /// Download error message, if any.
    error: std::sync::Mutex<Option<String>>,
    /// Signalled when new data arrives or download completes.
    condvar: std::sync::Condvar,
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
                atoms: Vec::new(),
                atoms_scanned_to: 0,
            }),
            total_size: std::sync::atomic::AtomicU64::new(0),
            done: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            error: std::sync::Mutex::new(None),
            condvar: std::sync::Condvar::new(),
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
        if buf_len > INIT_WARN_THRESHOLD
            && (buf_len - chunk.len()) <= INIT_WARN_THRESHOLD
        {
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

            s.atoms.push((scan_pos, atom_size, fourcc));

            // If this is moov and we have the full atom, retain it.
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

                    // Retain moov if found at end of file.
                    if &fourcc == b"moov" && s.moov.is_none() {
                        let end = (buf_off + atom_size as usize).min(s.buf.len());
                        let moov_data = s.buf[buf_off..end].to_vec();
                        log::info!(
                            "TV: retained moov atom ({} bytes) at offset {scan_pos} (end of file)",
                            moov_data.len(),
                        );
                        s.moov = Some((scan_pos, moov_data));
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
    fn bytes_received(&self) -> u64 {
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
        self.done
            .store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Whether this session has been cancelled.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// A reader cursor over a `StreamingInner` sliding-window buffer.
///
/// Implements `Read + Seek` with blocking semantics. Reads block until data
/// is available at the current position or the download completes/errors.
/// After each read, data far behind the cursor is evicted to bound memory
/// usage. The moov atom is retained separately and never evicted.
///
/// Eviction is disabled by default and must be enabled via
/// [`enable_eviction`](Self::enable_eviction) after the demuxer has finished
/// its initial full-file scan (avcC + probe). This prevents evicting data
/// that the demuxer needs to seek back to during initialization.
#[cfg(feature = "_video")]
struct StreamingBuffer {
    inner: std::sync::Arc<StreamingInner>,
    pos: u64,
    /// Whether sliding-window eviction is active. Starts `false` to allow
    /// the demuxer to `read_to_end` + `seek(Start(0))` without data loss.
    eviction_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "_video")]
impl StreamingBuffer {
    fn new(inner: std::sync::Arc<StreamingInner>) -> Self {
        Self {
            inner,
            pos: 0,
            eviction_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

            // Position is before the sliding window (evicted, non-moov data).
            if self.pos < s.base_offset {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "read at evicted offset {} (window starts at {})",
                        self.pos, s.base_offset
                    ),
                ));
            }

            // Position is beyond available data.
            if self.inner.is_done() {
                break 0; // EOF
            }

            // Block until more data arrives.
            let _guard = self
                .inner
                .condvar
                .wait_timeout(s, std::time::Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
        };

        // Evict old data after successful read.
        if n > 0 {
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
            std::io::SeekFrom::Current(off) => self.pos as i64 + off,
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
                    self.inner.bytes_received() as i64 + off
                } else {
                    total as i64 + off
                }
            },
        };

        if new_pos < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek to negative position",
            ));
        }

        self.pos = new_pos as u64;
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout".into());
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
    log::info!(
        "TV: Range response: HTTP {status}, {} bytes",
        body.len(),
    );
    Ok(body)
}

/// Parse the duration in seconds from a moov atom's mvhd box.
#[cfg(feature = "_video")]
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
                // v0: create(4) + mod(4) + timescale(4) + duration(4)
                let timescale = u32::from_be_bytes([data[4 + 4], data[4 + 5], data[4 + 6], data[4 + 7]]);
                let duration = u32::from_be_bytes([data[4 + 8], data[4 + 9], data[4 + 10], data[4 + 11]]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            } else if version == 1 && data.len() >= 32 {
                // v1: create(8) + mod(8) + timescale(4) + duration(8)
                let timescale = u32::from_be_bytes([data[4 + 16], data[4 + 17], data[4 + 18], data[4 + 19]]);
                let duration = u64::from_be_bytes([
                    data[4 + 20], data[4 + 21], data[4 + 22], data[4 + 23],
                    data[4 + 24], data[4 + 25], data[4 + 26], data[4 + 27],
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

/// Download from a specific byte offset using HTTP Range request.
/// Pushes data into the buffer starting at `start_offset`.
#[cfg(feature = "_video")]
fn stream_download_range(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &StreamingInner,
    start_offset: u64,
    total_size: u64,
) -> Result<(), String> {
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
         Range: bytes={start_offset}-{}\r\nConnection: close\r\n\r\n",
        total_size.saturating_sub(1),
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    let leftover_start = loop {
        if std::time::Instant::now() > deadline {
            buffer.finish();
            return Err("timeout reading headers".into());
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                buffer.finish();
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
                buffer.finish();
                return Err(format!("read headers: {e}"));
            },
        }
    };

    let leftover = &header_buf[leftover_start..];
    if !leftover.is_empty() {
        buffer.push(leftover);
    }

    // Stream body.
    loop {
        if buffer.is_cancelled() {
            log::info!("TV: range download cancelled");
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            buffer.finish();
            return Err("timeout downloading".into());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => buffer.push(&buf[..n]),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if buffer.bytes_received() > start_offset {
                    break;
                }
                buffer.finish();
                return Err(format!("read: {e}"));
            },
        }
    }

    let received = buffer.bytes_received();
    log::info!(
        "TV: range download complete: {:.1}MB received (from offset {:.1}MB)",
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
    buffer: &StreamingInner,
    seek_secs: u64,
) -> Result<(), String> {
    stream_download_inner(url, tls, buffer, 5, seek_secs)
}

#[cfg(feature = "_video")]
fn stream_download_inner(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &StreamingInner,
    redirects_left: u8,
    seek_secs: u64,
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

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
            return stream_download_inner(&loc, tls, buffer, redirects_left - 1, seek_secs);
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

    // Check if moov atom is at the end of a large file. If so, fetch it
    // concurrently via a Range request so the decoder can open immediately
    // instead of waiting for the entire file to download linearly.
    if content_length > 10 * 1024 * 1024 {
        let mdat_info = {
            let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.moov.is_some() {
                None // moov already found
            } else {
                // Find the large mdat atom — moov comes after it.
                s.atoms
                    .iter()
                    .find(|(_, size, cc)| cc == b"mdat" && *size > 10 * 1024 * 1024)
                    .map(|(off, size, _)| (*off, *size))
            }
        };
        if let Some((mdat_offset, mdat_size)) = mdat_info {
            let moov_file_offset = mdat_offset + mdat_size;
            let moov_approx_size = content_length.saturating_sub(moov_file_offset);
            if moov_approx_size > 0 && moov_approx_size < 50 * 1024 * 1024 {
                log::info!(
                    "TV: moov at offset {} (~{:.1}MB) in {:.0}MB file, fetching via Range",
                    moov_file_offset,
                    moov_approx_size as f64 / (1024.0 * 1024.0),
                    content_length as f64 / (1024.0 * 1024.0),
                );
                let final_url = format!("https://{host}{path}");
                match fetch_range(tls, &final_url, moov_file_offset, content_length) {
                    Ok(tail_data) => {
                        // Parse atoms starting from the exact moov position.
                        let mut offset = 0usize;
                        while offset + 8 <= tail_data.len() {
                            let size32 = u32::from_be_bytes([
                                tail_data[offset],
                                tail_data[offset + 1],
                                tail_data[offset + 2],
                                tail_data[offset + 3],
                            ]);
                            let fourcc: [u8; 4] = [
                                tail_data[offset + 4],
                                tail_data[offset + 5],
                                tail_data[offset + 6],
                                tail_data[offset + 7],
                            ];
                            let atom_size = if size32 == 1 && offset + 16 <= tail_data.len() {
                                u64::from_be_bytes([
                                    tail_data[offset + 8],
                                    tail_data[offset + 9],
                                    tail_data[offset + 10],
                                    tail_data[offset + 11],
                                    tail_data[offset + 12],
                                    tail_data[offset + 13],
                                    tail_data[offset + 14],
                                    tail_data[offset + 15],
                                ]) as usize
                            } else if size32 == 0 {
                                tail_data.len() - offset
                            } else {
                                size32 as usize
                            };
                            if atom_size < 8 || offset + atom_size > tail_data.len() {
                                break;
                            }
                            log::debug!(
                                "TV: tail atom '{}' at file offset {}, size {}",
                                String::from_utf8_lossy(&fourcc),
                                moov_file_offset + offset as u64,
                                atom_size,
                            );
                            if &fourcc == b"moov" {
                                let file_off = moov_file_offset + offset as u64;
                                let moov_data =
                                    tail_data[offset..offset + atom_size].to_vec();
                                log::info!(
                                    "TV: pre-fetched moov atom ({} bytes) at offset {}",
                                    moov_data.len(),
                                    file_off,
                                );

                                // Parse mvhd duration to estimate seek byte offset.
                                if seek_secs > 0
                                    && let Some(dur) = parse_moov_duration(&moov_data)
                                {
                                        let frac = (seek_secs as f64 / dur).clamp(0.0, 1.0);
                                        let seek_byte = mdat_offset
                                            + (frac * mdat_size as f64) as u64;
                                        // Start download from slightly before seek pos
                                        // to ensure keyframe is included.
                                        let start_from = seek_byte
                                            .saturating_sub(2 * 1024 * 1024)
                                            .max(mdat_offset);
                                        log::info!(
                                            "TV: duration={dur:.0}s, seek={seek_secs}s \
                                             -> byte ~{:.1}MB, downloading from {:.1}MB",
                                            seek_byte as f64 / (1024.0 * 1024.0),
                                            start_from as f64 / (1024.0 * 1024.0),
                                        );
                                        let mut s = buffer
                                            .state
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner());
                                        s.moov = Some((file_off, moov_data));
                                        // Set base_offset and bytes_received so the
                                        // buffer looks like it already has data up to
                                        // start_from (which it doesn't, but the decoder
                                        // only needs data from start_from onwards + moov).
                                        s.base_offset = start_from;
                                        s.bytes_received = start_from;
                                        drop(s);
                                        buffer.condvar.notify_all();

                                        // Restart download from seek position.
                                        let range_url =
                                            format!("https://{host}{path}");
                                        log::info!(
                                            "TV: restarting download from byte {}",
                                            start_from,
                                        );
                                        return stream_download_range(
                                            &range_url,
                                            tls,
                                            buffer,
                                            start_from,
                                            content_length,
                                        );
                                }

                                let mut s = buffer
                                    .state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                s.moov = Some((file_off, moov_data));
                                buffer.condvar.notify_all();
                                break;
                            }
                            offset += atom_size;
                        }
                    },
                    Err(e) => {
                        log::warn!("TV: Range request for moov failed: {e}");
                    },
                }
            }
        }
    }

    // For moov-at-start files with a seek target: once moov is found in the
    // linear stream, parse duration, calculate seek byte offset, and restart
    // the download from near the seek position via a Range request.
    let mut checked_moov_at_start = false;

    // Stream remaining body into the shared buffer.
    loop {
        if buffer.is_cancelled() {
            log::info!("TV: download cancelled");
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            buffer.finish();
            return Err("timeout downloading video".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                buffer.push(&buf[..n]);

                // Check once if moov was found at start of file.
                if !checked_moov_at_start && seek_secs > 0 && content_length > 10 * 1024 * 1024 {
                    let moov_info = {
                        let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                        s.moov
                            .as_ref()
                            .map(|(off, data)| (*off, data.clone()))
                    };
                    if let Some((_moov_off, moov_data)) = moov_info {
                        checked_moov_at_start = true;
                        if let Some(dur) = parse_moov_duration(&moov_data) {
                            // Find mdat for byte offset calculation.
                            let mdat_info = {
                                let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                                s.atoms
                                    .iter()
                                    .find(|(_, size, cc)| cc == b"mdat" && *size > 1024)
                                    .map(|(off, size, _)| (*off, *size))
                            };
                            if let Some((mdat_off, mdat_size)) = mdat_info {
                                let frac = (seek_secs as f64 / dur).clamp(0.0, 1.0);
                                let seek_byte = mdat_off + (frac * mdat_size as f64) as u64;
                                let start_from = seek_byte
                                    .saturating_sub(2 * 1024 * 1024)
                                    .max(mdat_off);
                                let downloaded = buffer.bytes_received();
                                // Only worth restarting if seek position is far ahead.
                                if start_from > downloaded + 4 * 1024 * 1024 {
                                    log::info!(
                                        "TV: moov-at-start: duration={dur:.0}s, seek={seek_secs}s \
                                         -> byte ~{:.1}MB (downloaded {:.1}MB), restarting from {:.1}MB",
                                        seek_byte as f64 / (1024.0 * 1024.0),
                                        downloaded as f64 / (1024.0 * 1024.0),
                                        start_from as f64 / (1024.0 * 1024.0),
                                    );
                                    let mut s = buffer
                                        .state
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    s.base_offset = start_from;
                                    s.bytes_received = start_from;
                                    s.buf.clear();
                                    drop(s);

                                    let range_url = format!("https://{host}{path}");
                                    return stream_download_range(
                                        &range_url,
                                        tls,
                                        buffer,
                                        start_from,
                                        content_length,
                                    );
                                }
                            }
                        }
                    }
                }
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
    if let Some(track) = state.tv_audio_track.take() {
        let _ = state.audio_backend.unload_track(track);
    }

    // Update the guide's clock so schedule_at returns the current episode.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    guide.current_time = now;

    // Re-tune to whatever should be playing now.
    let catalog = guide.catalogs.get(channel_idx).and_then(|c| c.as_ref());
    let Some(catalog) = catalog else { return };
    let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, now) else {
        return;
    };

    let url =
        oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(&slot.episode);
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
