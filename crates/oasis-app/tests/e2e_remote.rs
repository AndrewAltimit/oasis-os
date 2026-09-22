#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! End-to-end remote-control scenarios through the real desktop shell:
//! the remote terminal (`listen`), the FTP server (`ftp start`) and the
//! optional MCP control server (`mcp-server start`, feature `mcp`).
//!
//! The servers are started from the shell's own terminal and serviced by
//! the shell's per-frame polling (`Shell::step`), exactly as in the desktop
//! binary. Clients are real loopback `TcpStream`s driven from the test
//! thread: every wait steps the harness (one frame) between non-blocking
//! reads, with a wall-clock deadline.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use oasis_app::Mode;
use oasis_app::harness::Harness;
use oasis_core::input::Button;
use oasis_core::vfs::Vfs;

const DEADLINE: Duration = Duration::from_secs(15);

/// A currently free loopback port. The terminal commands treat port 0 as
/// "stop", so the tests cannot ask the OS for an ephemeral port directly.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Terminal output lines of the shell.
fn output(h: &Harness) -> Vec<String> {
    h.state().terminal.output_lines.clone()
}

fn output_contains(h: &Harness, needle: &str) -> bool {
    output(h).iter().any(|l| l.contains(needle))
}

/// Type `cmd` into the shell terminal and assert a line containing
/// `expect` was printed.
fn run(h: &mut Harness, cmd: &str, expect: &str) {
    h.terminal(cmd);
    assert!(
        output_contains(h, expect),
        "`{cmd}`: expected output containing {expect:?}, got {:#?}",
        output(h)
    );
}

/// A non-blocking loopback client whose reads step the harness.
struct Peer {
    s: TcpStream,
    buf: Vec<u8>,
    eof: bool,
}

impl Peer {
    fn connect(port: u16) -> Self {
        let s = TcpStream::connect(("127.0.0.1", port))
            .unwrap_or_else(|e| panic!("connect 127.0.0.1:{port}: {e}"));
        s.set_nonblocking(true).unwrap();
        s.set_nodelay(true).unwrap();
        Self {
            s,
            buf: Vec::new(),
            eof: false,
        }
    }

    fn send(&mut self, data: &str) {
        let mut rest = data.as_bytes();
        let start = Instant::now();
        while !rest.is_empty() {
            assert!(start.elapsed() < DEADLINE, "send timed out");
            match self.s.write(rest) {
                Ok(n) => rest = &rest[n..],
                Err(e) if e.kind() == ErrorKind::WouldBlock => {},
                Err(e) => panic!("send: {e}"),
            }
        }
    }

    /// Read whatever is available without blocking.
    fn read_available(&mut self) {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match self.s.read(&mut chunk) {
                Ok(0) => {
                    self.eof = true;
                    return;
                },
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == ErrorKind::Interrupted => {},
                Err(_) => {
                    self.eof = true;
                    return;
                },
            }
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }

    /// Step the shell until the received data contains `needle`; return
    /// everything up to and including it (and consume it).
    fn until(&mut self, h: &mut Harness, needle: &str) -> String {
        let start = Instant::now();
        loop {
            self.read_available();
            if let Some(pos) = self.text().find(needle) {
                let end = pos + needle.len();
                let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                self.buf.drain(..end);
                return head;
            }
            assert!(
                !self.eof && start.elapsed() < DEADLINE,
                "waiting for {needle:?}: eof={} got {:?}",
                self.eof,
                self.text()
            );
            h.step(&[]);
        }
    }

    /// Step the shell until the peer closes the connection; return all
    /// remaining data.
    fn until_eof(&mut self, h: &mut Harness) -> String {
        let start = Instant::now();
        while !self.eof {
            assert!(start.elapsed() < DEADLINE, "no EOF; got {:?}", self.text());
            h.step(&[]);
            self.read_available();
        }
        let out = self.text();
        self.buf.clear();
        out
    }
}

/// Remote-terminal client: connect and wait for the no-PSK banner.
fn remote_client(h: &mut Harness, port: u16) -> Peer {
    let mut p = Peer::connect(port);
    let banner = p.until(h, "> ");
    assert!(banner.contains("OASIS_OS remote terminal"), "{banner:?}");
    p
}

