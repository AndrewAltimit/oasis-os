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

use oasis_core::backend::NetworkStream;
use oasis_core::error::{OasisError, Result};
use oasis_core::net::tls::TlsProvider;

/// TLS record buffer size (16 KiB + overhead for one full TLS record).
const RECORD_BUF_SIZE: usize = 16384 + 256;

// ---------------------------------------------------------------------------
// PspRng: rand_core 0.6 RNG backed by PSP's MT19937 PRNG
// ---------------------------------------------------------------------------

/// RNG using PSP's hardware MT19937 PRNG (via sceKernelUtils).
///
/// MT19937 is deterministic from its seed, not cryptographically secure in the
/// formal sense, but acceptable for TLS on PSP where no better hardware entropy
/// source exists. The seed is taken from the CPU cycle counter (`$9` / Count
/// register), which provides reasonable per-session uniqueness.
struct PspRng {
    ctx: psp::sys::SceKernelUtilsMt19937Context,
}

impl PspRng {
    fn new() -> Self {
        // SAFETY: MT19937 context is stack-local, seed from CPU cycle counter.
        unsafe {
            let mut ctx = core::mem::zeroed();
            let seed: u32;
            core::arch::asm!("mfc0 {}, $9", out(reg) seed);
            psp::sys::sceKernelUtilsMt19937Init(&mut ctx, seed);
            Self { ctx }
        }
    }
}

impl rand_core::RngCore for PspRng {
    fn next_u32(&mut self) -> u32 {
        // SAFETY: ctx was initialized in new().
        unsafe { psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx) }
    }

    fn next_u64(&mut self) -> u64 {
        let lo = self.next_u32() as u64;
        let hi = self.next_u32() as u64;
        (hi << 32) | lo
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // SAFETY: ctx was initialized in new().
        unsafe {
            for byte in dest.iter_mut() {
                *byte =
                    (psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx) & 0xFF) as u8;
            }
        }
    }

    fn try_fill_bytes(
        &mut self,
        dest: &mut [u8],
    ) -> core::result::Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

// SAFETY: MT19937 is the best PRNG available on PSP hardware.
impl rand_core::CryptoRng for PspRng {}

// ---------------------------------------------------------------------------
// IoAdapter: bridge NetworkStream to embedded_io::Read + Write
// ---------------------------------------------------------------------------

/// Wraps a `Box<dyn NetworkStream>` as `embedded_io::Read` +
/// `embedded_io::Write` for embedded-tls.
struct IoAdapter(Box<dyn NetworkStream>);

impl embedded_io::ErrorType for IoAdapter {
    type Error = std::io::Error;
}

impl embedded_io::Read for IoAdapter {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        self.0
            .read(buf)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

impl embedded_io::Write for IoAdapter {
    fn write(&mut self, buf: &[u8]) -> core::result::Result<usize, Self::Error> {
        self.0
            .write(buf)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn flush(&mut self) -> core::result::Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PspTlsProvider
// ---------------------------------------------------------------------------

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
        use embedded_tls::UnsecureProvider;

        let adapter = IoAdapter(stream);

        // Pin buffers to the heap so they outlive the TlsConnection.
        let read_buf =
            Box::leak(vec![0u8; RECORD_BUF_SIZE].into_boxed_slice());
        let write_buf =
            Box::leak(vec![0u8; RECORD_BUF_SIZE].into_boxed_slice());

        // Save raw pointers before TlsConnection borrows the slices.
        let read_ptr: *mut [u8] = read_buf;
        let write_ptr: *mut [u8] = write_buf;

        let config = TlsConfig::new().with_server_name(server_name);

        let mut tls: TlsConnection<'_, IoAdapter, Aes128GcmSha256> =
            TlsConnection::new(adapter, read_buf, write_buf);

        // UnsecureProvider: PSP has no certificate store. For production,
        // add certificate pinning for specific hosts.
        let context = TlsContext::new(
            &config,
            UnsecureProvider::new::<Aes128GcmSha256>(PspRng::new()),
        );

        if let Err(e) = tls.open(context) {
            // Handshake failed -- drop TLS (releases borrow on buffers)
            // then reclaim the leaked buffers to avoid a memory leak.
            drop(tls);
            // SAFETY: buffers were created via Box::leak above and are
            // not borrowed after dropping `tls`.
            unsafe {
                let _ = Box::from_raw(read_ptr);
                let _ = Box::from_raw(write_ptr);
            }
            return Err(OasisError::Backend(format!(
                "TLS handshake failed: {:?}",
                e
            )));
        }

        Ok(Box::new(PspTlsStream {
            tls: Some(tls),
            read_buf: read_ptr,
            write_buf: write_ptr,
        }))
    }
}

// ---------------------------------------------------------------------------
// PspTlsStream
// ---------------------------------------------------------------------------

/// A TLS-wrapped network stream for PSP.
///
/// Owns the embedded-tls connection and heap-pinned record buffers.
/// Buffers are freed on drop via `Box::from_raw`.
struct PspTlsStream<'a> {
    tls: Option<
        embedded_tls::blocking::TlsConnection<
            'a,
            IoAdapter,
            embedded_tls::blocking::Aes128GcmSha256,
        >,
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
        embedded_io::Read::read(tls, buf)
            .map_err(|e| OasisError::Backend(format!("TLS read: {:?}", e)))
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let tls = self.tls.as_mut().ok_or_else(|| {
            OasisError::Backend("TLS connection closed".to_string())
        })?;
        embedded_io::Write::write(tls, data)
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
