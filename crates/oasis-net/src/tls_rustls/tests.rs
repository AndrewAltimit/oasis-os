use super::*;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

// ---------------------------------------------------------------
// MockNetworkStream: configurable mock for unit tests
// ---------------------------------------------------------------

struct MockNetworkStream {
    read_data: Vec<u8>,
    read_pos: usize,
    written: Vec<u8>,
}

impl MockNetworkStream {
    fn from_data(data: &[u8]) -> Self {
        Self {
            read_data: data.to_vec(),
            read_pos: 0,
            written: Vec::new(),
        }
    }

    fn empty() -> Self {
        Self::from_data(&[])
    }
}

impl NetworkStream for MockNetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.read_pos >= self.read_data.len() {
            return Ok(0); // EOF
        }
        let available = &self.read_data[self.read_pos..];
        let n = buf.len().min(available.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.read_pos += n;
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.written.extend_from_slice(data);
        Ok(data.len())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------
// TcpNetworkStream: wraps TcpStream preserving WouldBlock errors
// ---------------------------------------------------------------

/// A raw `TcpStream` as a `NetworkStream` that keeps the `WouldBlock`
/// error kind (as `StdNetworkStream` does), so `IoAdapter` and
/// `pull_plaintext` can tell "no data yet" from EOF.
struct TcpNetworkStream(TcpStream);

impl NetworkStream for TcpNetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf).map_err(OasisError::Io)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.0.write(data).map_err(OasisError::Io)
    }

    fn close(&mut self) -> Result<()> {
        self.0
            .shutdown(std::net::Shutdown::Both)
            .map_err(OasisError::Io)
    }
}

// ---------------------------------------------------------------
// TLS loopback helper
// ---------------------------------------------------------------

/// Create a self-signed certificate and matching rustls server config.
fn make_server_config() -> (Arc<rustls::ServerConfig>, rcgen::CertifiedKey) {
    make_server_config_with_alpn(&[])
}

/// Variant of [`make_server_config`] that advertises specific ALPN
/// protocols on the server side, so tests can assert that the
/// client and server agree on an application protocol.
fn make_server_config_with_alpn(
    alpn: &[&[u8]],
) -> (Arc<rustls::ServerConfig>, rcgen::CertifiedKey) {
    let certified_key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let cert_der = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::try_from(certified_key.key_pair.serialize_der()).unwrap();

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();

    (Arc::new(config), certified_key)
}

/// Build a client config that trusts only the given self-signed cert.
fn make_client_config(certified_key: &rcgen::CertifiedKey) -> RustlsTlsProvider {
    let cert_der = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(cert_der).unwrap();

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    RustlsTlsProvider {
        config: Arc::new(config),
    }
}

/// Spawn a TCP listener that accepts one connection and runs a TLS
/// server that sends `payload`, then reads any client data into
/// `received` (if provided), and closes.
fn spawn_server(
    server_config: Arc<rustls::ServerConfig>,
    payload: Vec<u8>,
) -> (std::thread::JoinHandle<()>, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // Use a short timeout for the drain-read after sending.
        // No timeout during handshake (write_all drives it).
        let conn = rustls::ServerConnection::new(server_config).unwrap();

        let mut tls_stream = rustls::StreamOwned::new(conn, stream);

        // Send payload to client.
        if !payload.is_empty() {
            let _ = io::Write::write_all(&mut tls_stream, &payload);
            let _ = io::Write::flush(&mut tls_stream);
        }

        // Set a short timeout for drain-reads, then read anything
        // the client sends (drives close_notify).
        tls_stream
            .sock
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .ok();
        let mut buf = [0u8; 4096];
        loop {
            match io::Read::read(&mut tls_stream, &mut buf) {
                Ok(0) => break,
                Err(ref e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut
                        || e.kind() == io::ErrorKind::ConnectionReset =>
                {
                    break;
                },
                Err(_) => break,
                Ok(_) => {},
            }
        }
    });

    (handle, port)
}

