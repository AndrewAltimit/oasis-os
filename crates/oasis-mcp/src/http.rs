//! Minimal non-blocking HTTP/1.1 request framing.
//!
//! Bytes arrive across multiple polls, so [`try_frame`] is called repeatedly on
//! a growing buffer. It yields [`Framing::Pending`] (without consuming) until a
//! full request — headers up to `CRLFCRLF` plus exactly `Content-Length` body
//! bytes — is available, at which point it drains those bytes and returns
//! [`Framing::Ready`]. This mirrors the line-buffering discipline in
//! `oasis-net`'s `RemoteListener`.

/// Maximum bytes allowed in the request header section.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Maximum allowed request body size.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// A fully-parsed HTTP request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// Request method (e.g. `POST`).
    pub method: String,
    /// Request target (e.g. `/mcp`).
    pub path: String,
    /// Whether the connection should be kept alive after responding.
    pub keep_alive: bool,
    /// Bearer token from the `Authorization` header, if present.
    pub auth_bearer: Option<String>,
    /// Request body bytes.
    pub body: Vec<u8>,
}

/// Result of attempting to frame one request from a buffer.
pub enum Framing {
    /// Not enough bytes yet; leave the buffer untouched and try again later.
    Pending,
    /// A complete request was extracted (consumed bytes were drained).
    Ready(HttpRequest),
    /// The request was malformed; respond with this status code then close.
    Error(u16),
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Attempt to extract one complete HTTP/1.1 request from `buf`.
///
/// On [`Framing::Ready`] the consumed bytes are drained from `buf`; on
/// [`Framing::Pending`] the buffer is left unchanged (except for the header
/// overflow guard).
pub fn try_frame(buf: &mut Vec<u8>) -> Framing {
    let hdr_end = match find_subsequence(buf, b"\r\n\r\n") {
        Some(i) => i + 4,
        None => {
            if buf.len() > MAX_HEADER_BYTES {
                return Framing::Error(431);
            }
            return Framing::Pending;
        },
    };

    let Ok(header_str) = std::str::from_utf8(&buf[..hdr_end]) else {
        return Framing::Error(400);
    };

    let mut lines = header_str.split("\r\n");
    let Some(request_line) = lines.next() else {
        return Framing::Error(400);
    };
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("");
    if method.is_empty() || path.is_empty() {
        return Framing::Error(400);
    }

    let mut content_len: usize = 0;
    let mut keep_alive = version != "HTTP/1.0";
    let mut auth_bearer = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name_l = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name_l.as_str() {
            "content-length" => content_len = value.parse().unwrap_or(0),
            "transfer-encoding" if value.eq_ignore_ascii_case("chunked") => {
                return Framing::Error(411);
            },
            "connection" => {
                if value.eq_ignore_ascii_case("close") {
                    keep_alive = false;
                } else if value.eq_ignore_ascii_case("keep-alive") {
                    keep_alive = true;
                }
            },
            "authorization" => {
                let token = value
                    .strip_prefix("Bearer ")
                    .or_else(|| value.strip_prefix("bearer "));
                if let Some(t) = token {
                    auth_bearer = Some(t.trim().to_string());
                }
            },
            _ => {},
        }
    }

    if content_len > MAX_BODY_BYTES {
        return Framing::Error(413);
    }

    let total = hdr_end + content_len;
    if buf.len() < total {
        return Framing::Pending;
    }

    let raw: Vec<u8> = buf.drain(..total).collect();
    let body = raw[hdr_end..].to_vec();
    Framing::Ready(HttpRequest {
        method,
        path,
        keep_alive,
        auth_bearer,
        body,
    })
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "OK",
    }
}

/// Build a complete HTTP/1.1 response.
///
/// `Content-Length` and `Connection` are always emitted (except `Content-Length`
/// is omitted for `204`). `content_type` and any `extra` headers are appended.
pub fn build_response(
    status: u16,
    keep_alive: bool,
    extra: &[(&str, &str)],
    content_type: Option<&str>,
    body: &[u8],
) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status} {}\r\n", reason_phrase(status)).into_bytes();
    let conn = if keep_alive { "keep-alive" } else { "close" };
    out.extend_from_slice(format!("Connection: {conn}\r\n").as_bytes());
    if status != 204 {
        out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    if let Some(ct) = content_type {
        out.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
    }
    for (k, v) in extra {
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    if status != 204 {
        out.extend_from_slice(body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_complete_post() {
        let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        match try_frame(&mut buf) {
            Framing::Ready(req) => {
                assert_eq!(req.method, "POST");
                assert_eq!(req.path, "/mcp");
                assert_eq!(req.body, b"hello");
                assert!(req.keep_alive);
                assert!(buf.is_empty());
            },
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn pending_until_body_complete() {
        // Headers arrive but body is short.
        let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel".to_vec();
        assert!(matches!(try_frame(&mut buf), Framing::Pending));
        // Buffer untouched.
        assert_eq!(
            buf.len(),
            b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel".len()
        );
        // Rest of the body arrives.
        buf.extend_from_slice(b"lo");
        assert!(matches!(try_frame(&mut buf), Framing::Ready(_)));
    }

    #[test]
    fn pending_until_headers_complete() {
        let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Len".to_vec();
        assert!(matches!(try_frame(&mut buf), Framing::Pending));
    }

    #[test]
    fn parses_bearer_and_connection_close() {
        let mut buf = b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer sekret\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_vec();
        match try_frame(&mut buf) {
            Framing::Ready(req) => {
                assert_eq!(req.auth_bearer.as_deref(), Some("sekret"));
                assert!(!req.keep_alive);
                assert!(req.body.is_empty());
            },
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn rejects_chunked() {
        let mut buf = b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        assert!(matches!(try_frame(&mut buf), Framing::Error(411)));
    }

    #[test]
    fn two_pipelined_requests() {
        let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 1\r\n\r\naPOST /mcp HTTP/1.1\r\nContent-Length: 1\r\n\r\nb".to_vec();
        let r1 = try_frame(&mut buf);
        assert!(matches!(r1, Framing::Ready(ref r) if r.body == b"a"));
        let r2 = try_frame(&mut buf);
        assert!(matches!(r2, Framing::Ready(ref r) if r.body == b"b"));
    }

    #[test]
    fn response_has_headers() {
        let resp = build_response(200, true, &[], Some("application/json"), b"{}");
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.contains("Content-Type: application/json\r\n"));
        assert!(text.contains("Connection: keep-alive\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
    }
}
