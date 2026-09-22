//! Loopback end-to-end tests for the remote terminal listener and client.
//!
//! Every test binds `127.0.0.1:0` (an ephemeral port read back through
//! `StdNetworkBackend::local_addr`), so there is no port-picking race. The
//! listener is polled on the test thread; raw `TcpStream` clients run on
//! helper threads with read timeouts, and every wait has a deadline.

#![allow(clippy::unwrap_used)]

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use oasis_net::{ClientState, ListenerConfig, RemoteClient, RemoteListener, StdNetworkBackend};

const DEADLINE: Duration = Duration::from_secs(10);

/// Start a listener on an ephemeral loopback port.
fn start(cfg: ListenerConfig) -> (StdNetworkBackend, RemoteListener, u16) {
    let mut backend = StdNetworkBackend::new();
    let mut listener = RemoteListener::new(ListenerConfig { port: 0, ..cfg });
    listener.start(&mut backend).unwrap();
    let port = backend.local_addr().unwrap().port();
    (backend, listener, port)
}

fn no_psk(max_connections: usize) -> ListenerConfig {
    ListenerConfig {
        max_connections,
        ..ListenerConfig::default()
    }
}

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    s
}

/// Read from `s` until the accumulated bytes contain `marker` (or EOF /
/// deadline). Returns everything read.
fn read_until(s: &mut TcpStream, acc: &mut Vec<u8>, marker: &[u8]) -> bool {
    let start = Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if contains(acc, marker) {
            return true;
        }
        if start.elapsed() > DEADLINE {
            return false;
        }
        match s.read(&mut buf) {
            Ok(0) => return contains(acc, marker),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {},
            Err(_) => return contains(acc, marker),
        }
    }
}

/// Read until EOF (or reset); returns the bytes and whether EOF was seen.
fn read_to_eof(s: &mut TcpStream) -> (Vec<u8>, bool) {
    let start = Instant::now();
    let mut acc = Vec::new();
    let mut buf = [0u8; 4096];
    while start.elapsed() < DEADLINE {
        match s.read(&mut buf) {
            Ok(0) => return (acc, true),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {},
            Err(_) => return (acc, true),
        }
    }
    (acc, false)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Poll the listener (answering each command with `respond`) until the
/// client thread finishes, then return its result.
fn serve<T>(
    backend: &mut StdNetworkBackend,
    listener: &mut RemoteListener,
    client: JoinHandle<T>,
    mut respond: impl FnMut(&str) -> String,
) -> T {
    let start = Instant::now();
    while !client.is_finished() {
        for (cmd, id) in listener.poll(backend) {
            let _ = listener.send_response(id, &respond(&cmd));
        }
        assert!(start.elapsed() < DEADLINE, "client thread did not finish");
        thread::sleep(Duration::from_millis(1));
    }
    client.join().unwrap()
}

/// Poll until `cond` holds or the deadline passes.
fn poll_until(
    backend: &mut StdNetworkBackend,
    listener: &mut RemoteListener,
    mut cond: impl FnMut(&RemoteListener) -> bool,
) -> Vec<(String, usize)> {
    let start = Instant::now();
    let mut cmds = Vec::new();
    while !cond(listener) {
        cmds.extend(listener.poll(backend));
        assert!(start.elapsed() < DEADLINE, "condition not reached");
        thread::sleep(Duration::from_millis(1));
    }
    cmds
}

fn reply(cmd: &str) -> String {
    format!("reply:{cmd}")
}

// ---------------------------------------------------------------------------
// Basic round-trips
// ---------------------------------------------------------------------------

#[test]
fn raw_tcp_round_trip_without_psk() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"remote terminal\n> "));
        s.write_all(b"echo hi\n").unwrap();
        assert!(read_until(&mut s, &mut acc, b"reply:echo hi\n> "));
        s.write_all(b"quit\n").unwrap();
        let (rest, eof) = read_to_eof(&mut s);
        acc.extend_from_slice(&rest);
        assert!(eof, "server did not close after quit");
        String::from_utf8_lossy(&acc).into_owned()
    });
    let out = serve(&mut backend, &mut listener, client, reply);
    assert!(out.contains("Goodbye."), "{out}");
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 0);
}

#[test]
fn remote_client_round_trip_without_psk() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let mut client_backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    client
        .connect(&mut client_backend, "127.0.0.1", port, None)
        .unwrap();
    assert_eq!(client.state(), ClientState::Connected);
    client.send("status").unwrap();

    let start = Instant::now();
    let mut lines = Vec::new();
    while !lines.iter().any(|l: &String| l.contains("reply:status")) {
        for (cmd, id) in listener.poll(&mut backend) {
            listener.send_response(id, &reply(&cmd)).unwrap();
        }
        lines.extend(client.poll());
        assert!(start.elapsed() < DEADLINE, "no reply; got {lines:?}");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(lines.iter().any(|l| l.contains("OASIS_OS remote terminal")));

    // Client-initiated disconnect sends `quit`; the slot is released.
    client.disconnect();
    assert!(!client.is_connected());
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 0);
}

