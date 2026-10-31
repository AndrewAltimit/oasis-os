//! [`TlsProvider`] backed by rustls + ring.
//!
//! Enabled by the `tls-rustls` feature.  Desktop and Pi builds use this
//! provider; the PSP backend supplies its own via `embedded-tls`.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, LazyLock};

use rustls::ClientConfig;
use rustls::pki_types::ServerName;

use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};
use oasis_types::tls::TlsConnection;

use super::tls::TlsProvider;

/// `write_tls` attempts `close` makes to drain queued ciphertext (about
/// 100 ms in all on a socket that stays full).
const CLOSE_FLUSH_ATTEMPTS: usize = 50;

/// Process-wide TLS client configuration (built once, shared by all providers).
///
/// Advertises ALPN protocols `h2` and `http/1.1` so servers that require
/// HTTP/2 (many CDNs) will negotiate it on the same connection. Callers
/// that don't speak HTTP/2 fall through to the default `http/1.1`
/// selection and use the plain HTTP/1.1 path.
static SHARED_CONFIG: LazyLock<Arc<ClientConfig>> = LazyLock::new(|| {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
});

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
    ///
    /// Uses a process-wide `LazyLock` singleton so the root certificate
    /// store is only initialized once, regardless of how many providers
    /// or `StdNetworkBackend` instances are created.
    pub fn new() -> Self {
        Self {
            config: Arc::clone(&SHARED_CONFIG),
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
            .map_err(|e| OasisError::Backend(format!("invalid server name: {e}").into()))?;

        let conn = rustls::ClientConnection::new(Arc::clone(&self.config), sni)
            .map_err(|e| OasisError::Backend(format!("TLS init: {e}").into()))?;

        let (stream, _alpn) = RustlsStream::new(conn, stream)?;
        Ok(Box::new(stream))
    }

    fn connect_tls_with_alpn(
        &self,
        stream: Box<dyn NetworkStream>,
        server_name: &str,
        alpn_protocols: &[&[u8]],
    ) -> Result<TlsConnection> {
        let sni = ServerName::try_from(server_name.to_owned())
            .map_err(|e| OasisError::Backend(format!("invalid server name: {e}").into()))?;

        // Build a per-request config only when the caller's ALPN list
        // differs from what's already baked into `self.config`. That
        // keeps the common "same as SHARED_CONFIG" case allocation-free
        // while still letting tests (and any caller with a custom base
        // config) negotiate arbitrary ALPN lists.
        let config = if alpn_matches(&self.config.alpn_protocols, alpn_protocols) {
            Arc::clone(&self.config)
        } else {
            let mut cfg = (*self.config).clone();
            cfg.alpn_protocols = alpn_protocols.iter().map(|p| p.to_vec()).collect();
            Arc::new(cfg)
        };

        let conn = rustls::ClientConnection::new(config, sni)
            .map_err(|e| OasisError::Backend(format!("TLS init: {e}").into()))?;

        let (stream, alpn) = RustlsStream::new(conn, stream)?;
        Ok(TlsConnection {
            stream: Box::new(stream),
            alpn,
        })
    }
}

fn alpn_matches(existing: &[Vec<u8>], requested: &[&[u8]]) -> bool {
    if existing.len() != requested.len() {
        return false;
    }
    existing
        .iter()
        .zip(requested.iter())
        .all(|(a, b)| a.as_slice() == *b)
}

// ---------------------------------------------------------------------------
// Adapter: bridge rustls's `Read`/`Write` to our `NetworkStream` trait
// ---------------------------------------------------------------------------

/// Result of a `pull_plaintext` call, distinguishing "got data",
/// "socket would block" and "connection EOF".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PullResult {
    /// New plaintext was added to `plaintext_buf`.
    GotData,
    /// The underlying socket returned WouldBlock — no data yet.
    WouldBlock,
    /// The peer closed the connection (EOF on the TLS layer).
    Eof,
}

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

impl std::fmt::Debug for RustlsStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustlsStream")
            .field("plaintext_buf_len", &self.plaintext_buf.len())
            .finish_non_exhaustive()
    }
}

impl RustlsStream {
    /// Maximum wall-clock time for the TLS handshake.
    const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    fn new(
        mut tls: rustls::ClientConnection,
        mut inner: Box<dyn NetworkStream>,
    ) -> Result<(Self, Option<Vec<u8>>)> {
        // Perform the TLS handshake eagerly so callers get a ready stream.
        // rustls is lazy -- we pump I/O until the handshake completes.
        let deadline = std::time::Instant::now() + Self::HANDSHAKE_TIMEOUT;
        let mut adapter = IoAdapter::new(&mut *inner);
        while tls.is_handshaking() {
            if std::time::Instant::now() > deadline {
                return Err(OasisError::Backend("TLS handshake timed out".into()));
            }
            if tls.wants_write() {
                match tls.write_tls(&mut adapter) {
                    Ok(_) => {},
                    // Socket send buffer full: retry after a short wait.
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    },
                    Err(e) => {
                        return Err(OasisError::Backend(
                            format!("TLS handshake write: {e}").into(),
                        ));
                    },
                }
            }
            if tls.wants_read() {
                match tls.read_tls(&mut adapter) {
                    Ok(0) => {
                        return Err(OasisError::Backend(
                            "TLS handshake failed: peer closed connection".into(),
                        ));
                    },
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // Non-blocking socket has nothing yet -- spin.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    },
                    Err(e) => {
                        return Err(OasisError::Backend(
                            format!("TLS handshake read: {e}").into(),
                        ));
                    },
                    Ok(_) => {},
                }
                tls.process_new_packets().map_err(|e| {
                    OasisError::Backend(format!("TLS handshake failed: {e}").into())
                })?;
            }
        }
        // Flush any remaining handshake bytes.
        let _ = tls.write_tls(&mut adapter);