/// Connect via TCP and wrap with our TLS provider.
///
/// Uses `TcpNetworkStream` (which preserves WouldBlock as a real
/// error) and a 5-second read timeout for TLS data reads.
fn connect_to(provider: &RustlsTlsProvider, port: u16) -> Result<Box<dyn NetworkStream>> {
    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let net: Box<dyn NetworkStream> = Box::new(TcpNetworkStream(tcp));
    provider.connect_tls(net, "localhost")
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[test]
fn test_handshake_failure_propagates() {
    // Garbage bytes should cause a TLS error, not silently succeed.
    let mock = Box::new(MockNetworkStream::from_data(b"not a TLS server"));
    let provider = RustlsTlsProvider::new();
    let sni = ServerName::try_from("example.com".to_owned()).unwrap();
    let conn = rustls::ClientConnection::new(Arc::clone(&provider.config), sni).unwrap();
    let result = RustlsStream::new(conn, mock);
    assert!(
        result.is_err(),
        "expected handshake to fail with garbage bytes"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("TLS handshake"),
        "expected 'TLS handshake' in error: {msg}",
    );
}

#[test]
fn test_handshake_eof_propagates() {
    // Empty stream (immediate EOF) should produce an error.
    let mock = Box::new(MockNetworkStream::empty());
    let provider = RustlsTlsProvider::new();
    let sni = ServerName::try_from("example.com".to_owned()).unwrap();
    let conn = rustls::ClientConnection::new(Arc::clone(&provider.config), sni).unwrap();
    let result = RustlsStream::new(conn, mock);
    assert!(result.is_err(), "expected handshake to fail on EOF");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("peer closed connection"),
        "expected 'peer closed connection' in error: {msg}",
    );
}

#[test]
fn test_handshake_success() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());
    let result = connect_to(&provider, port);
    assert!(result.is_ok(), "handshake failed: {:?}", result.err());
    let mut stream = result.unwrap();
    let _ = stream.close();
    let _ = handle.join();
}

/// TCP stream whose reads fail with an injected kind once armed.
struct InjectStream {
    tcp: TcpNetworkStream,
    fail: Arc<std::sync::Mutex<Option<io::ErrorKind>>>,
}

impl NetworkStream for InjectStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if let Some(kind) = *self.fail.lock().unwrap() {
            return Err(OasisError::Io(io::Error::from(kind)));
        }
        self.tcp.read(buf)
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.tcp.write(data)
    }
    fn close(&mut self) -> Result<()> {
        self.tcp.close()
    }
}

/// The TLS layer must pass "no data yet" through as WouldBlock but keep
/// a TimedOut (a dead peer on a non-blocking socket) as a real error,
/// with its kind intact, instead of folding it into WouldBlock.
#[test]
fn test_timed_out_is_an_error_not_would_block() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());
    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let fail = Arc::new(std::sync::Mutex::new(None));
    let inner: Box<dyn NetworkStream> = Box::new(InjectStream {
        tcp: TcpNetworkStream(tcp),
        fail: Arc::clone(&fail),
    });
    let mut stream = provider.connect_tls(inner, "localhost").unwrap();
    let mut buf = [0u8; 64];

    *fail.lock().unwrap() = Some(io::ErrorKind::WouldBlock);
    let err = stream.read(&mut buf).unwrap_err();
    assert!(err.is_would_block(), "got {err}");

    *fail.lock().unwrap() = Some(io::ErrorKind::TimedOut);
    let err = stream.read(&mut buf).unwrap_err();
    assert!(
        !err.is_would_block(),
        "TimedOut folded into WouldBlock: {err}"
    );
    assert!(err.is_timed_out(), "kind lost: {err}");

    *fail.lock().unwrap() = None;
    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_read_after_handshake() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, b"hello TLS".to_vec());
    let mut stream = connect_to(&provider, port).unwrap();

    // Server sends "hello TLS" immediately after handshake.
    let mut buf = [0u8; 64];
    let mut total = 0;
    while total < 9 {
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    assert_eq!(&buf[..total], b"hello TLS");

    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_write_after_handshake() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());
    let mut stream = connect_to(&provider, port).unwrap();

    let n = stream.write(b"test write data").unwrap();
    assert_eq!(n, 15);

    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_large_data_transfer() {
    // >16KB crosses a TLS record boundary (max record = 16384).
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let payload = vec![0xAB_u8; 20_000];
    let (handle, port) = spawn_server(server_cfg, payload.clone());
    let mut stream = connect_to(&provider, port).unwrap();

    let mut received = Vec::new();
    let mut buf = [0u8; 8192];
    while received.len() < payload.len() {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => received.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    assert_eq!(
        received.len(),
        payload.len(),
        "expected {} bytes, got {}",
        payload.len(),
        received.len(),
    );
    assert_eq!(received, payload);

    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_close_sends_close_notify() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());
    let mut stream = connect_to(&provider, port).unwrap();

    // close() should not panic and should succeed.
    assert!(stream.close().is_ok());
    let _ = handle.join();
}

