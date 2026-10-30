//! Resource loading for the OASIS_OS browser: URL parsing, content
//! type detection, and loader orchestration over VFS, HTTP/1.1, HTTP/2
//! (rustls via `oasis-net`) and Gemini, plus the cookie jar, the
//! resource cache, CSP parsing and the background I/O thread.
//!
//! `oasis-browser` re-exports this crate as `oasis_browser::loader`, so
//! embedders that only need to fetch (e.g. the `http` / `browse`
//! terminal commands in `oasis-core`) can depend on `oasis-http`
//! without pulling in the layout and paint engine.
//!
//! # Feature flags
//!
//! - `psp` — swap the std/rustls client for the PSP client in
//!   `http_psp` (raw `sceNetInet` sockets + embedded-tls). Forwarded
//!   from `oasis-browser/psp`.
//!
//! The build script sets `cfg(native_net)` on every target except
//! `wasm32` and `psp`: the I/O thread, the std TCP / rustls client, the
//! cookie jar and the DNS cache only exist there.

pub mod cache;
pub mod cookies;
pub mod csp;
pub mod gemini;
#[cfg(not(target_arch = "wasm32"))]
pub mod gemini_fetch;
#[cfg(not(target_arch = "wasm32"))]
pub mod hpack;
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
#[cfg(not(target_arch = "wasm32"))]
pub mod http1;
#[cfg(not(target_arch = "wasm32"))]
pub mod http2;
#[cfg(feature = "psp")]
pub mod http_psp;
#[cfg(native_net)]
pub mod io_thread;
mod url;
pub mod vfs;

pub use url::{Url, default_port, is_special_scheme, strip_referrer};

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
    /// Full URL of the page that initiated the request, for `SameSite`
    /// cookie enforcement ([`cookies::SameSiteContext`]). `None` for a
    /// navigation the user started (address bar, history), which counts
    /// as same-site. Unlike `referrer` it is never stripped or dropped.
    pub initiator: Option<String>,
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
    /// TrueType font (.ttf).
    FontTtf,
    /// OpenType font (.otf).
    FontOtf,
    /// WOFF font (.woff).
    FontWoff,
    /// WOFF2 font (.woff2).
    FontWoff2,
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
            "ttf" => ContentType::FontTtf,
            "otf" => ContentType::FontOtf,
            "woff" => ContentType::FontWoff,
            "woff2" => ContentType::FontWoff2,
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
            "font/ttf" | "application/x-font-ttf" => ContentType::FontTtf,
            "font/otf" | "application/x-font-opentype" => ContentType::FontOtf,
            "font/woff" | "application/font-woff" => ContentType::FontWoff,
            "font/woff2" => ContentType::FontWoff2,
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

/// Whether `s` is an HTTP token (RFC 9110 §5.6.2): valid as a method or
/// header field name.
pub fn is_http_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Check what an HTTP client is about to put on the wire: the method
/// and header names must be tokens, header values and the host free of
/// CR / LF / NUL. Anything else could inject headers or smuggle a
/// second request.
pub fn validate_request(method: &str, url: &Url, headers: &[(&str, &str)]) -> Result<()> {
    use oasis_types::error::OasisError;
    if !is_http_token(method) {
        return Err(OasisError::Backend(
            format!("invalid HTTP method {method:?}").into(),
        ));
    }
    if url.host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(OasisError::Backend("invalid host in request URL".into()));
    }
    for (name, value) in headers {
        if !is_http_token(name) || value.contains(['\r', '\n', '\0']) {
            return Err(OasisError::Backend(
                format!("invalid request header {name:?}").into(),
            ));
        }
    }
    Ok(())
}

