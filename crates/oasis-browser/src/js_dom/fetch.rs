//! `fetch()` for page scripts: an origin-aware [`FetchHandler`] bound
//! behind `oasis-js`'s Promise-based `fetch()` wrapper.
//!
//! The JS surface (`fetch`, `Response`, `Headers`) comes from
//! `oasis_js::fetch`; this module only supplies the transport and the
//! security policy. The request itself is synchronous on the JS thread
//! (bounded by the HTTP client's connect / read timeouts), after which
//! the promise settles and `.then()` chains run as microtasks.
//!
//! ## Policy (pages loaded over `http:` / `https:`)
//!
//! - Relative URLs resolve against the document URL.
//! - **Private network access:** a page may not reach an address class
//!   more private than its own (public < private/link-local < loopback).
//!   Literal hosts are checked up front; DNS names are checked on the
//!   exact address dialled (and on every redirect hop), so rebinding a
//!   public name to `127.0.0.1` does not get through.
//! - **Same-origin** requests may use any method, headers and body.
//! - **Cross-origin** requests must be *simple*: `GET`/`HEAD`, no body,
//!   only CORS-safelisted headers. An `Origin` header is attached and the
//!   response is only exposed when `Access-Control-Allow-Origin` is `*`
//!   or the page origin (no preflight is implemented, so non-simple
//!   cross-origin requests are rejected outright).
//! - CSP `connect-src` is enforced when the page has an active policy.
//!
//! Pages without a web origin (`vfs://` pages, HTML loaded from a
//! string) keep the permissive legacy behaviour: absolute `http(s)` URLs
//! only, no network-class or CORS restrictions.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use oasis_js::fetch::{FetchHandler, FetchRequest, FetchResponse};
use oasis_js::rquickjs::{Ctx, Result as JsResult};
use oasis_net::tls::TlsProvider;

use crate::loader::Url;
use crate::loader::csp::{CspPolicy, CspResourceType};

/// Bind the browser's fetch handler for a page at `url` into `ctx`.
pub(super) fn install_fetch_binding(
    ctx: &Ctx<'_>,
    url: &str,
    csp: Option<&CspPolicy>,
    tls: Option<Arc<dyn TlsProvider>>,
) -> JsResult<()> {
    let handler = BrowserFetchHandler::new(url, csp.cloned(), Box::new(NetTransport { tls }));
    oasis_js::fetch::bind_fetch_handler(ctx, Box::new(handler))
}

// ------------------------------------------------------------------
// Origins and network classes
// ------------------------------------------------------------------

/// Serialized origin (`scheme://host[:port]`) with the host lower-cased
/// and the scheme's default port elided, or `None` for URLs without a
/// host (opaque origins).
pub(crate) fn origin_key(url: &Url) -> Option<String> {
    if url.scheme.is_empty() || url.host.is_empty() {
        return None;
    }
    let host = url.host.to_ascii_lowercase();
    let default_port = match url.scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        "gemini" => Some(1965),
        _ => None,
    };
    Some(match url.port {
        Some(p) if Some(p) != default_port => format!("{}://{host}:{p}", url.scheme),
        _ => format!("{}://{host}", url.scheme),
    })
}

fn is_web_scheme(url: &Url) -> bool {
    url.scheme == "http" || url.scheme == "https"
}

/// Address space of a host, ordered from least to most private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NetClass {
    /// Publicly routable internet address.
    Public,
    /// RFC 1918, link-local, CGNAT, unique-local IPv6, multicast.
    Private,
    /// The local machine (`127/8`, `0/8`, `::1`, `::`, `localhost`).
    Loopback,
}

/// Classify one IP address.
pub(crate) fn classify_ip(ip: IpAddr) -> NetClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return NetClass::Loopback;
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return classify_v4(v4);
            }
            let seg = v6.segments();
            // NAT64 well-known prefix 64:ff9b::/96 embeds an IPv4 address.
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = seg[6].to_be_bytes();
                let [c, d] = seg[7].to_be_bytes();
                return classify_v4(Ipv4Addr::new(a, b, c, d));
            }
            let unique_local = seg[0] & 0xfe00 == 0xfc00; // fc00::/7
            let link_local = seg[0] & 0xffc0 == 0xfe80; // fe80::/10
            let site_local = seg[0] & 0xffc0 == 0xfec0; // fec0::/10 (deprecated)
            if unique_local || link_local || site_local || v6.is_multicast() {
                NetClass::Private
            } else {
                NetClass::Public
            }
        },
    }
}

