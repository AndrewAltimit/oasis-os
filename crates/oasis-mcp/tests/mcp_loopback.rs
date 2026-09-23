//! Loopback end-to-end tests for the MCP server.
//!
//! A real `StdNetworkBackend` listens on an ephemeral `127.0.0.1` port; the
//! `McpServer` is polled on the test thread (it is not `Send`) while a client
//! thread talks raw HTTP/1.1 over a blocking `TcpStream` with timeouts.

#![allow(clippy::unwrap_used)]

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use oasis_mcp::{McpServer, PROTOCOL_VERSION, ToolDispatcher, ToolResult, ToolSpec};
use oasis_net::StdNetworkBackend;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Upper bound for any single test scenario (server-side poll loop).
const SCENARIO_DEADLINE: Duration = Duration::from_secs(10);
/// Client socket read/write timeout.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

struct Harness {
    server: McpServer,
    addr: SocketAddr,
}

fn start(token: Option<&str>) -> Harness {
    let mut backend = StdNetworkBackend::new();
    backend.listen_loopback(0).unwrap();
    let addr = backend.local_addr().unwrap();
    let server = McpServer::new(Box::new(backend), token.map(str::to_string));
    Harness { server, addr }
}

impl Harness {
    /// Run `client` on a thread while polling the server on this one.
    fn run<T: Send + 'static>(
        &mut self,
        disp: &mut dyn ToolDispatcher,
        client: impl FnOnce(SocketAddr) -> T + Send + 'static,
    ) -> T {
        let addr = self.addr;
        let handle = thread::spawn(move || client(addr));
        let deadline = Instant::now() + SCENARIO_DEADLINE;
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "scenario timed out");
            self.server.poll(disp);
            thread::sleep(Duration::from_millis(1));
        }
        match handle.join() {
            Ok(v) => v,
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    /// Poll until the server holds no connections (or fail after a deadline).
    fn drain_connections(&mut self, disp: &mut dyn ToolDispatcher) {
        let deadline = Instant::now() + SCENARIO_DEADLINE;
        while self.server.connection_count() > 0 {
            assert!(
                Instant::now() < deadline,
                "server kept {} connection(s)",
                self.server.connection_count()
            );
            self.server.poll(disp);
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Fake tool dispatcher: `echo` (text), `fail` (in-band error), `snap` (image).
#[derive(Default)]
struct FakeDispatcher {
    calls: Vec<String>,
}

impl ToolDispatcher for FakeDispatcher {
    fn list_tools(&self) -> Vec<ToolSpec> {
        vec![
            ToolSpec::new("echo", "Echo the arguments", json!({ "type": "object" })),
            ToolSpec::new("fail", "Always fails", json!({ "type": "object" })),
            ToolSpec::new("snap", "Returns an image", json!({ "type": "object" })),
        ]
    }

    fn call_tool(&mut self, name: &str, args: Value) -> ToolResult {
        self.calls.push(name.to_string());
        match name {
            "echo" => ToolResult::text(format!("echo:{args}")),
            "fail" => ToolResult::error("tool failed on purpose"),
            "snap" => ToolResult::image(oasis_mcp::base64_encode(b"\x89PNG"), "image/png"),
            other => ToolResult::error(format!("unknown tool: {other}")),
        }
    }
}

/// A parsed HTTP response.
#[derive(Debug)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "body is not JSON ({e}): {:?}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

/// Client side of one connection, buffering bytes across responses so
/// pipelined responses can be read one at a time.
struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Client {
    fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT).unwrap();
        stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        stream.set_nodelay(true).unwrap();
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    /// Read more bytes; returns 0 on EOF.
    fn fill(&mut self) -> std::io::Result<usize> {
        let mut chunk = [0u8; 8192];
        let n = self.stream.read(&mut chunk)?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// Read one response, including interim `1xx` responses.
    fn try_read_any(&mut self) -> std::io::Result<Response> {
        let hdr_end = loop {
            if let Some(i) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            if self.fill()? == 0 {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    format!(
                        "EOF before response headers ({:?})",
                        String::from_utf8_lossy(&self.buf)
                    ),
                ));
            }
        };
        let head = String::from_utf8_lossy(&self.buf[..hdr_end]).to_string();
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or_default();
        let status: u16 = status_line
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("bad status line {status_line:?}"));
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .map_or(0, |(_, v)| v.parse().unwrap());
        while self.buf.len() < hdr_end + len {
            if self.fill()? == 0 {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "EOF inside response body",
                ));
            }
        }
        let raw: Vec<u8> = self.buf.drain(..hdr_end + len).collect();
        Ok(Response {
            status,
            headers,
            body: raw[hdr_end..].to_vec(),
        })
    }

    /// Read one final (non-`1xx`) response.
    fn read_response(&mut self) -> Response {
        loop {
            let resp = self.try_read_any().unwrap();
            if !(100..200).contains(&resp.status) {
                return resp;
            }
        }
    }

    /// Whether the server closed the connection (EOF or reset) with no
    /// further bytes pending.
    fn at_eof(&mut self) -> bool {
        match self.fill() {
            Ok(0) => self.buf.is_empty(),
            Ok(_) => false,
            Err(e) => matches!(
                e.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
            ),
        }
    }

    fn roundtrip(&mut self, req: &[u8]) -> Response {
        self.send(req);
        self.read_response()
    }
}

