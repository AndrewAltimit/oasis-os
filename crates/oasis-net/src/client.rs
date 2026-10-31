//! Remote terminal outbound client.
//!
//! Connects to a remote OASIS_OS instance (or any TCP text service),
//! sends commands, and receives output. Designed for non-blocking polling.
//! With a PSK it answers the listener's `AUTH_REQUIRED <nonce>` challenge
//! (see [`crate::auth`]); the key itself is never sent. It answers one
//! challenge per connection, and only a well-formed nonce
//! ([`crate::auth::is_valid_nonce`]), so a hostile server cannot use it as
//! an HMAC oracle. The server is not authenticated: see the relay
//! limitation in [`crate::auth`].

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

use crate::auth::{is_valid_nonce, psk_response};

/// Maximum time (in seconds) to wait for an authentication response.
const AUTH_TIMEOUT_SECS: u64 = 30;

/// Maximum bytes in a single input line (guard against unbounded buffer growth).
const MAX_LINE_LEN: usize = 16_384;

/// Maximum bytes read from the socket in a single poll.
const MAX_READ_PER_POLL: usize = 64 * 1024;

/// How long [`write_all`] keeps retrying a socket that would block.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Write all of `data` to a (possibly non-blocking) stream: a short write
/// or `WouldBlock` is retried until everything is sent or
/// [`WRITE_TIMEOUT`] passes, so a command line is never silently cut.
fn write_all(stream: &mut dyn NetworkStream, mut data: &[u8]) -> Result<()> {
    let started = Instant::now();
    while !data.is_empty() {
        match stream.write(data) {
            Ok(0) => return Err(OasisError::Backend("connection closed".into())),
            Ok(n) => data = &data[n..],
            Err(e) if e.is_would_block() => {
                if started.elapsed() > WRITE_TIMEOUT {
                    return Err(OasisError::Backend("write timed out".into()));
                }
                std::thread::yield_now();
            },
            Err(e) => return Err(e),
        }
    }
    let _ = stream.flush();
    Ok(())
}

/// State of the remote client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Not connected.
    Disconnected,
    /// Connected but awaiting authentication response.
    Authenticating,
    /// Connected and authenticated (or no auth needed).
    Connected,
}

/// Outbound remote terminal client.
pub struct RemoteClient {
    pub(crate) stream: Option<Box<dyn NetworkStream>>,
    pub(crate) state: ClientState,
    /// Accumulates received data between polls.
    pub(crate) read_buf: Vec<u8>,
    /// Lines received from the remote side.
    pub(crate) received_lines: Vec<String>,
    /// When authentication was initiated (for timeout detection).
    pub(crate) auth_started: Option<Instant>,
    /// Key used to answer the server's challenge while authenticating.
    psk: Option<String>,
    /// Whether this connection's challenge was already answered.
    challenge_answered: bool,
}

impl RemoteClient {
    /// A disconnected client.
    pub fn new() -> Self {
        Self {
            stream: None,
            state: ClientState::Disconnected,
            read_buf: Vec::with_capacity(256),
            received_lines: Vec::new(),
            auth_started: None,
            psk: None,
            challenge_answered: false,
        }
    }

    /// Current connection state.
    pub fn state(&self) -> ClientState {
        self.state
    }

    /// Connect to a remote host.
    ///
    /// With a `psk` the client waits in [`ClientState::Authenticating`] for
    /// the server's challenge and answers it from [`poll`](Self::poll).
    pub fn connect(
        &mut self,
        backend: &mut dyn NetworkBackend,
        address: &str,
        port: u16,
        psk: Option<&str>,
    ) -> Result<()> {
        let stream = backend.connect(address, port)?;
        self.stream = Some(stream);

        self.read_buf.clear();
        self.challenge_answered = false;
        if let Some(key) = psk {
            self.psk = Some(key.to_string());
            self.state = ClientState::Authenticating;
            self.auth_started = Some(Instant::now());
        } else {
            self.psk = None;
            self.state = ClientState::Connected;
        }

        Ok(())
    }

