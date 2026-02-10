//! VFS resource loader.
//!
//! Maps URLs to VFS paths and loads resources from the virtual file
//! system. This is the primary loader in sandbox mode.

use crate::error::{OasisError, Result};
use crate::vfs::Vfs;

use super::{ContentType, ResourceRequest, ResourceResponse, Url};

/// Load a resource from the VFS.
pub fn load_from_vfs(vfs: &dyn Vfs, request: &ResourceRequest) -> Result<ResourceResponse> {
    let url = Url::parse(&request.url)
        .ok_or_else(|| OasisError::Vfs(format!("invalid URL: {}", request.url)))?;

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
        _ => Err(OasisError::Vfs(format!(
            "unsupported scheme for VFS: {}",
            url.scheme
        ))),
    }
}

/// Validate that a VFS path does not escape via `..` traversal.
fn validate_path(path: &str) -> Result<()> {
    // Reject any path that contains a ".." segment. Checking the raw
    // path (before normalisation) catches attempts like
    // `/sites/../../etc/passwd` that would resolve away after
    // collapsing.
    if path.split('/').any(|seg| seg == "..") {
        return Err(OasisError::Vfs("path traversal not allowed".to_string()));
    }
    Ok(())
}

/// Generate a "page not found" HTML response.
pub fn not_found_page(url: &str) -> ResourceResponse {
    let html = format!(
        "<html><body><h1>Page Not Found</h1>\
         <p>The page <code>{url}</code> could not be found.</p>\
         </body></html>"
    );
    ResourceResponse {
        url: url.to_string(),
        content_type: ContentType::Html,
        body: html.into_bytes(),
        status: 404,
    }
}

/// Generate an error page HTML response.
pub fn error_page(url: &str, message: &str) -> ResourceResponse {
    let html = format!(
        "<html><body><h1>Error</h1>\
         <p>{message}</p>\
         <p>URL: <code>{url}</code></p>\
         </body></html>"
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
    use crate::browser::loader::ResourceSource;
    use crate::vfs::MemoryVfs;

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
        };
        let resp = load_from_vfs(&vfs, &req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"<html><body>About</body></html>");
    }
}
