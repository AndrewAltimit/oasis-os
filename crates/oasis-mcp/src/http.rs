//! Minimal non-blocking HTTP/1.1 request framing.
//!
//! Bytes arrive across multiple polls, so [`try_frame`] is called repeatedly on
//! a growing buffer. It yields [`Framing::Pending`] (without consuming) until a
//! full request — headers up to `CRLFCRLF` plus exactly `Content-Length` body
//! bytes — is available, at which point it drains those bytes and returns
//! [`Framing::Ready`]. This mirrors the line-buffering discipline in
//! `oasis-net`'s `RemoteListener`.
//!
//! The parser is deliberately strict (RFC 9112): the request line must be
//! `METHOD target HTTP/1.x`, every header line must be `name: value` with
//! no whitespace in or before the name (obs-fold continuation lines are
//! rejected), any `Transfer-Encoding` is refused (only `Content-Length`
//! bodies are supported), and the whole header section is capped at
//! [`MAX_HEADER_BYTES`] whether or not it arrives in one read.

/// Maximum bytes allowed in the request header section.
pub(crate) const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Maximum allowed request body size.
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The parsed header section of a request (everything but the body).
#[derive(Debug, Clone)]
pub(crate) struct RequestHead {
    /// Request method (e.g. `POST`).
    pub method: String,
    /// Request target (e.g. `/mcp`).
    pub path: String,
    /// Whether the connection should be kept alive after responding.
    pub keep_alive: bool,
    /// Bearer token from the `Authorization` header, if present.
    pub auth_bearer: Option<String>,
    /// Value of the `Host` header, if present.
    pub host: Option<String>,
    /// Value of the `Origin` header, if present.
    pub origin: Option<String>,
    /// Value of the `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// Declared body length (`0` without a `Content-Length`).
    pub content_len: usize,
    /// The client sent `Expect: 100-continue`.
    pub expect_continue: bool,
}

/// A fully-parsed HTTP request.
#[derive(Debug, Clone)]
pub(crate) struct HttpRequest {
    /// The header section.
    pub head: RequestHead,
    /// Request body bytes (`head.content_len` of them).
    pub body: Vec<u8>,
}

/// Result of attempting to frame one request from a buffer.
pub(crate) enum Framing {
    /// Not enough bytes yet; leave the buffer untouched and try again later.
    Pending,
    /// A complete request was extracted (consumed bytes were drained).
    Ready(HttpRequest),
    /// The request was malformed; respond with this status code then close.
    Error(u16),
}

pub(crate) fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Length of the header section (through `CRLFCRLF`) at the start of `buf`.
pub(crate) fn header_end(buf: &[u8]) -> Option<usize> {
    find_subsequence(buf, b"\r\n\r\n").map(|i| i + 4)
}

/// Parse a complete header section (`head` ends with `CRLFCRLF`).
///
/// Errors are the HTTP status to answer with.
pub(crate) fn parse_head(head: &[u8]) -> Result<RequestHead, u16> {
    if head.len() > MAX_HEADER_BYTES {
        return Err(431);
    }
    let header_str = std::str::from_utf8(head).map_err(|_| 400u16)?;
    let mut lines = header_str.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(400);
    };
    if method.is_empty() || path.is_empty() {
        return Err(400);
    }
    let mut keep_alive = match version {
        "HTTP/1.1" => true,
        "HTTP/1.0" => false,
        _ => return Err(505),
    };

    let mut content_len: Option<usize> = None;
    let mut auth_bearer = None;
    let mut host = None;
    let mut origin = None;
    let mut content_type = None;
    let mut expect_continue = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        // No colon, an empty name, or whitespace in / before the name
        // (including obs-fold continuation lines) is malformed.
        let Some((name, value)) = line.split_once(':') else {
            return Err(400);
        };
        if name.is_empty() || name.bytes().any(|b| b.is_ascii_whitespace()) {
            return Err(400);
        }
        let name_l = name.to_ascii_lowercase();
        let value = value.trim();
        match name_l.as_str() {
            "content-length" => {
                // Reject malformed and duplicate lengths outright: accepting
                // either is a classic request-smuggling / desync vector.
                if content_len.is_some() {
                    return Err(400);
                }
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(400);
                }
                match value.parse::<usize>() {
                    Ok(n) => content_len = Some(n),
                    // All digits but overflows usize: certainly too large.
                    Err(_) => return Err(413),
                }
            },
            // A second `Host` (RFC 9112 3.2: answer 400) or `Origin` would
            // silently replace the first, so the gate could vet a different
            // value than a front proxy or the client meant.
            "host" | "origin" => {
                let slot = if name_l == "host" {
                    &mut host
                } else {
                    &mut origin
                };
                if slot.is_some() {
                    return Err(400);
                }
                *slot = Some(value.to_string());
            },
            "content-type" => content_type = Some(value.to_string()),
            // Only `Content-Length` framing is supported: any transfer
            // coding (`chunked`, `gzip, chunked`, even `identity`) would
            // otherwise be ignored and its bytes parsed as the next request.
            "transfer-encoding" => return Err(411),
            "expect" if value.eq_ignore_ascii_case("100-continue") => expect_continue = true,
            "connection" => {
                if value.eq_ignore_ascii_case("close") {
                    keep_alive = false;
                } else if value.eq_ignore_ascii_case("keep-alive") {
                    keep_alive = true;
                }
            },
            "authorization" => {
                // The auth scheme is case-insensitive (RFC 9110 11.1).
                if let Some((scheme, token)) = value.split_once(' ')
                    && scheme.eq_ignore_ascii_case("bearer")
                {
                    auth_bearer = Some(token.trim().to_string());
                }
            },
            _ => {},
        }
    }

    let content_len = content_len.unwrap_or(0);
    if content_len > MAX_BODY_BYTES {
        return Err(413);
    }
    Ok(RequestHead {
        method: method.to_string(),
        path: path.to_string(),
        keep_alive,
        auth_bearer,
        host,
        origin,
        content_type,
        content_len,
        expect_continue,
    })
}

/// Attempt to extract one complete HTTP/1.1 request from `buf`.
///
/// On [`Framing::Ready`] the consumed bytes are drained from `buf`; on
/// [`Framing::Pending`] the buffer is left unchanged.
pub(crate) fn try_frame(buf: &mut Vec<u8>) -> Framing {
    let Some(hdr_end) = header_end(buf) else {
        if buf.len() > MAX_HEADER_BYTES {
            return Framing::Error(431);
        }
        return Framing::Pending;
    };
    let head = match parse_head(&buf[..hdr_end]) {
        Ok(head) => head,
        Err(code) => return Framing::Error(code),
    };
    let total = hdr_end + head.content_len;
    if buf.len() < total {
        return Framing::Pending;
    }

    let raw: Vec<u8> = buf.drain(..total).collect();
    let body = raw[hdr_end..].to_vec();
    Framing::Ready(HttpRequest { head, body })
}

/// The interim response sent for `Expect: 100-continue`.
pub(crate) const CONTINUE_RESPONSE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        505 => "HTTP Version Not Supported",
        // Every status the server sends is listed above; never call an
        // unlisted (error) status "OK".
        _ => "Unknown",
    }
}

/// Build a complete HTTP/1.1 response.
///
/// `Content-Length` and `Connection` are always emitted (except `Content-Length`
/// is omitted for `204`). `content_type` and any `extra` headers are appended.
pub(crate) fn build_response(
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
mod tests;
