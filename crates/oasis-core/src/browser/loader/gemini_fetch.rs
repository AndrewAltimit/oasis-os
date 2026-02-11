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
