//! VFS resource loader.
//!
//! Maps URLs to VFS paths and loads resources from the virtual file
//! system. This is the primary loader in sandbox mode.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

use super::{ContentType, ResourceRequest, ResourceResponse, Url};

/// Load a resource from the VFS.
pub fn load_from_vfs(vfs: &dyn Vfs, request: &ResourceRequest) -> Result<ResourceResponse> {
    let url = Url::parse(&request.url)
        .ok_or_else(|| OasisError::Vfs(format!("invalid URL: {}", request.url).into()))?;

    let vfs_path = url_to_vfs_path(&url)?;

    // Security: reject path traversal attempts.
    validate_path(&vfs_path)?;

    let body = vfs.read(&vfs_path)?;
    let content_type = super::detect_content_type(&url);

    Ok(ResourceResponse {
        url: request.url.clone(),
        content_type,
        body,
        status: 200,
    })
}

/// Map a URL to a VFS path.
///
/// Rules:
/// - `vfs://path` -> `/path`
/// - `http://host/path` -> `/sites/host/path`
/// - `https://host/path` -> `/sites/host/path`
fn url_to_vfs_path(url: &Url) -> Result<String> {
    match url.scheme.as_str() {
        "vfs" => Ok(format!("/{}{}", url.host, url.path)),
        "http" | "https" => {
            let mut path = format!("/sites/{}{}", url.host, url.path);
            // If path ends with `/` or the last segment has no
            // extension, append `index.html`.
            if path.ends_with('/') {
                path.push_str("index.html");
            } else {
                let last = path.rsplit('/').next().unwrap_or("");
                if !last.contains('.') {
                    path.push_str("/index.html");
                }
            }
            Ok(path)
        },
        _ => Err(OasisError::Vfs(
            format!("unsupported scheme for VFS: {}", url.scheme).into(),
        )),
    }
}

/// Validate that a VFS path does not escape via `..` traversal.
fn validate_path(path: &str) -> Result<()> {
    // Reject any path that contains a ".." segment. Checking the raw
    // path (before normalisation) catches attempts like
    // `/sites/../../etc/passwd` that would resolve away after
    // collapsing.
    if path.split('/').any(|seg| seg == "..") {
        return Err(OasisError::Vfs("path traversal not allowed".into()));
    }
    Ok(())
}

/// HTML-escape a string to prevent injection when interpolating into HTML.
pub(super) fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Generate a "page not found" HTML response.
pub fn not_found_page(url: &str) -> ResourceResponse {
    let escaped_url = escape_html(url);
    let html = format!(
        "<html><body><h1>Page Not Found</h1>\
         <p>The page <code>{escaped_url}</code> could not be found.</p>\
         </body></html>"
    );
    ResourceResponse {
        url: url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 404,
    }
}