/// Run `cmd` on a remote-terminal connection; return its response text
/// (without the trailing prompt).
fn remote_cmd(h: &mut Harness, p: &mut Peer, cmd: &str) -> String {
    p.send(&format!("{cmd}\n"));
    let resp = p.until(h, "\n> ");
    resp.trim_end_matches("\n> ").to_string()
}

/// FTP client: connect and wait for the `220` greeting.
fn ftp_client(h: &mut Harness, port: u16) -> Peer {
    let mut p = Peer::connect(port);
    let greeting = p.until(h, "\n");
    assert!(greeting.starts_with("220 "), "{greeting:?}");
    p
}

/// Send one FTP request line and return the status line of the reply.
fn ftp_cmd(h: &mut Harness, p: &mut Peer, cmd: &str) -> String {
    p.send(&format!("{cmd}\n"));
    p.until(h, "\n").trim_end().to_string()
}

/// Whether a TCP connection to `port` is refused (nothing listening).
fn refused(port: u16) -> bool {
    match TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(5),
    ) {
        Ok(_) => false,
        Err(e) => {
            assert_ne!(e.kind(), ErrorKind::TimedOut, "connect to {port} timed out");
            true
        },
    }
}

// ---------------------------------------------------------------------------
// Remote terminal + FTP side by side
// ---------------------------------------------------------------------------

/// `listen` and `ftp start` used to share one network backend, which holds a
/// single listening socket: starting FTP replaced the remote terminal's
/// socket, so remote-terminal clients were refused (or, worse, a new remote
/// client landed on the FTP port's accept loop) and stopping either server
/// never released its port.
#[test]
fn remote_terminal_and_ftp_run_side_by_side() {
    let mut h = Harness::new("classic");
    h.settle();
    let term_port = free_port();
    let ftp_port = free_port();
    assert_ne!(term_port, ftp_port);

    run(
        &mut h,
        &format!("listen {term_port}"),
        &format!("Listening on 127.0.0.1:{term_port}"),
    );
    run(
        &mut h,
        &format!("ftp start {ftp_port}"),
        &format!("FTP server listening on port {ftp_port}"),
    );
    assert_eq!(
        h.state()
            .net
            .listener_backend
            .local_addr()
            .map(|a| a.port()),
        Some(term_port)
    );
    assert_eq!(
        h.state().net.ftp_backend.local_addr().map(|a| a.port()),
        Some(ftp_port)
    );

    // Both servers answer on their own port.
    let mut term = remote_client(&mut h, term_port);
    assert_eq!(
        remote_cmd(&mut h, &mut term, "echo side-by-side"),
        "side-by-side"
    );
    let mut ftp = ftp_client(&mut h, ftp_port);
    assert!(ftp_cmd(&mut h, &mut ftp, "PUT /tmp/ftp.txt uploaded").starts_with("200"));
    assert_eq!(h.vfs().read("/tmp/ftp.txt").unwrap(), b"uploaded");

    // The remote terminal sees the file the FTP client uploaded (one VFS).
    assert_eq!(
        remote_cmd(&mut h, &mut term, "cat /tmp/ftp.txt"),
        "uploaded"
    );

    // `ftp status` reflects the running server.
    run(&mut h, "ftp status", &format!("{ftp_port}"));

    // Stop FTP: its port is released, the remote terminal keeps working
    // (existing and new connections).
    run(&mut h, "ftp stop", "FTP server stopped.");
    assert!(h.state().net.ftp_server.is_none());
    ftp.until_eof(&mut h);
    assert!(
        refused(ftp_port),
        "FTP port still accepting after `ftp stop`"
    );
    assert_eq!(
        remote_cmd(&mut h, &mut term, "echo still-here"),
        "still-here"
    );
    let mut term2 = remote_client(&mut h, term_port);
    assert_eq!(remote_cmd(&mut h, &mut term2, "echo second"), "second");
    run(&mut h, "ftp status", "inactive");

    // FTP can be restarted on the same port.
    run(
        &mut h,
        &format!("ftp start {ftp_port}"),
        &format!("FTP server listening on port {ftp_port}"),
    );
    let mut ftp = ftp_client(&mut h, ftp_port);

    // Stop the remote terminal: FTP keeps working, the terminal port is
    // released and can be reused.
    run(&mut h, "listen stop", "Remote listener stopped.");
    term.until_eof(&mut h);
    term2.until_eof(&mut h);
    assert!(
        refused(term_port),
        "terminal port still accepting after `listen stop`"
    );
    let get = ftp_cmd(&mut h, &mut ftp, "GET /tmp/ftp.txt");
    assert!(get.starts_with("200 8 bytes"), "{get:?}");
    assert_eq!(ftp.until(&mut h, "uploaded"), "uploaded");
    run(
        &mut h,
        &format!("listen {term_port}"),
        &format!("Listening on 127.0.0.1:{term_port}"),
    );
    let mut term = remote_client(&mut h, term_port);
    assert_eq!(remote_cmd(&mut h, &mut term, "echo back"), "back");
    run(&mut h, "ftp stop", "FTP server stopped.");
    run(&mut h, "listen stop", "Remote listener stopped.");
}

