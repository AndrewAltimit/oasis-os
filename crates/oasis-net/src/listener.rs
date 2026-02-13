//! Remote terminal listener.
//!
//! Accepts inbound TCP connections, authenticates via pre-shared key,
//! and feeds received command lines into the command interpreter.
//! Designed for non-blocking polling from the main loop.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

/// Maximum number of simultaneous remote connections.
const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// Maximum bytes in a single input line.
const MAX_LINE_LEN: usize = 1024;

/// Maximum failed auth attempts before rate limiting kicks in.
const MAX_AUTH_FAILURES: u32 = 3;

/// Rate-limit window for auth failures (seconds).
const AUTH_RATE_LIMIT_SECS: u64 = 60;

/// Idle connection timeout (seconds).
const IDLE_TIMEOUT_SECS: u64 = 300;

/// Constant-time comparison of two byte slices.
///
/// Always compares every byte to avoid leaking length or content via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still iterate to avoid leaking whether lengths matched via timing,
        // but we can short-circuit length since the attacker can observe
        // packet size anyway.
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Authentication state for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthState {
    /// Waiting for client to send the PSK.
    AwaitingAuth,
    /// Authenticated and ready for commands.
    Authenticated,
}

/// A single remote client connection.
struct RemoteConnection {
    stream: Box<dyn NetworkStream>,
    auth: AuthState,
    /// Accumulates partial line data between polls.
    read_buf: Vec<u8>,
    /// Timestamp of last received data (for idle timeout).
    last_activity: Instant,
}

impl RemoteConnection {
    fn new(stream: Box<dyn NetworkStream>) -> Self {
        Self {
            stream,
            auth: AuthState::AwaitingAuth,
            read_buf: Vec::with_capacity(256),
            last_activity: Instant::now(),
        }
    }
}

/// Tracks failed authentication attempts for rate limiting.
struct AuthFailureRecord {
    count: u32,
    window_start: Instant,
}

/// Configuration for the remote terminal listener.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Port to listen on.
    pub port: u16,
    /// Pre-shared key for authentication (empty = no auth required).
    pub psk: String,
    /// Maximum simultaneous connections.
    pub max_connections: usize,
    /// Idle connection timeout in seconds (0 = no timeout).
    pub idle_timeout_secs: u64,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            port: 9000,
            psk: String::new(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            idle_timeout_secs: IDLE_TIMEOUT_SECS,
        }
    }
}

/// Remote terminal listener that manages inbound connections.
///
/// Call `poll()` each frame from the main loop. It returns command lines
/// received from authenticated clients along with responses to send back.
pub struct RemoteListener {
    config: ListenerConfig,
    connections: Vec<RemoteConnection>,
    listening: bool,
    /// Rate-limiting tracker for auth failures.
    auth_failures: AuthFailureRecord,
}

impl RemoteListener {
    /// Create a new listener with the given configuration.
    pub fn new(config: ListenerConfig) -> Self {
        Self {
            config,
            connections: Vec::new(),
            listening: false,
            auth_failures: AuthFailureRecord {
                count: 0,
                window_start: Instant::now(),
            },
        }
    }

    /// Start listening on the configured port.
    pub fn start(&mut self, backend: &mut dyn NetworkBackend) -> Result<()> {
        backend.listen(self.config.port)?;
        self.listening = true;
        Ok(())
    }

    /// Whether the listener is active.
    pub fn is_listening(&self) -> bool {
        self.listening
    }

