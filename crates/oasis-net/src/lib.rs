//! Networking: std::net backend, remote terminal listener, outbound client,
//! TLS provider abstraction, and (feature `vfs`) the FTP-like file
//! transfer server.
//!
//! # Feature flags
//!
//! - `tls-rustls` — `tls_rustls::RustlsTlsProvider`: TLS
//!   1.3 client via rustls with the webpki-roots CA bundle. Without it outbound
//!   connections are plaintext TCP only.
//! - `vfs` — the FTP-like file transfer server (`ftp` module), serving a `Vfs`.
//!   Enabled by `oasis-core` for its `ftp` / `push` / `pull` commands.

#![warn(missing_docs)]

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

pub use auth::{NONCE_HEX_LEN, is_valid_nonce, psk_response, verify_auth_line};
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
