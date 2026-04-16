//! Minimal HTTP/1.1 GET client.
//!
//! Supports plain HTTP over `std::net::TcpStream` and, when a
//! [`TlsProvider`] is supplied, HTTPS via the backend's TLS stack.

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use brotli::Decompressor as BrotliDecoder;
use flate2::read::{DeflateDecoder, GzDecoder};
use oasis_net::tls::TlsProvider;
use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::{ContentType, ResourceResponse, Url};

/// Maximum response body size (8 MB).
const MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

/// Maximum HTTP header section size (16 KB).
///
/// Prevents a malicious server from exhausting memory by sending an
/// unbounded header block before the `\r\n\r\n` terminator.
const MAX_HEADER_SIZE: usize = 16_384;

/// Maximum number of redirects to follow.
const MAX_REDIRECTS: u8 = 5;

/// TCP connect timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// TCP read timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum idle connections per host in the connection pool.
const MAX_CONNS_PER_HOST: usize = 2;

/// Maximum total idle connections in the pool.
const MAX_TOTAL_CONNS: usize = 8;

/// Maximum age of an idle pooled connection (30 seconds).
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// DNS cache TTL (5 minutes).
#[allow(dead_code)]
const DNS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Global DNS cache: maps hostname -> (resolved IPs, cached time).
type DnsCacheMap = FxHashMap<String, (Vec<std::net::IpAddr>, Instant)>;

#[allow(dead_code)]
static DNS_CACHE: std::sync::LazyLock<Mutex<DnsCacheMap>> =
    std::sync::LazyLock::new(|| Mutex::new(FxHashMap::default()));

// -------------------------------------------------------------------
// Connection pool
// -------------------------------------------------------------------

/// An idle pooled connection with its insertion timestamp.
struct PooledConn {
    stream: TcpStream,
    inserted: Instant,
}

/// Simple HTTP/1.1 keep-alive connection pool.
///
/// Stores idle `TcpStream` connections keyed by `(host, port)`.
/// Not thread-safe -- designed for single-threaded browser use.
struct ConnectionPool {
    conns: FxHashMap<(String, u16), Vec<PooledConn>>,
    total: usize,
}

impl ConnectionPool {
    fn new() -> Self {
        Self {
            conns: FxHashMap::default(),
            total: 0,
        }
    }

    /// Take an idle connection for the given host:port, if available.
    ///
    /// Expired connections are discarded silently.
    fn take(&mut self, host: &str, port: u16) -> Option<TcpStream> {
        let key = (host.to_string(), port);
        let entries = self.conns.get_mut(&key)?;
        while let Some(entry) = entries.pop() {
            self.total -= 1;
            if entry.inserted.elapsed() < POOL_IDLE_TIMEOUT {
                if entries.is_empty() {
                    self.conns.remove(&key);
                }
                return Some(entry.stream);
            }
            // Expired -- drop and try next.
        }
        self.conns.remove(&key);
        None
    }

    /// Return a connection to the pool for future reuse.
    ///
    /// Silently drops the connection if the pool is full.
    fn put(&mut self, host: &str, port: u16, stream: TcpStream) {
        if self.total >= MAX_TOTAL_CONNS {
            return; // Pool full, discard.
        }
        let key = (host.to_string(), port);
        let entries = self.conns.entry(key).or_default();
        if entries.len() >= MAX_CONNS_PER_HOST {
            return; // Per-host limit reached, discard.
        }
        entries.push(PooledConn {
            stream,
            inserted: Instant::now(),
        });
        self.total += 1;
    }
}

thread_local! {
    static CONN_POOL: RefCell<ConnectionPool> = RefCell::new(ConnectionPool::new());
}

/// Resolve a hostname using the DNS cache.
///
/// Returns cached addresses if still within TTL, otherwise performs a
/// fresh DNS resolution and updates the cache.
#[allow(dead_code)]
pub(crate) fn dns_resolve_cached(host: &str) -> io::Result<Vec<std::net::IpAddr>> {
    use std::net::ToSocketAddrs;

    // Check cache first.
    if let Ok(cache) = DNS_CACHE.lock()
        && let Some((addrs, cached_at)) = cache.get(host)
        && cached_at.elapsed() < DNS_CACHE_TTL
    {
        return Ok(addrs.clone());
    }

    // Fresh resolution.
    let addrs: Vec<std::net::IpAddr> = (host, 0u16).to_socket_addrs()?.map(|sa| sa.ip()).collect();

    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("DNS resolution failed for {host}"),
        ));
    }

    // Update cache.
    if let Ok(mut cache) = DNS_CACHE.lock() {
        cache.insert(host.to_string(), (addrs.clone(), Instant::now()));
    }

    Ok(addrs)
}

