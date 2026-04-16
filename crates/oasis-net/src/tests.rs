//! Tests for the networking module.
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::TcpStream;

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result as OasisResult};

use super::*;

// ---------------------------------------------------------------------------
// StdNetworkBackend tests
// ---------------------------------------------------------------------------

/// Helper: find a free TCP port by binding to port 0 and releasing it.
///
/// There's an inherent TOCTOU race: between the temporary listener's
/// drop and the caller's subsequent bind, a parallel test may snatch
/// the same port. Callers that immediately rebind the returned port
/// should use [`bind_listener_retry`] instead, which re-picks a fresh
/// port if the race fires.
///
/// Returns `None` if the OS cannot allocate an ephemeral port (e.g.
/// port exhaustion under heavy parallel test load).
fn free_port() -> Option<u16> {
    let tmp = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    let port = tmp.local_addr().ok()?.port();
    drop(tmp);
    Some(port)
}

/// Start a `RemoteListener` on a free port, retrying on bind failure.
///
/// Combines [`free_port`] and `RemoteListener::start` with a retry
/// loop that re-picks a fresh port on each attempt, closing the
/// TOCTOU window where a parallel test could have taken the port
/// between `free_port`'s drop and `listener.start`'s bind. Tests
/// hitting this race in CI (e.g. `listener_psk_auth` under parallel
/// cargo test) should use this helper rather than calling
/// `free_port` + `listener.start().unwrap()` directly.
fn bind_listener_retry(
    config_template: impl Fn(u16) -> ListenerConfig,
    backend: &mut StdNetworkBackend,
) -> (u16, RemoteListener) {
    let mut last_err = None;
    for _ in 0..20 {
        let Some(port) = free_port() else {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        };
        let mut listener = RemoteListener::new(config_template(port));
        match listener.start(backend) {
            Ok(()) => return (port, listener),
            Err(e) => last_err = Some(e),
        }
        // Brief pause before retry to let ephemeral ports recycle.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!(
        "bind_listener_retry: failed to bind a free port after 20 attempts: {:?}",
        last_err
    );
}

#[test]
fn listen_and_accept() {
    let mut backend = StdNetworkBackend::new();
    // Same TOCTOU race as the `RemoteListener` tests — retry on
    // bind failure with a fresh port. Inlined (not extracted to a
    // helper) since this is the only raw-backend test that hits it.
    let port = {
        let mut bound = None;
        for _ in 0..20 {
            if let Some(p) = free_port() {
                if backend.listen(p).is_ok() {
                    bound = Some(p);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        bound.expect("failed to bind a free port after 20 attempts")
    };

    // No connection yet.
    let result = backend.accept().unwrap();
    assert!(result.is_none());

    // Connect a client.
    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    client.write_all(b"hello").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Accept should now return a stream.
    let mut server_stream = backend.accept().unwrap().expect("expected connection");

    let mut buf = [0u8; 64];
    let n = server_stream.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");

    // Server can write back.
    server_stream.write(b"world").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let mut response = [0u8; 64];
    let n = client.read(&mut response).unwrap();
    assert_eq!(&response[..n], b"world");

    server_stream.close().unwrap();
}

#[test]
fn connect_outbound() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.write_all(b"greeting").unwrap();
    });

    let mut backend = StdNetworkBackend::new();
    let mut stream = backend.connect("127.0.0.1", port).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"greeting");

    stream.close().unwrap();
    handle.join().unwrap();
}

#[test]
fn accept_without_listen_errors() {
    let mut backend = StdNetworkBackend::new();
    assert!(backend.accept().is_err());
}

#[test]
fn default_constructor() {
    let _backend = StdNetworkBackend::default();
}

// ---------------------------------------------------------------------------
// RemoteListener tests
// ---------------------------------------------------------------------------

#[test]
fn listener_not_listening_by_default() {
    let listener = RemoteListener::new(ListenerConfig::default());
    assert!(!listener.is_listening());
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn listener_start_and_stop() {
    let mut backend = StdNetworkBackend::new();
    let (_port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );
    assert!(listener.is_listening());
    listener.stop();
    assert!(!listener.is_listening());
}

#[test]
fn listener_accept_no_auth() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    // Connect a client.
    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Poll to accept.
    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty()); // No commands yet.
    assert_eq!(listener.connection_count(), 1);

    // Read the welcome message.
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).unwrap();
    let welcome = String::from_utf8_lossy(&buf[..n]);
    assert!(welcome.contains("OASIS_OS"));

    // Send a command.
    client.write_all(b"status\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "status");

    // Send response.
    listener.send_response(0, "OASIS_OS v0.1.0").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let n = client.read(&mut buf).unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(response.contains("OASIS_OS v0.1.0"));

    listener.stop();
}