    /// Send a command line to the remote host.
    pub fn send(&mut self, line: &str) -> Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| OasisError::Backend("not connected".into()))?;
        write_all(stream.as_mut(), format!("{line}\n").as_bytes())
            .map_err(|e| OasisError::Backend(format!("send: {e}").into()))
    }

    /// Poll for received data from the remote host.
    /// Returns new lines received since last poll.
    pub fn poll(&mut self) -> Vec<String> {
        if self.stream.is_none() {
            return Vec::new();
        }

        // Check for authentication timeout.
        if self.state == ClientState::Authenticating
            && let Some(started) = self.auth_started
            && started.elapsed().as_secs() >= AUTH_TIMEOUT_SECS
        {
            self.disconnect();
            self.received_lines
                .push("Authentication timed out.".to_string());
            return std::mem::take(&mut self.received_lines);
        }

        // Read until the socket would block, EOF, or the per-poll cap (one
        // small read per frame made multi-megabyte outputs take minutes).
        let Some(stream) = self.stream.as_mut() else {
            return Vec::new();
        };
        let mut eof = false;
        let mut lost = false;
        let mut buf = [0u8; 4096];
        let mut total = 0usize;
        while total < MAX_READ_PER_POLL {
            match stream.read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                },
                Ok(n) => {
                    total += n;
                    self.read_buf.extend_from_slice(&buf[..n]);
                },
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    lost = true;
                    break;
                },
            }
        }

        // Extract complete lines.
        while let Some(pos) = self.read_buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.read_buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

            // Handle auth responses.
            if self.state == ClientState::Authenticating {
                if let Some(nonce) = line.strip_prefix("AUTH_REQUIRED ") {
                    let nonce = nonce.trim();
                    if self.challenge_answered || !is_valid_nonce(nonce) {
                        self.disconnect();
                        self.received_lines
                            .push("Authentication failed: unexpected challenge.".to_string());
                        self.read_buf.clear();
                        return std::mem::take(&mut self.received_lines);
                    }
                    self.challenge_answered = true;
                    let answer = format!(
                        "AUTH {}\n",
                        psk_response(self.psk.as_deref().unwrap_or(""), nonce)
                    );
                    let sent = self
                        .stream
                        .as_mut()
                        .map(|s| write_all(s.as_mut(), answer.as_bytes()));
                    if !matches!(sent, Some(Ok(()))) {
                        self.disconnect();
                        self.received_lines
                            .push("Authentication failed: could not send.".to_string());
                        return std::mem::take(&mut self.received_lines);
                    }
                    continue;
                }
                if line == "AUTH_OK" {
                    self.state = ClientState::Connected;
                    self.auth_started = None;
                    continue;
                } else if line == "AUTH_FAIL" || line.starts_with("AUTH_FAIL ") {
                    self.auth_started = None;
                    self.disconnect();
                    self.received_lines
                        .push("Authentication failed.".to_string());
                    self.read_buf.clear();
                    return std::mem::take(&mut self.received_lines);
                }
            }

            if !line.is_empty() {
                self.received_lines.push(line);
            }
        }

        // Guard against unbounded buffer growth (no newline received).
        if self.read_buf.len() > MAX_LINE_LEN {
            self.read_buf.clear();
            self.disconnect();
            self.received_lines
                .push("Error: line too long, disconnected.".to_string());
        } else if lost {
            self.disconnect();
            self.received_lines.push("Connection lost.".to_string());
        } else if eof {
            // The remote side closed the connection (quit, shutdown, idle
            // timeout): stop reporting a live connection.
            self.read_buf.clear();
            self.disconnect();
            self.received_lines
                .push("Connection closed by remote host.".to_string());
        }

        std::mem::take(&mut self.received_lines)
    }

    /// Disconnect from the remote host.
    pub fn disconnect(&mut self) {
        if let Some(ref mut stream) = self.stream {
            let _ = stream.write(b"quit\n");
            let _ = stream.close();
        }
        self.stream = None;
        self.state = ClientState::Disconnected;
        self.psk = None;
    }

    /// Whether we are currently connected.
    pub fn is_connected(&self) -> bool {
        self.state != ClientState::Disconnected
    }
}

