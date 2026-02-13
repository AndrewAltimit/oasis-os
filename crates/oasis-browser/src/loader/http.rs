//! Minimal HTTP/1.1 GET client.
//!
//! Supports plain HTTP over `std::net::TcpStream` and, when a
//! [`TlsProvider`] is supplied, HTTPS via the backend's TLS stack.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use oasis_net::tls::TlsProvider;
use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::{ContentType, ResourceResponse, Url};

/// Maximum response body size (8 MB).
const MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

/// Maximum number of redirects to follow.
const MAX_REDIRECTS: u8 = 5;

/// TCP connect timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// TCP read timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Perform an HTTP(S) GET request for the given URL.
///
/// When `tls` is `Some`, HTTPS URLs are supported.  When `None`, HTTPS
/// URLs produce a user-friendly error page instead.
///
/// Follows redirects (301/302/307/308) up to [`MAX_REDIRECTS`] hops.
pub fn http_get(url: &Url, tls: Option<&dyn TlsProvider>) -> Result<ResourceResponse> {
    if url.scheme == "https" && tls.is_none() {
        return Ok(https_error_page(url, url));
    }
    if url.scheme != "http" && url.scheme != "https" {
        return Err(OasisError::Backend(format!(
            "unsupported scheme for HTTP client: {}",
            url.scheme,
        )));
    }

    let mut current_url = url.clone();
    for _ in 0..MAX_REDIRECTS {
        let resp = do_request(&current_url, tls)?;

        if is_redirect(resp.status_code)
            && let Some(location) = find_header(&resp.headers, "location")
        {
            let location = location.to_string();
            current_url = current_url
                .resolve(&location)
                .ok_or_else(|| OasisError::Backend(format!("bad redirect Location: {location}")))?;
            if current_url.scheme == "https" && tls.is_none() {
                return Ok(https_error_page(url, &current_url));
            }
            continue;
        }

        let content_type = find_header(&resp.headers, "content-type")
            .map(ContentType::from_mime)
            .unwrap_or_else(|| super::detect_content_type(&current_url));

        return Ok(ResourceResponse {
            url: current_url.to_string(),
            content_type,
            body: resp.body,
            status: resp.status_code,
        });
    }

    Err(OasisError::Backend("too many redirects".to_string()))
}

// -------------------------------------------------------------------
// Internal types
// -------------------------------------------------------------------

/// A raw parsed HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Response headers as (name, value) pairs.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

// -------------------------------------------------------------------
// Internals
// -------------------------------------------------------------------

/// Connect, optionally upgrade to TLS, send GET, read and parse.
fn do_request(url: &Url, tls: Option<&dyn TlsProvider>) -> Result<HttpResponse> {
    let host = &url.host;
    let is_https = url.scheme == "https";
    let default_port = if is_https { 443 } else { 80 };
    let port = url.port.unwrap_or(default_port);

    let stream = tcp_connect(host, port)?;

    if is_https {
        let tls_provider =
            tls.ok_or_else(|| OasisError::Backend("TLS not available".to_string()))?;

        // Wrap the TcpStream as a NetworkStream, then upgrade to TLS.
        let net_stream: Box<dyn NetworkStream> = Box::new(oasis_net::StdNetworkStream::new(stream));
        let tls_stream = tls_provider.connect_tls(net_stream, host)?;

        let mut adapter = NetworkStreamAdapter(tls_stream);
        send_request(&mut adapter, url, is_https)?;
        let raw = read_response(&mut adapter)?;
        parse_response(&raw)
    } else {
        let mut stream = stream;
        send_request(&mut stream, url, is_https)?;
        let raw = read_response(&mut stream)?;
        parse_response(&raw)
    }
}

/// Open a TCP connection with a connect timeout.
fn tcp_connect(host: &str, port: u16) -> Result<TcpStream> {
    use std::net::ToSocketAddrs;

    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| OasisError::Backend(format!("DNS resolution failed: {e}")))?
        .next()
        .ok_or_else(|| OasisError::Backend(format!("no addresses for {host}:{port}")))?;

    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| OasisError::Backend(format!("TCP connect failed: {e}")))?;

    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| OasisError::Backend(format!("set read timeout: {e}")))?;

    Ok(stream)
}