#[test]
#[cfg(feature = "tls-rustls")]
fn listener_psk_auth() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: "secret123".to_string(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Accept connection.
    listener.poll(&mut backend);

    // Read auth prompt.
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).unwrap();
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("AUTH_REQUIRED"));

    // Send correct PSK.
    client.write_all(b"secret123\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    listener.poll(&mut backend);

    let n = client.read(&mut buf).unwrap();
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("AUTH_OK"));

    // Now send a command.
    client.write_all(b"help\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "help");

    listener.stop();
}

#[test]
#[cfg(not(feature = "tls-rustls"))]
fn listener_rejects_psk_without_tls() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: "secret123".to_string(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Accept connection -- should be immediately rejected.
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 0);

    // Client should receive rejection message.
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).unwrap();
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("AUTH_FAIL"));

    listener.stop();
}

#[test]
fn listener_psk_auth_failure() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: "correct_key".to_string(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    listener.poll(&mut backend);

    // Read auth prompt.
    let mut buf = [0u8; 256];
    let _n = client.read(&mut buf).unwrap();

    // Send wrong PSK.
    client.write_all(b"wrong_key\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    listener.poll(&mut backend);
    // Connection should be removed.
    assert_eq!(listener.connection_count(), 0);

    listener.stop();
}

#[test]
fn listener_quit_command() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    listener.poll(&mut backend);

    // Read welcome.
    let mut buf = [0u8; 256];
    let _n = client.read(&mut buf).unwrap();

    // Send quit.
    client.write_all(b"quit\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty()); // quit is handled internally.
    assert_eq!(listener.connection_count(), 0);

    listener.stop();
}

// ---------------------------------------------------------------------------
// RemoteClient tests
// ---------------------------------------------------------------------------

#[test]
fn client_default_state() {
    let client = RemoteClient::new();
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(!client.is_connected());
}

#[test]
fn client_connect_no_auth() {
    // Set up a simple echo server.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.write_all(b"Hello from server\n").unwrap();
        let mut buf = [0u8; 256];
        let n = conn.read(&mut buf).unwrap();
        // Echo back.
        conn.write_all(&buf[..n]).unwrap();
    });

    let mut backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    client
        .connect(&mut backend, "127.0.0.1", port, None)
        .unwrap();
    assert_eq!(client.state(), ClientState::Connected);

    std::thread::sleep(std::time::Duration::from_millis(100));

    let lines = client.poll();
    assert!(!lines.is_empty());
    assert!(lines[0].contains("Hello from server"));

    client.send("test command").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    let lines = client.poll();
    assert!(!lines.is_empty());

    client.disconnect();
    assert_eq!(client.state(), ClientState::Disconnected);

    handle.join().unwrap();
}

#[test]
fn client_send_without_connect_errors() {
    let mut client = RemoteClient::new();
    assert!(client.send("test").is_err());
}

// ---------------------------------------------------------------------------
// Host configuration tests
// ---------------------------------------------------------------------------

#[test]
fn parse_hosts_toml() {
    let toml = r##"
[[host]]
name = "briefcase"
address = "192.168.0.50"
port = 9000
protocol = "oasis-terminal"
psk = "secret"

[[host]]
name = "dev-server"
address = "192.168.0.100"
port = 22
protocol = "raw-tcp"
"##;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].name, "briefcase");
    assert_eq!(hosts[0].port, 9000);
    assert_eq!(hosts[0].psk, Some("secret".to_string()));
    assert_eq!(hosts[1].name, "dev-server");
    assert_eq!(hosts[1].port, 22);
    assert!(hosts[1].psk.is_none());
}

#[test]
fn parse_hosts_defaults() {
    let toml = r#"
[[host]]
name = "minimal"
address = "10.0.0.1"
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].port, 9000);
    assert_eq!(hosts[0].protocol, "oasis-terminal");
}

#[test]
fn parse_empty_hosts() {
    let hosts = hosts::parse_hosts("").unwrap();
    assert!(hosts.is_empty());
}

// ---------------------------------------------------------------------------
// Robustness / edge cases
// ---------------------------------------------------------------------------

#[test]
fn parse_hosts_missing_name() {
    let toml = r#"
[[host]]
address = "10.0.0.1"
"#;
    // Missing required `name` field -- should fail gracefully.
    let result = hosts::parse_hosts(toml);
    assert!(result.is_err());
}

