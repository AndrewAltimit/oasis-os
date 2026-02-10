//! Resource loading: URL parsing, content type detection, and loader
//! orchestration.

pub mod cache;
pub mod http;
pub mod vfs;

use std::fmt;

use crate::error::Result;

/// How to resolve resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceSource {
    /// Fetch over the network (live mode).
    Network,
    /// Resolve against the VFS (sandbox mode).
    Vfs,
    /// Try VFS first, fall back to network.
    VfsThenNetwork,
}

/// A request for a resource.
#[derive(Debug, Clone)]
pub struct ResourceRequest {
    pub url: String,
    pub base_url: Option<String>,
    pub source: ResourceSource,
}

/// A loaded resource.
#[derive(Debug, Clone)]
pub struct ResourceResponse {
    pub url: String,
    pub content_type: ContentType,
    pub body: Vec<u8>,
    pub status: u16,
}

/// Content types the browser can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Html,
    Css,
    Jpeg,
    Png,
    Bmp,
    Gif,
    GeminiText,
    PlainText,
    Unknown,
}

impl ContentType {
    /// Detect content type from a file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "html" | "htm" => ContentType::Html,
            "css" => ContentType::Css,
            "jpg" | "jpeg" => ContentType::Jpeg,
            "png" => ContentType::Png,
            "bmp" => ContentType::Bmp,
            "gif" => ContentType::Gif,
            "gmi" | "gemini" => ContentType::GeminiText,
            "txt" => ContentType::PlainText,
            _ => ContentType::Unknown,
        }
    }

    /// Detect content type from a MIME type string.
    pub fn from_mime(mime: &str) -> Self {
        let mime = mime.split(';').next().unwrap_or("").trim();
        match mime {
            "text/html" => ContentType::Html,
            "text/css" => ContentType::Css,
            "image/jpeg" => ContentType::Jpeg,
            "image/png" => ContentType::Png,
            "image/bmp" => ContentType::Bmp,
            "image/gif" => ContentType::Gif,
            "text/gemini" => ContentType::GeminiText,
            "text/plain" => ContentType::PlainText,
            _ => ContentType::Unknown,
        }
    }

    /// Is this an image content type?
    pub fn is_image(&self) -> bool {
        matches!(
            self,
            ContentType::Jpeg | ContentType::Png | ContentType::Bmp | ContentType::Gif
        )
    }
}

// ---------------------------------------------------------------------------
// URL parsing and resolution (simplified RFC 3986)
// ---------------------------------------------------------------------------

/// A parsed URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    /// Scheme component (e.g. `"http"`, `"vfs"`, `"gemini"`).
    pub scheme: String,
    /// Host component (e.g. `"example.com"`). For `vfs://` URLs this is
    /// the first path segment.
    pub host: String,
    /// Optional explicit port number.
    pub port: Option<u16>,
    /// Path component starting with `/`.
    pub path: String,
    /// Optional query string (without the leading `?`).
    pub query: Option<String>,
    /// Optional fragment (without the leading `#`).
    pub fragment: Option<String>,
}

