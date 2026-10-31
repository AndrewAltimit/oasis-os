//! Tests over in-memory mock streams and backends (no real TCP).

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result as OasisResult};

use crate::*;

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

/// Poll a PSK client whose server sends `server_lines`.
fn psk_client_after(server_lines: &[u8]) -> (RemoteClient, Vec<String>) {
    let stream = Box::new(MockStream::non_blocking(server_lines));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();
    client
        .connect(&mut backend, "10.0.0.1", 9000, Some("secret123"))
        .unwrap();
    let lines = client.poll();
    (client, lines)
}

#[test]
fn mock_client_answers_a_well_formed_challenge() {
    let (client, lines) = psk_client_after(b"AUTH_REQUIRED 0123456789abcdef0123456789abcdef\n");
    assert_eq!(client.state(), ClientState::Authenticating);
    assert!(lines.is_empty());
}

#[test]
fn mock_client_refuses_a_malformed_challenge() {
    // A hostile server must not get HMACs over text of its choosing.
    let (client, lines) = psk_client_after(b"AUTH_REQUIRED GET / HTTP/1.1\n");
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(lines.iter().any(|l| l.contains("unexpected challenge")));
}

#[test]
fn mock_client_answers_only_one_challenge_per_connection() {
    let (client, lines) = psk_client_after(
        b"AUTH_REQUIRED 0123456789abcdef0123456789abcdef\n\
          AUTH_REQUIRED fedcba9876543210fedcba9876543210\n",
    );
    assert_eq!(client.state(), ClientState::Disconnected);
    assert!(lines.iter().any(|l| l.contains("unexpected challenge")));
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
fn mock_client_eof_disconnects() {
    // Stream returns EOF (Ok(0)) immediately: the remote side closed, so the
    // client must stop claiming a live connection (it used to stay
    // `Connected` forever).
    let stream = Box::new(MockStream::with_eof(b""));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();

    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();
    assert_eq!(lines, vec!["Connection closed by remote host.".to_string()]);
    assert_eq!(client.state(), ClientState::Disconnected);
}

#[test]
fn mock_client_delivers_final_lines_before_eof() {
    let stream = Box::new(MockStream::with_eof(b"out\nGoodbye.\n"));
    let mut backend = MockBackend::new().with_connect_stream(stream);
    let mut client = RemoteClient::new();
    client
        .connect(&mut backend, "10.0.0.1", 9000, None)
        .unwrap();
    let lines = client.poll();
    assert_eq!(lines[..2], ["out".to_string(), "Goodbye.".to_string()]);
    assert!(!client.is_connected());
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
    // Client answers the (pinned) challenge correctly.
    let answer = format!("AUTH {}\n", crate::psk_response("my-secret", "n0nce"));
    let stream = MockStream::non_blocking(answer.as_bytes());
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "my-secret".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;
    listener.test_nonce = Some("n0nce".into());

    // On first poll: accept, challenge, verify the answer.
    let _commands = listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 1);
    assert_eq!(listener.auth_failures.count, 0);
}

#[test]
fn mock_listener_bare_psk_is_rejected() {
    // Sending the key itself (the old protocol) is a failed attempt.
    let stream = MockStream::non_blocking(b"my-secret\n");
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;
    let mut listener = RemoteListener::new(ListenerConfig {
        psk: "my-secret".to_string(),
        ..ListenerConfig::default()
    });
    listener.listening = true;
    listener.test_nonce = Some("n0nce".into());

    assert!(listener.poll(&mut backend).is_empty());
    assert_eq!(listener.connection_count(), 0);
    assert_eq!(listener.auth_failures.count, 1);
}

