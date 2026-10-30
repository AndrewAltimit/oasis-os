//! Session-scoped HTTP cookie jar (RFC 6265 / RFC 6265bis subset).
//!
//! What is implemented:
//!
//! - **Host-only cookies.** A cookie set without `Domain=` is sent back
//!   to the exact host only, never to its subdomains (§5.3 step 6).
//! - **Domain validation.** `Domain=` must domain-match the setting host;
//!   it is refused for IP-literal hosts (unless it equals the host) and for
//!   public suffixes. The public-suffix guard is a "PSL-lite": every
//!   single-label domain (`com`, `localhost`) plus a short embedded list of
//!   common multi-label registry suffixes ([`MULTI_LABEL_SUFFIXES`]). A
//!   full Public Suffix List is not shipped.
//! - **Expiry.** `Max-Age` (preferred) and `Expires` (any of the date
//!   formats §5.1.1 accepts) set an absolute expiry; `Max-Age<=0` or a past
//!   `Expires` deletes the stored cookie. Lifetimes are capped at 400 days.
//!   Expired cookies are never sent and are swept on every store.
//! - **`Secure`.** Sent over `https:` only, and refused when set by a
//!   non-`https:` response; a non-secure response cannot overwrite an
//!   existing `Secure` cookie. The `__Secure-` / `__Host-` name prefixes
//!   are enforced.
//! - **Hygiene.** Names and values with control characters (which could
//!   otherwise be smuggled into our `Cookie:` request header) are refused,
//!   as is any `name=value` over [`MAX_COOKIE_SIZE`] bytes. The jar keeps
//!   at most [`MAX_COOKIES_PER_DOMAIN`] cookies per domain and
//!   [`MAX_COOKIES`] in total, evicting the oldest.
//! - `Path` defaults per §5.1.4 and `Cookie:` lists longer paths first.
//! - **`SameSite` on send.** Requests carry a [`SameSiteContext`] (the
//!   initiating page, top-level or not, safe method or not): cross-site
//!   subresources get no `Strict` / `Lax` cookies, cross-site top-level
//!   navigations get `Lax` ones only with a safe method (see
//!   [`SameSiteContext`] for the exact rule).
//!
//! Not implemented: persistence (the jar lives for the browser session),
//! `SameSite` on *store* (a cross-site response may still set `Lax` /
//! `Strict` cookies), a missing `SameSite` defaulting to `Lax` (it is
//! treated as `None`), schemeful same-site, and `document.cookie`
//! integration (`js_dom/storage.rs` keeps a separate per-page map that
//! never reaches the network).

use std::net::IpAddr;

use super::Url;

/// Largest accepted `name=value` pair, in bytes (RFC 6265 §6.1 minimum).
pub const MAX_COOKIE_SIZE: usize = 4096;

/// Most cookies kept for one domain (RFC 6265 §6.1 minimum is 50).
pub const MAX_COOKIES_PER_DOMAIN: usize = 50;

/// Most cookies kept in the whole jar (RFC 6265 §6.1 minimum is 3000).
pub const MAX_COOKIES: usize = 3000;

/// Upper bound on a cookie's lifetime (RFC 6265bis §5.5: 400 days).
const MAX_LIFETIME_SECS: u64 = 400 * 24 * 60 * 60;

/// Attribute values longer than this are ignored (RFC 6265bis §5.6).
const MAX_ATTRIBUTE_VALUE: usize = 1024;

/// Multi-label public suffixes refused as a `Domain=` value, in addition
/// to every single-label domain. A small, deliberately incomplete stand-in
/// for the Public Suffix List covering the most common country-code
/// second-level registries and shared hosting domains.
pub const MULTI_LABEL_SUFFIXES: &[&str] = &[
    // United Kingdom
    "co.uk",
    "org.uk",
    "me.uk",
    "ltd.uk",
    "plc.uk",
    "net.uk",
    "ac.uk",
    "gov.uk",
    "nhs.uk",
    "sch.uk",
    // Australia / New Zealand
    "com.au",
    "net.au",
    "org.au",
    "edu.au",
    "gov.au",
    "id.au",
    "co.nz",
    "net.nz",
    "org.nz",
    "govt.nz",
    // Asia
    "co.jp",
    "ne.jp",
    "or.jp",
    "ac.jp",
    "go.jp",
    "co.kr",
    "or.kr",
    "go.kr",
    "com.cn",
    "net.cn",
    "org.cn",
    "gov.cn",
    "com.hk",
    "com.tw",
    "com.sg",
    "com.my",
    "co.in",
    "net.in",
    "org.in",
    "gov.in",
    "co.th",
    "co.id",
    "com.ph",
    "com.vn",
    "com.pk",
    // Americas
    "com.br",
    "net.br",
    "org.br",
    "gov.br",
    "com.mx",
    "org.mx",
    "gob.mx",
    "com.ar",
    "com.co",
    "com.pe",
    "com.ve",
    // Europe / Middle East / Africa
    "com.tr",
    "gov.tr",
    "co.il",
    "org.il",
    "co.za",
    "org.za",
    "gov.za",
    "com.eg",
    "com.sa",
    "com.ua",
    "com.pl",
    "co.at",
    "or.at",
    // Shared hosting
    "github.io",
    "gitlab.io",
    "pages.dev",
    "netlify.app",
    "vercel.app",
    "herokuapp.com",
    "appspot.com",
    "blogspot.com",
    "azurewebsites.net",
    "cloudfront.net",
    "web.app",
    "firebaseapp.com",
];

