//! Gemini protocol network client.
//!
//! Gemini mandates TLS on every connection (default port 1965).
//! The request is a single URL terminated by CRLF; the response
//! starts with a status line followed by an optional body.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::browser::gemini;
use crate::error::{OasisError, Result};
use crate::net::tls::TlsProvider;

use super::{ContentType, ResourceResponse, Url};

/// Maximum Gemini response size (2 MB).
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Maximum number of redirects to follow.
const MAX_REDIRECTS: u8 = 5;

/// TCP connect timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Read timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetch a Gemini resource over TLS.
///
/// Returns an error page if no TLS provider is available (Gemini
/// requires TLS for every connection).
pub fn gemini_get(url: &Url, tls: Option<&dyn TlsProvider>) -> Result<ResourceResponse> {
    let tls = match tls {
        Some(t) => t,
        None => return Ok(tls_required_page(url)),
    };

    let mut current_url = url.clone();
    for _ in 0..MAX_REDIRECTS {
        let resp = do_gemini_request(&current_url, tls)?;

        if resp.status.is_redirect() {
            let target = &resp.meta;
            current_url = current_url
                .resolve(target)
                .ok_or_else(|| OasisError::Backend(format!("bad Gemini redirect: {target}")))?;
            continue;
        }

        if !resp.status.is_success() {
            let html = format!(
                "<html><body>\
                 <h1>Gemini Error</h1>\
                 <p>Status: {:?}</p>\
                 <p>{}</p>\
                 </body></html>",
                resp.status, resp.meta,
            );
            return Ok(ResourceResponse {
                url: current_url.to_string(),
                content_type: ContentType::Html,
                body: html.into_bytes(),
                status: 200,
            });
        }

        // Success -- determine content type from the meta line.
        let content_type = if resp.meta.starts_with("text/gemini") {
            ContentType::GeminiText
        } else if resp.meta.starts_with("text/html") {
            ContentType::Html
        } else if resp.meta.starts_with("text/") {
            ContentType::PlainText
        } else {
            ContentType::Html // fallback
        };

        let body = resp.body.unwrap_or_default();

        return Ok(ResourceResponse {
            url: current_url.to_string(),
            content_type,
            body: body.into_bytes(),
            status: 200,
        });
    }

    Err(OasisError::Backend("too many Gemini redirects".to_string()))
}

/// Perform a single Gemini request over TLS.
fn do_gemini_request(url: &Url, tls: &dyn TlsProvider) -> Result<gemini::GeminiResponse> {
    let host = &url.host;
    let port = url.port.unwrap_or(1965);

    // Connect TCP.
    let stream = tcp_connect(host, port)?;

    // Wrap in TLS.
    let net_stream: Box<dyn crate::backend::NetworkStream> =
        Box::new(crate::net::StdNetworkStream::new(stream));
    let tls_stream = tls.connect_tls(net_stream, host)?;

    let mut adapter = super::http::NetworkStreamAdapter(tls_stream);

    // Send Gemini request: full URL + CRLF.
    let request = gemini::build_request(&url.to_string());
    adapter
        .write_all(&request)
        .map_err(|e| OasisError::Backend(format!("Gemini send: {e}")))?;

    // Read response.
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        match adapter.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > MAX_BODY_SIZE {
                    return Err(OasisError::Backend("Gemini response too large".to_string()));
                }
                buf.extend_from_slice(&chunk[..n]);
            },
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            },
            Err(e) => {
                return Err(OasisError::Backend(format!("Gemini read: {e}")));
            },
        }
    }

    gemini::parse_response(&buf)
        .ok_or_else(|| OasisError::Backend("malformed Gemini response".to_string()))
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