/// Perform an HTTP(S) GET request for the given URL.
///
/// When `tls` is `Some`, HTTPS URLs are supported.  When `None`, HTTPS
/// URLs produce a user-friendly error page instead.
///
/// Follows redirects (301/302/307/308) up to `MAX_REDIRECTS` hops.
pub fn http_get(url: &Url, tls: Option<&dyn TlsProvider>) -> Result<ResourceResponse> {
    http_request("GET", url, None, &[], tls)
}

/// Perform an HTTP(S) request with an arbitrary method, optional body,
/// and optional extra headers.
///
/// When `tls` is `Some`, HTTPS URLs are supported.  When `None`, HTTPS
/// URLs produce a user-friendly error page instead.
///
/// For `POST` requests with a body, `Content-Type` and `Content-Length`
/// headers are added automatically when no explicit `Content-Type` is
/// provided in `extra_headers`.
///
/// Follows redirects (301/302/307/308) up to `MAX_REDIRECTS` hops.
/// On redirect, the method falls back to GET (as browsers do for 301/302/303).
pub fn http_request(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
) -> Result<ResourceResponse> {
    http_request_full(method, url, body, extra_headers, tls).map(|(resp, _headers)| resp)
}

/// Like [`http_request`] but also returns the raw response headers.
///
/// This is used internally by the loader to extract `Set-Cookie`,
/// `ETag`, and `Last-Modified` headers after the request completes.
pub fn http_request_full(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    if url.scheme == "https" && tls.is_none() {
        return Ok((https_error_page(url, url), Vec::new()));
    }
    if url.scheme != "http" && url.scheme != "https" {
        return Err(OasisError::Backend(
            format!("unsupported scheme for HTTP client: {}", url.scheme,).into(),
        ));
    }

    let mut current_url = url.clone();
    // After a redirect, the method reverts to GET (standard browser
    // behaviour for 301/302/303).
    let mut current_method = method.to_string();
    let mut current_body: Option<Vec<u8>> = body.map(|b| b.to_vec());

    for _ in 0..MAX_REDIRECTS {
        let resp = do_request_with_method(
            &current_method,
            &current_url,
            current_body.as_deref(),
            extra_headers,
            tls,
        )?;

        if is_redirect(resp.status_code)
            && let Some(location) = find_header(&resp.headers, "location")
        {
            let location = location.to_string();
            current_url = current_url.resolve(&location).ok_or_else(|| {
                OasisError::Backend(format!("bad redirect Location: {location}").into())
            })?;
            if current_url.scheme == "https" && tls.is_none() {
                return Ok((https_error_page(url, &current_url), Vec::new()));
            }
            // Redirects revert to GET and drop the body (per HTTP spec
            // for 301/302/303; 307/308 should preserve, but browsers
            // typically do not re-POST).
            current_method = "GET".to_string();
            current_body = None;
            continue;
        }

        let content_type = find_header(&resp.headers, "content-type")
            .map(ContentType::from_mime)
            .unwrap_or_else(|| super::detect_content_type(&current_url));

        let headers = resp.headers;
        return Ok((
            ResourceResponse {
                url: current_url.to_string(),
                content_type,
                body: resp.body,
                status: resp.status_code,
            },
            headers,
        ));
    }

    Err(OasisError::Backend("too many redirects".into()))
}

/// Case-insensitive header lookup (public for use by other loader modules).
pub fn response_find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    find_header(headers, name)
}

// -------------------------------------------------------------------
// Internal types
// -------------------------------------------------------------------

/// A raw parsed HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Response headers as (name, value) pairs.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

// -------------------------------------------------------------------
// Internals
// -------------------------------------------------------------------

