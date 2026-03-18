//! PSP HTTP client using `TlsProvider::connect_tcp` for raw sockets.
//!
//! On PSP, `std::net::TcpStream` is unsupported. Instead, we use the
//! `TlsProvider::connect_tcp()` method which the PSP backend implements
//! using raw `sceNetInet*` sockets + `psp::net::resolve_hostname` DNS.
//!
//! For HTTPS, `connect_tcp` + `connect_tls` gives a TLS-wrapped stream.
//! For HTTP, `connect_tcp` alone gives a raw TCP stream.
//!
//! This module mirrors the public API of `loader/http.rs` so that
//! `loader/mod.rs` can swap between them with a cfg gate.

use oasis_net::tls::TlsProvider;
use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::{ContentType, ResourceResponse, Url};

/// Maximum response body size (2 MB -- PSP RAM constraint).
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Maximum HTTP header section size (16 KB).
const MAX_HEADER_SIZE: usize = 16_384;

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

    for _ in 0..MAX_REDIRECTS {
        let resp = do_request(
            tls_provider,
            &current_method,
            &current_url,
            current_body.as_deref(),
            extra_headers,
        )?;

        if is_redirect(resp.status_code) {
            if let Some(location) = find_header(&resp.headers, "location") {
                let location = location.to_string();
                current_url = current_url.resolve(&location).ok_or_else(|| {
                    OasisError::Backend(format!("bad redirect Location: {location}").into())
                })?;
                // 307/308 must preserve the original method and body.
                // 301/302/303 convert to GET and drop the body.
                if !matches!(resp.status_code, 307 | 308) {
                    current_method = "GET".to_string();
                    current_body = None;
                }
                continue;
            }
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

/// Case-insensitive header lookup.
pub fn response_find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    find_header(headers, name)
}

// -------------------------------------------------------------------
// Internal types
// -------------------------------------------------------------------

struct HttpResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
    let path = if let Some(ref q) = url.query {
        format!("{}?{}", url.path, q)
    } else {
        url.path.clone()
    };

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

    // Read response with early header detection.
    // We parse headers as they arrive to determine body length, avoiding
    // a 30-second timeout wait on PSP sockets.
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    let mut body_start: Option<usize> = None;
    let mut expected_body_len: Option<usize> = None;
    let mut is_chunked = false;
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

                // Detect header/body boundary once.
                if body_start.is_none() {
                    if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                        let hdr_end = pos + 4;
                        body_start = Some(hdr_end);
                        let hdr = String::from_utf8_lossy(&buf[..pos]);
                        let hdr_lower = hdr.to_ascii_lowercase();
                        // Anchor header searches to line boundaries to
                        // avoid matching e.g. X-Content-Length.
                        if hdr_lower.contains("\ntransfer-encoding:")
                            && hdr_lower.contains("chunked")
                        {
                            is_chunked = true;
                        } else if let Some(cl_start) = hdr_lower.find("\ncontent-length:") {
                            let after = &hdr_lower[cl_start + 16..];
                            let line_end = after.find('\n').unwrap_or(after.len());
                            if let Ok(cl) = after[..line_end].trim().parse::<usize>() {
                                expected_body_len = Some(cl);
                            }
                        }
                    } else if let Some(pos) = find_subsequence(&buf, b"\n\n") {
                        let hdr_end = pos + 2;
                        body_start = Some(hdr_end);
                        let hdr = String::from_utf8_lossy(&buf[..pos]);
                        let hdr_lower = hdr.to_ascii_lowercase();
                        // Anchor header searches to line boundaries to
                        // avoid matching e.g. X-Content-Length.
                        if hdr_lower.contains("\ntransfer-encoding:")
                            && hdr_lower.contains("chunked")
                        {
                            is_chunked = true;
                        } else if let Some(cl_start) = hdr_lower.find("\ncontent-length:") {
                            let after = &hdr_lower[cl_start + 16..];
                            let line_end = after.find('\n').unwrap_or(after.len());
                            if let Ok(cl) = after[..line_end].trim().parse::<usize>() {
                                expected_body_len = Some(cl);
                            }
                        }
                    } else if buf.len() > MAX_HEADER_SIZE {
                        break;
                    }
                }

                // Check if we have the complete body.
                if let Some(bs) = body_start {
                    if let Some(expected) = expected_body_len {
                        if buf.len() - bs >= expected {
                            buf.truncate(bs + expected);
                            break;
                        }
                    } else if is_chunked {
                        // Search for the chunked terminator anywhere in
                        // the body data, not just at the end. Servers may
                        // append trailing headers or padding after the
                        // final `0\r\n\r\n` chunk marker.
                        let chunk_data = &buf[bs..];
                        if find_subsequence(chunk_data, b"\r\n0\r\n\r\n").is_some()
                            || find_subsequence(chunk_data, b"\n0\r\n\r\n").is_some()
                            || (chunk_data.starts_with(b"0\r\n") && chunk_data.len() <= 7)
                        {
                            break;
                        }
                    }
                    // No content-length + not chunked: read until EOF/error.
                }
            },
            Err(_) => {
                // Read error (timeout, connection reset, etc.).
                // If we have headers and some body data, use what we got.
                if body_start.is_some() && buf.len() > body_start.unwrap_or(0) + 64 {
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

    parse_response(&buf).map_err(|e| {
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
// Response parsing
// -------------------------------------------------------------------

fn parse_response(data: &[u8]) -> Result<HttpResponse> {
    let (header_end, separator_len) = if let Some(pos) = find_subsequence(data, b"\r\n\r\n") {
        (pos, 4)
    } else if let Some(pos) = find_subsequence(data, b"\n\n") {
        (pos, 2)
    } else {
        return Err(OasisError::Backend(
            "malformed HTTP response: no header terminator".into(),
        ));
    };

    if header_end > MAX_HEADER_SIZE {
        return Err(OasisError::Backend(
            "HTTP headers exceed 16 KB limit".into(),
        ));
    }

    let header_bytes = &data[..header_end];
    let body_start = header_end + separator_len;

    // Use lossy conversion -- real-world servers sometimes send
    // ISO-8859-1 in headers (RFC 7230 allows only ASCII, but we
    // should be robust).
    let header_str = String::from_utf8_lossy(header_bytes);

    let header_owned;
    let header_normalized: &str = if header_str.contains("\r\n") {
        header_owned = header_str.replace("\r\n", "\n");
        header_owned.as_str()
    } else {
        &header_str
    };

    let mut lines = header_normalized.split('\n');

    let status_line = lines
        .next()
        .ok_or_else(|| OasisError::Backend("empty response".into()))?;
    let status_code = parse_status_line(status_line)?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }

    let raw_body = &data[body_start..];
    let body = if find_header(&headers, "transfer-encoding").is_some_and(|v| v.contains("chunked"))
    {
        decode_chunked(raw_body)?
    } else if let Some(cl) = find_header(&headers, "content-length") {
        let len: usize = cl
            .parse()
            .map_err(|_| OasisError::Backend("bad Content-Length".into()))?;
        let capped = len.min(MAX_BODY_SIZE);
        raw_body[..raw_body.len().min(capped)].to_vec()
    } else {
        raw_body.to_vec()
    };

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

fn parse_status_line(line: &str) -> Result<u16> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(OasisError::Backend(
            format!("bad status line: {line}").into(),
        ));
    }
    parts[1]
        .parse()
        .map_err(|_| OasisError::Backend(format!("bad status code in: {line}").into()))
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let name_lower = name.to_lowercase();
    headers
        .iter()
        .find(|(k, _)| k == &name_lower)
        .map(|(_, v)| v.as_str())
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let remaining = &data[pos..];
        let Some(i) = find_subsequence(remaining, b"\r\n") else {
            break;
        };
        let line_end = pos + i;

        let size_str = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| OasisError::Backend("bad chunk size".into()))?
            .trim();
        let size_str = size_str.split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|_| OasisError::Backend("bad chunk size".into()))?;

        if chunk_size == 0 {
            break;
        }

        let chunk_start = line_end + 2;
        let chunk_end = match chunk_start.checked_add(chunk_size) {
            Some(end) => end,
            None => break,
        };

        if chunk_end > data.len() {
            if chunk_start < data.len() {
                result.extend_from_slice(&data[chunk_start..]);
            }
            break;
        }

        if result.len() + chunk_size > MAX_BODY_SIZE {
            break;
        }

        result.extend_from_slice(&data[chunk_start..chunk_end]);
        pos = match chunk_end.checked_add(2) {
            Some(p) => p,
            None => break,
        };
    }

    Ok(result)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

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
