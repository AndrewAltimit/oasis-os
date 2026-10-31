//! Remote terminal listener.
//!
//! Accepts inbound TCP connections, authenticates them with the PSK
//! challenge-response described in [`crate::auth`], and feeds received
//! command lines into the command interpreter. Designed for non-blocking
//! polling from the main loop.

use std::time::{Duration, Instant};

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

#[cfg(test)]
use crate::auth::{AUTH_RATE_LIMIT_BASE_SECS, MAX_AUTH_FAILURES};
use crate::auth::{AUTH_TIMEOUT, AuthThrottle, verify_auth_line};
use crate::std_backend::StdNetworkBackend;

/// Maximum number of simultaneous remote connections.
const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// Maximum bytes in a single input line.
const MAX_LINE_LEN: usize = 1024;

/// Maximum command lines taken from one connection in a single poll. The
/// rest stay buffered (and no more is read) until later polls, so one peer
/// cannot hand the host thousands of commands in a single frame.
const MAX_LINES_PER_POLL: usize = 64;

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

/// Authentication state for a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthState {
    /// Challenge sent; waiting for `AUTH <response>` to this nonce.
    AwaitingAuth { nonce: String },
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
    /// When the connection was accepted (for the auth timeout).
    accepted_at: Instant,
    /// Set when the connection should close once its output has drained.
    closing_since: Option<Instant>,
    /// Hard failure (read/write error or output backlog overflow): drop now.
    dead: bool,
}

