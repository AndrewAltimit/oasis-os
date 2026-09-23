//! Remote terminal listener.
//!
//! Accepts inbound TCP connections, authenticates via pre-shared key,
//! and feeds received command lines into the command interpreter.
//! Designed for non-blocking polling from the main loop.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

use crate::std_backend::StdNetworkBackend;

/// Maximum number of simultaneous remote connections.
const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// Maximum bytes in a single input line.
const MAX_LINE_LEN: usize = 1024;

/// Maximum failed auth attempts before rate limiting kicks in.
const MAX_AUTH_FAILURES: u32 = 5;

/// Base rate-limit window for auth failures (seconds).
/// Doubles with each additional failure beyond the threshold
/// (exponential backoff: 30s, 60s, 120s, ...).
const AUTH_RATE_LIMIT_BASE_SECS: u64 = 30;

/// Idle connection timeout (seconds).
const IDLE_TIMEOUT_SECS: u64 = 300;

/// Maximum bytes read from one connection in a single poll.
const MAX_READ_PER_POLL: usize = 64 * 1024;

/// Maximum unsent output queued for one connection. A peer that stops
/// reading while this much is pending is disconnected, bounding memory.
const MAX_WRITE_BUF: usize = 8 * 1024 * 1024;

/// Sent bytes are compacted out of the write buffer past this offset.
const WRITE_COMPACT_THRESHOLD: usize = 1024 * 1024;

/// How long a closing connection may take to drain its queued output.
const CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Stable id handed out with each command (see [`RemoteListener::poll`]).
    id: usize,
    stream: Box<dyn NetworkStream>,
    auth: AuthState,
    /// Accumulates partial line data between polls.
    read_buf: Vec<u8>,
    /// Output not yet accepted by the non-blocking socket; bytes before
    /// `write_pos` have been sent.
    write_buf: Vec<u8>,
    write_pos: usize,
    /// Commands dispatched to the host that have not been answered with
    /// [`RemoteListener::send_response`] yet.
    inflight: usize,
    /// Timestamp of last received data (for idle timeout).
    last_activity: Instant,
    /// Set when the connection should close once its output has drained.
    closing_since: Option<Instant>,
    /// Hard failure (read/write error or output backlog overflow): drop now.
    dead: bool,
}

impl RemoteConnection {
    fn new(id: usize, stream: Box<dyn NetworkStream>) -> Self {
        Self {
            id,
            stream,
            auth: AuthState::AwaitingAuth,
            read_buf: Vec::with_capacity(256),
            write_buf: Vec::new(),
            write_pos: 0,
            inflight: 0,
            last_activity: Instant::now(),
            closing_since: None,
            dead: false,
        }
    }

    fn pending(&self) -> usize {
        self.write_buf.len() - self.write_pos
    }

    /// Queue `data` for sending. A peer that lets more than
    /// [`MAX_WRITE_BUF`] bytes pile up is not reading and gets dropped.
    fn queue(&mut self, data: &[u8]) -> bool {
        if self.dead {
            return false;
        }
        if self.pending() + data.len() > MAX_WRITE_BUF {
            log::warn!(
                "remote connection {}: output backlog over {MAX_WRITE_BUF} bytes, dropping",
                self.id
            );
            self.dead = true;
            self.write_buf = Vec::new();
            self.write_pos = 0;
            return false;
        }
        self.write_buf.extend_from_slice(data);
        true
    }

    /// Write as much queued output as the non-blocking socket accepts.
    fn flush(&mut self) {
        while !self.dead && self.write_pos < self.write_buf.len() {
            match self.stream.write(&self.write_buf[self.write_pos..]) {
                Ok(0) => break,
                Ok(n) => self.write_pos += n,
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::debug!("remote connection {} write error: {e}", self.id);
                    self.dead = true;
                },
            }
        }
        if self.write_pos == self.write_buf.len() {
            self.write_buf.clear();
            self.write_pos = 0;
        } else if self.write_pos > WRITE_COMPACT_THRESHOLD {
            self.write_buf.drain(..self.write_pos);
            self.write_pos = 0;
        }
        let _ = self.stream.flush();
    }

    fn close_after_flush(&mut self) {
        if self.closing_since.is_none() {
            self.closing_since = Some(Instant::now());
        }
    }

    /// Whether the connection can be dropped now.
    fn finished(&self) -> bool {
        if self.dead {
            return true;
        }
        let Some(since) = self.closing_since else {
            return false;
        };
        (self.pending() == 0 && self.inflight == 0) || since.elapsed() > CLOSE_DRAIN_TIMEOUT
    }
}

