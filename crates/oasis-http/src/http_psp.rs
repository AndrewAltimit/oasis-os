//! PSP HTTP client using `TlsProvider::connect_tcp` for raw sockets.
//!
//! On PSP, `std::net::TcpStream` is unsupported. Instead, we use the
//! `TlsProvider::connect_tcp()` method which the PSP backend implements
//! using raw `sceNetInet*` sockets + `psp::net::resolve_hostname` DNS.
//!
//! For HTTPS, `connect_tcp` + `connect_tls` gives a TLS-wrapped stream.
//! For HTTP, `connect_tcp` alone gives a raw TCP stream.
//!
//! This module mirrors the public API of `http.rs` so that
//! `lib.rs` can swap between them with a cfg gate. Response
//! framing (header parsing, `Content-Length` / chunked detection and
//! decoding) and the per-redirect-hop header policy are shared with the
//! desktop client through `http1.rs`; only connection setup and
//! the body cap differ.

use oasis_net::tls::TlsProvider;
use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::http1::{
    self, BodyLimit, Hop, HttpResponse, MAX_HEADER_SIZE, Oversize, ResponseTracker, find_header,
    is_redirect,
};
use super::{ContentType, ResourceResponse, Url};

/// Maximum response body size (2 MB -- PSP RAM constraint).
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Body-size policy: an oversized page is cut at the cap and rendered
/// as far as it got rather than failing (PSP RAM constraint).
const BODY_LIMIT: BodyLimit = BodyLimit {
    max_body: MAX_BODY_SIZE,
    oversize: Oversize::Truncate,
};

/// Maximum number of redirects to follow.
const MAX_REDIRECTS: u8 = 5;

/// Perform an HTTP(S) request and return the response with raw headers.
pub fn http_request_full(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    http_request_guarded(method, url, body, extra_headers, tls, &|_| true)
}

/// Like [`http_request_full`] but consults `redirect_ok` before following
/// each redirect; `false` aborts with an error. (PSP resolves hosts inside
/// `TlsProvider::connect_tcp`, so there is no per-connect address hook
/// here, unlike the desktop client's `RequestGuard`.)
pub fn http_request_guarded(
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
    tls: Option<&dyn TlsProvider>,
    redirect_ok: &dyn Fn(&Url) -> bool,
) -> Result<(ResourceResponse, Vec<(String, String)>)> {
    if url.scheme != "http" && url.scheme != "https" {
        return Err(OasisError::Backend(
            format!("unsupported scheme for PSP HTTP client: {}", url.scheme).into(),
        ));
    }

    let tls_provider = tls.ok_or_else(|| {
        OasisError::Backend("PSP requires a TLS provider for network connections".into())
    })?;

    let mut current_url = url.clone();
    let mut current_method = method.to_string();
    let mut current_body: Option<Vec<u8>> = body.map(|b| b.to_vec());

    let mut method_changed = false;

    // The initial request plus up to MAX_REDIRECTS followed redirects.
    for hop_index in 0..=usize::from(MAX_REDIRECTS) {
        // Credentials / validators / Referer are re-derived per hop
        // (see `http1::hop_headers`).
        let hop = Hop {
            original: url,
            current: &current_url,
            index: hop_index,
            method_changed,
        };
        let headers = http1::hop_headers(extra_headers, hop);
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let resp = do_request(
            tls_provider,
            &current_method,
            &current_url,
            current_body.as_deref(),
            &header_refs,
        )?;

        if is_redirect(resp.status_code)
            && let Some(location) = find_header(&resp.headers, "location")
        {
            let location = location.to_string();
            current_url = current_url.resolve(&location).ok_or_else(|| {
                OasisError::Backend(format!("bad redirect Location: {location}").into())
            })?;
            // Network responses may only redirect to the network (see
            // `http::http_request_inner`).
            if current_url.scheme != "http" && current_url.scheme != "https" {
                return Err(OasisError::Backend(
                    format!("redirect to non-HTTP scheme blocked: {current_url}").into(),
                ));
            }
            if !redirect_ok(&current_url) {
                return Err(OasisError::Backend(
                    format!("redirect to {current_url} blocked by request policy").into(),
                ));
            }
            // 307/308 must preserve the original method and body.
            // 301/302/303 convert to GET and drop the body.
            if !matches!(resp.status_code, 307 | 308) {
                method_changed |= current_method != "GET";
                current_method = "GET".to_string();
                current_body = None;
            }
            continue;
        }

        let content_type = find_header(&resp.headers, "content-type")
            .map(ContentType::from_mime)
            .unwrap_or_else(|| super::detect_content_type(&current_url));

        let headers = resp.headers;
        return Ok((
            ResourceResponse {
                url: current_url.to_string(),
                content_type,
                body: resp.body,
                status: resp.status_code,
            },
            headers,
        ));
    }

    Err(OasisError::Backend("too many redirects".into()))
}

