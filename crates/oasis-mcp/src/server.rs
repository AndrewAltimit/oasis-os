//! The pollable MCP server object.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};

use crate::gate::{gate, handle_http_request, refusal};
use crate::http::{CONTINUE_RESPONSE, Framing, build_response, header_end, parse_head, try_frame};
use crate::tools::ToolDispatcher;

const DEFAULT_MAX_CONNECTIONS: usize = 4;
/// A keep-alive connection with no request in progress is closed after
/// this long, so idle sockets do not hold the few connection slots.
const IDLE_TIMEOUT_SECS: u64 = 60;
/// A request's header section must arrive within this window (slowloris guard).
const HEADER_TIMEOUT_SECS: u64 = 10;
/// The whole request (headers and body) must arrive within this window, so
/// a client cannot hold a slot by trickling a body one byte at a time.
const REQUEST_TIMEOUT_SECS: u64 = 30;
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
    /// The header section of the request being received has passed the
    /// Origin / Host / token / route checks (and `100 Continue` was sent if
    /// asked for): its body may be buffered.
    head_checked: bool,
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
            head_checked: false,
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
                Err(ref e) if e.is_would_block() => break,
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
        header_end(&self.read_buf).is_some()
    }

    /// A request is pending with its headers in but not its body: vet the
    /// headers now ([`gate`]), before buffering (up to 1 MiB of) body or
    /// inviting it with `100 Continue`. A refused request closes the
    /// connection, since its unread body cannot be skipped.
    fn vet_pending_head(&mut self, token: Option<&str>) {
        if self.head_checked {
            return;
        }
        let Some(end) = header_end(&self.read_buf) else {
            return;
        };
        // `try_frame` already reports a malformed head as an error.
        let Ok(head) = parse_head(&self.read_buf[..end]) else {
            return;
        };
        if let Some(status) = gate(&head, token) {
            self.read_buf.clear();
            self.queue_write(&refusal(status, false));
            self.close_after_flush = true;
        } else {
            self.head_checked = true;
            if head.expect_continue {
                self.queue_write(CONTINUE_RESPONSE);
            }
        }
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
                Err(ref e) if e.is_would_block() => break,
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
                Err(ref e) if e.is_would_block() => break,
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
    request_timeout: Duration,
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
            request_timeout: Duration::from_secs(REQUEST_TIMEOUT_SECS),
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
        let request_timeout = self.request_timeout;
        let (linger_quiet, linger_max) = (self.linger_quiet, self.linger_max);
        let mut to_remove = Vec::new();

        for (idx, conn) in self.conns.iter_mut().enumerate() {
            conn.flush_writes();

            if conn.last_activity.elapsed() > idle_timeout {
                to_remove.push(idx);
                continue;
            }

            // Slowloris guard: a request whose header section has not
            // completed within the header window, or that is still incomplete
            // after the request window, gets a 408 and the connection closes.
            let late = conn
                .request_started
                .map(|t| t.elapsed())
                .is_some_and(|age| {
                    age >= request_timeout || (age >= header_timeout && !conn.headers_complete())
                });
            if !conn.close_after_flush && late {
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
                            conn.vet_pending_head(token.as_deref());
                            break;
                        },
                        Framing::Error(code) => {
                            conn.read_buf.clear();
                            conn.queue_write(&build_response(code, false, &[], None, b""));
                            conn.close_after_flush = true;
                            break;
                        },
                        Framing::Ready(req) => {
                            conn.head_checked = false;
                            let (response, keep_alive) =
                                handle_http_request(&req, disp, token.as_deref());
                            conn.queue_write(&response);
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
                // Peer half-closed: finish once every complete request it sent
                // has been answered (backpressure may have paused the loop
                // with pipelined requests still buffered).
                let paused = conn.write_buf.len() >= MAX_WRITE_BUF && !conn.read_buf.is_empty();
                if conn.eof && !paused {
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

#[cfg(test)]
mod tests;
