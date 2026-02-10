//! NetworkBackend implementation for PSP.
//!
//! TCP client connections use `psp::net::resolve_hostname` for DNS and
//! raw `sceNetInet*` syscalls for the socket (because the high-level
//! `TcpStream` is `!Send`, but `NetworkStream` requires `Send`).
//!
//! Server sockets (listen/accept) use raw syscalls since the rust-psp
//! SDK does not provide a `TcpListener` type.

use std::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};

use psp::sys;

use oasis_core::backend::{NetworkBackend, NetworkStream};
use oasis_core::error::{OasisError, Result};

/// PSP socket option constants (BSD-compatible).
const SOL_SOCKET: i32 = 0xFFFF;
const SO_REUSEADDR: i32 = 0x0004;
const SO_NONBLOCK: i32 = 0x0080;

// ---------------------------------------------------------------------------
// Network initialization (lazy, one-shot)
// ---------------------------------------------------------------------------

/// Whether the network subsystem has been initialized.
static NET_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Public wrapper for use from the I/O thread.
pub(crate) fn ensure_net_init_pub() -> Result<()> {
    ensure_net_init()
}

/// Initialize the PSP network stack and connect to WiFi access point 1.
///
/// No-op if already initialized. Returns an error if WiFi is unavailable
/// or the connection fails.
fn ensure_net_init() -> Result<()> {
    if NET_INITIALIZED.load(Ordering::Acquire) {
        return Ok(());
    }

    if !psp::wlan::is_available() {
        return Err(OasisError::Backend(
            "WLAN not available (switch off or no hardware)".into(),
        ));
    }

    // 128 KiB memory pool for the networking stack.
    psp::net::init(0x20000).map_err(|e| OasisError::Backend(format!("net init failed: {e}")))?;

    // Connect to the first stored WiFi profile (30s timeout).
    if let Err(e) = psp::net::connect_ap(1) {
        psp::net::term();
        return Err(OasisError::Backend(format!("WiFi connect failed: {e}")));
    }

    NET_INITIALIZED.store(true, Ordering::Release);

    if let Ok(ip) = psp::net::get_ip_address() {
        let ip_str = core::str::from_utf8(&ip)
            .unwrap_or("?")
            .trim_end_matches('\0');
        psp::dprintln!("OASIS_OS: Network up, IP: {}", ip_str);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Raw sockaddr_in helper (mirrors psp::net internal make_sockaddr_in)
// ---------------------------------------------------------------------------

fn make_sockaddr_in(ip: [u8; 4], port: u16) -> sys::sockaddr {
    let mut sa = sys::sockaddr {
        sa_len: 16,
        sa_family: 2, // AF_INET
        sa_data: [0u8; 14],
    };
    let port_be = port.to_be_bytes();
    sa.sa_data[0] = port_be[0];
    sa.sa_data[1] = port_be[1];
    sa.sa_data[2] = ip[0];
    sa.sa_data[3] = ip[1];
    sa.sa_data[4] = ip[2];
    sa.sa_data[5] = ip[3];
    sa
}

/// Extract `(ip, port)` from a `sockaddr` returned by accept.
fn parse_sockaddr(sa: &sys::sockaddr) -> ([u8; 4], u16) {
    let port = u16::from_be_bytes([sa.sa_data[0], sa.sa_data[1]]);
    let ip = [sa.sa_data[2], sa.sa_data[3], sa.sa_data[4], sa.sa_data[5]];
    (ip, port)
}

// ---------------------------------------------------------------------------
// PspNetworkStream -- Send-safe TCP stream over raw fd
// ---------------------------------------------------------------------------

/// A TCP connection wrapping a raw PSP socket fd.
///
/// Implements `NetworkStream + Send`. The fd is closed on drop.
pub struct PspNetworkStream {
    fd: i32,
    closed: bool,
}

// SAFETY: PSP is single-core. The fd is an integer handle safe to move
// between logical threads (all run on the same core with cooperative
// scheduling). The `NetworkStream` trait requires `Send`.
unsafe impl Send for PspNetworkStream {}

impl PspNetworkStream {
    fn new(fd: i32) -> Self {
        Self { fd, closed: false }
    }
}

impl NetworkStream for PspNetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.closed {
            return Ok(0);
        }
        // SAFETY: sceNetInetRecv is the PSP inet recv syscall.
        // buf is a valid mutable slice.
        let ret =
            unsafe { sys::sceNetInetRecv(self.fd, buf.as_mut_ptr() as *mut c_void, buf.len(), 0) };
        if ret < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            Err(OasisError::Backend(format!(
                "recv failed: errno {:#x}",
                errno as u32,
            )))
        } else {
            Ok(ret as usize)
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<usize> {
        if self.closed {
            return Err(OasisError::Backend("stream closed".into()));
        }
        // SAFETY: sceNetInetSend is the PSP inet send syscall.
        // data is a valid byte slice.
        let ret =
            unsafe { sys::sceNetInetSend(self.fd, data.as_ptr() as *const c_void, data.len(), 0) };
        if ret < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            Err(OasisError::Backend(format!(
                "send failed: errno {:#x}",
                errno as u32,
            )))
        } else {
            Ok(ret as usize)
        }
    }

    fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            // SAFETY: Closes the socket fd. Only called once due to
            // the `closed` flag.
            unsafe { sys::sceNetInetClose(self.fd) };
        }
        Ok(())
    }
}

