//! Resource loading: URL parsing, content type detection, and loader
//! orchestration.

pub mod cache;
pub mod cookies;
pub mod csp;
#[cfg(not(target_arch = "wasm32"))]
pub mod gemini_fetch;
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
#[cfg(feature = "psp")]
pub mod http_psp;
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
pub mod io_thread;
pub mod vfs;

use std::fmt;

use oasis_types::error::Result;

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

/// HTTP method for a resource request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HttpMethod {
    /// Standard GET request (default).
    #[default]
    Get,
    /// POST request with a body.
    Post,
}

/// A request for a resource.
#[derive(Debug, Clone)]
pub struct ResourceRequest {
    pub url: String,
    pub base_url: Option<String>,
    pub source: ResourceSource,
    /// HTTP method (defaults to GET).
    pub method: HttpMethod,
    /// Optional request body (used for POST).
    pub body: Option<Vec<u8>>,
    /// Optional referrer URL (privacy-stripped) to send as `Referer`
    /// header. Set when navigating from one page to another, or when
    /// loading sub-resources (images, stylesheets).
    pub referrer: Option<String>,
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

/// Build a privacy-stripped referrer string from a page URL.
///
/// Removes fragment and query components from the URL so that
/// sensitive data (session tokens, search queries) is not leaked
/// to the target server.
pub fn strip_referrer(page_url: &str) -> Option<String> {
    let url = Url::parse(page_url)?;
    // Return scheme://host[:port]/path (no query or fragment).
    let mut s = format!("{}://{}", url.scheme, url.host);
    if let Some(port) = url.port {
        s.push_str(&format!(":{port}"));
    }
    s.push_str(&url.path);
    Some(s)
}

/// Returns `true` if `href` is a fragment-only reference (e.g. `#section`).
pub fn is_fragment_only(href: &str) -> bool {
    let trimmed = href.trim();
    trimmed.starts_with('#')
}

/// Parse a `data:` URI into a `ResourceResponse`.
///
/// Format: `data:[<mediatype>][;base64],<data>`
pub fn parse_data_uri(uri: &str) -> Option<ResourceResponse> {
    let rest = uri.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;

    let is_base64 = meta.ends_with(";base64");
    let mime = if is_base64 {
        meta.strip_suffix(";base64").unwrap_or("")
    } else {
        meta
    };

    let content_type = if mime.is_empty() {
        ContentType::PlainText
    } else {
        ContentType::from_mime(mime)
    };

    let body = if is_base64 {
        // Simple base64 decode (no padding validation).
        base64_decode(data)?
    } else {
        // URL-decode the data.
        url_decode(data).into_bytes()
    };

    Some(ResourceResponse {
        url: uri.to_string(),
        content_type,
        body,
        status: 200,
    })
}

/// Simple URL percent-decoding.
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

/// Minimal base64 decode (standard alphabet, ignores padding).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let table = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'=' && b != b'\n' && b != b'\r' && b != b' ')
        .collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let b0 = table(*chunk.first()?)?;
        let b1 = table(*chunk.get(1)?)?;
        result.push((b0 << 2) | (b1 >> 4));
        if let Some(&c2) = chunk.get(2) {
            let b2 = table(c2)?;
            result.push((b1 << 4) | (b2 >> 2));
            if let Some(&c3) = chunk.get(3) {
                let b3 = table(c3)?;
                result.push((b2 << 6) | b3);
            }
        }
    }
    Some(result)
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
///
/// `tls` is forwarded to the HTTP client for HTTPS support.
/// `cookie_jar` is used to send/receive HTTP cookies.
/// `cache` is consulted for conditional request headers (ETag /
/// If-Modified-Since).
pub fn load_resource(
    vfs_backend: &dyn oasis_vfs::Vfs,
    request: &ResourceRequest,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))] cookie_jar: Option<
        &mut cookies::CookieJar,
    >,
    #[cfg(not(any(target_arch = "wasm32", feature = "psp")))] resource_cache: Option<
        &cache::ResourceCache,
    >,
) -> Result<LoadedResource> {
    match request.source {
        ResourceSource::Vfs => {
            vfs::load_from_vfs(vfs_backend, request).map(LoadedResource::from_response)
        },
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        ResourceSource::Network => load_from_network(request, tls, cookie_jar, resource_cache),
        #[cfg(any(target_arch = "wasm32", feature = "psp"))]
        ResourceSource::Network => {
            load_from_network(request, tls).map(LoadedResource::from_response)
        },
        ResourceSource::VfsThenNetwork => match vfs::load_from_vfs(vfs_backend, request) {
            Ok(resp) => Ok(LoadedResource::from_response(resp)),
            #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
            Err(_) => load_from_network(request, tls, cookie_jar, resource_cache),
            #[cfg(any(target_arch = "wasm32", feature = "psp"))]
            Err(_) => load_from_network(request, tls).map(LoadedResource::from_response),
        },
    }
}

/// A loaded resource with optional HTTP cache metadata.
pub struct LoadedResource {
    /// The resource response.
    pub response: ResourceResponse,
    /// ETag from the server (for conditional requests).
    pub etag: Option<String>,
    /// Last-Modified from the server (for conditional requests).
    pub last_modified: Option<String>,
    /// Content-Security-Policy parsed from response headers (if present).
    pub csp: Option<csp::CspPolicy>,
}

