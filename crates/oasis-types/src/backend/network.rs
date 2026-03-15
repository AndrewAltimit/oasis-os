//! Network backend trait.

use crate::error::Result;

/// Network backend trait.
///
/// Abstracts TCP operations across sceNetInet (PSP) and std::net (Linux).
pub trait NetworkBackend {
    /// Start listening for incoming connections on the given port.
    fn listen(&mut self, port: u16) -> Result<()>;

    /// Accept a pending connection. Returns `None` if no connection waiting.
    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>>;

    /// Open an outbound TCP connection.
    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>>;

    /// Return the TLS provider for this backend, if available.
    ///
    /// When `Some`, the browser can negotiate HTTPS and Gemini connections.
    /// Backends without TLS support return `None` (the default).
    fn tls_provider(&self) -> Option<&dyn crate::tls::TlsProvider> {
        None
    }
}

/// A bidirectional byte stream (TCP connection).
pub trait NetworkStream: Send {
    /// Read up to `buf.len()` bytes into `buf`. Returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// Write `data` to the stream. Returns the number of bytes written.
    fn write(&mut self, data: &[u8]) -> Result<usize>;
    /// Close the connection and release resources.
    fn close(&mut self) -> Result<()>;
}
