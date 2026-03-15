//! HTTP connection logic for video downloads: persistent sceHttp template,
//! redirect handling, and `HttpDataSource` abstraction over sceHttp / TLS.

use super::io_log;
use super::tls_http::TlsHttpReader;

// ---------------------------------------------------------------------------
// Persistent HTTP template
// ---------------------------------------------------------------------------

/// Persistent sceHttp template ID.  Initialized once, never torn down.
/// Mirrors how `psp::http::HttpClient` works (one template, many requests).
static DL_TEMPLATE_ID: core::sync::atomic::AtomicI32 =
    core::sync::atomic::AtomicI32::new(-1);

/// Ensure sceHttp is initialized and return the persistent template ID.
///
/// On first call: `sceHttpInit` + `sceHttpCreateTemplate`.
/// On subsequent calls: returns the cached template ID immediately.
///
/// # Safety
///
/// Must only be called from the I/O thread. Calls PSP HTTP syscalls.
unsafe fn ensure_dl_template() -> Result<i32, String> {
    use core::sync::atomic::Ordering;
    use psp::sys;

    let cached = DL_TEMPLATE_ID.load(Ordering::Relaxed);
    if cached >= 0 {
        return Ok(cached);
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
    DL_TEMPLATE_ID.store(tid, Ordering::Relaxed);
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
pub(super) unsafe fn http_open_with_redirect(
    url: &str,
) -> Result<(i32, i32, u64), (String, Option<String>)> {
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
// HTTP data source abstraction
// ---------------------------------------------------------------------------

/// Abstraction over HTTP data sources.
pub(super) enum HttpDataSource {
    /// PSP's built-in HTTP library (for `http://` URLs).
    SceHttp { req_id: i32, conn_id: i32 },
    /// Raw TCP + TLS 1.3 via embedded-tls (for `https://` URLs).
    Tls(TlsHttpReader),
}

impl HttpDataSource {
    /// Read data into `buf`. Returns bytes read, 0 on EOF, negative on error.
    pub(super) fn read_data(&mut self, buf: &mut [u8]) -> i32 {
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
            HttpDataSource::Tls(reader) => match reader.read_data(buf) {
                Ok(n) => n,
                Err(e) => {
                    io_log(&format!("[IO-DL] TLS read error: {e}"));
                    0
                },
            },
        }
    }

    /// Clean up the connection — abort the in-flight request, delete
    /// request+connection handles. The persistent template stays alive.
    pub(super) fn cleanup(self) {
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
