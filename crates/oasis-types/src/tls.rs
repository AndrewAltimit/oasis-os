//! TLS provider abstraction.
//!
//! Backends supply a [`TlsProvider`] that wraps a plain TCP
//! [`NetworkStream`] in a TLS session.  The browser and networking
//! code use this trait so they never depend on a concrete TLS library.

use std::net::IpAddr;

use crate::backend::NetworkStream;
use crate::error::{OasisError, Result};

/// Result of a TLS handshake that carried an ALPN offer.
///
/// `alpn` holds the server-selected protocol bytes (e.g. `b"h2"` or
/// `b"http/1.1"`), or `None` if the server did not pick anything.
pub struct TlsConnection {
    pub stream: Box<dyn NetworkStream>,
    pub alpn: Option<Vec<u8>>,
}

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

    /// Wrap `stream` in a TLS client session and offer the given
    /// ALPN protocols during the handshake.
    ///
    /// The default implementation falls back to [`Self::connect_tls`]
    /// and reports no ALPN selection — providers that support ALPN
    /// (rustls, embedded-tls) override this to actually negotiate.
    fn connect_tls_with_alpn(
        &self,
        stream: Box<dyn NetworkStream>,
        server_name: &str,
        _alpn_protocols: &[&[u8]],
    ) -> Result<TlsConnection> {
        let s = self.connect_tls(stream, server_name)?;
        Ok(TlsConnection {
            stream: s,
            alpn: None,
        })
    }

    /// Resolve `host` to its addresses (empty when it cannot be resolved).
    ///
    /// The default uses the std resolver. PSP overrides it with the
    /// firmware resolver, so callers that classify a host before
    /// connecting (the browser's private-network check) see the same
    /// addresses the connection will use.
    fn resolve_host(&self, host: &str) -> Vec<IpAddr> {
        use std::net::ToSocketAddrs;
        (host, 0)
            .to_socket_addrs()
            .map(|addrs| addrs.map(|a| a.ip()).collect())
            .unwrap_or_default()
    }

    /// Open a raw TCP connection to `host:port`.
    ///
    /// The default implementation uses `std::net::TcpStream` which works
    /// on desktop. PSP overrides this with raw `sceNetInet*` sockets
    /// since `std::net` is not supported on that platform.
    fn connect_tcp(&self, host: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        self.connect_tcp_checked(host, port, &|_| true)
    }

    /// Like [`connect_tcp`](Self::connect_tcp), but dial only an address
    /// `addr_ok` accepts. The host is resolved once and the checked
    /// address is the one connected to, so a DNS answer cannot change
    /// between the check and the connect (rebinding). Errors when no
    /// resolved address passes.
    fn connect_tcp_checked(
        &self,
        host: &str,
        port: u16,
        addr_ok: &dyn Fn(IpAddr) -> bool,
    ) -> Result<Box<dyn NetworkStream>> {
        use std::io::{self, Read, Write};
        use std::net::ToSocketAddrs;
        use std::time::Duration;

        let addrs: Vec<_> = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(|e| OasisError::Backend(format!("DNS resolution failed: {e}").into()))?
            .collect();
        if addrs.is_empty() {
            return Err(OasisError::Backend(
                format!("no addresses for {host}:{port}").into(),
            ));
        }
        let addr = addrs.into_iter().find(|a| addr_ok(a.ip())).ok_or_else(|| {
            OasisError::Backend(format!("{host}:{port} blocked by request policy").into())
        })?;

        let stream =
            std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(15)).map_err(|e| {
                crate::error::OasisError::Backend(
                    format!("TCP connect to {host}:{port}: {e}").into(),
                )
            })?;

        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        // Minimal NetworkStream wrapper for std::net::TcpStream.
        //
        // The socket is blocking with a 30 s read timeout, so an expired
        // timeout is a dead/silent peer, never "no data yet". Linux reports
        // it as WouldBlock (EAGAIN) and Windows as TimedOut; normalise both
        // to TimedOut so `OasisError::is_would_block` callers do not retry
        // a dead connection forever.
        struct TcpNetStream(std::net::TcpStream);
        impl NetworkStream for TcpNetStream {
            fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
                Read::read(&mut self.0, buf).map_err(|e| {
                    let kind = match e.kind() {
                        io::ErrorKind::WouldBlock => io::ErrorKind::TimedOut,
                        k => k,
                    };
                    crate::error::OasisError::Io(io::Error::new(kind, e.to_string()))
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
    fn checked_connect_refuses_a_rejected_address_before_dialing() {
        // `localhost` resolves without a network; rejecting every
        // address must fail before any socket is opened.
        let err = MockTlsProvider
            .connect_tcp_checked("localhost", 9, &|_| false)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("blocked by request policy"), "{err}");
    }

    #[test]
    fn default_resolver_classifies_literals_and_localhost() {
        let ips = MockTlsProvider.resolve_host("127.0.0.1");
        assert_eq!(ips, vec![IpAddr::from([127, 0, 0, 1])]);
        assert!(
            MockTlsProvider
                .resolve_host("localhost")
                .iter()
                .all(|ip| ip.is_loopback())
        );
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