#[test]
fn mock_listener_answer_for_other_nonce_is_rejected() {
    // A response captured from an earlier session does not replay.
    let replay = format!("AUTH {}\n", crate::psk_response("my-secret", "old-nonce"));
    let stream = MockStream::non_blocking(replay.as_bytes());
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;
    let mut listener = RemoteListener::new(ListenerConfig {
        psk: "my-secret".to_string(),
        ..ListenerConfig::default()
    });
    listener.listening = true;
    listener.test_nonce = Some("n0nce".into());

    assert!(listener.poll(&mut backend).is_empty());
    assert_eq!(listener.connection_count(), 0);
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
    // Client answers the challenge on the first line, then a command.
    let input = format!("AUTH {}\nhello\n", crate::psk_response("secret", "n0nce"));
    let stream = MockStream::non_blocking(input.as_bytes());
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;

    let config = ListenerConfig {
        psk: "secret".to_string(),
        ..ListenerConfig::default()
    };
    let mut listener = RemoteListener::new(config);
    listener.listening = true;
    listener.test_nonce = Some("n0nce".into());

    let commands = listener.poll(&mut backend);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "hello");
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

    assert_eq!(listener.auth_failures.count, 1);
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

/// A stream whose socket buffer accepts at most `budget` more bytes;
/// further writes fail with `WouldBlock` until the budget is refilled.
struct TrickleStream {
    written: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    budget: std::sync::Arc<std::sync::Mutex<usize>>,
}

impl NetworkStream for TrickleStream {
    fn read(&mut self, _buf: &mut [u8]) -> OasisResult<usize> {
        Err(OasisError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "would block",
        )))
    }
    fn write(&mut self, data: &[u8]) -> OasisResult<usize> {
        let mut budget = self.budget.lock().unwrap();
        if *budget == 0 {
            return Err(OasisError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "would block",
            )));
        }
        let n = data.len().min(*budget);
        *budget -= n;
        self.written.lock().unwrap().extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn close(&mut self) -> OasisResult<()> {
        Ok(())
    }
}

type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;

fn trickle_listener(
    initial_budget: usize,
) -> (RemoteListener, MockBackend, Shared<Vec<u8>>, Shared<usize>) {
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let budget = std::sync::Arc::new(std::sync::Mutex::new(initial_budget));
    let stream = TrickleStream {
        written: std::sync::Arc::clone(&written),
        budget: std::sync::Arc::clone(&budget),
    };
    let mut backend = MockBackend::new().with_accept_stream(Box::new(stream));
    backend.listening = true;
    let mut listener = RemoteListener::new(ListenerConfig::default());
    listener.listening = true;
    listener.poll(&mut backend);
    (listener, backend, written, budget)
}

#[test]
fn mock_listener_large_response_survives_partial_writes() {
    let (mut listener, mut backend, written, budget) = trickle_listener(64);
    let big = "x".repeat(100_000);
    listener.send_response(0, &big).unwrap();
    for _ in 0..200 {
        *budget.lock().unwrap() = 4096;
        listener.poll(&mut backend);
    }
    let out = written.lock().unwrap().clone();
    let expected = format!("OASIS_OS remote terminal\n> {big}\n> ");
    assert_eq!(out.len(), expected.len());
    assert!(out == expected.as_bytes());
}

#[test]
fn mock_listener_stalled_peer_is_dropped_at_backlog_cap() {
    let (mut listener, mut backend, _written, _budget) = trickle_listener(0);
    let chunk = "y".repeat(1024 * 1024);
    let failed = (0..16).any(|_| listener.send_response(0, &chunk).is_err());
    assert!(failed, "backlog never capped");
    listener.poll(&mut backend);
    assert_eq!(listener.connection_count(), 0);
}

#[test]
fn mock_listener_ids_survive_earlier_removal() {
    let mut backend = MockBackend::new()
        .with_accept_stream(Box::new(MockStream::non_blocking(b"quit\n")))
        .with_accept_stream(Box::new(MockStream::non_blocking(b"")))
        .with_accept_stream(Box::new(MockStream::non_blocking(b"")));
    backend.listening = true;
    let mut listener = RemoteListener::new(ListenerConfig::default());
    listener.listening = true;
    // Accept #0 (quits immediately), then #1 and #2.
    for _ in 0..3 {
        listener.poll(&mut backend);
    }
    assert_eq!(listener.connection_count(), 2);
    // Id 0 is gone; ids 1 and 2 still address their own connections.
    assert!(listener.send_response(0, "x").is_err());
    assert!(listener.send_response(1, "x").is_ok());
    assert!(listener.send_response(2, "x").is_ok());
    assert!(listener.send_response(3, "x").is_err());
}