// ---------------------------------------------------------------------------
// Remote terminal driving the shell
// ---------------------------------------------------------------------------

/// A PSK-protected listener (as configured by an embedder) authenticates
/// before running commands, and a wrong key is turned away.
#[test]
fn remote_terminal_psk_authentication() {
    use oasis_core::net::{ListenerConfig, RemoteListener};

    let mut h = Harness::new("classic");
    h.settle();
    let port = free_port();
    {
        let net = &mut h.state_mut().net;
        let mut l = RemoteListener::new(ListenerConfig {
            port,
            psk: "s3cret".into(),
            ..ListenerConfig::default()
        });
        l.start(&mut net.listener_backend).unwrap();
        net.listener = Some(l);
    }

    let mut bad = Peer::connect(port);
    bad.until(&mut h, "AUTH_REQUIRED\n");
    bad.send("guess\n");
    let rest = bad.until_eof(&mut h);
    assert!(rest.contains("AUTH_FAIL"), "{rest:?}");

    let mut good = Peer::connect(port);
    good.until(&mut h, "AUTH_REQUIRED\n");
    good.send("s3cret\n");
    good.until(&mut h, "AUTH_OK\n> ");
    assert_eq!(remote_cmd(&mut h, &mut good, "echo authed"), "authed");
}

/// Commands run over the remote terminal change the live shell: the VFS,
/// the skin (a full swap, identical to typing `skin` locally), and the
/// windows of the window manager.
#[test]
fn remote_terminal_commands_change_shell_state() {
    let mut h = Harness::new("classic");
    h.settle();
    let port = free_port();
    run(
        &mut h,
        &format!("listen {port}"),
        &format!("Listening on 127.0.0.1:{port}"),
    );
    // Back to the dashboard; the listener keeps running.
    h.button(Button::Start);
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
    let mut p = remote_client(&mut h, port);

    // VFS write through the remote shell lands in the shell's VFS.
    let resp = remote_cmd(&mut h, &mut p, "write /tmp/remote.txt hello from afar");
    assert!(!resp.starts_with("error"), "{resp:?}");
    assert_eq!(h.vfs().read("/tmp/remote.txt").unwrap(), b"hello from afar");
    // Working directory persists across commands on the connection.
    remote_cmd(&mut h, &mut p, "cd /tmp");
    assert_eq!(remote_cmd(&mut h, &mut p, "pwd"), "/tmp");

    // Skin swap over the remote terminal.
    let resp = remote_cmd(&mut h, &mut p, "skin xp");
    assert!(resp.contains("Switched to skin"), "{resp:?}");
    h.settle();
    h.render_now();

    // Reference: the same swap typed into the local terminal.
    let mut local = Harness::new("classic");
    local.settle();
    local.terminal("skin xp");
    local.button(Button::Start);
    local.settle();
    local.render_now();

    let (remote_state, local_state) = (h.state(), local.state());
    assert_eq!(
        remote_state.skin.manifest.name,
        local_state.skin.manifest.name
    );
    assert_eq!(
        remote_state.bg_color, local_state.bg_color,
        "remote skin swap did not update the clear color"
    );
    assert_eq!(remote_state.bg_color, remote_state.active_theme.clear_color);
    assert_eq!(
        h.app_icon_rect("Calculator"),
        local.app_icon_rect("Calculator"),
        "remote skin swap did not rebuild the dashboard for the new skin"
    );
    assert_eq!(
        h.screenshot(),
        local.screenshot(),
        "remote and local skin swaps render differently"
    );

    // Window management over the remote terminal.
    assert!(h.open_app("Calculator"));
    h.settle();
    let win = h.find_window("Calculator").expect("Calculator window");
    let list = remote_cmd(&mut h, &mut p, "wm list");
    assert!(list.contains(&win.id), "{list:?}");
    assert!(list.contains("Calculator"), "{list:?}");
    remote_cmd(&mut h, &mut p, &format!("wm minimize {}", win.id));
    h.settle();
    assert!(h.find_window("Calculator").unwrap().minimized);
    remote_cmd(&mut h, &mut p, &format!("wm close {}", win.id));
    h.settle();
    assert!(h.find_window("Calculator").is_none(), "{:?}", h.windows());

    // `quit` ends the session.
    p.send("quit\n");
    let bye = p.until_eof(&mut h);
    assert!(bye.contains("Goodbye."), "{bye:?}");
}

