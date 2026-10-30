//! HTTP/1.1 message framing and redirect-hop policy shared by the
//! desktop client (`http.rs`) and the PSP client (`http_psp.rs`).
//!
//! Everything here is pure: no sockets, no TLS. The two clients differ
//! only in how they open a stream and in their body-size cap; response
//! framing (header-block parsing, `Content-Length` / chunked detection,
//! chunked decoding), the "is the message complete yet" tracker used by
//! their read loops, and the per-hop request-header policy live here so
//! a fix lands on both.

use oasis_types::error::{OasisError, Result};

use super::Url;

/// Maximum HTTP header section size (16 KB).
///
/// Prevents a malicious server from exhausting memory by sending an
/// unbounded header block before the `\r\n\r\n` terminator.
pub const MAX_HEADER_SIZE: usize = 16_384;

/// A raw parsed HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Response headers as (name, value) pairs; names lower-cased.
    pub headers: Vec<(String, String)>,
    /// Response body bytes (de-chunked, not content-decoded).
    pub body: Vec<u8>,
}

/// What to do with a body larger than the client's cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oversize {
    /// Fail the request.
    Error,
    /// Keep the first `max_body` bytes (PSP: render what fits in RAM).
    Truncate,
}

/// Body-size policy for [`parse_response`].
#[derive(Debug, Clone, Copy)]
pub struct BodyLimit {
    /// Largest body accepted.
    pub max_body: usize,
    /// Behaviour above `max_body`.
    pub oversize: Oversize,
}

/// How a response body is delimited (RFC 9112 §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    /// `Transfer-Encoding: chunked` (wins over any `Content-Length`).
    Chunked,
    /// Exactly this many bytes.
    Length(usize),
    /// Read until the connection closes.
    UntilClose,
}

/// Find the end of the header section: `(offset, separator length)` for
/// the canonical `\r\n\r\n`, or a bare `\n\n` (RFC 9112 §2.2 leniency).
pub fn find_header_end(data: &[u8]) -> Option<(usize, usize)> {
    match (
        find_subsequence(data, b"\r\n\r\n"),
        find_subsequence(data, b"\n\n"),
    ) {
        (Some(crlf), Some(lf)) if lf < crlf => Some((lf, 2)),
        (Some(crlf), _) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

/// Parse a header section (status line plus header lines, without the
/// terminating blank line) into the status code and lower-cased headers.
///
/// Header bytes are decoded lossily: real servers occasionally send
/// ISO-8859-1 in header values.
pub fn parse_header_block(header_bytes: &[u8]) -> Result<(u16, Vec<(String, String)>)> {
    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l));

    // Status line: "HTTP/1.x STATUS REASON"
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
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok((status_code, headers))
}

/// Decide the body framing from parsed headers.
///
/// Header *lines* are inspected, so a value such as
/// `X-Note: transfer-encoding chunked` or a header named
/// `X-Content-Length` never changes the framing.
pub fn body_framing(headers: &[(String, String)]) -> BodyFraming {
    let chunked = headers
        .iter()
        .filter(|(k, _)| k == "transfer-encoding")
        .flat_map(|(_, v)| v.split(','))
        .any(|coding| coding.trim().eq_ignore_ascii_case("chunked"));
    if chunked {
        return BodyFraming::Chunked;
    }
    match find_header(headers, "content-length").map(|v| v.trim().parse::<usize>()) {
        Some(Ok(n)) => BodyFraming::Length(n),
        _ => BodyFraming::UntilClose,
    }
}

/// Incremental "is the response complete?" detector for read loops.
///
/// Feed it the whole receive buffer after every read; once headers have
/// arrived it knows the framing, and it reports the total message length
/// as soon as the body is complete, so keep-alive and `Connection:
/// close`-ignoring servers don't make the caller wait for EOF.
#[derive(Debug, Default)]
pub struct ResponseTracker {
    body_start: Option<usize>,
    framing: Option<BodyFraming>,
    /// Offset (within the body) of the next unparsed chunk-size line.
    chunk_cursor: usize,
}

impl ResponseTracker {
    /// A tracker for a fresh response.
    pub fn new() -> Self {
        Self::default()
    }