impl Default for RemoteClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_client_state_equality() {
        assert_eq!(ClientState::Disconnected, ClientState::Disconnected);
        assert_eq!(ClientState::Authenticating, ClientState::Authenticating);
        assert_eq!(ClientState::Connected, ClientState::Connected);
        assert_ne!(ClientState::Disconnected, ClientState::Authenticating);
        assert_ne!(ClientState::Authenticating, ClientState::Connected);
        assert_ne!(ClientState::Disconnected, ClientState::Connected);
    }

    #[test]
    fn test_remote_client_new() {
        let client = RemoteClient::new();
        assert_eq!(client.state(), ClientState::Disconnected);
        assert!(!client.is_connected());
        assert!(client.stream.is_none());
    }

    #[test]
    fn test_remote_client_default() {
        let client = RemoteClient::default();
        assert_eq!(client.state(), ClientState::Disconnected);
        assert!(!client.is_connected());
    }

    #[test]
    fn test_remote_client_initial_state() {
        let client = RemoteClient::new();
        assert_eq!(client.state(), ClientState::Disconnected);
    }

    #[test]
    fn test_remote_client_is_connected_when_disconnected() {
        let client = RemoteClient::new();
        assert!(!client.is_connected());
    }

    #[test]
    fn test_remote_client_disconnect() {
        let mut client = RemoteClient::new();
        // Disconnecting when already disconnected should be safe
        client.disconnect();
        assert_eq!(client.state(), ClientState::Disconnected);
        assert!(!client.is_connected());
        assert!(client.stream.is_none());
    }

    #[test]
    fn test_remote_client_buffer_initialization() {
        let client = RemoteClient::new();
        // read_buf should be initialized with capacity
        assert_eq!(client.read_buf.len(), 0);
        assert_eq!(client.received_lines.len(), 0);
    }

    #[test]
    fn test_remote_client_poll_when_disconnected() {
        let mut client = RemoteClient::new();
        let lines = client.poll();
        assert!(lines.is_empty());
    }

    #[test]
    fn test_client_state_connected_check() {
        // Disconnected is not connected
        assert!(!matches!(ClientState::Disconnected, ClientState::Connected));
        // Authenticating counts as connected (connection established)
        let state = ClientState::Authenticating;
        assert_ne!(state, ClientState::Disconnected);
        // Connected is connected
        let state = ClientState::Connected;
        assert_ne!(state, ClientState::Disconnected);
    }

    #[test]
    fn test_send_when_not_connected() {
        let mut client = RemoteClient::new();
        let result = client.send("test command");
        assert!(result.is_err());
        if let Err(OasisError::Backend(msg)) = result {
            assert_eq!(msg.to_string(), "not connected");
        } else {
            panic!("Expected Backend error with 'not connected' message");
        }
    }

    #[test]
    fn test_client_state_debug() {
        // Ensure Debug trait works for ClientState
        let state = ClientState::Disconnected;
        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("Disconnected"));
    }

    #[test]
    fn test_client_state_clone() {
        let state1 = ClientState::Authenticating;
        let state2 = state1;
        assert_eq!(state1, state2);
    }

    #[test]
    fn test_client_state_copy() {
        let state1 = ClientState::Connected;
        let state2 = state1;
        // Both should be usable after copy
        assert_eq!(state1, ClientState::Connected);
        assert_eq!(state2, ClientState::Connected);
    }

    #[test]
    fn test_remote_client_multiple_disconnects() {
        let mut client = RemoteClient::new();
        client.disconnect();
        client.disconnect();
        client.disconnect();
        // Multiple disconnects should be idempotent
        assert_eq!(client.state(), ClientState::Disconnected);
    }

    #[test]
    fn test_remote_client_state_after_disconnect() {
        let mut client = RemoteClient::new();
        client.disconnect();
        assert_eq!(client.state(), ClientState::Disconnected);
        assert!(!client.is_connected());
        assert!(client.stream.is_none());
    }

    /// Accepts one byte per write and would-block on every other call.
    struct Stingy {
        written: Vec<u8>,
        block_next: bool,
    }

    impl NetworkStream for Stingy {
        fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
            Err(OasisError::Io(std::io::ErrorKind::WouldBlock.into()))
        }
        fn write(&mut self, data: &[u8]) -> Result<usize> {
            self.block_next = !self.block_next;
            if !self.block_next {
                return Err(OasisError::Io(std::io::ErrorKind::WouldBlock.into()));
            }
            self.written.push(data[0]);
            Ok(1)
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_all_survives_short_writes_and_would_block() {
        // `send` used to issue one `write` and drop whatever it did not take.
        let mut s = Stingy {
            written: Vec::new(),
            block_next: false,
        };
        write_all(&mut s, b"ls -la /home\n").unwrap();
        assert_eq!(s.written, b"ls -la /home\n");
    }

    #[test]
    fn test_client_received_lines_empty_initially() {
        let client = RemoteClient::new();
        assert_eq!(client.received_lines.len(), 0);
    }
}
