//! Networking: std::net backend, remote terminal listener, outbound client,
//! and TLS provider abstraction.

mod client;
mod hosts;
#[cfg(not(target_arch = "wasm32"))]
mod listener;
#[cfg(not(target_arch = "wasm32"))]
mod std_backend;
pub mod tls;
#[cfg(all(feature = "tls-rustls", not(target_arch = "wasm32")))]
pub mod tls_rustls;

pub use client::{ClientState, RemoteClient};
pub use hosts::{HostEntry, parse_hosts};
#[cfg(not(target_arch = "wasm32"))]
pub use listener::{ListenerConfig, RemoteListener};
#[cfg(not(target_arch = "wasm32"))]
pub use std_backend::{StdNetworkBackend, StdNetworkStream};
pub use tls::TlsProvider;
#[cfg(all(feature = "tls-rustls", not(target_arch = "wasm32")))]
pub use tls_rustls::RustlsTlsProvider;

#[cfg(test)]
mod tests;