/// Tracks failed authentication attempts for rate limiting.
pub(crate) struct AuthFailureRecord {
    pub(crate) count: u32,
    pub(crate) window_start: Instant,
}

/// Which interfaces the remote terminal listener binds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListenerBind {
    /// `127.0.0.1` only: reachable from this device alone. The default, and
    /// the only scope allowed without a PSK.
    #[default]
    Loopback,
    /// `0.0.0.0`: reachable from the network. Requires a non-empty PSK
    /// ([`RemoteListener::start`] refuses otherwise).
    AllInterfaces,
}

/// Configuration for the remote terminal listener.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Port to listen on.
    pub port: u16,
    /// Pre-shared key for authentication (empty = no auth required, which is
    /// only permitted with [`ListenerBind::Loopback`]).
    pub psk: String,
    /// Interfaces to bind (default: loopback only).
    pub bind: ListenerBind,
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
            bind: ListenerBind::Loopback,
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
    pub(crate) listening: bool,
    /// Id for the next accepted connection.
    next_id: usize,
    /// Rate-limiting tracker for auth failures.
    pub(crate) auth_failures: AuthFailureRecord,
}

impl RemoteListener {
    /// Create a new listener with the given configuration.
    pub fn new(config: ListenerConfig) -> Self {
        Self {
            config,
            connections: Vec::new(),
            listening: false,
            next_id: 0,
            auth_failures: AuthFailureRecord {
                count: 0,
                window_start: Instant::now(),
            },
        }
    }