/// Signals that need the local terminal (nested listeners, outbound
/// connections) are refused over the remote terminal instead of acting.
#[test]
fn remote_terminal_refuses_server_toggles() {
    let mut h = Harness::new("classic");
    h.settle();
    let port = free_port();
    run(
        &mut h,
        &format!("listen {port}"),
        &format!("Listening on 127.0.0.1:{port}"),
    );
    let mut p = remote_client(&mut h, port);
    let other = free_port();
    for cmd in ["listen stop".to_string(), format!("ftp start {other}")] {
        let resp = remote_cmd(&mut h, &mut p, &cmd);
        assert_eq!(resp, "Not available via remote.", "{cmd}");
    }
    assert!(h.state().net.listener.is_some());
    assert!(h.state().net.ftp_server.is_none());
    // Unknown commands report an error and the connection stays usable.
    let resp = remote_cmd(&mut h, &mut p, "definitely-not-a-command");
    assert!(resp.starts_with("error"), "{resp:?}");
    assert_eq!(remote_cmd(&mut h, &mut p, "echo ok"), "ok");
}

/// `tv tune <ch>` (terminal, remote terminal) writes a `tune_ch:` request
/// that nothing consumed; the shell now tunes the open TV Guide with it.
#[test]
fn remote_tv_tune_tunes_the_open_guide() {
    use oasis_core::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};

    let mut h = Harness::new("classic");
    h.settle();
    let port = free_port();
    run(
        &mut h,
        &format!("listen {port}"),
        &format!("Listening on 127.0.0.1:{port}"),
    );
    h.button(Button::Start);
    h.settle();
    assert!(h.open_app("TV Guide"));
    h.settle();
    let (index, number) = {
        let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
        for (i, ch) in g.channels.clone().iter().enumerate() {
            let mut catalog = ChannelCatalog::new(ch.number);
            catalog.add_episodes(vec![VideoEpisode {
                item_id: format!("mock-{}", ch.number),
                filename: "ep.mp4".into(),
                title: format!("Show {}", ch.number),
                duration_secs: 1800.0,
                width: 640,
                height: 480,
                size_bytes: 1_000_000,
                format: "MPEG4".into(),
                original: None,
            }]);
            g.catalogs[i] = Some(catalog);
            g.rebuild_cached_schedule(i);
        }
        g.fetch_attempted = true;
        let last = g.channels.len() - 1;
        (last, g.channels[last].number)
    };
    h.app_runner("TV Guide").unwrap().refresh_tv_text();

    let mut p = remote_client(&mut h, port);
    let resp = remote_cmd(&mut h, &mut p, &format!("tv tune {number}"));
    assert!(resp.contains(&format!("Tuning to CH {number}")), "{resp:?}");
    h.run_frames(3);
    let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
    assert_eq!(g.selected_channel, index);
    assert_eq!(g.tuned_channel, Some(index));
    // The request was consumed.
    let pending = h
        .vfs()
        .read(oasis_core::apps::tv_guide::TV_REQUEST_PATH)
        .unwrap_or_default();
    assert!(
        !String::from_utf8_lossy(&pending).contains("tune_ch"),
        "{pending:?}"
    );
}