#[test]
fn test_provider_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RustlsTlsProvider>();
}

#[test]
fn test_stream_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<RustlsStream>();
}

#[test]
fn test_alpn_negotiates_h2_when_offered() {
    // Server advertises h2 only; client offers [h2, http/1.1].
    // Handshake should succeed and report h2 as negotiated.
    let (server_cfg, cert_key) = make_server_config_with_alpn(&[b"h2"]);
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let net: Box<dyn NetworkStream> = Box::new(TcpNetworkStream(tcp));
    let conn = provider
        .connect_tls_with_alpn(net, "localhost", &[b"h2", b"http/1.1"])
        .unwrap();
    assert_eq!(conn.alpn.as_deref(), Some(b"h2".as_slice()));
    let _ = handle.join();
}

#[test]
fn test_alpn_falls_back_to_http11() {
    // Server advertises http/1.1 only; client offers both.
    let (server_cfg, cert_key) = make_server_config_with_alpn(&[b"http/1.1"]);
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let net: Box<dyn NetworkStream> = Box::new(TcpNetworkStream(tcp));
    let conn = provider
        .connect_tls_with_alpn(net, "localhost", &[b"h2", b"http/1.1"])
        .unwrap();
    assert_eq!(conn.alpn.as_deref(), Some(b"http/1.1".as_slice()));
    let _ = handle.join();
}

#[test]
fn test_alpn_none_when_server_silent() {
    // Server advertises no ALPN protocols — handshake still
    // succeeds, but with no negotiated protocol.
    let (server_cfg, cert_key) = make_server_config_with_alpn(&[]);
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let net: Box<dyn NetworkStream> = Box::new(TcpNetworkStream(tcp));
    let conn = provider
        .connect_tls_with_alpn(net, "localhost", &[b"h2", b"http/1.1"])
        .unwrap();
    assert!(conn.alpn.is_none());
    let _ = handle.join();
}

#[test]
fn test_clone_shares_config() {
    let p1 = RustlsTlsProvider::new();
    let p2 = p1.clone();
    assert!(Arc::ptr_eq(&p1.config, &p2.config));
}

// -- Phase 3.5: TLS certificate validation & edge-case tests --

#[test]
fn test_default_provider() {
    let p = RustlsTlsProvider::default();
    // Both default() and new() share the process-wide LazyLock config.
    assert!(Arc::ptr_eq(&p.config, &RustlsTlsProvider::new().config));
}

#[test]
fn test_invalid_server_name_rejected() {
    let provider = RustlsTlsProvider::new();
    let mock: Box<dyn NetworkStream> = Box::new(MockNetworkStream::empty());
    // An empty server name is invalid for SNI.
    let result = provider.connect_tls(mock, "");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("invalid server name"),
        "expected 'invalid server name' in: {msg}",
    );
}

#[test]
fn test_untrusted_self_signed_rejected() {
    // A server with a self-signed cert should be rejected by a
    // client that only trusts Mozilla roots.
    let (server_cfg, _cert_key) = make_server_config();
    let provider = RustlsTlsProvider::new(); // trusts Mozilla roots
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let result = connect_to(&provider, port);
    assert!(
        result.is_err(),
        "self-signed cert should be rejected by default provider",
    );

    let _ = handle.join();
}

#[test]
fn test_sni_mismatch_rejected() {
    // Server cert is for "localhost" but client connects with
    // "not-localhost" -- handshake should fail.
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let tcp = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let net: Box<dyn NetworkStream> = Box::new(TcpNetworkStream(tcp));
    let result = provider.connect_tls(net, "not-localhost");
    assert!(result.is_err(), "SNI mismatch should fail handshake",);

    let _ = handle.join();
}

#[test]
fn test_expired_cert_rejected() {
    // Generate a certificate that expired in the past.
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2020, 1, 2);

    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    let certified_key = rcgen::CertifiedKey { cert, key_pair };

    let cert_der = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::try_from(certified_key.key_pair.serialize_der()).unwrap();

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    let server_cfg = Arc::new(server_config);

    let provider = make_client_config(&certified_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let result = connect_to(&provider, port);
    assert!(result.is_err(), "expired certificate should be rejected",);

    let _ = handle.join();
}