// -------------------------------------------------------------------
// Core request logic
// -------------------------------------------------------------------

/// Open a connection (raw TCP for HTTP, TLS-wrapped for HTTPS), send
/// the request, and read the response.
fn do_request(
    tls_provider: &dyn TlsProvider,
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse> {
    crate::validate_request(method, url, extra_headers)?;
    let host = &url.host;
    let is_https = url.scheme == "https";
    let default_port: u16 = if is_https { 443 } else { 80 };
    let port = url.port.unwrap_or(default_port);

    // Create raw TCP connection via TlsProvider::connect_tcp.
    // On PSP this uses sceNetInet* sockets; on desktop it uses std::net.
    let tcp_stream = tls_provider.connect_tcp(host, port)?;

    // For HTTPS, upgrade to TLS.
    let mut stream: Box<dyn NetworkStream> = if is_https {
        tls_provider.connect_tls(tcp_stream, host)?
    } else {
        tcp_stream
    };

    // Build and send HTTP request.
    let path = url.request_target();

    let host_header = match url.port {
        Some(p) if p != default_port => format!("{host}:{p}"),
        _ => host.to_string(),
    };

    let mut request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         User-Agent: OASIS-PSP/1.0\r\n\
         Accept: text/html, application/xhtml+xml, */*\r\n\
         Accept-Encoding: identity\r\n\
         Connection: close\r\n"
    );

    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }

    if let Some(data) = body {
        let has_ct = extra_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"));
        if !has_ct {
            request.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        }
        request.push_str(&format!("Content-Length: {}\r\n", data.len()));
    }

    request.push_str("\r\n");

    write_all(&mut *stream, request.as_bytes())?;

    if let Some(data) = body {
        write_all(&mut *stream, data)?;
    }

    // Flush TLS buffers -- embedded-tls buffers records internally and
    // won't send until flushed. Without this, the server never receives
    // the request and we read 0 bytes.
    stream
        .flush()
        .map_err(|e| OasisError::Backend(format!("flush: {e}").into()))?;

    // Read response with early header detection: the shared tracker
    // parses the header lines as they arrive and stops at the end of the
    // body (Content-Length or the real chunked terminator), avoiding a
    // 30-second timeout wait on PSP sockets.
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    let mut tracker = ResponseTracker::for_method(method);
    // Safety limit: max read iterations to prevent infinite loops when
    // the TLS layer blocks without returning EOF (e.g. missing
    // close_notify from servers that ignore Connection: close).
    let mut reads_since_progress = 0u32;
    const MAX_STALL_READS: u32 = 64;

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                reads_since_progress = 0;
                if buf.len() + n > MAX_BODY_SIZE + MAX_HEADER_SIZE {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                match tracker.poll(&buf) {
                    Ok(Some(end)) => {
                        buf.truncate(end);
                        break;
                    },
                    Ok(None) => {},
                    // Oversized header block: stop reading; parsing
                    // below reports the error.
                    Err(_) => break,
                }
            },
            Err(_) => {
                // Read error (timeout, connection reset, etc.).
                // If we have headers and some body data, use what we got.
                if let Some(bs) = tracker.body_start()
                    && buf.len() > bs + 64
                {
                    break;
                }
                reads_since_progress += 1;
                if reads_since_progress >= MAX_STALL_READS {
                    break;
                }
            },
        }
    }
    let _ = stream.close();

    if buf.is_empty() {
        return Err(OasisError::Backend(
            format!("empty response from {host}:{port} (0 bytes received)").into(),
        ));
    }

    http1::parse_response(&buf, BODY_LIMIT).map_err(|e| {
        // Include buffer preview for debugging.
        let preview_len = buf.len().min(128);
        let preview = String::from_utf8_lossy(&buf[..preview_len]);
        OasisError::Backend(
            format!(
                "{e} (got {} bytes, preview: {:?})",
                buf.len(),
                preview.chars().take(80).collect::<String>()
            )
            .into(),
        )
    })
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Write all bytes, handling partial writes.
fn write_all(stream: &mut dyn NetworkStream, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let n = stream
            .write(data)
            .map_err(|e| OasisError::Backend(format!("write: {e}").into()))?;
        if n == 0 {
            return Err(OasisError::Backend("write returned 0 bytes".into()));
        }
        data = &data[n..];
    }
    Ok(())
}