/// The `SameSite` attribute, enforced through [`SameSiteContext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    /// `SameSite=Strict`.
    Strict,
    /// `SameSite=Lax`.
    Lax,
    /// `SameSite=None`.
    None,
}

/// Who asked for a request, for `SameSite` enforcement (RFC 6265bis
/// §5.2 / §5.8.3, simplified).
///
/// A request is *cross-site* when its URL's registrable domain (see
/// [`registrable_domain`]; schemeless) differs from the initiator's.
/// Cross-site requests carry only cookies without `SameSite` or with
/// `SameSite=None` (missing is treated as `None`, the legacy
/// behaviour), plus `SameSite=Lax` cookies on a top-level navigation
/// with a safe method. Same-site requests and requests without an
/// initiator (the user typed the URL) carry every matching cookie. The
/// context applies to every redirect hop.
#[derive(Debug, Clone, Default)]
pub struct SameSiteContext {
    /// URL of the document that initiated the request; `None` for a
    /// navigation the user started.
    pub initiator: Option<Url>,
    /// A top-level (page) navigation rather than a subresource / fetch.
    pub top_level: bool,
    /// The request method is safe (`GET` / `HEAD`).
    pub safe_method: bool,
}

impl SameSiteContext {
    /// Context for a request from `initiator` (a URL string, as carried
    /// by `ResourceRequest::initiator`; unparsable means none).
    pub fn new(initiator: Option<&str>, top_level: bool, method: &str) -> Self {
        Self {
            initiator: initiator.and_then(Url::parse),
            top_level,
            safe_method: matches!(method, "GET" | "HEAD"),
        }
    }

    /// Whether `cookie` may be sent on a request to `url` in this context.
    pub fn allows(&self, cookie: &Cookie, url: &Url) -> bool {
        let Some(initiator) = self.initiator.as_ref() else {
            return true;
        };
        if is_same_site(initiator, url) {
            return true;
        }
        match cookie.same_site {
            Some(SameSite::Strict) => false,
            Some(SameSite::Lax) => self.top_level && self.safe_method,
            Some(SameSite::None) | None => true,
        }
    }
}

/// The registrable domain ("site") of `host` under the PSL-lite rules:
/// the public suffix plus one label (`a.b.example.co.uk` ->
/// `example.co.uk`). IP literals and single-label hosts are their own
/// site.
pub fn registrable_domain(host: &str) -> &str {
    if is_ip_host(host) {
        return host;
    }
    let host = host.trim_end_matches('.');
    // Walk the labels left to right: the first label whose remainder is
    // a public suffix starts the site.
    let mut label_start = 0;
    while let Some(dot) = host[label_start..].find('.') {
        if is_public_suffix(&host[label_start + dot + 1..]) {
            return &host[label_start..];
        }
        label_start += dot + 1;
    }
    host
}

/// Whether `a` and `b` are same-site (equal registrable domains).
pub fn is_same_site(a: &Url, b: &Url) -> bool {
    let (ha, hb) = (a.host.to_ascii_lowercase(), b.host.to_ascii_lowercase());
    registrable_domain(&ha) == registrable_domain(&hb)
}

/// A single HTTP cookie.
#[derive(Debug, Clone)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie belongs to (lower-case, no leading dot).
    pub domain: String,
    /// `true` when set without `Domain=`: sent to `domain` exactly, not
    /// to its subdomains.
    pub host_only: bool,
    /// Path scope for the cookie.
    pub path: String,
    /// Whether the cookie requires HTTPS.
    pub secure: bool,
    /// Whether the cookie is inaccessible to JavaScript.
    pub http_only: bool,
    /// Absolute expiry as Unix seconds; `None` for a session cookie.
    pub expires_at: Option<u64>,
    /// Parsed `SameSite` attribute.
    pub same_site: Option<SameSite>,
    /// Insertion order, for eviction and `Cookie:` ordering.
    seq: u64,
}

impl Cookie {
    fn is_expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }
}

/// Cookie access for the HTTP client's redirect loop: the client asks for
/// a `Cookie:` header for every hop's own URL and hands every hop's
/// response headers back, so each redirect hop gets exactly its own
/// cookies and `Set-Cookie` lands under the host that sent it.
pub trait CookieHook {
    /// The `Cookie:` header value to send to `url`, if any.
    fn cookie_header(&mut self, url: &Url) -> Option<String>;
    /// Store the `Set-Cookie` headers of a response received from `url`.
    fn store(&mut self, url: &Url, headers: &[(String, String)]);
}