#[test]
fn test_not_yet_valid_cert_rejected() {
    // Generate a certificate that is not yet valid.
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2099, 1, 1);
    params.not_after = rcgen::date_time_ymd(2100, 1, 1);

    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    let certified_key = rcgen::CertifiedKey { cert, key_pair };

    let cert_der = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::try_from(certified_key.key_pair.serialize_der()).unwrap();

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    let server_cfg = Arc::new(server_config);

    let provider = make_client_config(&certified_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());

    let result = connect_to(&provider, port);
    assert!(
        result.is_err(),
        "not-yet-valid certificate should be rejected",
    );

    let _ = handle.join();
}

#[test]
fn test_read_zero_length_buffer() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, b"data".to_vec());
    let mut stream = connect_to(&provider, port).unwrap();

    // Reading into a zero-length buffer should return 0.
    let mut buf = [0u8; 0];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_write_empty_data() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let (handle, port) = spawn_server(server_cfg, Vec::new());
    let mut stream = connect_to(&provider, port).unwrap();

    // Writing empty data should succeed with 0 bytes.
    let n = stream.write(b"").unwrap();
    assert_eq!(n, 0);

    let _ = stream.close();
    let _ = handle.join();
}

#[test]
fn test_multiple_writes_and_reads() {
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);
    let payload = b"chunked data stream test".to_vec();
    let (handle, port) = spawn_server(server_cfg, payload.clone());
    let mut stream = connect_to(&provider, port).unwrap();

    // Read the payload in small chunks.
    let mut received = Vec::new();
    let mut buf = [0u8; 4];
    while received.len() < payload.len() {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                received.extend_from_slice(&buf[..n]);
            },
            Err(_) => break,
        }
    }
    assert_eq!(received, payload);

    let _ = stream.close();
    let _ = handle.join();
}