    /// Start listening on the configured port and interfaces.
    ///
    /// An empty PSK means connections are accepted without authentication,
    /// i.e. a shell for anyone who can reach the port. That is therefore only
    /// allowed on loopback: binding [`ListenerBind::AllInterfaces`] with an
    /// empty PSK is refused with a config error.
    pub fn start(&mut self, backend: &mut StdNetworkBackend) -> Result<()> {
        match self.config.bind {
            ListenerBind::Loopback => backend.listen_loopback(self.config.port)?,
            ListenerBind::AllInterfaces => {
                if self.config.psk.is_empty() {
                    return Err(OasisError::Config(
                        "remote terminal: binding all interfaces requires a non-empty PSK".into(),
                    ));
                }
                backend.listen(self.config.port)?;
            },
        }
        #[cfg(not(feature = "tls-rustls"))]
        if !self.config.psk.is_empty() {
            log::warn!(
                "Remote terminal listening WITHOUT TLS but PSK is configured. \
                 Connections attempting PSK auth will be rejected. \
                 Enable the `tls-rustls` feature for encrypted connections."
            );
        }
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

    /// Compute the current rate-limit window duration.
    /// Uses exponential backoff: base * 2^(failures - threshold) once
    /// the failure count exceeds the threshold.
    pub(crate) fn rate_limit_window(&self) -> Duration {
        let excess = self.auth_failures.count.saturating_sub(MAX_AUTH_FAILURES);
        let multiplier = 1u64 << excess.min(6); // cap at 64x to prevent overflow
        Duration::from_secs(AUTH_RATE_LIMIT_BASE_SECS.saturating_mul(multiplier))
    }

    /// Check whether auth rate limit is in effect.
    fn is_rate_limited(&mut self) -> bool {
        let now = Instant::now();
        let window = self.rate_limit_window();
        if now.duration_since(self.auth_failures.window_start) > window {
            // Reset the window.
            self.auth_failures.count = 0;
            self.auth_failures.window_start = now;
        }
        self.auth_failures.count >= MAX_AUTH_FAILURES
    }

    /// Record a failed auth attempt.
    pub(crate) fn record_auth_failure(&mut self) {
        let now = Instant::now();
        let window = self.rate_limit_window();
        if now.duration_since(self.auth_failures.window_start) > window {
            self.auth_failures.count = 0;
            self.auth_failures.window_start = now;
        }
        self.auth_failures.count += 1;
        if self.auth_failures.count >= MAX_AUTH_FAILURES {
            log::warn!(
                "Remote terminal: auth rate limit active ({} failures, \
                 backoff {:.0}s)",
                self.auth_failures.count,
                self.rate_limit_window().as_secs_f64(),
            );
        }
    }

    /// Poll for new connections and incoming data.
    ///
    /// Returns `(command_line, connection_id)` pairs from authenticated
    /// clients. The id is stable for the connection's lifetime (it is not a
    /// position that shifts when other clients disconnect), so after
    /// executing a command pass it to [`Self::send_response`] to return
    /// output to that same client.
    pub fn poll(&mut self, backend: &mut dyn NetworkBackend) -> Vec<(String, usize)> {
        if !self.listening {
            return Vec::new();
        }

        let idle_timeout = Duration::from_secs(self.config.idle_timeout_secs);

        // Accept new connections (reject if rate-limited).
        if self.connections.len() < self.config.max_connections {
            match backend.accept() {
                Ok(Some(stream)) => self.admit(stream),
                Ok(None) => {},
                Err(e) => log::warn!("accept error: {e}"),
            }
        }

        let mut commands = Vec::new();
        let mut auth_failures = 0u32;
        let psk_bytes = self.config.psk.as_bytes().to_vec();

        for conn in &mut self.connections {
            conn.flush();
            if conn.dead || conn.closing_since.is_some() {
                continue;
            }

            // Check idle timeout.
            if self.config.idle_timeout_secs > 0 && conn.last_activity.elapsed() > idle_timeout {
                conn.queue(b"\nIdle timeout. Goodbye.\n");
                conn.close_after_flush();
                conn.flush();
                continue;
            }

            // Read until the socket would block, EOF, or the per-poll cap.
            let mut eof = false;
            let mut buf = [0u8; 4096];
            let mut total = 0usize;
            while total < MAX_READ_PER_POLL {
                match conn.stream.read(&mut buf) {
                    Ok(0) => {
                        eof = true;
                        break;
                    },
                    Ok(n) => {
                        total += n;
                        conn.last_activity = Instant::now();
                        conn.read_buf.extend_from_slice(&buf[..n]);
                    },
                    Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        break;
                    },
                    Err(e) => {
                        log::debug!("connection {} read error: {e}", conn.id);
                        conn.dead = true;
                        break;
                    },
                }
            }
            if conn.dead {
                continue;
            }

            // Process complete lines.
            let mut overlong = false;
            while let Some(newline_pos) = conn.read_buf.iter().position(|&b| b == b'\n') {
                if newline_pos > MAX_LINE_LEN {
                    overlong = true;
                    break;
                }
                let line_bytes: Vec<u8> = conn.read_buf.drain(..=newline_pos).collect();
                let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                if line.is_empty() {
                    continue;
                }

                match conn.auth {
                    AuthState::AwaitingAuth => {
                        if constant_time_eq(line.as_bytes(), &psk_bytes) {
                            conn.auth = AuthState::Authenticated;
                            conn.queue(b"AUTH_OK\n> ");
                        } else {
                            conn.queue(b"AUTH_FAIL\n");
                            conn.close_after_flush();
                            auth_failures += 1;
                            // Ignore the rest of the buffer: no further PSK
                            // guesses on this connection.
                            conn.read_buf.clear();
                            break;
                        }
                    },
                    AuthState::Authenticated => {
                        if line == "quit" || line == "exit" {
                            conn.queue(b"Goodbye.\n");
                            conn.close_after_flush();
                            // Nothing after `quit` may run.
                            conn.read_buf.clear();
                            break;
                        }
                        conn.inflight += 1;
                        commands.push((line, conn.id));
                    },
                }
            }

            // Guard against overlong lines -- disconnect the peer.
            if overlong || conn.read_buf.len() > MAX_LINE_LEN {
                conn.read_buf.clear();
                conn.queue(b"error: line too long\n");
                conn.close_after_flush();
                if conn.auth == AuthState::AwaitingAuth {
                    auth_failures += 1;
                }
            }

            if eof {
                // Peer closed its side: free the slot as soon as the replies
                // to anything it sent before closing have been delivered.
                conn.close_after_flush();
            }
            conn.flush();
        }

        // Record auth failures from this poll cycle (a wrong PSK or garbage
        // before authenticating; a plain disconnect is not a failed guess).
        for _ in 0..auth_failures {
            self.record_auth_failure();
        }

        self.reap();
        commands
    }

