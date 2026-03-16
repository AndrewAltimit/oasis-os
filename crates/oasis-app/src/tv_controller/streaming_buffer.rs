//! Sliding-window streaming buffer: `Read + Seek` over an in-memory buffer
//! fed by a background download thread.

/// How much data to retain behind the decoder's read cursor.
/// Allows minor backward seeks without re-downloading.  Sized to cover
/// symphonia's internal seek-back distance when the H.264 decoder skips
/// many packets in tolerant-decode mode (openh264 Baseline-only gaps).
#[cfg(feature = "_video")]
pub(crate) const RETAIN_BEHIND: usize = 8 * 1024 * 1024; // 8 MB

/// Maximum bytes the download thread may be ahead of the decoder's read
/// cursor before it pauses.  Keeps memory bounded to ~16 MB lookahead.
#[cfg(feature = "_video")]
pub(crate) const MAX_LOOKAHEAD: u64 = 16 * 1024 * 1024; // 16 MB

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

/// Maximum valid atom size (10 GB). Atoms larger than this in a corrupt
/// file are treated as invalid to avoid integer overflow in offset math.
#[cfg(feature = "_video")]
const MAX_ATOM_SIZE: u64 = 10_000_000_000;

/// Sliding-window buffer state, protected by a mutex.
#[cfg(feature = "_video")]
pub(crate) struct SlidingState {
    /// Contiguous buffer of the currently-retained byte range.
    /// `buf[0]` corresponds to file offset `base_offset`.
    pub(crate) buf: Vec<u8>,
    /// File offset of `buf[0]`. Increases as old data is evicted.
    pub(crate) base_offset: u64,
    /// Retained moov atom -- copied out so it survives eviction.
    /// `(file_offset, data)`.  Wrapped in `Arc` to avoid redundant
    /// multi-MB clones when multiple callers read the moov data.
    pub(crate) moov: Option<(u64, std::sync::Arc<Vec<u8>>)>,
    /// Retained file header (ftyp atom, typically 24-32 bytes).
    /// Kept so symphonia can probe the container format after seek restart.
    pub(crate) header: Option<Vec<u8>>,
    /// Parsed top-level atom boundaries: `(offset, size, fourcc)`.
    /// Used to detect moov/mdat locations.
    pub(crate) atoms: Vec<(u64, u64, [u8; 4])>,
    /// How far we have scanned for atom headers.
    pub(crate) atoms_scanned_to: u64,
}

/// Shared inner state for the streaming buffer, fed by the download thread.
#[cfg(feature = "_video")]
pub(crate) struct StreamingInner {
    pub(crate) state: std::sync::Mutex<SlidingState>,
    /// Total content length from HTTP Content-Length header.
    pub(crate) total_size: std::sync::atomic::AtomicU64,
    /// Total bytes received from the network so far (monotonically increasing,
    /// never decremented by eviction).  Atomic to avoid locking the state
    /// mutex for progress/throttle checks (hot path).
    pub(crate) bytes_received: std::sync::atomic::AtomicU64,
    /// Whether the download is complete (all data received).
    done: std::sync::atomic::AtomicBool,
    /// Whether this session has been cancelled (new channel tuned).
    cancelled: std::sync::atomic::AtomicBool,
    /// Download error message, if any.
    pub(crate) error: std::sync::Mutex<Option<String>>,
    /// Signalled when new data arrives or download completes.
    pub(crate) condvar: std::sync::Condvar,
    /// Decoder's current read position (updated by `StreamingBuffer::read`).
    /// The download thread uses this to throttle when too far ahead.
    pub(crate) decoder_pos: std::sync::atomic::AtomicU64,
    /// When true, reads beyond retained data return zeros instantly
    /// instead of blocking.  Used during symphonia's probe phase so
    /// ignore_bytes() can skip the mdat body without downloading it.
    pub(crate) probe_mode: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "_video")]
impl StreamingInner {
    pub(crate) fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(SlidingState {
                buf: Vec::with_capacity(8 * 1024 * 1024),
                base_offset: 0,
                moov: None,
                header: None,
                atoms: Vec::new(),
                atoms_scanned_to: 0,
            }),
            total_size: std::sync::atomic::AtomicU64::new(0),
            bytes_received: std::sync::atomic::AtomicU64::new(0),
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
    pub(crate) fn push(&self, chunk: &[u8]) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.buf.extend_from_slice(chunk);