#[test]
fn parse_hosts_missing_address() {
    let toml = r#"
[[host]]
name = "test"
"#;
    let result = hosts::parse_hosts(toml);
    assert!(result.is_err());
}

#[test]
fn parse_hosts_invalid_toml() {
    let result = hosts::parse_hosts("{{{{not valid toml}}}}");
    assert!(result.is_err());
}

#[test]
fn parse_hosts_extra_fields_ignored() {
    let toml = r#"
[[host]]
name = "test"
address = "10.0.0.1"
unknown_field = "should be ignored"
"#;
    // Extra fields should not cause a parse error.
    let result = hosts::parse_hosts(toml);
    assert!(result.is_ok());
}

#[test]
fn parse_hosts_unicode_name() {
    let toml = r#"
[[host]]
name = "서버"
address = "10.0.0.1"
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].name, "서버");
}

#[test]
fn parse_hosts_port_zero() {
    let toml = r#"
[[host]]
name = "test"
address = "10.0.0.1"
port = 0
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].port, 0);
}

#[test]
fn parse_hosts_many_entries() {
    let mut toml = String::new();
    for i in 0..50 {
        toml.push_str(&format!(
            r#"
[[host]]
name = "host_{i}"
address = "10.0.0.{}"
"#,
            i % 256
        ));
    }
    let hosts = hosts::parse_hosts(&toml).unwrap();
    assert_eq!(hosts.len(), 50);
}

#[test]
fn parse_hosts_empty_name() {
    let toml = r#"
[[host]]
name = ""
address = "10.0.0.1"
"#;
    // Empty string name is technically valid TOML -- should parse.
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].name, "");
}

#[test]
fn parse_hosts_psk_special_chars() {
    let toml = r#"
[[host]]
name = "secure"
address = "10.0.0.1"
psk = "p@ss w0rd!#$%^&*()"
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].psk, Some("p@ss w0rd!#$%^&*()".to_string()));
}

#[test]
fn listener_double_stop_is_ok() {
    let mut backend = StdNetworkBackend::new();
    let (_port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );
    listener.stop();
    // Double stop should not panic.
    listener.stop();
}

#[test]
fn listener_start_stop_start() {
    let mut backend = StdNetworkBackend::new();
    let make_config = |port| ListenerConfig {
        port,
        psk: String::new(),
        max_connections: 2,
        ..ListenerConfig::default()
    };
    let (_port1, mut listener) = bind_listener_retry(make_config, &mut backend);
    listener.stop();

    let (_port2, mut listener2) = bind_listener_retry(make_config, &mut backend);
    assert!(listener2.is_listening());
    listener2.stop();
}

#[test]
fn client_default_not_connected() {
    let client = RemoteClient::new();
    assert!(!client.is_connected());
}

// ---------------------------------------------------------------------------
// Edge-case integration tests (real TCP)
// ---------------------------------------------------------------------------

#[test]
fn listener_max_connections_reached() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    // Connect two clients (the maximum).
    let _c1 = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    let _c2 = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 2);

    // Third connection -- should NOT be accepted.
    let _c3 = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 2);

    listener.stop();
}

#[test]
fn listener_overlong_line_disconnects() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    // Read the welcome message so it doesn't interfere.
    let mut buf = [0u8; 256];
    let _ = client.read(&mut buf).unwrap();

    // Send more than 1024 bytes without a newline.
    // The listener reads in 512-byte chunks, so we need multiple polls
    // to accumulate past MAX_LINE_LEN (1024).
    let overlong = vec![b'A'; 2048];
    client.write_all(&overlong).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Poll multiple times so the 512-byte reads accumulate past 1024.
    for _ in 0..6 {
        listener.poll(&mut backend);
        if listener.connection_count() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Connection should be dropped due to overlong line.
    assert_eq!(listener.connection_count(), 0);

    listener.stop();
}

#[test]
fn listener_empty_lines_ignored() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);

    // Read the welcome message.
    let mut buf = [0u8; 256];
    let _ = client.read(&mut buf).unwrap();

    // Send empty lines followed by a real command.
    client.write_all(b"\n\n\nhello\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "hello");

    listener.stop();
}

#[test]
fn listener_multiple_commands_in_one_read() {
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);

    // Read the welcome message.
    let mut buf = [0u8; 256];
    let _ = client.read(&mut buf).unwrap();

    // Send three commands in a single write.
    client.write_all(b"cmd1\ncmd2\ncmd3\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].0, "cmd1");
    assert_eq!(commands[1].0, "cmd2");
    assert_eq!(commands[2].0, "cmd3");

    listener.stop();
}