/// Whether a page at `page_url` may load the subresource `resource_url`.
///
/// A page with a network origin (`http:`, `https:`, `gemini:`) must not
/// read the shell's virtual file system: `vfs://` images, stylesheets,
/// `@import`s and fonts are refused for it, the way real browsers keep
/// web pages away from `file://`. Its `http(s)` subresources may still be
/// served from the offline cache (`/sites/<host>/...`) by the
/// `VfsThenNetwork` loader. Local pages (`vfs://`, `about:`, HTML loaded
/// from a string) keep full VFS access.
pub fn subresource_allowed(page_url: Option<&str>, resource_url: &str) -> bool {
    let is_vfs = resource_url
        .trim_start()
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("vfs://"));
    if !is_vfs {
        return true;
    }
    let page_is_web = page_url
        .and_then(Url::parse)
        .is_some_and(|u| is_special_scheme(&u.scheme));
    !page_is_web
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
        // Whitespace-tolerant, padding optional.
        oasis_types::util::base64::decode(data).ok()?
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
/// `cookie_jar` is used to send/receive HTTP cookies; such a load is
/// treated as a top-level navigation by `request.initiator` for
/// `SameSite` (subresources go through the I/O thread).
/// `cache` is consulted for conditional request headers (ETag /
/// If-Modified-Since).
pub fn load_resource(
    vfs_backend: &dyn oasis_vfs::Vfs,
    request: &ResourceRequest,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
    #[cfg(native_net)] cookie_jar: Option<&mut cookies::CookieJar>,
    #[cfg(native_net)] resource_cache: Option<&cache::ResourceCache>,
) -> Result<LoadedResource> {
    match request.source {
        ResourceSource::Vfs => {
            vfs::load_from_vfs(vfs_backend, request).map(LoadedResource::from_response)
        },
        #[cfg(native_net)]
        ResourceSource::Network => load_from_network(request, tls, cookie_jar, resource_cache),
        #[cfg(all(feature = "psp", not(target_arch = "wasm32")))]
        ResourceSource::Network => load_from_network(request, tls),
        #[cfg(target_arch = "wasm32")]
        ResourceSource::Network => {
            load_from_network(request, tls).map(LoadedResource::from_response)
        },
        ResourceSource::VfsThenNetwork => match vfs::load_from_vfs(vfs_backend, request) {
            Ok(resp) => Ok(LoadedResource::from_response(resp)),
            Err(vfs_err) => {
                // Only fall back to network for schemes the network
                // loader actually handles. A missing `vfs://` file
                // should surface the real VFS error, not the
                // misleading "unsupported network scheme: vfs"
                // produced by handing a vfs URL to the HTTP code
                // path.
                let scheme = Url::parse(&request.url)
                    .map(|u| u.scheme.clone())
                    .unwrap_or_default();
                if matches!(scheme.as_str(), "vfs" | "about" | "data") {
                    return Err(vfs_err);
                }
                #[cfg(native_net)]
                {
                    load_from_network(request, tls, cookie_jar, resource_cache)
                }
                #[cfg(all(feature = "psp", not(target_arch = "wasm32")))]
                {
                    load_from_network(request, tls)
                }
                #[cfg(target_arch = "wasm32")]
                {
                    load_from_network(request, tls).map(LoadedResource::from_response)
                }
            },
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
#[cfg(native_net)]
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

            // Build extra headers for the referrer and conditional
            // requests. Cookies are added per redirect hop by the HTTP
            // client, so they always match the hop's own URL.
            let mut extra: Vec<(String, String)> = Vec::new();

            // Add Referer header (privacy-stripped).
            if let Some(ref referrer) = request.referrer {
                extra.push(("Referer".to_string(), referrer.clone()));
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

            let body = request.body.as_deref();
            let (resp, headers) = match cookie_jar {
                Some(jar) => http::http_request_with_cookies(
                    method,
                    &url,
                    body,
                    &extra_refs,
                    tls,
                    // Synchronous cookie-carrying loads are page loads.
                    &mut cookies::ScopedJar {
                        jar,
                        ctx: cookies::SameSiteContext::new(
                            request.initiator.as_deref(),
                            true,
                            method,
                        ),
                    },
                    http::MAX_REDIRECTS,
                )?,
                None => http::http_request_full(method, &url, body, &extra_refs, tls)?,
            };

            // Extract cache validators.
            let etag = http::response_find_header(&headers, "etag").map(String::from);
            let last_modified =
                http::response_find_header(&headers, "last-modified").map(String::from);

            // Content-Security-Policy of the final response.
            let csp = csp::parse_csp_headers(&headers);

            // Handle 304 Not Modified -- return cached body.
            if resp.status == 304
                && let Some(rc) = resource_cache
            {
                let url_str = url.to_string();
                if let Some(cached) = rc.peek_response(&url_str) {
                    // A 304 may omit the CSP header: the cached body
                    // keeps the policy it was served with.
                    let csp = csp.or_else(|| rc.peek_csp(&url_str));
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
///
/// No cookie jar is wired on PSP yet; the response's CSP and cache
/// validators are carried like on desktop.
#[cfg(all(feature = "psp", not(target_arch = "wasm32")))]
fn load_from_network(
    request: &ResourceRequest,
    tls: Option<&dyn oasis_net::tls::TlsProvider>,
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

            let mut extra: Vec<(&str, &str)> = Vec::new();

            // Add Referer header (privacy-stripped).
            let referrer_owned;
            if let Some(ref referrer) = request.referrer {
                referrer_owned = referrer.clone();
                extra.push(("Referer", &referrer_owned));
            }

            let (resp, headers) =
                http_psp::http_request_full(method, &url, request.body.as_deref(), &extra, tls)?;

            Ok(LoadedResource {
                response: resp,
                etag: http1::find_header(&headers, "etag").map(String::from),
                last_modified: http1::find_header(&headers, "last-modified").map(String::from),
                csp: csp::parse_csp_headers(&headers),
            })
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

    // -- Request-line / header hygiene --------------------------------------

    #[test]
    fn request_target_percent_encodes_controls_space_and_non_ascii() {
        let mut url = Url::parse("http://h/a").unwrap();
        url.path = "/a\r\n\r\nPOST /x HTTP/1.1".into();
        url.query = Some("q=a b\u{e9}".into());
        let target = url.request_target();
        assert_eq!(target, "/a%0D%0A%0D%0APOST%20/x%20HTTP/1.1?q=a%20b%C3%A9");
        assert!(!target.contains(['\r', '\n', ' ']));
        url.path.clear();
        url.query = None;
        assert_eq!(url.request_target(), "/");
        // Already-encoded input is left alone.
        url.path = "/%41%20b".into();
        assert_eq!(url.request_target(), "/%41%20b");
    }

    #[test]
    fn validate_request_rejects_injection() {
        let url = Url::parse("http://h/").unwrap();
        assert!(validate_request("GET", &url, &[("Accept", "text/html")]).is_ok());
        assert!(validate_request("GET\r\n", &url, &[]).is_err());
        assert!(validate_request("GET", &url, &[("Accept", "x\r\nCookie: a")]).is_err());
        assert!(validate_request("GET", &url, &[("X-A\nB", "1")]).is_err());
        assert!(validate_request("GET", &url, &[("X", "a\0b")]).is_err());
        let mut bad_host = url.clone();
        bad_host.host = "h\r\nX: 1".into();
        assert!(validate_request("GET", &bad_host, &[]).is_err());
    }

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
    fn resolve_relative_with_url_in_query() {
        let base = Url::parse("http://example.com/a/page.html").unwrap();
        let resolved = base.resolve("login?next=http://other.test/x").unwrap();
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.path, "/a/login");
        assert_eq!(resolved.query.as_deref(), Some("next=http://other.test/x"));
        let resolved = base.resolve("/go?to=https://other.test/").unwrap();
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.path, "/go");
        // A real absolute URL still wins.
        let resolved = base.resolve("HTTPS://other.test/p").unwrap();
        assert_eq!(resolved.host, "other.test");
    }

    #[test]
    fn resolve_dotdot_in_relative_paths() {
        let base = Url::parse("http://example.com/a/b/c.html").unwrap();
        let resolved = base.resolve("../../d.html").unwrap();
        assert_eq!(resolved.path, "/d.html");
    }

    /// Regression: directory references lost their trailing slash
    /// (`sub/` resolved to `/a/sub`, so servers answered with a
    /// redirect and relative links on that page resolved one level too
    /// high), and absolute-path references kept their dot segments
    /// (`/x/../y`, which the VFS loader then refused as traversal).
    #[test]
    fn resolve_keeps_directory_slash_and_removes_dot_segments() {
        let base = Url::parse("http://example.com/a/b/c.html").unwrap();
        let path = |r: &str| base.resolve(r).unwrap().path;
        assert_eq!(path("sub/"), "/a/b/sub/");
        assert_eq!(path("./"), "/a/b/");
        assert_eq!(path("."), "/a/b/");
        assert_eq!(path(".."), "/a/");
        assert_eq!(path("../../../.."), "/");
        assert_eq!(path("x/./y/../z"), "/a/b/x/z");
        assert_eq!(path("/x/../y"), "/y");
        assert_eq!(path("/../../etc/passwd"), "/etc/passwd");
        assert_eq!(path("/dir/"), "/dir/");
        assert_eq!(path("/"), "/");
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
        let url = Url::parse("https://example.com:8443/path?q=1#frag").unwrap();
        assert_eq!(url.to_string(), "https://example.com:8443/path?q=1#frag");
        // The default port is elided.
        let url = Url::parse("https://example.com:443/path?q=1#frag").unwrap();
        assert_eq!(url.to_string(), "https://example.com/path?q=1#frag");
    }

    // -- normalization (F15) -------------------------------------------

    #[test]
    fn userinfo_is_split_from_the_host() {
        let url = Url::parse("http://user:pw@Example.COM:8080/p").unwrap();
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, Some(8080));
        assert_eq!(url.userinfo.as_deref(), Some("user:pw"));
        // Credentials never reach the serialized URL.
        assert_eq!(url.to_string(), "http://example.com:8080/p");
        // The last `@` ends the userinfo (WHATWG).
        let url = Url::parse("https://a@b@evil.test/").unwrap();
        assert_eq!(url.host, "evil.test");
        assert_eq!(url.userinfo.as_deref(), Some("a@b"));
    }

    #[test]
    fn network_hosts_are_lower_cased_but_vfs_hosts_are_not() {
        assert_eq!(
            Url::parse("HTTPS://WWW.Example.Com/").unwrap().host,
            "www.example.com"
        );
        assert_eq!(Url::parse("gemini://Gem.Test/").unwrap().host, "gem.test");
        assert_eq!(Url::parse("vfs://Sites/Corp/x.html").unwrap().host, "Sites");
    }

    #[test]
    fn backslash_is_a_path_separator_for_network_schemes() {
        let url = Url::parse("http://good.test\\@evil.test/").unwrap();
        assert_eq!(url.host, "good.test");
        assert_eq!(url.userinfo, None);
    }

    #[test]
    fn ipv6_literals_and_bad_ports() {
        let url = Url::parse("http://[::1]:8080/x").unwrap();
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.port, Some(8080));
        let url = Url::parse("http://[::1]/x").unwrap();
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.port, None);
        assert!(Url::parse("http://host:abc/").is_none());
        assert!(Url::parse("http://host:99999/").is_none());
        assert!(Url::parse("http://[::1]x/").is_none());
        // An empty port is the default.
        assert_eq!(Url::parse("http://host:/").unwrap().port, None);
    }

    #[test]
    fn origin_is_normalized() {
        let a = Url::parse("https://Example.com:443/a").unwrap();
        let b = Url::parse("https://example.com/b").unwrap();
        assert_eq!(a.origin().as_deref(), Some("https://example.com"));
        assert!(a.same_origin(&b));
        // A hand-built Url with an explicit default port still matches.
        let mut c = b.clone();
        c.port = Some(443);
        c.host = "EXAMPLE.com".into();
        assert!(c.same_origin(&a));
        assert!(!a.same_origin(&Url::parse("http://example.com/").unwrap()));
        assert_eq!(Url::parse("#frag").unwrap().origin(), None);
    }

    #[test]
    fn vfs_subresources_are_blocked_from_web_pages() {
        let web = Some("https://site.test/page");
        assert!(!subresource_allowed(web, "vfs://home/secret.png"));
        assert!(!subresource_allowed(web, "VFS://home/secret.png"));
        assert!(subresource_allowed(web, "https://cdn.test/a.png"));
        // The offline cache maps http(s) URLs into `/sites/<host>`.
        assert!(subresource_allowed(web, "http://other.test/a.png"));
        assert!(subresource_allowed(web, "data:image/png;base64,AA=="));
        // Local pages keep VFS access.
        assert!(subresource_allowed(
            Some("vfs://sites/a/index.html"),
            "vfs://x/y.png"
        ));
        assert!(subresource_allowed(None, "vfs://x/y.png"));
        assert!(subresource_allowed(Some("about:blank"), "vfs://x/y.png"));
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
        assert_eq!(url.origin().as_deref(), Some("https://example.com:8443"));
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
    fn resolve_query_only_with_fragment() {
        let base = Url::parse("http://example.com/search?old=1#s").unwrap();
        let resolved = base.resolve("?q=new#results").unwrap();
        assert_eq!(resolved.path, "/search");
        assert_eq!(resolved.query, Some("q=new".to_string()));
        assert_eq!(resolved.fragment, Some("results".to_string()));
        assert_eq!(
            resolved.to_string(),
            "http://example.com/search?q=new#results"
        );

        // Empty query with a fragment still splits correctly.
        let resolved = base.resolve("?#top").unwrap();
        assert_eq!(resolved.query, Some(String::new()));
        assert_eq!(resolved.fragment, Some("top".to_string()));
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