impl Url {
    /// Parse a URL string.
    ///
    /// Handles full URLs (`http://host/path`), VFS URLs
    /// (`vfs://sites/corp/index.html`), Gemini URLs
    /// (`gemini://host/path`), fragment-only (`#section`), and
    /// protocol-relative (`//host/path`).
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }

        // Fragment-only reference.
        if let Some(frag) = url.strip_prefix('#') {
            return Some(Url {
                scheme: String::new(),
                host: String::new(),
                port: None,
                path: String::new(),
                query: None,
                fragment: Some(frag.to_string()),
            });
        }

        // Protocol-relative URL: //host/path
        if let Some(rest) = url.strip_prefix("//") {
            return Self::parse_authority_and_path("", rest);
        }

        // Full URL with scheme.
        if let Some(idx) = url.find("://") {
            let scheme = &url[..idx];
            let rest = &url[idx + 3..];
            return Self::parse_authority_and_path(scheme, rest);
        }

        None
    }

    /// Internal helper: parse `host[:port]/path?query#fragment` after
    /// the scheme has been stripped.
    fn parse_authority_and_path(scheme: &str, rest: &str) -> Option<Url> {
        // Split off fragment first.
        let (rest, fragment) = match rest.find('#') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };

        // Split off query.
        let (rest, query) = match rest.find('?') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };

        // Split authority from path.
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };

        // Parse host and optional port from authority.
        let (host, port) = match authority.rfind(':') {
            Some(i) => {
                let maybe_port = &authority[i + 1..];
                if let Ok(p) = maybe_port.parse::<u16>() {
                    (&authority[..i], Some(p))
                } else {
                    (authority, None)
                }
            },
            None => (authority, None),
        };

        let path = if path.is_empty() { "/" } else { path };

        Some(Url {
            scheme: scheme.to_lowercase(),
            host: host.to_string(),
            port,
            path: path.to_string(),
            query,
            fragment,
        })
    }

    /// Resolve a relative reference against this base URL.
    ///
    /// Handles absolute URLs (returned as-is), protocol-relative
    /// (`//host/path`), absolute paths (`/path`), relative paths
    /// (`path`, `../path`), query-only (`?q=x`), and fragment-only
    /// (`#frag`) references.
    pub fn resolve(&self, relative: &str) -> Option<Url> {
        let relative = relative.trim();
        if relative.is_empty() {
            return Some(self.clone());
        }

        // Absolute URL (has scheme) -- return as-is.
        if relative.contains("://") {
            return Url::parse(relative);
        }

        // Protocol-relative.
        if relative.starts_with("//") {
            return Url::parse(&format!("{}:{}", self.scheme, relative));
        }

        // Fragment-only.
        if let Some(frag) = relative.strip_prefix('#') {
            let mut resolved = self.clone();
            resolved.fragment = Some(frag.to_string());
            return Some(resolved);
        }

        // Query-only.
        if let Some(query) = relative.strip_prefix('?') {
            let mut resolved = self.clone();
            resolved.query = Some(query.to_string());
            resolved.fragment = None;
            return Some(resolved);
        }

        // Absolute path.
        if relative.starts_with('/') {
            // Split off query and fragment.
            let (path, query, fragment) = split_path_query_fragment(relative);
            return Some(Url {
                scheme: self.scheme.clone(),
                host: self.host.clone(),
                port: self.port,
                path,
                query,
                fragment,
            });
        }

        // Relative path -- resolve against base directory.
        let base_dir = self.directory();
        let (rel_path, query, fragment) = split_path_query_fragment(relative);
        let resolved_path = resolve_path(base_dir, &rel_path);
        Some(Url {
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.port,
            path: resolved_path,
            query,
            fragment,
        })
    }

    /// Get the file extension from the path (without the dot).
    pub fn extension(&self) -> Option<&str> {
        let path = self.path.split('?').next().unwrap_or(&self.path);
        let filename = path.rsplit('/').next()?;
        let dot_pos = filename.rfind('.')?;
        let ext = &filename[dot_pos + 1..];
        if ext.is_empty() { None } else { Some(ext) }
    }

    /// Get the directory portion of the path (everything up to and
    /// including the last `/`).
    pub fn directory(&self) -> &str {
        match self.path.rfind('/') {
            Some(i) => &self.path[..=i],
            None => "/",
        }
    }

    /// Get the origin (`scheme://host[:port]`).
    pub fn origin(&self) -> String {
        let mut s = format!("{}://{}", self.scheme, self.host);
        if let Some(port) = self.port {
            s.push_str(&format!(":{port}"));
        }
        s
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        write!(f, "{}", self.path)?;
        if let Some(ref q) = self.query {
            write!(f, "?{q}")?;
        }
        if let Some(ref frag) = self.fragment {
            write!(f, "#{frag}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Split a (possibly relative) path string into `(path, query, fragment)`.
fn split_path_query_fragment(s: &str) -> (String, Option<String>, Option<String>) {
    let (s, fragment) = match s.find('#') {
        Some(i) => (&s[..i], Some(s[i + 1..].to_string())),
        None => (s, None),
    };
    let (path, query) = match s.find('?') {
        Some(i) => (s[..i].to_string(), Some(s[i + 1..].to_string())),
        None => (s.to_string(), None),
    };
    (path, query, fragment)
}

/// Resolve a relative path against a base directory, handling `..` and
/// `.` segments.
fn resolve_path(base_dir: &str, relative: &str) -> String {
    let mut segments: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();

    for seg in relative.split('/') {
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

/// Detect the content type for a URL by inspecting its file extension.
/// Defaults to [`ContentType::Html`] when no extension is recognised.
pub fn detect_content_type(url: &Url) -> ContentType {
    url.extension()
        .map(ContentType::from_extension)
        .unwrap_or(ContentType::Html)
}

/// Load a resource according to the request's [`ResourceSource`].
///
/// For `Vfs` requests only the VFS is consulted. For `Network` requests
/// the HTTP client is used directly. For `VfsThenNetwork` it tries the
/// VFS first and falls back to the network.
pub fn load_resource(
    vfs_backend: &dyn crate::vfs::Vfs,
    request: &ResourceRequest,
) -> Result<ResourceResponse> {
    match request.source {
        ResourceSource::Vfs => vfs::load_from_vfs(vfs_backend, request),
        ResourceSource::Network => load_from_network(request),
        ResourceSource::VfsThenNetwork => match vfs::load_from_vfs(vfs_backend, request) {
            Ok(resp) => Ok(resp),
            Err(_) => load_from_network(request),
        },
    }
}

/// Load a resource over the network (HTTP only).
fn load_from_network(request: &ResourceRequest) -> Result<ResourceResponse> {
    let url = Url::parse(&request.url).ok_or_else(|| {
        crate::error::OasisError::Backend(format!("invalid URL: {}", request.url,))
    })?;

    match url.scheme.as_str() {
        "http" => http::http_get(&url),
        "https" => Err(crate::error::OasisError::Backend(
            "HTTPS not supported: TLS not available".to_string(),
        )),
        scheme => Err(crate::error::OasisError::Backend(format!(
            "unsupported network scheme: {scheme}",
        ))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- URL parsing -------------------------------------------------------

    #[test]
    fn parse_full_http_url() {
        let url = Url::parse("http://example.com/page.html").unwrap();
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, None);
        assert_eq!(url.path, "/page.html");
        assert_eq!(url.query, None);
        assert_eq!(url.fragment, None);
    }

    #[test]
    fn parse_vfs_url() {
        let url = Url::parse("vfs://sites/corp/index.html").unwrap();
        assert_eq!(url.scheme, "vfs");
        assert_eq!(url.host, "sites");
        assert_eq!(url.path, "/corp/index.html");
    }

    #[test]
    fn parse_url_with_port() {
        let url = Url::parse("http://localhost:8080/api").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, Some(8080));
        assert_eq!(url.path, "/api");
    }

    #[test]
    fn parse_url_with_query_and_fragment() {
        let url = Url::parse("https://example.com/search?q=test#results").unwrap();
        assert_eq!(url.scheme, "https");
        assert_eq!(url.path, "/search");
        assert_eq!(url.query, Some("q=test".to_string()));
        assert_eq!(url.fragment, Some("results".to_string()));
    }

    #[test]
    fn resolve_relative_url() {
        let base = Url::parse("http://example.com/docs/intro.html").unwrap();
        let resolved = base.resolve("chapter2.html").unwrap();
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.path, "/docs/chapter2.html");
    }

    #[test]
    fn resolve_absolute_path() {
        let base = Url::parse("http://example.com/docs/intro.html").unwrap();
        let resolved = base.resolve("/other/page.html").unwrap();
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.path, "/other/page.html");
    }

    #[test]
    fn resolve_protocol_relative() {
        let base = Url::parse("https://example.com/page.html").unwrap();
        let resolved = base.resolve("//cdn.example.com/style.css").unwrap();
        assert_eq!(resolved.scheme, "https");
        assert_eq!(resolved.host, "cdn.example.com");
        assert_eq!(resolved.path, "/style.css");
    }

    #[test]
    fn resolve_fragment_only() {
        let base = Url::parse("http://example.com/page.html").unwrap();
        let resolved = base.resolve("#section2").unwrap();
        assert_eq!(resolved.path, "/page.html");
        assert_eq!(resolved.fragment, Some("section2".to_string()));
    }

    #[test]
    fn resolve_dotdot_in_relative_paths() {
        let base = Url::parse("http://example.com/a/b/c.html").unwrap();
        let resolved = base.resolve("../../d.html").unwrap();
        assert_eq!(resolved.path, "/d.html");
    }

    #[test]
    fn content_type_from_extension() {
        assert_eq!(ContentType::from_extension("html"), ContentType::Html);
        assert_eq!(ContentType::from_extension("CSS"), ContentType::Css);
        assert_eq!(ContentType::from_extension("jpg"), ContentType::Jpeg);
        assert_eq!(ContentType::from_extension("PNG"), ContentType::Png);
        assert_eq!(ContentType::from_extension("bmp"), ContentType::Bmp);
        assert_eq!(ContentType::from_extension("gif"), ContentType::Gif);
        assert_eq!(ContentType::from_extension("gmi"), ContentType::GeminiText);
        assert_eq!(ContentType::from_extension("txt"), ContentType::PlainText);
        assert_eq!(ContentType::from_extension("xyz"), ContentType::Unknown);
    }

    #[test]
    fn content_type_from_mime() {
        assert_eq!(
            ContentType::from_mime("text/html; charset=utf-8"),
            ContentType::Html
        );
        assert_eq!(ContentType::from_mime("image/png"), ContentType::Png);
        assert_eq!(
            ContentType::from_mime("application/octet-stream"),
            ContentType::Unknown
        );
    }

    // -- Display -----------------------------------------------------------

    #[test]
    fn url_display_round_trip() {
        let url = Url::parse("https://example.com:443/path?q=1#frag").unwrap();
        assert_eq!(url.to_string(), "https://example.com:443/path?q=1#frag");
    }

    // -- helpers -----------------------------------------------------------

    #[test]
    fn url_extension() {
        let url = Url::parse("http://example.com/style.css").unwrap();
        assert_eq!(url.extension(), Some("css"));
    }

    #[test]
    fn url_directory() {
        let url = Url::parse("http://example.com/a/b/c.html").unwrap();
        assert_eq!(url.directory(), "/a/b/");
    }

    #[test]
    fn url_origin() {
        let url = Url::parse("https://example.com:8443/path").unwrap();
        assert_eq!(url.origin(), "https://example.com:8443");
    }

    #[test]
    fn detect_content_type_for_html() {
        let url = Url::parse("http://example.com/index.html").unwrap();
        assert_eq!(detect_content_type(&url), ContentType::Html);
    }

    #[test]
    fn detect_content_type_defaults_to_html() {
        let url = Url::parse("http://example.com/page").unwrap();
        assert_eq!(detect_content_type(&url), ContentType::Html);
    }

    #[test]
    fn content_type_is_image() {
        assert!(ContentType::Jpeg.is_image());
        assert!(ContentType::Png.is_image());
        assert!(ContentType::Bmp.is_image());
        assert!(ContentType::Gif.is_image());
        assert!(!ContentType::Html.is_image());
        assert!(!ContentType::Css.is_image());
    }

    #[test]
    fn resolve_query_only() {
        let base = Url::parse("http://example.com/search?old=1#s").unwrap();
        let resolved = base.resolve("?q=new").unwrap();
        assert_eq!(resolved.path, "/search");
        assert_eq!(resolved.query, Some("q=new".to_string()));
        assert_eq!(resolved.fragment, None);
    }

    #[test]
    fn parse_gemini_url() {
        let url = Url::parse("gemini://gem.example/page.gmi").unwrap();
        assert_eq!(url.scheme, "gemini");
        assert_eq!(url.host, "gem.example");
        assert_eq!(url.path, "/page.gmi");
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(Url::parse("").is_none());
    }

    #[test]
    fn resolve_empty_returns_self() {
        let base = Url::parse("http://example.com/page.html").unwrap();
        let resolved = base.resolve("").unwrap();
        assert_eq!(resolved, base);
    }
}
