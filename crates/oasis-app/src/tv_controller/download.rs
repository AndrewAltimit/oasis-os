//! HTTP streaming download: linear stream + Range-resume on stall, deferred
//! tail probe for moov-at-end files, seek-restart when moov is found.
//!
//! The actual HTTP transport (TLS, headers, Range requests, redirect chase)
//! lives in [`super::cdn_failover`]. moov parsing and seek estimation live
//! in [`super::seek`]. This module is the orchestrator that drives both.

#[cfg(feature = "_video")]
use super::cdn_failover::{
    MAX_WOULD_BLOCK_BACKOFF_MS, fetch_range, is_would_block, open_range_connection,
    split_redirect_target,
};
#[cfg(feature = "_video")]
use super::seek::{check_moov_at_start_restart, parse_tail_for_moov};
#[cfg(feature = "_video")]
use super::streaming_buffer::StreamingInner;

/// Time without receiving any body bytes before we consider the connection
/// stalled and attempt a reconnect (inspired by ffmpeg's `reconnect_on_http`).
#[cfg(feature = "_video")]
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Maximum number of reconnect attempts on a stalled Range download.
#[cfg(feature = "_video")]
const MAX_RECONNECTS: u32 = 5;

/// Download from a specific byte offset using HTTP Range request.
/// Pushes data into the buffer starting at `start_offset`.
///
/// Implements stall detection with automatic reconnect (inspired by ffmpeg's
/// `reconnect_on_http`): if no data arrives for `STALL_TIMEOUT` seconds,
/// the connection is dropped and a new Range request resumes from the last
/// byte received.  Up to `MAX_RECONNECTS` reconnect attempts are made.
#[cfg(feature = "_video")]
pub(crate) fn stream_download_range(
    url: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    buffer: &StreamingInner,
    start_offset: u64,
    total_size: u64,
) -> Result<(), String> {
    let target = split_redirect_target(url).ok_or_else(|| format!("unsupported URL: {url}"))?;

    let range_end = total_size.saturating_sub(1);
    let mut current_offset = start_offset;
    let mut reconnects = 0u32;

    'outer: loop {
        if buffer.is_cancelled() {
            log::info!("TV: range download cancelled before connect");
            return Ok(());
        }

        let (mut stream, leftover) = match open_range_connection(
            target.is_https,
            &target.host,
            target.port,
            &target.path,
            tls,
            current_offset,
            range_end,
        ) {
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
        let mut buf = [0u8; 65536];
        let mut last_data_time = std::time::Instant::now();
        let mut was_throttled = false;
        let mut first_data_logged = current_offset > start_offset;
        let mut wb_backoff_ms = 1u64;

        loop {
            if buffer.is_cancelled() {
                log::info!(
                    "TV: range download cancelled ({:.1}MB received)",
                    (current_offset - start_offset) as f64 / (1024.0 * 1024.0),
                );
                return Ok(());
            }
            if buffer.should_throttle() {
                // Don't reset stall timer during throttle -- if the decoder
                // is truly stuck (not just slow), we need to detect the stall
                // and reconnect rather than sleeping forever.
                if last_data_time.elapsed() > STALL_TIMEOUT * 3 {
                    log::warn!(
                        "TV: stalled while throttling ({:.0}s no decoder progress), \
                         forcing reconnect",
                        last_data_time.elapsed().as_secs_f64(),
                    );
                    if reconnects >= MAX_RECONNECTS {
                        buffer.set_error("stall during throttle, max reconnects exhausted".into());
                        return Ok(());
                    }
                    reconnects += 1;
                    drop(stream);
                    continue 'outer;
                }
                // Use condvar wait so the decoder can wake us immediately
                // when it catches up, instead of fixed 100ms sleep.
                let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                let _guard = buffer
                    .condvar
                    .wait_timeout(s, std::time::Duration::from_millis(50))
                    .unwrap_or_else(|e| e.into_inner());
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
                    wb_backoff_ms = 1; // reset backoff on data
                },
                Err(e) => {
                    if is_would_block(&e) {
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
                        std::thread::sleep(std::time::Duration::from_millis(wb_backoff_ms));
                        wb_backoff_ms = (wb_backoff_ms * 2).min(MAX_WOULD_BLOCK_BACKOFF_MS);
                        continue;
                    }
                    // Hard error -- try reconnect or finish.
                    if reconnects < MAX_RECONNECTS && current_offset < total_size {
                        reconnects += 1;
                        log::warn!(
                            "TV: read error ({e}), reconnect \
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
                    return Err(format!("read: {e}"));
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
pub(crate) fn stream_download(
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
    use super::cdn_failover::MAX_HEADER_SIZE;
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

    // Force HTTP/1.1 ALPN — see comment in fetch_range_inner.
    let mut stream = tls
        .connect_tls_with_alpn(tcp, host, &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))?
        .stream;

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
                if is_would_block(&e) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("write: {e}"));
            },
        }
    }

    // Read HTTP headers (and body — reuses the same buffer).
    let mut header_buf = Vec::with_capacity(4096);
    let mut buf = [0u8; 65536];
    let mut deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    let (header_end, leftover_start) = loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout reading headers".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err("connection closed before headers complete".to_string()),
            Ok(n) => {
                header_buf.extend_from_slice(&buf[..n]);
                if header_buf.len() > MAX_HEADER_SIZE {
                    return Err("HTTP headers too large".into());
                }
                if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (pos, pos + 4);
                }
            },
            Err(e) => {
                if is_would_block(&e) {
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
            // Case-insensitive prefix match without allocation.
            if line.len() > 15
                && line.as_bytes()[14] == b':'
                && line[..14].eq_ignore_ascii_case("content-length")
            {
                line[15..].trim().parse().ok()
            } else {
                None
            }
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
    let mut wb_backoff_ms = 1u64;
    loop {
        if buffer.is_cancelled() {
            log::info!("TV: download cancelled");
            return Ok(());
        }

        // Check for moov-based seek restart BEFORE throttle check so that
        // when the tail probe finds moov (and clears the buffer), we
        // immediately issue the Range restart instead of sleeping.
        if !checked_seek_restart && seek_secs > 0 && content_length > 10 * 1024 * 1024 {
            // CRITICAL: The restart decision and buffer mutation must happen
            // inside a single lock acquisition to prevent a race with the
            // tail probe thread (which also mutates base_offset/buf under
            // the same lock). Without this, a stale base_offset read could
            // cause a Range request to the wrong byte offset.
            let restart_from = {
                let mut s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.moov.is_some() {
                    checked_seek_restart = true;
                    // If base_offset was moved far ahead by the tail probe
                    // (moov-at-end case), restart from there.
                    let start_from = if s.base_offset > 4 * 1024 * 1024 && s.buf.len() < 1024 * 1024
                    {
                        Some(s.base_offset)
                    } else {
                        // moov found at start -- check if seek position is far ahead.
                        check_moov_at_start_restart(&s, seek_secs, buffer.bytes_received())
                    };
                    if let Some(start_from) = start_from {
                        // Retain header for symphonia probe after restart.
                        // Only do this for moov-at-start (base_offset == 0)
                        // to avoid overwriting correct moov from tail probe.
                        if s.base_offset == 0 && !s.buf.is_empty() {
                            let current_len = s.header.as_ref().map_or(0, |h| h.len());
                            if s.buf.len() > current_len {
                                s.header = Some(s.buf.clone());
                            }
                        }
                        s.base_offset = start_from;
                        buffer
                            .bytes_received
                            .store(start_from, std::sync::atomic::Ordering::Release);
                        s.buf.clear();
                    }
                    start_from
                } else {
                    None
                }
            };
            if let Some(start_from) = restart_from {
                // Reset decoder_pos so throttle doesn't think we're
                // far ahead of the decoder (decoder_pos was left at the
                // probe position ~1MB while bytes_received jumps to
                // the seek restart offset).
                buffer
                    .decoder_pos
                    .store(start_from, std::sync::atomic::Ordering::Release);
                log::info!(
                    "TV: restarting download from byte {:.1}MB via Range",
                    start_from as f64 / (1024.0 * 1024.0),
                );
                // Use original archive.org URL for Range requests --
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
                // moov already found in linear stream -- no tail probe needed.
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
                    // Use the buffer's cancelled flag so this download
                    // aborts promptly when the session is cancelled.
                    let cancel_flag = &tail_buffer.cancelled_flag();
                    match fetch_range(
                        &tail_tls,
                        &tail_url,
                        tail_offset,
                        content_length,
                        Some(cancel_flag),
                    ) {
                        Ok(tail_data) => {
                            // Check cancellation before processing results
                            // to avoid mutating buffer state after cancel.
                            if tail_buffer.is_cancelled() {
                                log::info!("TV: tail probe cancelled, discarding result");
                                return;
                            }
                            parse_tail_for_moov(
                                &tail_buffer,
                                &tail_data,
                                tail_offset,
                                content_length,
                                seek_secs,
                            );
                        },
                        Err(e) => {
                            if tail_buffer.is_cancelled() {
                                log::info!("TV: tail probe cancelled");
                            } else {
                                log::warn!("TV: Range request for moov failed: {e}");
                            }
                        },
                    }
                });
            }
        }

        // Backpressure: pause downloading when buffer is far ahead of decoder.
        // Use condvar-based wait so the decoder can wake us immediately
        // instead of sleeping a fixed 100ms.
        if buffer.should_throttle() {
            let s = buffer.state.lock().unwrap_or_else(|e| e.into_inner());
            let _guard = buffer
                .condvar
                .wait_timeout(s, std::time::Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
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
                wb_backoff_ms = 1; // reset backoff on successful data
                // Reset deadline on successful data receipt so long
                // videos (and intentional throttle pauses) don't hit the
                // timeout.
                deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            },
            Err(e) => {
                if is_would_block(&e) {
                    std::thread::sleep(std::time::Duration::from_millis(wb_backoff_ms));
                    wb_backoff_ms = (wb_backoff_ms * 2).min(MAX_WOULD_BLOCK_BACKOFF_MS);
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
