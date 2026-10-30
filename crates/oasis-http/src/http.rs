//! Minimal HTTP/1.1 GET client.
//!
//! Supports plain HTTP over `std::net::TcpStream` and, when a
//! [`TlsProvider`] is supplied, HTTPS via the backend's TLS stack.

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::net::TcpStream;
#[cfg(native_net)]
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use brotli::Decompressor as BrotliDecoder;
use flate2::read::{DeflateDecoder, GzDecoder};
use oasis_net::tls::TlsProvider;
use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::cookies::CookieHook;
use super::http1::{
    self, BodyLimit, Hop, MAX_HEADER_SIZE, Oversize, ResponseTracker, find_header, is_redirect,
};
use super::vfs::escape_html;
use super::{ContentType, ResourceResponse, Url};

pub use super::http1::HttpResponse;

/// Maximum response body size (8 MB).
const MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

/// Body-size policy of the desktop client: oversized bodies fail.
const BODY_LIMIT: BodyLimit = BodyLimit {
    max_body: MAX_BODY_SIZE,
    oversize: Oversize::Error,
};

/// Default maximum number of redirects to follow
/// (`BrowserConfig::max_redirects` overrides it for page loads).
pub const MAX_REDIRECTS: u8 = 5;

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

/// DNS cache TTL (5 minutes). The cache backs JS `fetch()` connect-src
/// checks (`oasis-browser`'s `js_dom::fetch`), so it only exists on
/// native-net targets.
#[cfg(native_net)]
const DNS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Global DNS cache: maps hostname -> (resolved IPs, cached time).
#[cfg(native_net)]
type DnsCacheMap = FxHashMap<String, (Vec<std::net::IpAddr>, Instant)>;

#[cfg(native_net)]
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
#[cfg(native_net)]
pub fn dns_resolve_cached(host: &str) -> io::Result<Vec<std::net::IpAddr>> {
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
/// A 301/302/303 redirect falls back to GET without the body (as
/// browsers do); 307/308 keep the method and body. Redirects to a
/// non-http(s) scheme are refused.
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
    http_request_inner(
        method,
        url,
        body,
        extra_headers,
        tls,
        RequestOptions::default(),
    )
}

/// Like [`http_request_full`], following at most `max_redirects`
/// redirects (0 = none), with cookie handling done per redirect hop: every hop is sent the `Cookie:` header `cookies` yields
/// for that hop's own URL, and every hop's `Set-Cookie` headers
/// (redirect responses included) are stored against the hop that sent
/// them. A `Cookie` header in `extra_headers` is ignored in favour of the
/// jar.
///
/// See [`http1::hop_headers`] for how the other caller-supplied headers
/// (`Referer`, conditional and credential headers) change across hops.
pub fn http_request_with_cookies(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
    cookies: &mut dyn CookieHook,
    max_redirects: u8,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    let opts = RequestOptions {
        max_redirects,
        cookies: Some(cookies),
        ..RequestOptions::default()
    };
    http_request_inner(method, url, body, extra_headers, tls, opts)
}

/// Policy hooks for [`http_request_guarded`].
///
/// Used by the JS `fetch()` binding to keep page-initiated requests off
/// loopback / private networks even via redirects or DNS rebinding.
pub struct RequestGuard<'a> {
    /// Consulted before following each redirect; `false` aborts the
    /// request with an error.
    pub redirect_ok: &'a dyn Fn(&Url) -> bool,
    /// Consulted with the resolved peer address right before every TCP
    /// connect; `false` aborts the request with an error. Guarded
    /// requests also bypass the keep-alive pool so every request is
    /// checked against a fresh resolution.
    pub addr_ok: &'a dyn Fn(std::net::IpAddr) -> bool,
}

/// Like [`http_request_full`] but enforces a [`RequestGuard`] on every
/// redirect hop and connection.
pub fn http_request_guarded(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
    guard: &RequestGuard<'_>,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    let opts = RequestOptions {
        guard: Some(guard),
        ..RequestOptions::default()
    };
    http_request_inner(method, url, body, extra_headers, tls, opts)
}

/// Per-request knobs of [`http_request_inner`].
struct RequestOptions<'a, 'g> {
    guard: Option<&'a RequestGuard<'g>>,
    max_redirects: u8,
    cookies: Option<&'a mut dyn CookieHook>,
}

impl Default for RequestOptions<'_, '_> {
    fn default() -> Self {
        Self {
            guard: None,
            max_redirects: MAX_REDIRECTS,
            cookies: None,
        }
    }
}