        // Update the atomic counter (lock-free reads by throttle/progress).
        let received = self
            .bytes_received
            .fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Release)
            + chunk.len() as u64;

        // Scan for top-level MP4 atom headers in newly arrived data.
        Self::scan_atoms(&mut s);

        // Log progress periodically (every 4MB).
        let buf_len = s.buf.len();
        if received.is_multiple_of(4 * 1024 * 1024) {
            let total = self.total_size.load(std::sync::atomic::Ordering::Relaxed);
            let pct = match std::num::NonZero::new(total) {
                Some(t) => format!(" ({}%)", received * 100 / t.get()),
                None => String::new(),
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
    pub(crate) fn scan_atoms(s: &mut SlidingState) {
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
                // Extended size -- need 16 bytes total header.
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
                // Atom extends to EOF -- we don't know total yet.
                // Use bytes_received as a conservative estimate; we'll
                // re-scan when more data arrives.
                break;
            } else {
                size32 as u64
            };

            if !(8..=MAX_ATOM_SIZE).contains(&atom_size) {
                break; // Invalid or implausibly large atom, stop scanning.
            }

            log::debug!(
                "TV: MP4 atom '{}' at offset {scan_pos}, size {atom_size}",
                String::from_utf8_lossy(&fourcc),
            );

            // For moov: don't advance past it until the full atom is
            // available and retained.  Otherwise we'd skip over moov and
            // never revisit it on subsequent push() calls.
            if &fourcc == b"moov" && s.moov.is_none() && scan_pos + atom_size > total {
                // moov found but incomplete -- wait for more data.
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
                s.moov = Some((scan_pos, std::sync::Arc::new(moov_data)));
            }

            s.atoms_scanned_to = scan_pos + atom_size;
        }

        // Update retained file header -- includes all scanned atom
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
    pub(crate) fn finish(&self) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // Final scan -- handle size==0 atoms (extends to EOF).
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
                            s.moov = Some((scan_pos, std::sync::Arc::new(moov_data)));
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
    pub(crate) fn set_error(&self, msg: String) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Total bytes received (for progress/logging).
    /// Lock-free read from the atomic counter.
    pub(crate) fn bytes_received(&self) -> u64 {
        self.bytes_received
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Whether download is complete.
    pub(crate) fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Cancel the download and unblock any waiting readers.
    pub(crate) fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.condvar.notify_all();
    }

    /// Whether this session has been cancelled.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Returns a reference to the cancellation flag for passing to
    /// functions that need to poll cancellation without a full
    /// `StreamingInner` reference.
    pub(crate) fn cancelled_flag(&self) -> &std::sync::atomic::AtomicBool {
        &self.cancelled
    }

    /// Wait until moov data is available (or download finishes/cancels).
    /// Returns an `Arc` reference to the moov data (cheap clone, no copy).
    pub(crate) fn wait_for_moov(
        &self,
        timeout: std::time::Duration,
    ) -> Option<std::sync::Arc<Vec<u8>>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some((_, ref data)) = s.moov {
                return Some(std::sync::Arc::clone(data));
            }
            if self.is_cancelled() {
                return None;
            }
            // After download finishes, `finish()` does a final atom scan
            // that may store moov under the lock.  Re-check moov one last
            // time (we already hold the lock via `s`) before giving up.
            if self.is_done() {
                // `s` is the current MutexGuard -- moov was just checked
                // above and was None, so the download truly has no moov.
                // Drop and re-acquire to pick up any store that happened
                // between `finish()`'s unlock and `done=true`.
                drop(s);
                let s2 = self.state.lock().unwrap_or_else(|e| e.into_inner());
                return s2
                    .moov
                    .as_ref()
                    .map(|(_, data)| std::sync::Arc::clone(data));
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
    ///
    /// Uses `Release` ordering so that the store is visible to the reader
    /// thread (which loads with `Acquire`) before any subsequent reads.
    pub(crate) fn disable_probe_mode(&self) {
        log::info!("TV: StreamingBuffer probe_mode disabled -- reads will now block");
        self.probe_mode
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Returns `true` if the download is far enough ahead of the decoder
    /// that it should pause to avoid unbounded memory growth.
    ///
    /// Uses only atomic loads for the fast path (decoder active). Falls back
    /// to locking the state mutex only when `decoder_pos == 0` and we need
    /// to check `has_moov` and `buf_size`.
    pub(crate) fn should_throttle(&self) -> bool {
        let decoder = self.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        let received = self
            .bytes_received
            .load(std::sync::atomic::Ordering::Acquire);

        if decoder > 0 {
            // Fast path: entirely lock-free.
            return received > decoder + MAX_LOOKAHEAD;
        }

        // Slow path: need moov/buf_size from state (only during init).
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let buf_size = s.buf.len() as u64;
        let has_moov = s.moov.is_some();
        drop(s);
        has_moov && buf_size > MAX_LOOKAHEAD
    }
}

/// Pure logic for throttle decision, extracted for testability.
///
/// - `decoder_pos > 0`: throttle if `received > decoder_pos + MAX_LOOKAHEAD`
/// - `decoder_pos == 0`: throttle if moov found AND `buf_size > MAX_LOOKAHEAD`
#[cfg(all(test, feature = "_video"))]
pub(crate) fn should_throttle_pure(
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
pub(crate) fn linear_seek_interpolation(
    seek_secs: f64,
    duration: f64,
    mdat_offset: u64,
    mdat_size: u64,
) -> u64 {
    if duration <= 0.0 {
        return mdat_offset;
    }
    let frac = (seek_secs / duration).clamp(0.0, 1.0);
    mdat_offset.saturating_add((frac * mdat_size as f64) as u64)
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
pub(crate) struct StreamingBuffer {
    inner: std::sync::Arc<StreamingInner>,
    pub(crate) pos: u64,
    /// Whether sliding-window eviction is active. Starts `false` to allow
    /// the demuxer to `read_to_end` + `seek(Start(0))` without data loss.
    pub(crate) eviction_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Last time a "waiting for data" message was logged (rate-limits spam).
    last_wait_log: Option<std::time::Instant>,
    /// Cached header (ftyp) data — once set it never changes, so reads from
    /// the header region can be served without acquiring the state mutex.
    cached_header: Option<Vec<u8>>,
    /// Cached moov atom — once set it never changes. Reads from the moov
    /// region can be served without acquiring the state mutex.
    cached_moov: Option<(u64, std::sync::Arc<Vec<u8>>)>,
}

#[cfg(feature = "_video")]
impl StreamingBuffer {
    pub(crate) fn new(inner: std::sync::Arc<StreamingInner>) -> Self {
        Self {
            inner,
            pos: 0,
            eviction_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_wait_log: None,
            cached_header: None,
            cached_moov: None,
        }
    }

    /// Evict data that is far behind the read cursor, except moov.
    ///
    /// Eviction is ONLY active after `on_init` enables it (post-demuxer probe).
    /// During init the demuxer does `read_to_end()` + `seek(0)`, so ALL data
    /// must remain in the buffer -- evicting early breaks format probing.
    ///
    /// Uses a 2x RETAIN_BEHIND threshold to batch evictions: the cursor must
    /// be at least 2 * RETAIN_BEHIND into the buffer before eviction triggers.
    /// This reduces lock acquisition frequency (evictions are ~50% less
    /// frequent) and each eviction shifts a larger block, amortizing the
    /// `copy_within` cost.
    pub(crate) fn maybe_evict(&self) {
        if !self
            .eviction_enabled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }

        let mut s = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let cursor_in_buf = self.pos.saturating_sub(s.base_offset) as usize;
        // Only evict when cursor is 2x RETAIN_BEHIND into the buffer to
        // batch evictions and reduce lock/memcpy frequency.
        if cursor_in_buf > RETAIN_BEHIND * 2 {
            let evict = cursor_in_buf - RETAIN_BEHIND;
            let evict = evict.min(s.buf.len());
            if evict > 0 {
                // In-place shift: copy_within avoids the Drain iterator
                // overhead and keeps the Vec's allocation stable.
                let new_len = s.buf.len() - evict;
                s.buf.copy_within(evict.., 0);
                s.buf.truncate(new_len);
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

            // Try serving from cached header/moov without acquiring the lock.
            // These are set once and never modified, so lock-free access is safe.
            if let Some(ref hdr) = self.cached_header
                && (self.pos as usize) < hdr.len()
            {
                let local = self.pos as usize;
                let n = buf.len().min(hdr.len() - local);
                buf[..n].copy_from_slice(&hdr[local..local + n]);
                self.pos += n as u64;
                break n;
            }
            if let Some((moov_off, ref moov_data)) = self.cached_moov {
                let moov_end = moov_off + moov_data.len() as u64;
                if self.pos >= moov_off && self.pos < moov_end {
                    let local = (self.pos - moov_off) as usize;
                    let n = buf.len().min(moov_data.len() - local);
                    buf[..n].copy_from_slice(&moov_data[local..local + n]);
                    self.pos += n as u64;
                    break n;
                }
            }

            let s = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());

            // Populate caches from state if not yet set (one-time copy).
            if self.cached_header.is_none()
                && let Some(ref hdr) = s.header
            {
                self.cached_header = Some(hdr.clone());
            }
            if self.cached_moov.is_none()
                && let Some((off, ref arc)) = s.moov
            {
                self.cached_moov = Some((off, std::sync::Arc::clone(arc)));
            }

            // Retry header/moov from newly-populated caches before hitting
            // the sliding buffer.  This handles the first read after moov
            // becomes available.
            if let Some(ref hdr) = self.cached_header
                && (self.pos as usize) < hdr.len()
            {
                let local = self.pos as usize;
                let n = buf.len().min(hdr.len() - local);
                buf[..n].copy_from_slice(&hdr[local..local + n]);
                self.pos += n as u64;
                break n;
            }
            if let Some((moov_off, ref moov_data)) = self.cached_moov {
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

            // Position is at or beyond file end -- EOF.
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
                .load(std::sync::atomic::Ordering::Acquire);
            if in_probe {
                // Fill with zeros up to base_offset, total_size, or
                // buf.len() -- whichever comes first.
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

            // Position is in the gap (evicted data) -- the sliding window
            // has moved past this region.  Return an error so the demuxer
            // can handle it cleanly instead of silently processing zeros.
            if self.pos < s.base_offset {
                log::warn!(
                    "TV: StreamingBuffer read from evicted region at {:.1}MB \
                     (base_offset={:.1}MB, gap={:.0}KB)",
                    self.pos as f64 / (1024.0 * 1024.0),
                    s.base_offset as f64 / (1024.0 * 1024.0),
                    (s.base_offset - self.pos) as f64 / 1024.0,
                );
                return Err(std::io::Error::other("read from evicted buffer region"));
            }

            // Position is beyond available data.
            if self.inner.is_done() {
                break 0; // EOF
            }

            // Rate-limit "waiting for data" logs to every 2 seconds.
            // Purely time-based to avoid spam from rapid seek+read cycles.
            if self
                .last_wait_log
                .is_none_or(|t| t.elapsed().as_millis() >= 2000)
            {
                log::info!(
                    "TV: StreamingBuffer waiting for data at {:.1}MB \
                     (buffer: {:.1}MB..{:.1}MB, {:.1}MB available)",
                    self.pos as f64 / (1024.0 * 1024.0),
                    s.base_offset as f64 / (1024.0 * 1024.0),
                    buf_end as f64 / (1024.0 * 1024.0),
                    s.buf.len() as f64 / (1024.0 * 1024.0),
                );
                self.last_wait_log = Some(std::time::Instant::now());
            }

            // Block until more data arrives.
            let _guard = self
                .inner
                .condvar
                .wait_timeout(s, std::time::Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
        };

        // Update decoder position and evict old data after successful read.
        // Skip during probe_mode -- probe reads return zeros and don't
        // represent real decoder progress.  Updating decoder_pos during
        // probe would race with the download thread's seek-restart logic
        // that resets decoder_pos to the Range start offset.
        if n > 0
            && !self
                .inner
                .probe_mode
                .load(std::sync::atomic::Ordering::Acquire)
        {
            let old_pos = self
                .inner
                .decoder_pos
                .swap(self.pos, std::sync::atomic::Ordering::Release);
            self.maybe_evict();
            // Notify the download thread if decoder advanced significantly
            // (>256KB) so throttle-sleeping threads wake up promptly.
            if self.pos.saturating_sub(old_pos) > 256 * 1024 {
                self.inner.condvar.notify_all();
            }
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
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                    loop {
                        let t = self.inner.total_size.load(Ordering::Acquire);
                        if t > 0 || self.inner.is_done() {
                            break;
                        }
                        if std::time::Instant::now() >= deadline {
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

        // Update decoder_pos on seek (when not in probe mode) so the
        // throttle logic knows where the decoder needs data.  Without
        // this, decoder_pos stays at 0 after a seek, causing the
        // throttle to cap the buffer at MAX_LOOKAHEAD while the decoder
        // is blocked waiting for data far beyond the buffer end.
        if !self
            .inner
            .probe_mode
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.inner
                .decoder_pos
                .store(self.pos, std::sync::atomic::Ordering::Release);
            // Wake the download thread so it can re-check throttle
            // with the updated decoder_pos.
            self.inner.condvar.notify_all();
        }

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

// StreamingBuffer is automatically Send + Sync because all fields are:
// - Arc<StreamingInner> (Send + Sync, all inner fields are Mutex/Atomic)
// - u64, bool (trivially Send + Sync)
// - Arc<AtomicBool> (Send + Sync)
// No manual unsafe impl needed.

// ---------------------------------------------------------------------------
// Item 72: StreamingBuffer throttle logic tests
// Item 73: Seek interpolation boundary tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "_video"))]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::sync::Arc;

    // ---------------------------------------------------------------
    // Item 72: should_throttle_pure tests (15 tests)
    // ---------------------------------------------------------------

    #[test]
    fn throttle_decoder_active_below_lookahead() {
        // decoder_pos > 0 and received is within MAX_LOOKAHEAD.
        assert!(!should_throttle_pure(
            1_000_000,
            1_000_000 + MAX_LOOKAHEAD,
            true,
            MAX_LOOKAHEAD,
        ));
    }

    #[test]
    fn throttle_decoder_active_above_lookahead() {
        // decoder_pos > 0 and received exceeds MAX_LOOKAHEAD ahead.
        assert!(should_throttle_pure(
            1_000_000,
            1_000_000 + MAX_LOOKAHEAD + 1,
            true,
            MAX_LOOKAHEAD + 1,
        ));
    }

    #[test]
    fn throttle_decoder_active_exactly_at_boundary() {
        // Exactly at the boundary -- should NOT throttle.
        assert!(!should_throttle_pure(
            1_000_000,
            1_000_000 + MAX_LOOKAHEAD,
            true,
            MAX_LOOKAHEAD,
        ));
    }

    #[test]
    fn throttle_decoder_zero_no_moov_no_throttle() {
        // decoder_pos == 0, no moov -> should not throttle.
        assert!(!should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 1_000_000,
            false,
            MAX_LOOKAHEAD + 1_000_000,
        ));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_below_lookahead() {
        // decoder_pos == 0, moov found, buffer < MAX_LOOKAHEAD.
        assert!(!should_throttle_pure(0, 1_000_000, true, 1_000_000));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_above_lookahead() {
        // decoder_pos == 0, moov found, buffer > MAX_LOOKAHEAD.
        assert!(should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 1,
            true,
            MAX_LOOKAHEAD + 1,
        ));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_exactly_at_lookahead() {
        // Exactly at MAX_LOOKAHEAD -- should NOT throttle.
        assert!(!should_throttle_pure(0, MAX_LOOKAHEAD, true, MAX_LOOKAHEAD));
    }

    #[test]
    fn throttle_decoder_large_position() {
        // Large decoder position, received just barely over threshold.
        let dec = 100 * 1024 * 1024; // 100 MB
        let received = dec + MAX_LOOKAHEAD + 1;
        assert!(should_throttle_pure(dec, received, true, 50_000_000));
    }

    #[test]
    fn throttle_decoder_large_position_within_range() {
        let dec = 100 * 1024 * 1024;
        let received = dec + MAX_LOOKAHEAD - 1;
        assert!(!should_throttle_pure(dec, received, true, 50_000_000));
    }

    #[test]
    fn throttle_decoder_zero_no_moov_large_buffer() {
        // Even with a huge buffer, no moov means no throttle at decoder_pos=0.
        assert!(!should_throttle_pure(0, 500_000_000, false, 500_000_000,));
    }

    #[test]
    fn throttle_decoder_one_byte_ahead_of_zero() {
        // decoder_pos = 1 (active), received just at threshold.
        assert!(!should_throttle_pure(
            1,
            1 + MAX_LOOKAHEAD,
            true,
            MAX_LOOKAHEAD
        ));
        assert!(should_throttle_pure(
            1,
            1 + MAX_LOOKAHEAD + 1,
            true,
            MAX_LOOKAHEAD + 1
        ));
    }

    #[test]
    fn throttle_all_zeros() {
        // Everything zero: decoder not started, no data. No throttle.
        assert!(!should_throttle_pure(0, 0, false, 0));
    }

    #[test]
    fn throttle_decoder_zero_moov_zero_buffer() {
        // Moov found but buffer is empty. Should not throttle.
        assert!(!should_throttle_pure(0, 0, true, 0));
    }

    #[test]
    fn throttle_decoder_active_received_less_than_decoder() {
        // Edge case: received < decoder (shouldn't happen but test robustness).
        assert!(!should_throttle_pure(
            10_000_000, 5_000_000, true, 5_000_000
        ));
    }

    #[test]
    fn throttle_max_u64_values_no_overflow() {
        // Near-max values should not overflow.
        assert!(should_throttle_pure(0, u64::MAX, true, u64::MAX));
    }

    // ---------------------------------------------------------------
    // Item 73: Seek interpolation boundary tests (8 tests)
    // ---------------------------------------------------------------

    #[test]
    fn seek_interpolation_zero_duration() {
        // Duration 0 or negative should return mdat_offset (no div by zero).
        assert_eq!(linear_seek_interpolation(5.0, 0.0, 1000, 500_000), 1000);
        assert_eq!(linear_seek_interpolation(5.0, -1.0, 1000, 500_000), 1000);
    }

    #[test]
    fn seek_interpolation_start() {
        // Seeking to 0s should return mdat_offset.
        assert_eq!(linear_seek_interpolation(0.0, 120.0, 1000, 500_000), 1000);
    }

    #[test]
    fn seek_interpolation_end() {
        // Seeking to exactly duration should return mdat_offset + mdat_size.
        assert_eq!(
            linear_seek_interpolation(120.0, 120.0, 1000, 500_000),
            1000 + 500_000
        );
    }

    #[test]
    fn seek_interpolation_midpoint() {
        // Seeking to 50% should return mdat_offset + mdat_size/2.
        assert_eq!(
            linear_seek_interpolation(60.0, 120.0, 1000, 500_000),
            1000 + 250_000
        );
    }

    #[test]
    fn seek_interpolation_beyond_duration_clamped() {
        // Seeking beyond duration should clamp to 1.0.
        assert_eq!(
            linear_seek_interpolation(200.0, 120.0, 1000, 500_000),
            1000 + 500_000
        );
    }

    #[test]
    fn seek_interpolation_negative_seek_clamped() {
        // Negative seek time should clamp to 0.
        assert_eq!(linear_seek_interpolation(-5.0, 120.0, 1000, 500_000), 1000);
    }

    #[test]
    fn seek_interpolation_large_file() {
        // 1 GB file, seek to 50%.
        let mdat_offset = 32_768u64;
        let mdat_size = 1_000_000_000u64;
        let result = linear_seek_interpolation(60.0, 120.0, mdat_offset, mdat_size);
        assert_eq!(result, mdat_offset + mdat_size / 2);
    }

    #[test]
    fn seek_interpolation_max_mdat_no_overflow() {
        // Near-max mdat_offset and size should use saturating_add.
        let result = linear_seek_interpolation(60.0, 120.0, u64::MAX - 100, 200);
        // saturating_add prevents overflow: (MAX-100) + 100 = MAX
        assert_eq!(result, u64::MAX);
    }

    // ---------------------------------------------------------------
    // StreamingInner / StreamingBuffer integration tests
    // ---------------------------------------------------------------

    #[test]
    fn streaming_inner_push_and_bytes_received() {
        let inner = StreamingInner::new();
        assert_eq!(inner.bytes_received(), 0);
        inner.push(&[0xAA; 1024]);
        assert_eq!(inner.bytes_received(), 1024);
        inner.push(&[0xBB; 2048]);
        assert_eq!(inner.bytes_received(), 3072);
    }

    #[test]
    fn streaming_inner_finish_marks_done() {
        let inner = StreamingInner::new();
        assert!(!inner.is_done());
        inner.finish();
        assert!(inner.is_done());
    }

    #[test]
    fn streaming_inner_cancel() {
        let inner = StreamingInner::new();
        assert!(!inner.is_cancelled());
        inner.cancel();
        assert!(inner.is_cancelled());
        assert!(inner.is_done());
    }

    #[test]
    fn streaming_inner_set_error() {
        let inner = StreamingInner::new();
        inner.set_error("test error".to_string());
        assert!(inner.is_done());
        let err = inner.error.lock().unwrap();
        assert_eq!(err.as_deref(), Some("test error"));
    }

    #[test]
    fn streaming_buffer_seek_start() {
        let inner = Arc::new(StreamingInner::new());
        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        assert_eq!(buf.pos, 0);
        let pos = std::io::Seek::seek(&mut buf, std::io::SeekFrom::Start(100)).unwrap();
        assert_eq!(pos, 100);
        assert_eq!(buf.pos, 100);
    }

    #[test]
    fn streaming_buffer_seek_current() {
        let inner = Arc::new(StreamingInner::new());
        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        buf.pos = 50;
        let pos = std::io::Seek::seek(&mut buf, std::io::SeekFrom::Current(25)).unwrap();
        assert_eq!(pos, 75);
    }

    #[test]
    fn streaming_buffer_seek_negative_errors() {
        let inner = Arc::new(StreamingInner::new());
        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        let result = std::io::Seek::seek(&mut buf, std::io::SeekFrom::Current(-1));
        assert!(result.is_err());
    }

    #[test]
    fn streaming_buffer_read_from_buffer() {
        let inner = Arc::new(StreamingInner::new());
        inner.push(&[0x11, 0x22, 0x33, 0x44, 0x55]);
        inner.finish();
        inner.disable_probe_mode();

        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        let mut out = [0u8; 3];
        let n = std::io::Read::read(&mut buf, &mut out).unwrap();
        assert_eq!(n, 3);
        assert_eq!(out, [0x11, 0x22, 0x33]);
        assert_eq!(buf.pos, 3);
    }

    #[test]
    fn streaming_buffer_read_eof_when_done() {
        let inner = Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(5, std::sync::atomic::Ordering::SeqCst);
        inner.push(&[0x11, 0x22, 0x33, 0x44, 0x55]);
        inner.finish();
        inner.disable_probe_mode();

        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        buf.pos = 5; // at end
        let mut out = [0u8; 4];
        let n = std::io::Read::read(&mut buf, &mut out).unwrap();
        assert_eq!(n, 0); // EOF
    }

    #[test]
    fn streaming_buffer_read_cancelled() {
        let inner = Arc::new(StreamingInner::new());
        inner.cancel();

        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        let mut out = [0u8; 4];
        let result = std::io::Read::read(&mut buf, &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn streaming_buffer_probe_mode_returns_zeros() {
        let inner = Arc::new(StreamingInner::new());
        // probe_mode is true by default.
        // Set total_size so the probe read has a limit.
        inner
            .total_size
            .store(1000, std::sync::atomic::Ordering::SeqCst);

        let mut buf = StreamingBuffer::new(Arc::clone(&inner));
        buf.pos = 500; // beyond any buffered data
        let mut out = [0xFFu8; 10];
        let n = std::io::Read::read(&mut buf, &mut out).unwrap();
        assert_eq!(n, 10);
        assert!(
            out.iter().all(|&b| b == 0),
            "probe mode should return zeros"
        );
    }

    #[test]
    fn scan_atoms_detects_moov() {
        let mut data = Vec::new();
        // ftyp atom (24 bytes)
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 16]);
        // moov atom (20 bytes)
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 12]);

        let inner = StreamingInner::new();
        inner.push(&data);

        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "should detect moov atom");
        assert_eq!(s.atoms.len(), 2);
        assert_eq!(&s.atoms[0].2, b"ftyp");
        assert_eq!(&s.atoms[1].2, b"moov");
    }

    #[test]
    fn scan_atoms_skips_invalid_size() {
        // Atom with size = 3 (< 8) should stop scanning.
        let mut data = Vec::new();
        data.extend_from_slice(&3u32.to_be_bytes());
        data.extend_from_slice(b"bad!");
        data.extend_from_slice(&[0u8; 20]);

        let inner = StreamingInner::new();
        inner.push(&data);

        let s = inner.state.lock().unwrap();
        assert!(s.atoms.is_empty());
    }
}

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
