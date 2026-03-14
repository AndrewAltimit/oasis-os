//! SDL backend network module.
//!
//! Re-exports `StdNetworkBackend` from `oasis-net` as the SDL
//! backend's `NetworkBackend` implementation. On desktop and Raspberry
//! Pi, all TCP and TLS operations use `std::net`.
//!
//! This module adds a thin wrapper (`SdlNetworkBackend`) that delegates
//! to `StdNetworkBackend` so the SDL crate has a named type it owns.

use oasis_core::backend::{NetworkBackend, NetworkStream};
use oasis_core::error::Result;

/// SDL network backend wrapping [`oasis_core::net::StdNetworkBackend`].
///
/// All methods delegate to the inner `StdNetworkBackend` which uses
/// `std::net` for TCP and (when the `tls-rustls` feature is active)
/// `rustls` for TLS.
pub struct SdlNetworkBackend {
    inner: oasis_core::net::StdNetworkBackend,
}

impl SdlNetworkBackend {
    /// Create a new network backend.
    pub fn new() -> Self {
        Self {
            inner: oasis_core::net::StdNetworkBackend::new(),
        }
    }
}

impl Default for SdlNetworkBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBackend for SdlNetworkBackend {
    fn listen(&mut self, port: u16) -> Result<()> {
        self.inner.listen(port)
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        self.inner.accept()
    }

    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        self.inner.connect(address, port)
    }

    fn tls_provider(&self) -> Option<&dyn oasis_core::tls::TlsProvider> {
        self.inner.tls_provider()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// Find a free TCP port by binding to port 0 and releasing it.
    fn free_port() -> u16 {
        let tmp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tmp.local_addr().unwrap().port();
        drop(tmp);
        port
    }

    #[test]
    fn new_creates_backend() {
        let _backend = SdlNetworkBackend::new();
    }

    #[test]
    fn default_creates_backend() {
        let _backend = SdlNetworkBackend::default();
    }

    #[test]
    fn listen_succeeds_on_free_port() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();
    }

    #[test]
    fn accept_returns_none_when_no_connection() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();
        let result = backend.accept().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn accept_without_listen_errors() {
        let mut backend = SdlNetworkBackend::new();
        assert!(backend.accept().is_err());
    }

    #[test]
    fn listen_and_accept_connection() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        // Connect a client.
        let _client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let stream = backend.accept().unwrap();
        assert!(stream.is_some());
    }

    #[test]
    fn server_reads_client_data() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        client.write_all(b"hello").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = backend.accept().unwrap().unwrap();
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn server_writes_to_client() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        // Set a read timeout so the test doesn't hang under ASAN.
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let mut stream = backend.accept().unwrap().unwrap();
        stream.write(b"world").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let mut buf = [0u8; 64];
        let n = client.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"world");
    }

    #[test]
    fn connect_outbound() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.write_all(b"greeting").unwrap();
        });

        let mut backend = SdlNetworkBackend::new();
        let mut stream = backend.connect("127.0.0.1", port).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"greeting");

        stream.close().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn connect_to_invalid_address_fails() {
        let mut backend = SdlNetworkBackend::new();
        // Port 1 is almost certainly not listening.
        let result = backend.connect("127.0.0.1", 1);
        assert!(result.is_err());
    }

    #[test]
    fn stream_close() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let _client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = backend.accept().unwrap().unwrap();
        stream.close().unwrap();
    }

    #[test]
    fn bidirectional_data_exchange() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        client.write_all(b"ping").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = backend.accept().unwrap().unwrap();
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");

        stream.write(b"pong").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let n = client.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"pong");
    }

    #[test]
    fn multiple_accepts() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let _c1 = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let s1 = backend.accept().unwrap();
        assert!(s1.is_some());

        let _c2 = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let s2 = backend.accept().unwrap();
        assert!(s2.is_some());
    }

    #[test]
    fn tls_provider_is_available() {
        let backend = SdlNetworkBackend::new();
        // With tls-rustls feature enabled, should return Some.
        assert!(backend.tls_provider().is_some());
    }

    #[test]
    fn write_returns_byte_count() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let _client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = backend.accept().unwrap().unwrap();
        let n = stream.write(b"test data").unwrap();
        assert_eq!(n, 9);
    }

    #[test]
    fn read_with_no_data_returns_zero_or_would_block() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let _client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = backend.accept().unwrap().unwrap();
        let mut buf = [0u8; 64];
        // Non-blocking socket: read returns Ok(0) or WouldBlock.
        let read_result = stream.read(&mut buf);
        assert!(
            matches!(&read_result, Ok(0) | Err(_)),
            "expected 0 or error, got {read_result:?}"
        );
    }

    #[test]
    fn large_data_transfer() {
        let mut backend = SdlNetworkBackend::new();
        let port = free_port();
        backend.listen(port).unwrap();

        let handle = std::thread::spawn(move || {
            let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let payload = vec![0xABu8; 4096];
            client.write_all(&payload).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut stream = backend.accept().unwrap().unwrap();

        let mut received = Vec::new();
        let mut buf = [0u8; 1024];
        while received.len() < 4096 {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        assert_eq!(received.len(), 4096);
        assert!(received.iter().all(|&b| b == 0xAB));

        handle.join().unwrap();
    }

    #[test]
    fn connect_write_close_cycle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = conn.read(&mut buf);
        });

        let mut backend = SdlNetworkBackend::new();
        let mut stream = backend.connect("127.0.0.1", port).unwrap();
        stream.write(b"data").unwrap();
        stream.close().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn listen_on_two_ports() {
        let mut b1 = SdlNetworkBackend::new();
        let mut b2 = SdlNetworkBackend::new();
        let p1 = free_port();
        let p2 = free_port();
        b1.listen(p1).unwrap();
        b2.listen(p2).unwrap();

        let _c1 = TcpStream::connect(format!("127.0.0.1:{p1}")).unwrap();
        let _c2 = TcpStream::connect(format!("127.0.0.1:{p2}")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        assert!(b1.accept().unwrap().is_some());
        assert!(b2.accept().unwrap().is_some());
    }
}