impl Drop for PspNetworkStream {
    fn drop(&mut self) {
        if !self.closed {
            // SAFETY: Last-resort cleanup. Socket fd is valid and
            // has not been closed yet.
            unsafe { sys::sceNetInetClose(self.fd) };
        }
    }
}

// ---------------------------------------------------------------------------
// PspNetworkBackend
// ---------------------------------------------------------------------------

/// PSP implementation of `NetworkBackend`.
///
/// Lazily initializes the network stack on first use. Server sockets
/// use raw `sceNetInet*` syscalls; client connections use
/// `psp::net::resolve_hostname` for DNS.
pub struct PspNetworkBackend {
    /// Listener socket fd, or -1 if not listening.
    listener_fd: i32,
}

impl PspNetworkBackend {
    pub fn new() -> Self {
        Self { listener_fd: -1 }
    }
}

impl NetworkBackend for PspNetworkBackend {
    fn listen(&mut self, port: u16) -> Result<()> {
        ensure_net_init()?;

        // Close any existing listener.
        if self.listener_fd >= 0 {
            // SAFETY: Closes the previous listener socket.
            unsafe { sys::sceNetInetClose(self.listener_fd) };
            self.listener_fd = -1;
        }

        // SAFETY: Create a TCP socket (AF_INET=2, SOCK_STREAM=1).
        let fd = unsafe { sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            return Err(OasisError::Backend(format!(
                "socket() failed: errno {:#x}",
                errno as u32,
            )));
        }

        // Set SO_REUSEADDR so we can rebind quickly.
        let one: i32 = 1;
        // SAFETY: sceNetInetSetsockopt with valid SOL_SOCKET options.
        unsafe {
            sys::sceNetInetSetsockopt(
                fd,
                SOL_SOCKET,
                SO_REUSEADDR,
                &one as *const i32 as *const c_void,
                mem::size_of::<i32>() as u32,
            );
        }

        // Set non-blocking so accept() doesn't block the caller.
        let nb: i32 = 1;
        // SAFETY: SO_NONBLOCK is a PSP-specific socket option.
        unsafe {
            sys::sceNetInetSetsockopt(
                fd,
                SOL_SOCKET,
                SO_NONBLOCK,
                &nb as *const i32 as *const c_void,
                mem::size_of::<i32>() as u32,
            );
        }

        let sa = make_sockaddr_in([0, 0, 0, 0], port);
        // SAFETY: Bind the socket to the given port on all interfaces.
        let ret = unsafe { sys::sceNetInetBind(fd, &sa, mem::size_of::<sys::sockaddr>() as u32) };
        if ret < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            unsafe { sys::sceNetInetClose(fd) };
            return Err(OasisError::Backend(format!(
                "bind(:{port}) failed: errno {:#x}",
                errno as u32,
            )));
        }