fn http_post(body: &str, extra_headers: &str) -> Vec<u8> {
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n{extra_headers}\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn json_post(body: &str) -> Vec<u8> {
    http_post(body, "Content-Type: application/json\r\n")
}

fn rpc(id: u64, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

fn notification(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

fn one_shot(h: &mut Harness, disp: &mut FakeDispatcher, req: Vec<u8>) -> Response {
    h.run(disp, move |addr| Client::connect(addr).roundtrip(&req))
}

// ---------------------------------------------------------------------------
// MCP protocol flows
// ---------------------------------------------------------------------------

#[test]
fn full_session_over_one_keepalive_connection() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let responses = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let init = c.roundtrip(&json_post(&rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "e2e", "version": "0" },
            }),
        )));
        let initialized = c.roundtrip(&json_post(&notification(
            "notifications/initialized",
            json!({}),
        )));
        let ping = c.roundtrip(&json_post(&rpc(2, "ping", json!({}))));
        let list = c.roundtrip(&json_post(&rpc(3, "tools/list", json!({}))));
        let echo = c.roundtrip(&json_post(&rpc(
            4,
            "tools/call",
            json!({ "name": "echo", "arguments": { "msg": "hi" } }),
        )));
        let fail = c.roundtrip(&json_post(&rpc(
            5,
            "tools/call",
            json!({ "name": "fail", "arguments": {} }),
        )));
        let snap = c.roundtrip(&json_post(&rpc(6, "tools/call", json!({ "name": "snap" }))));
        let unknown = c.roundtrip(&json_post(&rpc(
            7,
            "tools/call",
            json!({ "name": "nope", "arguments": {} }),
        )));
        vec![init, initialized, ping, list, echo, fail, snap, unknown]
    });

    let [init, initialized, ping, list, echo, fail, snap, unknown]: [Response; 8] =
        responses.try_into().unwrap();

    assert_eq!(init.status, 200);
    assert_eq!(
        init.header("Content-Type").map(str::to_ascii_lowercase),
        Some("application/json".to_string())
    );
    assert_eq!(init.header("Connection"), Some("keep-alive"));
    let v = init.json();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(v["result"]["serverInfo"]["name"], "oasis-mcp");
    assert!(v["result"]["capabilities"]["tools"].is_object());

    assert_eq!(initialized.status, 202);
    assert!(initialized.body.is_empty());

    assert_eq!(ping.status, 200);
    assert_eq!(ping.json()["id"], 2);
    assert_eq!(ping.json()["result"], json!({}));

    let tools = list.json()["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["echo", "fail", "snap"]);
    assert_eq!(tools[0]["inputSchema"]["type"], "object");

    let v = echo.json();
    assert_eq!(v["id"], 4);
    assert_eq!(v["result"]["isError"], false);
    assert_eq!(v["result"]["content"][0]["type"], "text");
    assert_eq!(v["result"]["content"][0]["text"], r#"echo:{"msg":"hi"}"#);

    let v = fail.json();
    assert!(v["error"].is_null(), "tool failure must be in-band: {v}");
    assert_eq!(v["result"]["isError"], true);
    assert_eq!(v["result"]["content"][0]["text"], "tool failed on purpose");

    let v = snap.json();
    assert_eq!(v["result"]["content"][0]["type"], "image");
    assert_eq!(v["result"]["content"][0]["mimeType"], "image/png");
    assert_eq!(
        v["result"]["content"][0]["data"],
        oasis_mcp::base64_encode(b"\x89PNG")
    );

    assert_eq!(unknown.json()["result"]["isError"], true);
    assert_eq!(disp.calls, ["echo", "fail", "snap", "nope"]);
}

#[test]
fn jsonrpc_error_codes() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resps = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        [
            c.roundtrip(&json_post("{not json")),
            c.roundtrip(&json_post(&rpc(11, "frobnicate", json!({})))),
            c.roundtrip(&json_post(&rpc(
                12,
                "tools/call",
                json!({ "arguments": {} }),
            ))),
            c.roundtrip(&json_post(r#"{"jsonrpc":"2.0","id":13}"#)),
            c.roundtrip(&json_post(r#"{"jsonrpc":"1.0","id":14,"method":"ping"}"#)),
            c.roundtrip(&json_post(r#"[{"jsonrpc":"2.0","id":15,"method":"ping"}]"#)),
            c.roundtrip(&json_post(r#""just a string""#)),
            c.roundtrip(&json_post("")),
        ]
    });
    let codes: Vec<(Value, Value)> = resps
        .iter()
        .map(|r| {
            assert_eq!(r.status, 200, "JSON-RPC errors travel in a 200 body");
            let v = r.json();
            assert_eq!(v["jsonrpc"], "2.0");
            assert!(v["result"].is_null(), "{v}");
            (v["id"].clone(), v["error"]["code"].clone())
        })
        .collect();
    assert_eq!(codes[0], (Value::Null, json!(-32700)), "parse error");
    assert_eq!(codes[1], (json!(11), json!(-32601)), "unknown method");
    assert_eq!(codes[2], (json!(12), json!(-32602)), "invalid params");
    assert_eq!(codes[3], (json!(13), json!(-32600)), "missing method");
    assert_eq!(
        codes[4],
        (json!(14), json!(-32600)),
        "wrong jsonrpc version"
    );
    assert_eq!(
        codes[5],
        (Value::Null, json!(-32600)),
        "batch not supported"
    );
    assert_eq!(codes[6], (Value::Null, json!(-32600)), "non-object request");
    assert_eq!(codes[7], (Value::Null, json!(-32700)), "empty body");
    assert!(disp.calls.is_empty());
}

#[test]
fn notifications_get_202_and_no_body() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resps = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        [
            c.roundtrip(&json_post(&notification(
                "notifications/cancelled",
                json!({}),
            ))),
            c.roundtrip(&json_post(&notification("ping", json!({})))),
            c.roundtrip(&json_post(&notification("tools/list", json!({})))),
            c.roundtrip(&json_post(&notification(
                "tools/call",
                json!({ "name": "echo", "arguments": {} }),
            ))),
            // Explicit `"id": null` is not a notification in JSON-RPC 2.0,
            // but it is not a valid MCP request id either; it must still get
            // a reply rather than silently running a tool.
            c.roundtrip(&json_post(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#)),
        ]
    });
    for r in &resps[..4] {
        assert_eq!(r.status, 202, "{r:?}");
        assert!(r.body.is_empty(), "{r:?}");
    }
    assert_eq!(resps[4].status, 200);
    // A tools/call without an id is never executed: its result could not be
    // delivered, and a side-effecting call nobody can observe is unsafe.
    assert!(disp.calls.is_empty(), "{:?}", disp.calls);
}

// ---------------------------------------------------------------------------
// HTTP surface
// ---------------------------------------------------------------------------

#[test]
fn http_methods_and_paths() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resps = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        [
            c.roundtrip(b"GET /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n"),
            c.roundtrip(b"PUT /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}"),
            c.roundtrip(b"OPTIONS /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n"),
            c.roundtrip(b"DELETE /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n"),
            c.roundtrip(
                b"POST /other HTTP/1.1\r\nHost: localhost\r\n\
                  Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
            ),
            c.roundtrip(&json_post(&rpc(1, "ping", json!({})))),
        ]
    });
    assert_eq!(resps[0].status, 405);
    assert_eq!(resps[0].header("Allow"), Some("POST, DELETE"));
    assert_eq!(resps[1].status, 405);
    assert_eq!(resps[2].status, 204);
    assert!(
        !resps[2]
            .headers
            .iter()
            .any(|(k, _)| k.to_ascii_lowercase().starts_with("access-control-")),
        "OPTIONS must never approve a CORS preflight: {:?}",
        resps[2].headers
    );
    assert_eq!(resps[3].status, 200);
    assert_eq!(resps[4].status, 404);
    // The connection survived all of the above.
    assert_eq!(resps[5].status, 200);
}

#[test]
fn bearer_token_gate() {
    let mut h = start(Some("s3cret"));
    let mut disp = FakeDispatcher::default();
    let ping = rpc(1, "ping", json!({}));
    let resps = h.run(&mut disp, move |addr| {
        let mut c = Client::connect(addr);
        [
            c.roundtrip(&json_post(&ping)),
            c.roundtrip(&http_post(
                &ping,
                "Content-Type: application/json\r\nAuthorization: Bearer wrong\r\n",
            )),
            c.roundtrip(&http_post(
                &ping,
                "Content-Type: application/json\r\nAuthorization: Bearer s3cret-and-more\r\n",
            )),
            c.roundtrip(&http_post(
                &ping,
                "Content-Type: application/json\r\nAuthorization: Basic s3cret\r\n",
            )),
            c.roundtrip(&http_post(
                &ping,
                "Content-Type: application/json\r\nAuthorization: Bearer s3cret\r\n",
            )),
        ]
    });
    for r in &resps[..4] {
        assert_eq!(r.status, 401, "{r:?}");
        assert_eq!(r.header("WWW-Authenticate"), Some("Bearer"));
    }
    assert_eq!(resps[4].status, 200);
    assert_eq!(resps[4].json()["id"], 1);
}

#[test]
fn browser_attack_defences() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let call = rpc(1, "tools/call", json!({ "name": "echo", "arguments": {} }));
    let reqs: Vec<(Vec<u8>, u16)> = vec![
        (
            http_post(
                &call,
                "Origin: https://evil.example\r\nContent-Type: application/json\r\n",
            ),
            403,
        ),
        (
            http_post(&call, "Origin: null\r\nContent-Type: application/json\r\n"),
            403,
        ),
        (
            format!(
                "POST /mcp HTTP/1.1\r\nHost: rebind.evil.example:7345\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{call}",
                call.len()
            )
            .into_bytes(),
            403,
        ),
        (http_post(&call, "Content-Type: text/plain\r\n"), 415),
        (http_post(&call, ""), 415),
        (
            http_post(
                &call,
                "Origin: http://127.0.0.1:9999\r\nContent-Type: application/json\r\n",
            ),
            200,
        ),
    ];
    for (req, want) in reqs {
        let resp = one_shot(&mut h, &mut disp, req);
        assert_eq!(resp.status, want, "{resp:?}");
    }
    // Only the final, legitimate request reached the dispatcher.
    assert_eq!(disp.calls, ["echo"]);
}

#[test]
fn connection_close_and_http10_close_after_response() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let (closed, http10_closed, r1, r2) = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let r1 = c.roundtrip(&http_post(
            &rpc(1, "ping", json!({})),
            "Content-Type: application/json\r\nConnection: close\r\n",
        ));
        let closed = c.at_eof();

        let mut c = Client::connect(addr);
        let body = rpc(2, "ping", json!({}));
        let req = format!(
            "POST /mcp HTTP/1.0\r\nHost: localhost\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let r2 = c.roundtrip(req.as_bytes());
        (closed, c.at_eof(), r1, r2)
    });
    assert_eq!(r1.status, 200);
    assert_eq!(r1.header("Connection"), Some("close"));
    assert!(
        closed,
        "server kept the connection open after Connection: close"
    );
    assert_eq!(r2.status, 200);
    assert!(http10_closed, "server kept an HTTP/1.0 connection open");
    h.drain_connections(&mut disp);
}