fn http_request_inner(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
    opts: RequestOptions<'_, '_>,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    let RequestOptions {
        guard,
        max_redirects,
        mut cookies,
    } = opts;
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
    let mut method_changed = false;

    // The initial request plus up to `max_redirects` followed redirects.
    for hop_index in 0..=usize::from(max_redirects) {
        // Headers are recomputed for every hop: credentials and cache
        // validators belong to the original URL, cookies to the hop.
        let hop = Hop {
            original: url,
            current: &current_url,
            index: hop_index,
            method_changed,
        };
        let mut headers = http1::hop_headers(extra_headers, hop);
        if let Some(jar) = cookies.as_deref_mut() {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("cookie"));
            if let Some(value) = jar.cookie_header(&current_url) {
                headers.push(("Cookie".to_string(), value));
            }
        }
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let resp = do_request_with_method(
            &current_method,
            &current_url,
            current_body.as_deref(),
            &header_refs,
            tls,
            guard.map(|g| g.addr_ok),
        )?;

        // Every hop's Set-Cookie is filed under the hop that sent it
        // (a login -> 302 -> dashboard flow sets its session here).
        if let Some(jar) = cookies.as_deref_mut() {
            jar.store(&current_url, &resp.headers);
        }

        if is_redirect(resp.status_code)
            && let Some(location) = find_header(&resp.headers, "location")
        {
            let location = location.to_string();
            current_url = current_url.resolve(&location).ok_or_else(|| {
                OasisError::Backend(format!("bad redirect Location: {location}").into())
            })?;
            // A network response may only redirect to the network: a
            // `Location: vfs://…` / `file:` / `about:` hop would hand the
            // attacker's body back labelled with a local URL (local-page
            // privileges: VFS subresources, no fetch origin checks).
            if current_url.scheme != "http" && current_url.scheme != "https" {
                return Err(OasisError::Backend(
                    format!("redirect to non-HTTP scheme blocked: {current_url}").into(),
                ));
            }
            if let Some(g) = guard
                && !(g.redirect_ok)(&current_url)
            {
                return Err(OasisError::Backend(
                    format!("redirect to {current_url} blocked by request policy").into(),
                ));
            }
            if current_url.scheme == "https" && tls.is_none() {
                return Ok((https_error_page(url, &current_url), Vec::new()));
            }
            // 301/302/303 revert to GET and drop the body (what browsers
            // do); 307/308 must preserve the method and body (RFC 9110
            // §15.4.8-9), as the PSP client already did.
            if !matches!(resp.status_code, 307 | 308) {
                method_changed |= current_method != "GET";
                current_method = "GET".to_string();
                current_body = None;
            }
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
    addr_ok: Option<&dyn Fn(std::net::IpAddr) -> bool>,
) -> Result<HttpResponse> {
    super::validate_request(method, url, extra_headers)?;
    let host = &url.host;
    let is_https = url.scheme == "https";
    let default_port = if is_https { 443 } else { 80 };
    let port = url.port.unwrap_or(default_port);

    if is_https {
        let tls_provider = tls.ok_or_else(|| OasisError::Backend("TLS not available".into()))?;

        let stream = tcp_connect(host, port, addr_ok)?;
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
        // duplicate side-effects on the server. Guarded requests never
        // reuse (or feed) the pool: every connect must pass `addr_ok`.
        let is_idempotent = matches!(method, "GET" | "HEAD" | "OPTIONS" | "TRACE");
        let pooled = if is_idempotent && addr_ok.is_none() {
            CONN_POOL.with(|pool| pool.borrow_mut().take(host, port))
        } else {
            None
        };
        if let Some(mut stream) = pooled {
            match try_request_on_stream(&mut stream, method, url, body, extra_headers, is_https) {
                Ok((resp, framed)) => {
                    if framed {
                        maybe_return_to_pool(&resp, host, port, stream);
                    }
                    return Ok(resp);
                },
                Err(_) => {
                    // Stale connection -- fall through to fresh connect.
                },
            }
        }

        let mut stream = tcp_connect(host, port, addr_ok)?;
        send_request(&mut stream, method, url, body, extra_headers, is_https)?;
        let (raw, framed) = read_response_framed(&mut stream)?;
        let resp = parse_response(&raw)?;
        if framed && addr_ok.is_none() {
            maybe_return_to_pool(&resp, host, port, stream);
        }
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
) -> Result<(HttpResponse, bool)> {
    send_request(stream, method, url, body, extra_headers, is_https)?;
    let (raw, framed) = read_response_framed(stream)?;
    Ok((parse_response(&raw)?, framed))
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
///
/// When `addr_ok` is given, the resolved peer address must pass it or
/// the connect is refused (checked on the exact address we dial, so a
/// DNS answer cannot change between the check and the connect).
fn tcp_connect(
    host: &str,
    port: u16,
    addr_ok: Option<&dyn Fn(std::net::IpAddr) -> bool>,
) -> Result<TcpStream> {
    use std::net::ToSocketAddrs;

    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| OasisError::Backend(format!("DNS resolution failed: {e}").into()))?
        .next()
        .ok_or_else(|| OasisError::Backend(format!("no addresses for {host}:{port}").into()))?;

    if let Some(check) = addr_ok
        && !check(addr.ip())
    {
        return Err(OasisError::Backend(
            format!("connection to {} blocked by request policy", addr.ip()).into(),
        ));
    }

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

    let path = url.request_target();

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
/// after the response, so we cannot rely on EOF. A [`ResponseTracker`]
/// parses the header lines as they arrive to find the body length
/// (`Content-Length`, or the chunked terminator) and stops there; without
/// either, reading runs until EOF or the read timeout.
fn read_response(stream: &mut impl Read) -> Result<Vec<u8>> {
    read_response_framed(stream).map(|(raw, _)| raw)
}

/// [`read_response`] plus whether the response ended at its framed
/// length (`Content-Length` / chunked terminator). Only such a
/// connection may go back to the keep-alive pool: after a timeout or
/// EOF the response may be truncated, and leftover bytes would be read
/// as the next request's response.
fn read_response_framed(stream: &mut impl Read) -> Result<(Vec<u8>, bool)> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let mut tracker = ResponseTracker::new();
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > MAX_BODY_SIZE + MAX_HEADER_SIZE {
                    return Err(OasisError::Backend("response too large".into()));
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = tracker.poll(&buf)? {
                    // Trim any excess bytes (pipelined response).
                    buf.truncate(end);
                    return Ok((buf, true));
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
    Ok((buf, false))
}

/// Parse raw bytes into status code, headers, and body, then undo any
/// `Content-Encoding` (gzip / deflate / brotli, bounded at 8 MB).
///
/// Accepts both `\r\n` (standard) and bare `\n` (RFC 9112 §2.2) line
/// endings in the header section.
pub fn parse_response(data: &[u8]) -> Result<HttpResponse> {
    let mut resp = http1::parse_response(data, BODY_LIMIT)?;
    resp.body = decode_body(&resp.headers, std::mem::take(&mut resp.body))?;
    Ok(resp)
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
        "gzip" => read_decoded_bounded(GzDecoder::new(&body[..]), "gzip"),
        "deflate" => read_decoded_bounded(DeflateDecoder::new(&body[..]), "deflate"),
        "br" => read_decoded_bounded(BrotliDecoder::new(&body[..], 4096), "brotli"),
        _ => Ok(body),
    }
}

/// Drain a decompressor, refusing output larger than [`MAX_BODY_SIZE`].
///
/// The compressed body is already capped, but a few KB of gzip can expand to
/// gigabytes (a "decompression bomb"), so the decoded size is capped too.
/// Reading at most `MAX_BODY_SIZE + 1` bytes lets us tell "exactly at the
/// limit" from "over it" without ever buffering more than that.
fn read_decoded_bounded(decoder: impl Read, name: &str) -> Result<Vec<u8>> {
    let mut decompressed = Vec::new();
    decoder
        .take(MAX_BODY_SIZE as u64 + 1)
        .read_to_end(&mut decompressed)
        .map_err(|e| OasisError::Backend(format!("{name} decode: {e}").into()))?;
    if decompressed.len() > MAX_BODY_SIZE {
        return Err(OasisError::Backend(
            format!("{name} decode: decompressed body exceeds 8 MB limit").into(),
        ));
    }
    Ok(decompressed)
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
    let escaped_https = escape_html(&https_url.to_string());
    let escaped_original = escape_html(&original_url.to_string());
    let html = format!(
        "<html><head><style>\
         body {{ font-family: sans-serif; margin: 40px; color: #333; \
                background: #f8f8f8; }}\
         .error-box {{ background: white; border: 1px solid #ddd; \
                       border-radius: 8px; padding: 24px; max-width: 480px; }}\
         h1 {{ color: #c33; font-size: 18px; margin: 0 0 12px 0; }}\
         p {{ font-size: 13px; line-height: 1.5; margin: 8px 0; }}\
         ul {{ font-size: 13px; line-height: 1.6; padding-left: 20px; }}\
         code {{ background: #eee; padding: 2px 4px; border-radius: 3px; \
                 font-size: 12px; word-break: break-all; }}\
         .detail {{ color: #888; font-size: 11px; margin-top: 16px; \
                    border-top: 1px solid #eee; padding-top: 12px; }}\
         </style></head>\
         <body><div class=\"error-box\">\
         <h1>HTTPS Required</h1>\
         <p>This site requires a secure (HTTPS) connection, but TLS/SSL \
         is not available in this browser configuration.</p>\
         <p>The site redirected to: <code>{escaped_https}</code></p>\
         <p><strong>Try:</strong></p>\
         <ul>\
         <li>Access a site that serves plain HTTP.</li>\
         <li>Enable TLS support if your platform supports it.</li>\
         </ul>\
         <div class=\"detail\">\
         <p>Original URL: <code>{escaped_original}</code></p>\
         </div></div></body></html>"
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
    use super::http1::{find_subsequence, parse_status_line};
    use super::*;

    fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
        http1::decode_chunked(data, BODY_LIMIT)
    }

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

    /// Yields `data` once, then a read timeout.
    struct ThenTimeout(Option<Vec<u8>>);

    impl Read for ThenTimeout {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.take() {
                Some(d) => {
                    buf[..d.len()].copy_from_slice(&d);
                    Ok(d.len())
                },
                None => Err(io::Error::new(io::ErrorKind::TimedOut, "timeout")),
            }
        }
    }

    #[test]
    fn only_framed_responses_count_as_complete() {
        // A truncated body ended by a timeout must not be reported as a
        // framed end: that connection would go back to the keep-alive
        // pool with the rest of the body still in flight.
        let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_vec();
        let (raw, framed) = read_response_framed(&mut ThenTimeout(Some(partial))).unwrap();
        assert!(!framed);
        assert!(raw.ends_with(b"abc"));
        let full = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc".to_vec();
        let (_, framed) = read_response_framed(&mut ThenTimeout(Some(full))).unwrap();
        assert!(framed);
    }

    #[test]
    fn redirect_to_local_scheme_is_refused() {
        use std::io::Write as IoWrite;
        use std::net::TcpListener;

        // Regression: `Location: vfs://…` used to be followed (as plain
        // HTTP to the same host) and the attacker's body came back
        // labelled `vfs://…`, i.e. with local-page privileges.
        for target in [
            "vfs://127.0.0.1/x",
            "file:///etc/passwd",
            "gemini://127.0.0.1/",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                // Serve every connection the same redirect (the vfs://
                // case would otherwise reconnect here).
                for _ in 0..2 {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {target}\r\n\
                         Content-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
            });
            let url = Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();
            let err = http_get(&url, None).expect_err(target);
            assert!(
                err.to_string().contains("non-HTTP scheme"),
                "{target}: {err}"
            );
            // Unblock the server's second accept and let it exit.
            let _ = std::net::TcpStream::connect(("127.0.0.1", port));
            let _ = handle.join();
        }
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

    // -- Decompression bomb protection --

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn encoding(value: &str) -> Vec<(String, String)> {
        vec![("content-encoding".to_string(), value.to_string())]
    }

    /// Decoded size well beyond the cap: 12 MiB of zeros vs an 8 MiB limit.
    const BOMB_SIZE: usize = 12 * 1024 * 1024;

    #[test]
    fn gzip_body_round_trips() {
        let body = gzip(b"<html>hello</html>");
        let out = decode_body(&encoding("gzip"), body).unwrap();
        assert_eq!(out, b"<html>hello</html>");
    }

    #[test]
    fn gzip_bomb_is_rejected() {
        const { assert!(BOMB_SIZE > MAX_BODY_SIZE) };
        let body = gzip(&vec![0u8; BOMB_SIZE]);
        // The compressed form is tiny, so it passes the wire-size limit...
        assert!(body.len() < 64 * 1024, "compressed {} bytes", body.len());
        // ...but decoding it must fail rather than allocate 12 MiB+.
        let err = decode_body(&encoding("gzip"), body).unwrap_err();
        assert!(err.to_string().contains("exceeds 8 MB"), "{err}");
    }

    #[test]
    fn gzip_body_exactly_at_limit_is_accepted() {
        let body = gzip(&vec![0u8; MAX_BODY_SIZE]);
        let out = decode_body(&encoding("gzip"), body).unwrap();
        assert_eq!(out.len(), MAX_BODY_SIZE);
    }

    #[test]
    fn deflate_bomb_is_rejected() {
        use std::io::Write as _;
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&vec![0u8; BOMB_SIZE]).unwrap();
        let body = enc.finish().unwrap();
        let err = decode_body(&encoding("deflate"), body).unwrap_err();
        assert!(err.to_string().contains("exceeds 8 MB"), "{err}");
    }

    #[test]
    fn brotli_bomb_is_rejected() {
        use std::io::Write as _;
        let mut body = Vec::new();
        {
            let mut enc = brotli::CompressorWriter::new(&mut body, 4096, 5, 22);
            enc.write_all(&vec![0u8; BOMB_SIZE]).unwrap();
        }
        let err = decode_body(&encoding("br"), body).unwrap_err();
        assert!(err.to_string().contains("exceeds 8 MB"), "{err}");
    }

    #[test]
    fn parse_response_rejects_gzip_bomb() {
        let body = gzip(&vec![0u8; BOMB_SIZE]);
        let mut raw = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(&body);
        assert!(parse_response(&raw).is_err());
    }
}