        // SAFETY: Start listening with a backlog of 4.
        let ret = unsafe { sys::sceNetInetListen(fd, 4) };
        if ret < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            unsafe { sys::sceNetInetClose(fd) };
            return Err(OasisError::Backend(format!(
                "listen(:{port}) failed: errno {:#x}",
                errno as u32,
            )));
        }

        self.listener_fd = fd;
        psp::dprintln!("OASIS_OS: Listening on port {}", port);
        Ok(())
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        if self.listener_fd < 0 {
            return Err(OasisError::Backend("not listening".into()));
        }

        let mut sa = sys::sockaddr {
            sa_len: 16,
            sa_family: 2,
            sa_data: [0u8; 14],
        };
        let mut sa_len = mem::size_of::<sys::sockaddr>() as u32;

        // SAFETY: Non-blocking accept. Returns -1 with EAGAIN/EWOULDBLOCK
        // if no connection is pending.
        let client_fd = unsafe { sys::sceNetInetAccept(self.listener_fd, &mut sa, &mut sa_len) };

        if client_fd < 0 {
            // EAGAIN / EWOULDBLOCK: no connection pending.
            let errno = unsafe { sys::sceNetInetGetErrno() };
            // EAGAIN = 11 (0xB), EWOULDBLOCK = 11 on PSP
            if errno == 0x0B || errno == 35 {
                return Ok(None);
            }
            return Err(OasisError::Backend(format!(
                "accept failed: errno {:#x}",
                errno as u32,
            )));
        }

        let (_ip, _port) = parse_sockaddr(&sa);
        Ok(Some(Box::new(PspNetworkStream::new(client_fd))))
    }

    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        ensure_net_init()?;

        // Resolve hostname to IPv4.
        let mut host_bytes: Vec<u8> = address.as_bytes().to_vec();
        host_bytes.push(0); // null-terminate for the resolver

        let addr = psp::net::resolve_hostname(&host_bytes)
            .map_err(|e| OasisError::Backend(format!("DNS resolve '{}' failed: {e}", address,)))?;

        // Create TCP socket and connect.
        // SAFETY: AF_INET=2, SOCK_STREAM=1.
        let fd = unsafe { sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            return Err(OasisError::Backend(format!(
                "socket() failed: errno {:#x}",
                errno as u32,
            )));
        }

        let sa = make_sockaddr_in(addr.0, port);
        // SAFETY: Connect to the resolved address.
        let ret =
            unsafe { sys::sceNetInetConnect(fd, &sa, mem::size_of::<sys::sockaddr>() as u32) };
        if ret < 0 {
            let errno = unsafe { sys::sceNetInetGetErrno() };
            unsafe { sys::sceNetInetClose(fd) };
            return Err(OasisError::Backend(format!(
                "connect {}:{} failed: errno {:#x}",
                address, port, errno as u32,
            )));
        }

        Ok(Box::new(PspNetworkStream::new(fd)))
    }
}

impl Drop for PspNetworkBackend {
    fn drop(&mut self) {
        if self.listener_fd >= 0 {
            // SAFETY: Close the listener socket on backend teardown.
            unsafe { sys::sceNetInetClose(self.listener_fd) };
        }
    }
}

// ---------------------------------------------------------------------------
// NetworkService (WiFi status for terminal commands)
// ---------------------------------------------------------------------------

use oasis_core::platform::{HttpResponse, NetworkService, WifiInfo};

/// PSP WiFi status service for terminal `wifi` command.
pub struct PspNetworkService;

impl NetworkService for PspNetworkService {
    fn http_get(&self, url: &str) -> Result<HttpResponse> {
        ensure_net_init()?;

        // Null-terminate the URL for PSP sceHttp APIs.
        let mut url_bytes: Vec<u8> = url.as_bytes().to_vec();
        url_bytes.push(0);

        let client = psp::http::HttpClient::new()
            .map_err(|e| OasisError::Backend(format!("HTTP init failed: {e}")))?;

        let resp = client
            .get(&url_bytes)
            .map_err(|e| OasisError::Backend(format!("HTTP GET failed: {e}")))?;

        Ok(HttpResponse {
            status_code: resp.status_code,
            body: resp.body,
        })
    }

    fn wifi_info(&self) -> Result<WifiInfo> {
        let wlan = psp::wlan::status();
        let available = wlan.power_on && wlan.switch_on;

        // Check if we're connected by querying the AP state.
        let connected = if available && NET_INITIALIZED.load(Ordering::Acquire) {
            let mut state = sys::ApctlState::Disconnected;
            // SAFETY: sceNetApctlGetState reads the current AP state.
            let ret = unsafe { sys::sceNetApctlGetState(&mut state) };
            ret >= 0 && matches!(state, sys::ApctlState::GotIp)
        } else {
            false
        };

        let ip_address = if connected {
            psp::net::get_ip_address().ok().and_then(|buf| {
                core::str::from_utf8(&buf)
                    .ok()
                    .map(|s| s.trim_end_matches('\0').to_string())
            })
        } else {
            None
        };

        Ok(WifiInfo {
            available,
            connected,
            ip_address,
            mac_address: wlan.mac_address,
        })
    }
}