// ---------------------------------------------------------------------------
// Framing over a real non-blocking socket
// ---------------------------------------------------------------------------

#[test]
fn pipelined_requests_in_one_write() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let ids = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let mut burst = Vec::new();
        for id in 1..=5 {
            burst.extend_from_slice(&json_post(&rpc(id, "ping", json!({}))));
        }
        // A notification in the middle yields a 202 in its slot.
        burst.extend_from_slice(&json_post(&notification(
            "notifications/initialized",
            json!({}),
        )));
        burst.extend_from_slice(&json_post(&rpc(6, "tools/list", json!({}))));
        c.send(&burst);
        (0..7)
            .map(|_| {
                let r = c.read_response();
                if r.status == 202 {
                    Value::String("202".into())
                } else {
                    r.json()["id"].clone()
                }
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        ids,
        [
            json!(1),
            json!(2),
            json!(3),
            json!(4),
            json!(5),
            json!("202"),
            json!(6)
        ]
    );
}

#[test]
fn request_dribbled_byte_by_byte() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resp = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let req = json_post(&rpc(
            42,
            "tools/call",
            json!({ "name": "echo", "arguments": { "k": "v" } }),
        ));
        for b in req {
            c.send(&[b]);
            thread::sleep(Duration::from_micros(300));
        }
        c.read_response()
    });
    assert_eq!(resp.status, 200);
    let v = resp.json();
    assert_eq!(v["id"], 42);
    assert_eq!(v["result"]["content"][0]["text"], r#"echo:{"k":"v"}"#);
}

