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
                return Err(OasisError::Backend("mock TLS error".to_string()));
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

    #[cfg(feature = "tls-rustls")]
    #[test]
    fn rustls_provider_is_constructible() {
        let provider = crate::net::RustlsTlsProvider::new();
        let _: &dyn TlsProvider = &provider;
    }
}
