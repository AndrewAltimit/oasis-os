//! TLS provider for PSP using `embedded-tls`.
//!
//! Uses RustCrypto (pure Rust) for all cryptographic operations,
//! avoiding C/asm dependencies that would fail on mipsel-sony-psp.
//!
//! embedded-tls supports TLS 1.3 only, which is sufficient for
//! modern HTTPS and Gemini protocol connections.
//!
//! **Note:** This module cannot be tested in the main workspace
//! (requires mipsel-sony-psp target). Compilation is verified via
//! the PSP CI step.

use std::io::{self, Read, Write};

use oasis_core::backend::NetworkStream;
use oasis_core::error::{OasisError, Result};
use oasis_core::net::tls::TlsProvider;

/// TLS record buffer size (16 KiB + overhead for one full TLS record).
const RECORD_BUF_SIZE: usize = 16384 + 256;

/// PSP TLS provider using embedded-tls with RustCrypto.
pub struct PspTlsProvider;

impl PspTlsProvider {
    pub fn new() -> Self {
        Self
    }
}

// SAFETY: PSP is single-core with cooperative scheduling.
unsafe impl Send for PspTlsProvider {}
unsafe impl Sync for PspTlsProvider {}

impl TlsProvider for PspTlsProvider {
    fn connect_tls(
        &self,
        stream: Box<dyn NetworkStream>,
        server_name: &str,
    ) -> Result<Box<dyn NetworkStream>> {
        use embedded_tls::blocking::{Aes128GcmSha256, TlsConfig, TlsConnection, TlsContext};
        use embedded_tls::NoVerify;

        let mut adapter = IoAdapter(stream);

        // Pin buffers to the heap so they outlive the TlsConnection.
        let read_buf = Box::leak(vec![0u8; RECORD_BUF_SIZE].into_boxed_slice());
        let write_buf = Box::leak(vec![0u8; RECORD_BUF_SIZE].into_boxed_slice());

        let config = TlsConfig::new().with_server_name(server_name);

        let mut tls: TlsConnection<'_, IoAdapter, Aes128GcmSha256> =
            TlsConnection::new(adapter, read_buf, write_buf);

        // NoVerify: PSP has no certificate store. For production, add
        // certificate pinning for specific hosts.
        let context = TlsContext::new(&config, NoVerify);

        if let Err(e) = tls.open(context) {
            // Handshake failed -- drop TLS (releases borrow on buffers)
            // then reclaim the leaked buffers to avoid a memory leak.
            drop(tls);
            // SAFETY: buffers were created via Box::leak above and are
            // not borrowed after dropping `tls`.
            unsafe {
                let _ = Box::from_raw(read_buf);
                let _ = Box::from_raw(write_buf);
            }
            return Err(OasisError::Backend(format!("TLS handshake failed: {:?}", e)));
        }

        Ok(Box::new(PspTlsStream {
            tls: Some(tls),
            read_buf: read_buf as *mut [u8],
            write_buf: write_buf as *mut [u8],
        }))
    }
}

/// A TLS-wrapped network stream for PSP.
///
/// Owns the embedded-tls connection and heap-pinned record buffers.
/// Buffers are freed on drop via `Box::from_raw`.
struct PspTlsStream<'a> {
    tls: Option<
        embedded_tls::blocking::TlsConnection<'a, IoAdapter, embedded_tls::blocking::Aes128GcmSha256>,
    >,
    /// Raw pointer to heap-allocated read buffer (freed on drop).
    read_buf: *mut [u8],
    /// Raw pointer to heap-allocated write buffer (freed on drop).
    write_buf: *mut [u8],
}

// SAFETY: PSP is single-core. See PspNetworkStream safety comment.
unsafe impl Send for PspTlsStream<'_> {}

impl NetworkStream for PspTlsStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let tls = self.tls.as_mut().ok_or_else(|| {
            OasisError::Backend("TLS connection closed".to_string())
        })?;
        tls.read(buf)
            .map_err(|e| OasisError::Backend(format!("TLS read: {:?}", e)))
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let tls = self.tls.as_mut().ok_or_else(|| {
            OasisError::Backend("TLS connection closed".to_string())
        })?;
        tls.write(data)
            .map_err(|e| OasisError::Backend(format!("TLS write: {:?}", e)))
    }

    fn close(&mut self) -> Result<()> {
        // embedded-tls does NOT send close_notify on drop.  The PSP
        // environment is single-core with cooperative scheduling and
        // typically connects to Gemini capsules over LAN/WAN where an
        // abrupt TCP close is acceptable.  A proper close_notify would
        // require an explicit `close_notify()` API that embedded-tls
        // currently does not expose.
        self.tls.take();
        Ok(())
    }
}

impl Drop for PspTlsStream<'_> {
    fn drop(&mut self) {
        // Drop TLS connection first (it borrows the buffers).
        self.tls.take();
        // SAFETY: read_buf and write_buf were created via Box::leak
        // in connect_tls and are only freed here, exactly once.
        unsafe {
            let _ = Box::from_raw(self.read_buf);
            let _ = Box::from_raw(self.write_buf);
        }
    }
}

// ---------------------------------------------------------------------------
// IoAdapter: bridge NetworkStream to std::io::Read + Write
// ---------------------------------------------------------------------------

/// Wraps a `Box<dyn NetworkStream>` as `std::io::Read` + `std::io::Write`
/// for embedded-tls.
struct IoAdapter(Box<dyn NetworkStream>);

impl Read for IoAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .read(buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

impl Write for IoAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .write(buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
