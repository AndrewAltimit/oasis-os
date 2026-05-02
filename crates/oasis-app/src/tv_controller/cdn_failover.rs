//! HTTP redirect + Range request handling for archive.org CDN.
//!
//! `archive.org` returns 302 redirects to a CDN node that may be `dn*`
//! (HTTPS-only) or `ia*` (HTTP-or-HTTPS).  Some CDN nodes also reject direct
//! Range requests without a fresh redirect (returning 401), so the main
//! streaming path always issues Range requests against the original
//! archive.org URL and lets the redirect chain resolve to a current CDN.
//!
//! This module owns the low-level HTTP/1.1 bring-up: TCP+TLS connect, request
//! write, header read, redirect chase up to a small bounded count.  Two entry
//! points:
//!
//! - [`fetch_range`] — one-shot fetch of a byte range, returning the body.
//!   Used by the deferred tail probe to look for a moov-at-end atom.
//! - [`open_range_connection`] — opens a streaming Range connection and
//!   returns the live `NetworkStream` plus any leftover body bytes that came
//!   along with the headers.  Used by `download::stream_download_range`.

/// Maximum HTTP header size before aborting (prevents unbounded allocation
/// from malicious or broken servers sending endless header data).
#[cfg(feature = "_video")]
pub(crate) const MAX_HEADER_SIZE: usize = 64 * 1024; // 64 KB

/// Maximum WouldBlock backoff (8ms). Caps the exponential growth.
#[cfg(feature = "_video")]
pub(crate) const MAX_WOULD_BLOCK_BACKOFF_MS: u64 = 8;

/// Slack added on top of the requested Range size before we declare the
/// server is ignoring our Range header and abort. Keeps `fetch_range` from
/// silently buffering an entire file into memory if the server returns 200.
#[cfg(feature = "_video")]
pub(crate) const FETCH_RANGE_SIZE_SLACK: u64 = 64 * 1024;