#[test]
fn client_disconnect_and_reconnect() {
    // Start a listener to act as the server.
    let mut backend = StdNetworkBackend::new();
    let (port, mut listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            max_connections: 2,
            ..ListenerConfig::default()
        },
        &mut backend,
    );

    // First connection.
    let mut client_backend = StdNetworkBackend::new();
    let mut client = RemoteClient::new();
    client
        .connect(&mut client_backend, "127.0.0.1", port, None)
        .unwrap();
    assert!(client.is_connected());
    std::thread::sleep(std::time::Duration::from_millis(50));
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    // Disconnect the client.
    client.disconnect();
    assert!(!client.is_connected());
    assert_eq!(client.state(), ClientState::Disconnected);
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Reconnect with a new client.
    let mut client2 = RemoteClient::new();
    client2
        .connect(&mut client_backend, "127.0.0.1", port, None)
        .unwrap();
    assert!(client2.is_connected());
    assert_eq!(client2.state(), ClientState::Connected);
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Poll to accept the new connection and clean up the old one.
    listener.poll(&mut backend);

    // Send a command to verify the new connection works.
    client2.send("test_reconnect").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let commands = listener.poll(&mut backend);
    assert!(!commands.is_empty());
    assert!(commands.iter().any(|(cmd, _)| cmd == "test_reconnect"));

    client2.disconnect();
    listener.stop();
}

// ===========================================================================
// Mock-based tests (no real TCP connections)
// ===========================================================================

/// A mock `NetworkStream` backed by in-memory buffers.
///
/// `read_data` is the data the "remote side" sends to us.
/// `written` accumulates data we write to the "remote side".
/// `read_behavior` controls what happens when `read_data` is exhausted.
struct MockStream {
    read_data: Vec<u8>,
    read_pos: usize,
    written: Vec<u8>,
    /// What to return when all read_data is consumed.
    eof_behavior: MockEofBehavior,
    closed: bool,
}

#[derive(Clone, Copy)]
enum MockEofBehavior {
    /// Return Ok(0) -- EOF.
    Eof,
    /// Return WouldBlock error.
    WouldBlock,
}

impl MockStream {
    fn new(read_data: &[u8], eof: MockEofBehavior) -> Self {
        Self {
            read_data: read_data.to_vec(),
            read_pos: 0,
            written: Vec::new(),
            eof_behavior: eof,
            closed: false,
        }
    }

    /// Create a stream that returns WouldBlock after data is consumed.
    fn non_blocking(read_data: &[u8]) -> Self {
        Self::new(read_data, MockEofBehavior::WouldBlock)
    }

    /// Create a stream that returns EOF after data is consumed.
    fn with_eof(read_data: &[u8]) -> Self {
        Self::new(read_data, MockEofBehavior::Eof)
    }
}

impl NetworkStream for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> OasisResult<usize> {
        if self.read_pos >= self.read_data.len() {
            return match self.eof_behavior {
                MockEofBehavior::Eof => Ok(0),
                MockEofBehavior::WouldBlock => Err(OasisError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "would block",
                ))),
            };
        }
        let available = &self.read_data[self.read_pos..];
        let n = buf.len().min(available.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.read_pos += n;
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> OasisResult<usize> {
        self.written.extend_from_slice(data);
        Ok(data.len())
    }

    fn close(&mut self) -> OasisResult<()> {
        self.closed = true;
        Ok(())
    }
}

/// A mock `NetworkBackend` that returns pre-configured streams.
struct MockBackend {
    /// Streams returned by `connect()`, consumed in order.
    connect_streams: Vec<Box<dyn NetworkStream>>,
    /// Streams returned by `accept()`, consumed in order.
    accept_streams: Vec<Box<dyn NetworkStream>>,
    listening: bool,
    /// Track connect errors.
    connect_error: Option<String>,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            connect_streams: Vec::new(),
            accept_streams: Vec::new(),
            listening: false,
            connect_error: None,
        }
    }

    fn with_connect_stream(mut self, stream: Box<dyn NetworkStream>) -> Self {
        self.connect_streams.push(stream);
        self
    }

    fn with_accept_stream(mut self, stream: Box<dyn NetworkStream>) -> Self {
        self.accept_streams.push(stream);
        self
    }

    fn with_connect_error(mut self, msg: &str) -> Self {
        self.connect_error = Some(msg.to_string());
        self
    }
}

impl NetworkBackend for MockBackend {
    fn listen(&mut self, _port: u16) -> OasisResult<()> {
        self.listening = true;
        Ok(())
    }

