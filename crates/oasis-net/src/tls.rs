//! TLS provider abstraction.
//!
//! Re-exports the [`TlsProvider`] trait from `oasis-types`.

pub use oasis_types::tls::*;

#[cfg(all(test, feature = "tls-rustls"))]
mod tests {
    use super::*;

    #[test]
    fn rustls_provider_is_constructible() {
        let provider = crate::tls_rustls::RustlsTlsProvider::new();
        let _: &dyn TlsProvider = &provider;
    }
}