/// The client notices when the server goes away, and a large response
/// arrives in a reasonable number of polls.
#[test]
fn remote_client_large_output_and_server_shutdown() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let mut client_backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    client
        .connect(&mut client_backend, "127.0.0.1", port, None)
        .unwrap();
    client.send("dump").unwrap();
    let big: String = (0..20_000).map(|i| format!("row {i}\n")).collect();

    let start = Instant::now();
    let mut lines: Vec<String> = Vec::new();
    let mut polls = 0usize;
    while !lines.iter().any(|l| l == "row 19999") {
        for (_, id) in listener.poll(&mut backend) {
            listener.send_response(id, &big).unwrap();
        }
        let got = client.poll();
        if !got.is_empty() {
            polls += 1;
        }
        lines.extend(got);
        assert!(start.elapsed() < DEADLINE, "large output stalled");
    }
    // ~150 KB: must not trickle in 512 bytes per poll (~290 polls).
    assert!(polls < 100, "took {polls} data-bearing polls");
    // The first row shares a line with the banner prompt ("> row 0").
    let rows = lines
        .iter()
        .filter(|l| l.trim_start_matches("> ").starts_with("row "))
        .count();
    assert_eq!(rows, 20_000);

    listener.stop();
    let start = Instant::now();
    while client.is_connected() {
        let got = client.poll();
        lines.extend(got);
        assert!(start.elapsed() < DEADLINE, "client never noticed shutdown");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(lines.iter().any(|l| l.contains("Server shutting down.")));
}

// ---------------------------------------------------------------------------
// PSK authentication
// ---------------------------------------------------------------------------

fn with_psk(psk: &str) -> ListenerConfig {
    ListenerConfig {
        psk: psk.to_string(),
        max_connections: 4,
        ..ListenerConfig::default()
    }
}

#[cfg(feature = "tls-rustls")]
#[test]
fn psk_success_then_command() {
    let (mut backend, mut listener, port) = start(with_psk("s3cret"));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"AUTH_REQUIRED\n"));
        s.write_all(b"s3cret\n").unwrap();
        assert!(read_until(&mut s, &mut acc, b"AUTH_OK\n> "));
        s.write_all(b"whoami\n").unwrap();
        assert!(read_until(&mut s, &mut acc, b"reply:whoami\n> "));
    });
    let mut seen = Vec::new();
    serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    // The PSK itself is never surfaced as a command.
    assert_eq!(seen, vec!["whoami".to_string()]);
}

#[cfg(feature = "tls-rustls")]
#[test]
fn wrong_psk_is_rejected_and_closed() {
    let (mut backend, mut listener, port) = start(with_psk("s3cret"));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"AUTH_REQUIRED\n"));
        s.write_all(b"guess\n").unwrap();
        let (rest, eof) = read_to_eof(&mut s);
        acc.extend_from_slice(&rest);
        assert!(eof);
        String::from_utf8_lossy(&acc).into_owned()
    });
    let mut seen = Vec::new();
    let out = serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    assert!(out.contains("AUTH_FAIL"), "{out}");
    assert!(seen.is_empty());
    assert_eq!(listener.connection_count(), 0);
}

/// A wrong guess followed by more lines in the same packet must not get
/// further PSK attempts (or commands) on the rejected connection.
#[cfg(feature = "tls-rustls")]
#[test]
fn lines_after_auth_fail_are_ignored() {
    let (mut backend, mut listener, port) = start(with_psk("s3cret"));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"AUTH_REQUIRED\n"));
        s.write_all(b"guess1\ns3cret\nrm -rf /\n").unwrap();
        let (rest, _) = read_to_eof(&mut s);
        acc.extend_from_slice(&rest);
        String::from_utf8_lossy(&acc).into_owned()
    });
    let mut seen = Vec::new();
    let out = serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    assert!(
        seen.is_empty(),
        "commands dispatched after AUTH_FAIL: {seen:?}"
    );
    assert!(!out.contains("AUTH_OK"), "{out}");
}

