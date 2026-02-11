//! Networking: std::net backend, remote terminal listener, outbound client,
//! and TLS provider abstraction.

mod client;
mod hosts;
mod listener;
mod std_backend;
pub mod tls;
#[cfg(feature = "tls-rustls")]
pub mod tls_rustls;

pub use client::{ClientState, RemoteClient};
pub use hosts::{HostEntry, parse_hosts};
pub use listener::{ListenerConfig, RemoteListener};
pub use std_backend::{StdNetworkBackend, StdNetworkStream};
pub use tls::TlsProvider;
#[cfg(feature = "tls-rustls")]
pub use tls_rustls::RustlsTlsProvider;

#[cfg(test)]
mod tests;