/// Send an HTTP/1.1 GET request.
fn send_request(stream: &mut impl Write, url: &Url, is_https: bool) -> Result<()> {
    let default_port: u16 = if is_https { 443 } else { 80 };
    let host_header = match url.port {
        Some(p) if p != default_port => format!("{}:{}", url.host, p),
        _ => url.host.clone(),
    };

    let path = if let Some(ref q) = url.query {
        format!("{}?{}", url.path, q)
    } else {
        url.path.clone()
    };

    let request = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         User-Agent: OASIS/1.0\r\n\
         Accept: */*\r\n\
         Connection: close\r\n\
         \r\n"
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|e| OasisError::Backend(format!("send request: {e}")))?;

    Ok(())
}

/// Read the entire response until EOF or until the read timeout fires.
fn read_response(stream: &mut impl Read) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > MAX_BODY_SIZE + 4096 {
                    return Err(OasisError::Backend("response too large".to_string()));
                }
                buf.extend_from_slice(&chunk[..n]);
            },
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                break;
            },
            Err(e) => {
                return Err(OasisError::Backend(format!("read response: {e}")));
            },
        }
    }
    Ok(buf)
}

/// Parse raw bytes into status code, headers, and body.
pub fn parse_response(data: &[u8]) -> Result<HttpResponse> {
    // Find the header/body boundary (\r\n\r\n).
    let header_end = find_subsequence(data, b"\r\n\r\n").ok_or_else(|| {
        OasisError::Backend("malformed HTTP response: no header terminator".to_string())
    })?;

    let header_bytes = &data[..header_end];
    let body_start = header_end + 4;

    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|_| OasisError::Backend("non-UTF-8 headers".to_string()))?;

    let mut lines = header_str.split("\r\n");

    // Status line: "HTTP/1.x STATUS REASON"
    let status_line = lines
        .next()
        .ok_or_else(|| OasisError::Backend("empty response".to_string()))?;
    let status_code = parse_status_line(status_line)?;

    // Parse headers.
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }

    // Decode body.
    let raw_body = &data[body_start..];
    let body = if find_header(&headers, "transfer-encoding").is_some_and(|v| v.contains("chunked"))
    {
        decode_chunked(raw_body)?
    } else if let Some(cl) = find_header(&headers, "content-length") {
        let len: usize = cl
            .parse()
            .map_err(|_| OasisError::Backend("bad Content-Length".to_string()))?;
        if len > MAX_BODY_SIZE {
            return Err(OasisError::Backend(
                "response body exceeds 8 MB limit".to_string(),
            ));
        }
        raw_body[..raw_body.len().min(len)].to_vec()
    } else {
        raw_body.to_vec()
    };

    if body.len() > MAX_BODY_SIZE {
        return Err(OasisError::Backend(
            "response body exceeds 8 MB limit".to_string(),
        ));
    }

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

/// Parse the HTTP status code from the status line.
fn parse_status_line(line: &str) -> Result<u16> {
    // Expected: "HTTP/1.x NNN ..."
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(OasisError::Backend(format!("bad status line: {line}")));
    }
    parts[1]
        .parse()
        .map_err(|_| OasisError::Backend(format!("bad status code in: {line}")))
}

/// Case-insensitive header lookup.
fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let name_lower = name.to_lowercase();
    headers
        .iter()
        .find(|(k, _)| k == &name_lower)
        .map(|(_, v)| v.as_str())
}

/// Decode a chunked transfer-encoded body.
fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    while let Some(i) = find_subsequence(&data[pos..], b"\r\n") {
        let line_end = pos + i;

        let size_str = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| OasisError::Backend("bad chunk size".to_string()))?
            .trim();

        // Strip optional chunk extensions (after `;`).
        let size_str = size_str.split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|_| OasisError::Backend("bad chunk size".to_string()))?;

        if chunk_size == 0 {
            break;
        }

        let chunk_start = line_end + 2;
        let chunk_end = chunk_start + chunk_size;

        if chunk_end > data.len() {
            // Partial chunk -- take what we have.
            result.extend_from_slice(&data[chunk_start..]);
            break;
        }

        if result.len() + chunk_size > MAX_BODY_SIZE {
            return Err(OasisError::Backend(
                "chunked body exceeds 8 MB limit".to_string(),
            ));
        }

        result.extend_from_slice(&data[chunk_start..chunk_end]);
        // Skip past chunk data and trailing \r\n.
        pos = chunk_end + 2;
    }

    Ok(result)
}

/// Whether a status code is a redirect we should follow.
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 307 | 308)
}

/// Find the position of a byte subsequence in a slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// -------------------------------------------------------------------
// NetworkStream → Read + Write adapter
// -------------------------------------------------------------------

/// Adapts a `Box<dyn NetworkStream>` to `std::io::Read` + `std::io::Write`
/// so it can be used with the generic `send_request` / `read_response`.
pub(super) struct NetworkStreamAdapter(pub(super) Box<dyn NetworkStream>);

impl Read for NetworkStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(oasis_err_to_io)
    }
}

impl Write for NetworkStreamAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf).map_err(oasis_err_to_io)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Convert an [`OasisError`] to [`io::Error`], preserving the original
/// `io::Error` (and its error kind) when the variant is `OasisError::Io`.
fn oasis_err_to_io(e: OasisError) -> io::Error {
    match e {
        OasisError::Io(io_err) => io_err,
        other => io::Error::other(other.to_string()),
    }
}

// -------------------------------------------------------------------
// Error pages
// -------------------------------------------------------------------

/// Generate a user-friendly error page when a site requires HTTPS.
fn https_error_page(original_url: &Url, https_url: &Url) -> ResourceResponse {
    let html = format!(
        "<html><body>\
         <h1>HTTPS Required</h1>\
         <p>This site redirected to a secure (HTTPS) connection:</p>\
         <p>{https_url}</p>\
         <p>OASIS browser only supports plain HTTP. \
         TLS/SSL is not available.</p>\
         <p>Try a site that serves plain HTTP, such as:</p>\
         <p>http://example.com</p>\
         </body></html>"
    );
    ResourceResponse {
        url: original_url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 200,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_response() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html\r\n\
                     Content-Length: 13\r\n\
                     \r\n\
                     <html>hi</html>";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(
            find_header(&resp.headers, "content-type"),
            Some("text/html"),
        );
        // Body is trimmed to Content-Length (13 bytes).
        assert_eq!(resp.body, b"<html>hi</htm");
    }

    #[test]
    fn parse_response_no_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Content-Type: text/plain\r\n\
                     \r\n\
                     hello world";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, b"hello world");
    }

    #[test]
    fn parse_404_response() {
        let raw = b"HTTP/1.1 404 Not Found\r\n\
                     Content-Length: 9\r\n\
                     \r\n\
                     not found";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 404);
        assert_eq!(resp.body, b"not found");
    }

    #[test]
    fn parse_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\n\
                     Transfer-Encoding: chunked\r\n\
                     \r\n\
                     5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.body, b"hello world");
    }

    #[test]
    fn decode_chunked_basic() {
        let data = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"hello world");
    }

    #[test]
    fn decode_chunked_with_extension() {
        let data = b"5;ext=val\r\nhello\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn https_returns_error_page_without_tls() {
        let url = Url::parse("https://example.com/page").unwrap();
        let resp = http_get(&url, None).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("HTTPS Required"));
    }

    #[test]
    fn unsupported_scheme_rejected() {
        let url = Url::parse("ftp://example.com/file").unwrap();
        let err = http_get(&url, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unsupported scheme"));
    }

    #[test]
    fn redirect_location_detected() {
        let raw = b"HTTP/1.1 301 Moved\r\n\
                     Location: /new-page\r\n\
                     \r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status_code, 301);
        assert!(is_redirect(resp.status_code));
        assert_eq!(find_header(&resp.headers, "location"), Some("/new-page"),);
    }

    #[test]
    fn case_insensitive_header_lookup() {
        let headers = vec![
            ("content-type".to_string(), "text/html".to_string()),
            ("x-custom".to_string(), "value".to_string()),
        ];
        assert_eq!(find_header(&headers, "Content-Type"), Some("text/html"));
        assert_eq!(find_header(&headers, "CONTENT-TYPE"), Some("text/html"));
        assert_eq!(find_header(&headers, "X-Custom"), Some("value"));
        assert_eq!(find_header(&headers, "missing"), None);
    }

    #[test]
    fn max_body_enforced_content_length() {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_SIZE + 1,
        );
        let err = parse_response(header.as_bytes()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("8 MB"));
    }

    #[test]
    fn is_redirect_codes() {
        assert!(is_redirect(301));
        assert!(is_redirect(302));
        assert!(is_redirect(307));
        assert!(is_redirect(308));
        assert!(!is_redirect(200));
        assert!(!is_redirect(404));
        assert!(!is_redirect(500));
    }

    #[test]
    fn parse_status_line_ok() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK").unwrap(), 200);
        assert_eq!(
            parse_status_line("HTTP/1.0 301 Moved Permanently").unwrap(),
            301,
        );
    }

    #[test]
    fn parse_status_line_bad() {
        assert!(parse_status_line("garbage").is_err());
    }

    #[test]
    fn find_subsequence_works() {
        assert_eq!(
            find_subsequence(b"hello\r\n\r\nworld", b"\r\n\r\n"),
            Some(5)
        );
        assert_eq!(find_subsequence(b"no boundary", b"\r\n\r\n"), None);
    }

    #[test]
    fn http_to_https_redirect_without_tls() {
        use std::io::Write as IoWrite;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 301 Moved\r\n\
                 Location: https://127.0.0.1:{port}/secure\r\n\
                 Content-Length: 0\r\n\
                 \r\n"
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        });

        let url = Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();
        // No TLS provider -- redirect to HTTPS should produce error page.
        let resp = http_get(&url, None).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(
            body.contains("HTTPS Required"),
            "expected HTTPS Required page, got: {body}",
        );
        let _ = handle.join();
    }
}