#[cfg(feature = "tls-rustls")]
#[test]
fn remote_client_psk_authenticates() {
    let (mut backend, mut listener, port) = start(with_psk("s3cret"));
    let mut client_backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    client
        .connect(&mut client_backend, "127.0.0.1", port, Some("s3cret"))
        .unwrap();
    let start = Instant::now();
    while client.state() != ClientState::Connected {
        listener.poll(&mut backend);
        client.poll();
        assert!(start.elapsed() < DEADLINE);
        thread::sleep(Duration::from_millis(1));
    }
    client.send("ping").unwrap();
    let start = Instant::now();
    let mut got = Vec::new();
    while got.is_empty() {
        got = listener.poll(&mut backend);
        assert!(start.elapsed() < DEADLINE);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(got[0].0, "ping");
}

/// Without TLS support the listener refuses PSK auth outright.
#[cfg(not(feature = "tls-rustls"))]
#[test]
fn psk_without_tls_is_refused() {
    let (mut backend, mut listener, port) = start(with_psk("s3cret"));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let (out, eof) = read_to_eof(&mut s);
        assert!(eof);
        String::from_utf8_lossy(&out).into_owned()
    });
    let out = serve(&mut backend, &mut listener, client, reply);
    assert!(out.contains("AUTH_FAIL"), "{out}");
    assert_eq!(listener.connection_count(), 0);

    // The client side refuses to send a PSK in plaintext as well.
    let mut client_backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    assert!(
        client
            .connect(&mut client_backend, "127.0.0.1", port, Some("s3cret"))
            .is_err()
    );
    assert!(!client.is_connected());
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

/// `quit` followed by more lines in one packet: nothing after `quit` runs.
#[test]
fn lines_after_quit_are_not_dispatched() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        s.write_all(b"quit\necho leaked\n").unwrap();
        read_to_eof(&mut s);
    });
    let mut seen = Vec::new();
    serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    assert!(seen.is_empty(), "command after quit dispatched: {seen:?}");
}

/// A multi-megabyte response reaches the client intact (the socket buffer
/// is far smaller, so this needs queued, resumable writes).
#[test]
fn large_output_arrives_complete() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let big: String = (0..(3 * 1024 * 1024 / 16))
        .map(|i| format!("{i:015}\n"))
        .collect();
    let expected = big.clone();
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        acc.clear();
        s.write_all(b"dump\n").unwrap();
        // Response is `big` + "\n> ".
        let marker = format!("{:015}\n\n> ", 3 * 1024 * 1024 / 16 - 1);
        assert!(read_until(&mut s, &mut acc, marker.as_bytes()));
        acc
    });
    let got = serve(&mut backend, &mut listener, client, |_| big.clone());
    let got = String::from_utf8(got).unwrap();
    let body = got.strip_suffix("\n> ").unwrap();
    assert_eq!(body.len(), expected.len());
    assert!(body == expected, "large response corrupted");
}

/// Clients that vanish without `quit` must release their slot promptly,
/// otherwise `max_connections` abrupt disconnects lock everyone out.
#[test]
fn abrupt_disconnect_frees_slot() {
    let (mut backend, mut listener, port) = start(no_psk(2));
    let mut a = connect(port);
    let mut b = connect(port);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 2);
    // Drain the banners so the drop is a clean FIN (unread data would make
    // the OS send a reset instead, which takes the error path).
    for s in [&mut a, &mut b] {
        let mut acc = Vec::new();
        assert!(read_until(s, &mut acc, b"> "));
    }
    drop(a);
    drop(b);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 0);

    // A new client is served normally.
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        s.write_all(b"after\n").unwrap();
        assert!(read_until(&mut s, &mut acc, b"reply:after"));
    });
    serve(&mut backend, &mut listener, client, reply);
}

/// Many more sequential clients than slots, alternating `quit` and abrupt
/// drops.
#[test]
fn many_sequential_clients() {
    let (mut backend, mut listener, port) = start(no_psk(2));
    let client = thread::spawn(move || {
        for i in 0..20 {
            let mut s = connect(port);
            let mut acc = Vec::new();
            assert!(read_until(&mut s, &mut acc, b"> "), "client {i}: no banner");
            s.write_all(format!("n{i}\n").as_bytes()).unwrap();
            assert!(
                read_until(&mut s, &mut acc, format!("reply:n{i}\n").as_bytes()),
                "client {i}: no reply"
            );
            if i % 2 == 0 {
                s.write_all(b"exit\n").unwrap();
                read_to_eof(&mut s);
            }
        }
    });
    serve(&mut backend, &mut listener, client, reply);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 0);
}

/// Concurrent clients up to the limit each get exactly their own replies.
#[test]
fn concurrent_clients_get_their_own_replies() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let clients: Vec<JoinHandle<()>> = (0..4)
        .map(|c| {
            thread::spawn(move || {
                let mut s = connect(port);
                let mut acc = Vec::new();
                assert!(read_until(&mut s, &mut acc, b"> "));
                for i in 0..15 {
                    s.write_all(format!("c{c}-{i}\n").as_bytes()).unwrap();
                    assert!(read_until(
                        &mut s,
                        &mut acc,
                        format!("reply:c{c}-{i}\n").as_bytes()
                    ));
                }
                let text = String::from_utf8_lossy(&acc).into_owned();
                for other in (0..4).filter(|o| *o != c) {
                    assert!(
                        !text.contains(&format!("reply:c{other}-")),
                        "client {c} received client {other}'s output"
                    );
                }
                s.write_all(b"quit\n").unwrap();
                read_to_eof(&mut s);
            })
        })
        .collect();
    let joined = thread::spawn(move || {
        for c in clients {
            c.join().unwrap();
        }
    });
    serve(&mut backend, &mut listener, joined, reply);
    assert!(listener.connection_count() <= 4);
}