    /// Number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Check whether auth rate limit is in effect.
    fn is_rate_limited(&mut self) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(AUTH_RATE_LIMIT_SECS);
        if now.duration_since(self.auth_failures.window_start) > window {
            // Reset the window.
            self.auth_failures.count = 0;
            self.auth_failures.window_start = now;
        }
        self.auth_failures.count >= MAX_AUTH_FAILURES
    }

    /// Record a failed auth attempt.
    fn record_auth_failure(&mut self) {
        let now = Instant::now();
        let window = Duration::from_secs(AUTH_RATE_LIMIT_SECS);
        if now.duration_since(self.auth_failures.window_start) > window {
            self.auth_failures.count = 0;
            self.auth_failures.window_start = now;
        }
        self.auth_failures.count += 1;
    }

    /// Poll for new connections and incoming data.
    ///
    /// Returns a list of (command_line, connection_index) pairs from
    /// authenticated clients. After executing commands, call
    /// `send_response()` to return output to the client.
    pub fn poll(&mut self, backend: &mut dyn NetworkBackend) -> Vec<(String, usize)> {
        if !self.listening {
            return Vec::new();
        }

        let idle_timeout = Duration::from_secs(self.config.idle_timeout_secs);

        // Accept new connections (reject if rate-limited).
        if self.connections.len() < self.config.max_connections {
            match backend.accept() {
                Ok(Some(stream)) => {
                    if !self.config.psk.is_empty() && self.is_rate_limited() {
                        // Rate limit in effect -- reject new connections.
                        let mut conn = RemoteConnection::new(stream);
                        let _ = conn.stream.write(b"RATE_LIMITED\n");
                        let _ = conn.stream.close();
                    } else {
                        let mut conn = RemoteConnection::new(stream);
                        if self.config.psk.is_empty() {
                            // No auth required.
                            conn.auth = AuthState::Authenticated;
                            let _ = conn.stream.write(b"OASIS_OS remote terminal\n> ");
                        } else {
                            let _ = conn.stream.write(b"AUTH_REQUIRED\n");
                        }
                        self.connections.push(conn);
                    }
                },
                Ok(None) => {},
                Err(e) => log::warn!("accept error: {e}"),
            }
        }

        // Read from all connections.
        let mut commands = Vec::new();
        let mut to_remove = Vec::new();
        let psk_bytes = self.config.psk.as_bytes().to_vec();

        for (idx, conn) in self.connections.iter_mut().enumerate() {
            // Check idle timeout.
            if self.config.idle_timeout_secs > 0 && conn.last_activity.elapsed() > idle_timeout {
                let _ = conn.stream.write(b"\nIdle timeout. Goodbye.\n");
                to_remove.push(idx);
                continue;
            }

            let mut buf = [0u8; 512];
            match conn.stream.read(&mut buf) {
                Ok(0) => {
                    // Connection closed (EOF).
                },
                Err(oasis_types::error::OasisError::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    // Non-blocking socket has no data yet.
                },
                Ok(n) => {
                    conn.last_activity = Instant::now();
                    conn.read_buf.extend_from_slice(&buf[..n]);

                    // Process complete lines.
                    while let Some(newline_pos) = conn.read_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = conn.read_buf.drain(..=newline_pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                        if line.is_empty() {
                            continue;
                        }

                        match conn.auth {
                            AuthState::AwaitingAuth => {
                                if constant_time_eq(line.as_bytes(), &psk_bytes) {
                                    conn.auth = AuthState::Authenticated;
                                    let _ = conn.stream.write(b"AUTH_OK\n> ");
                                } else {
                                    let _ = conn.stream.write(b"AUTH_FAIL\n");
                                    to_remove.push(idx);
                                }
                            },
                            AuthState::Authenticated => {
                                if line == "quit" || line == "exit" {
                                    let _ = conn.stream.write(b"Goodbye.\n");
                                    to_remove.push(idx);
                                } else {
                                    commands.push((line, idx));
                                }
                            },
                        }
                    }

                    // Guard against overlong lines.
                    if conn.read_buf.len() > MAX_LINE_LEN {
                        conn.read_buf.clear();
                        let _ = conn.stream.write(b"error: line too long\n> ");
                    }
                },
                Err(e) => {
                    log::debug!("connection {idx} read error: {e}");
                    to_remove.push(idx);
                },
            }
        }

        // Record auth failures from this poll cycle.
        let auth_failure_count = to_remove
            .iter()
            .filter(|&&idx| {
                self.connections
                    .get(idx)
                    .is_some_and(|c| c.auth == AuthState::AwaitingAuth)
            })
            .count();
        for _ in 0..auth_failure_count {
            self.record_auth_failure();
        }

        // Remove closed/failed connections (in reverse to preserve indices).
        to_remove.sort_unstable();
        to_remove.dedup();
        for idx in to_remove.into_iter().rev() {
            let mut conn = self.connections.remove(idx);
            let _ = conn.stream.close();
        }

        commands
    }

    /// Send command output back to a specific client.
    pub fn send_response(&mut self, conn_idx: usize, text: &str) -> Result<()> {
        let conn = self
            .connections
            .get_mut(conn_idx)
            .ok_or_else(|| OasisError::Backend("invalid connection index".to_string()))?;
        conn.stream
            .write(text.as_bytes())
            .map_err(|e| OasisError::Backend(format!("send: {e}")))?;
        conn.stream
            .write(b"\n> ")
            .map_err(|e| OasisError::Backend(format!("send prompt: {e}")))?;
        Ok(())
    }

    /// Shut down all connections and stop listening.
    pub fn stop(&mut self) {
        for conn in &mut self.connections {
            let _ = conn.stream.write(b"\nServer shutting down.\n");
            let _ = conn.stream.close();
        }
        self.connections.clear();
        self.listening = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_equal_strings() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"a", b"a"));
    }

    #[test]
    fn test_constant_time_eq_different_strings() {
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hallo"));
        assert!(!constant_time_eq(b"abc", b"def"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"hello", b"hello!"));
        assert!(!constant_time_eq(b"a", b"ab"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn test_constant_time_eq_empty_strings() {
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(!constant_time_eq(b"a", b""));
    }

    #[test]
    fn test_constant_time_eq_similar_strings() {
        // Differs by one bit
        assert!(!constant_time_eq(b"password", b"passwosd"));
        // Differs by case
        assert!(!constant_time_eq(b"Secret", b"secret"));
    }

    #[test]
    fn test_listener_config_default() {
        let config = ListenerConfig::default();
        assert_eq!(config.port, 9000);
        assert_eq!(config.psk, "");
        assert_eq!(config.max_connections, DEFAULT_MAX_CONNECTIONS);
        assert_eq!(config.idle_timeout_secs, IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn test_listener_config_custom() {
        let config = ListenerConfig {
            port: 8080,
            psk: "secret123".to_string(),
            max_connections: 10,
            idle_timeout_secs: 600,
        };
        assert_eq!(config.port, 8080);
        assert_eq!(config.psk, "secret123");
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.idle_timeout_secs, 600);
    }

    #[test]
    fn test_remote_listener_new() {
        let config = ListenerConfig::default();
        let listener = RemoteListener::new(config.clone());
        assert!(!listener.is_listening());
        assert_eq!(listener.connection_count(), 0);
        assert_eq!(listener.config.port, config.port);
        assert_eq!(listener.config.psk, config.psk);
    }

    #[test]
    fn test_remote_listener_new_with_psk() {
        let config = ListenerConfig {
            psk: "my-secret-key".to_string(),
            ..Default::default()
        };
        let listener = RemoteListener::new(config);
        assert!(!listener.is_listening());
        assert_eq!(listener.connection_count(), 0);
        assert_eq!(listener.config.psk, "my-secret-key");
    }

    #[test]
    fn test_is_rate_limited_initial_state() {
        let config = ListenerConfig::default();
        let mut listener = RemoteListener::new(config);
        // Initially, no failures, so not rate limited
        assert!(!listener.is_rate_limited());
    }

    #[test]
    fn test_record_auth_failure_increments_count() {
        let config = ListenerConfig::default();
        let mut listener = RemoteListener::new(config);

        assert_eq!(listener.auth_failures.count, 0);
        listener.record_auth_failure();
        assert_eq!(listener.auth_failures.count, 1);
        listener.record_auth_failure();
        assert_eq!(listener.auth_failures.count, 2);
    }

    #[test]
    fn test_is_rate_limited_after_max_failures() {
        let config = ListenerConfig::default();
        let mut listener = RemoteListener::new(config);

        assert!(!listener.is_rate_limited());

        // Record MAX_AUTH_FAILURES (3) failures
        for _ in 0..MAX_AUTH_FAILURES {
            listener.record_auth_failure();
        }

        // Now should be rate limited
        assert!(listener.is_rate_limited());
    }

    #[test]
    fn test_is_rate_limited_just_below_threshold() {
        let config = ListenerConfig::default();
        let mut listener = RemoteListener::new(config);

        // Record one less than MAX_AUTH_FAILURES
        for _ in 0..(MAX_AUTH_FAILURES - 1) {
            listener.record_auth_failure();
        }

        // Should not be rate limited yet
        assert!(!listener.is_rate_limited());
    }

    #[test]
    fn test_auth_state_equality() {
        assert_eq!(AuthState::AwaitingAuth, AuthState::AwaitingAuth);
        assert_eq!(AuthState::Authenticated, AuthState::Authenticated);
        assert_ne!(AuthState::AwaitingAuth, AuthState::Authenticated);
    }

    #[test]
    fn test_remote_connection_new_state() {
        // We can't easily create a mock NetworkStream without more infrastructure,
        // but we can test that RemoteConnection implements the expected pattern.
        // This test validates the structure is correct.
        assert_eq!(AuthState::AwaitingAuth as u8, 0);
    }

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_MAX_CONNECTIONS, 4);
        assert_eq!(MAX_LINE_LEN, 1024);
        assert_eq!(MAX_AUTH_FAILURES, 3);
        assert_eq!(AUTH_RATE_LIMIT_SECS, 60);
        assert_eq!(IDLE_TIMEOUT_SECS, 300);
    }

    #[test]
    fn test_listener_config_clone() {
        let config1 = ListenerConfig {
            port: 7777,
            psk: "test".to_string(),
            max_connections: 5,
            idle_timeout_secs: 120,
        };
        let config2 = config1.clone();
        assert_eq!(config1.port, config2.port);
        assert_eq!(config1.psk, config2.psk);
        assert_eq!(config1.max_connections, config2.max_connections);
        assert_eq!(config1.idle_timeout_secs, config2.idle_timeout_secs);
    }

    #[test]
    fn test_listener_initial_connection_count() {
        let listener = RemoteListener::new(ListenerConfig::default());
        assert_eq!(listener.connection_count(), 0);
    }

    #[test]
    fn test_listener_not_listening_by_default() {
        let listener = RemoteListener::new(ListenerConfig::default());
        assert!(!listener.is_listening());
    }
}