        let alpn = tls.alpn_protocol().map(|b| b.to_vec());

        Ok((
            Self {
                tls,
                inner,
                plaintext_buf: VecDeque::new(),
            },
            alpn,
        ))
    }

    /// Pump ciphertext from the network into rustls and move any resulting
    /// plaintext into `self.plaintext_buf`.
    fn pull_plaintext(&mut self) -> Result<PullResult> {
        // First, drain any plaintext already buffered inside rustls
        // (e.g. application data received during the handshake).
        if self.drain_reader() {
            return Ok(PullResult::GotData);
        }

        // Loop to handle partial TLS records: a single read_tls may not
        // provide enough ciphertext for a complete record, so we keep
        // reading until plaintext is available, EOF, or WouldBlock.
        loop {
            // Flush any pending TLS writes before reading.  This mirrors
            // what rustls::StreamOwned::complete_io() does and prevents
            // deadlocks when the server is waiting for client messages
            // (e.g. post-handshake acknowledgments).
            self.flush_tls()?;

            let mut adapter = IoAdapter::new(&mut *self.inner);

            match self.tls.read_tls(&mut adapter) {
                Ok(0) => return Ok(PullResult::Eof),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(PullResult::WouldBlock);
                },
                // Keep the kind (TimedOut, ConnectionReset, ...) so callers
                // can tell a dead peer from "no data yet": a TimedOut here is
                // the kernel giving up on the connection, not a retry hint.
                Err(e) => {
                    return Err(OasisError::Io(io::Error::new(
                        e.kind(),
                        format!("TLS read_tls: {e}"),
                    )));
                },
                Ok(_) => {},
            }

            // Decrypt.
            self.tls
                .process_new_packets()
                .map_err(|e| OasisError::Backend(format!("TLS process: {e}").into()))?;

            if self.drain_reader() {
                return Ok(PullResult::GotData);
            }
            // No plaintext yet -- TLS record may be incomplete, try reading more.
        }
    }

    /// Push queued ciphertext to the socket until rustls has none left or
    /// the non-blocking socket is full. On `WouldBlock` the rest stays
    /// queued inside rustls and goes out on the next read, write or flush,
    /// so a full send buffer is never reported as a failed write (the
    /// plaintext was already accepted, and a caller retrying it would send
    /// it twice).
    fn flush_tls(&mut self) -> Result<()> {
        let mut adapter = IoAdapter::new(&mut *self.inner);
        while self.tls.wants_write() {
            match self.tls.write_tls(&mut adapter) {
                // The socket took nothing: stop rather than spin.
                Ok(0) => break,
                Ok(_) => {},
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    return Err(OasisError::Io(io::Error::new(
                        e.kind(),
                        format!("TLS write_tls: {e}"),
                    )));
                },
            }
        }
        Ok(())
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
        match self.pull_plaintext()? {
            PullResult::GotData => {
                let n = buf.len().min(self.plaintext_buf.len());
                drain_deque(&mut self.plaintext_buf, &mut buf[..n]);
                Ok(n)
            },
            PullResult::Eof => Ok(0),
            PullResult::WouldBlock => Err(OasisError::Io(io::Error::new(
                io::ErrorKind::WouldBlock,
                "would block",
            ))),
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        // Make room: ciphertext an earlier full socket left queued.
        self.flush_tls()?;
        // Feed plaintext into rustls.
        let n = self
            .tls
            .writer()
            .write(data)
            .map_err(|e| OasisError::Backend(format!("TLS write: {e}").into()))?;
        if n == 0 && !data.is_empty() {
            // rustls's buffer is full and the socket is not draining it.
            return Err(OasisError::Io(io::ErrorKind::WouldBlock.into()));
        }
        // `n` bytes are accepted whatever the socket does next; what it
        // cannot take now stays queued (see `flush_tls`).
        self.flush_tls()?;
        Ok(n)
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_tls()
    }

    fn close(&mut self) -> Result<()> {
        self.tls.send_close_notify();
        // `write` may have left ciphertext queued on a full socket (it
        // reported the plaintext as sent): give the socket a short,
        // bounded chance to take it and the close_notify before closing,
        // or a write followed straight by close loses the tail.
        let mut adapter = IoAdapter::new(&mut *self.inner);
        for _ in 0..CLOSE_FLUSH_ATTEMPTS {
            if !self.tls.wants_write() {
                break;
            }
            match self.tls.write_tls(&mut adapter) {
                Ok(0) => break,
                Ok(_) => {},
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    #[cfg(not(target_arch = "wasm32"))]
                    std::thread::sleep(std::time::Duration::from_millis(2));
                },
                Err(_) => break,
            }
        }
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
#[allow(clippy::unwrap_used)]
mod tests;
