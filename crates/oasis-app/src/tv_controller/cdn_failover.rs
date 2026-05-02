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

    let (host, path) =
        split_redirect_target(url).ok_or_else(|| format!("unsupported URL: {url}"))?;

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let tcp = net
        .connect(&host, 443)
        .map_err(|e| format!("connect: {e}"))?;
    // This is an HTTP/1.1 client; force ALPN to http/1.1 so the shared
    // config's h2 offer doesn't put us on a frame-encoded stream we can't
    // parse.
    let mut stream = tls
        .connect_tls_with_alpn(tcp, &host, &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))?
        .stream;

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
                if is_would_block(&e) {
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
                deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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

/// Open a TLS connection, send an HTTP Range request, and return the stream
/// plus any leftover body bytes from the header read.
///
/// Returns `(stream, leftover_body_bytes)` on success.
#[cfg(feature = "_video")]
pub(crate) fn open_range_connection(
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
    // Force HTTP/1.1 ALPN — see comment in fetch_range_inner.
    let mut stream = tls
        .connect_tls_with_alpn(tcp, host, &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))?
        .stream;

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
            let (redir_host, redir_path) =
                split_redirect_target(&loc).ok_or_else(|| format!("bad redirect: {loc}"))?;
            drop(stream);
            return open_range_connection_inner(
                &redir_host,
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

/// Parse the `Location:` header out of an HTTP response and split a redirect
/// URL into `(host, path)`.  Pure helper extracted so the redirect chain in
/// [`open_range_connection_inner`] is testable.
///
/// Returns `None` if the URL has no `http`/`https` scheme prefix.
#[cfg(feature = "_video")]
pub(crate) fn split_redirect_target(loc: &str) -> Option<(String, String)> {
    let stripped = loc
        .strip_prefix("https://")
        .or_else(|| loc.strip_prefix("http://"))?;
    let (host, path) = stripped
        .split_once('/')
        .map(|(h, p)| (h.to_string(), format!("/{p}")))
        .unwrap_or((stripped.to_string(), "/".to_string()));
    Some((host, path))
}