/// Connect, optionally upgrade to TLS, send request, read and parse.
///
/// For plain HTTP, attempts to reuse a pooled keep-alive connection
/// before opening a new one.  On success with keep-alive, the
/// connection is returned to the pool.
fn do_request_with_method(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
) -> Result<HttpResponse> {
    let host = &url.host;
    let is_https = url.scheme == "https";
    let default_port = if is_https { 443 } else { 80 };
    let port = url.port.unwrap_or(default_port);

    if is_https {
        let tls_provider = tls.ok_or_else(|| OasisError::Backend("TLS not available".into()))?;

        let stream = tcp_connect(host, port)?;
        // Wrap the TcpStream as a NetworkStream, then upgrade to TLS
        // while offering ALPN. If the server picks `h2`, route the
        // request through the HTTP/2 driver; otherwise fall through
        // to the HTTP/1.1 path.
        let net_stream: Box<dyn NetworkStream> = Box::new(oasis_net::StdNetworkStream::new(stream));
        let tls_conn =
            tls_provider.connect_tls_with_alpn(net_stream, host, &[b"h2", b"http/1.1"])?;

        let mut adapter = NetworkStreamAdapter(tls_conn.stream);
        if tls_conn.alpn.as_deref() == Some(b"h2") {
            return super::http2::h2_request(&mut adapter, method, url, body, extra_headers);
        }
        send_request(&mut adapter, method, url, body, extra_headers, is_https)?;
        let raw = read_response(&mut adapter)?;
        parse_response(&raw)
    } else {
        // Try a pooled connection first, but only for idempotent methods.
        // Re-sending a POST/PUT/PATCH on a stale connection could cause
        // duplicate side-effects on the server.
        let is_idempotent = matches!(method, "GET" | "HEAD" | "OPTIONS" | "TRACE");
        let pooled = if is_idempotent {
            CONN_POOL.with(|pool| pool.borrow_mut().take(host, port))
        } else {
            None
        };
        if let Some(mut stream) = pooled {
            match try_request_on_stream(&mut stream, method, url, body, extra_headers, is_https) {
                Ok(resp) => {
                    maybe_return_to_pool(&resp, host, port, stream);
                    return Ok(resp);
                },
                Err(_) => {
                    // Stale connection -- fall through to fresh connect.
                },
            }
        }

        let mut stream = tcp_connect(host, port)?;
        send_request(&mut stream, method, url, body, extra_headers, is_https)?;
        let raw = read_response(&mut stream)?;
        let resp = parse_response(&raw)?;
        maybe_return_to_pool(&resp, host, port, stream);
        Ok(resp)
    }
}

/// Attempt a full request/response cycle on an existing stream.
///
/// Returns `Err` on any I/O failure so the caller can retry with a fresh
/// connection.
fn try_request_on_stream(
    stream: &mut TcpStream,
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    is_https: bool,
) -> Result<HttpResponse> {
    send_request(stream, method, url, body, extra_headers, is_https)?;
    let raw = read_response(stream)?;
    parse_response(&raw)
}

/// Return a plain-HTTP connection to the pool if the response indicates
/// keep-alive (HTTP/1.1 default unless `Connection: close` is present).
fn maybe_return_to_pool(resp: &HttpResponse, host: &str, port: u16, stream: TcpStream) {
    let dominated_close =
        find_header(&resp.headers, "connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
    if !dominated_close {
        CONN_POOL.with(|pool| pool.borrow_mut().put(host, port, stream));
    }
}

/// Open a TCP connection with a connect timeout.
fn tcp_connect(host: &str, port: u16) -> Result<TcpStream> {
    use std::net::ToSocketAddrs;

    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| OasisError::Backend(format!("DNS resolution failed: {e}").into()))?
        .next()
        .ok_or_else(|| OasisError::Backend(format!("no addresses for {host}:{port}").into()))?;

    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| OasisError::Backend(format!("TCP connect failed: {e}").into()))?;

    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| OasisError::Backend(format!("set read timeout: {e}").into()))?;

    Ok(stream)
}