fn classify_v4(v4: Ipv4Addr) -> NetClass {
    let o = v4.octets();
    if v4.is_loopback() || o[0] == 0 {
        NetClass::Loopback
    } else if v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
    {
        NetClass::Private
    } else {
        NetClass::Public
    }
}

/// Parse a host that is an IP literal, including bracketed IPv6 and the
/// legacy `inet_aton` IPv4 spellings browsers accept (`127.1`,
/// `2130706433`, `0x7f.0.0.1`, `0177.0.0.1`).
pub(crate) fn parse_ip_host(host: &str) -> Option<IpAddr> {
    let host = host.trim_end_matches('.');
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse().ok();
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    parse_ipv4_loose(host).map(IpAddr::V4)
}

fn parse_ipv4_loose(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut nums = Vec::with_capacity(parts.len());
    for p in &parts {
        let n = if let Some(hex) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
            if hex.is_empty() {
                0
            } else {
                u32::from_str_radix(hex, 16).ok()?
            }
        } else if p.len() > 1 && p.starts_with('0') {
            u32::from_str_radix(&p[1..], 8).ok()?
        } else {
            p.parse::<u32>().ok()?
        };
        nums.push(n);
    }
    let (last, head) = nums.split_last()?;
    if head.iter().any(|&n| n > 255) {
        return None;
    }
    let rest_bits = 8 * (4 - head.len() as u32);
    if rest_bits < 32 && *last >= (1u32 << rest_bits) {
        return None;
    }
    let mut addr: u32 = 0;
    for (i, &n) in head.iter().enumerate() {
        addr |= n << (24 - 8 * i as u32);
    }
    Some(Ipv4Addr::from(addr | last))
}

/// Class of a host name: `localhost` names and IP literals are
/// classified directly; other names are classified by `resolve`.
fn classify_host(host: &str, resolve: &dyn Fn(&str) -> Vec<IpAddr>) -> NetClass {
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return NetClass::Loopback;
    }
    if let Some(ip) = parse_ip_host(&lower) {
        return classify_ip(ip);
    }
    resolve(&lower)
        .into_iter()
        .map(classify_ip)
        .max()
        .unwrap_or(NetClass::Public)
}

// ------------------------------------------------------------------
// Header rules
// ------------------------------------------------------------------

/// Request headers page scripts may never set (Fetch "forbidden
/// request-header" list); they are silently dropped.
fn is_forbidden_request_header(name: &str) -> bool {
    const FORBIDDEN: &[&str] = &[
        "accept-charset",
        "accept-encoding",
        "access-control-request-headers",
        "access-control-request-method",
        "connection",
        "content-length",
        "cookie",
        "cookie2",
        "date",
        "dnt",
        "expect",
        "host",
        "keep-alive",
        "origin",
        "referer",
        "set-cookie",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "via",
    ];
    FORBIDDEN.contains(&name) || name.starts_with("proxy-") || name.starts_with("sec-")
}

/// Headers allowed on a cross-origin request without a preflight.
fn is_cors_safelisted_header(name: &str) -> bool {
    matches!(name, "accept" | "accept-language" | "content-language")
}

fn is_valid_method(method: &str) -> bool {
    !method.is_empty()
        && method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

// ------------------------------------------------------------------
// Transport
// ------------------------------------------------------------------

/// A request ready for the wire.
pub(crate) struct TransportRequest<'a> {
    pub method: &'a str,
    pub url: &'a Url,
    pub headers: &'a [(String, String)],
    pub body: Option<&'a [u8]>,
}

/// The raw result of a request.
pub(crate) struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Final URL after redirects.
    pub url: String,
}

/// Network access used by [`BrowserFetchHandler`]; mocked in tests.
pub(crate) trait FetchTransport {
    /// Resolve `host` to addresses (empty when unknown / unsupported).
    fn resolve_host(&self, host: &str) -> Vec<IpAddr>;
    /// Whether `https:` requests can be made.
    fn supports_https(&self) -> bool;
    /// Perform the request. `redirect_ok` gates each redirect hop and
    /// `addr_ok` each address actually dialled.
    fn send(
        &self,
        req: &TransportRequest<'_>,
        redirect_ok: &dyn Fn(&Url) -> bool,
        addr_ok: &dyn Fn(IpAddr) -> bool,
    ) -> Result<RawResponse, String>;
}

