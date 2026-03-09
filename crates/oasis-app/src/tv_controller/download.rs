//! HTTP streaming download logic: initial connection, Range requests,
//! CDN failover, tail probe for moov-at-end files.

#[cfg(feature = "_video")]
use super::streaming_buffer::{SlidingState, StreamingInner, linear_seek_interpolation};

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
pub(crate) const SHORT_SEEK_THRESHOLD: u64 = 4 * 1024 * 1024; // 4 MB

/// Fetch a byte range from a URL via HTTP Range request.
/// Returns the raw body bytes on success.
#[cfg(feature = "_video")]
pub(crate) fn fetch_range(
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
pub(crate) fn check_moov_at_start_restart(s: &SlidingState, seek_secs: u64) -> Option<u64> {
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
    // to find sync points -- its internal seek may land somewhat before
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
pub(crate) fn parse_tail_for_moov(
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
    let mut search_from = 4usize; // need >=4 bytes before fourcc for size
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
        // Validate: atom must be >=8 bytes and fit within the tail data.
        if atom_size >= 8 && atom_start + atom_size <= tail_data.len() {
            break Some((atom_start, atom_size));
        }
        // False positive -- keep scanning past this occurrence.
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
                // Cannot estimate -- retain moov and let decoder seek.
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

pub(crate) fn parse_moov_duration(moov_data: &[u8]) -> Option<f64> {
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
                // v0 layout after version(1)+flags(3): create(4) + mod(4) + timescale(4) +
                // duration(4) timescale starts at byte 12, duration at byte 16
                let timescale = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
                let duration = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                if timescale > 0 {
                    return Some(duration as f64 / timescale as f64);
                }
            } else if version == 1 && data.len() >= 32 {
                // v1 layout after version(1)+flags(3): create(8) + mod(8) + timescale(4) +
                // duration(8) timescale starts at byte 20, duration at byte 24
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

    // Follow redirects (archive.org 302 -> CDN node).
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
            // Partial Content -- server honoured the Range request.
        },
        200 => {
            // Server ignored Range header and is sending the full file
            // from byte 0.  Pushing this data at `range_start` would
            // corrupt the stream with misaligned data.
            return Err("HTTP 200 (server ignored Range header) -- cannot resume".into());
        },
        416 => {
            return Err("HTTP 416 Range Not Satisfiable".into());
        },
        _ if (200..300).contains(&status) => {
            // Other 2xx -- unexpected but not fatal.
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
pub(crate) fn stream_download_range(
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
                    // Hard error -- try reconnect or finish.
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
                        // moov found at start -- check if seek position is far ahead.
                        check_moov_at_start_restart(&s, seek_secs)
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
                    // For moov-at-start: retain the current buffer as a
                    // combined header (ftyp + moov + mdat leader) so
                    // symphonia can probe after the Range restart.
                    // Only do this if moov hasn't already been retained
                    // separately by the tail probe (moov-at-end case) --
                    // otherwise we'd overwrite the correct moov data with
                    // a raw buffer that isn't a valid moov atom.
                    if s.base_offset == 0 && !s.buf.is_empty() {
                        let current_len = s.header.as_ref().map_or(0, |h| h.len());
                        if s.buf.len() > current_len {
                            s.header = Some(s.buf.clone());
                        }
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