#[test]
fn large_valid_body_spanning_many_polls() {
    // ~900 KiB (under the 1 MiB cap): arrives over many reads/polls.
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resp = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let pad = "x".repeat(900 * 1024);
        c.roundtrip(&json_post(&rpc(
            1,
            "tools/call",
            json!({ "name": "echo", "arguments": { "pad": pad } }),
        )))
    });
    assert_eq!(resp.status, 200);
    let text = resp.json()["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .len();
    assert!(text > 900 * 1024, "echo lost data: {text}");
}

#[test]
fn oversized_declared_body_gets_413() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resp = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        c.send(
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
              Content-Length: 2000000\r\n\r\n",
        );
        let r = c.read_response();
        (r, c.at_eof())
    });
    assert_eq!(resp.0.status, 413);
    assert!(resp.1, "connection must close after 413");
    h.drain_connections(&mut disp);
}

#[test]
fn oversized_streamed_body_gets_413_not_reset() {
    // The client really streams a 2 MiB body. The server must reply 413 and
    // the client must be able to read it (closing a socket with unread input
    // makes the OS send RST, which can destroy the response in flight).
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resp = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let total = 2 * 1024 * 1024;
        c.send(
            format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\n\
                 Content-Type: application/json\r\nContent-Length: {total}\r\n\r\n"
            )
            .as_bytes(),
        );
        let chunk = vec![b' '; 64 * 1024];
        let mut sent = 0;
        while sent < total {
            if c.stream.write_all(&chunk).is_err() {
                break;
            }
            sent += chunk.len();
        }
        c.try_read_any().map(|r| r.status)
    });
    assert_eq!(resp.unwrap(), 413);
    h.drain_connections(&mut disp);
}