    /// Offset of the first body byte, once the headers have arrived.
    pub fn body_start(&self) -> Option<usize> {
        self.body_start
    }

    /// Inspect `buf` (everything received so far). Returns the complete
    /// message length once it has fully arrived, `Ok(None)` while more
    /// data is needed (including bodies delimited by connection close),
    /// and an error when the header section exceeds [`MAX_HEADER_SIZE`].
    pub fn poll(&mut self, buf: &[u8]) -> Result<Option<usize>> {
        if self.body_start.is_none() {
            let Some((end, sep)) = find_header_end(buf) else {
                if buf.len() > MAX_HEADER_SIZE {
                    return Err(header_limit_error());
                }
                return Ok(None);
            };
            if end > MAX_HEADER_SIZE {
                return Err(header_limit_error());
            }
            self.body_start = Some(end + sep);
            // A malformed status line is reported by `parse_response`;
            // here it just means "read until close".
            self.framing = Some(
                parse_header_block(&buf[..end])
                    .map(|(_, h)| body_framing(&h))
                    .unwrap_or(BodyFraming::UntilClose),
            );
        }
        let (Some(bs), Some(framing)) = (self.body_start, self.framing) else {
            return Ok(None);
        };
        let body = &buf[bs..];
        Ok(match framing {
            BodyFraming::Length(n) => (body.len() >= n).then(|| bs.saturating_add(n)),
            BodyFraming::Chunked => scan_chunked(body, &mut self.chunk_cursor).map(|end| bs + end),
            BodyFraming::UntilClose => None,
        })
    }
}

fn header_limit_error() -> OasisError {
    OasisError::Backend("HTTP headers exceed 16 KB limit".into())
}

/// Incrementally scan a chunked transfer-encoded body.
///
/// `cursor` is the offset of the first chunk-size line not yet known to
/// be complete; it advances over every fully received chunk so repeated
/// calls (one per socket read) stay linear. Returns the total length of
/// the chunked body — through the zero-size chunk and its trailer
/// section — once it has fully arrived, or `None` while more data is
/// needed. Malformed framing ends the scan at the data received so far
/// so the caller stops reading and `decode_chunked` reports the error.
///
/// Matching byte patterns at the tail of the buffer is not enough: a
/// chunk whose data ends in `0\r\n` looks like the terminator when a read
/// happens to stop right after it.
pub fn scan_chunked(data: &[u8], cursor: &mut usize) -> Option<usize> {
    loop {
        let rest = data.get(*cursor..)?;
        let Some(line_len) = find_subsequence(rest, b"\r\n") else {
            // A chunk-size line is a few hex digits plus optional
            // extensions; anything this long without CRLF is garbage.
            return (rest.len() > 1024).then_some(data.len());
        };
        let size_str = std::str::from_utf8(&rest[..line_len]).unwrap_or("");
        let size_str = size_str.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_str, 16) else {
            return Some(data.len());
        };
        let data_start = *cursor + line_len + 2;
        if size == 0 {
            // Trailer section: header lines, then an empty line.
            let trailers = data.get(data_start..)?;
            if trailers.starts_with(b"\r\n") {
                return Some(data_start + 2);
            }
            return find_subsequence(trailers, b"\r\n\r\n").map(|i| data_start + i + 4);
        }
        let Some(next) = data_start.checked_add(size).and_then(|e| e.checked_add(2)) else {
            return Some(data.len());
        };
        if next > data.len() {
            return None;
        }
        *cursor = next;
    }
}

/// Decode a chunked transfer-encoded body, enforcing `limit`.
///
/// A truncated final chunk keeps the bytes that arrived (the page renders
/// what it got). With [`Oversize::Truncate`] an oversized body is cut at
/// `limit.max_body`; with [`Oversize::Error`] it fails.
pub fn decode_chunked(data: &[u8], limit: BodyLimit) -> Result<Vec<u8>> {
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

        // Strip optional chunk extensions (after `;`).
        let size_str = size_str.split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|_| OasisError::Backend("bad chunk size".into()))?;

        if chunk_size == 0 {
            break;
        }

        let chunk_start = line_end + 2;
        let Some(chunk_end) = chunk_start.checked_add(chunk_size) else {
            break; // overflow
        };
        let chunk = &data[chunk_start.min(data.len())..chunk_end.min(data.len())];

        if result.len() + chunk.len() > limit.max_body {
            match limit.oversize {
                Oversize::Error => {
                    return Err(OasisError::Backend(
                        format!("chunked body exceeds {} limit", size_label(limit.max_body)).into(),
                    ));
                },
                Oversize::Truncate => {
                    let room = limit.max_body - result.len();
                    result.extend_from_slice(&chunk[..room]);
                    break;
                },
            }
        }
        result.extend_from_slice(chunk);
        if chunk_end > data.len() {
            // Partial chunk -- keep what we have.
            break;
        }
        // Skip past chunk data and trailing \r\n.
        let Some(p) = chunk_end.checked_add(2) else {
            break; // overflow
        };
        pos = p;
    }

    Ok(result)
}