/// Session-scoped cookie store.
///
/// Cookies are held in memory for the lifetime of the browser session
/// and are never persisted to disk. See the module docs for the RFC 6265
/// rules that are enforced.
#[derive(Debug, Clone, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
    next_seq: u64,
}

impl CookieJar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored (possibly expired, not yet swept) cookies.
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Whether the jar holds no cookies.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Parse `Set-Cookie` headers from a response and store the cookies.
    ///
    /// Only headers whose name (case-insensitive) is `set-cookie` are
    /// processed. `url` must be the URL that actually produced the
    /// response (the redirect hop, not the originally requested URL): it
    /// supplies the default domain / path and is what `Domain=` and
    /// `Secure` are validated against.
    pub fn set_cookies(&mut self, url: &Url, headers: &[(String, String)]) {
        self.set_cookies_at(url, headers, now_secs());
    }

    /// Build a `Cookie` header value for the given URL.
    ///
    /// Returns `None` when no cookies match.
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        self.cookie_header_at(url, now_secs())
    }

    /// [`cookie_header`](Self::cookie_header) with `SameSite`
    /// enforced for a request made in `ctx`.
    pub fn cookie_header_in(&self, url: &Url, ctx: &SameSiteContext) -> Option<String> {
        self.cookie_header_filtered(url, now_secs(), |c| ctx.allows(c, url))
    }

    fn set_cookies_at(&mut self, url: &Url, headers: &[(String, String)], now: u64) {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("set-cookie")
                && let Some(cookie) = parse_set_cookie(value, url, now)
            {
                self.insert(cookie, url.scheme == "https", now);
            }
        }
    }

    fn cookie_header_at(&self, url: &Url, now: u64) -> Option<String> {
        self.cookie_header_filtered(url, now, |_| true)
    }

    fn cookie_header_filtered(
        &self,
        url: &Url,
        now: u64,
        allow: impl Fn(&Cookie) -> bool,
    ) -> Option<String> {
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| cookie_matches(c, url, now) && allow(c))
            .collect();
        if matching.is_empty() {
            return None;
        }
        // RFC 6265 §5.4 step 2: longer paths first, then oldest first.
        matching.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.seq.cmp(&b.seq)));
        let pairs: Vec<String> = matching
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        Some(pairs.join("; "))
    }

    fn insert(&mut self, mut cookie: Cookie, secure_origin: bool, now: u64) {
        self.cookies.retain(|c| !c.is_expired(now));

        // RFC 6265bis §5.7 step 16: an insecure response may not
        // overwrite (or shadow) a Secure cookie of the same name. This
        // runs before the deletion branch below, so plain http can't
        // delete a Secure cookie with `Max-Age=0` either.
        if !secure_origin
            && self.cookies.iter().any(|c| {
                c.secure
                    && c.name == cookie.name
                    && (domain_match(&c.domain, &cookie.domain)
                        || domain_match(&cookie.domain, &c.domain))
                    && path_matches(&cookie.path, &c.path)
            })
        {
            return;
        }

        // An expiry in the past is a deletion request.
        if cookie.is_expired(now) {
            self.cookies.retain(|c| {
                !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
            });
            return;
        }

        if let Some(existing) = self
            .cookies
            .iter_mut()
            .find(|c| c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        {
            // Replacement keeps the original creation order (§5.3 step 11).
            cookie.seq = existing.seq;
            *existing = cookie;
            return;
        }

        cookie.seq = self.next_seq;
        self.next_seq += 1;
        let domain = cookie.domain.clone();
        self.cookies.push(cookie);

        // Caps: evict the oldest cookie of the domain, then overall.
        if self.cookies.iter().filter(|c| c.domain == domain).count() > MAX_COOKIES_PER_DOMAIN {
            self.evict_oldest(|c| c.domain == domain);
        }
        if self.cookies.len() > MAX_COOKIES {
            self.evict_oldest(|_| true);
        }
    }

    fn evict_oldest(&mut self, pred: impl Fn(&Cookie) -> bool) {
        let oldest = self
            .cookies
            .iter()
            .enumerate()
            .filter(|(_, c)| pred(c))
            .min_by_key(|(_, c)| c.seq)
            .map(|(i, _)| i);
        if let Some(i) = oldest {
            self.cookies.remove(i);
        }
    }
}

impl CookieHook for CookieJar {
    fn cookie_header(&mut self, url: &Url) -> Option<String> {
        CookieJar::cookie_header(self, url)
    }

    fn store(&mut self, url: &Url, headers: &[(String, String)]) {
        self.set_cookies(url, headers);
    }
}

/// A [`CookieJar`] seen through one request's [`SameSiteContext`]: the
/// hook to hand the HTTP client so every hop's `Cookie:` header honours
/// `SameSite`.
pub struct ScopedJar<'a> {
    /// The jar read and written.
    pub jar: &'a mut CookieJar,
    /// The request's initiator context.
    pub ctx: SameSiteContext,
}

impl CookieHook for ScopedJar<'_> {
    fn cookie_header(&mut self, url: &Url) -> Option<String> {
        self.jar.cookie_header_in(url, &self.ctx)
    }

    fn store(&mut self, url: &Url, headers: &[(String, String)]) {
        self.jar.set_cookies(url, headers);
    }
}

