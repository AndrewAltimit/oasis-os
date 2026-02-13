//! Remote terminal outbound client.
//!
//! Connects to a remote OASIS_OS instance (or any TCP text service),
//! sends commands, and receives output. Designed for non-blocking polling.

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

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
    stream: Option<Box<dyn NetworkStream>>,
    state: ClientState,
    /// Accumulates received data between polls.
    read_buf: Vec<u8>,
    /// Lines received from the remote side.
    received_lines: Vec<String>,
}

impl RemoteClient {
    pub fn new() -> Self {
        Self {
            stream: None,
            state: ClientState::Disconnected,
            read_buf: Vec::with_capacity(256),
            received_lines: Vec::new(),
        }
    }

    /// Current connection state.
    pub fn state(&self) -> ClientState {
        self.state
    }

    /// Connect to a remote host.
    pub fn connect(
        &mut self,
        backend: &mut dyn NetworkBackend,
        address: &str,
        port: u16,
        psk: Option<&str>,
    ) -> Result<()> {
        let stream = backend.connect(address, port)?;
        self.stream = Some(stream);

        if let Some(key) = psk {
            // Send PSK immediately.
            if let Some(ref mut s) = self.stream {
                s.write(format!("{key}\n").as_bytes())
                    .map_err(|e| OasisError::Backend(format!("auth send: {e}")))?;
            }
            self.state = ClientState::Authenticating;
        } else {
            self.state = ClientState::Connected;
        }

        Ok(())
    }

    /// Send a command line to the remote host.
    pub fn send(&mut self, line: &str) -> Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| OasisError::Backend("not connected".to_string()))?;
        stream
            .write(format!("{line}\n").as_bytes())
            .map_err(|e| OasisError::Backend(format!("send: {e}")))?;
        Ok(())
    }

    /// Poll for received data from the remote host.
    /// Returns new lines received since last poll.
    pub fn poll(&mut self) -> Vec<String> {
        let Some(ref mut stream) = self.stream else {
            return Vec::new();
        };

        let mut buf = [0u8; 512];
        match stream.read(&mut buf) {
            Ok(0) => {},
            Ok(n) => {
                self.read_buf.extend_from_slice(&buf[..n]);

                // Extract complete lines.
                while let Some(pos) = self.read_buf.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = self.read_buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                    // Handle auth responses.
                    if self.state == ClientState::Authenticating {
                        if line == "AUTH_OK" {
                            self.state = ClientState::Connected;
                            continue;
                        } else if line == "AUTH_FAIL" {
                            self.disconnect();
                            self.received_lines
                                .push("Authentication failed.".to_string());
                            break;
                        }
                    }

                    if !line.is_empty() {
                        self.received_lines.push(line);
                    }
                }
            },
            Err(oasis_types::error::OasisError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // Non-blocking socket has no data yet.
            },
            Err(_) => {
                // Connection likely dropped.
                self.disconnect();
                self.received_lines.push("Connection lost.".to_string());
            },
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
            assert_eq!(msg, "not connected");
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

    #[test]
    fn test_client_received_lines_empty_initially() {
        let client = RemoteClient::new();
        assert_eq!(client.received_lines.len(), 0);
    }
}