#[test]
fn oversized_headers_get_431() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let resp = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let mut req = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\n".to_vec();
        for i in 0..400 {
            req.extend_from_slice(format!("X-Filler-{i}: {}\r\n", "a".repeat(60)).as_bytes());
        }
        // Never terminated with CRLFCRLF.
        c.send(&req);
        c.try_read_any().map(|r| r.status)
    });
    assert_eq!(resp.unwrap(), 431);
    h.drain_connections(&mut disp);
}

#[test]
fn garbage_bytes_are_rejected_without_panicking() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let garbage: Vec<Vec<u8>> = vec![
        b"\x00\xff\xfe\xfd\r\n\r\n".to_vec(),
        b"GARBAGE\r\n\r\n".to_vec(),
        b"\r\n\r\n".to_vec(),
        b"POST /mcp HTTP/1.1\r\nContent-Length: -5\r\n\r\n".to_vec(),
        b"POST /mcp HTTP/1.1\r\nContent-Length: 99999999999999999999999\r\n\r\n".to_vec(),
        b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec(),
    ];
    let statuses = h.run(&mut disp, move |addr| {
        garbage
            .into_iter()
            .map(|g| {
                let mut c = Client::connect(addr);
                c.send(&g);
                let status = c.try_read_any().map(|r| r.status).ok();
                (status, c.at_eof())
            })
            .collect::<Vec<_>>()
    });
    let expected = [400, 400, 400, 400, 413, 411];
    for ((status, closed), want) in statuses.iter().zip(expected) {
        assert_eq!(*status, Some(want), "{statuses:?}");
        assert!(closed, "connection left open after a {want}");
    }
    h.drain_connections(&mut disp);
    // Still serving.
    let resp = one_shot(&mut h, &mut disp, json_post(&rpc(1, "ping", json!({}))));
    assert_eq!(resp.status, 200);
}