// -------------------------------------------------------------------
// Internals
// -------------------------------------------------------------------

/// Current time as Unix seconds.
fn now_secs() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_ip_host(host: &str) -> bool {
    host.starts_with('[') || host.parse::<IpAddr>().is_ok()
}

/// Whether `domain` is a public suffix under the PSL-lite rules.
fn is_public_suffix(domain: &str) -> bool {
    !domain.contains('.') || MULTI_LABEL_SUFFIXES.contains(&domain)
}

/// RFC 6265 §5.1.3 domain-match: `host` equals `domain` or is a
/// (non-IP) subdomain of it.
fn domain_match(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && !is_ip_host(host))
}

/// Whether a cookie name or value contains a character that must never
/// appear in a header (RFC 6265bis §5.6: CTLs other than HTAB).
fn has_control_chars(s: &str) -> bool {
    s.chars()
        .any(|c| (c.is_ascii_control() && c != '\t') || c == '\u{7f}')
}

/// RFC 6265 §5.1.4 default path: the request path up to (not including)
/// its last `/`, or `/`.
fn default_path(url: &Url) -> String {
    let path = url.path.as_str();
    if !path.starts_with('/') {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

/// Parse a single `Set-Cookie` header value into a [`Cookie`], applying
/// the storage rules (domain, secure, prefixes). `None` means "ignore".
fn parse_set_cookie(header: &str, url: &Url, now: u64) -> Option<Cookie> {
    if url.scheme != "http" && url.scheme != "https" {
        return None;
    }
    let host = url.host.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let secure_origin = url.scheme == "https";

    let mut parts = header.split(';');

    // First segment: name=value
    let name_value = parts.next()?.trim();
    let (name, value) = name_value.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty()
        || has_control_chars(name)
        || has_control_chars(value)
        || name.len() + value.len() > MAX_COOKIE_SIZE
    {
        return None;
    }

    let mut domain_attr: Option<String> = None;
    let mut path: Option<String> = None;
    let mut secure = false;
    let mut http_only = false;
    let mut max_age: Option<i64> = None;
    let mut expires: Option<i64> = None;
    let mut same_site = None;

    for part in parts {
        let part = part.trim();
        let (attr, attr_val) = match part.split_once('=') {
            Some((a, v)) => (a.trim(), v.trim()),
            None => (part, ""),
        };
        if attr_val.len() > MAX_ATTRIBUTE_VALUE {
            continue;
        }
        match attr.to_ascii_lowercase().as_str() {
            "domain" if !attr_val.is_empty() => {
                domain_attr = Some(attr_val.trim_start_matches('.').to_ascii_lowercase());
            },
            "path" => {
                // A missing or relative path falls back to the default.
                path = attr_val.starts_with('/').then(|| attr_val.to_string());
            },
            "max-age" => {
                if let Some(secs) = parse_max_age(attr_val) {
                    max_age = Some(secs);
                }
            },
            "expires" => {
                if let Some(t) = parse_cookie_date(attr_val) {
                    expires = Some(t);
                }
            },
            "secure" => secure = true,
            "httponly" => http_only = true,
            "samesite" => {
                same_site = match attr_val.to_ascii_lowercase().as_str() {
                    "strict" => Some(SameSite::Strict),
                    "lax" => Some(SameSite::Lax),
                    "none" => Some(SameSite::None),
                    _ => same_site,
                };
            },
            _ => {},
        }
    }

    // `Max-Age` wins over `Expires` (§5.3 step 3). Both are capped.
    let latest = now.saturating_add(MAX_LIFETIME_SECS);
    let expires_at = match (max_age, expires) {
        (Some(secs), _) if secs <= 0 => Some(0),
        (Some(secs), _) => Some(now.saturating_add(secs as u64).min(latest)),
        (None, Some(t)) => Some(u64::try_from(t).unwrap_or(0).min(latest)),
        (None, None) => None,
    };

    // Domain (§5.3 steps 4-6).
    let (domain, host_only) = match domain_attr {
        None => (host.clone(), true),
        Some(d) if is_ip_host(&host) => {
            // IP hosts have no parent domains to share with.
            if d != host {
                return None;
            }
            (host.clone(), true)
        },
        Some(d) if is_public_suffix(&d) => {
            // A public suffix may only name the setting host itself, and
            // then the cookie is host-only.
            if d != host {
                return None;
            }
            (host.clone(), true)
        },
        Some(d) => {
            if !domain_match(&host, &d) {
                return None;
            }
            (d, false)
        },
    };

    // Secure cookies only from secure origins (RFC 6265bis §5.7 step 8).
    if secure && !secure_origin {
        return None;
    }

    let path = path.unwrap_or_else(|| default_path(url));

    // Cookie name prefixes (RFC 6265bis §4.1.3).
    let lower_name = name.to_ascii_lowercase();
    if lower_name.starts_with("__secure-") && !(secure && secure_origin) {
        return None;
    }
    if lower_name.starts_with("__host-") && !(secure && secure_origin && host_only && path == "/") {
        return None;
    }

    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain,
        host_only,
        path,
        secure,
        http_only,
        expires_at,
        same_site,
        seq: 0,
    })
}

