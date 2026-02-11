//! [`TlsProvider`] backed by rustls + ring.
//!
//! Enabled by the `tls-rustls` feature.  Desktop and Pi builds use this
//! provider; the PSP backend supplies its own via `embedded-tls`.

use std::io::{self, Read, Write};
use std::sync::Arc;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;

use crate::backend::NetworkStream;
use crate::error::{OasisError, Result};

use super::tls::TlsProvider;

/// Shared, reusable TLS client configuration (one per process).
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

        Ok(Box::new(RustlsStream::new(conn, stream)))
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
    plaintext_buf: Vec<u8>,
}

impl RustlsStream {
    fn new(mut tls: rustls::ClientConnection, mut inner: Box<dyn NetworkStream>) -> Self {
        // Perform the TLS handshake eagerly so callers get a ready stream.
        // rustls is lazy -- we pump I/O until the handshake completes.
        let mut adapter = IoAdapter::new(&mut *inner);
        while tls.is_handshaking() {
            if tls.wants_write() {
                // Ignore WouldBlock during handshake -- keep trying.
                let _ = tls.write_tls(&mut adapter);
            }
            if tls.wants_read() {
                match tls.read_tls(&mut adapter) {
                    Ok(0) => break, // EOF from peer
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // Non-blocking socket has nothing yet -- spin.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    },
                    Err(_) => break,
                    Ok(_) => {},
                }
                if let Err(_e) = tls.process_new_packets() {
                    break;
                }
            }
        }
        // Flush any remaining handshake bytes.
        let _ = tls.write_tls(&mut adapter);

        Self {
            tls,
            inner,
            plaintext_buf: Vec::new(),
        }
    }

    /// Pump ciphertext from the network into rustls and move any resulting
    /// plaintext into `self.plaintext_buf`.
    fn pull_plaintext(&mut self) -> Result<()> {
        let mut adapter = IoAdapter::new(&mut *self.inner);

        // Read ciphertext from the network.
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

        // Drain plaintext into our buffer.
        let mut tmp = [0u8; 8192];
        loop {
            match self.tls.reader().read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => self.plaintext_buf.extend_from_slice(&tmp[..n]),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        Ok(())
    }
}

impl NetworkStream for RustlsStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Return buffered plaintext first.
        if !self.plaintext_buf.is_empty() {
            let n = buf.len().min(self.plaintext_buf.len());
            buf[..n].copy_from_slice(&self.plaintext_buf[..n]);
            self.plaintext_buf.drain(..n);
            return Ok(n);
        }

        // Try to get more plaintext from the network.
        self.pull_plaintext()?;

        if self.plaintext_buf.is_empty() {
            return Ok(0); // Nothing available (non-blocking).
        }

        let n = buf.len().min(self.plaintext_buf.len());
        buf[..n].copy_from_slice(&self.plaintext_buf[..n]);
        self.plaintext_buf.drain(..n);
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

// Send is safe: rustls::ClientConnection is Send, and our inner stream
// is already Send (required by NetworkStream).
// SAFETY: RustlsStream owns all its fields and delegates Send to them.
unsafe impl Send for RustlsStream {}

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
        self.inner
            .read(buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

impl io::Write for IoAdapter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner
            .write(buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
