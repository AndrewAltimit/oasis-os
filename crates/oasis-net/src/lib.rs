//! Networking: std::net backend, remote terminal listener, outbound client,
//! TLS provider abstraction, and (feature `vfs`) the FTP-like file
//! transfer server.

pub mod auth;
mod client;
#[cfg(feature = "vfs")]
pub mod ftp;
mod hosts;
#[cfg(not(target_arch = "wasm32"))]
mod listener;
#[cfg(not(target_arch = "wasm32"))]
mod std_backend;
pub mod tls;
#[cfg(all(feature = "tls-rustls", not(target_arch = "wasm32")))]
pub mod tls_rustls;

pub use auth::{psk_response, verify_auth_line};
pub use client::{ClientState, RemoteClient};
pub use hosts::{HostEntry, parse_hosts};
#[cfg(not(target_arch = "wasm32"))]
pub use listener::{ListenerBind, ListenerConfig, RemoteListener};
#[cfg(not(target_arch = "wasm32"))]
pub use std_backend::{DEFAULT_CONNECT_TIMEOUT, StdNetworkBackend, StdNetworkStream};
pub use tls::TlsProvider;
#[cfg(all(feature = "tls-rustls", not(target_arch = "wasm32")))]
pub use tls_rustls::RustlsTlsProvider;

#[cfg(test)]
mod tests;