/// `Max-Age` value: `-`? DIGIT+ (§5.2.2). Saturates on overflow.
fn parse_max_age(v: &str) -> Option<i64> {
    let digits = v.strip_prefix('-').unwrap_or(v);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let negative = v.starts_with('-');
    let n = digits.parse::<i64>().unwrap_or(i64::MAX);
    Some(if negative { -n } else { n })
}

/// Check whether a cookie should be sent for the given URL (RFC 6265 §5.4).
fn cookie_matches(cookie: &Cookie, url: &Url, now: u64) -> bool {
    if cookie.is_expired(now) {
        return false;
    }
    if url.scheme != "http" && url.scheme != "https" {
        return false;
    }
    // Secure cookies only over HTTPS.
    if cookie.secure && url.scheme != "https" {
        return false;
    }
    let host = url.host.to_ascii_lowercase();
    let host_ok = if cookie.host_only {
        host == cookie.domain
    } else {
        domain_match(&host, &cookie.domain)
    };
    host_ok && path_matches(&url.path, &cookie.path)
}

/// RFC 6265 ss5.1.4 path matching.
///
/// Returns `true` if `request_path` matches `cookie_path`:
/// - The paths are identical, OR
/// - `cookie_path` is a prefix of `request_path` and either:
///   - `cookie_path` ends with '/', OR
///   - the character in `request_path` immediately after `cookie_path` is '/'
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    if request_path.starts_with(cookie_path) {
        if cookie_path.ends_with('/') {
            return true;
        }
        if request_path.as_bytes().get(cookie_path.len()) == Some(&b'/') {
            return true;
        }
    }
    false
}

// -------------------------------------------------------------------
// Cookie dates (RFC 6265 §5.1.1)
// -------------------------------------------------------------------

/// Parse a cookie date with the lenient algorithm of RFC 6265 §5.1.1,
/// which accepts IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`), RFC 850
/// (`Sunday, 06-Nov-94 08:49:37 GMT`), asctime (`Sun Nov  6 08:49:37
/// 1994`) and the usual real-world variations. Returns Unix seconds
/// (negative before 1970), or `None` for an unparseable date.
pub fn parse_cookie_date(s: &str) -> Option<i64> {
    let is_delim = |b: u8| {
        b == 0x09
            || (0x20..=0x2f).contains(&b)
            || (0x3b..=0x40).contains(&b)
            || (0x5b..=0x60).contains(&b)
            || (0x7b..=0x7e).contains(&b)
    };
    let mut time: Option<(u32, u32, u32)> = None;
    let mut day: Option<u32> = None;
    let mut month: Option<u32> = None;
    let mut year: Option<u32> = None;

    for token in s
        .as_bytes()
        .split(|&b| is_delim(b))
        .filter(|t| !t.is_empty())
    {
        if time.is_none()
            && let Some(t) = parse_time_token(token)
        {
            time = Some(t);
        } else if day.is_none()
            && let Some(d) = leading_number(token, 1, 2)
        {
            day = Some(d);
        } else if month.is_none()
            && let Some(m) = parse_month(token)
        {
            month = Some(m);
        } else if year.is_none()
            && let Some(y) = leading_number(token, 2, 4)
        {
            year = Some(y);
        }
    }

    let (hour, minute, second) = time?;
    let day = day?;
    let month = month?;
    let mut year = year?;
    if (70..=99).contains(&year) {
        year += 1900;
    } else if year <= 69 {
        year += 2000;
    }
    if !(1..=31).contains(&day) || year < 1601 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    if day > days_in_month(year, month) {
        return None;
    }
    let days = days_from_civil(i64::from(year), month, day);
    Some(days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))
}

/// `min..=max` leading digits, followed by nothing or a non-digit.
fn leading_number(token: &[u8], min: usize, max: usize) -> Option<u32> {
    let n = token.iter().take_while(|b| b.is_ascii_digit()).count();
    if n < min || n > max {
        return None;
    }
    std::str::from_utf8(&token[..n]).ok()?.parse().ok()
}

/// `hms-time = 1*2DIGIT ":" 1*2DIGIT ":" 1*2DIGIT ( non-digit *OCTET )`.
fn parse_time_token(token: &[u8]) -> Option<(u32, u32, u32)> {
    let mut fields = [0u32; 3];
    let mut rest = token;
    for (i, field) in fields.iter_mut().enumerate() {
        let n = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if !(1..=2).contains(&n) {
            return None;
        }
        *field = std::str::from_utf8(&rest[..n]).ok()?.parse().ok()?;
        rest = &rest[n..];
        if i < 2 {
            rest = rest.strip_prefix(b":")?;
        }
    }
    if rest.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some((fields[0], fields[1], fields[2]))
}

