//! [`TlsProvider`] backed by rustls + ring.
//!
//! Enabled by the `tls-rustls` feature.  Desktop and Pi builds use this
//! provider; the PSP backend supplies its own via `embedded-tls`.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;

use crate::backend::NetworkStream;
use crate::error::{OasisError, Result};

use super::tls::TlsProvider;

/// Shared, reusable TLS client configuration (one per process).
///
/// Cloning is cheap: it increments the `Arc` reference count, sharing
/// the root certificate store and TLS session cache across all clones.
#[derive(Clone)]
pub struct RustlsTlsProvider {
    config: Arc<ClientConfig>,
}

impl RustlsTlsProvider {
    /// Build a provider that trusts Mozilla's root CA bundle.
    pub fn new() -> Self {
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Self {
            config: Arc::new(config),
        }
    }
}

impl Default for RustlsTlsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TlsProvider for RustlsTlsProvider {
    fn connect_tls(
        &self,
        stream: Box<dyn NetworkStream>,
        server_name: &str,
    ) -> Result<Box<dyn NetworkStream>> {
        let sni = ServerName::try_from(server_name.to_owned())
            .map_err(|e| OasisError::Backend(format!("invalid server name: {e}")))?;

        let conn = rustls::ClientConnection::new(Arc::clone(&self.config), sni)
            .map_err(|e| OasisError::Backend(format!("TLS init: {e}")))?;

        Ok(Box::new(RustlsStream::new(conn, stream)?))
    }
}

// ---------------------------------------------------------------------------
// Adapter: bridge rustls's `Read`/`Write` to our `NetworkStream` trait
// ---------------------------------------------------------------------------

/// A TLS-wrapped network stream.
///
/// Internally uses [`rustls::ClientConnection`] for the crypto and
/// delegates raw I/O to the inner [`NetworkStream`].
struct RustlsStream {
    tls: rustls::ClientConnection,
    inner: Box<dyn NetworkStream>,
    /// Buffer for data decrypted by rustls but not yet consumed.
    plaintext_buf: VecDeque<u8>,
}

impl RustlsStream {
    /// Maximum wall-clock time for the TLS handshake.
    const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    fn new(mut tls: rustls::ClientConnection, mut inner: Box<dyn NetworkStream>) -> Result<Self> {
        // Perform the TLS handshake eagerly so callers get a ready stream.
        // rustls is lazy -- we pump I/O until the handshake completes.
        let deadline = std::time::Instant::now() + Self::HANDSHAKE_TIMEOUT;
        let mut adapter = IoAdapter::new(&mut *inner);
        while tls.is_handshaking() {
            if std::time::Instant::now() > deadline {
                return Err(OasisError::Backend("TLS handshake timed out".to_string()));
            }
            if tls.wants_write() {
                tls.write_tls(&mut adapter)
                    .map_err(|e| OasisError::Backend(format!("TLS handshake write: {e}")))?;
            }
            if tls.wants_read() {
                match tls.read_tls(&mut adapter) {
                    Ok(0) => {
                        return Err(OasisError::Backend(
                            "TLS handshake failed: peer closed connection".to_string(),
                        ));
                    },
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // Non-blocking socket has nothing yet -- spin.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    },
                    Err(e) => {
                        return Err(OasisError::Backend(format!("TLS handshake read: {e}")));
                    },
                    Ok(_) => {},
                }
                tls.process_new_packets()
                    .map_err(|e| OasisError::Backend(format!("TLS handshake failed: {e}")))?;
            }
        }
        // Flush any remaining handshake bytes.
        let _ = tls.write_tls(&mut adapter);

        Ok(Self {
            tls,
            inner,
            plaintext_buf: VecDeque::new(),
        })
    }

    /// Pump ciphertext from the network into rustls and move any resulting
    /// plaintext into `self.plaintext_buf`.
    fn pull_plaintext(&mut self) -> Result<()> {
        // First, drain any plaintext already buffered inside rustls
        // (e.g. application data received during the handshake).
        if self.drain_reader() {
            return Ok(());
        }

        // Loop to handle partial TLS records: a single read_tls may not
        // provide enough ciphertext for a complete record, so we keep
        // reading until plaintext is available, EOF, or WouldBlock.
        loop {
            // Flush any pending TLS writes before reading.  This mirrors
            // what rustls::StreamOwned::complete_io() does and prevents
            // deadlocks when the server is waiting for client messages
            // (e.g. post-handshake acknowledgments).
            {
                let mut adapter = IoAdapter::new(&mut *self.inner);
                while self.tls.wants_write() {
                    self.tls
                        .write_tls(&mut adapter)
                        .map_err(|e| OasisError::Backend(format!("TLS write_tls: {e}")))?;
                }
            }

            let mut adapter = IoAdapter::new(&mut *self.inner);

            match self.tls.read_tls(&mut adapter) {
                Ok(0) => return Ok(()), // EOF
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => {
                    return Err(OasisError::Backend(format!("TLS read_tls: {e}")));
                },
                Ok(_) => {},
            }

            // Decrypt.
            self.tls
                .process_new_packets()
                .map_err(|e| OasisError::Backend(format!("TLS process: {e}")))?;

            if self.drain_reader() {
                return Ok(());
            }
            // No plaintext yet -- TLS record may be incomplete, try reading more.
        }
    }