#[test]
fn expect_100_continue_is_honoured() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let (interim, fin) = h.run(&mut disp, |addr| {
        let mut c = Client::connect(addr);
        let body = rpc(1, "ping", json!({}));
        c.send(
            format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
                 Expect: 100-continue\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        );
        // A client waits for the interim response before sending the body.
        c.stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let interim = c.try_read_any().map(|r| r.status).ok();
        c.stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        c.send(body.as_bytes());
        (interim, c.read_response())
    });
    assert_eq!(interim, Some(100));
    assert_eq!(fin.status, 200);
    assert_eq!(fin.json()["id"], 1);
}

// ---------------------------------------------------------------------------
// Connection lifecycle
// ---------------------------------------------------------------------------

#[test]
fn abrupt_disconnects_free_their_slots() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    h.run(&mut disp, |addr| {
        // Mid-headers.
        let mut c = Client::connect(addr);
        c.send(b"POST /mcp HTTP/1.1\r\nHost: loc");
        thread::sleep(Duration::from_millis(30));
        drop(c);
        // Mid-body.
        let mut c = Client::connect(addr);
        c.send(
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
              Content-Length: 100\r\n\r\n{\"jsonrpc\"",
        );
        thread::sleep(Duration::from_millis(30));
        drop(c);
        // Connect and leave without sending anything.
        let c = Client::connect(addr);
        thread::sleep(Duration::from_millis(30));
        drop(c);
        // Send a request and disconnect before reading the reply.
        let mut c = Client::connect(addr);
        c.send(&json_post(&rpc(1, "ping", json!({}))));
        drop(c);
        thread::sleep(Duration::from_millis(30));
    });
    h.drain_connections(&mut disp);
    // The half-sent requests never reached the dispatcher.
    assert!(disp.calls.is_empty());
    let resp = one_shot(&mut h, &mut disp, json_post(&rpc(9, "ping", json!({}))));
    assert_eq!(resp.status, 200);
}

#[test]
fn slot_limit_queues_extra_clients_until_one_frees() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let (fifth_blocked, fifth) = h.run(&mut disp, |addr| {
        let mut held: Vec<Client> = (0..4).map(|_| Client::connect(addr)).collect();
        for (i, c) in held.iter_mut().enumerate() {
            let r = c.roundtrip(&json_post(&rpc(i as u64, "ping", json!({}))));
            assert_eq!(r.status, 200);
        }
        // Fifth client: the TCP handshake completes (kernel backlog) but the
        // server has no free slot, so it is not served yet.
        let mut fifth = Client::connect(addr);
        fifth.send(&json_post(&rpc(5, "ping", json!({}))));
        fifth
            .stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let blocked = matches!(
            fifth.fill(),
            Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
        );
        fifth.stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        // Free a slot: the queued client is then accepted and answered.
        drop(held.remove(0));
        let r = fifth.read_response();
        (blocked, r)
    });
    assert!(
        fifth_blocked,
        "fifth client was served beyond the slot limit"
    );
    assert_eq!(fifth.status, 200);
    assert_eq!(fifth.json()["id"], 5);
}

#[test]
fn many_sequential_clients() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let n = h.run(&mut disp, |addr| {
        let mut ok = 0;
        for i in 0..40u64 {
            let mut c = Client::connect(addr);
            let r = c.roundtrip(&http_post(
                &rpc(i, "tools/call", json!({ "name": "echo", "arguments": i })),
                "Content-Type: application/json\r\nConnection: close\r\n",
            ));
            assert_eq!(r.json()["id"], i);
            ok += 1;
        }
        ok
    });
    assert_eq!(n, 40);
    assert_eq!(disp.calls.len(), 40);
    h.drain_connections(&mut disp);
}

#[test]
fn stop_closes_all_connections() {
    let mut h = start(None);
    let mut disp = FakeDispatcher::default();
    let clients = h.run(&mut disp, |addr| {
        let mut cs: Vec<Client> = (0..3).map(|_| Client::connect(addr)).collect();
        for c in &mut cs {
            assert_eq!(
                c.roundtrip(&json_post(&rpc(1, "ping", json!({})))).status,
                200
            );
        }
        cs
    });
    assert_eq!(h.server.connection_count(), 3);
    h.server.stop();
    assert_eq!(h.server.connection_count(), 0);
    for mut c in clients {
        assert!(c.at_eof());
    }
}
