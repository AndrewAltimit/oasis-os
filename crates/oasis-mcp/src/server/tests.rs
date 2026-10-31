use super::*;
use crate::gate::is_loopback_authority;
use crate::tools::{ToolResult, ToolSpec};
use oasis_types::error::{OasisError, Result};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Shared byte pipes for a single mock connection.
#[derive(Default)]
struct Pipe {
    client_to_server: Vec<u8>,
    server_to_client: Vec<u8>,
    /// When set, writes fail with `WouldBlock` (peer not draining).
    write_blocked: bool,
    /// When set, reads fail with this kind once the input is drained.
    read_error: Option<std::io::ErrorKind>,
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
            return Err(match p.read_error {
                Some(kind) => OasisError::Io(std::io::Error::from(kind)),
                None => would_block(),
            });
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
    assert!(resp.contains("Allow: POST, OPTIONS"));
    // DELETE: no sessions are issued, so there is nothing to delete.
    let resp = roundtrip(b"DELETE /mcp HTTP/1.1\r\n\r\n");
    assert!(resp.starts_with("HTTP/1.1 405"), "{resp}");
    // OPTIONS advertises the same set.
    let resp = roundtrip(b"OPTIONS /mcp HTTP/1.1\r\n\r\n");
    assert!(resp.contains("Allow: POST, OPTIONS"), "{resp}");
}

#[test]
fn unauthorized_body_is_refused_before_it_is_buffered() {
    // Headers announce a large body and ask for `100 Continue`: without a
    // token the server answers 401 at once instead of inviting the body.
    let head = "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
                Expect: 100-continue\r\nContent-Length: 900000\r\n\r\n";
    let (mut server, pipe) = server_with(head.as_bytes(), Some("sekret"));
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    server.poll(&mut disp);
    let resp = last_http_response(&pipe);
    assert!(resp.starts_with("HTTP/1.1 401"), "{resp}");
    assert!(!resp.contains("100 Continue"), "{resp}");
    // With the token, the same headers get the interim response.
    let authed = head.replace("Expect:", "Authorization: Bearer sekret\r\nExpect:");
    let (mut server, pipe) = server_with(authed.as_bytes(), Some("sekret"));
    server.poll(&mut disp);
    server.poll(&mut disp);
    assert!(last_http_response(&pipe).starts_with("HTTP/1.1 100 Continue"));
}

#[test]
fn trickled_body_times_out_with_408() {
    // Headers complete, body never finishes: the request deadline (not
    // just the header deadline) closes the connection.
    let head = "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
                Content-Length: 100\r\n\r\n{";
    let (mut server, pipe) = server_with(head.as_bytes(), None);
    server.linger_quiet = Duration::ZERO;
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    server.poll(&mut disp);
    assert_eq!(server.connection_count(), 1);
    server.request_timeout = Duration::ZERO;
    server.poll(&mut disp);
    assert!(last_http_response(&pipe).starts_with("HTTP/1.1 408"));
    assert_eq!(server.connection_count(), 0);
}

#[test]
fn early_refusal_closes_the_connection() {
    // Refused on its headers alone, with the body still unread: the
    // server cannot resync, so it closes instead of keeping alive.
    let head = "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\
                Content-Length: 500\r\n\r\n";
    let (mut server, pipe) = server_with(head.as_bytes(), None);
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    server.poll(&mut disp);
    let resp = last_http_response(&pipe);
    assert!(resp.starts_with("HTTP/1.1 415"), "{resp}");
    assert!(resp.contains("Connection: close"), "{resp}");
}

#[test]
fn half_close_with_backlog_still_answers_every_request() {
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"big"}}"#;
    let mut input = Vec::new();
    for _ in 0..8 {
        input.extend_from_slice(&json_post(call));
    }
    let (mut server, pipe) = server_with(&input, None);
    lock(&pipe).write_blocked = true;
    lock(&pipe).read_error = None;
    let mut disp = BigDispatcher;
    server.poll(&mut disp);
    server.poll(&mut disp);
    // The peer shuts down its sending side while responses are backed up.
    server.conns[0].eof = true;
    lock(&pipe).write_blocked = false;
    for _ in 0..20 {
        server.poll(&mut disp);
    }
    let resp = last_http_response(&pipe);
    assert_eq!(resp.matches("HTTP/1.1 200 OK").count(), 8);
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
fn wrong_token_closes_the_connection() {
    // Regression: a 401 kept the connection alive, so one client could
    // pipeline a burst of token guesses into a single poll.
    let mut input = Vec::new();
    for guess in ["a", "b", "c", "sekret"] {
        input.extend_from_slice(&post(
            PING_BODY,
            &format!("Content-Type: application/json\r\nAuthorization: Bearer {guess}\r\n"),
        ));
    }
    let (mut server, pipe) = server_with(&input, Some("sekret"));
    let mut disp = StubDispatcher;
    for _ in 0..5 {
        server.poll(&mut disp);
    }
    let out = String::from_utf8_lossy(&lock(&pipe).server_to_client).into_owned();
    assert_eq!(out.matches("HTTP/1.1 401").count(), 1, "{out}");
    assert!(
        !out.contains("HTTP/1.1 200"),
        "later guesses must not run: {out}"
    );
    assert!(out.contains("Connection: close"), "{out}");
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
fn timed_out_read_drops_the_connection() {
    // On a non-blocking socket ETIMEDOUT is the kernel declaring the
    // peer dead; it must not be retried like WouldBlock until the
    // (5 minute) idle timeout.
    let (mut server, pipe) = server_with(b"", None);
    lock(&pipe).read_error = Some(std::io::ErrorKind::TimedOut);
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    assert_eq!(server.connection_count(), 0);
}

#[test]
fn would_block_read_keeps_the_connection() {
    let (mut server, _pipe) = server_with(b"", None);
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    server.poll(&mut disp);
    assert_eq!(server.connection_count(), 1);
}

#[test]
fn silent_peer_is_dropped_after_idle_timeout() {
    // A client that connects and then never sends anything (and never
    // closes) only ever yields WouldBlock: the idle timeout reaps it.
    let (mut server, pipe) = server_with(b"", None);
    let mut disp = StubDispatcher;
    server.poll(&mut disp);
    assert_eq!(server.connection_count(), 1);
    server.idle_timeout = Duration::ZERO;
    std::thread::sleep(Duration::from_millis(2));
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