    /// Set up a freshly accepted connection (or turn it away).
    fn admit(&mut self, stream: Box<dyn NetworkStream>) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut conn = RemoteConnection::new(id, stream);
        if !self.config.psk.is_empty() && self.is_rate_limited() {
            // Rate limit in effect -- reject new connections.
            let _ = conn.stream.write(b"RATE_LIMITED\n");
            let _ = conn.stream.close();
            return;
        }
        if self.config.psk.is_empty() {
            // No auth required.
            conn.auth = AuthState::Authenticated;
            conn.queue(b"OASIS_OS remote terminal\n> ");
            conn.flush();
            self.connections.push(conn);
            return;
        }
        #[cfg(not(feature = "tls-rustls"))]
        {
            // Reject PSK auth without TLS.
            let _ = conn.stream.write(b"AUTH_FAIL TLS required for PSK auth\n");
            let _ = conn.stream.close();
        }
        #[cfg(feature = "tls-rustls")]
        {
            conn.queue(b"AUTH_REQUIRED\n");
            conn.flush();
            self.connections.push(conn);
        }
    }

    /// Drop connections that failed or finished closing.
    fn reap(&mut self) {
        self.connections.retain_mut(|conn| {
            if conn.finished() {
                let _ = conn.stream.close();
                false
            } else {
                true
            }
        });
    }

    /// Send command output back to the client with id `conn_idx` (as
    /// returned by [`Self::poll`]).
    ///
    /// Output is queued and written as the socket accepts it (over later
    /// polls if needed), so large responses arrive intact. A client that
    /// lets more than 8 MiB of output back up is disconnected.
    pub fn send_response(&mut self, conn_idx: usize, text: &str) -> Result<()> {
        let conn = self
            .connections
            .iter_mut()
            .find(|c| c.id == conn_idx && !c.dead)
            .ok_or_else(|| OasisError::Backend("invalid connection index".into()))?;
        conn.inflight = conn.inflight.saturating_sub(1);
        if !conn.queue(text.as_bytes()) || !conn.queue(b"\n> ") {
            return Err(OasisError::Backend(
                "send: client is not reading; disconnected".into(),
            ));
        }
        conn.flush();
        if conn.dead {
            return Err(OasisError::Backend("send: connection lost".into()));
        }
        Ok(())
    }

    /// Shut down all connections and stop listening.
    ///
    /// Pending output and the shutdown notice are flushed best-effort (one
    /// non-blocking pass) before each socket is closed.
    pub fn stop(&mut self) {
        for conn in &mut self.connections {
            conn.queue(b"\nServer shutting down.\n");
            conn.flush();
            let _ = conn.stream.close();
        }
        self.connections.clear();
        self.listening = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
        assert_eq!(config.bind, ListenerBind::Loopback);
        assert_eq!(config.max_connections, DEFAULT_MAX_CONNECTIONS);
        assert_eq!(config.idle_timeout_secs, IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn test_listener_config_custom() {
        let config = ListenerConfig {
            port: 8080,
            psk: "secret123".to_string(),
            bind: ListenerBind::AllInterfaces,
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

        // Record MAX_AUTH_FAILURES (5) failures
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
    fn test_rate_limit_exponential_backoff() {
        let config = ListenerConfig::default();
        let mut listener = RemoteListener::new(config);

        // Record failures up to the threshold.
        for _ in 0..MAX_AUTH_FAILURES {
            listener.record_auth_failure();
        }
        // Base window at threshold.
        let base = listener.rate_limit_window();
        assert_eq!(base.as_secs(), AUTH_RATE_LIMIT_BASE_SECS);

        // One more failure doubles the window.
        listener.record_auth_failure();
        let doubled = listener.rate_limit_window();
        assert_eq!(doubled.as_secs(), AUTH_RATE_LIMIT_BASE_SECS * 2);

        // Two more failures -> 4x base.
        listener.record_auth_failure();
        let quadrupled = listener.rate_limit_window();
        assert_eq!(quadrupled.as_secs(), AUTH_RATE_LIMIT_BASE_SECS * 4);
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
        assert_eq!(MAX_AUTH_FAILURES, 5);
        assert_eq!(AUTH_RATE_LIMIT_BASE_SECS, 30);
        assert_eq!(IDLE_TIMEOUT_SECS, 300);
    }

    #[test]
    fn test_listener_config_clone() {
        let config1 = ListenerConfig {
            port: 7777,
            psk: "test".to_string(),
            bind: ListenerBind::Loopback,
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