fn parse_month(token: &[u8]) -> Option<u32> {
    const MONTHS: [&[u8; 3]; 12] = [
        b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov",
        b"dec",
    ];
    let prefix = token.get(..3)?.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| m[..] == prefix[..])
        .map(|i| i as u32 + 1)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        },
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed "now" for deterministic expiry tests (2024-01-01).
    const NOW: u64 = 1_704_067_200;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid test URL")
    }

    fn sc(v: &str) -> Vec<(String, String)> {
        vec![("set-cookie".to_string(), v.to_string())]
    }

    fn jar_with(u: &str, cookies: &[&str]) -> CookieJar {
        let mut jar = CookieJar::new();
        let u = url(u);
        for c in cookies {
            jar.set_cookies_at(&u, &sc(c), NOW);
        }
        jar
    }

    fn header(jar: &CookieJar, u: &str) -> Option<String> {
        jar.cookie_header_at(&url(u), NOW)
    }

    #[test]
    fn parse_simple_set_cookie() {
        let u = url("http://example.com/path/page");
        let cookie = parse_set_cookie("session=abc123", &u, NOW).expect("should parse");
        assert_eq!(cookie.name, "session");
        assert_eq!(cookie.value, "abc123");
        assert_eq!(cookie.domain, "example.com");
        assert!(cookie.host_only);
        // RFC 6265 §5.1.4: up to, not including, the last '/'.
        assert_eq!(cookie.path, "/path");
        assert!(!cookie.secure);
        assert!(!cookie.http_only);
        assert_eq!(cookie.expires_at, None);
    }

    #[test]
    fn parse_full_set_cookie() {
        let u = url("https://example.com/");
        let cookie = parse_set_cookie(
            "id=42; Path=/; Domain=.example.com; Secure; HttpOnly; Max-Age=3600; SameSite=Lax",
            &u,
            NOW,
        )
        .expect("should parse");
        assert_eq!(cookie.name, "id");
        assert_eq!(cookie.value, "42");
        assert_eq!(cookie.domain, "example.com");
        assert!(!cookie.host_only);
        assert_eq!(cookie.path, "/");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.expires_at, Some(NOW + 3600));
        assert_eq!(cookie.same_site, Some(SameSite::Lax));
    }

    #[test]
    fn cookie_jar_stores_and_retrieves() {
        let jar = jar_with("http://example.com/app/index", &["token=xyz"]);
        assert_eq!(
            header(&jar, "http://example.com/app/index"),
            Some("token=xyz".into())
        );
    }

    #[test]
    fn cookie_jar_domain_mismatch() {
        let jar = jar_with("http://example.com/", &["a=1"]);
        assert!(header(&jar, "http://other.com/").is_none());
    }

    #[test]
    fn cookie_jar_subdomain_match() {
        let jar = jar_with("http://example.com/", &["a=1; Domain=.example.com"]);
        assert_eq!(header(&jar, "http://sub.example.com/"), Some("a=1".into()));
        // Domain-match is on label boundaries.
        assert!(header(&jar, "http://notexample.com/").is_none());
    }

    #[test]
    fn host_only_cookie_is_not_sent_to_subdomains() {
        let jar = jar_with("http://example.com/", &["a=1"]);
        assert_eq!(header(&jar, "http://example.com/x"), Some("a=1".into()));
        assert!(header(&jar, "http://evil.sub.example.com/").is_none());
        assert!(header(&jar, "http://sub.example.com/").is_none());
    }

    #[test]
    fn cookie_jar_path_mismatch() {
        let jar = jar_with("http://example.com/app/page", &["a=1; Path=/app/"]);
        assert!(header(&jar, "http://example.com/other/page").is_none());
    }

    #[test]
    fn relative_path_attribute_uses_the_default_path() {
        let jar = jar_with("http://example.com/app/page", &["a=1; Path=relative"]);
        assert_eq!(header(&jar, "http://example.com/app/x"), Some("a=1".into()));
        assert!(header(&jar, "http://example.com/other").is_none());
    }

    #[test]
    fn secure_cookie_not_sent_over_http() {
        let jar = jar_with("https://example.com/", &["s=1; Secure"]);
        assert_eq!(header(&jar, "https://example.com/"), Some("s=1".into()));
        assert!(header(&jar, "http://example.com/").is_none());
    }

    #[test]
    fn secure_cookie_from_http_is_refused() {
        let jar = jar_with("http://example.com/", &["s=1; Secure"]);
        assert!(jar.is_empty());
    }

    #[test]
    fn insecure_response_cannot_overwrite_a_secure_cookie() {
        let mut jar = jar_with("https://example.com/", &["sid=good; Secure; Path=/"]);
        jar.set_cookies_at(&url("http://example.com/"), &sc("sid=evil; Path=/"), NOW);
        assert_eq!(
            header(&jar, "https://example.com/"),
            Some("sid=good".into())
        );
    }

    #[test]
    fn registrable_domain_uses_the_psl_lite() {
        assert_eq!(registrable_domain("example.com"), "example.com");
        assert_eq!(registrable_domain("a.b.example.com"), "example.com");
        assert_eq!(registrable_domain("shop.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("me.github.io"), "me.github.io");
        assert_eq!(registrable_domain("localhost"), "localhost");
        assert_eq!(registrable_domain("127.0.0.1"), "127.0.0.1");
        assert_eq!(registrable_domain("[::1]"), "[::1]");
    }

    #[test]
    fn same_site_context_filters_cross_site_cookies() {
        let jar = jar_with(
            "https://bank.example.com/",
            &[
                "s=1; Path=/; SameSite=Strict",
                "l=1; Path=/; SameSite=Lax",
                "n=1; Path=/; SameSite=None; Secure",
                "d=1; Path=/",
            ],
        );
        let target = url("https://bank.example.com/x");
        let send = |initiator: Option<&str>, top: bool, method: &str| {
            jar.cookie_header_filtered(&target, NOW, |c| {
                SameSiteContext::new(initiator, top, method).allows(c, &target)
            })
        };
        let all = Some("s=1; l=1; n=1; d=1".to_string());
        // User-initiated and same-site (other subdomain) requests: all.
        assert_eq!(send(None, false, "GET"), all);
        assert_eq!(send(Some("https://www.example.com/"), false, "POST"), all);
        // Cross-site subresource: unrestricted cookies only.
        let loose = Some("n=1; d=1".to_string());
        assert_eq!(send(Some("https://evil.test/"), false, "GET"), loose);
        // Cross-site top-level GET adds Lax; POST does not.
        assert_eq!(
            send(Some("https://evil.test/"), true, "GET"),
            Some("l=1; n=1; d=1".to_string())
        );
        assert_eq!(send(Some("https://evil.test/"), true, "POST"), loose);
    }

    #[test]
    fn insecure_response_cannot_delete_a_secure_cookie() {
        // Regression: the expired-cookie deletion branch ran before the
        // Secure-shadowing check, so `Max-Age=0` over http wiped it.
        let mut jar = jar_with("https://example.com/", &["sid=good; Secure; Path=/"]);
        jar.set_cookies_at(
            &url("http://example.com/"),
            &sc("sid=; Max-Age=0; Path=/"),
            NOW,
        );
        assert_eq!(
            header(&jar, "https://example.com/"),
            Some("sid=good".into())
        );
        // The secure origin itself can still delete it.
        jar.set_cookies_at(
            &url("https://example.com/"),
            &sc("sid=; Max-Age=0; Path=/"),
            NOW,
        );
        assert_eq!(header(&jar, "https://example.com/"), None);
    }

    #[test]
    fn public_suffix_domains_are_refused() {
        // Single-label "supercookie" domain.
        assert!(jar_with("http://example.com/", &["a=1; Domain=com"]).is_empty());
        assert!(jar_with("http://example.com/", &["a=1; Domain=.com"]).is_empty());
        // Embedded multi-label suffix.
        assert!(jar_with("http://shop.example.co.uk/", &["a=1; Domain=co.uk"]).is_empty());
        assert!(jar_with("http://me.github.io/", &["a=1; Domain=github.io"]).is_empty());
        // The registrable domain itself is fine.
        let jar = jar_with("http://shop.example.co.uk/", &["a=1; Domain=example.co.uk"]);
        assert_eq!(
            header(&jar, "http://www.example.co.uk/"),
            Some("a=1".into())
        );
        // `Domain=` equal to a single-label host degrades to host-only.
        let jar = jar_with("http://localhost/", &["a=1; Domain=localhost"]);
        assert_eq!(header(&jar, "http://localhost/"), Some("a=1".into()));
        assert!(header(&jar, "http://x.localhost/").is_none());
    }

    #[test]
    fn domain_must_match_the_setting_host() {
        assert!(jar_with("http://example.com/", &["a=1; Domain=other.com"]).is_empty());
        assert!(jar_with("http://example.com/", &["a=1; Domain=sub.example.com"]).is_empty());
    }

    #[test]
    fn ip_hosts_cannot_set_a_domain() {
        assert!(jar_with("http://10.0.0.1/", &["a=1; Domain=0.0.1"]).is_empty());
        let jar = jar_with("http://10.0.0.1/", &["a=1; Domain=10.0.0.1"]);
        assert_eq!(header(&jar, "http://10.0.0.1/"), Some("a=1".into()));
    }

    #[test]
    fn max_age_zero_deletes_the_cookie() {
        let mut jar = jar_with("http://example.com/", &["sid=1; Path=/"]);
        assert!(header(&jar, "http://example.com/").is_some());
        jar.set_cookies_at(
            &url("http://example.com/"),
            &sc("sid=; Path=/; Max-Age=0"),
            NOW,
        );
        assert!(header(&jar, "http://example.com/").is_none());
        assert!(jar.is_empty());
    }

    #[test]
    fn past_expires_deletes_the_cookie() {
        let mut jar = jar_with("http://example.com/", &["sid=1; Path=/"]);
        jar.set_cookies_at(
            &url("http://example.com/"),
            &sc("sid=deleted; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"),
            NOW,
        );
        assert!(jar.is_empty());
    }

    #[test]
    fn max_age_wins_over_expires_and_cookies_expire() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies_at(
            &u,
            &sc("a=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=60"),
            NOW,
        );
        assert_eq!(jar.cookie_header_at(&u, NOW + 59), Some("a=1".into()));
        assert_eq!(jar.cookie_header_at(&u, NOW + 60), None);
    }

    #[test]
    fn lifetimes_are_capped() {
        let u = url("http://example.com/");
        let c = parse_set_cookie("a=1; Max-Age=999999999999", &u, NOW).unwrap();
        assert_eq!(c.expires_at, Some(NOW + MAX_LIFETIME_SECS));
    }

    #[test]
    fn cookie_dates_parse() {
        // 1994-11-06 08:49:37 UTC.
        const T: i64 = 784_111_777;
        assert_eq!(parse_cookie_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(T));
        assert_eq!(parse_cookie_date("Sunday, 06-Nov-94 08:49:37 GMT"), Some(T));
        assert_eq!(parse_cookie_date("Sun Nov  6 08:49:37 1994"), Some(T));
        assert_eq!(parse_cookie_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_cookie_date("Wed, 31 Dec 2025 23:59:59 GMT"),
            Some(1_767_225_599)
        );
        assert_eq!(
            parse_cookie_date("Fri, 29 Feb 2024 00:00:00 GMT"),
            Some(1_709_164_800)
        );
        assert_eq!(parse_cookie_date("Thu, 29 Feb 2023 00:00:00 GMT"), None);
        assert_eq!(parse_cookie_date("garbage"), None);
        assert_eq!(parse_cookie_date("Sun, 06 Nov 1994 25:00:00 GMT"), None);
        assert_eq!(parse_cookie_date("06 Nov 1500 08:49:37"), None);
    }

    #[test]
    fn control_characters_and_oversized_cookies_are_refused() {
        assert!(jar_with("http://example.com/", &["a=1\rInjected: x"]).is_empty());
        assert!(jar_with("http://example.com/", &["a\u{1}=1"]).is_empty());
        let big = format!("a={}", "x".repeat(MAX_COOKIE_SIZE));
        assert!(jar_with("http://example.com/", &[big.as_str()]).is_empty());
    }

    #[test]
    fn cookie_prefixes_are_enforced() {
        assert!(jar_with("https://example.com/", &["__Secure-a=1"]).is_empty());
        assert!(!jar_with("https://example.com/", &["__Secure-a=1; Secure"]).is_empty());
        assert!(jar_with("https://example.com/", &["__Host-a=1; Secure; Path=/app"]).is_empty());
        assert!(
            jar_with(
                "https://www.example.com/",
                &["__Host-a=1; Secure; Path=/; Domain=example.com"]
            )
            .is_empty()
        );
        assert!(!jar_with("https://example.com/", &["__Host-a=1; Secure; Path=/"]).is_empty());
    }

    #[test]
    fn per_domain_and_total_caps_evict_the_oldest() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        for i in 0..=MAX_COOKIES_PER_DOMAIN {
            jar.set_cookies_at(&u, &sc(&format!("c{i}=v")), NOW);
        }
        assert_eq!(jar.len(), MAX_COOKIES_PER_DOMAIN);
        let h = jar.cookie_header_at(&u, NOW).unwrap();
        assert!(!h.split("; ").any(|p| p == "c0=v"), "oldest evicted: {h}");
        assert!(h.contains(&format!("c{MAX_COOKIES_PER_DOMAIN}=v")));

        let mut jar = CookieJar::new();
        for d in 0..(MAX_COOKIES / 10 + 1) {
            let u = url(&format!("http://d{d}.test/"));
            for i in 0..10 {
                jar.set_cookies_at(&u, &sc(&format!("c{i}=v")), NOW);
            }
        }
        assert!(jar.len() <= MAX_COOKIES);
        assert!(jar.cookie_header_at(&url("http://d0.test/"), NOW).is_none());
    }

    #[test]
    fn cookie_jar_replaces_duplicate() {
        let mut jar = jar_with("http://example.com/", &["a=1", "b=2"]);
        jar.set_cookies_at(&url("http://example.com/"), &sc("a=3"), NOW);
        // Replacement keeps the original position.
        assert_eq!(header(&jar, "http://example.com/"), Some("a=3; b=2".into()));
    }

    #[test]
    fn longer_paths_are_listed_first() {
        let jar = jar_with(
            "http://example.com/a/b/c",
            &["root=1; Path=/", "deep=2; Path=/a/b", "mid=3; Path=/a"],
        );
        assert_eq!(
            header(&jar, "http://example.com/a/b/c"),
            Some("deep=2; mid=3; root=1".into())
        );
    }

    #[test]
    fn multiple_cookies_in_header() {
        let jar = jar_with("http://example.com/", &["a=1", "b=2"]);
        let h = header(&jar, "http://example.com/").expect("should have cookies");
        assert_eq!(h, "a=1; b=2");
    }

    #[test]
    fn non_http_urls_get_no_cookies() {
        let jar = jar_with("http://example.com/", &["a=1"]);
        assert!(header(&jar, "gemini://example.com/").is_none());
        assert!(jar_with("gemini://example.com/", &["a=1"]).is_empty());
    }
}