// ---------------------------------------------------------------------------
// MCP control server
// ---------------------------------------------------------------------------

#[cfg(feature = "mcp")]
mod mcp {
    use super::*;
    use serde_json::{Value, json};

    /// Start the MCP server from the shell terminal, return to the
    /// dashboard and hand back its port.
    pub(super) fn start(h: &mut Harness) -> u16 {
        let port = free_port();
        run(
            h,
            &format!("mcp-server start {port}"),
            &format!("MCP server listening on 127.0.0.1:{port}"),
        );
        h.button(Button::Start);
        h.settle();
        assert_eq!(h.mode(), Mode::Dashboard);
        port
    }

    /// One JSON-RPC request over its own HTTP/1.1 connection.
    pub(super) fn rpc(h: &mut Harness, port: u16, method: &str, params: Value) -> Value {
        let body =
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
        let mut p = Peer::connect(port);
        p.send(&format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        ));
        let raw = p.until_eof(h);
        let (head, body) = raw.split_once("\r\n\r\n").expect("HTTP response");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let v: Value = serde_json::from_str(body).unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    /// Call a tool; returns `(is_error, content)`.
    pub(super) fn call(h: &mut Harness, port: u16, name: &str, args: Value) -> (bool, Value) {
        let r = rpc(
            h,
            port,
            "tools/call",
            json!({ "name": name, "arguments": args }),
        );
        (r["isError"].as_bool().unwrap(), r["content"].clone())
    }

    /// Call a tool that must succeed; return its text.
    pub(super) fn ok(h: &mut Harness, port: u16, name: &str, args: Value) -> String {
        let (err, content) = call(h, port, name, args.clone());
        assert!(!err, "{name}({args}) failed: {content}");
        content[0]["text"].as_str().unwrap_or_default().to_string()
    }

    /// Call a tool that must fail in-band; return its message.
    pub(super) fn fails(h: &mut Harness, port: u16, name: &str, args: Value) -> String {
        let (err, content) = call(h, port, name, args.clone());
        assert!(err, "{name}({args}) should fail, got {content}");
        content[0]["text"].as_str().unwrap().to_string()
    }