impl RemoteConnection {
    fn new(id: usize, stream: Box<dyn NetworkStream>, auth: AuthState) -> Self {
        Self {
            id,
            stream,
            auth,
            read_buf: Vec::with_capacity(256),
            write_buf: Vec::new(),
            write_pos: 0,
            inflight: 0,
            last_activity: Instant::now(),
            accepted_at: Instant::now(),
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
    /// only permitted with [`ListenerBind::Loopback`]). It never crosses the
    /// wire: clients prove knowledge of it with the challenge-response in
    /// [`crate::auth`].
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
    pub(crate) auth_failures: AuthThrottle,
    /// Tests pin the challenge nonce so canned input can answer it.
    #[cfg(test)]
    pub(crate) test_nonce: Option<String>,
}

impl RemoteListener {
    /// Create a new listener with the given configuration.
    pub fn new(config: ListenerConfig) -> Self {
        Self {
            config,
            connections: Vec::new(),
            listening: false,
            next_id: 0,
            auth_failures: AuthThrottle::new(),
            #[cfg(test)]
            test_nonce: None,
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
        self.listening = true;
        Ok(())
    }

    /// The configured port.
    pub fn port(&self) -> u16 {
        self.config.port
    }

    /// Whether the listener is active.
    pub fn is_listening(&self) -> bool {
        self.listening
    }

    /// Number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// The current auth lockout length (see [`AuthThrottle`]).
    #[cfg(test)]
    pub(crate) fn rate_limit_window(&self) -> Duration {
        self.auth_failures.window()
    }

    /// Whether new connections are turned away after repeated failures.
    fn is_rate_limited(&self) -> bool {
        self.auth_failures.is_locked()
    }

    /// Record a failed auth attempt.
    pub(crate) fn record_auth_failure(&mut self) {
        self.auth_failures.record_failure("Remote terminal");
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
        let psk = self.config.psk.as_str();

        for conn in &mut self.connections {
            conn.flush();
            if conn.dead || conn.closing_since.is_some() {
                continue;
            }

            // Unauthenticated peers get a short deadline, so idle sockets
            // cannot hold every slot for the whole idle timeout.
            if matches!(conn.auth, AuthState::AwaitingAuth { .. })
                && conn.accepted_at.elapsed() > AUTH_TIMEOUT
            {
                conn.queue(b"AUTH_FAIL timeout\n");
                conn.close_after_flush();
                conn.flush();
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
            // Lines left over from an earlier burst are worked off first.
            let mut eof = false;
            let mut buf = [0u8; 4096];
            let mut total = if conn.read_buf.len() > MAX_LINE_LEN {
                MAX_READ_PER_POLL
            } else {
                0
            };
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
            let mut lines = 0usize;
            while lines < MAX_LINES_PER_POLL
                && let Some(newline_pos) = conn.read_buf.iter().position(|&b| b == b'\n')
            {
                if newline_pos > MAX_LINE_LEN {
                    overlong = true;
                    break;
                }
                lines += 1;
                let line_bytes: Vec<u8> = conn.read_buf.drain(..=newline_pos).collect();
                let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                if line.is_empty() {
                    continue;
                }

                match &conn.auth {
                    AuthState::AwaitingAuth { nonce } => {
                        if verify_auth_line(psk, nonce, &line) {
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

            // Guard against overlong lines -- disconnect the peer. (A long
            // buffer of complete lines left by the per-poll cap is fine.)
            let unterminated = conn.read_buf.len() > MAX_LINE_LEN
                && !conn.read_buf[..=MAX_LINE_LEN].contains(&b'\n');
            if overlong || unterminated {
                conn.read_buf.clear();
                conn.queue(b"error: line too long\n");
                conn.close_after_flush();
                if matches!(conn.auth, AuthState::AwaitingAuth { .. }) {
                    auth_failures += 1;
                }
            }

            if eof && !conn.read_buf.contains(&b'\n') {
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
    fn admit(&mut self, mut stream: Box<dyn NetworkStream>) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        if self.config.psk.is_empty() {
            // No auth required.
            let mut conn = RemoteConnection::new(id, stream, AuthState::Authenticated);
            conn.queue(b"OASIS_OS remote terminal\n> ");
            conn.flush();
            self.connections.push(conn);
            return;
        }
        if self.is_rate_limited() {
            // Rate limit in effect -- reject new connections.
            let _ = stream.write(b"RATE_LIMITED\n");
            let _ = stream.close();
            return;
        }
        #[cfg(test)]
        let nonce = self.test_nonce.clone().unwrap_or_else(|| new_nonce(id));
        #[cfg(not(test))]
        let nonce = new_nonce(id);
        let challenge = format!("AUTH_REQUIRED {nonce}\n");
        let mut conn = RemoteConnection::new(id, stream, AuthState::AwaitingAuth { nonce });
        conn.queue(challenge.as_bytes());
        conn.flush();
        self.connections.push(conn);
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

/// A fresh, unpredictable challenge nonce: 128 bits as 32 hex digits.
///
/// `RandomState` keys come from the OS RNG (once per thread, then stepped
/// per instance); four independently keyed hashes of the connection id and
/// the clock are condensed with SHA-256. Nonces only need to be unique and
/// unguessable -- they are not keys.
fn new_nonce(conn_id: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut seed = Vec::with_capacity(32);
    for round in 0u8..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(round);
        h.write_usize(conn_id);
        h.write_u128(nanos);
        seed.extend_from_slice(&h.finish().to_le_bytes());
    }
    oasis_types::crypto::hex(&oasis_types::crypto::sha256(&seed)[..16])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
        let listener = RemoteListener::new(config);
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
        let awaiting = AuthState::AwaitingAuth { nonce: "ab".into() };
        assert_eq!(awaiting, awaiting.clone());
        assert_eq!(AuthState::Authenticated, AuthState::Authenticated);
        assert_ne!(awaiting, AuthState::Authenticated);
    }

    #[test]
    fn nonces_are_fresh_hex() {
        let a = new_nonce(1);
        let b = new_nonce(1);
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "every challenge must differ");
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

    /// In-memory stream: serves `input`, then would-block; records output.
    struct Scripted {
        input: Vec<u8>,
        pos: usize,
        out: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl NetworkStream for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.pos >= self.input.len() {
                return Err(OasisError::Io(std::io::ErrorKind::WouldBlock.into()));
            }
            let n = buf.len().min(self.input.len() - self.pos);
            buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
        fn write(&mut self, data: &[u8]) -> Result<usize> {
            self.out.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    type Output = std::sync::Arc<std::sync::Mutex<Vec<u8>>>;

    fn admitted(listener: &mut RemoteListener, input: &[u8]) -> Output {
        let out = Output::default();
        listener.listening = true;
        listener.admit(Box::new(Scripted {
            input: input.to_vec(),
            pos: 0,
            out: std::sync::Arc::clone(&out),
        }));
        out
    }

    struct NoAccept;

    impl NetworkBackend for NoAccept {
        fn listen(&mut self, _port: u16) -> Result<()> {
            Ok(())
        }
        fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
            Ok(None)
        }
        fn connect(&mut self, _address: &str, _port: u16) -> Result<Box<dyn NetworkStream>> {
            Err(OasisError::Backend("no".into()))
        }
    }

    #[test]
    fn unauthenticated_connection_times_out() {
        let mut listener = RemoteListener::new(ListenerConfig {
            psk: "k".into(),
            ..ListenerConfig::default()
        });
        let out = admitted(&mut listener, b"");
        listener.poll(&mut NoAccept);
        assert_eq!(listener.connection_count(), 1, "still within the deadline");
        listener.connections[0].accepted_at = Instant::now() - AUTH_TIMEOUT * 2;
        listener.poll(&mut NoAccept);
        assert_eq!(listener.connection_count(), 0);
        let out = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        assert!(out.ends_with("AUTH_FAIL timeout\n"), "{out:?}");
        // Timing out is not a wrong guess.
        assert_eq!(listener.auth_failures.count, 0);
    }

    #[test]
    fn authenticated_connection_has_no_auth_deadline() {
        let mut listener = RemoteListener::new(ListenerConfig::default());
        admitted(&mut listener, b"");
        listener.connections[0].accepted_at = Instant::now() - AUTH_TIMEOUT * 2;
        listener.poll(&mut NoAccept);
        assert_eq!(listener.connection_count(), 1);
    }

    #[test]
    fn command_burst_is_spread_over_polls() {
        let mut listener = RemoteListener::new(ListenerConfig::default());
        let burst: String = (0..150).map(|i| format!("c{i}\n")).collect();
        admitted(&mut listener, burst.as_bytes());
        let mut seen = Vec::new();
        for _ in 0..3 {
            let got = listener.poll(&mut NoAccept);
            assert!(got.len() <= MAX_LINES_PER_POLL);
            seen.extend(got.into_iter().map(|(c, _)| c));
        }
        let expected: Vec<String> = (0..150).map(|i| format!("c{i}")).collect();
        assert_eq!(seen, expected, "every line runs, in order");
        assert_eq!(
            listener.connection_count(),
            1,
            "a long buffer is not overlong"
        );
    }

    #[test]
    fn rate_limited_listener_still_serves_open_loopback() {
        // Without a PSK there is nothing to throttle.
        let mut listener = RemoteListener::new(ListenerConfig::default());
        for _ in 0..MAX_AUTH_FAILURES {
            listener.record_auth_failure();
        }
        admitted(&mut listener, b"");
        assert_eq!(listener.connection_count(), 1);
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
