//! The pollable MCP server object.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::OasisError;

use crate::dispatch::{Handled, handle_message};
use crate::http::{
    CONTINUE_RESPONSE, Framing, HttpRequest, build_response, expects_continue, find_subsequence,
    try_frame,
};
use crate::tools::ToolDispatcher;

const DEFAULT_MAX_CONNECTIONS: usize = 4;
const IDLE_TIMEOUT_SECS: u64 = 300;
/// A request's header section must arrive within this window (slowloris guard).
const HEADER_TIMEOUT_SECS: u64 = 10;
const READ_CHUNK: usize = 4096;
/// Maximum bytes read from one connection in a single poll.
const MAX_READ_PER_POLL: usize = 64 * 1024;
/// Write-buffer high-water mark. Once a connection has this many unsent bytes
/// queued (the peer is not draining responses) the server stops reading and
/// dispatching further requests on it until the backlog drains, so memory per
/// connection stays bounded at roughly this cap plus one response.
const MAX_WRITE_BUF: usize = 4 * 1024 * 1024;
/// Lingering close: after the final response of a connection has been
/// written, keep reading (and discarding) input until the peer has been quiet
/// this long, so the close does not hit unread data. Closing a socket with
/// unread input makes the OS send RST, which can destroy the response still
/// in flight (e.g. a `413` while the client is streaming an oversized body).
const LINGER_QUIET_MS: u64 = 100;
/// Hard cap on how long a lingering close may take.
const LINGER_MAX_SECS: u64 = 2;

/// Constant-time byte comparison (avoids leaking the token via timing).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn is_would_block(e: &OasisError) -> bool {
    matches!(e, OasisError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}

struct HttpConn {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    write_buf: Vec<u8>,
    last_activity: Instant,
    /// When the currently pending (header-incomplete) request started: set at
    /// accept and when the first byte of a subsequent request arrives; `None`
    /// while the connection sits idle between requests.
    request_started: Option<Instant>,
    /// Peer half-closed the connection (read returned 0).
    eof: bool,
    close_after_flush: bool,
    /// A `100 Continue` was already sent for the request being received.
    continue_sent: bool,
    /// Lingering-close state: `(started, last input seen)`.
    linger: Option<(Instant, Instant)>,
}

impl HttpConn {
    fn new(stream: Box<dyn NetworkStream>) -> Self {
        let now = Instant::now();
        Self {
            stream,
            read_buf: Vec::with_capacity(READ_CHUNK),
            write_buf: Vec::new(),
            last_activity: now,
            request_started: Some(now),
            eof: false,
            close_after_flush: false,
            continue_sent: false,
            linger: None,
        }
    }

    /// Lingering close step: discard whatever input is available (bounded
    /// per poll, never buffered). Returns `true` once the connection can be
    /// closed without resetting it: the peer hit EOF, went quiet for `quiet`,
    /// errored, or the `max` linger time elapsed.
    fn linger_done(&mut self, quiet: Duration, max: Duration) -> bool {
        let now = Instant::now();
        let (started, mut last_input) = *self.linger.get_or_insert((now, now));
        let mut buf = [0u8; READ_CHUNK];
        let mut total = 0usize;
        while !self.eof && total < MAX_READ_PER_POLL {
            match self.stream.read(&mut buf) {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    total += n;
                    last_input = now;
                },
                Err(ref e) if is_would_block(e) => break,
                Err(_) => return true,
            }
        }
        self.linger = Some((started, last_input));
        self.eof || last_input.elapsed() >= quiet || started.elapsed() >= max
    }

    fn queue_write(&mut self, bytes: &[u8]) {
        self.write_buf.extend_from_slice(bytes);
    }

    fn headers_complete(&self) -> bool {
        find_subsequence(&self.read_buf, b"\r\n\r\n").is_some()
    }

    /// Read until the socket would block, EOF, or `MAX_READ_PER_POLL` bytes.
    ///
    /// Returns `false` on a hard read error (the connection should be dropped).
    fn read_available(&mut self) -> bool {
        let mut buf = [0u8; READ_CHUNK];
        let mut total = 0usize;
        while total < MAX_READ_PER_POLL {
            let want = READ_CHUNK.min(MAX_READ_PER_POLL - total);
            match self.stream.read(&mut buf[..want]) {
                Ok(0) => {
                    self.eof = true;
                    break;
                },
                Ok(n) => {
                    total += n;
                    self.last_activity = Instant::now();
                    if self.read_buf.is_empty() && self.request_started.is_none() {
                        self.request_started = Some(Instant::now());
                    }
                    self.read_buf.extend_from_slice(&buf[..n]);
                },
                Err(ref e) if is_would_block(e) => break,
                Err(e) => {
                    log::debug!("mcp read error: {e}");
                    return false;
                },
            }
        }
        true
    }

    /// Write as much of `write_buf` as the non-blocking socket will accept.
    fn flush_writes(&mut self) {
        while !self.write_buf.is_empty() {
            match self.stream.write(&self.write_buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.write_buf.drain(..n);
                },
                Err(ref e) if is_would_block(e) => break,
                Err(e) => {
                    log::debug!("mcp write error: {e}");
                    self.write_buf.clear();
                    self.close_after_flush = true;
                    break;
                },
            }
        }
        let _ = self.stream.flush();
    }
}

