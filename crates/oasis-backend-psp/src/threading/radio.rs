//! Internet radio connection handler (I/O thread).
//!
//! Connects to an internet radio stream via raw TCP + HTTP, parses
//! ICY metadata headers, and hands the socket to the audio thread.

use super::{
    find_header_end, parse_icy_metaint, parse_radio_url,
    IoResponse, IO_RESP_QUEUE,
};

/// Connect to an internet radio stream via raw TCP + HTTP.
///
/// Sends an HTTP GET with `Icy-MetaData: 1`, reads headers to extract
/// `icy-metaint`, then passes the connected socket fd to the audio thread.
pub(super) fn handle_radio_connect(url: String) {
    use std::ffi::c_void;

    // Initialize network.
    if let Err(e) = crate::network::ensure_net_init_pub() {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: format!("net init: {e}"),
        });
        return;
    }

    // Parse URL.
    let (host, port, path) = match parse_radio_url(&url) {
        Some(v) => v,
        None => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("bad URL: {url}"),
            });
            return;
        },
    };

    // DNS resolve.
    let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
    host_bytes.push(0);
    let addr = match psp::net::resolve_hostname(&host_bytes) {
        Ok(a) => a,
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("DNS: {e}"),
            });
            return;
        },
    };

    // Create TCP socket.
    // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
    let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
    if fd < 0 {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "socket() failed".into(),
        });
        return;
    }

    // Connect.
    let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
    // SAFETY: Connect to the resolved address.
    let ret = unsafe {
        psp::sys::sceNetInetConnect(fd, &sa, core::mem::size_of::<psp::sys::sockaddr>() as u32)
    };
    if ret < 0 {
        // SAFETY: fd is a valid socket descriptor; closing on connect failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: format!("connect {}:{} failed", host, port),
        });
        return;
    }

    // Send HTTP GET with ICY metadata request.
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nIcy-MetaData: 1\r\n\
         User-Agent: OASIS_OS/1.0\r\nAccept: */*\r\n\r\n",
        path, host,
    );
    let req_bytes = request.as_bytes();
    // SAFETY: Send the HTTP request over the connected socket.
    let sent = unsafe {
        psp::sys::sceNetInetSend(fd, req_bytes.as_ptr() as *const c_void, req_bytes.len(), 0)
    };
    if sent <= 0 {
        // SAFETY: fd is a valid socket descriptor; closing on send failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "send failed".into(),
        });
        return;
    }

    // Read response headers (up to 4KB).
    let mut hdr_buf = vec![0u8; 4096];
    let mut hdr_len = 0usize;
    let mut attempts = 0;
    while hdr_len < hdr_buf.len() && attempts < 200 {
        // SAFETY: Blocking recv for header data.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                fd,
                hdr_buf.as_mut_ptr().add(hdr_len) as *mut c_void,
                (hdr_buf.len() - hdr_len).min(512),
                0,
            )
        };
        if n > 0 {
            hdr_len += n as usize;
            // Check for end of headers.
            if hdr_len >= 4 {
                let search_start = if hdr_len > n as usize + 3 {
                    hdr_len - n as usize - 3
                } else {
                    0
                };
                let haystack = &hdr_buf[search_start..hdr_len];
                if find_header_end(haystack).is_some() {
                    break;
                }
            }
        } else if n == 0 {
            break; // Connection closed.
        } else {
            attempts += 1;
            psp::thread::sleep_ms(20);
        }
    }

    // Validate that we received a complete header (with \r\n\r\n terminator).
    let header_end = if hdr_len > 0 {
        find_header_end(&hdr_buf[..hdr_len])
    } else {
        None
    };

    if header_end.is_none() {
        // SAFETY: fd is a valid socket descriptor; closing on incomplete headers.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "incomplete headers".into(),
        });
        return;
    }

    let header_end = header_end.unwrap();

    // Parse icy-metaint from headers.
    let hdr_str = String::from_utf8_lossy(&hdr_buf[..hdr_len]);
    let icy_metaint = parse_icy_metaint(&hdr_str);

    // Extract any leftover audio data after the header boundary.
    let initial_data = hdr_buf[header_end..hdr_len].to_vec();

    // Set non-blocking for streaming.
    let nb: i32 = 1;
    // SAFETY: SO_NONBLOCK is a PSP-specific socket option.
    unsafe {
        psp::sys::sceNetInetSetsockopt(
            fd,
            0xFFFF, // SOL_SOCKET
            0x0080, // SO_NONBLOCK
            &nb as *const i32 as *const c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }

    let _ = IO_RESP_QUEUE.push(IoResponse::RadioConnected {
        fd,
        icy_metaint,
        initial_data,
    });
}