/// Real transport: the loader's HTTP client with the browser's TLS
/// provider (the same stack the page loader uses for HTTPS).
struct NetTransport {
    tls: Option<Arc<dyn TlsProvider>>,
}

impl FetchTransport for NetTransport {
    fn resolve_host(&self, host: &str) -> Vec<IpAddr> {
        #[cfg(not(any(feature = "psp", target_arch = "wasm32")))]
        {
            crate::loader::http::dns_resolve_cached(host).unwrap_or_default()
        }
        #[cfg(any(feature = "psp", target_arch = "wasm32"))]
        {
            let _ = host;
            Vec::new()
        }
    }

    fn supports_https(&self) -> bool {
        self.tls.is_some()
    }

    fn send(
        &self,
        req: &TransportRequest<'_>,
        redirect_ok: &dyn Fn(&Url) -> bool,
        addr_ok: &dyn Fn(IpAddr) -> bool,
    ) -> Result<RawResponse, String> {
        let headers: Vec<(&str, &str)> = req
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let tls = self.tls.as_deref();

        #[cfg(not(any(feature = "psp", target_arch = "wasm32")))]
        let result = {
            let guard = crate::loader::http::RequestGuard {
                redirect_ok,
                addr_ok,
            };
            crate::loader::http::http_request_guarded(
                req.method, req.url, req.body, &headers, tls, &guard,
            )
        };
        #[cfg(feature = "psp")]
        let result = {
            let _ = addr_ok;
            crate::loader::http_psp::http_request_guarded(
                req.method,
                req.url,
                req.body,
                &headers,
                tls,
                redirect_ok,
            )
        };
        #[cfg(all(target_arch = "wasm32", not(feature = "psp")))]
        let result: oasis_types::error::Result<(crate::loader::ResourceResponse, Vec<_>)> = {
            let _ = (redirect_ok, addr_ok, headers, tls);
            Err(oasis_types::error::OasisError::Backend(
                "fetch: no network access on this target".into(),
            ))
        };

        let (resp, headers) = result.map_err(|e| e.to_string())?;
        Ok(RawResponse {
            status: resp.status,
            headers,
            body: resp.body,
            url: resp.url,
        })
    }
}

// ------------------------------------------------------------------
// Handler
// ------------------------------------------------------------------

/// Origin-aware `fetch()` backend for one page.
pub(crate) struct BrowserFetchHandler {
    /// Parsed document URL (`None` for opaque / empty URLs).
    page_url: Option<Url>,
    /// Document URL as given (for CSP `'self'` matching).
    page_url_str: String,
    csp: Option<CspPolicy>,
    transport: Box<dyn FetchTransport>,
}

impl BrowserFetchHandler {
    pub(crate) fn new(
        page_url: &str,
        csp: Option<CspPolicy>,
        transport: Box<dyn FetchTransport>,
    ) -> Self {
        Self {
            page_url: crate::loader::Url::parse(page_url).filter(|u| !u.host.is_empty()),
            page_url_str: page_url.to_string(),
            csp,
            transport,
        }
    }

    fn classify(&self, url: &Url) -> NetClass {
        classify_host(&url.host, &|h| self.transport.resolve_host(h))
    }

    fn fetch_data_uri(&self, raw: &str, method: &str) -> Result<FetchResponse, String> {
        if method != "GET" && method != "HEAD" {
            return Err("data: URLs only support GET".into());
        }
        let resp = crate::loader::parse_data_uri(raw).ok_or("malformed data: URL")?;
        let mime = raw
            .get(5..)
            .and_then(|r| r.split([',', ';']).next())
            .filter(|m| !m.is_empty())
            .unwrap_or("text/plain");
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), mime.to_string());
        Ok(FetchResponse {
            status: 200,
            headers,
            body: String::from_utf8_lossy(&resp.body).into_owned(),
            status_text: "OK".into(),
            url: raw.to_string(),
        })
    }
}

impl FetchHandler for BrowserFetchHandler {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, String> {
        let method = request.method.as_str();
        if !is_valid_method(method) {
            return Err(format!("invalid method '{method}'"));
        }
        if matches!(
            method.to_ascii_uppercase().as_str(),
            "CONNECT" | "TRACE" | "TRACK"
        ) {
            return Err(format!("method '{method}' is forbidden"));
        }

        let raw = request.url.trim();
        if raw
            .get(..5)
            .is_some_and(|p| p.eq_ignore_ascii_case("data:"))
        {
            return self.fetch_data_uri(raw, method);
        }