/// When an earlier connection is removed in the same poll that yields a
/// later connection's command, the returned id must still address the
/// right client (no cross-client output).
#[test]
fn connection_ids_stay_stable_across_removals() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let mut a = connect(port);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 1);
    let mut b = connect(port);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 2);
    let mut c = connect(port);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 3);
    for s in [&mut a, &mut b, &mut c] {
        let mut acc = Vec::new();
        assert!(read_until(s, &mut acc, b"> "));
    }

    // A quits while B issues a command, landing in the same poll.
    a.write_all(b"quit\n").unwrap();
    b.write_all(b"for-b\n").unwrap();
    thread::sleep(Duration::from_millis(100));
    let cmds = listener.poll(&mut backend);
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    listener.send_response(cmds[0].1, "secret-for-b").unwrap();
    // Flush.
    for _ in 0..5 {
        listener.poll(&mut backend);
        thread::sleep(Duration::from_millis(5));
    }

    let mut got_b = Vec::new();
    assert!(
        read_until(&mut b, &mut got_b, b"secret-for-b"),
        "B got nothing"
    );
    let mut got_c = Vec::new();
    c.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut buf = [0u8; 256];
    if let Ok(n) = c.read(&mut buf) {
        got_c.extend_from_slice(&buf[..n]);
    }
    assert!(
        !contains(&got_c, b"secret-for-b"),
        "B's output was delivered to C"
    );
}

/// Binary garbage and invalid UTF-8 do not panic or wedge the connection.
#[test]
fn garbage_input_is_survivable() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        let mut junk: Vec<u8> = (0..=255u8).filter(|b| *b != b'\n').collect();
        junk.extend_from_slice(b"\n\xff\xfe\xc3\x28\n\0\0\0\n\r\r\r\n");
        s.write_all(&junk).unwrap();
        s.write_all(b"still-alive\n").unwrap();
        assert!(read_until(&mut s, &mut acc, b"reply:still-alive"));
    });
    let mut seen = Vec::new();
    serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    assert_eq!(seen.last().map(String::as_str), Some("still-alive"));
}

/// An endless line without a newline gets the peer disconnected rather
/// than buffered without bound.
#[test]
fn overlong_line_disconnects() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        // Write until the server hangs up (bounded to 8 MiB).
        let chunk = vec![b'A'; 16 * 1024];
        let mut sent = 0usize;
        s.set_write_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        while sent < 8 * 1024 * 1024 {
            match s.write(&chunk) {
                Ok(n) => sent += n,
                Err(_) => break,
            }
        }
        let (rest, eof) = read_to_eof(&mut s);
        acc.extend_from_slice(&rest);
        assert!(eof, "server kept an endless line open");
        (String::from_utf8_lossy(&acc).into_owned(), sent)
    });
    let mut seen = Vec::new();
    let (out, sent) = serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.len());
        reply(c)
    });
    assert!(seen.is_empty());
    assert!(sent < 8 * 1024 * 1024, "server never disconnected");
    let _ = out; // The "line too long" notice may be lost to a reset.
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 0);
}

/// A burst of many lines in one write is dispatched completely and in
/// order.
#[test]
fn burst_of_lines_dispatched_in_order() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let client = thread::spawn(move || {
        let mut s = connect(port);
        let mut acc = Vec::new();
        assert!(read_until(&mut s, &mut acc, b"> "));
        let burst: String = (0..300).map(|i| format!("line{i}\n")).collect();
        s.write_all(burst.as_bytes()).unwrap();
        assert!(read_until(&mut s, &mut acc, b"reply:line299\n"));
    });
    let mut seen = Vec::new();
    serve(&mut backend, &mut listener, client, |c| {
        seen.push(c.to_string());
        reply(c)
    });
    let expected: Vec<String> = (0..300).map(|i| format!("line{i}")).collect();
    assert_eq!(seen, expected);
}

/// `stop()` tells connected clients and closes them.
#[test]
fn stop_notifies_clients() {
    let (mut backend, mut listener, port) = start(no_psk(4));
    let mut s = connect(port);
    poll_until(&mut backend, &mut listener, |l| l.connection_count() == 1);
    listener.stop();
    assert!(!listener.is_listening());
    let (out, eof) = read_to_eof(&mut s);
    assert!(eof);
    assert!(contains(&out, b"Server shutting down."));
}
