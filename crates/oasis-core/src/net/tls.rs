//! TLS provider abstraction.
//!
//! Re-exports the [`TlsProvider`] trait from `oasis-types`.

pub use oasis_types::tls::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "tls-rustls")]
    #[test]
    fn rustls_provider_is_constructible() {
        let provider = crate::net::RustlsTlsProvider::new();
        let _: &dyn TlsProvider = &provider;
    }
}