    pub(super) fn window_id(h: &Harness, title: &str) -> String {
        h.find_window(title)
            .unwrap_or_else(|| panic!("no {title} window: {:?}", h.windows()))
            .id
    }
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_tools_list_matches_dispatcher() {
    use serde_json::json;
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    let init = mcp::rpc(&mut h, port, "initialize", json!({}));
    assert!(init["serverInfo"]["name"].is_string(), "{init}");
    let tools = mcp::rpc(&mut h, port, "tools/list", json!({}));
    let mut names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    let mut want = vec![
        "list_apps",
        "open_app",
        "list_windows",
        "focus_window",
        "close_window",
        "minimize_window",
        "maximize_window",
        "restore_window",
        "move_window",
        "resize_window",
        "run_command",
        "browser_navigate",
        "play_media",
        "tune",
        "get_state",
        "screenshot",
    ];
    want.sort_unstable();
    assert_eq!(names, want);
    // Unknown tools are an in-band error.
    mcp::fails(&mut h, port, "no_such_tool", json!({}));
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_drives_apps_and_windows() {
    use serde_json::{Value, json};
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    // list_apps includes the dashboard apps.
    let apps = mcp::ok(&mut h, port, "list_apps", json!({}));
    for title in ["Calculator", "Terminal", "Browser", "TV Guide"] {
        assert!(apps.lines().any(|l| l == title), "{title} missing: {apps}");
    }

    // open_app opens a real window, drawn in the frame.
    let resp = mcp::ok(&mut h, port, "open_app", json!({ "title": "calculator" }));
    assert!(resp.contains("Calculator"), "{resp}");
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    let id = mcp::window_id(&h, "Calculator");
    h.render_now();
    assert!(h.text_drawn_contains("Calculator"));

    // list_windows reports it (focused).
    let list: Value =
        serde_json::from_str(&mcp::ok(&mut h, port, "list_windows", json!({}))).unwrap();
    let w = list["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == id.as_str())
        .unwrap_or_else(|| panic!("{list}"))
        .clone();
    assert_eq!(w["title"], "Calculator");
    assert_eq!(w["focused"], true);

    // move_window to an absolute position.
    mcp::ok(
        &mut h,
        port,
        "move_window",
        json!({ "id": id, "x": 40, "y": 30 }),
    );
    h.settle();
    let f = h.find_window("Calculator").unwrap().frame;
    assert_eq!((f.0, f.1), (40, 30), "move_window");

    // resize_window.
    mcp::ok(
        &mut h,
        port,
        "resize_window",
        json!({ "id": id, "width": 200, "height": 150 }),
    );
    h.settle();
    let f = h.find_window("Calculator").unwrap().frame;
    assert_eq!((f.2, f.3), (200, 150), "resize_window");

    // A second window; focus switches between them.
    mcp::ok(&mut h, port, "open_app", json!({ "title": "Text Editor" }));
    h.settle();
    let editor = mcp::window_id(&h, "Text Editor");
    assert_eq!(h.state().wm.active_window(), Some(editor.as_str()));
    mcp::ok(&mut h, port, "focus_window", json!({ "id": id }));
    h.settle();
    assert_eq!(h.state().wm.active_window(), Some(id.as_str()));

    // minimize / restore / maximize.
    mcp::ok(&mut h, port, "minimize_window", json!({ "id": id }));
    h.settle();
    assert!(h.find_window("Calculator").unwrap().minimized);
    mcp::ok(&mut h, port, "restore_window", json!({ "id": id }));
    h.settle();
    assert!(!h.find_window("Calculator").unwrap().minimized);
    mcp::ok(&mut h, port, "maximize_window", json!({ "id": id }));
    h.settle();
    let (sw, _) = h.size();
    let f = h.find_window("Calculator").unwrap().frame;
    assert!(f.2 >= sw * 9 / 10, "maximized width {} of {sw}", f.2);

    // close_window: the window and its app are gone (the app no longer
    // runs invisibly).
    mcp::ok(&mut h, port, "close_window", json!({ "id": editor }));
    h.settle();
    assert!(h.find_window("Text Editor").is_none(), "{:?}", h.windows());
    assert!(h.app_runner("Text Editor").is_none(), "runner leaked");

    // get_state mirrors the shell.
    let st: Value = serde_json::from_str(&mcp::ok(&mut h, port, "get_state", json!({}))).unwrap();
    assert_eq!(st["skin"], h.state().skin.manifest.name.as_str());
    assert_eq!(st["mode"], format!("{:?}", h.mode()));
    assert_eq!(
        st["open_windows"].as_array().unwrap().len(),
        h.windows().len()
    );
    assert_eq!(st["focused_window"], id.as_str());
    assert!(st["agent_calls"].as_u64().unwrap() >= 12, "{st}");

    // Closing the last window returns to the dashboard, like the titlebar
    // close button.
    mcp::ok(&mut h, port, "close_window", json!({ "id": id }));
    h.settle();
    assert!(h.windows().is_empty(), "{:?}", h.windows());
    assert!(h.app_runner("Calculator").is_none(), "runner leaked");
    assert_eq!(h.mode(), Mode::Dashboard);
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_bad_arguments_are_errors_not_panics() {
    use serde_json::json;
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    mcp::fails(&mut h, port, "open_app", json!({}));
    mcp::fails(&mut h, port, "open_app", json!({ "title": 7 }));
    let msg = mcp::fails(&mut h, port, "open_app", json!({ "title": "Nope" }));
    assert!(msg.contains("unknown app"), "{msg}");
    for op in [
        "focus_window",
        "close_window",
        "minimize_window",
        "maximize_window",
        "restore_window",
    ] {
        mcp::fails(&mut h, port, op, json!({}));
        let msg = mcp::fails(&mut h, port, op, json!({ "id": "ghost" }));
        assert!(msg.contains("ghost"), "{op}: {msg}");
    }
    mcp::fails(
        &mut h,
        port,
        "move_window",
        json!({ "id": "ghost", "x": 1 }),
    );
    mcp::fails(
        &mut h,
        port,
        "move_window",
        json!({ "id": "ghost", "x": 1, "y": 2 }),
    );
    mcp::fails(&mut h, port, "resize_window", json!({ "id": "ghost" }));
    mcp::fails(&mut h, port, "run_command", json!({}));
    mcp::fails(&mut h, port, "browser_navigate", json!({}));
    mcp::fails(&mut h, port, "play_media", json!({}));
    mcp::fails(&mut h, port, "tune", json!({ "source": "tv" }));
    mcp::fails(
        &mut h,
        port,
        "tune",
        json!({ "source": "tv", "channel": "abc" }),
    );
    mcp::fails(
        &mut h,
        port,
        "tune",
        json!({ "source": "vhs", "channel": "1" }),
    );

    // Extreme but well-typed values are clamped, not trusted.
    mcp::ok(&mut h, port, "open_app", json!({ "title": "Calculator" }));
    h.settle();
    let id = mcp::window_id(&h, "Calculator");
    mcp::ok(
        &mut h,
        port,
        "move_window",
        json!({ "id": id, "x": i64::MAX, "y": i64::MIN }),
    );
    mcp::ok(
        &mut h,
        port,
        "resize_window",
        json!({ "id": id, "width": -5, "height": 1_000_000_000 }),
    );
    h.settle();
    h.render_now();
    let f = h.find_window("Calculator").unwrap().frame;
    let (sw, sh) = h.size();
    assert!(f.2 >= 80 && f.3 <= 4096, "{f:?}");
    assert!(
        f.0 < sw as i32 && f.1 < sh as i32,
        "window moved off screen: {f:?}"
    );

    // The shell is still healthy.
    assert_eq!(
        mcp::ok(
            &mut h,
            port,
            "run_command",
            json!({ "command": "echo alive" })
        ),
        "alive"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_run_command_changes_shell_state() {
    use serde_json::json;
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    let out = mcp::ok(
        &mut h,
        port,
        "run_command",
        json!({ "command": "write /tmp/agent.txt from the agent" }),
    );
    assert!(!out.starts_with("error"), "{out}");
    assert_eq!(h.vfs().read("/tmp/agent.txt").unwrap(), b"from the agent");
    let out = mcp::ok(
        &mut h,
        port,
        "run_command",
        json!({ "command": "cat /tmp/agent.txt" }),
    );
    assert_eq!(out, "from the agent");
    let out = mcp::ok(
        &mut h,
        port,
        "run_command",
        json!({ "command": "nope-cmd" }),
    );
    assert!(out.starts_with("error"), "{out}");

    // A skin swap through the agent is a full swap.
    let out = mcp::ok(&mut h, port, "run_command", json!({ "command": "skin xp" }));
    assert!(out.contains("Switched to skin"), "{out}");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "xp");
    assert_eq!(h.state().bg_color, h.state().active_theme.clear_color);

    // Server toggles are refused over MCP.
    let out = mcp::ok(
        &mut h,
        port,
        "run_command",
        json!({ "command": "mcp-server stop" }),
    );
    assert_eq!(out, "Not available via remote.");
    assert!(h.state().mcp.is_some());
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_browser_navigate_renders_a_local_page() {
    use serde_json::{Value, json};
    let mut h = Harness::new("classic");
    h.settle();
    h.vfs_mut()
        .write(
            "/tmp/agent.html",
            b"<html><head><title>Agent Page</title></head>\
              <body><h1>Hello from MCP</h1><p>local page</p></body></html>",
        )
        .unwrap();
    let port = mcp::start(&mut h);

    let out = mcp::ok(
        &mut h,
        port,
        "browser_navigate",
        json!({ "url": "vfs:///tmp/agent.html" }),
    );
    assert!(out.contains("Navigated"), "{out}");
    h.settle();
    assert!(h.find_window("Browser").is_some(), "{:?}", h.windows());
    h.render_now();
    // The page is laid out word by word.
    for word in ["Hello", "from", "MCP", "local", "page"] {
        assert!(
            h.frame_text().iter().any(|t| t.trim() == word),
            "page word {word:?} not drawn: {:?}",
            h.frame_text()
        );
    }
    let st: Value = serde_json::from_str(&mcp::ok(&mut h, port, "get_state", json!({}))).unwrap();
    assert!(
        st["browser_url"]
            .as_str()
            .unwrap_or_default()
            .contains("agent.html"),
        "{st}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_play_media_and_radio_tune_reach_the_controllers() {
    use serde_json::json;
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    // play_media on a missing file: the music controller consumes the
    // request (and reports nothing playing) instead of leaving it queued.
    mcp::ok(
        &mut h,
        port,
        "play_media",
        json!({ "path": "/music/missing.mp3" }),
    );
    h.run_frames(3);
    let pending = h
        .vfs()
        .read(oasis_app_media::MEDIA_REQUEST_PATH)
        .unwrap_or_default();
    assert!(pending.is_empty(), "request left queued: {pending:?}");
    assert!(h.state().media_track.is_none());

    // Radio tune (offline): the radio controller resolves the station and
    // reports the network as disabled.
    mcp::ok(
        &mut h,
        port,
        "tune",
        json!({ "source": "radio", "channel": "0" }),
    );
    h.run_frames(3);
    let err = h.state().radio_manager.error_msg().to_string();
    assert!(err.contains("offline"), "radio error: {err:?}");
    let msg = mcp::ok(
        &mut h,
        port,
        "tune",
        json!({ "source": "radio", "channel": "No Such FM" }),
    );
    assert!(msg.contains("No Such FM"), "{msg}");
    h.run_frames(3);
    let err = h.state().radio_manager.error_msg().to_string();
    assert!(err.contains("station not found"), "radio error: {err:?}");
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_screenshot_matches_the_framebuffer_and_overlay_is_drawn() {
    use serde_json::json;
    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);

    let (err, content) = mcp::call(&mut h, port, "screenshot", json!({}));
    assert!(!err, "{content}");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    let png_bytes = base64_decode(content[0]["data"].as_str().unwrap());
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut img = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut img).unwrap();
    assert_eq!((info.width, info.height), h.size());
    assert!(
        img.chunks_exact(4).any(|p| p[..3] != img[..3]),
        "blank screenshot"
    );

    // The agent-activity pill is painted after a tool call.
    h.render_now();
    assert!(
        h.text_drawn_contains("agent: screenshot"),
        "overlay missing: {:?}",
        h.frame_text()
    );
}

#[cfg(feature = "mcp")]
fn base64_decode(s: &str) -> Vec<u8> {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|&c| c != b'=') {
        let v = ALPHA.iter().position(|&a| a == c).unwrap() as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(all(feature = "mcp", feature = "_video"))]
#[test]
fn mcp_tv_tune_starts_playback() {
    use oasis_core::apps::tv_guide::catalog::{ChannelCatalog, VideoEpisode};
    use serde_json::json;

    let mut h = Harness::new("classic");
    h.settle();
    let port = mcp::start(&mut h);
    // Tuning needs the guide (it owns the catalogs).
    let msg = mcp::fails(
        &mut h,
        port,
        "tune",
        json!({ "source": "tv", "channel": "1" }),
    );
    assert!(msg.contains("TV Guide"), "{msg}");

    mcp::ok(&mut h, port, "open_app", json!({ "title": "TV Guide" }));
    h.settle();
    let target = {
        let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
        for (i, ch) in g.channels.clone().iter().enumerate() {
            let mut catalog = ChannelCatalog::new(ch.number);
            catalog.add_episodes(
                (0..3)
                    .map(|e| VideoEpisode {
                        item_id: format!("mock-{}-{e}", ch.number),
                        filename: format!("ep{e}.mp4"),
                        title: format!("Episode {e}"),
                        duration_secs: 1800.0,
                        width: 640,
                        height: 480,
                        size_bytes: 1_000_000,
                        format: "MPEG4".into(),
                        original: None,
                    })
                    .collect(),
            );
            g.catalogs[i] = Some(catalog);
            g.rebuild_cached_schedule(i);
        }
        g.fetch_attempted = true;
        // Not the channel the guide has selected.
        (1, g.channels[1].number)
    };
    h.app_runner("TV Guide").unwrap().refresh_tv_text();

    mcp::ok(
        &mut h,
        port,
        "tune",
        json!({ "source": "tv", "channel": target.1.to_string() }),
    );
    h.run_frames(3);
    let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
    assert_eq!(g.tuned_channel, Some(target.0));
    assert!(h.state().video_player.is_active(), "tune started playback");
    assert!(h.state().tv_audio_track.is_some());
}