impl LoadedResource {
    /// Wrap a plain response with no cache metadata.
    pub fn from_response(response: ResourceResponse) -> Self {
        Self {
            response,
            etag: None,
            last_modified: None,
            csp: None,
        }
    }
}

/// Load a resource over the network (HTTP/HTTPS).
#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
fn load_from_network(
    request: &ResourceRequest,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
    cookie_jar: Option<&mut cookies::CookieJar>,
    resource_cache: Option<&cache::ResourceCache>,
) -> Result<LoadedResource> {
    let url = Url::parse(&request.url).ok_or_else(|| {
        oasis_types::error::OasisError::Backend(format!("invalid URL: {}", request.url,).into())
    })?;

    match url.scheme.as_str() {
        "http" | "https" => {
            let method = match request.method {
                HttpMethod::Get => "GET",
                HttpMethod::Post => "POST",
            };

            // Build extra headers for cookies, referrer, and conditional
            // requests.
            let mut extra: Vec<(String, String)> = Vec::new();

            // Add Referer header (privacy-stripped).
            if let Some(ref referrer) = request.referrer {
                extra.push(("Referer".to_string(), referrer.clone()));
            }

            // Add cookies.
            if let Some(ref jar) = cookie_jar
                && let Some(cookie_val) = jar.cookie_header(&url)
            {
                extra.push(("Cookie".to_string(), cookie_val));
            }

            // Add conditional request headers from cache.
            if let Some(rc) = resource_cache {
                let url_str = url.to_string();
                if let Some((etag, last_mod)) = rc.peek_validators(&url_str) {
                    if let Some(e) = etag {
                        extra.push(("If-None-Match".to_string(), e));
                    }
                    if let Some(lm) = last_mod {
                        extra.push(("If-Modified-Since".to_string(), lm));
                    }
                }
            }

            let extra_refs: Vec<(&str, &str)> = extra
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            let (resp, headers) =
                http::http_request_full(method, &url, request.body.as_deref(), &extra_refs, tls)?;

            // Store Set-Cookie headers from the response.
            if let Some(jar) = cookie_jar {
                jar.set_cookies(&url, &headers);
            }

            // Extract cache validators.
            let etag = http::response_find_header(&headers, "etag").map(String::from);
            let last_modified =
                http::response_find_header(&headers, "last-modified").map(String::from);

            // Parse Content-Security-Policy header if present.
            let csp =
                http::response_find_header(&headers, "content-security-policy").map(csp::parse_csp);

            // Handle 304 Not Modified -- return cached body.
            if resp.status == 304
                && let Some(rc) = resource_cache
            {
                let url_str = url.to_string();
                if let Some(cached) = rc.peek_response(&url_str) {
                    return Ok(LoadedResource {
                        response: cached,
                        etag,
                        last_modified,
                        csp,
                    });
                }
            }

            Ok(LoadedResource {
                response: resp,
                etag,
                last_modified,
                csp,
            })
        },
        "gemini" => gemini_fetch::gemini_get(&url, tls).map(LoadedResource::from_response),
        scheme => Err(oasis_types::error::OasisError::Backend(
            format!("unsupported network scheme: {scheme}",).into(),
        )),
    }
}

/// PSP network loader using raw `sceNetInet*` sockets + embedded-tls.
#[cfg(feature = "psp")]
fn load_from_network(
    request: &ResourceRequest,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
) -> Result<ResourceResponse> {
    let url = Url::parse(&request.url).ok_or_else(|| {
        oasis_types::error::OasisError::Backend(format!("invalid URL: {}", request.url,).into())
    })?;

    match url.scheme.as_str() {
        "http" | "https" => {
            let method = match request.method {
                HttpMethod::Get => "GET",
                HttpMethod::Post => "POST",
            };

            let mut extra: Vec<(&str, &str)> = Vec::new();

            // Add Referer header (privacy-stripped).
            let referrer_owned;
            if let Some(ref referrer) = request.referrer {
                referrer_owned = referrer.clone();
                extra.push(("Referer", &referrer_owned));
            }

            let (resp, _headers) =
                http_psp::http_request_full(method, &url, request.body.as_deref(), &extra, tls)?;

            Ok(resp)
        },
        scheme => Err(oasis_types::error::OasisError::Backend(
            format!("unsupported network scheme: {scheme}",).into(),
        )),
    }
}

/// Stub network loader for WASM builds (no TCP sockets in browsers).
#[cfg(target_arch = "wasm32")]
fn load_from_network(
    request: &ResourceRequest,
    _tls: Option<&dyn oasis_net::tls::TlsProvider>,
) -> Result<ResourceResponse> {
    Err(oasis_types::error::OasisError::Backend(
        format!("network loading not available in browser: {}", request.url,).into(),
    ))
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

    // -- strip_referrer -----------------------------------------------

    #[test]
    fn strip_referrer_removes_query_and_fragment() {
        let referrer = strip_referrer("http://example.com/page?token=abc#section");
        assert_eq!(referrer.as_deref(), Some("http://example.com/page"),);
    }

    #[test]
    fn strip_referrer_preserves_path() {
        let referrer = strip_referrer("https://example.com:8443/a/b/c.html");
        assert_eq!(
            referrer.as_deref(),
            Some("https://example.com:8443/a/b/c.html"),
        );
    }

    #[test]
    fn strip_referrer_invalid_url_returns_none() {
        assert_eq!(strip_referrer("not a url"), None);
    }
}
