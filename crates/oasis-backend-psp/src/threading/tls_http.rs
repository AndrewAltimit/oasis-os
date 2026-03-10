//! HTTPS support via raw TCP + embedded-tls (TLS 1.3).
//!
//! PSP's sceHttp SSL stack uses firmware root CAs from 2008 and SSL 3.0,
//! which can't connect to modern HTTPS servers. Instead, we use raw TCP
//! sockets wrapped with embedded-tls for TLS 1.3 with UnsecureProvider
//! (no certificate validation -- acceptable for PSP media streaming).

use core::sync::atomic::Ordering;

use super::{find_header_end, io_log, parse_url, DOWNLOAD_CANCEL};

/// Wraps a raw PSP socket fd for `embedded_io::Read + Write`.
struct PspSocketIo {
    fd: i32,
}

impl embedded_io::ErrorType for PspSocketIo {
    type Error = embedded_io::ErrorKind;
}

impl embedded_io::Read for PspSocketIo {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        // SAFETY: fd is a valid socket descriptor, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                self.fd,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            Err(embedded_io::ErrorKind::Other)
        } else {
            Ok(n as usize)
        }
    }
}

impl embedded_io::Write for PspSocketIo {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        // SAFETY: fd is a valid socket descriptor, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetSend(
                self.fd,
                buf.as_ptr() as *const core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            Err(embedded_io::ErrorKind::Other)
        } else {
            Ok(n as usize)
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

type PspTlsConn<'a> =
    embedded_tls::blocking::TlsConnection<'a, PspSocketIo, embedded_tls::blocking::Aes128GcmSha256>;

/// HTTPS reader: raw TCP socket + TLS 1.3 + HTTP/1.1.
///
/// Buffers are heap-allocated via `Box::leak` to get 'static lifetime
/// for the TLS connection (same pattern as `tls.rs`).
pub(super) struct TlsHttpReader {
    tls: PspTlsConn<'static>,
    fd: i32,
    read_buf_ptr: *mut [u8],
    write_buf_ptr: *mut [u8],
    /// Leftover body data read during header parsing.
    leftover: Vec<u8>,
}

/// RNG for TLS handshake using PSP's MT19937 PRNG.
struct IoRng {
    ctx: psp::sys::SceKernelUtilsMt19937Context,
}

impl IoRng {
    fn new() -> Self {
        // SAFETY: MT19937 context is initialized before use.
        // Seed from system timer (user-mode safe). mfc0 $9 (COP0 Count)
        // is privileged on PSP Allegrex and crashes in user mode.
        unsafe {
            let mut ctx = core::mem::MaybeUninit::uninit();
            let seed = psp::sys::sceKernelGetSystemTimeLow() as u32;
            psp::sys::sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
            Self {
                ctx: ctx.assume_init(),
            }
        }
    }
}

impl rand_core::RngCore for IoRng {
    fn next_u32(&mut self) -> u32 {
        // SAFETY: ctx was initialized in new().
        unsafe { psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx) }
    }

    fn next_u64(&mut self) -> u64 {
        let lo = self.next_u32() as u64;
        let hi = self.next_u32() as u64;
        (hi << 32) | lo
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // SAFETY: ctx was initialized in new().
        unsafe {
            for byte in dest.iter_mut() {
                *byte = (psp::sys::sceKernelUtilsMt19937UInt(&mut self.ctx) & 0xFF) as u8;
            }
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> core::result::Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

// SAFETY: MT19937 is the best PRNG available on PSP hardware.
impl rand_core::CryptoRng for IoRng {}

impl TlsHttpReader {
    /// Open an HTTPS connection: TCP connect → TLS handshake → HTTP GET.
    ///
    /// Returns the reader and content length (0 if unknown).
    pub(super) fn open(url: &str) -> Result<(Self, u64), String> {
        Self::open_with_redirects(url, 5)
    }

    /// Open with redirect depth limit to prevent infinite recursion.
    fn open_with_redirects(url: &str, redirects_left: u8) -> Result<(Self, u64), String> {
        let (host, port, path, _) = parse_url(url).ok_or_else(|| format!("bad URL: {url}"))?;

        io_log(&format!("[IO-TLS] resolving {host}..."));

        // DNS resolve.
        let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
        host_bytes.push(0);
        let addr =
            psp::net::resolve_hostname(&host_bytes).map_err(|e| format!("DNS {host}: {e}"))?;

        io_log(&format!(
            "[IO-TLS] resolved {host} → {}.{}.{}.{}",
            addr.0[0], addr.0[1], addr.0[2], addr.0[3]
        ));

        // TCP connect with non-blocking + polling timeout.
        // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
        let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            return Err("socket() failed".into());
        }

        // Set non-blocking mode for connect with timeout.
        // SAFETY: PSP_SO_NBIO enables non-blocking connect polling.
        unsafe {
            let nb: u32 = 1;
            psp::sys::sceNetInetSetsockopt(
                fd,
                crate::network::PSP_SOL_SOCKET,
                crate::network::PSP_SO_NBIO,
                &nb as *const u32 as *const core::ffi::c_void,
                4,
            );
        }

        io_log(&format!("[IO-TLS] TCP connecting to {host}:{port}..."));

        let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
        // SAFETY: Non-blocking connect returns immediately.
        unsafe {
            psp::sys::sceNetInetConnect(fd, &sa, core::mem::size_of::<psp::sys::sockaddr>() as u32);
        }

        // Poll for connection (up to 10 seconds, 100ms intervals).
        // SAFETY: getpeername succeeds only when socket is connected.
        let mut connected = false;
        for tick in 0..100u32 {
            // Check for download cancellation during connect wait.
            if DOWNLOAD_CANCEL.load(Ordering::Acquire) {
                io_log("[IO-TLS] cancelled during TCP connect");
                // SAFETY: Close socket on cancellation.
                unsafe { psp::sys::sceNetInetClose(fd) };
                return Err("cancelled".into());
            }

            // SAFETY: sockaddr is a plain data struct, safe to zero-init.
            let mut sa_out: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
            let mut sa_len: u32 = core::mem::size_of::<psp::sys::sockaddr>() as u32;
            // SAFETY: fd is valid, sa_out/sa_len are properly initialized.
            let ret = unsafe { psp::sys::sceNetInetGetpeername(fd, &mut sa_out, &mut sa_len) };
            if ret == 0 {
                connected = true;
                break;
            }
            if tick == 0 {
                io_log("[IO-TLS] waiting for TCP connect...");
            }
            // SAFETY: Sleep 100ms between polls.
            unsafe {
                psp::sys::sceKernelDelayThread(100_000);
            }
        }

        if !connected {
            // SAFETY: Retrieves errno from the PSP BSD socket layer.
            let errno = unsafe { psp::sys::sceNetInetGetErrno() };
            // SAFETY: Close socket on connect timeout.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!(
                "connect timeout {host}:{port} (10s, errno={errno})"
            ));
        }

        // Set back to blocking mode for TLS I/O + timeouts.
        // SAFETY: Valid socket options on PSP BSD stack.
        #[repr(C)]
        struct Timeval {
            tv_sec: i32,
            tv_usec: i32,
        }
        unsafe {
            // Disable non-blocking mode now that connection is established.
            let nb: u32 = 0;
            psp::sys::sceNetInetSetsockopt(
                fd,
                crate::network::PSP_SOL_SOCKET,
                crate::network::PSP_SO_NBIO,
                &nb as *const u32 as *const core::ffi::c_void,
                4,
            );
            // Set send/receive timeouts for the TLS stream.
            let timeout = Timeval {
                tv_sec: 30,
                tv_usec: 0,
            };
            let timeout_ptr = &timeout as *const Timeval as *const core::ffi::c_void;
            let timeout_len = core::mem::size_of::<Timeval>() as u32;
            psp::sys::sceNetInetSetsockopt(
                fd,
                crate::network::PSP_SOL_SOCKET,
                crate::network::PSP_SO_SNDTIMEO,
                timeout_ptr,
                timeout_len,
            );
            psp::sys::sceNetInetSetsockopt(
                fd,
                crate::network::PSP_SOL_SOCKET,
                crate::network::PSP_SO_RCVTIMEO,
                timeout_ptr,
                timeout_len,
            );
        }

        io_log("[IO-TLS] TCP connected, starting TLS...");

        // TLS 1.3 handshake via embedded-tls.
        let socket_io = PspSocketIo { fd };

        const RECORD_BUF: usize = 16384 + 256;
        let read_buf = Box::leak(vec![0u8; RECORD_BUF].into_boxed_slice());
        let write_buf = Box::leak(vec![0u8; RECORD_BUF].into_boxed_slice());
        let read_buf_ptr: *mut [u8] = read_buf;
        let write_buf_ptr: *mut [u8] = write_buf;

        let config = embedded_tls::blocking::TlsConfig::new().with_server_name(&host);

        let mut tls: PspTlsConn<'static> =
            embedded_tls::blocking::TlsConnection::new(socket_io, read_buf, write_buf);

        let provider = embedded_tls::UnsecureProvider::new::<embedded_tls::blocking::Aes128GcmSha256>(
            IoRng::new(),
        );
        let context = embedded_tls::blocking::TlsContext::new(&config, provider);
        io_log("[IO-TLS] starting TLS 1.3 handshake...");
        if let Err(e) = tls.open(context) {
            drop(tls);
            // SAFETY: Reclaim leaked buffers after TLS is dropped.
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
            }
            // SAFETY: Close socket on handshake failure.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("TLS handshake: {e:?}"));
        }

        io_log("[IO-TLS] TLS 1.3 handshake OK");

        // Send HTTP/1.1 GET request.
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             User-Agent: oasis-psp/1.0\r\n\
             Accept: */*\r\n\
             Connection: keep-alive\r\n\r\n"
        );
        if let Err(e) = embedded_io::Write::write_all(&mut tls, request.as_bytes())
            .and_then(|_| embedded_io::Write::flush(&mut tls))
        {
            drop(tls);
            // SAFETY: Reclaim leaked TLS buffers and close socket on write error.
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }
            return Err(format!("TLS write: {e:?}"));
        }

        io_log("[IO-TLS] HTTP GET sent, reading response headers...");

        // Read response headers (up to 8KB).
        let mut hdr_buf = vec![0u8; 8192];
        let mut hdr_len = 0usize;
        loop {
            if hdr_len >= hdr_buf.len() {
                break;
            }
            match embedded_io::Read::read(&mut tls, &mut hdr_buf[hdr_len..]) {
                Ok(0) => break,
                Ok(n) => {
                    hdr_len += n;
                    if let Some(_end) = find_header_end(&hdr_buf[..hdr_len]) {
                        break;
                    }
                },
                Err(e) => {
                    drop(tls);
                    // SAFETY: Reclaim leaked TLS buffers and close socket
                    // on header read error.
                    unsafe {
                        let _ = Box::from_raw(read_buf_ptr);
                        let _ = Box::from_raw(write_buf_ptr);
                        psp::sys::sceNetInetClose(fd);
                    }
                    return Err(format!("TLS read headers: {e:?}"));
                },
            }
        }

        let header_end = find_header_end(&hdr_buf[..hdr_len])
            .ok_or_else(|| "incomplete HTTP headers".to_string())?;

        let hdr_str = core::str::from_utf8(&hdr_buf[..header_end]).unwrap_or("");
        io_log(&format!(
            "[IO-TLS] response: {}",
            hdr_str.lines().next().unwrap_or("?")
        ));

        // Check status code (first line: "HTTP/1.1 200 OK").
        let status = hdr_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // Handle redirects (follow up to 5).
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = hdr_str.lines().find_map(|l| {
                if l.len() > 9 && l[..9].eq_ignore_ascii_case("location:") {
                    l.split_once(':').map(|(_, v)| v.trim().to_string())
                } else {
                    None
                }
            });

            // Clean up current connection.
            drop(tls);
            // SAFETY: Reclaim leaked TLS buffers and close socket before redirect.
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }

            if let Some(loc) = location {
                if redirects_left == 0 {
                    return Err("too many TLS redirects".into());
                }
                io_log(&format!("[IO-TLS] redirect → {loc}"));
                return Self::open_with_redirects(&loc, redirects_left - 1);
            }
            return Err(format!("redirect {status}, no Location"));
        }

        if status < 200 || status >= 300 {
            drop(tls);
            // SAFETY: Reclaim leaked TLS buffers and close socket on HTTP error.
            unsafe {
                let _ = Box::from_raw(read_buf_ptr);
                let _ = Box::from_raw(write_buf_ptr);
                psp::sys::sceNetInetClose(fd);
            }
            return Err(format!("HTTP {status}"));
        }

        // Parse Content-Length.
        let content_length: u64 = hdr_str
            .lines()
            .find_map(|l| {
                if l.len() > 15 && l[..15].eq_ignore_ascii_case("content-length:") {
                    l[15..].trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        io_log(&format!(
            "[IO-TLS] status={status} content-length={content_length}"
        ));

        // Any leftover body data after headers.
        let leftover = hdr_buf[header_end..hdr_len].to_vec();

        Ok((
            Self {
                tls,
                fd,
                read_buf_ptr,
                write_buf_ptr,
                leftover,
            },
            content_length,
        ))
    }

    /// Read body data. Returns leftover data first, then reads from TLS.
    pub(super) fn read_data(&mut self, buf: &mut [u8]) -> Result<i32, String> {
        if !self.leftover.is_empty() {
            let take = core::cmp::min(self.leftover.len(), buf.len());
            buf[..take].copy_from_slice(&self.leftover[..take]);
            self.leftover.drain(..take);
            return Ok(take as i32);
        }

        match embedded_io::Read::read(&mut self.tls, buf) {
            Ok(n) => Ok(n as i32),
            Err(_) => Ok(0), // treat errors as EOF
        }
    }

    /// Clean up: drop TLS, free buffers, close socket.
    pub(super) fn cleanup(self) {
        let Self {
            tls,
            fd,
            read_buf_ptr,
            write_buf_ptr,
            ..
        } = self;
        drop(tls);
        // SAFETY: Buffers were created via Box::leak and are freed
        // exactly once here. Socket fd is valid and open.
        unsafe {
            let _ = Box::from_raw(read_buf_ptr);
            let _ = Box::from_raw(write_buf_ptr);
            psp::sys::sceNetInetClose(fd);
        }
    }
}