/// Generate an error page HTML response with categorized error UX.
///
/// Detects the error category from the message (DNS failure, connection
/// timeout, TLS error, HTTP error) and produces a styled page with an
/// explanation and suggested actions.
pub fn error_page(url: &str, message: &str) -> ResourceResponse {
    let msg_lower = message.to_ascii_lowercase();
    let escaped_url = escape_html(url);
    let escaped_message = escape_html(message);
    let (title, explanation, suggestions) = if msg_lower.contains("dns")
        || msg_lower.contains("resolve")
        || msg_lower.contains("no addresses")
        || msg_lower.contains("name or service not known")
    {
        (
            "DNS Lookup Failed",
            "The browser could not find the server's address. The domain \
             name could not be resolved to an IP address.",
            "<li>Check that the URL is spelled correctly.</li>\
             <li>Verify your network connection is active.</li>\
             <li>The site may not exist or its DNS records may be missing.</li>",
        )
    } else if msg_lower.contains("timed out")
        || msg_lower.contains("timeout")
        || msg_lower.contains("connect failed")
        || msg_lower.contains("connection refused")
    {
        (
            "Connection Failed",
            "The browser could not establish a connection to the server. \
             The server may be down, unreachable, or rejecting connections.",
            "<li>Check your network connection.</li>\
             <li>The server may be temporarily unavailable — try again later.</li>\
             <li>A firewall may be blocking the connection.</li>",
        )
    } else if msg_lower.contains("tls")
        || msg_lower.contains("ssl")
        || msg_lower.contains("certificate")
        || msg_lower.contains("handshake")
    {
        (
            "Secure Connection Failed",
            "The browser could not establish a secure (TLS/SSL) connection \
             to the server.",
            "<li>The server's certificate may be invalid or expired.</li>\
             <li>Try accessing the site over plain HTTP if available.</li>\
             <li>The server may require a TLS version that is not supported.</li>",
        )
    } else if msg_lower.contains("too many redirects") {
        (
            "Too Many Redirects",
            "The page redirected too many times. This usually means the \
             server is misconfigured and has created a redirect loop.",
            "<li>Try clearing cookies for this site.</li>\
             <li>The site may be misconfigured — contact the site owner.</li>",
        )
    } else {
        (
            "Page Load Error",
            "The browser encountered an error while loading the page.",
            "<li>Check that the URL is correct.</li>\
             <li>Verify your network connection.</li>\
             <li>Try again later.</li>",
        )
    };

    let html = format!(
        "<html><head><style>\
         body {{ font-family: sans-serif; margin: 40px; color: #333; \
                background: #f8f8f8; }}\
         .error-box {{ background: white; border: 1px solid #ddd; \
                       border-radius: 8px; padding: 24px; max-width: 480px; }}\
         h1 {{ color: #c33; font-size: 18px; margin: 0 0 12px 0; }}\
         p {{ font-size: 13px; line-height: 1.5; margin: 8px 0; }}\
         ul {{ font-size: 13px; line-height: 1.6; padding-left: 20px; }}\
         code {{ background: #eee; padding: 2px 4px; border-radius: 3px; \
                 font-size: 12px; word-break: break-all; }}\
         .detail {{ color: #888; font-size: 11px; margin-top: 16px; \
                    border-top: 1px solid #eee; padding-top: 12px; }}\
         </style></head>\
         <body><div class=\"error-box\">\
         <h1>{title}</h1>\
         <p>{explanation}</p>\
         <p><strong>Try:</strong></p>\
         <ul>{suggestions}</ul>\
         <div class=\"detail\">\
         <p>URL: <code>{escaped_url}</code></p>\
         <p>Details: {escaped_message}</p>\
         </div></div></body></html>"
    );
    ResourceResponse {
        url: url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 500,
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{HttpMethod, ResourceSource};
    use oasis_vfs::MemoryVfs;

    /// Normalise a path by resolving `.` and `..` segments (test helper).
    fn normalize_path(path: &str) -> String {
        let mut segments: Vec<&str> = Vec::new();
        for seg in path.split('/') {
            match seg {
                "" | "." => {},
                ".." => {
                    segments.pop();
                },
                s => segments.push(s),
            }
        }
        format!("/{}", segments.join("/"))
    }

    /// Helper: create a VFS with a simple site tree.
    fn test_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/example.com").unwrap();
        vfs.write(
            "/sites/example.com/index.html",
            b"<html><body>Hello</body></html>",
        )
        .unwrap();
        vfs.mkdir("/pages").unwrap();
        vfs.write("/pages/about.html", b"<html><body>About</body></html>")
            .unwrap();
        vfs
    }

    #[test]
    fn load_html_from_vfs() {
        let vfs = test_vfs();
        let req = ResourceRequest {
            url: "http://example.com/index.html".to_string(),
            base_url: None,
            source: ResourceSource::Vfs,
            method: HttpMethod::Get,
            body: None,
            referrer: None,
        };
        let resp = load_from_vfs(&vfs, &req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, ContentType::Html);
        assert_eq!(resp.body, b"<html><body>Hello</body></html>");
    }

    #[test]
    fn url_to_vfs_path_http() {
        let url = Url::parse("http://example.com/page.html").unwrap();
        let path = url_to_vfs_path(&url).unwrap();
        assert_eq!(path, "/sites/example.com/page.html");
    }

    #[test]
    fn url_to_vfs_path_vfs_scheme() {
        let url = Url::parse("vfs://pages/about.html").unwrap();
        let path = url_to_vfs_path(&url).unwrap();
        assert_eq!(path, "/pages/about.html");
    }

    #[test]
    fn path_traversal_rejected() {
        let result = validate_path("/sites/../../etc/passwd");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("path traversal"));
    }

    #[test]
    fn auto_append_index_html_for_directory_url() {
        let url = Url::parse("http://example.com/").unwrap();
        let path = url_to_vfs_path(&url).unwrap();
        assert_eq!(path, "/sites/example.com/index.html");
    }

    #[test]
    fn auto_append_index_html_no_extension() {
        let url = Url::parse("http://example.com/docs").unwrap();
        let path = url_to_vfs_path(&url).unwrap();
        assert_eq!(path, "/sites/example.com/docs/index.html");
    }

    #[test]
    fn not_found_page_generation() {
        let resp = not_found_page("http://missing.example/x");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.content_type, ContentType::Html);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Page Not Found"));
        assert!(body.contains("http://missing.example/x"));
    }

    #[test]
    fn error_page_generation() {
        let resp = error_page("http://err.example/y", "timeout");
        assert_eq!(resp.status, 500);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("timeout"));
    }

    #[test]
    fn error_page_dns_failure_categorized() {
        let resp = error_page("http://bad.example", "DNS resolution failed: not found");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("DNS Lookup Failed"));
        assert!(body.contains("domain name"));
    }

    #[test]
    fn error_page_timeout_categorized() {
        let resp = error_page("http://slow.example", "TCP connect failed: timed out");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Connection Failed"));
        assert!(body.contains("connection"));
    }

    #[test]
    fn error_page_tls_categorized() {
        let resp = error_page("https://secure.example", "TLS handshake failed");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Secure Connection Failed"));
    }

    #[test]
    fn error_page_redirect_loop_categorized() {
        let resp = error_page("http://loop.example", "too many redirects");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Too Many Redirects"));
    }

    #[test]
    fn error_page_generic_fallback() {
        let resp = error_page("http://x.example", "some unknown error");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Page Load Error"));
    }

    #[test]
    fn error_page_includes_url_and_details() {
        let resp = error_page("http://test.example/path", "connection refused");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("http://test.example/path"));
        assert!(body.contains("connection refused"));
    }

    #[test]
    fn unsupported_scheme_rejected() {
        let url = Url::parse("ftp://example.com/file").unwrap();
        let result = url_to_vfs_path(&url);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unsupported scheme"));
    }

    #[test]
    fn normalize_collapses_dotdot() {
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
    }

    #[test]
    fn normalize_collapses_dot() {
        assert_eq!(normalize_path("/a/./b"), "/a/b");
    }

    #[test]
    fn vfs_load_missing_file_returns_error() {
        let vfs = test_vfs();
        let req = ResourceRequest {
            url: "http://example.com/missing.html".to_string(),
            base_url: None,
            source: ResourceSource::Vfs,
            method: HttpMethod::Get,
            body: None,
            referrer: None,
        };
        assert!(load_from_vfs(&vfs, &req).is_err());
    }

    #[test]
    fn vfs_load_with_vfs_scheme() {
        let vfs = test_vfs();
        let req = ResourceRequest {
            url: "vfs://pages/about.html".to_string(),
            base_url: None,
            source: ResourceSource::Vfs,
            method: HttpMethod::Get,
            body: None,
            referrer: None,
        };
        let resp = load_from_vfs(&vfs, &req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"<html><body>About</body></html>");
    }
}