        // Resolve against the document URL. The fragment is never sent,
        // so drop it before resolving (`Url::resolve` would otherwise
        // fold it into a query-only reference's query string).
        let no_frag = raw.split('#').next().unwrap_or(raw);
        let mut target = match self.page_url {
            Some(ref base) => base.resolve(no_frag),
            None => Url::parse(no_frag),
        }
        .filter(|u| !u.host.is_empty())
        .ok_or_else(|| format!("invalid URL '{raw}'"))?;
        target.fragment = None;
        if !is_web_scheme(&target) {
            return Err(format!("unsupported URL scheme '{}'", target.scheme));
        }
        if target.host.contains(['@', ' ', '\\']) {
            return Err(format!("invalid host in URL '{raw}'"));
        }
        if target.scheme == "https" && !self.transport.supports_https() {
            return Err("HTTPS is unavailable (no TLS provider configured)".into());
        }
        let target_str = target.to_string();

        if let Some(ref policy) = self.csp
            && policy.is_active()
            && !policy.allows(&target_str, &self.page_url_str, CspResourceType::Connect)
        {
            return Err(format!(
                "{target_str} blocked by Content-Security-Policy connect-src"
            ));
        }

        let mut headers: Vec<(String, String)> = request
            .headers
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .filter(|(k, _)| !is_forbidden_request_header(k))
            .collect();
        headers.sort();
        let body = request.body.as_deref().map(str::as_bytes);
        let simple = matches!(method, "GET" | "HEAD")
            && body.is_none()
            && headers.iter().all(|(k, _)| is_cors_safelisted_header(k));

        // Web-origin policy: network class, same-origin / CORS.
        let page = self.page_url.as_ref().filter(|u| is_web_scheme(u));
        let page_origin = page.and_then(origin_key);
        let page_class = page.map(|p| self.classify(p));
        let target_origin = origin_key(&target);
        if let (Some(page_origin), Some(page_class)) = (&page_origin, page_class) {
            let target_class = self.classify(&target);
            if target_class > page_class {
                return Err(format!(
                    "{target_str} blocked: a {page_class:?} page may not reach a \
                     {target_class:?} address"
                ));
            }
            let same_origin = target_origin.as_deref() == Some(page_origin.as_str());
            if !same_origin && !simple {
                return Err(format!(
                    "cross-origin request to {target_str} blocked: only simple GET/HEAD \
                     requests without a body or custom headers may leave the page origin"
                ));
            }
            if !same_origin || !matches!(method, "GET" | "HEAD") {
                headers.push(("origin".to_string(), page_origin.clone()));
            }
        }

        let redirect_ok = |next: &Url| -> bool {
            if !is_web_scheme(next) {
                return false;
            }
            if let Some(pc) = page_class
                && self.classify(next) > pc
            {
                return false;
            }
            // A non-simple request must not be bounced to another origin.
            simple || origin_key(next) == target_origin
        };
        let addr_ok = |ip: IpAddr| -> bool { page_class.is_none_or(|pc| classify_ip(ip) <= pc) };

        let resp = self.transport.send(
            &TransportRequest {
                method,
                url: &target,
                headers: &headers,
                body,
            },
            &redirect_ok,
            &addr_ok,
        )?;

        // CORS: expose a cross-origin response only with a matching ACAO.
        if let Some(ref page_origin) = page_origin {
            let final_origin = Url::parse(&resp.url)
                .as_ref()
                .and_then(origin_key)
                .or_else(|| target_origin.clone());
            if final_origin.as_deref() != Some(page_origin.as_str()) {
                let acao = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("access-control-allow-origin"))
                    .map(|(_, v)| v.trim());
                if !matches!(acao, Some(v) if v == "*" || v == page_origin) {
                    return Err(format!(
                        "CORS: {} did not allow origin {page_origin}",
                        resp.url
                    ));
                }
            }
        }

        let mut out_headers: HashMap<String, String> = HashMap::new();
        for (k, v) in resp.headers {
            let k = k.to_ascii_lowercase();
            if k == "set-cookie" || k == "set-cookie2" {
                continue;
            }
            out_headers
                .entry(k)
                .and_modify(|e| {
                    e.push_str(", ");
                    e.push_str(&v);
                })
                .or_insert(v);
        }
        Ok(FetchResponse {
            status: resp.status,
            headers: out_headers,
            body: String::from_utf8_lossy(&resp.body).into_owned(),
            status_text: String::new(),
            url: if resp.url.is_empty() {
                target_str
            } else {
                resp.url
            },
        })
    }
}
