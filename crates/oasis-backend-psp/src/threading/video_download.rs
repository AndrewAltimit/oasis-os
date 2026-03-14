//! Video download handler: streaming MP4 download with moov parsing,
//! HTTP redirect handling, and TLS fallback.

use core::sync::atomic::Ordering;

use super::{
    io_log, parse_url, send_audio_cmd, find_header_end,
    AudioCmd, IoResponse, AUDIO_QUEUE, DOWNLOAD_CANCEL, IO_RESP_QUEUE,
};
use super::tls_http::TlsHttpReader;

// ---------------------------------------------------------------------------
// MP4 box parsing
// ---------------------------------------------------------------------------

/// Parse MP4 box headers from the first bytes of a download to find where
/// the moov atom ends.  Returns `Some(moov_offset + moov_size)` for
/// faststart files (moov before mdat), or `None` if moov wasn't found.
fn find_moov_end(header_bytes: &[u8]) -> Option<u64> {
    let mut pos = 0usize;
    while pos + 8 <= header_bytes.len() {
        let size = u32::from_be_bytes([
            header_bytes[pos],
            header_bytes[pos + 1],
            header_bytes[pos + 2],
            header_bytes[pos + 3],
        ]) as u64;
        let box_type = &header_bytes[pos + 4..pos + 8];

        if box_type == b"moov" {
            if size == 0 {
                return None; // extends to EOF, can't determine end
            }
            return Some(pos as u64 + size);
        }

        // 64-bit extended size
        if size == 1 {
            if pos + 16 > header_bytes.len() {
                break;
            }
            let big = u64::from_be_bytes([
                header_bytes[pos + 8],
                header_bytes[pos + 9],
                header_bytes[pos + 10],
                header_bytes[pos + 11],
                header_bytes[pos + 12],
                header_bytes[pos + 13],
                header_bytes[pos + 14],
                header_bytes[pos + 15],
            ]);
            pos += big as usize;
        } else if size == 0 {
            break; // box extends to EOF
        } else {
            pos += size as usize;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Persistent HTTP template
// ---------------------------------------------------------------------------

/// Persistent sceHttp template ID.  Initialized once, never torn down.
/// Mirrors how `psp::http::HttpClient` works (one template, many requests).
/// SAFETY: Only accessed from the I/O thread (single producer).
static mut DL_TEMPLATE_ID: i32 = -1;

/// Ensure sceHttp is initialized and return the persistent template ID.
///
/// On first call: `sceHttpInit` + `sceHttpCreateTemplate`.
/// On subsequent calls: returns the cached template ID immediately.
///
/// # Safety
///
/// Must only be called from the I/O thread. Accesses mutable statics
/// (`DL_TEMPLATE_ID`) and PSP HTTP syscalls.
unsafe fn ensure_dl_template() -> Result<i32, String> {
    use psp::sys;

    // SAFETY: Only accessed from the I/O thread (single producer).
    if unsafe { DL_TEMPLATE_ID } >= 0 {
        return Ok(unsafe { DL_TEMPLATE_ID });
    }

    // SAFETY: PSP HTTP subsystem init with 128KB pool.
    let ret = unsafe { sys::sceHttpInit(0x20000) };
    // Accept "already initialized" (0x80431020) in case IO-TV already
    // initialized it via psp::http::HttpClient.
    if ret < 0 && ret != -0x7FBCEFE0_i32 {
        io_log(&format!("[IO-DL] sceHttpInit failed: {ret:#x}"));
        return Err(format!("sceHttpInit failed: {ret:#x}"));
    }
    io_log(&format!("[IO-DL] sceHttpInit: {ret:#x}"));

    // SAFETY: Creating HTTP template with user-agent string.
    let tid = unsafe {
        sys::sceHttpCreateTemplate(b"oasis-psp/1.0\0".as_ptr() as *mut u8, 1, 0)
    };
    if tid < 0 {
        return Err(format!("template: {tid:#x}"));
    }
    // SAFETY: Configuring template options on valid template ID.
    unsafe {
        // Disable keep-alive so each request gets a fresh TCP connection.
        sys::sceHttpDisableKeepAlive(tid);
        // Disable auto-redirect. archive.org redirects some items'
        // HTTP URLs to HTTPS, and PSP's built-in SSL (2008 root CAs)
        // can't connect. We handle redirects manually, rewriting
        // HTTPS→HTTP in the Location header.
        sys::sceHttpDisableRedirect(tid);
    }
    io_log(&format!(
        "[IO-DL] template created: {tid} (keep-alive off, redirect off)"
    ));
    // SAFETY: Only accessed from the I/O thread (single producer).
    unsafe { DL_TEMPLATE_ID = tid; }
    Ok(tid)
}

// ---------------------------------------------------------------------------
// HTTP open with manual redirect handling
// ---------------------------------------------------------------------------

/// Open an HTTP connection with manual redirect handling.
///
/// PSP's `sceHttpEnableRedirect` follows HTTP→HTTPS redirects which fail
/// because the firmware's root CAs are from 2008. Instead, we handle
/// 301/302/307/308 manually, rewriting `https://` → `http://` in the
/// Location header.
///
/// Returns `(req_id, conn_id, content_length)` on success.
/// Uses a persistent template — caller must only clean up req_id and conn_id.
///
/// On redirect-loop failure (CDN requires HTTPS), returns the HTTPS
/// redirect URL as the second element so the caller can try TLS.
///
/// # Safety
///
/// Must only be called from the I/O thread. Calls PSP HTTP syscalls
/// (`sceHttpCreateConnection`, `sceHttpCreateRequest`, `sceHttpSendRequest`,
/// `sceHttpReadData`) and accesses the persistent template ID.
unsafe fn http_open_with_redirect(url: &str) -> Result<(i32, i32, u64), (String, Option<String>)> {
    use psp::sys;

    // SAFETY: Delegates to ensure_dl_template which accesses mutable statics
    // and PSP HTTP syscalls; only called from I/O thread.
    let template_id = unsafe { ensure_dl_template() }.map_err(|e| (e, None))?;
    let mut current_url = url.to_string();
    // Track the last HTTPS redirect URL for TLS fallback.
    let mut last_https_redirect: Option<String> = None;

    for attempt in 0..5 {
        let mut url_bytes: Vec<u8> = current_url.as_bytes().to_vec();
        url_bytes.push(0);

        // SAFETY: Valid template ID and null-terminated URL.
        let conn_id = unsafe {
            sys::sceHttpCreateConnectionWithURL(template_id, url_bytes.as_ptr(), 0)
        };
        if conn_id < 0 {
            return Err((format!("connect: {conn_id:#x}"), None));
        }

        // SAFETY: Valid connection ID and null-terminated URL.
        let req_id = unsafe {
            sys::sceHttpCreateRequestWithURL(
                conn_id,
                sys::HttpMethod::Get,
                url_bytes.as_ptr() as *mut u8,
                0,
            )
        };
        if req_id < 0 {
            // SAFETY: Cleaning up valid connection ID.
            unsafe { sys::sceHttpDeleteConnection(conn_id); }
            return Err((format!("request: {req_id:#x}"), None));
        }

        // SAFETY: Setting timeouts on valid request ID.
        unsafe {
            sys::sceHttpSetConnectTimeOut(req_id, 30_000_000);
            sys::sceHttpSetRecvTimeOut(req_id, 30_000_000);
        }

        // SAFETY: Sending HTTP GET request with no body.
        let ret = unsafe {
            sys::sceHttpSendRequest(req_id, core::ptr::null_mut(), 0)
        };
        if ret < 0 {
            io_log(&format!("[IO-DL] send failed: {ret:#x}"));
            // SAFETY: Cleaning up valid request and connection IDs.
            unsafe {
                sys::sceHttpDeleteRequest(req_id);
                sys::sceHttpDeleteConnection(conn_id);
            }
            return Err((format!("send: {ret:#x}"), last_https_redirect.clone()));
        }

        let mut status_code: i32 = 0;
        // SAFETY: Valid request ID, writing to local variable.
        unsafe { sys::sceHttpGetStatusCode(req_id, &mut status_code); }
        io_log(&format!("[IO-DL] status={status_code} (attempt {attempt})"));

        // Handle redirects manually.
        if matches!(status_code, 301 | 302 | 303 | 307 | 308) {
            // Read all headers to find Location — must copy BEFORE
            // deleting the request, since the pointer is into its buffer.
            let mut hdr_ptr: *mut u8 = core::ptr::null_mut();
            let mut hdr_len: u32 = 0;
            // SAFETY: Valid request ID, writing to local variables.
            let ret = unsafe {
                sys::sceHttpGetAllHeader(req_id, &mut hdr_ptr, &mut hdr_len)
            };

            let location_url = if ret >= 0 && !hdr_ptr.is_null() && hdr_len > 0 {
                // SAFETY: Pointer valid while request alive; length from kernel.
                let hdrs = unsafe {
                    core::slice::from_raw_parts(hdr_ptr, hdr_len as usize)
                };
                let hdr_str = core::str::from_utf8(hdrs).unwrap_or("");
                hdr_str
                    .lines()
                    .find(|l| l.len() > 9 && l[..9].eq_ignore_ascii_case("location:"))
                    .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            } else {
                None
            };

            // Now safe to delete the request+connection (template persists).
            // SAFETY: Cleaning up valid request and connection IDs.
            unsafe {
                sys::sceHttpDeleteRequest(req_id);
                sys::sceHttpDeleteConnection(conn_id);
            }

            if let Some(loc) = location_url {
                // Save HTTPS URL for TLS fallback before rewriting.
                if loc.starts_with("https://") {
                    last_https_redirect = Some(loc.clone());
                }
                // Rewrite HTTPS → HTTP so PSP can follow it.
                let new_url = loc.replacen("https://", "http://", 1);
                // Detect redirect loop: same URL after rewrite.
                if new_url == current_url {
                    io_log(&format!("[IO-DL] redirect loop detected → {new_url}"));
                    return Err((
                        String::from("redirect loop (CDN requires HTTPS)"),
                        last_https_redirect,
                    ));
                }
                io_log(&format!("[IO-DL] redirect → {new_url}"));
                current_url = new_url;
                continue;
            } else {
                return Err((format!("redirect {status_code}, no Location"), None));
            }
        }

        if status_code < 200 || status_code >= 300 {
            // SAFETY: Cleaning up valid request and connection IDs.
            unsafe {
                sys::sceHttpDeleteRequest(req_id);
                sys::sceHttpDeleteConnection(conn_id);
            }
            return Err((format!("HTTP {status_code}"), None));
        }

        let mut content_length: u64 = 0;
        // SAFETY: Valid request ID, writing to local variable.
        unsafe { sys::sceHttpGetContentLength(req_id, &mut content_length); }

        return Ok((req_id, conn_id, content_length));
    }

    Err((String::from("too many redirects"), last_https_redirect))
}

// ---------------------------------------------------------------------------
// Raw TCP HTTP reader (bypasses sceHttp)
// ---------------------------------------------------------------------------

/// Raw TCP HTTP reader — bypasses sceHttp entirely using BSD sockets.
///
/// sceHttp's internal state corrupts after the first download session,
/// causing `0x80431079` on subsequent `sceHttpSendRequest` calls.
/// Raw sockets have no such state — each connection is independent.
#[allow(dead_code)]
struct RawHttpReader {
    fd: i32,
    /// Leftover body data read during header parsing.
    leftover: Vec<u8>,
}

#[allow(dead_code)]
impl RawHttpReader {
    /// Open an HTTP connection via raw TCP: DNS → connect → GET → parse headers.
    ///
    /// Returns the reader and content length (0 if unknown).
    /// Follows up to 5 redirects.
    fn open(url: &str) -> Result<(Self, u64), String> {
        Self::open_with_redirects(url, 5)
    }

    fn open_with_redirects(url: &str, max_redirects: u32) -> Result<(Self, u64), String> {
        let (host, port, path, _) = parse_url(url).ok_or_else(|| format!("bad URL: {url}"))?;

        io_log(&format!("[IO-RAW] resolving {host}..."));

        let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
        host_bytes.push(0);
        let addr =
            psp::net::resolve_hostname(&host_bytes).map_err(|e| format!("DNS {host}: {e}"))?;

        io_log(&format!(
            "[IO-RAW] resolved {host} → {}.{}.{}.{}",
            addr.0[0], addr.0[1], addr.0[2], addr.0[3]
        ));

        // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
        let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
        if fd < 0 {
            return Err("socket() failed".into());
        }

        // Set recv/send timeouts (30s) before connect.
        // SAFETY: Valid socket options on PSP BSD stack.
        unsafe {
            #[repr(C)]
            struct Timeval {
                tv_sec: i32,
                tv_usec: i32,
            }
            let timeout = Timeval {
                tv_sec: 30,
                tv_usec: 0,
            };
            let timeout_ptr = &timeout as *const Timeval as *const core::ffi::c_void;
            let timeout_len = core::mem::size_of::<Timeval>() as u32;
            psp::sys::sceNetInetSetsockopt(fd, 0xFFFF, 0x1005, timeout_ptr, timeout_len);
            psp::sys::sceNetInetSetsockopt(fd, 0xFFFF, 0x1006, timeout_ptr, timeout_len);
        }

        io_log(&format!("[IO-RAW] connecting to {host}:{port}..."));

        let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
        // SAFETY: Blocking connect — will return when connected or on
        // TCP timeout. Port 80 is not blocked so this completes quickly.
        let ret = unsafe {
            psp::sys::sceNetInetConnect(fd, &sa, core::mem::size_of::<psp::sys::sockaddr>() as u32)
        };
        if ret < 0 {
            io_log(&format!("[IO-RAW] connect failed: {ret:#x}"));
            // SAFETY: fd is a valid socket descriptor from sceNetInetSocket.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("connect failed {host}:{port}: {ret:#x}"));
        }

        io_log("[IO-RAW] connected, sending HTTP GET...");

        // Send HTTP/1.1 GET request.
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             User-Agent: oasis-psp/1.0\r\n\
             Accept: */*\r\n\
             Connection: close\r\n\r\n"
        );
        let req_bytes = request.as_bytes();
        let mut sent = 0usize;
        while sent < req_bytes.len() {
            // SAFETY: fd is valid, buf points to request data.
            let n = unsafe {
                psp::sys::sceNetInetSend(
                    fd,
                    req_bytes[sent..].as_ptr() as *const core::ffi::c_void,
                    req_bytes.len() - sent,
                    0,
                )
            };
            if n <= 0 {
                // SAFETY: fd is a valid socket descriptor.
                unsafe { psp::sys::sceNetInetClose(fd) };
                return Err("send failed".into());
            }
            sent += n as usize;
        }

        // Read response headers (up to 8KB).
        let mut hdr_buf = vec![0u8; 8192];
        let mut hdr_len = 0usize;
        loop {
            if hdr_len >= hdr_buf.len() {
                break;
            }
            // SAFETY: fd is valid, buffer is valid.
            let n = unsafe {
                psp::sys::sceNetInetRecv(
                    fd,
                    hdr_buf[hdr_len..].as_mut_ptr() as *mut core::ffi::c_void,
                    hdr_buf.len() - hdr_len,
                    0,
                )
            };
            if n <= 0 {
                break;
            }
            hdr_len += n as usize;
            if find_header_end(&hdr_buf[..hdr_len]).is_some() {
                break;
            }
        }

        let header_end = find_header_end(&hdr_buf[..hdr_len])
            .ok_or_else(|| "incomplete HTTP headers".to_string())?;

        let hdr_str = core::str::from_utf8(&hdr_buf[..header_end]).unwrap_or("");
        io_log(&format!(
            "[IO-RAW] response: {}",
            hdr_str.lines().next().unwrap_or("?")
        ));

        let status = hdr_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // Handle redirects.
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            // SAFETY: fd is a valid socket descriptor.
            unsafe { psp::sys::sceNetInetClose(fd) };

            if max_redirects == 0 {
                return Err("too many redirects".into());
            }

            let location = hdr_str.lines().find_map(|l| {
                if l.len() > 9 && l[..9].eq_ignore_ascii_case("location:") {
                    l.split_once(':').map(|(_, v)| v.trim().to_string())
                } else {
                    None
                }
            });

            if let Some(loc) = location {
                // Rewrite HTTPS → HTTP for PSP.
                let new_url = loc.replacen("https://", "http://", 1);
                io_log(&format!("[IO-RAW] redirect → {new_url}"));
                return Self::open_with_redirects(&new_url, max_redirects - 1);
            }
            return Err(format!("redirect {status}, no Location"));
        }

        if status < 200 || status >= 300 {
            // SAFETY: fd is a valid socket descriptor.
            unsafe { psp::sys::sceNetInetClose(fd) };
            return Err(format!("HTTP {status}"));
        }

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
            "[IO-RAW] status={status} content-length={content_length}"
        ));

        let leftover = hdr_buf[header_end..hdr_len].to_vec();

        Ok((Self { fd, leftover }, content_length))
    }

    /// Read body data. Returns leftover first, then reads from socket.
    fn read_data(&mut self, buf: &mut [u8]) -> i32 {
        if !self.leftover.is_empty() {
            let take = core::cmp::min(self.leftover.len(), buf.len());
            buf[..take].copy_from_slice(&self.leftover[..take]);
            self.leftover.drain(..take);
            return take as i32;
        }
        // SAFETY: fd is valid, buf is valid.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                self.fd,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 { 0 } else { n as i32 }
    }

    /// Close the socket.
    fn cleanup(self) {
        // SAFETY: fd is a valid open socket.
        unsafe { psp::sys::sceNetInetClose(self.fd) };
    }
}

// ---------------------------------------------------------------------------
// HTTP data source abstraction
// ---------------------------------------------------------------------------

/// Abstraction over HTTP data sources.
enum HttpDataSource {
    /// PSP's built-in HTTP library (for `http://` URLs).
    SceHttp { req_id: i32, conn_id: i32 },
    /// Raw TCP + TLS 1.3 via embedded-tls (for `https://` URLs).
    Tls(TlsHttpReader),
}

impl HttpDataSource {
    /// Read data into `buf`. Returns bytes read, 0 on EOF, negative on error.
    fn read_data(&mut self, buf: &mut [u8]) -> i32 {
        match self {
            HttpDataSource::SceHttp { req_id, .. } => {
                // SAFETY: req_id is a valid HTTP request handle.
                unsafe {
                    psp::sys::sceHttpReadData(
                        *req_id,
                        buf.as_mut_ptr() as *mut core::ffi::c_void,
                        buf.len() as u32,
                    )
                }
            },
            HttpDataSource::Tls(reader) => reader.read_data(buf).unwrap_or(0),
        }
    }

    /// Clean up the connection — abort the in-flight request, delete
    /// request+connection handles. The persistent template stays alive.
    fn cleanup(self) {
        match self {
            HttpDataSource::SceHttp { req_id, conn_id } => {
                // SAFETY: IDs are valid sceHttp handles.
                unsafe {
                    psp::sys::sceHttpAbortRequest(req_id);
                    psp::sys::sceHttpDeleteRequest(req_id);
                    psp::sys::sceHttpDeleteConnection(conn_id);
                }
                io_log("[IO-DL] cleanup: abort+delete done");
            },
            HttpDataSource::Tls(reader) => reader.cleanup(),
        }
    }
}

// ---------------------------------------------------------------------------
// Video download handler
// ---------------------------------------------------------------------------

/// Streaming video download: buffers moov atom in memory, parses MP4 track
/// tables, then extracts and pushes demuxed samples directly to the video
/// and audio threads as HTTP data arrives. No disk I/O.
///
/// Supports both HTTP (via sceHttp) and HTTPS (via raw TCP + embedded-tls).
pub(super) fn handle_video_download(url: String, _dest: String, tag: u32) {
    use oasis_video::demux_lite::Mp4Lite;

    io_log(&format!("[IO-DL] starting stream: {url}"));

    // Clear any previous cancellation flag.
    DOWNLOAD_CANCEL.store(false, Ordering::Release);

    // Check connectivity without showing a dialog (must not call
    // ensure_net_init_pub from background thread -- freezes EBOOT).
    if !psp::net::is_connected() {
        let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
            tag,
            msg: "not connected to WiFi".to_string(),
        });
        return;
    }

    // Try sceHttp first (HTTP port 80), fall back to raw TCP + TLS 1.3
    // (HTTPS port 443) if sceHttp fails. sceHttp's internal connection
    // pool corrupts after an aborted partial download, causing 0x80431079
    // on subsequent requests. The TLS path uses independent raw sockets.
    let http_url = if url.starts_with("https://") {
        url.replacen("https://", "http://", 1)
    } else {
        url.clone()
    };

    // SAFETY: All sceHttp calls use IDs returned by prior creation.
    let (mut source, content_length) = match unsafe { http_open_with_redirect(&http_url) } {
        Ok((req_id, conn_id, cl)) => {
            io_log("[IO-DL] sceHttp OK");
            (HttpDataSource::SceHttp { req_id, conn_id }, cl)
        },
        Err((msg, https_redirect)) => {
            // Try TLS fallback. First try archive.org HTTPS
            // (may redirect to a different, reachable CDN node),
            // then try CDN HTTPS directly as a last resort.
            let origin_tls = if url.starts_with("http://") {
                url.replacen("http://", "https://", 1)
            } else {
                url.clone()
            };

            // Build candidate list: origin first, then CDN if different.
            let mut tls_candidates: Vec<String> = vec![origin_tls];
            if let Some(cdn_url) = &https_redirect {
                if !tls_candidates.contains(cdn_url) {
                    tls_candidates.push(cdn_url.clone());
                }
            }

            let mut last_err = msg.clone();
            let mut found = None;
            for (i, tls_url) in tls_candidates.iter().enumerate() {
                io_log(&format!(
                    "[IO-DL] sceHttp failed ({msg}), trying TLS \
                         #{} to {tls_url}...",
                    i + 1
                ));
                match TlsHttpReader::open(tls_url) {
                    Ok((reader, cl)) => {
                        io_log(&format!("[IO-DL] TLS fallback #{} OK, len={cl}", i + 1));
                        found = Some((reader, cl));
                        break;
                    },
                    Err(e) => {
                        io_log(&format!("[IO-DL] TLS fallback #{} failed: {e}", i + 1));
                        last_err = e;
                    },
                }
            }

            match found {
                Some((reader, cl)) => (HttpDataSource::Tls(reader), cl),
                None => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                        tag,
                        msg: format!("HTTP: {msg}; TLS: {last_err}"),
                    });
                    return;
                },
            }
        },
    };

    let total = if content_length > 0 {
        Some(content_length)
    } else {
        None
    };
    io_log(&format!("[IO-DL] content-length={content_length}"));

    // Phase 1: buffer data until moov atom is fully received.
    let mut moov_buf: Vec<u8> = Vec::new();
    let mut moov_end: Option<u64> = None;
    let mut buf = [0u8; 8192];
    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    loop {
        // Check for cancellation (user pressed Circle during download).
        if DOWNLOAD_CANCEL.load(Ordering::Acquire) {
            io_log("[IO-DL] cancelled during moov buffering");
            source.cleanup();
            return;
        }

        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase1): {n:#x}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("read: {n:#x}"),
            });
            return;
        }
        if n == 0 {
            break; // EOF during moov buffering
        }

        moov_buf.extend_from_slice(&buf[..n as usize]);
        downloaded += n as u64;

        // Report progress.
        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }

        // Try to find moov end from headers once we have enough.
        if moov_end.is_none() && moov_buf.len() >= 32 {
            moov_end = find_moov_end(&moov_buf);
            if let Some(end) = moov_end {
                io_log(&format!("[IO-DL] moov ends at byte {end}"));
            }
        }

        // Check if we've buffered past moov end.
        if let Some(end) = moov_end {
            if downloaded >= end {
                io_log(&format!(
                    "[IO-DL] moov fully buffered ({downloaded} bytes, \
                     moov_end={end})"
                ));
                break;
            }
        }

        // Safety limit: if moov hasn't been found after 8MB, abort.
        if moov_buf.len() > 8 * 1024 * 1024 {
            io_log("[IO-DL] moov not found in first 8MB, aborting");
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: String::from("moov atom not found (non-faststart?)"),
            });
            return;
        }
    }

    // Parse moov using Mp4Lite with a Cursor over the buffered data.
    io_log(&format!(
        "[IO-DL] parsing moov ({} bytes buffered)...",
        moov_buf.len()
    ));

    let cursor = std::io::Cursor::new(&moov_buf);
    let mp4 = match Mp4Lite::open(cursor) {
        Ok(m) => m,
        Err(e) => {
            io_log(&format!("[IO-DL] Mp4Lite parse failed: {e}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("MP4 parse: {e}"),
            });
            return;
        },
    };

    let video_track = mp4.video_track_info().cloned();
    let audio_track = mp4.audio_track_info().cloned();
    drop(mp4);

    let v_count = video_track.as_ref().map_or(0, |t| t.sample_count());
    let a_count = audio_track.as_ref().map_or(0, |t| t.sample_count());
    io_log(&format!(
        "[IO-DL] parsed: {v_count} video, {a_count} audio samples"
    ));

    // Send AAC config to audio thread before any frames arrive.
    if let Some(ref at) = audio_track {
        if let Some(ref aac) = at.aac_config {
            send_audio_cmd(AudioCmd::VideoAudioAacConfig {
                sample_rate: aac.sample_rate,
                channels: aac.channels,
            });
            io_log(&format!(
                "[IO-DL] AAC config: rate={}, ch={}",
                aac.sample_rate, aac.channels
            ));
        }
    }

    io_log("[IO-DL] setting video_playing...");

    // Pre-arm the playing flag BEFORE sending StreamStart to avoid a
    // race: the I/O thread checks is_video_playing() in the phase-2
    // loop, but the video thread may not have processed the command yet.
    crate::video::set_video_playing(true);
    crate::video::request_stream_start();

    io_log("[IO-DL] sending VideoStreamReady...");

    // Notify main thread that streaming playback has begun.
    let _ = IO_RESP_QUEUE.push(IoResponse::VideoStreamReady {
        tag,
        path: String::new(),
        content_length: content_length as u32,
    });

    // Phase 2: stream mdat samples from HTTP(S).
    let mut v_idx = 0usize;
    let mut a_idx = 0usize;
    let mut http_pos: u64;

    let mut sample_data: Vec<u8> = Vec::new();
    let mut sample_offset: u64 = 0;
    let mut sample_size: u32 = 0;
    let mut sample_is_video = false;
    let mut have_target = false;

    let moov_end_off = moov_end.unwrap_or(downloaded);
    let leftover_start = moov_end_off as usize;
    let leftover = if leftover_start < moov_buf.len() {
        &moov_buf[leftover_start..]
    } else {
        &[]
    };
    http_pos = moov_end_off;

    io_log(&format!(
        "[IO-DL] leftover={} bytes, moov_end_off={moov_end_off}",
        leftover.len()
    ));

    if !leftover.is_empty() {
        process_stream_chunk(
            leftover,
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );
        io_log(&format!("[IO-DL] leftover processed: v={v_idx} a={a_idx}"));
    }

    io_log("[IO-DL] dropping moov_buf...");
    drop(moov_buf);
    io_log("[IO-DL] entering phase 2 loop");

    let mut loop_iter = 0u32;
    loop {
        if !crate::video::is_video_playing() || DOWNLOAD_CANCEL.load(Ordering::Acquire) {
            io_log("[IO-DL] playback stopped, ending stream");
            break;
        }

        if loop_iter < 3 {
            io_log(&format!("[IO-DL] phase2 read #{loop_iter}..."));
        }
        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase2): {n:#x}"));
            break;
        }
        if n == 0 {
            break; // EOF
        }
        if loop_iter < 3 {
            io_log(&format!("[IO-DL] phase2 read #{loop_iter}: {n} bytes"));
        }

        downloaded += n as u64;
        loop_iter += 1;

        process_stream_chunk(
            &buf[..n as usize],
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );

        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }
    }

    source.cleanup();

    io_log(&format!(
        "[IO-DL] stream complete: {downloaded} bytes, \
         {v_idx}/{v_count} video, {a_idx}/{a_count} audio"
    ));

    crate::video::set_video_playing(false);
    send_audio_cmd(AudioCmd::VideoAudioStop);
}

// ---------------------------------------------------------------------------
// Stream demux helpers
// ---------------------------------------------------------------------------

/// Determine the next sample to extract (lowest file offset among pending
/// video and audio samples).
fn next_sample_target(
    v_idx: usize,
    a_idx: usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) -> Option<(u64, u32, bool)> {
    let v_next = video_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(v_idx));
    let a_next = audio_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(a_idx));

    match (v_next, a_next) {
        (Some((vo, vs)), Some((ao, a_s))) => {
            if vo <= ao {
                Some((vo, vs, true))
            } else {
                Some((ao, a_s, false))
            }
        },
        (Some((vo, vs)), None) => Some((vo, vs, true)),
        (None, Some((ao, a_s))) => Some((ao, a_s, false)),
        (None, None) => None,
    }
}

/// Process a chunk of HTTP data, extracting complete samples and pushing
/// them to the video/audio decode threads.
#[allow(clippy::too_many_arguments)]
fn process_stream_chunk(
    chunk: &[u8],
    http_pos: &mut u64,
    have_target: &mut bool,
    sample_offset: &mut u64,
    sample_size: &mut u32,
    sample_is_video: &mut bool,
    sample_data: &mut Vec<u8>,
    v_idx: &mut usize,
    a_idx: &mut usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) {
    let mut chunk_pos = 0usize;

    while chunk_pos < chunk.len() {
        // Find next sample target if we don't have one.
        if !*have_target {
            match next_sample_target(*v_idx, *a_idx, video_track, audio_track) {
                Some((off, sz, is_v)) => {
                    *sample_offset = off;
                    *sample_size = sz;
                    *sample_is_video = is_v;
                    *have_target = true;
                    sample_data.clear();
                },
                None => {
                    // All samples extracted; skip remaining data.
                    *http_pos += (chunk.len() - chunk_pos) as u64;
                    return;
                },
            }
        }

        // Skip bytes before sample start.
        if *http_pos < *sample_offset {
            let skip = core::cmp::min(
                (*sample_offset - *http_pos) as usize,
                chunk.len() - chunk_pos,
            );
            chunk_pos += skip;
            *http_pos += skip as u64;
            if *http_pos < *sample_offset {
                return; // need more data to reach sample
            }
        }

        if *sample_is_video {
            // Skip video sample data — just advance stream position.
            // sample_data is unused for video; track progress via offset.
            let sample_end = *sample_offset + *sample_size as u64;
            let available = chunk.len() - chunk_pos;
            let remaining = (sample_end - *http_pos) as usize;
            let skip = core::cmp::min(remaining, available);
            chunk_pos += skip;
            *http_pos += skip as u64;

            if *http_pos >= sample_end {
                *v_idx += 1;
                *have_target = false;
            }
        } else {
            // Buffer audio sample data.
            let remaining = *sample_size as usize - sample_data.len();
            let available = chunk.len() - chunk_pos;
            let take = core::cmp::min(remaining, available);
            sample_data.extend_from_slice(&chunk[chunk_pos..chunk_pos + take]);
            chunk_pos += take;
            *http_pos += take as u64;

            if sample_data.len() == *sample_size as usize {
                let data = core::mem::take(sample_data);
                // Blocking push with backpressure: retry until the audio
                // queue has space, sleeping 2ms between attempts. This
                // throttles the I/O thread to match the audio decode rate,
                // preventing frame drops and choppy playback.
                let mut cmd = AudioCmd::VideoAudioAac { data };
                loop {
                    match AUDIO_QUEUE.push(cmd) {
                        Ok(()) => break,
                        Err(returned) => {
                            cmd = returned;
                            // Check if playback was stopped to avoid
                            // deadlocking the I/O thread.
                            if !crate::video::is_video_playing() {
                                break;
                            }
                            // SAFETY: sceKernelDelayThread sleeps thread.
                            unsafe {
                                psp::sys::sceKernelDelayThread(2_000);
                            }
                        },
                    }
                }
                *a_idx += 1;
                *have_target = false;
                sample_data.clear();
            }
        }
    }
}