/// Send an HTTP/1.1 request with the given method, optional body, and
/// optional extra headers.
fn send_request(
    stream: &mut impl Write,
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    is_https: bool,
) -> Result<()> {
    let default_port: u16 = if is_https { 443 } else { 80 };
    let host_header = match url.port {
        Some(p) if p != default_port => format!("{}:{}", url.host, p),
        _ => url.host.clone(),
    };

    let path = if let Some(ref q) = url.query {
        format!("{}?{}", url.path, q)
    } else {
        url.path.clone()
    };

    let mut request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         User-Agent: OASIS/1.0\r\n\
         Accept: */*\r\n\
         Accept-Encoding: gzip, deflate, br\r\n\
         Connection: keep-alive\r\n"
    );

    // Append extra headers.
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }

    // For POST with a body, add Content-Type and Content-Length if not
    // already provided by extra_headers.
    if let Some(data) = body {
        let has_ct = extra_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"));
        if !has_ct {
            request.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        }
        request.push_str(&format!("Content-Length: {}\r\n", data.len()));
    }

    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .map_err(|e| OasisError::Backend(format!("send request: {e}").into()))?;

    // Write body after headers.
    if let Some(data) = body {
        stream
            .write_all(data)
            .map_err(|e| OasisError::Backend(format!("send body: {e}").into()))?;
    }

    Ok(())
}

/// Read the entire HTTP response, stopping when we have the complete body.
///
/// With `Connection: keep-alive` the server does not close the connection
/// after the response, so we cannot rely on EOF. Instead we parse headers
/// as they arrive to determine the body length:
/// - `Content-Length`: stop after that many body bytes.
/// - `Transfer-Encoding: chunked`: stop after the `0\r\n\r\n` terminator.
/// - Neither: fall back to reading until EOF or timeout.
fn read_response(stream: &mut impl Read) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let mut body_start: Option<usize> = None;
    let mut expected_body_len: Option<usize> = None;
    let mut is_chunked = false;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > MAX_BODY_SIZE + MAX_HEADER_SIZE {
                    return Err(OasisError::Backend("response too large".into()));
                }
                buf.extend_from_slice(&chunk[..n]);

                // Once we find the header/body boundary, determine body length.
                if body_start.is_none() {
                    if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                        let hdr_end = pos + 4;
                        body_start = Some(hdr_end);
                        let header_bytes = std::str::from_utf8(&buf[..pos]).unwrap_or("");
                        let header_lower = header_bytes.to_ascii_lowercase();
                        if header_lower.contains("transfer-encoding")
                            && header_lower.contains("chunked")
                        {
                            is_chunked = true;
                        } else if let Some(cl_start) = header_lower.find("content-length:") {
                            let after = &header_lower[cl_start + 15..];
                            let line_end = after.find('\n').unwrap_or(after.len());
                            if let Ok(cl) = after[..line_end].trim().parse::<usize>() {
                                expected_body_len = Some(cl);
                            }
                        }
                    } else if find_subsequence(&buf, b"\n\n").is_some() {
                        // Bare \n\n -- fall through to EOF-based reading.
                        body_start = Some(buf.len());
                    } else if buf.len() > MAX_HEADER_SIZE {
                        return Err(OasisError::Backend(
                            "HTTP headers exceed 16 KB limit".into(),
                        ));
                    }
                }

                // Check if we have the complete body.
                if let Some(bs) = body_start {
                    if let Some(expected) = expected_body_len {
                        // Content-Length: stop once we have enough bytes.
                        if buf.len() - bs >= expected {
                            // Trim any excess bytes (pipelined response).
                            buf.truncate(bs + expected);
                            break;
                        }
                    } else if is_chunked {
                        // Chunked: stop after the final `0\r\n\r\n` marker.
                        // Only check the tail of the buffer to avoid false
                        // positives from binary data containing the same
                        // byte sequence mid-stream.
                        let chunk_data = &buf[bs..];
                        if chunk_data.ends_with(b"\r\n0\r\n\r\n")
                            || chunk_data.ends_with(b"\r\n0\r\n")
                            || chunk_data.ends_with(b"0\r\n\r\n")
                            || (chunk_data.starts_with(b"0\r\n") && chunk_data.len() <= 5)
                        {
                            break;
                        }
                    }
                    // No content-length, not chunked: read until EOF/timeout.
                }
            },
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                break;
            },
            Err(e) => {
                return Err(OasisError::Backend(format!("read response: {e}").into()));
            },
        }
    }
    Ok(buf)
}

/// Parse raw bytes into status code, headers, and body.
///
/// Accepts both `\r\n` (standard) and bare `\n` (RFC 7230 §3.5) line
/// endings in the header section.
pub fn parse_response(data: &[u8]) -> Result<HttpResponse> {
    // Find the header/body boundary.  Try canonical \r\n\r\n first,
    // then fall back to bare \n\n (RFC 7230 §3.5 robustness).
    let (header_end, separator_len) = if let Some(pos) = find_subsequence(data, b"\r\n\r\n") {
        (pos, 4)
    } else if let Some(pos) = find_subsequence(data, b"\n\n") {
        (pos, 2)
    } else {
        return Err(OasisError::Backend(
            "malformed HTTP response: no header terminator".into(),
        ));
    };

    if header_end > MAX_HEADER_SIZE {
        return Err(OasisError::Backend(
            "HTTP headers exceed 16 KB limit".into(),
        ));
    }

    let header_bytes = &data[..header_end];
    let body_start = header_end + separator_len;

    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|_| OasisError::Backend("non-UTF-8 headers".into()))?;

    // Normalize \r\n to \n so the rest of the parser handles both.
    let header_owned;
    let header_normalized = if header_str.contains("\r\n") {
        header_owned = header_str.replace("\r\n", "\n");
        header_owned.as_str()
    } else {
        header_str
    };

    let mut lines = header_normalized.split('\n');

    // Status line: "HTTP/1.x STATUS REASON"
    let status_line = lines
        .next()
        .ok_or_else(|| OasisError::Backend("empty response".into()))?;
    let status_code = parse_status_line(status_line)?;

    // Parse headers.
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }

    // Decode body.
    let raw_body = &data[body_start..];
    let body = if find_header(&headers, "transfer-encoding").is_some_and(|v| v.contains("chunked"))
    {
        decode_chunked(raw_body)?
    } else if let Some(cl) = find_header(&headers, "content-length") {
        let len: usize = cl
            .parse()
            .map_err(|_| OasisError::Backend("bad Content-Length".into()))?;
        if len > MAX_BODY_SIZE {
            return Err(OasisError::Backend(
                "response body exceeds 8 MB limit".into(),
            ));
        }
        raw_body[..raw_body.len().min(len)].to_vec()
    } else {
        raw_body.to_vec()
    };

    if body.len() > MAX_BODY_SIZE {
        return Err(OasisError::Backend(
            "response body exceeds 8 MB limit".into(),
        ));
    }

    // Decompress body if content-encoding is gzip or deflate.
    let body = decode_body(&headers, body)?;

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

/// Public wrapper for [`decode_body`] so the HTTP/2 driver can reuse
/// the same gzip/deflate/brotli plumbing.
pub(super) fn decode_body_public(headers: &[(String, String)], body: Vec<u8>) -> Result<Vec<u8>> {
    decode_body(headers, body)
}

/// Decompress the response body based on the `Content-Encoding` header.
fn decode_body(headers: &[(String, String)], body: Vec<u8>) -> Result<Vec<u8>> {
    if body.is_empty() {
        return Ok(body);
    }

    let encoding = match find_header(headers, "content-encoding") {
        Some(e) => e.trim().to_lowercase(),
        None => return Ok(body),
    };

    match encoding.as_str() {
        "gzip" => {
            let mut decoder = GzDecoder::new(&body[..]);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| OasisError::Backend(format!("gzip decode: {e}").into()))?;
            Ok(decompressed)
        },
        "deflate" => {
            let mut decoder = DeflateDecoder::new(&body[..]);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| OasisError::Backend(format!("deflate decode: {e}").into()))?;
            Ok(decompressed)
        },
        "br" => {
            let mut decoder = BrotliDecoder::new(&body[..], 4096);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| OasisError::Backend(format!("brotli decode: {e}").into()))?;
            Ok(decompressed)
        },
        _ => Ok(body),
    }
}

/// Parse the HTTP status code from the status line.
fn parse_status_line(line: &str) -> Result<u16> {
    // Expected: "HTTP/1.x NNN ..."
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(OasisError::Backend(
            format!("bad status line: {line}").into(),
        ));
    }
    parts[1]
        .parse()
        .map_err(|_| OasisError::Backend(format!("bad status code in: {line}").into()))
}

/// Case-insensitive header lookup.
fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let name_lower = name.to_lowercase();
    headers
        .iter()
        .find(|(k, _)| k == &name_lower)
        .map(|(_, v)| v.as_str())
}

/// Decode a chunked transfer-encoded body.
fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let remaining = &data[pos..];
        let Some(i) = find_subsequence(remaining, b"\r\n") else {
            break;
        };
        let line_end = pos + i;

        let size_str = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| OasisError::Backend("bad chunk size".into()))?
            .trim();

        // Strip optional chunk extensions (after `;`).
        let size_str = size_str.split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|_| OasisError::Backend("bad chunk size".into()))?;

        if chunk_size == 0 {
            break;
        }

        let chunk_start = line_end + 2;
        let chunk_end = match chunk_start.checked_add(chunk_size) {
            Some(end) => end,
            None => break, // overflow
        };

        if chunk_end > data.len() {
            // Partial chunk -- take what we have.
            if chunk_start < data.len() {
                result.extend_from_slice(&data[chunk_start..]);
            }
            break;
        }

        if result.len() + chunk_size > MAX_BODY_SIZE {
            return Err(OasisError::Backend(
                "chunked body exceeds 8 MB limit".into(),
            ));
        }

        result.extend_from_slice(&data[chunk_start..chunk_end]);
        // Skip past chunk data and trailing \r\n.
        pos = match chunk_end.checked_add(2) {
            Some(p) => p,
            None => break, // overflow
        };
    }

    Ok(result)
}

/// Whether a status code is a redirect we should follow.
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Find the position of a byte subsequence in a slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// -------------------------------------------------------------------
// NetworkStream → Read + Write adapter
// -------------------------------------------------------------------

/// Adapts a `Box<dyn NetworkStream>` to `std::io::Read` + `std::io::Write`
/// so it can be used with the generic `send_request` / `read_response`.
pub(super) struct NetworkStreamAdapter(pub(super) Box<dyn NetworkStream>);

impl Read for NetworkStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(oasis_err_to_io)
    }
}

impl Write for NetworkStreamAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf).map_err(oasis_err_to_io)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Convert an [`OasisError`] to [`io::Error`], preserving the original
/// `io::Error` (and its error kind) when the variant is `OasisError::Io`.
fn oasis_err_to_io(e: OasisError) -> io::Error {
    match e {
        OasisError::Io(io_err) => io_err,
        other => io::Error::other(other.to_string()),
    }
}

// -------------------------------------------------------------------
// Error pages
// -------------------------------------------------------------------

/// Generate a user-friendly error page when a site requires HTTPS.
fn https_error_page(original_url: &Url, https_url: &Url) -> ResourceResponse {
    let html = format!(
        "<html><body>\
         <h1>HTTPS Required</h1>\
         <p>This site redirected to a secure (HTTPS) connection:</p>\
         <p>{https_url}</p>\
         <p>OASIS browser only supports plain HTTP. \
         TLS/SSL is not available.</p>\
         <p>Try a site that serves plain HTTP, such as:</p>\
         <p>http://example.com</p>\
         </body></html>"
    );
    ResourceResponse {
        url: original_url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 200,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_response() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html\r\n\
                     Content-Length: 13\r\n\
                     \r\n\
                     <html>hi</html>";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(
            find_header(&resp.headers, "content-type"),
            Some("text/html"),
        );
        // Body is trimmed to Content-Length (13 bytes).
        assert_eq!(resp.body, b"<html>hi</htm");
    }

    #[test]
    fn parse_response_no_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Content-Type: text/plain\r\n\
                     \r\n\
                     hello world";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, b"hello world");
    }

    #[test]
    fn parse_404_response() {
        let raw = b"HTTP/1.1 404 Not Found\r\n\
                     Content-Length: 9\r\n\
                     \r\n\
                     not found";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 404);
        assert_eq!(resp.body, b"not found");
    }

    #[test]
    fn parse_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Transfer-Encoding: chunked\r\n\
                     \r\n\
                     5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.body, b"hello world");
    }

    #[test]
    fn decode_chunked_basic() {
        let data = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"hello world");
    }

    #[test]
    fn decode_chunked_with_extension() {
        let data = b"5;ext=val\r\nhello\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn https_returns_error_page_without_tls() {
        let url = Url::parse("https://example.com/page").unwrap();
        let resp = http_get(&url, None).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("HTTPS Required"));
    }

    #[test]
    fn unsupported_scheme_rejected() {
        let url = Url::parse("ftp://example.com/file").unwrap();
        let err = http_get(&url, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unsupported scheme"));
    }

    #[test]
    fn redirect_location_detected() {
        let raw = b"HTTP/1.1 301 Moved\r\n\
                     Location: /new-page\r\n\
                     \r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 301);
        assert!(is_redirect(resp.status_code));
        assert_eq!(find_header(&resp.headers, "location"), Some("/new-page"),);
    }

    #[test]
    fn case_insensitive_header_lookup() {
        let headers = vec![
            ("content-type".to_string(), "text/html".to_string()),
            ("x-custom".to_string(), "value".to_string()),
        ];
        assert_eq!(find_header(&headers, "Content-Type"), Some("text/html"));
        assert_eq!(find_header(&headers, "CONTENT-TYPE"), Some("text/html"));
        assert_eq!(find_header(&headers, "X-Custom"), Some("value"));
        assert_eq!(find_header(&headers, "missing"), None);
    }

    #[test]
    fn max_body_enforced_content_length() {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_SIZE + 1,
        );
        let err = parse_response(header.as_bytes()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("8 MB"));
    }

    #[test]
    fn is_redirect_codes() {
        assert!(is_redirect(301));
        assert!(is_redirect(302));
        assert!(is_redirect(307));
        assert!(is_redirect(308));
        assert!(!is_redirect(200));
        assert!(!is_redirect(404));
        assert!(!is_redirect(500));
    }

    #[test]
    fn parse_status_line_ok() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK").unwrap(), 200);
        assert_eq!(
            parse_status_line("HTTP/1.0 301 Moved Permanently").unwrap(),
            301,
        );
    }

    #[test]
    fn parse_status_line_bad() {
        assert!(parse_status_line("garbage").is_err());
    }

    #[test]
    fn find_subsequence_works() {
        assert_eq!(
            find_subsequence(b"hello\r\n\r\nworld", b"\r\n\r\n"),
            Some(5)
        );
        assert_eq!(find_subsequence(b"no boundary", b"\r\n\r\n"), None);
    }

    #[test]
    fn decode_chunked_truncated_no_panic() {
        // Chunk size says 100 but data is much shorter -- must not panic.
        let data = b"64\r\nshort";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"short");
    }

    #[test]
    fn decode_chunked_huge_size_no_panic() {
        // Chunk size overflows usize::MAX-range -- must not panic.
        let data = b"fffffffffffffffe\r\ndata\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert!(result.is_empty() || result == b"data");
    }

    #[test]
    fn parse_response_fuzz_crash_regression() {
        // Crash input from fuzzer: malformed chunked response with truncated
        // chunks that previously caused slice-out-of-bounds panics.
        let crash: &[u8] = &[
            0x50, 0x2f, 0x31, 0x20, 0x32, 0x30, 0x30, 0x20, 0x4f, 0x0a, 0x44, 0x00, 0x00, 0x0a,
            0x20, 0x00, 0x0a, 0x0a, 0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x0a, 0x70,
            0x3e, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20, 0x4a, 0x0d, 0x0a, 0x44, 0x00, 0x00,
            0x0a, 0x0d, 0x0a, 0x0a, 0x0d, 0x0a, 0x0a, 0x3c, 0x00, 0x20, 0x0a, 0x0a, 0x0d, 0x0a,
            0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20, 0x4a, 0x0d, 0x0a, 0x44, 0x00, 0x00, 0x0a,
            0x0d, 0x0a, 0x0a, 0x0d, 0x0a, 0x0a, 0x20, 0x31, 0x32, 0x0d, 0x48, 0x32, 0x5d, 0x44,
            0x00, 0x00, 0x0a, 0x20, 0x00, 0x0a, 0x0a, 0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d,
            0x3a, 0x20, 0x31, 0x76, 0x65, 0x64, 0x4d, 0x0a, 0x48, 0x65, 0x0d, 0x0a, 0x43, 0x0a,
            0x44, 0x00, 0x00, 0x0a, 0x20, 0x00, 0x0a, 0x0a, 0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9,
            0x0d, 0x3a, 0x0a, 0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20, 0x4a, 0x0d,
            0x0a, 0x44, 0x00, 0x00, 0x30, 0x2e, 0x31, 0x20, 0x32, 0x30, 0x30, 0x20, 0x0d, 0x6c,
            0x0d, 0x36, 0x31, 0x0d, 0x0a, 0x74, 0x72, 0x61, 0x6e, 0x73, 0x66, 0x65, 0x72, 0x2d,
            0x65, 0x6e, 0x63, 0x6f, 0x64, 0x69, 0x6e, 0x67, 0x0a, 0x3a, 0x4d, 0x63, 0x68, 0x75,
            0x6e, 0x6b, 0x65, 0x64, 0x4e, 0x00, 0x0a, 0x3a, 0x2d, 0x00, 0x00, 0x00, 0x00, 0x74,
            0x68, 0x3a, 0x0d, 0x0a, 0x2f, 0x20, 0x0d, 0x0a, 0x0d, 0x0a, 0x32, 0x30, 0x32, 0x0d,
            0x0a, 0xf3, 0x0a, 0x0d, 0x0a, 0x0a, 0x0d, 0x0a, 0x0a, 0x3c, 0x00, 0x20, 0x0a, 0x70,
            0xc2, 0xb9, 0x0d, 0x3a, 0x0a, 0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20,
            0x6e, 0x0d, 0x3a, 0x20, 0x31, 0x33, 0x0d, 0x48, 0x32, 0x5d, 0x44, 0x00, 0x00, 0x0a,
            0x0d, 0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20, 0x6e, 0x0d, 0x3a, 0x20, 0x31,
            0x32, 0x0d, 0x48, 0x32, 0x5d, 0x44, 0x00, 0x00, 0x0a, 0x20, 0x00, 0x0a, 0x0a, 0x0d,
            0x0a, 0x3c, 0x70, 0xc2, 0xb9, 0x0d, 0x3a, 0x20, 0x31, 0x76, 0x65, 0x64, 0x4d, 0x0a,
            0x48, 0x65, 0x0d, 0x0a, 0xc0, 0x91, 0xaf, 0xd0, 0xce, 0xdf, 0xcd, 0xcf, 0x30, 0x20,
            0x4f, 0x0a, 0x44, 0x00, 0x00, 0x0a, 0x20, 0x00, 0x0a, 0x0a, 0x0d,
        ];
        // Must not panic. Either Ok or Err is fine.
        let _ = parse_response(crash);
    }

    #[test]
    fn http_to_https_redirect_without_tls() {
        use std::io::Write as IoWrite;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 301 Moved\r\n\
                 Location: https://127.0.0.1:{port}/secure\r\n\
                 Content-Length: 0\r\n\
                 \r\n"
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        });

        let url = Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();
        // No TLS provider -- redirect to HTTPS should produce error page.
        let resp = http_get(&url, None).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(
            body.contains("HTTPS Required"),
            "expected HTTPS Required page, got: {body}",
        );
        let _ = handle.join();
    }

    #[test]
    fn parse_response_lf_only() {
        let raw = b"HTTP/1.1 200 OK\n\
                     Content-Type: text/plain\n\
                     Content-Length: 5\n\
                     \n\
                     hello";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(
            find_header(&resp.headers, "content-type"),
            Some("text/plain"),
        );
        assert_eq!(resp.body, b"hello");
    }

    #[test]
    fn header_size_limit_enforced() {
        // Build a response with headers exceeding MAX_HEADER_SIZE.
        let mut huge = b"HTTP/1.1 200 OK\r\n".to_vec();
        // Each header line ~110 bytes; 160 of them ≈ 17.6 KB > 16 KB.
        for i in 0..160 {
            let line = format!("X-Pad-{i}: {}\r\n", "A".repeat(90));
            huge.extend_from_slice(line.as_bytes());
        }
        huge.extend_from_slice(b"\r\n");
        huge.extend_from_slice(b"body");
        let err = parse_response(&huge).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("16 KB"), "expected header limit error: {msg}");
    }
}