    /// Drain any available plaintext from `tls.reader()` into
    /// `plaintext_buf`.  Returns `true` if any data was drained.
    fn drain_reader(&mut self) -> bool {
        let mut tmp = [0u8; 8192];
        let mut got = false;
        loop {
            match self.tls.reader().read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    self.plaintext_buf.extend(tmp[..n].iter().copied());
                    got = true;
                },
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        got
    }
}

impl NetworkStream for RustlsStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Return buffered plaintext first.
        if !self.plaintext_buf.is_empty() {
            let n = buf.len().min(self.plaintext_buf.len());
            drain_deque(&mut self.plaintext_buf, &mut buf[..n]);
            return Ok(n);
        }

        // Try to get more plaintext from the network.
        self.pull_plaintext()?;

        if self.plaintext_buf.is_empty() {
            return Ok(0); // Nothing available (non-blocking).
        }

        let n = buf.len().min(self.plaintext_buf.len());
        drain_deque(&mut self.plaintext_buf, &mut buf[..n]);
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        // Feed plaintext into rustls.
        let n = self
            .tls
            .writer()
            .write(data)
            .map_err(|e| OasisError::Backend(format!("TLS write: {e}")))?;

        // Flush the resulting ciphertext to the network.
        let mut adapter = IoAdapter::new(&mut *self.inner);
        self.tls
            .write_tls(&mut adapter)
            .map_err(|e| OasisError::Backend(format!("TLS write_tls: {e}")))?;

        Ok(n)
    }

    fn close(&mut self) -> Result<()> {
        self.tls.send_close_notify();
        let mut adapter = IoAdapter::new(&mut *self.inner);
        let _ = self.tls.write_tls(&mut adapter);
        self.inner.close()
    }
}

/// Copy `n` bytes from the front of `deque` into `buf` and remove them.
///
/// Uses `VecDeque::as_slices()` to avoid O(remaining) drain overhead that
/// `Vec::drain(..n)` would incur.
fn drain_deque(deque: &mut VecDeque<u8>, buf: &mut [u8]) {
    let n = buf.len();
    let (front, back) = deque.as_slices();
    if n <= front.len() {
        buf.copy_from_slice(&front[..n]);
    } else {
        let mid = front.len();
        buf[..mid].copy_from_slice(front);
        buf[mid..n].copy_from_slice(&back[..n - mid]);
    }
    deque.drain(..n);
}

// ---------------------------------------------------------------------------
// IoAdapter: bridge NetworkStream to std::io::Read + std::io::Write
// ---------------------------------------------------------------------------

/// Thin wrapper that lets rustls call `std::io::Read` / `Write` on a
/// `&mut dyn NetworkStream`.
struct IoAdapter<'a> {
    inner: &'a mut dyn NetworkStream,
}

impl<'a> IoAdapter<'a> {
    fn new(inner: &'a mut dyn NetworkStream) -> Self {
        Self { inner }
    }
}

impl io::Read for IoAdapter<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf).map_err(oasis_err_to_io)
    }
}

impl io::Write for IoAdapter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf).map_err(oasis_err_to_io)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Convert an [`OasisError`] to [`io::Error`], preserving the original
/// `io::Error` (and its error kind) when the variant is `OasisError::Io`.
fn oasis_err_to_io(e: OasisError) -> io::Error {
    match e {
        OasisError::Io(io_err) => io_err,
        other => io::Error::other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
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

    /// Unlike [`StdNetworkStream`] which maps `WouldBlock` to `Ok(0)`,
    /// this wrapper preserves the `WouldBlock` error kind so that
    /// `IoAdapter` and `pull_plaintext` can distinguish timeout from EOF.
    struct TcpNetworkStream(TcpStream);

    impl NetworkStream for TcpNetworkStream {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            self.0.read(buf).map_err(|e| OasisError::Io(e))
        }

        fn write(&mut self, data: &[u8]) -> Result<usize> {
            self.0.write(data).map_err(|e| OasisError::Io(e))
        }

        fn close(&mut self) -> Result<()> {
            self.0
                .shutdown(std::net::Shutdown::Both)
                .map_err(|e| OasisError::Io(e))
        }
    }

    // ---------------------------------------------------------------
    // TLS loopback helper
    // ---------------------------------------------------------------

    /// Create a self-signed certificate and matching rustls server config.
    fn make_server_config() -> (Arc<rustls::ServerConfig>, rcgen::CertifiedKey) {
        let certified_key =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

        let cert_der = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
        let key_der =
            rustls::pki_types::PrivateKeyDer::try_from(certified_key.key_pair.serialize_der())
                .unwrap();

        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();

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
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected handshake to fail with garbage bytes"),
        };
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
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected handshake to fail on EOF"),
        };
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
    fn test_clone_shares_config() {
        let p1 = RustlsTlsProvider::new();
        let p2 = p1.clone();
        assert!(Arc::ptr_eq(&p1.config, &p2.config));
    }
}