/// Records writes; refuses them with `WouldBlock` while `blocked`.
struct FullSocket {
    blocked: Arc<std::sync::atomic::AtomicBool>,
    written: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl NetworkStream for FullSocket {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Err(OasisError::Io(io::ErrorKind::WouldBlock.into()))
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        if self.blocked.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(OasisError::Io(io::ErrorKind::WouldBlock.into()));
        }
        self.written.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn test_full_socket_is_not_a_failed_write() {
    // Regression: a `WouldBlock` from the socket while flushing
    // ciphertext failed the whole `write`, although rustls had already
    // taken the plaintext -- a caller retrying would send it twice.
    let blocked = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let written = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sni = ServerName::try_from("example.com").unwrap();
    let tls = rustls::ClientConnection::new(Arc::clone(&SHARED_CONFIG), sni).unwrap();
    let mut stream = RustlsStream {
        tls,
        inner: Box::new(FullSocket {
            blocked: Arc::clone(&blocked),
            written: Arc::clone(&written),
        }),
        plaintext_buf: VecDeque::new(),
    };
    // The ClientHello is queued and the socket refuses it.
    assert!(stream.tls.wants_write());
    assert_eq!(stream.write(b"GET / HTTP/1.1\r\n").unwrap(), 16);
    assert!(stream.flush().is_ok());
    assert!(written.lock().unwrap().is_empty());
    assert!(stream.read(&mut [0u8; 16]).unwrap_err().is_would_block());
    // Once the socket drains, the queued ciphertext goes out.
    blocked.store(false, std::sync::atomic::Ordering::SeqCst);
    stream.flush().unwrap();
    assert!(!written.lock().unwrap().is_empty());
    assert!(!stream.tls.wants_write());
}

/// Refuses the first `busy` writes (`WouldBlock`), then takes everything.
struct BusyThenOpen {
    busy: usize,
    written: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl NetworkStream for BusyThenOpen {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Err(OasisError::Io(io::ErrorKind::WouldBlock.into()))
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        if self.busy > 0 {
            self.busy -= 1;
            return Err(OasisError::Io(io::ErrorKind::WouldBlock.into()));
        }
        self.written.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn test_close_flushes_ciphertext_a_full_socket_held_back() {
    // `write` reports the plaintext as sent while the socket is full;
    // a `close` right after must still get the queued bytes out.
    let written = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sni = ServerName::try_from("example.com").unwrap();
    let tls = rustls::ClientConnection::new(Arc::clone(&SHARED_CONFIG), sni).unwrap();
    let mut stream = RustlsStream {
        tls,
        inner: Box::new(BusyThenOpen {
            busy: 3,
            written: Arc::clone(&written),
        }),
        plaintext_buf: VecDeque::new(),
    };
    assert_eq!(stream.write(b"GET / HTTP/1.1\r\n").unwrap(), 16);
    stream.close().unwrap();
    assert!(
        !written.lock().unwrap().is_empty(),
        "queued ClientHello sent"
    );
    assert!(!stream.tls.wants_write(), "nothing left queued");
}

#[test]
fn test_drain_deque_front_only() {
    let mut deque = VecDeque::from(vec![1, 2, 3, 4, 5]);
    let mut buf = [0u8; 3];
    drain_deque(&mut deque, &mut buf);
    assert_eq!(buf, [1, 2, 3]);
    assert_eq!(deque.len(), 2);
}

#[test]
fn test_drain_deque_wrapping() {
    // Force the deque to wrap around internally.
    let mut deque = VecDeque::with_capacity(4);
    deque.push_back(10);
    deque.push_back(20);
    deque.push_back(30);
    // Pop from front so that subsequent pushes wrap.
    let _ = deque.pop_front();
    let _ = deque.pop_front();
    deque.push_back(40);
    deque.push_back(50);
    deque.push_back(60);
    // Deque now has [30, 40, 50, 60] but internally wrapped.
    let mut buf = [0u8; 4];
    drain_deque(&mut deque, &mut buf);
    assert_eq!(buf, [30, 40, 50, 60]);
    assert!(deque.is_empty());
}

#[test]
fn test_drain_deque_exact_front_size() {
    let mut deque = VecDeque::from(vec![1, 2, 3]);
    let mut buf = [0u8; 3];
    drain_deque(&mut deque, &mut buf);
    assert_eq!(buf, [1, 2, 3]);
    assert!(deque.is_empty());
}

#[test]
fn test_oasis_err_to_io_preserves_io_error() {
    let io_err = io::Error::new(io::ErrorKind::WouldBlock, "test block");
    let oasis_err = OasisError::Io(io_err);
    let converted = oasis_err_to_io(oasis_err);
    assert_eq!(converted.kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn test_oasis_err_to_io_converts_other() {
    let oasis_err = OasisError::Backend("some error".into());
    let converted = oasis_err_to_io(oasis_err);
    assert_eq!(converted.kind(), io::ErrorKind::Other);
    assert!(
        converted.to_string().contains("some error"),
        "error message should be preserved",
    );
}

#[test]
fn test_io_adapter_read_eof() {
    let mut mock = MockNetworkStream::empty();
    let mut adapter = IoAdapter::new(&mut mock);
    let mut buf = [0u8; 16];
    let n = io::Read::read(&mut adapter, &mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn test_io_adapter_write_and_flush() {
    let mut mock = MockNetworkStream::empty();
    {
        let mut adapter = IoAdapter::new(&mut mock);
        let n = io::Write::write(&mut adapter, b"hello").unwrap();
        assert_eq!(n, 5);
        // flush is a no-op.
        assert!(io::Write::flush(&mut adapter).is_ok());
    }
    assert_eq!(mock.written, b"hello");
}

#[test]
fn test_bidirectional_data() {
    // Client writes data, server echoes it back.
    let (server_cfg, cert_key) = make_server_config();
    let provider = make_client_config(&cert_key);

    // Spawn an echo server.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let conn = rustls::ServerConnection::new(server_cfg).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, stream);
        tls.sock
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .ok();
        let mut buf = [0u8; 256];
        match io::Read::read(&mut tls, &mut buf) {
            Ok(n) if n > 0 => {
                let _ = io::Write::write_all(&mut tls, &buf[..n]);
                let _ = io::Write::flush(&mut tls);
            },
            _ => {},
        }
        // Brief sleep so client can read echo.
        std::thread::sleep(std::time::Duration::from_millis(100));
    });

    let mut stream = connect_to(&provider, port).unwrap();
    stream.write(b"echo me").unwrap();

    // Read the echoed data.
    let mut buf = [0u8; 64];
    let mut total = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while total < 7 && std::time::Instant::now() < deadline {
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    assert_eq!(&buf[..total], b"echo me");

    let _ = stream.close();
    let _ = handle.join();
}