/// Error page when TLS is not available (Gemini requires it).
fn tls_required_page(url: &Url) -> ResourceResponse {
    let html = format!(
        "<html><body>\
         <h1>TLS Required</h1>\
         <p>Gemini protocol requires TLS, which is not available.</p>\
         <p>Requested: {url}</p>\
         </body></html>"
    );
    ResourceResponse {
        url: url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 200,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NetworkStream;
    use std::io::Write as IoWrite;
    use std::net::TcpListener;

    /// A TLS provider that passes the stream through unchanged.
    /// Used to test Gemini protocol over plain TCP.
    struct PassthroughTlsProvider;

    impl TlsProvider for PassthroughTlsProvider {
        fn connect_tls(
            &self,
            stream: Box<dyn NetworkStream>,
            _server_name: &str,
        ) -> crate::error::Result<Box<dyn NetworkStream>> {
            Ok(stream)
        }
    }

    /// Spawn a local TCP server that accepts one connection, reads
    /// the Gemini request, and sends the given raw response bytes.
    fn spawn_gemini_server(response: Vec<u8>) -> (std::thread::JoinHandle<()>, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read the request (URL + CRLF).
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            // Send the response.
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });

        (handle, port)
    }

    #[test]
    fn test_gemini_without_tls_returns_tls_required() {
        let url = Url::parse("gemini://example.com/page").unwrap();
        let resp = gemini_get(&url, None).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("TLS Required"));
        assert!(body.contains("example.com"));
        assert_eq!(resp.content_type, ContentType::Html);
    }

    #[test]
    fn test_gemini_success_response() {
        let (handle, port) = spawn_gemini_server(b"20 text/gemini\r\n# Hello\nWelcome!".to_vec());
        let url = Url::parse(&format!("gemini://localhost:{port}/")).unwrap();
        let provider = PassthroughTlsProvider;
        let resp = gemini_get(&url, Some(&provider)).unwrap();
        assert_eq!(resp.content_type, ContentType::GeminiText);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Hello"));
        assert!(body.contains("Welcome!"));
        let _ = handle.join();
    }

    #[test]
    fn test_gemini_redirect_following() {
        // Need to know port before building responses, so bind first.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let responses = vec![
            format!("30 gemini://localhost:{port}/target\r\n").into_bytes(),
            b"20 text/gemini\r\nRedirected!".to_vec(),
        ];
        let handle = std::thread::spawn(move || {
            for resp in &responses {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(resp);
                    let _ = stream.flush();
                }
            }
        });
        let url = Url::parse(&format!("gemini://localhost:{port}/start")).unwrap();
        let provider = PassthroughTlsProvider;
        let resp = gemini_get(&url, Some(&provider)).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Redirected!"));
        let _ = handle.join();
    }

    #[test]
    fn test_gemini_max_redirects_exceeded() {
        // Server always redirects.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Server accepts up to MAX_REDIRECTS connections, then stops.
        // We don't join the thread because the client errors out after
        // MAX_REDIRECTS and the server may be blocked on accept().
        std::thread::spawn(move || {
            for _ in 0..MAX_REDIRECTS + 1 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let resp = format!("30 gemini://localhost:{port}/loop\r\n");
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
            }
        });
        let url = Url::parse(&format!("gemini://localhost:{port}/start")).unwrap();
        let provider = PassthroughTlsProvider;
        let result = gemini_get(&url, Some(&provider));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too many"));
    }

    #[test]
    fn test_gemini_error_status() {
        let (handle, port) = spawn_gemini_server(b"51 Not Found\r\n".to_vec());
        let url = Url::parse(&format!("gemini://localhost:{port}/missing")).unwrap();
        let provider = PassthroughTlsProvider;
        let resp = gemini_get(&url, Some(&provider)).unwrap();
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Gemini Error"));
        assert!(body.contains("Not Found"));
        assert_eq!(resp.content_type, ContentType::Html);
        let _ = handle.join();
    }

    #[test]
    fn test_gemini_content_type_detection() {
        // text/gemini -> GeminiText
        let (h1, p1) = spawn_gemini_server(b"20 text/gemini\r\n# Test".to_vec());
        let url1 = Url::parse(&format!("gemini://localhost:{p1}/")).unwrap();
        let provider = PassthroughTlsProvider;
        let r1 = gemini_get(&url1, Some(&provider)).unwrap();
        assert_eq!(r1.content_type, ContentType::GeminiText);
        let _ = h1.join();

        // text/html -> Html
        let (h2, p2) = spawn_gemini_server(b"20 text/html\r\n<html>hi</html>".to_vec());
        let url2 = Url::parse(&format!("gemini://localhost:{p2}/")).unwrap();
        let r2 = gemini_get(&url2, Some(&provider)).unwrap();
        assert_eq!(r2.content_type, ContentType::Html);
        let _ = h2.join();

        // text/plain -> PlainText
        let (h3, p3) = spawn_gemini_server(b"20 text/plain\r\nhello".to_vec());
        let url3 = Url::parse(&format!("gemini://localhost:{p3}/")).unwrap();
        let r3 = gemini_get(&url3, Some(&provider)).unwrap();
        assert_eq!(r3.content_type, ContentType::PlainText);
        let _ = h3.join();
    }
}
