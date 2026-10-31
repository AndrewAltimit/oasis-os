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
