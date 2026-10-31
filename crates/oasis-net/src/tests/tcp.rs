//! Tests over real loopback TCP sockets.

use std::io::{Read, Write};
use std::net::TcpStream;

use oasis_types::backend::NetworkBackend;

use crate::*;

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
            if let Some(p) = free_port()
                && backend.listen(p).is_ok()
            {
                bound = Some(p);
                break;
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

#[test]
fn connect_resolves_hostname() {
    // `localhost` must go through name resolution (it may resolve to ::1
    // first, which is refused, before falling back to 127.0.0.1).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let _ = listener.accept().unwrap();
    });
    let mut backend = StdNetworkBackend::new();
    let mut stream = backend.connect("localhost", port).unwrap();
    stream.close().unwrap();
    handle.join().unwrap();
}

#[test]
fn connect_unresolvable_host_errors() {
    let mut backend = StdNetworkBackend::new();
    assert!(backend.connect("does-not-exist.invalid", 80).is_err());
}

#[test]
fn connect_times_out_instead_of_blocking() {
    // 10.255.255.1 is a non-routable address: SYNs go unanswered, so a
    // blocking connect would hang for the OS default (often 20s+). On hosts
    // without a route it errors immediately instead; either way the call
    // must return promptly with an error.
    let mut backend = StdNetworkBackend::new();
    backend.set_connect_timeout(std::time::Duration::from_millis(300));
    let start = std::time::Instant::now();
    let result = backend.connect("10.255.255.1", 9);
    assert!(result.is_err());
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "connect took {:?}",
        start.elapsed()
    );
}

#[test]
fn listener_without_psk_binds_loopback_only() {
    let mut backend = StdNetworkBackend::new();
    let (_port, listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: String::new(),
            ..ListenerConfig::default()
        },
        &mut backend,
    );
    assert!(listener.is_listening());
    let addr = backend.local_addr().unwrap();
    assert!(addr.ip().is_loopback(), "bound to {addr}");
}

#[test]
fn listener_all_interfaces_requires_psk() {
    let mut backend = StdNetworkBackend::new();
    let mut listener = RemoteListener::new(ListenerConfig {
        port: free_port().unwrap_or(0),
        psk: String::new(),
        bind: ListenerBind::AllInterfaces,
        ..ListenerConfig::default()
    });
    let err = listener.start(&mut backend).unwrap_err();
    assert!(err.to_string().contains("PSK"), "{err}");
    assert!(!listener.is_listening());
    assert!(backend.local_addr().is_none(), "nothing may be bound");
}

#[test]
fn listener_all_interfaces_with_psk_binds_unspecified() {
    let mut backend = StdNetworkBackend::new();
    let (_port, listener) = bind_listener_retry(
        |port| ListenerConfig {
            port,
            psk: "secret".to_string(),
            bind: ListenerBind::AllInterfaces,
            ..ListenerConfig::default()
        },
        &mut backend,
    );
    assert!(listener.is_listening());
    assert!(backend.local_addr().unwrap().ip().is_unspecified());
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

    // Read the challenge.
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).unwrap();
    let msg = String::from_utf8_lossy(&buf[..n]).into_owned();
    let nonce = msg
        .strip_prefix("AUTH_REQUIRED ")
        .and_then(|r| r.strip_suffix('\n'))
        .unwrap();
    assert!(!msg.contains("secret123"));

    // A bare PSK is no longer an accepted answer; send the HMAC.
    let answer = format!("AUTH {}\n", crate::psk_response("secret123", nonce));
    client.write_all(answer.as_bytes()).unwrap();
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
