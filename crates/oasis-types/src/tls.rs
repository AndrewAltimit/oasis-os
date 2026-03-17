//! TLS provider abstraction.
//!
//! Backends supply a [`TlsProvider`] that wraps a plain TCP
//! [`NetworkStream`] in a TLS session.  The browser and networking
//! code use this trait so they never depend on a concrete TLS library.

use crate::backend::NetworkStream;
use crate::error::Result;

/// Provides TLS client connections.
///
/// Each platform backend implements this with its preferred TLS library
/// (e.g. rustls on desktop, embedded-tls on PSP).
pub trait TlsProvider: Send + Sync {
    /// Wrap `stream` in a TLS client session, performing the handshake.
    ///
    /// `server_name` is used for SNI and certificate verification.
    fn connect_tls(
        &self,
        stream: Box<dyn NetworkStream>,
        server_name: &str,
    ) -> Result<Box<dyn NetworkStream>>;

    /// Open a raw TCP connection to `host:port`.
    ///
    /// The default implementation uses `std::net::TcpStream` which works
    /// on desktop. PSP overrides this with raw `sceNetInet*` sockets
    /// since `std::net` is not supported on that platform.
    fn connect_tcp(&self, host: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        use std::io::{self, Read, Write};
        use std::net::ToSocketAddrs;
        use std::time::Duration;

        let addr = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(|e| {
                crate::error::OasisError::Backend(format!("DNS resolution failed: {e}").into())
            })?
            .next()
            .ok_or_else(|| {
                crate::error::OasisError::Backend(format!("no addresses for {host}:{port}").into())
            })?;

        let stream =
            std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(15)).map_err(|e| {
                crate::error::OasisError::Backend(
                    format!("TCP connect to {host}:{port}: {e}").into(),
                )
            })?;

        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        // Minimal NetworkStream wrapper for std::net::TcpStream.
        struct TcpNetStream(std::net::TcpStream);
        impl NetworkStream for TcpNetStream {
            fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
                Read::read(&mut self.0, buf).map_err(|e| {
                    crate::error::OasisError::Io(io::Error::new(e.kind(), e.to_string()))
                })
            }
            fn write(&mut self, data: &[u8]) -> Result<usize> {
                Write::write(&mut self.0, data).map_err(|e| {
                    crate::error::OasisError::Io(io::Error::new(e.kind(), e.to_string()))
                })
            }
            fn close(&mut self) -> Result<()> {
                self.0.shutdown(std::net::Shutdown::Both).ok();
                Ok(())
            }
        }

        Ok(Box::new(TcpNetStream(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::OasisError;

    /// A mock TLS provider that echoes data back with a "TLS:" prefix.
    struct MockTlsProvider;

    impl TlsProvider for MockTlsProvider {
        fn connect_tls(
            &self,
            _stream: Box<dyn NetworkStream>,
            server_name: &str,
        ) -> Result<Box<dyn NetworkStream>> {
            if server_name == "bad.example.com" {
                return Err(OasisError::Backend("mock TLS error".into()));
            }
            Ok(_stream) // pass-through for testing
        }
    }

    #[test]
    fn trait_is_object_safe_and_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockTlsProvider>();

        // Verify it can be used as a trait object.
        let provider = MockTlsProvider;
        let _: &dyn TlsProvider = &provider;
    }
}