/// Parsed redirect target: scheme, bare host, port, and path.
///
/// `split_redirect_target` produces this so callers can pass the right port
/// and choose between TLS and plain TCP. Pre-refactor the parser dropped the
/// scheme and embedded `:port` in the host string, which broke any redirect
/// to `http://` or to a non-default port.
#[cfg(feature = "_video")]
pub(crate) struct RedirectTarget {
    pub is_https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

/// Check if an I/O error is a WouldBlock (non-blocking socket not ready).
#[cfg(feature = "_video")]
pub(crate) fn is_would_block(e: &(impl std::error::Error + 'static)) -> bool {
    // Try to extract io::ErrorKind directly; fall back to string matching
    // for wrapped error types (e.g. OasisError::Io).
    if let Some(io_err) = <dyn std::error::Error>::downcast_ref::<std::io::Error>(e) {
        return io_err.kind() == std::io::ErrorKind::WouldBlock;
    }
    // Walk the source chain for wrapped io::Error.
    let mut source = e.source();
    while let Some(src) = source {
        if let Some(io_err) = src.downcast_ref::<std::io::Error>() {
            return io_err.kind() == std::io::ErrorKind::WouldBlock;
        }
        source = src.source();
    }
    false
}

/// Fetch a byte range from a URL via HTTP Range request.
/// Returns the raw body bytes on success.
///
/// If `cancel` is provided, the download loop checks it periodically and
/// returns early with an `Err` when the flag is set.
#[cfg(feature = "_video")]
pub(crate) fn fetch_range(
    tls: &oasis_core::net::RustlsTlsProvider,
    url: &str,
    start: u64,
    end: u64,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u8>, String> {
    fetch_range_inner(tls, url, start, end, 3, cancel)
}

#[cfg(feature = "_video")]
fn fetch_range_inner(
    tls: &oasis_core::net::RustlsTlsProvider,
    url: &str,
    start: u64,
    end: u64,
    redirects_left: u8,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u8>, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let target = split_redirect_target(url).ok_or_else(|| format!("unsupported URL: {url}"))?;
    let RedirectTarget {
        is_https,
        host,
        port,
        path,
    } = target;

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let tcp = net
        .connect(&host, port)
        .map_err(|e| format!("connect: {e}"))?;
    // This is an HTTP/1.1 client; force ALPN to http/1.1 so the shared
    // config's h2 offer doesn't put us on a frame-encoded stream we can't
    // parse. For plain http:// targets we skip TLS entirely.
    let mut stream: Box<dyn oasis_core::backend::NetworkStream> = if is_https {
        tls.connect_tls_with_alpn(tcp, &host, &[b"http/1.1"])
            .map_err(|e| format!("TLS: {e}"))?
            .stream
    } else {
        tcp
    };

    // Host header includes the port for non-default ports; this is what
    // most servers (and certainly archive.org's CDN nodes) expect.
    let host_header = if (is_https && port == 443) || (!is_https && port == 80) {
        host.clone()
    } else {
        format!("{host}:{port}")
    };

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Range: bytes={start}-{}\r\nConnection: close\r\n\r\n",
        end.saturating_sub(1),
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

    // Read response. Cap total bytes at the requested range size plus
    // header room and a small slack — a server that ignores the Range
    // header and starts streaming the whole file would otherwise buffer
    // the entire body in memory before the 30s timeout fires.
    let max_response: u64 = end
        .saturating_sub(start)
        .saturating_add(MAX_HEADER_SIZE as u64)
        .saturating_add(FETCH_RANGE_SIZE_SLACK);
    let mut response = Vec::new();
    let mut buf = [0u8; 8192];
    // Fixed absolute deadline matches `open_range_connection_inner`. A
    // rolling deadline that reset on every successful read would let a
    // server trickle one byte every 29s and never trigger the timeout;
    // since `max_response` is already capped, 30s for the bounded body
    // is plenty.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(flag) = cancel
            && flag.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("cancelled".into());
        }
        if std::time::Instant::now() > deadline {
            return Err("timeout".into());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                if (response.len() as u64) > max_response {
                    return Err(format!(
                        "response exceeded {max_response} bytes \
                         (server may be ignoring Range header)"
                    ));
                }
            },
            Err(e) => {
                if is_would_block(&e) {
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
            return fetch_range_inner(tls, &loc, start, end, redirects_left - 1, cancel);
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

/// Open a connection (TLS for `https://`, plain TCP for `http://`), send an
/// HTTP Range request, and return the stream plus any leftover body bytes
/// from the header read.
///
/// Returns `(stream, leftover_body_bytes)` on success.
#[cfg(feature = "_video")]
pub(crate) fn open_range_connection(
    is_https: bool,
    host: &str,
    port: u16,
    path: &str,
    tls: &oasis_core::net::RustlsTlsProvider,
    range_start: u64,
    range_end: u64,
) -> Result<(Box<dyn oasis_core::backend::NetworkStream>, Vec<u8>), String> {
    open_range_connection_inner(is_https, host, port, path, tls, range_start, range_end, 5)
}

#[cfg(feature = "_video")]
#[allow(clippy::too_many_arguments)]
fn open_range_connection_inner(
    is_https: bool,
    host: &str,
    port: u16,
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
        .connect(host, port)
        .map_err(|e| format!("connect: {e}"))?;
    // Force HTTP/1.1 ALPN — see comment in fetch_range_inner. Skip TLS for
    // plain http:// targets.
    let mut stream: Box<dyn oasis_core::backend::NetworkStream> = if is_https {
        tls.connect_tls_with_alpn(tcp, host, &[b"http/1.1"])
            .map_err(|e| format!("TLS: {e}"))?
            .stream
    } else {
        tcp
    };

    let host_header_owned;
    let host_header: &str = if (is_https && port == 443) || (!is_https && port == 80) {
        host
    } else {
        host_header_owned = format!("{host}:{port}");
        &host_header_owned
    };

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Range: bytes={range_start}-{range_end}\r\nConnection: close\r\n\r\n",
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
                if header_buf.len() > MAX_HEADER_SIZE {
                    return Err("HTTP headers too large".into());
                }
                if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
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
            let redir =
                split_redirect_target(&loc).ok_or_else(|| format!("bad redirect: {loc}"))?;
            drop(stream);
            return open_range_connection_inner(
                redir.is_https,
                &redir.host,
                redir.port,
                &redir.path,
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

/// Parse a redirect URL into scheme + host + port + path.  Pure helper
/// extracted so the redirect chain in [`open_range_connection_inner`] is
/// testable.
///
/// Returns `None` if the URL has no `http`/`https` scheme prefix.
///
/// `host` is the bare hostname with any `:port` stripped off; `port`
/// defaults to 443 for `https://` and 80 for `http://` when not explicit.
/// IPv6 literals (`[::1]:8080`) aren't parsed — archive.org redirects are
/// always DNS hostnames, and that's the only redirect target this is used
/// for today.
#[cfg(feature = "_video")]
pub(crate) fn split_redirect_target(loc: &str) -> Option<RedirectTarget> {
    let (is_https, stripped) = if let Some(s) = loc.strip_prefix("https://") {
        (true, s)
    } else if let Some(s) = loc.strip_prefix("http://") {
        (false, s)
    } else {
        return None;
    };
    let (host_with_port, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h.to_string(), format!("/{p}")))
        .unwrap_or((stripped.to_string(), "/".to_string()));

    let default_port: u16 = if is_https { 443 } else { 80 };
    // Only treat the trailing `:NNNN` as a port if it actually parses as
    // u16; otherwise it's part of the host (e.g. an IPv6 literal that we
    // can't usefully split this way).
    let (host, port) = match host_with_port.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(port) => (h.to_string(), port),
            Err(_) => (host_with_port, default_port),
        },
        None => (host_with_port, default_port),
    };

    Some(RedirectTarget {
        is_https,
        host,
        port,
        path,
    })
}
