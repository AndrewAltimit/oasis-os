//! `std::net` implementation of `NetworkBackend` and `NetworkStream`.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

/// Default timeout for establishing an outbound TCP connection (per address).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Network backend using `std::net` for desktop and Raspberry Pi.
pub struct StdNetworkBackend {
    listener: Option<TcpListener>,
    connect_timeout: Duration,
    #[cfg(feature = "tls-rustls")]
    tls: super::tls_rustls::RustlsTlsProvider,
}

impl StdNetworkBackend {
    pub fn new() -> Self {
        Self {
            listener: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            #[cfg(feature = "tls-rustls")]
            tls: super::tls_rustls::RustlsTlsProvider::new(),
        }
    }

    /// Create a backend sharing an existing TLS provider (cheap `Arc` bump).
    #[cfg(feature = "tls-rustls")]
    pub fn with_tls(tls: super::tls_rustls::RustlsTlsProvider) -> Self {
        Self {
            listener: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            tls,
        }
    }

    /// Start listening on `127.0.0.1:{port}` only (loopback).
    ///
    /// Unlike [`NetworkBackend::listen`], which binds `0.0.0.0` (all
    /// interfaces), this restricts the listener to local connections. Used by
    /// the optional MCP control server and by the remote terminal when no PSK
    /// is configured, so neither is exposed to the network.
    pub fn listen_loopback(&mut self, port: u16) -> Result<()> {
        self.listen_on(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    /// Start a non-blocking listener on `ip:port`.
    pub fn listen_on(&mut self, ip: IpAddr, port: u16) -> Result<()> {
        let addr = SocketAddr::new(ip, port);
        let listener = TcpListener::bind(addr)
            .map_err(|e| OasisError::Backend(format!("bind: {e}").into()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| OasisError::Backend(format!("set_nonblocking: {e}").into()))?;
        log::info!("Listening on {addr}");
        self.listener = Some(listener);
        Ok(())
    }

    /// Address the current listener is bound to, if any.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Set the per-address timeout used by [`NetworkBackend::connect`]
    /// (default [`DEFAULT_CONNECT_TIMEOUT`]).
    pub fn set_connect_timeout(&mut self, timeout: Duration) {
        self.connect_timeout = timeout;
    }
}

/// Resolve `address:port` and connect to the first resolved address that
/// accepts within `timeout`. `address` may be a hostname, an IPv4 literal, or
/// an IPv6 literal with or without brackets.
fn connect_with_timeout(address: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let host = address
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(address);
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| OasisError::Backend(format!("resolve {address}: {e}").into()))?;
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }
    Err(match last_err {
        Some(e) => OasisError::Backend(format!("connect {address}:{port}: {e}").into()),
        None => OasisError::Backend(format!("resolve {address}: no addresses").into()),
    })
}

impl Default for StdNetworkBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBackend for StdNetworkBackend {
    #[cfg(feature = "tls-rustls")]
    fn tls_provider(&self) -> Option<&dyn super::tls::TlsProvider> {
        Some(&self.tls)
    }

    /// Bind `0.0.0.0:{port}` (all interfaces). Servers that do not
    /// authenticate their peers should use [`StdNetworkBackend::listen_loopback`].
    fn listen(&mut self, port: u16) -> Result<()> {
        self.listen_on(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
    }

    fn listen_loopback(&mut self, port: u16) -> Result<()> {
        self.listen_on(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        let Some(ref listener) = self.listener else {
            return Err(OasisError::Backend("not listening".into()));
        };
        match listener.accept() {
            Ok((stream, addr)) => {
                log::info!("Remote connection from {addr}");
                stream
                    .set_nonblocking(true)
                    .map_err(|e| OasisError::Backend(format!("set_nonblocking: {e}").into()))?;
                if let Err(e) = stream.set_nodelay(true) {
                    log::warn!("set_nodelay failed for {addr}: {e}");
                }
                Ok(Some(Box::new(StdNetworkStream::new(stream))))
            },
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(OasisError::Backend(format!("accept: {e}").into())),
        }
    }

    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        let addr = format!("{address}:{port}");
        let stream = connect_with_timeout(address, port, self.connect_timeout)?;
        stream
            .set_nonblocking(true)
            .map_err(|e| OasisError::Backend(format!("set_nonblocking: {e}").into()))?;
        // Disable Nagle: streaming/RPC traffic is latency-sensitive and the
        // request/response patterns here (HTTP headers, TLS handshakes, ICY
        // polls) otherwise wait out delayed-ACK round trips.
        if let Err(e) = stream.set_nodelay(true) {
            log::warn!("set_nodelay failed for {addr}: {e}");
        }
        log::info!("Connected to {addr}");
        Ok(Box::new(StdNetworkStream::new(stream)))
    }
}

/// A TCP stream wrapping `std::net::TcpStream`.
pub struct StdNetworkStream {
    stream: TcpStream,
}

impl StdNetworkStream {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }
}

impl NetworkStream for StdNetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.stream.read(buf).map_err(OasisError::Io)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.stream.write(data).map_err(OasisError::Io)
    }

    fn close(&mut self) -> Result<()> {
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .map_err(|e| OasisError::Backend(format!("close: {e}").into()))
    }
}

// Implement Send for StdNetworkStream (TcpStream is Send).
// NetworkStream requires Send, which TcpStream satisfies.