/// Parse raw bytes into status code, headers, and (de-chunked) body.
///
/// Accepts both `\r\n` (standard) and bare `\n` (RFC 9112 §2.2) line
/// endings in the header section. Content-decoding (`gzip`, ...) is left
/// to the caller.
pub fn parse_response(data: &[u8], limit: BodyLimit) -> Result<HttpResponse> {
    let (header_end, separator_len) = find_header_end(data).ok_or_else(|| {
        OasisError::Backend("malformed HTTP response: no header terminator".into())
    })?;
    if header_end > MAX_HEADER_SIZE {
        return Err(header_limit_error());
    }
    let (status_code, headers) = parse_header_block(&data[..header_end])?;
    let raw_body = &data[header_end + separator_len..];

    let too_large = || {
        OasisError::Backend(
            format!("response body exceeds {} limit", size_label(limit.max_body)).into(),
        )
    };
    let body = match body_framing(&headers) {
        BodyFraming::Chunked => decode_chunked(raw_body, limit)?,
        BodyFraming::Length(len) => {
            if len > limit.max_body && limit.oversize == Oversize::Error {
                return Err(too_large());
            }
            let take = len.min(limit.max_body).min(raw_body.len());
            raw_body[..take].to_vec()
        },
        BodyFraming::UntilClose => {
            if find_header(&headers, "content-length").is_some() {
                return Err(OasisError::Backend("bad Content-Length".into()));
            }
            if raw_body.len() > limit.max_body && limit.oversize == Oversize::Error {
                return Err(too_large());
            }
            raw_body[..raw_body.len().min(limit.max_body)].to_vec()
        },
    };

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

/// Human-readable size for limit errors ("8 MB").
fn size_label(bytes: usize) -> String {
    if bytes >= 1024 * 1024 && bytes.is_multiple_of(1024 * 1024) {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{bytes} bytes")
    }
}

/// Parse the HTTP status code from the status line.
pub fn parse_status_line(line: &str) -> Result<u16> {
    // Expected: "HTTP/1.x NNN ..."
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

/// Case-insensitive header lookup (headers are stored lower-cased).
pub fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Whether a status code is a redirect we should follow.
pub fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Find the position of a byte subsequence in a slice.
pub fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// -------------------------------------------------------------------
// Redirect-hop header policy
// -------------------------------------------------------------------

/// Where a request hop stands relative to the originally requested URL.
#[derive(Debug, Clone, Copy)]
pub struct Hop<'a> {
    /// The URL the caller asked for.
    pub original: &'a Url,
    /// The URL this hop is sent to.
    pub current: &'a Url,
    /// `0` for the initial request, `n` after `n` followed redirects.
    pub index: usize,
    /// The method was rewritten (POST → GET) by a redirect, so the body
    /// and its headers were dropped.
    pub method_changed: bool,
}

/// Request headers to send on `hop`, derived from the caller-supplied
/// headers of the original request:
///
/// - Conditional headers (`If-None-Match`, `If-Modified-Since`, ...) are
///   cache validators for the *original* URL and are sent on hop 0 only.
/// - Credentials (`Cookie`, `Authorization`, `Proxy-Authorization`) are
///   dropped as soon as a hop leaves the original origin. Cookies for the
///   hop itself come from the client's [`super::cookies::CookieHook`].
/// - `Referer` follows `strict-origin-when-cross-origin` against the hop
///   URL: the full (already query-stripped) referrer same-origin, only
///   its origin cross-origin, and nothing on an `https:` → `http:`
///   downgrade.
/// - When a redirect turned the request into a body-less GET, the body
///   headers (`Content-Type`, `Content-Encoding`, `Content-Language`,
///   `Content-Location`) go with the body.
pub fn hop_headers(extra: &[(&str, &str)], hop: Hop<'_>) -> Vec<(String, String)> {
    let cross_origin = !hop.current.same_origin(hop.original);
    let mut out = Vec::with_capacity(extra.len());
    for &(name, value) in extra {
        let lower = name.to_ascii_lowercase();
        let keep = match lower.as_str() {
            "if-none-match"
            | "if-modified-since"
            | "if-match"
            | "if-unmodified-since"
            | "if-range" => hop.index == 0,
            "cookie" | "authorization" | "proxy-authorization" => !cross_origin,
            "content-type" | "content-encoding" | "content-language" | "content-location" => {
                !hop.method_changed
            },
            "referer" => {
                if let Some(r) = referrer_for(value, hop.current) {
                    out.push((name.to_string(), r));
                }
                false
            },
            _ => true,
        };
        if keep {
            out.push((name.to_string(), value.to_string()));
        }
    }
    out
}

/// Apply `strict-origin-when-cross-origin` to `referrer` for a request to
/// `target`.
pub fn referrer_for(referrer: &str, target: &Url) -> Option<String> {
    let Some(r) = Url::parse(referrer) else {
        return Some(referrer.to_string());
    };
    if r.scheme == "https" && target.scheme != "https" {
        return None;
    }
    if r.same_origin(target) {
        return Some(referrer.to_string());
    }
    r.origin().map(|o| format!("{o}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: BodyLimit = BodyLimit {
        max_body: 8 * 1024 * 1024,
        oversize: Oversize::Error,
    };

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn framing_parses_header_lines_not_substrings() {
        let h = |raw: &str| parse_header_block(raw.as_bytes()).unwrap().1;
        // A header *value* mentioning chunked must not switch framing.
        let headers = h("HTTP/1.1 200 OK\r\nX-Note: transfer-encoding chunked\r\n\
                         Content-Length: 5");
        assert_eq!(body_framing(&headers), BodyFraming::Length(5));
        // `X-Content-Length` is not `Content-Length`.
        let headers = h("HTTP/1.1 200 OK\r\nX-Content-Length: 99");
        assert_eq!(body_framing(&headers), BodyFraming::UntilClose);
        // Chunked wins over Content-Length; codings are a list.
        let headers = h("HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\
                         Transfer-Encoding: gzip, Chunked");
        assert_eq!(body_framing(&headers), BodyFraming::Chunked);
    }

    #[test]
    fn tracker_stops_at_content_length_and_chunk_terminator() {
        let mut t = ResponseTracker::new();
        let msg = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbodyEXTRA";
        assert_eq!(t.poll(&msg[..20]).unwrap(), None);
        assert_eq!(t.poll(msg).unwrap(), Some(msg.len() - 5));

        // The value-substring trap that used to force chunked mode.
        let mut t = ResponseTracker::new();
        let msg = b"HTTP/1.1 200 OK\r\nX-Hint: transfer-encoding chunked\r\n\
                    Content-Length: 2\r\n\r\nok";
        assert_eq!(t.poll(msg).unwrap(), Some(msg.len()));

        let mut t = ResponseTracker::new();
        let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        let mut msg = head.clone();
        // Chunk data ending in "0\r\n" must not look like the end.
        msg.extend_from_slice(b"10\r\nabcdefghijklm0\r\n\r\n");
        assert_eq!(t.poll(&msg).unwrap(), None);
        msg.extend_from_slice(b"0\r\n\r\n");
        assert_eq!(t.poll(&msg).unwrap(), Some(msg.len()));
    }

    #[test]
    fn scan_chunked_waits_for_the_real_terminator() {
        let mut cur = 0;
        // Chunk data ending in "0\r\n" must not look like the end.
        let partial = b"10\r\nabcdefghijklm0\r\n\r\n";
        assert_eq!(scan_chunked(partial, &mut cur), None);
        let mut full = partial.to_vec();
        full.extend_from_slice(b"3\r\nxyz\r\n0\r\n\r\nNEXT");
        assert_eq!(scan_chunked(&full, &mut cur), Some(full.len() - 4));
    }

    #[test]
    fn scan_chunked_handles_trailers_and_garbage() {
        let mut cur = 0;
        let body = b"2\r\nhi\r\n0\r\nX-T: 1\r\n\r\n";
        assert_eq!(scan_chunked(body, &mut cur), Some(body.len()));
        let mut cur = 0;
        assert_eq!(scan_chunked(b"zz\r\n", &mut cur), Some(4));
        let mut cur = 0;
        assert_eq!(scan_chunked(b"0\r\n", &mut cur), None);
    }

    #[test]
    fn tracker_rejects_oversized_headers() {
        let mut t = ResponseTracker::new();
        let msg = vec![b'a'; MAX_HEADER_SIZE + 1];
        assert!(t.poll(&msg).is_err());
    }

    #[test]
    fn decode_chunked_limits() {
        let data = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let small_err = BodyLimit {
            max_body: 7,
            oversize: Oversize::Error,
        };
        assert!(decode_chunked(data, small_err).is_err());
        let small_trunc = BodyLimit {
            max_body: 7,
            oversize: Oversize::Truncate,
        };
        assert_eq!(decode_chunked(data, small_trunc).unwrap(), b"hello w");
        assert_eq!(decode_chunked(data, DESKTOP).unwrap(), b"hello world");
    }

    #[test]
    fn content_length_truncate_vs_error() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789";
        let trunc = BodyLimit {
            max_body: 4,
            oversize: Oversize::Truncate,
        };
        assert_eq!(parse_response(raw, trunc).unwrap().body, b"0123");
        let err = BodyLimit {
            max_body: 4,
            oversize: Oversize::Error,
        };
        assert!(parse_response(raw, err).is_err());
    }

    #[test]
    fn hop_zero_keeps_everything_same_origin() {
        let u = url("http://a.test/x");
        let extra = [
            ("Cookie", "a=1"),
            ("If-None-Match", "\"e\""),
            ("Referer", "http://a.test/page"),
        ];
        let hop = Hop {
            original: &u,
            current: &u,
            index: 0,
            method_changed: false,
        };
        let out = hop_headers(&extra, hop);
        assert_eq!(out.len(), 3);
        assert!(
            out.iter()
                .any(|(k, v)| k == "Referer" && v == "http://a.test/page")
        );
    }

    #[test]
    fn cross_origin_hop_strips_credentials_and_validators() {
        let orig = url("https://bank.test/login");
        let next = url("https://evil.test/");
        let extra = [
            ("Cookie", "sid=1"),
            ("Authorization", "Basic x"),
            ("If-Modified-Since", "Mon, 01 Jan 2024 00:00:00 GMT"),
            ("Referer", "https://bank.test/account"),
            ("Accept-Language", "en"),
            ("Content-Type", "application/json"),
        ];
        let hop = Hop {
            original: &orig,
            current: &next,
            index: 1,
            method_changed: true,
        };
        let out = hop_headers(&extra, hop);
        let names: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["Referer", "Accept-Language"]);
        // Cross-origin referrer is reduced to the origin.
        assert_eq!(out[0].1, "https://bank.test/");
    }

    #[test]
    fn referrer_is_dropped_on_downgrade() {
        assert_eq!(
            referrer_for("https://bank.test/a", &url("http://bank.test/b")),
            None
        );
        assert_eq!(
            referrer_for("http://a.test/p", &url("https://a.test/q")),
            Some("http://a.test/".into())
        );
        assert_eq!(
            referrer_for("https://a.test/p", &url("https://a.test/q")),
            Some("https://a.test/p".into())
        );
    }

    #[test]
    fn same_origin_redirect_keeps_cookie_but_drops_validators() {
        let orig = url("http://a.test/old");
        let next = url("http://A.test:80/new");
        let extra = [("Cookie", "a=1"), ("If-None-Match", "\"e\"")];
        let hop = Hop {
            original: &orig,
            current: &next,
            index: 1,
            method_changed: false,
        };
        let out = hop_headers(&extra, hop);
        assert_eq!(out, vec![("Cookie".to_string(), "a=1".to_string())]);
    }
}