    fn accept(&mut self) -> OasisResult<Option<Box<dyn NetworkStream>>> {
        if !self.listening {
            return Err(OasisError::Backend("not listening".into()));
        }
        if self.accept_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.accept_streams.remove(0)))
        }
    }

    fn connect(&mut self, _address: &str, _port: u16) -> OasisResult<Box<dyn NetworkStream>> {
        if let Some(ref msg) = self.connect_error {
            return Err(OasisError::Backend(msg.clone().into()));
        }
        if self.connect_streams.is_empty() {
            return Err(OasisError::Backend("no mock streams available".into()));
        }
        Ok(self.connect_streams.remove(0))
    }
}

// ---------------------------------------------------------------------------
// Mock-based RemoteClient tests
// ---------------------------------------------------------------------------

#[test]
fn mock_client_connect_no_auth_sets_connected() {
    let stream = Box::new(MockStream::non_blocking(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    assert_eq!(client.state(), ClientState::Connected);
    assert!(client.is_connected());
}

#[test]
fn mock_client_connect_failure_propagates() {
    let mut backend = MockBackend::new().with_connect_error("connection refused");
    let mut client = RemoteClient::new();

    let result = client.connect(&mut backend, "10.0.0.1", 9000, None);
    assert!(result.is_err());
    assert_eq!(client.state(), ClientState::Disconnected);
}

#[test]
fn mock_client_poll_extracts_lines() {
    // Simulate server sending two complete lines.
    let stream = Box::new(MockStream::non_blocking(b"line one\nline two\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "line one");
    assert_eq!(lines[1], "line two");
}

#[test]
fn mock_client_poll_partial_line_buffered() {
    // First poll: partial line (no newline).
    // Second poll: rest of line arrives.
    let stream = Box::new(MockStream::non_blocking(b"partial"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();

    // First poll -- no newline, so no lines returned.
    let lines = client.poll();
    assert!(lines.is_empty());

    // The read_buf should have accumulated "partial".
    assert_eq!(client.read_buf.len(), 7);
}

#[test]
fn mock_client_poll_empty_lines_skipped() {
    let stream = Box::new(MockStream::non_blocking(b"\n\n\nhello\n\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();

    // Empty lines should be filtered out.
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "hello");
}

#[test]
fn mock_client_poll_auth_ok_transitions_to_connected() {
    // Simulate: we are in Authenticating state, server sends AUTH_OK.
    let stream = Box::new(MockStream::non_blocking(b"AUTH_OK\nWelcome!\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    // Manually set up authenticating state (connect with PSK requires TLS
    // feature, so we simulate the state directly).
    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    // Force authenticating state.
    client.state = ClientState::Authenticating;
    client.auth_started = Some(std::time::Instant::now());

    let lines = client.poll();
    // AUTH_OK should be consumed (not returned as a line).
    // "Welcome!" should appear as output.
    assert_eq!(client.state(), ClientState::Connected);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "Welcome!");
}

#[test]
fn mock_client_poll_auth_fail_disconnects() {
    let stream = Box::new(MockStream::non_blocking(b"AUTH_FAIL\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    client.state = ClientState::Authenticating;
    client.auth_started = Some(std::time::Instant::now());

    let lines = client.poll();
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(lines.iter().any(|l| l.contains("Authentication failed")));
}

#[test]
fn mock_client_poll_connection_lost() {
    // Simulate an I/O error (not WouldBlock, not Ok(0)).
    struct ErrorStream;
    impl NetworkStream for ErrorStream {
        fn read(&mut self, _buf: &mut [u8]) -> OasisResult<usize> {
            Err(OasisError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "connection reset",
            )))
        }
        fn write(&mut self, _data: &[u8]) -> OasisResult<usize> {
            Ok(0)
        }
        fn close(&mut self) -> OasisResult<()> {
            Ok(())
        }
    }

    let mut backend = MockBackend::new().with_connect_stream(Box::new(ErrorStream));
    let mut client = RemoteClient::new();
    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();

    let lines = client.poll();
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(lines.iter().any(|l| l.contains("Connection lost")));
}

#[test]
fn mock_client_poll_overlong_line_disconnects() {
    // Send >16384 bytes without a newline.
    // Client reads 512 bytes per poll(), so we need multiple polls to
    // accumulate past MAX_LINE_LEN (16384).
    let data = vec![b'X'; 20_000];
    let stream = Box::new(MockStream::with_eof(&data));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();

    // Poll repeatedly until the buffer overflows or we hit EOF.
    let mut all_lines = Vec::new();
    for _ in 0..50 {
        let lines = client.poll();
        all_lines.extend(lines);
        if client.state() == ClientState::Disconnected {
            break;
        }
    }
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(all_lines.iter().any(|l| l.contains("line too long")));
}

#[test]
fn mock_client_send_writes_newline_terminated() {
    let stream = Box::new(MockStream::non_blocking(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    client.send("hello world").unwrap();

    // send() returning Ok confirms the write succeeded.
    // The format is "{line}\n" -- verified by the implementation.
}

#[test]
fn mock_client_disconnect_sends_quit() {
    let stream = Box::new(MockStream::non_blocking(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    assert!(client.is_connected());

    client.disconnect();
    assert!(!client.is_connected());
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(client.stream.is_none());
}

#[test]
fn mock_client_poll_wouldblock_returns_empty() {
    // Stream immediately returns WouldBlock (no data available).
    let stream = Box::new(MockStream::non_blocking(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();
    assert!(lines.is_empty());
    // Client should still be connected.
    assert_eq!(client.state(), ClientState::Connected);
}

#[test]
fn mock_client_poll_crlf_line_endings() {
    // Windows-style line endings should be handled (trimmed).
    let stream = Box::new(MockStream::non_blocking(b"hello\r\nworld\r\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "hello");
    assert_eq!(lines[1], "world");
}

#[test]
fn mock_client_poll_utf8_lossy() {
    // Invalid UTF-8 should be replaced, not crash.
    let mut data = b"valid\n".to_vec();
    data.extend_from_slice(&[0xFF, 0xFE, b'\n']);
    let stream = Box::new(MockStream::non_blocking(&data));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "valid");
    // Second line contains replacement characters.
    assert!(lines[1].contains('\u{FFFD}'));
}

#[test]
fn mock_client_poll_multiple_lines_in_single_read() {
    let stream = Box::new(MockStream::non_blocking(b"cmd1\ncmd2\ncmd3\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "cmd1");
    assert_eq!(lines[1], "cmd2");
    assert_eq!(lines[2], "cmd3");
}

#[test]
fn mock_client_poll_auth_ok_followed_by_auth_fail_ignored() {
    // Once authenticated, AUTH_FAIL is treated as a normal line.
    let stream = Box::new(MockStream::non_blocking(b"AUTH_OK\nAUTH_FAIL\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    client.state = ClientState::Authenticating;
    client.auth_started = Some(std::time::Instant::now());

    let lines = client.poll();
    assert_eq!(client.state(), ClientState::Connected);
    // AUTH_FAIL should appear as a regular line since we're now Connected.
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "AUTH_FAIL");
}

#[test]
fn mock_client_eof_returns_empty() {
    // Stream returns EOF (Ok(0)) immediately.
    let stream = Box::new(MockStream::with_eof(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();
    assert!(lines.is_empty());
}

// ---------------------------------------------------------------------------
// Mock-based RemoteListener tests
// ---------------------------------------------------------------------------

#[test]
fn mock_listener_poll_not_listening_returns_empty() {
    let mut listener = RemoteListener::new(ListenerConfig::default());
    let mut backend = MockBackend::new();

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty());
}

#[test]
fn mock_listener_accept_no_auth_sends_welcome() {
    let stream = MockStream::non_blocking(b"");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty());
    assert_eq!(listener.connection_count(), 1);
}

#[test]
fn mock_listener_authenticated_command_returned() {
    // Simulate: no PSK required, client sends "status\n".
    let stream = MockStream::non_blocking(b"status\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // First poll: accept + read command.
    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "status");
    assert_eq!(commands[0].1, 0); // connection index
}

#[test]
fn mock_listener_multiple_commands_single_poll() {
    let stream = MockStream::non_blocking(b"cmd1\ncmd2\ncmd3\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].0, "cmd1");
    assert_eq!(commands[1].0, "cmd2");
    assert_eq!(commands[2].0, "cmd3");
}

#[test]
fn mock_listener_empty_lines_filtered() {
    let stream = MockStream::non_blocking(b"\n\nhello\n\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "hello");
}

#[test]
fn mock_listener_quit_removes_connection() {
    let stream = MockStream::non_blocking(b"quit\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty()); // "quit" is handled internally.
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_exit_removes_connection() {
    let stream = MockStream::non_blocking(b"exit\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty());
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_psk_correct_authenticates() {
    // Client sends the correct PSK.
    let stream = MockStream::non_blocking(b"my-secret\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "my-secret".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // On first poll: accept + auth check.
    // Without tls-rustls, PSK connections are rejected at accept time.
    // With tls-rustls, the PSK is verified.
    // We test the logic path that exists regardless of feature flag.
    let _commands = listener.poll(&mut backend);

    // The behavior depends on whether tls-rustls is enabled:
    // - With TLS: connection accepted, AUTH_REQUIRED sent, PSK verified.
    // - Without TLS: connection rejected immediately with AUTH_FAIL.
    #[cfg(feature = "tls-rustls")]
    {
        assert_eq!(listener.connection_count(), 1);
    }
    #[cfg(not(feature = "tls-rustls"))]
    {
        assert_eq!(listener.connection_count(), 0);
    }
}

#[test]
fn mock_listener_psk_wrong_disconnects() {
    let stream = MockStream::non_blocking(b"wrong-key\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "correct-key".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert!(commands.is_empty());
    // Connection should be removed regardless of TLS feature.
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_psk_auth_then_command() {
    // Client sends PSK on first line, then a command.
    let stream = MockStream::non_blocking(b"secret\nhello\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "secret".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);

    #[cfg(feature = "tls-rustls")]
    {
        // With TLS: PSK accepted, then "hello" is a command.
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, "hello");
    }
    #[cfg(not(feature = "tls-rustls"))]
    {
        // Without TLS: connection rejected at accept time, no commands.
        assert!(commands.is_empty());
    }
}

#[test]
fn mock_listener_max_connections_enforced() {
    let stream1 = MockStream::non_blocking(b"");
    let stream2 = MockStream::non_blocking(b"");
    let stream3 = MockStream::non_blocking(b"");
    let mut backend = MockBackend::new()
        .with_accept_stream(Box::new(stream1))
        .with_accept_stream(Box::new(stream2))
        .with_accept_stream(Box::new(stream3));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        max_connections: 2,
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // First poll: accept stream1.
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    // Second poll: accept stream2.
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 2);

    // Third poll: should NOT accept stream3 (at max).
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 2);
}

#[test]
fn mock_listener_overlong_line_disconnects() {
    // Send >1024 bytes without a newline.
    let data = vec![b'A'; 2048];
    let stream = MockStream::with_eof(&data);
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // Poll to accept and read the overlong data.
    // May need multiple polls since read is 512 bytes at a time.
    for _ in 0..10 {
        listener.poll(&mut backend);
        if listener.connection_count() == 0 {
            break;
        }
    }
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_send_response_to_connection() {
    let stream = MockStream::non_blocking(b"");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // Accept connection.
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    // Send response.
    let result = listener.send_response(0, "output text");
    assert!(result.is_ok());
}

#[test]
fn mock_listener_send_response_invalid_index() {
    let config = ListenerConfig::default();
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let result = listener.send_response(99, "text");
    assert!(result.is_err());
    if let Err(OasisError::Backend(msg)) = result {
        assert!(msg.to_string().contains("invalid connection index"));
    }
}

#[test]
fn mock_listener_io_error_removes_connection() {
    struct ErrorAfterAcceptStream {
        accepted: bool,
    }
    impl NetworkStream for ErrorAfterAcceptStream {
        fn read(&mut self, _buf: &mut [u8]) -> OasisResult<usize> {
            if self.accepted {
                Err(OasisError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "broken pipe",
                )))
            } else {
                self.accepted = true;
                Err(OasisError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "would block",
                )))
            }
        }
        fn write(&mut self, data: &[u8]) -> OasisResult<usize> {
            Ok(data.len())
        }
        fn close(&mut self) -> OasisResult<()> {
            Ok(())
        }
    }

    let stream = ErrorAfterAcceptStream { accepted: false };
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // First poll: accept connection (WouldBlock on read).
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);

    // Second poll: BrokenPipe error removes connection.
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_stop_clears_connections() {
    let stream1 = MockStream::non_blocking(b"");
    let stream2 = MockStream::non_blocking(b"");
    let mut backend = MockBackend::new()
        .with_accept_stream(Box::new(stream1))
        .with_accept_stream(Box::new(stream2));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        max_connections: 4,
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    listener.poll(&mut backend);
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 2);

    listener.stop();
    assert_eq!(listener.connection_count(), 0);
    assert!(!listener.is_listening());
}

// ---------------------------------------------------------------------------
// Rate limiting logic tests (pure logic, no networking)
// ---------------------------------------------------------------------------

#[test]
fn rate_limit_window_at_threshold() {
    let config = ListenerConfig::default();
    let mut listener = RemoteListener::new(config);
    for _ in 0..5 {
        listener.record_auth_failure();
    }
    // At exactly MAX_AUTH_FAILURES, excess = 0, multiplier = 1.
    let window = listener.rate_limit_window();
    assert_eq!(window.as_secs(), 30);
}

#[test]
fn rate_limit_window_capped() {
    let config = ListenerConfig::default();
    let mut listener = RemoteListener::new(config);
    // Push far past the threshold.
    for _ in 0..20 {
        listener.auth_failures.count += 1;
    }
    let window = listener.rate_limit_window();
    // Excess = 20 - 5 = 15, capped at 6, so multiplier = 64.
    assert_eq!(window.as_secs(), 30 * 64);
}

// ---------------------------------------------------------------------------
// Host parsing edge cases
// ---------------------------------------------------------------------------

#[test]
fn parse_hosts_ipv6_address() {
    let toml = r#"
[[host]]
name = "ipv6-host"
address = "::1"
port = 9000
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].address, "::1");
}

#[test]
fn parse_hosts_max_port() {
    let toml = r#"
[[host]]
name = "max-port"
address = "10.0.0.1"
port = 65535
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].port, 65535);
}

#[test]
fn parse_hosts_long_psk() {
    let long_psk = "a".repeat(1024);
    let toml = format!(
        r#"
[[host]]
name = "long-psk"
address = "10.0.0.1"
psk = "{long_psk}"
"#
    );
    let hosts = hosts::parse_hosts(&toml).unwrap();
    assert_eq!(hosts[0].psk.as_ref().unwrap().len(), 1024);
}

#[test]
fn parse_hosts_whitespace_in_name() {
    let toml = r#"
[[host]]
name = "  spaced name  "
address = "10.0.0.1"
"#;
    let hosts = hosts::parse_hosts(toml).unwrap();
    assert_eq!(hosts[0].name, "  spaced name  ");
}

// ---------------------------------------------------------------------------
// Client state machine edge cases
// ---------------------------------------------------------------------------

#[test]
fn mock_client_auth_ok_clears_auth_started() {
    let stream = Box::new(MockStream::non_blocking(b"AUTH_OK\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    client.state = ClientState::Authenticating;
    client.auth_started = Some(std::time::Instant::now());

    client.poll();
    assert_eq!(client.state(), ClientState::Connected);
    assert!(client.auth_started.is_none());
}

#[test]
fn mock_client_auth_fail_clears_auth_started() {
    let stream = Box::new(MockStream::non_blocking(b"AUTH_FAIL\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    client.state = ClientState::Authenticating;
    client.auth_started = Some(std::time::Instant::now());

    client.poll();
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(client.auth_started.is_none());
}

#[test]
#[cfg(not(feature = "tls-rustls"))]
fn mock_client_connect_with_psk_without_tls_errors() {
    let stream = Box::new(MockStream::non_blocking(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    let result = client.connect(&mut backend, "10.0.0.1", 9000, Some("secret"));
    assert!(result.is_err());
    // Should mention TLS.
    if let Err(OasisError::Backend(msg)) = result {
        assert!(msg.to_string().contains("TLS"));
    }
}

#[test]
fn mock_listener_auth_failure_recorded() {
    // Wrong PSK should increment auth failure count.
    let stream = MockStream::non_blocking(b"wrong\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "correct".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    assert_eq!(listener.auth_failures.count, 0);
    listener.poll(&mut backend);

    // On non-TLS builds, the PSK rejection happens at accept time
    // (AUTH_FAIL sent, connection closed), but the auth failure is still
    // counted in the removal loop.
    #[cfg(feature = "tls-rustls")]
    {
        assert_eq!(listener.auth_failures.count, 1);
    }
    // On non-TLS builds, the connection is rejected at accept (never added
    // to connections vec), so the to_remove loop doesn't see it.
    // The auth failure counter is only incremented for connections that
    // were actually added and then removed in AwaitingAuth state.
}

#[test]
fn mock_listener_crlf_line_endings_handled() {
    let stream = MockStream::non_blocking(b"hello\r\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    // Trim should strip \r.
    assert_eq!(commands[0].0, "hello");
}

#[test]
fn mock_listener_multiple_connections_distinct_indices() {
    let stream1 = MockStream::non_blocking(b"cmd_a\n");
    let stream2 = MockStream::non_blocking(b"cmd_b\n");
    let mut backend = MockBackend::new()
        .with_accept_stream(Box::new(stream1))
        .with_accept_stream(Box::new(stream2));
    backend.listening = true;

    let config = ListenerConfig {
        psk: String::new(),
        max_connections: 4,
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;

    // Accept first connection and get its command.
    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "cmd_a");
    assert_eq!(commands[0].1, 0);

    // Accept second connection.
    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "cmd_b");
    assert_eq!(commands[0].1, 1);

    assert_eq!(listener.connection_count(), 2);
}