/// A minimal MCP server over Streamable HTTP, driven by per-frame polling.
///
/// The server owns its own (already-listening) network backend so it never
/// contends with other listeners in the host. Construct it with a backend that
/// has been bound to a loopback port, then call [`McpServer::poll`] once per
/// frame from the main loop.
pub struct McpServer {
    backend: Box<dyn NetworkBackend>,
    conns: Vec<HttpConn>,
    max_connections: usize,
    idle_timeout: Duration,
    header_timeout: Duration,
    linger_quiet: Duration,
    linger_max: Duration,
    /// Optional bearer token required on every request.
    token: Option<String>,
}

impl McpServer {
    /// Create a server over an already-listening `backend`.
    ///
    /// If `token` is `Some`, every request must carry a matching
    /// `Authorization: Bearer <token>` header.
    pub fn new(backend: Box<dyn NetworkBackend>, token: Option<String>) -> Self {
        Self {
            backend,
            conns: Vec::new(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            idle_timeout: Duration::from_secs(IDLE_TIMEOUT_SECS),
            header_timeout: Duration::from_secs(HEADER_TIMEOUT_SECS),
            linger_quiet: Duration::from_millis(LINGER_QUIET_MS),
            linger_max: Duration::from_secs(LINGER_MAX_SECS),
            token: token.filter(|t| !t.is_empty()),
        }
    }

    /// Number of currently open connections.
    pub fn connection_count(&self) -> usize {
        self.conns.len()
    }

    /// Close all connections.
    pub fn stop(&mut self) {
        for conn in &mut self.conns {
            let _ = conn.stream.close();
        }
        self.conns.clear();
    }

    /// Accept new connections, read pending requests, dispatch them against
    /// `disp`, and write responses. Non-blocking; returns within the frame.
    pub fn poll(&mut self, disp: &mut dyn ToolDispatcher) {
        // Accept at most one new connection per poll (matches RemoteListener).
        if self.conns.len() < self.max_connections {
            match self.backend.accept() {
                Ok(Some(stream)) => self.conns.push(HttpConn::new(stream)),
                Ok(None) => {},
                Err(e) => log::warn!("mcp accept error: {e}"),
            }
        }

        let token = self.token.clone();
        let idle_timeout = self.idle_timeout;
        let header_timeout = self.header_timeout;
        let (linger_quiet, linger_max) = (self.linger_quiet, self.linger_max);
        let mut to_remove = Vec::new();

        for (idx, conn) in self.conns.iter_mut().enumerate() {
            conn.flush_writes();

            if conn.last_activity.elapsed() > idle_timeout {
                to_remove.push(idx);
                continue;
            }

            // Slowloris guard: a request whose header section has not
            // completed within the window gets a 408 and the connection closes.
            if !conn.close_after_flush
                && conn
                    .request_started
                    .is_some_and(|t| t.elapsed() >= header_timeout)
                && !conn.headers_complete()
            {
                conn.read_buf.clear();
                conn.queue_write(&build_response(408, false, &[], None, b""));
                conn.close_after_flush = true;
                conn.flush_writes();
            }

            // Backpressure: stop consuming input while the peer is not
            // draining the responses already queued for it.
            if !conn.close_after_flush && conn.write_buf.len() < MAX_WRITE_BUF {
                if !conn.read_available() {
                    to_remove.push(idx);
                    continue;
                }

                while conn.write_buf.len() < MAX_WRITE_BUF && !conn.read_buf.is_empty() {
                    match try_frame(&mut conn.read_buf) {
                        Framing::Pending => {
                            // Headers are in but the body is not: a client
                            // that sent `Expect: 100-continue` is waiting.
                            if !conn.continue_sent && expects_continue(&conn.read_buf) {
                                conn.queue_write(CONTINUE_RESPONSE);
                                conn.continue_sent = true;
                            }
                            break;
                        },
                        Framing::Error(code) => {
                            conn.read_buf.clear();
                            conn.queue_write(&build_response(code, false, &[], None, b""));
                            conn.close_after_flush = true;
                            break;
                        },
                        Framing::Ready(req) => {
                            conn.continue_sent = false;
                            let keep_alive = req.keep_alive;
                            conn.queue_write(&handle_http_request(&req, disp, token.as_deref()));
                            // The next request's clock starts when its first
                            // byte is already buffered (pipelining) or arrives.
                            conn.request_started = (!conn.read_buf.is_empty()).then(Instant::now);
                            if !keep_alive {
                                conn.close_after_flush = true;
                                break;
                            }
                        },
                    }
                }
                if conn.eof {
                    conn.close_after_flush = true;
                }
                conn.flush_writes();
            }

            if conn.close_after_flush
                && conn.write_buf.is_empty()
                && conn.linger_done(linger_quiet, linger_max)
            {
                to_remove.push(idx);
            }
        }

        to_remove.sort_unstable();
        to_remove.dedup();
        for idx in to_remove.into_iter().rev() {
            if idx < self.conns.len() {
                let mut conn = self.conns.remove(idx);
                let _ = conn.stream.close();
            }
        }
    }
}

fn path_matches(path: &str) -> bool {
    path == "/mcp" || path == "/" || path.starts_with("/mcp?")
}

/// Whether `authority` (a `Host` value or the host part of an `Origin`) names
/// the loopback interface: `127.0.0.1`, `localhost` or `[::1]`, optionally
/// followed by `:port`.
fn is_loopback_authority(authority: &str) -> bool {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: `[::1]` or `[::1]:port`.
        let Some((h, after)) = rest.split_once(']') else {
            return false;
        };
        if !after.is_empty() && !after.starts_with(':') {
            return false;
        }
        (format!("[{h}]"), after.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    if let Some(p) = port
        && (p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "[::1]"
}

/// `Origin` check: absent (non-browser clients) is allowed; otherwise it must
/// be an `http`/`https` origin on a loopback authority. `Origin: null`
/// (sandboxed iframes, `file://`) is rejected.
fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let lower = origin.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    rest.is_some_and(is_loopback_authority)
}

/// `Host` check (DNS-rebinding defence): a rebound hostname such as
/// `attacker.example:7345` resolves to 127.0.0.1 but still carries the
/// attacker's name in `Host`, so only loopback names are accepted. Absent
/// `Host` (HTTP/1.0 tooling) is allowed; browsers always send it.
fn host_allowed(host: Option<&str>) -> bool {
    host.is_none_or(is_loopback_authority)
}

/// Whether a `Content-Type` value is `application/json` (parameters such as
/// `; charset=utf-8` are allowed).
fn is_json_content_type(ct: Option<&str>) -> bool {
    ct.is_some_and(|v| {
        let media = v.split(';').next().unwrap_or("").trim();
        media.eq_ignore_ascii_case("application/json")
    })
}

fn handle_http_request(
    req: &HttpRequest,
    disp: &mut dyn ToolDispatcher,
    token: Option<&str>,
) -> Vec<u8> {
    // Browser-originated attacks: a cross-site page can POST to loopback
    // (simple `text/plain` requests skip CORS preflight) and DNS rebinding can
    // make a foreign hostname resolve to 127.0.0.1. Reject both up front.
    if !origin_allowed(req.origin.as_deref()) || !host_allowed(req.host.as_deref()) {
        return build_response(403, false, &[], None, b"");
    }

    // Bearer-token gate (only POST carries a body worth protecting, but we
    // check every method for consistency).
    if let Some(expected) = token {
        let ok = req
            .auth_bearer
            .as_deref()
            .is_some_and(|got| constant_time_eq(got.as_bytes(), expected.as_bytes()));
        if !ok {
            return build_response(
                401,
                req.keep_alive,
                &[("WWW-Authenticate", "Bearer")],
                None,
                b"",
            );
        }
    }

    match req.method.as_str() {
        "POST" => {
            if !path_matches(&req.path) {
                return build_response(404, req.keep_alive, &[], None, b"");
            }
            // Requiring `application/json` forces a CORS preflight for any
            // cross-origin browser request (which we never approve).
            if !is_json_content_type(req.content_type.as_deref()) {
                return build_response(415, req.keep_alive, &[], None, b"");
            }
            match handle_message(&req.body, disp) {
                Handled::Notification => build_response(202, req.keep_alive, &[], None, b""),
                Handled::Response(body) => {
                    build_response(200, req.keep_alive, &[], Some("application/json"), &body)
                },
            }
        },
        // No server-initiated SSE stream: 405 is spec-permitted here.
        "GET" => build_response(405, req.keep_alive, &[("Allow", "POST, DELETE")], None, b""),
        "DELETE" => build_response(200, req.keep_alive, &[], None, b""),
        "OPTIONS" => build_response(
            204,
            req.keep_alive,
            &[("Allow", "POST, GET, DELETE, OPTIONS")],
            None,
            b"",
        ),
        _ => build_response(405, req.keep_alive, &[("Allow", "POST, DELETE")], None, b""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolResult, ToolSpec};
    use oasis_types::error::Result;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    /// Shared byte pipes for a single mock connection.
    #[derive(Default)]
    struct Pipe {
        client_to_server: Vec<u8>,
        server_to_client: Vec<u8>,
        /// When set, writes fail with `WouldBlock` (peer not draining).
        write_blocked: bool,
        closed: bool,
    }

    type SharedPipe = Arc<Mutex<Pipe>>;

    struct MockStream {
        pipe: SharedPipe,
    }

    fn lock(pipe: &SharedPipe) -> std::sync::MutexGuard<'_, Pipe> {
        pipe.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn would_block() -> OasisError {
        OasisError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "would block",
        ))
    }

    impl NetworkStream for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let mut p = lock(&self.pipe);
            if p.client_to_server.is_empty() {
                return Err(would_block());
            }
            let n = buf.len().min(p.client_to_server.len());
            let drained: Vec<u8> = p.client_to_server.drain(..n).collect();
            buf[..n].copy_from_slice(&drained);
            Ok(n)
        }
        fn write(&mut self, data: &[u8]) -> Result<usize> {
            let mut p = lock(&self.pipe);
            if p.write_blocked {
                return Err(would_block());
            }
            p.server_to_client.extend_from_slice(data);
            Ok(data.len())
        }
        fn close(&mut self) -> Result<()> {
            lock(&self.pipe).closed = true;
            Ok(())
        }
    }

    /// Hands out a single pending connection, then `None`.
    struct MockBackend {
        pending: Option<SharedPipe>,
    }

    impl NetworkBackend for MockBackend {
        fn listen(&mut self, _port: u16) -> Result<()> {
            Ok(())
        }
        fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
            match self.pending.take() {
                Some(pipe) => Ok(Some(Box::new(MockStream { pipe }))),
                None => Ok(None),
            }
        }
        fn connect(&mut self, _address: &str, _port: u16) -> Result<Box<dyn NetworkStream>> {
            Err(OasisError::Backend("unsupported".into()))
        }
    }

    struct StubDispatcher;
    impl ToolDispatcher for StubDispatcher {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec::new(
                "noop",
                "does nothing",
                json!({ "type": "object" }),
            )]
        }
        fn call_tool(&mut self, _name: &str, _args: Value) -> ToolResult {
            ToolResult::text("ok")
        }
    }

    /// Every tool call returns a 1 MiB text payload.
    struct BigDispatcher;
    impl ToolDispatcher for BigDispatcher {
        fn list_tools(&self) -> Vec<ToolSpec> {
            Vec::new()
        }
        fn call_tool(&mut self, _name: &str, _args: Value) -> ToolResult {
            ToolResult::text("x".repeat(1024 * 1024))
        }
    }

    const INIT_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    const PING_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

    /// Build a POST /mcp request with `extra_headers` (each `Name: value\r\n`).
    fn post(body: &str, extra_headers: &str) -> Vec<u8> {
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:7345\r\n\
             {extra_headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn json_post(body: &str) -> Vec<u8> {
        post(body, "Content-Type: application/json\r\n")
    }

    /// Server with one pending mock connection preloaded with `input`.
    fn server_with(input: &[u8], token: Option<&str>) -> (McpServer, SharedPipe) {
        let pipe: SharedPipe = Arc::new(Mutex::new(Pipe::default()));
        lock(&pipe).client_to_server.extend_from_slice(input);
        let backend = MockBackend {
            pending: Some(Arc::clone(&pipe)),
        };
        let server = McpServer::new(Box::new(backend), token.map(str::to_string));
        (server, pipe)
    }

    /// Accept + one read/dispatch poll.
    fn roundtrip(input: &[u8]) -> String {
        let (mut server, pipe) = server_with(input, None);
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        last_http_response(&pipe)
    }

    fn last_http_response(pipe: &SharedPipe) -> String {
        String::from_utf8_lossy(&lock(pipe).server_to_client).to_string()
    }

    #[test]
    fn end_to_end_initialize_over_socket() {
        let resp = roundtrip(&json_post(INIT_BODY));
        assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
        assert!(resp.contains("\"serverInfo\""));
    }

    #[test]
    fn get_returns_405() {
        let resp = roundtrip(b"GET /mcp HTTP/1.1\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 405"));
        assert!(resp.contains("Allow: POST, DELETE"));
    }

    #[test]
    fn missing_token_returns_401() {
        let (mut server, pipe) = server_with(&json_post(PING_BODY), Some("sekret"));
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        let resp = last_http_response(&pipe);
        assert!(resp.starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn cross_site_origin_returns_403() {
        let req = post(
            INIT_BODY,
            "Origin: https://evil.example\r\nContent-Type: application/json\r\n",
        );
        assert!(roundtrip(&req).starts_with("HTTP/1.1 403"));
        // `Origin: null` (sandboxed iframe / file://) is also rejected.
        let req = post(
            INIT_BODY,
            "Origin: null\r\nContent-Type: application/json\r\n",
        );
        assert!(roundtrip(&req).starts_with("HTTP/1.1 403"));
        // A loopback origin (e.g. a local dev tool) is allowed.
        let req = post(
            INIT_BODY,
            "Origin: http://localhost:3000\r\nContent-Type: application/json\r\n",
        );
        assert!(roundtrip(&req).starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn rebound_host_returns_403() {
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: attacker.example:7345\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{INIT_BODY}",
            INIT_BODY.len()
        );
        assert!(roundtrip(req.as_bytes()).starts_with("HTTP/1.1 403"));
        // Tricks that embed a loopback name must not pass either.
        for host in [
            "127.0.0.1.evil.example",
            "localhost:80:80",
            "[::1]x",
            "127.0.0.1:",
        ] {
            assert!(!is_loopback_authority(host), "{host} accepted");
        }
        for host in [
            "127.0.0.1",
            "localhost:7345",
            "LOCALHOST",
            "[::1]",
            "[::1]:7345",
        ] {
            assert!(is_loopback_authority(host), "{host} rejected");
        }
    }

    #[test]
    fn non_json_content_type_returns_415() {
        // `text/plain` is the CORS "simple request" content type.
        let resp = roundtrip(&post(INIT_BODY, "Content-Type: text/plain\r\n"));
        assert!(resp.starts_with("HTTP/1.1 415"), "{resp}");
        // Missing Content-Type is rejected too.
        assert!(roundtrip(&post(INIT_BODY, "")).starts_with("HTTP/1.1 415"));
        // Parameters are fine.
        let resp = roundtrip(&post(
            INIT_BODY,
            "Content-Type: Application/JSON; charset=utf-8\r\n",
        ));
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    }

    #[test]
    fn duplicate_content_length_returns_400() {
        let req = post(
            INIT_BODY,
            "Content-Type: application/json\r\nContent-Length: 3\r\n",
        );
        let resp = roundtrip(&req);
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
    }

    #[test]
    fn incomplete_headers_time_out_with_408() {
        let (mut server, pipe) = server_with(b"POST /mcp HTTP/1.1\r\nHost: 127", None);
        server.header_timeout = Duration::ZERO;
        server.linger_quiet = Duration::ZERO;
        let mut disp = StubDispatcher;
        // Accept; the (zero) header deadline has already passed -> 408 + close.
        server.poll(&mut disp);
        assert!(last_http_response(&pipe).starts_with("HTTP/1.1 408"));
        assert!(lock(&pipe).closed);
        assert_eq!(server.connection_count(), 0);
    }

    #[test]
    fn error_close_lingers_and_discards_trailing_input() {
        // A 413 while the client keeps streaming its body: the connection is
        // not closed on top of unread input (which would RST the response).
        let mut input = b"POST /mcp HTTP/1.1\r\nContent-Length: 9999999\r\n\r\n".to_vec();
        input.extend_from_slice(&[b' '; 1000]);
        let (mut server, pipe) = server_with(&input, None);
        server.linger_quiet = Duration::from_secs(60);
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        assert!(last_http_response(&pipe).starts_with("HTTP/1.1 413"));
        // More body arrives after the response: still lingering, input drained
        // without being buffered.
        lock(&pipe)
            .client_to_server
            .extend_from_slice(&[b' '; 5000]);
        server.poll(&mut disp);
        assert_eq!(server.connection_count(), 1);
        assert!(lock(&pipe).client_to_server.is_empty());
        assert!(server.conns[0].read_buf.is_empty());
        assert!(!lock(&pipe).closed);
        // Once the peer is quiet long enough, the connection closes.
        server.linger_quiet = Duration::ZERO;
        server.poll(&mut disp);
        assert_eq!(server.connection_count(), 0);
        assert!(lock(&pipe).closed);
    }

    #[test]
    fn idle_keepalive_between_requests_is_not_header_timed_out() {
        let (mut server, pipe) = server_with(&json_post(PING_BODY), None);
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        assert!(last_http_response(&pipe).starts_with("HTTP/1.1 200"));
        // With nothing buffered, the header clock is not running.
        server.header_timeout = Duration::ZERO;
        server.poll(&mut disp);
        assert_eq!(server.connection_count(), 1);
    }

    #[test]
    fn reads_until_would_block_within_one_poll() {
        // Three pipelined requests plus a >4 KiB body: all handled in the
        // first read poll, which the old single-4 KiB-read loop could not do.
        let padded = format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"ping"}}{}"#,
            " ".repeat(9000)
        );
        let mut input = json_post(PING_BODY);
        input.extend_from_slice(&json_post(PING_BODY));
        input.extend_from_slice(&json_post(&padded));
        let resp = roundtrip(&input);
        assert_eq!(resp.matches("HTTP/1.1 200 OK").count(), 3, "{resp}");
        assert!(resp.contains("\"id\":9"));
    }

    #[test]
    fn read_is_capped_per_poll() {
        let body = " ".repeat(200 * 1024);
        let input = json_post(&body);
        let total = input.len();
        let (mut server, pipe) = server_with(&input, None);
        let mut disp = StubDispatcher;
        server.poll(&mut disp); // accept + one capped read
        assert_eq!(
            lock(&pipe).client_to_server.len(),
            total - MAX_READ_PER_POLL
        );
    }

    #[test]
    fn write_buffer_is_capped_when_peer_stops_reading() {
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"big"}}"#;
        let mut input = Vec::new();
        for _ in 0..12 {
            input.extend_from_slice(&json_post(call));
        }
        let (mut server, pipe) = server_with(&input, None);
        lock(&pipe).write_blocked = true;
        let mut disp = BigDispatcher;
        for _ in 0..20 {
            server.poll(&mut disp);
        }
        let queued = server.conns[0].write_buf.len();
        // Bounded by the cap plus at most one (~1 MiB) response...
        assert!(queued < MAX_WRITE_BUF + 2 * 1024 * 1024, "queued {queued}");
        // ...and the remaining requests are left unprocessed, not dropped.
        assert!(queued >= MAX_WRITE_BUF);
        assert!(!server.conns[0].read_buf.is_empty());

        // Once the peer drains, the backlog is processed.
        lock(&pipe).write_blocked = false;
        for _ in 0..20 {
            server.poll(&mut disp);
        }
        let resp = last_http_response(&pipe);
        assert_eq!(resp.matches("HTTP/1.1 200 OK").count(), 12);
    }
}
