//! The pollable MCP server object.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::OasisError;

use crate::dispatch::{Handled, handle_message};
use crate::http::{Framing, HttpRequest, build_response, try_frame};
use crate::tools::ToolDispatcher;

const DEFAULT_MAX_CONNECTIONS: usize = 4;
const IDLE_TIMEOUT_SECS: u64 = 300;
const READ_CHUNK: usize = 4096;

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

struct HttpConn {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    write_buf: Vec<u8>,
    last_activity: Instant,
    close_after_flush: bool,
}

impl HttpConn {
    fn new(stream: Box<dyn NetworkStream>) -> Self {
        Self {
            stream,
            read_buf: Vec::with_capacity(READ_CHUNK),
            write_buf: Vec::new(),
            last_activity: Instant::now(),
            close_after_flush: false,
        }
    }

    fn queue_write(&mut self, bytes: &[u8]) {
        self.write_buf.extend_from_slice(bytes);
    }

    /// Write as much of `write_buf` as the non-blocking socket will accept.
    fn flush_writes(&mut self) {
        if self.write_buf.is_empty() {
            return;
        }
        match self.stream.write(&self.write_buf) {
            Ok(0) => {},
            Ok(n) => {
                self.write_buf.drain(..n);
            },
            Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {},
            Err(e) => {
                log::debug!("mcp write error: {e}");
                self.write_buf.clear();
                self.close_after_flush = true;
            },
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
        let mut to_remove = Vec::new();

        for (idx, conn) in self.conns.iter_mut().enumerate() {
            conn.flush_writes();

            if conn.last_activity.elapsed() > idle_timeout {
                to_remove.push(idx);
                continue;
            }

            let mut buf = [0u8; READ_CHUNK];
            match conn.stream.read(&mut buf) {
                Ok(0) => {
                    if conn.write_buf.is_empty() {
                        to_remove.push(idx);
                    }
                },
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {},
                Ok(n) => {
                    conn.last_activity = Instant::now();
                    conn.read_buf.extend_from_slice(&buf[..n]);

                    loop {
                        match try_frame(&mut conn.read_buf) {
                            Framing::Pending => break,
                            Framing::Error(code) => {
                                conn.queue_write(&build_response(code, false, &[], None, b""));
                                conn.close_after_flush = true;
                                break;
                            },
                            Framing::Ready(req) => {
                                let keep_alive = req.keep_alive;
                                conn.queue_write(&handle_http_request(
                                    &req,
                                    disp,
                                    token.as_deref(),
                                ));
                                if !keep_alive {
                                    conn.close_after_flush = true;
                                    break;
                                }
                            },
                        }
                    }
                    conn.flush_writes();
                },
                Err(e) => {
                    log::debug!("mcp read error: {e}");
                    to_remove.push(idx);
                },
            }

            if conn.close_after_flush && conn.write_buf.is_empty() {
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

fn handle_http_request(
    req: &HttpRequest,
    disp: &mut dyn ToolDispatcher,
    token: Option<&str>,
) -> Vec<u8> {
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
        closed: bool,
    }

    type SharedPipe = Arc<Mutex<Pipe>>;

    struct MockStream {
        pipe: SharedPipe,
    }

    fn lock(pipe: &SharedPipe) -> std::sync::MutexGuard<'_, Pipe> {
        pipe.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl NetworkStream for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let mut p = lock(&self.pipe);
            if p.client_to_server.is_empty() {
                return Err(OasisError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "would block",
                )));
            }
            let n = buf.len().min(p.client_to_server.len());
            let drained: Vec<u8> = p.client_to_server.drain(..n).collect();
            buf[..n].copy_from_slice(&drained);
            Ok(n)
        }
        fn write(&mut self, data: &[u8]) -> Result<usize> {
            lock(&self.pipe).server_to_client.extend_from_slice(data);
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

    fn last_http_response(pipe: &SharedPipe) -> String {
        String::from_utf8_lossy(&lock(pipe).server_to_client).to_string()
    }

    #[test]
    fn end_to_end_initialize_over_socket() {
        let pipe: SharedPipe = Arc::new(Mutex::new(Pipe::default()));
        lock(&pipe).client_to_server.extend_from_slice(
            b"POST /mcp HTTP/1.1\r\nContent-Length: 58\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}",
        );
        let backend = MockBackend {
            pending: Some(Arc::clone(&pipe)),
        };
        let mut server = McpServer::new(Box::new(backend), None);
        let mut disp = StubDispatcher;
        // First poll accepts; second poll reads + dispatches.
        server.poll(&mut disp);
        server.poll(&mut disp);
        let resp = last_http_response(&pipe);
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"serverInfo\""));
    }

    #[test]
    fn get_returns_405() {
        let pipe: SharedPipe = Arc::new(Mutex::new(Pipe::default()));
        lock(&pipe)
            .client_to_server
            .extend_from_slice(b"GET /mcp HTTP/1.1\r\n\r\n");
        let backend = MockBackend {
            pending: Some(Arc::clone(&pipe)),
        };
        let mut server = McpServer::new(Box::new(backend), None);
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        let resp = last_http_response(&pipe);
        assert!(resp.starts_with("HTTP/1.1 405"));
        assert!(resp.contains("Allow: POST, DELETE"));
    }

    #[test]
    fn missing_token_returns_401() {
        let pipe: SharedPipe = Arc::new(Mutex::new(Pipe::default()));
        lock(&pipe).client_to_server.extend_from_slice(
            b"POST /mcp HTTP/1.1\r\nContent-Length: 45\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}     ",
        );
        let backend = MockBackend {
            pending: Some(Arc::clone(&pipe)),
        };
        let mut server = McpServer::new(Box::new(backend), Some("sekret".to_string()));
        let mut disp = StubDispatcher;
        server.poll(&mut disp);
        server.poll(&mut disp);
        let resp = last_http_response(&pipe);
        assert!(resp.starts_with("HTTP/1.1 401"));
    }
}
