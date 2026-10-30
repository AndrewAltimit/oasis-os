//! Content Security Policy (CSP Level 3 subset): parsing and checks.
//!
//! ## Enforced by the browser
//!
//! - `script-src-elem` → `script-src` → `default-src`: inline `<script>`
//!   (matching nonce, `'sha256-…'` hash, or `'unsafe-inline'`).
//! - `script-src-attr` → `script-src` → `default-src`: inline event
//!   handlers (`onclick=…`), which run only under `'unsafe-inline'`.
//! - `style-src-elem` → `style-src` → `default-src`: inline `<style>`
//!   (nonce / hash / `'unsafe-inline'`), `<link rel=stylesheet>` and
//!   `@import` URLs.
//! - `style-src-attr` → `style-src` → `default-src`: parser-inserted
//!   `style=""` attributes (`'unsafe-inline'`).
//! - `connect-src` → `default-src`: script `fetch()`.
//!
//! `img-src` is parsed but deliberately not enforced (images don't run
//! code; see `widget/images.rs`). External `<script src>` is never
//! loaded at all, so script URL checks have nothing to gate. Every
//! header (and every comma-separated policy inside one) is enforced, and
//! a load must pass all of them. `Content-Security-Policy-Report-Only`
//! and `<meta http-equiv>` policies are ignored; violations are logged
//! (`log::warn!`), never reported.
//!
//! ## Source expressions
//!
//! `'self'`, `'none'`, `*`, `'unsafe-inline'`, `'nonce-…'`, `'sha256-…'`
//! (base64 or base64url; `'sha384-…'` / `'sha512-…'` are recognised but
//! never match), `'unsafe-hashes'`, `'strict-dynamic'`, scheme sources
//! (`https:`, `data:`, `blob:`) and host sources
//! (`[scheme://]host[:port][/path]`, `*.` subdomain wildcards, `:*`
//! ports). As in CSP3, a nonce or hash in a list disables
//! `'unsafe-inline'` for that list, `'self'` and `http:` also match the
//! `https:` upgrade, and `*.example.com` does not match `example.com`.

use super::{Url, default_port};

/// A parsed Content Security Policy: one or more policies, each of which
/// must allow a load.
#[derive(Debug, Clone, Default)]
pub struct CspPolicy {
    policies: Vec<DirectiveSet>,
}

/// The directives of a single policy. `None` = directive absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DirectiveSet {
    /// `default-src` (fallback for every fetch directive).
    pub default_src: Option<Vec<CspSource>>,
    /// `script-src`.
    pub script_src: Option<Vec<CspSource>>,
    /// `script-src-elem` (`<script>` elements).
    pub script_src_elem: Option<Vec<CspSource>>,
    /// `script-src-attr` (inline event handlers).
    pub script_src_attr: Option<Vec<CspSource>>,
    /// `style-src`.
    pub style_src: Option<Vec<CspSource>>,
    /// `style-src-elem` (`<style>`, `<link rel=stylesheet>`).
    pub style_src_elem: Option<Vec<CspSource>>,
    /// `style-src-attr` (`style=""` attributes).
    pub style_src_attr: Option<Vec<CspSource>>,
    /// `img-src` (parsed, not enforced).
    pub img_src: Option<Vec<CspSource>>,
    /// `connect-src` (fetch, XHR, WebSocket).
    pub connect_src: Option<Vec<CspSource>>,
}

/// Supported hash algorithms of `'<alg>-<digest>'` sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    /// `'sha256-…'` (checked).
    Sha256,
    /// `'sha384-…'` (recognised, never matches).
    Sha384,
    /// `'sha512-…'` (recognised, never matches).
    Sha512,
}

/// A single CSP source expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CspSource {
    /// `'self'` -- same origin (or its `https:` upgrade).
    SelF,
    /// `'none'` -- matches nothing.
    None,
    /// `*` -- any network-scheme URL.
    Wildcard,
    /// `'unsafe-inline'`.
    UnsafeInline,
    /// `'unsafe-hashes'` (lets hashes allow attribute handlers).
    UnsafeHashes,
    /// `'strict-dynamic'` (disables `'unsafe-inline'` for scripts).
    StrictDynamic,
    /// `'nonce-<value>'`.
    Nonce(String),
    /// `'<alg>-<base64 digest>'` with the digest decoded.
    Hash(HashAlg, Vec<u8>),
    /// A scheme source such as `https:` or `data:` (scheme lower-cased,
    /// without the colon).
    Scheme(String),
    /// A host source such as `https://cdn.example.com:8443/static/`.
    Host(HostSource),
    /// A keyword we accept but that never matches a URL
    /// (`'unsafe-eval'`, `'report-sample'`, ...), or an unparseable token.
    Other(String),
}

/// `[scheme://]host[:port][/path]`.
#[derive(Debug, Clone, PartialEq)]
pub struct HostSource {
    /// Required scheme, lower-cased.
    pub scheme: Option<String>,
    /// Lower-cased host; may start with `*.`.
    pub host: String,
    /// `None` = scheme default, `Some(None)` = `:*`, `Some(Some(p))`.
    pub port: Option<Option<u16>>,
    /// Path prefix (ends with `/`) or exact path.
    pub path: Option<String>,
}

/// Resource types for CSP URL checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspResourceType {
    /// Image sub-resources (`<img>`).
    Image,
    /// Stylesheets (`<link rel=stylesheet>`, `@import`).
    Style,
    /// Scripts (`<script src>`).
    Script,
    /// Connections (XHR, fetch, WebSocket).
    Connect,
    /// Other or unknown resource type.
    Default,
}

/// Inline content kinds for [`CspPolicy::allows_inline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    /// An inline `<script>` element.
    ScriptElement,
    /// An inline event-handler attribute (`onclick="…"`).
    ScriptAttribute,
    /// An inline `<style>` element.
    StyleElement,
    /// A `style=""` attribute.
    StyleAttribute,
}

/// Parse one CSP header value. A comma separates several policies in
/// one header (CSP3 §2.2.1); each must allow a load.
pub fn parse_csp(header: &str) -> CspPolicy {
    let policies = header
        .split(',')
        .map(parse_directive_set)
        .filter(DirectiveSet::is_active)
        .collect();
    CspPolicy { policies }
}

/// Combine every `Content-Security-Policy` header of a response.
/// Returns `None` when the response has no active policy.
pub fn parse_csp_headers(headers: &[(String, String)]) -> Option<CspPolicy> {
    let policies: Vec<DirectiveSet> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-security-policy"))
        .flat_map(|(_, v)| parse_csp(v).policies)
        .collect();
    (!policies.is_empty()).then_some(CspPolicy { policies })
}

fn parse_directive_set(policy: &str) -> DirectiveSet {
    let mut set = DirectiveSet::default();
    for directive in policy.split(';') {
        let mut parts = directive.split_ascii_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        let slot = match name.to_ascii_lowercase().as_str() {
            "default-src" => &mut set.default_src,
            "script-src" => &mut set.script_src,
            "script-src-elem" => &mut set.script_src_elem,
            "script-src-attr" => &mut set.script_src_attr,
            "style-src" => &mut set.style_src,
            "style-src-elem" => &mut set.style_src_elem,
            "style-src-attr" => &mut set.style_src_attr,
            "img-src" => &mut set.img_src,
            "connect-src" => &mut set.connect_src,
            _ => continue,
        };
        // Duplicate directives are ignored (CSP3 §2.2.1 step 2.4).
        if slot.is_none() {
            *slot = Some(parts.map(parse_source).collect());
        }
    }
    set
}

/// Parse a single CSP source token.
fn parse_source(token: &str) -> CspSource {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "'self'" => return CspSource::SelF,
        "'none'" => return CspSource::None,
        "*" => return CspSource::Wildcard,
        "'unsafe-inline'" => return CspSource::UnsafeInline,
        "'unsafe-hashes'" => return CspSource::UnsafeHashes,
        "'strict-dynamic'" => return CspSource::StrictDynamic,
        _ => {},
    }
    if let Some(inner) = token.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        // Keyword values (nonce, digest) are case-sensitive.
        let inner_lower = inner.to_ascii_lowercase();
        if inner_lower.starts_with("nonce-") && inner.len() > 6 {
            return CspSource::Nonce(inner[6..].to_string());
        }
        for (prefix, alg) in [
            ("sha256-", HashAlg::Sha256),
            ("sha384-", HashAlg::Sha384),
            ("sha512-", HashAlg::Sha512),
        ] {
            if inner_lower.starts_with(prefix)
                && let Some(digest) = base64_decode_any(&inner[prefix.len()..])
            {
                return CspSource::Hash(alg, digest);
            }
        }
        return CspSource::Other(token.to_string());
    }
    // Scheme source: `scheme:` with nothing after the colon.
    if let Some(scheme) = lower.strip_suffix(':')
        && is_scheme(scheme)
    {
        return CspSource::Scheme(scheme.to_string());
    }
    parse_host_source(&lower).map_or_else(|| CspSource::Other(token.to_string()), CspSource::Host)
}

fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `[scheme "://"] host [":" port] [path]` (input already lower-cased).
fn parse_host_source(s: &str) -> Option<HostSource> {
    let (scheme, rest) = match s.find("://") {
        Some(i) if is_scheme(&s[..i]) => (Some(s[..i].to_string()), &s[i + 3..]),
        Some(_) => return None,
        None => (None, s),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], Some(rest[i..].to_string())),
        None => (rest, None),
    };
    let (host, port) = match authority.rfind(':') {
        Some(i) if !authority.starts_with('[') || authority[..i].ends_with(']') => {
            let p = &authority[i + 1..];
            let port = if p == "*" {
                Some(None)
            } else {
                Some(Some(p.parse::<u16>().ok()?))
            };
            (&authority[..i], port)
        },
        _ => (authority, None),
    };
    let valid_host = host == "*"
        || host
            .strip_prefix("*.")
            .unwrap_or(host)
            .split('.')
            .all(|label| {
                !label.is_empty()
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '[' | ']' | ':'))
            });
    if host.is_empty() || !valid_host {
        return None;
    }
    Some(HostSource {
        scheme,
        host: host.to_string(),
        port,
        path: path.filter(|p| p != "/"),
    })
}

impl DirectiveSet {
    fn is_active(&self) -> bool {
        self.default_src.is_some()
            || self.script_src.is_some()
            || self.script_src_elem.is_some()
            || self.script_src_attr.is_some()
            || self.style_src.is_some()
            || self.style_src_elem.is_some()
            || self.style_src_attr.is_some()
            || self.img_src.is_some()
            || self.connect_src.is_some()
    }

    fn for_resource(&self, kind: CspResourceType) -> Option<&Vec<CspSource>> {
        match kind {
            CspResourceType::Script => self
                .script_src_elem
                .as_ref()
                .or(self.script_src.as_ref())
                .or(self.default_src.as_ref()),
            CspResourceType::Style => self
                .style_src_elem
                .as_ref()
                .or(self.style_src.as_ref())
                .or(self.default_src.as_ref()),
            CspResourceType::Image => self.img_src.as_ref().or(self.default_src.as_ref()),
            CspResourceType::Connect => self.connect_src.as_ref().or(self.default_src.as_ref()),
            CspResourceType::Default => self.default_src.as_ref(),
        }
    }

    fn for_inline(&self, kind: InlineKind) -> Option<&Vec<CspSource>> {
        let (specific, general) = match kind {
            InlineKind::ScriptElement => (&self.script_src_elem, &self.script_src),
            InlineKind::ScriptAttribute => (&self.script_src_attr, &self.script_src),
            InlineKind::StyleElement => (&self.style_src_elem, &self.style_src),
            InlineKind::StyleAttribute => (&self.style_src_attr, &self.style_src),
        };
        specific
            .as_ref()
            .or(general.as_ref())
            .or(self.default_src.as_ref())
    }
}

impl CspPolicy {
    /// Check whether a resource URL is allowed for a given resource type.
    ///
    /// `page_url` is the URL of the page that declared the policy (for
    /// `'self'` and scheme-less host sources). Returns `true` if the load
    /// is permitted by every policy.
    pub fn allows(
        &self,
        resource_url: &str,
        page_url: &str,
        resource_type: CspResourceType,
    ) -> bool {
        let page = Url::parse(page_url);
        let Some(resource) = Url::parse(resource_url) else {
            // Unparseable URLs (`data:`, `blob:`) only pass scheme sources.
            return self.policies.iter().all(|p| {
                p.for_resource(resource_type).is_none_or(|list| {
                    let scheme = resource_url
                        .split_once(':')
                        .map(|(s, _)| s.to_ascii_lowercase())
                        .unwrap_or_default();
                    list.iter()
                        .any(|s| matches!(s, CspSource::Scheme(x) if *x == scheme))
                })
            });
        };
        self.policies.iter().all(|p| {
            p.for_resource(resource_type)
                .is_none_or(|list| url_matches_list(&resource, page.as_ref(), list))
        })
    }

    /// Whether inline content of `kind` may run / apply.
    ///
    /// `nonce` is the element's `nonce` attribute (ignored for attribute
    /// kinds), `content` its text (for hash sources).
    pub fn allows_inline(&self, kind: InlineKind, nonce: Option<&str>, content: &str) -> bool {
        let mut digest: Option<[u8; 32]> = None;
        self.policies.iter().all(|p| {
            p.for_inline(kind)
                .is_none_or(|list| inline_matches_list(kind, nonce, content, list, &mut digest))
        })
    }

    /// Returns `true` if this policy has any directives set.
    pub fn is_active(&self) -> bool {
        !self.policies.is_empty()
    }

    /// The individual policies (one per header / comma-separated part).
    pub fn policies(&self) -> &[DirectiveSet] {
        &self.policies
    }
}

/// CSP3 §6.7.3.3 "does element match source list for type and source".
fn inline_matches_list(
    kind: InlineKind,
    nonce: Option<&str>,
    content: &str,
    list: &[CspSource],
    digest: &mut Option<[u8; 32]>,
) -> bool {
    let is_script = matches!(
        kind,
        InlineKind::ScriptElement | InlineKind::ScriptAttribute
    );
    let is_attr = matches!(
        kind,
        InlineKind::ScriptAttribute | InlineKind::StyleAttribute
    );
    let has_nonce_or_hash = list
        .iter()
        .any(|s| matches!(s, CspSource::Nonce(_) | CspSource::Hash(..)));
    let strict_dynamic = is_script && list.contains(&CspSource::StrictDynamic);
    if list.contains(&CspSource::UnsafeInline) && !has_nonce_or_hash && !strict_dynamic {
        return true;
    }
    if !is_attr
        && let Some(n) = nonce.filter(|n| !n.is_empty())
        && list
            .iter()
            .any(|s| matches!(s, CspSource::Nonce(v) if v == n))
    {
        return true;
    }
    if is_attr && !list.contains(&CspSource::UnsafeHashes) {
        return false;
    }
    list.iter().any(|s| match s {
        CspSource::Hash(HashAlg::Sha256, want) => {
            let got = digest.get_or_insert_with(|| sha256(content.as_bytes()));
            want.as_slice() == got.as_slice()
        },
        _ => false,
    })
}

/// CSP3 §6.7.2.5 "does url match source list".
fn url_matches_list(url: &Url, page: Option<&Url>, list: &[CspSource]) -> bool {
    list.iter().any(|source| match source {
        CspSource::Wildcard => {
            matches!(url.scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp")
                || page.is_some_and(|p| p.scheme == url.scheme)
        },
        CspSource::SelF => page.is_some_and(|p| {
            p.same_origin(url)
                // http://host -> https://host on default ports.
                || (p.scheme == "http"
                    && url.scheme == "https"
                    && p.host.eq_ignore_ascii_case(&url.host)
                    && p.port.is_none_or(|port| port == 80)
                    && url.port.is_none_or(|port| port == 443))
        }),
        CspSource::Scheme(s) => scheme_matches(s, &url.scheme),
        CspSource::Host(h) => host_source_matches(h, url, page),
        _ => false,
    })
}

/// `http:` also allows `https:`, `ws:` also `wss:` (CSP3 §6.7.2.8).
fn scheme_matches(expr: &str, scheme: &str) -> bool {
    expr == scheme || (expr == "http" && scheme == "https") || (expr == "ws" && scheme == "wss")
}

fn host_source_matches(h: &HostSource, url: &Url, page: Option<&Url>) -> bool {
    let scheme_ok = match &h.scheme {
        Some(s) => scheme_matches(s, &url.scheme),
        // Scheme-less sources inherit the page's scheme (with upgrade).
        None => match page.map(|p| p.scheme.as_str()) {
            Some(ps) if !ps.is_empty() => scheme_matches(ps, &url.scheme),
            _ => matches!(url.scheme.as_str(), "http" | "https"),
        },
    };
    if !scheme_ok || url.host.is_empty() {
        return false;
    }
    let host = url.host.to_ascii_lowercase();
    let host_ok = match h.host.strip_prefix("*.") {
        Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
        None => h.host == "*" || h.host == host,
    };
    if !host_ok {
        return false;
    }
    let effective = url.port.or_else(|| default_port(&url.scheme));
    let port_ok = match h.port {
        Some(None) => true,
        Some(Some(p)) => {
            Some(p) == effective
                // `http://host:80` also matches the upgraded `https://host`.
                || (p == 80 && url.scheme == "https" && effective == Some(443))
        },
        // No port in the source: the URL must use its scheme's default
        // (which also covers the `http:80` -> `https:443` upgrade).
        None => effective == default_port(&url.scheme),
    };
    if !port_ok {
        return false;
    }
    match &h.path {
        None => true,
        Some(p) if p.ends_with('/') => url.path.starts_with(p.as_str()),
        Some(p) => url.path == *p,
    }
}

// -------------------------------------------------------------------
// Base64 + SHA-256 (for hash sources)
// -------------------------------------------------------------------

/// Decode standard or URL-safe base64, padding optional.
fn base64_decode_any(input: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let bytes = input.trim_end_matches('=').as_bytes();
    if bytes.is_empty() || bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            acc |= val(b)? << (18 - 6 * i);
        }
        let n = chunk.len() * 6 / 8;
        out.extend_from_slice(&acc.to_be_bytes()[1..=n]);
    }
    Some(out)
}

/// SHA-256 (FIPS 180-4) of `data`.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = word.to_be_bytes();
    }
    out
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn first(policy: &CspPolicy) -> &DirectiveSet {
        &policy.policies()[0]
    }

    fn allows(csp: &str, resource: &str, page: &str, kind: CspResourceType) -> bool {
        parse_csp(csp).allows(resource, page, kind)
    }

    #[test]
    fn parse_empty_header() {
        assert!(!parse_csp("").is_active());
        assert!(!parse_csp("unknown-directive x").is_active());
    }

    #[test]
    fn parse_default_src_none() {
        let policy = parse_csp("default-src 'none'");
        assert!(policy.is_active());
        assert_eq!(first(&policy).default_src, Some(vec![CspSource::None]));
    }

    #[test]
    fn parse_source_kinds() {
        let policy = parse_csp(
            "script-src 'self' 'nonce-AbC=' 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=' \
             https: data: *.cdn.test:* https://x.test/static/ 'unsafe-eval'",
        );
        let list = first(&policy).script_src.clone().unwrap();
        assert_eq!(list[0], CspSource::SelF);
        // Nonce values keep their case.
        assert_eq!(list[1], CspSource::Nonce("AbC=".into()));
        assert!(matches!(&list[2], CspSource::Hash(HashAlg::Sha256, d) if d.len() == 32));
        assert_eq!(list[3], CspSource::Scheme("https".into()));
        assert_eq!(list[4], CspSource::Scheme("data".into()));
        assert!(
            matches!(&list[5], CspSource::Host(h) if h.host == "*.cdn.test"
            && h.port == Some(None))
        );
        assert!(
            matches!(&list[6], CspSource::Host(h) if h.scheme.as_deref() == Some("https")
            && h.path.as_deref() == Some("/static/"))
        );
        assert!(matches!(&list[7], CspSource::Other(_)));
    }

    #[test]
    fn duplicate_directives_are_ignored() {
        let policy = parse_csp("default-src 'self'; default-src *");
        assert_eq!(first(&policy).default_src, Some(vec![CspSource::SelF]));
    }

    #[test]
    fn allows_wildcard() {
        assert!(allows(
            "default-src *",
            "http://evil.com/script.js",
            "http://example.com/page",
            CspResourceType::Default,
        ));
        // `*` does not cover data: URLs.
        assert!(!allows(
            "style-src *",
            "data:text/css,a{}",
            "http://example.com/",
            CspResourceType::Style
        ));
    }

    #[test]
    fn blocks_none() {
        assert!(!allows(
            "default-src 'none'",
            "http://example.com/image.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
    }

    #[test]
    fn self_matches_normalized_origin_and_upgrade() {
        let csp = "default-src 'self'";
        let kind = CspResourceType::Style;
        assert!(allows(
            csp,
            "http://example.com/s.css",
            "http://example.com/p",
            kind
        ));
        // Case and default port don't create a new origin (F15).
        assert!(allows(
            csp,
            "https://EXAMPLE.com:443/s.css",
            "https://example.com/p",
            kind
        ));
        // http page, https resource on the same host: allowed upgrade.
        assert!(allows(
            csp,
            "https://example.com/s.css",
            "http://example.com/p",
            kind
        ));
        assert!(!allows(
            csp,
            "http://example.com/s.css",
            "https://example.com/p",
            kind
        ));
        assert!(!allows(
            csp,
            "http://other.com/style.css",
            "http://example.com/page",
            kind
        ));
    }

    #[test]
    fn host_sources() {
        let kind = CspResourceType::Image;
        let page = "https://example.com/page";
        assert!(allows(
            "img-src https://cdn.example.com",
            "https://cdn.example.com/a",
            page,
            kind
        ));
        assert!(!allows(
            "img-src https://cdn.example.com",
            "https://evil.com/a",
            page,
            kind
        ));
        // Scheme-less source inherits the page scheme.
        assert!(allows("img-src cdn.test", "https://cdn.test/a", page, kind));
        assert!(!allows("img-src cdn.test", "http://cdn.test/a", page, kind));
        // `*.` matches subdomains only.
        assert!(allows(
            "img-src *.example.com",
            "https://a.b.example.com/x",
            page,
            kind
        ));
        assert!(!allows(
            "img-src *.example.com",
            "https://example.com/x",
            page,
            kind
        ));
        // Ports.
        assert!(!allows(
            "img-src https://cdn.test",
            "https://cdn.test:8443/a",
            page,
            kind
        ));
        assert!(allows(
            "img-src https://cdn.test:*",
            "https://cdn.test:8443/a",
            page,
            kind
        ));
        assert!(allows(
            "img-src https://cdn.test:8443",
            "https://cdn.test:8443/a",
            page,
            kind
        ));
        // Paths.
        let csp = "img-src https://cdn.test/static/";
        assert!(allows(csp, "https://cdn.test/static/a.png", page, kind));
        assert!(!allows(csp, "https://cdn.test/other/a.png", page, kind));
        let csp = "img-src https://cdn.test/logo.png";
        assert!(allows(csp, "https://cdn.test/logo.png", page, kind));
        assert!(!allows(csp, "https://cdn.test/logo.png2", page, kind));
    }

    #[test]
    fn scheme_sources() {
        let kind = CspResourceType::Style;
        let page = "https://example.com/";
        assert!(allows(
            "style-src https:",
            "https://any.test/a.css",
            page,
            kind
        ));
        assert!(!allows(
            "style-src https:",
            "http://any.test/a.css",
            page,
            kind
        ));
        assert!(allows(
            "style-src http:",
            "https://any.test/a.css",
            page,
            kind
        ));
        assert!(allows("style-src data:", "data:text/css,a{}", page, kind));
    }

    #[test]
    fn directive_fallbacks() {
        let csp = "default-src 'self'; script-src 'none'; style-src-elem https://cdn.test";
        let page = "http://example.com/page";
        assert!(allows(
            csp,
            "http://example.com/img.png",
            page,
            CspResourceType::Image
        ));
        assert!(!allows(
            csp,
            "http://example.com/app.js",
            page,
            CspResourceType::Script
        ));
        assert!(allows(
            csp,
            "https://cdn.test/a.css",
            page,
            CspResourceType::Style
        ));
        assert!(!allows(
            csp,
            "http://example.com/a.css",
            page,
            CspResourceType::Style
        ));
        assert!(allows(
            "",
            "http://any.com/x",
            page,
            CspResourceType::Default
        ));
    }

    #[test]
    fn every_policy_must_allow() {
        // Two policies in one header (comma) and across headers.
        let policy = parse_csp("default-src *, connect-src 'self'");
        assert_eq!(policy.policies().len(), 2);
        assert!(!policy.allows(
            "http://evil.test/",
            "http://a.test/",
            CspResourceType::Connect
        ));
        assert!(policy.allows(
            "http://a.test/x",
            "http://a.test/",
            CspResourceType::Connect
        ));

        let headers = vec![
            (
                "content-security-policy".to_string(),
                "connect-src *".to_string(),
            ),
            (
                "Content-Security-Policy".to_string(),
                "connect-src 'none'".to_string(),
            ),
            (
                "content-security-policy-report-only".to_string(),
                "default-src *".to_string(),
            ),
        ];
        let policy = parse_csp_headers(&headers).unwrap();
        assert_eq!(policy.policies().len(), 2);
        assert!(!policy.allows("http://a.test/", "http://a.test/", CspResourceType::Connect));
        assert!(parse_csp_headers(&[]).is_none());
    }

    #[test]
    fn inline_scripts_need_unsafe_inline_nonce_or_hash() {
        let k = InlineKind::ScriptElement;
        assert!(parse_csp("img-src *").allows_inline(k, None, "x()"));
        assert!(!parse_csp("script-src 'self'").allows_inline(k, None, "x()"));
        assert!(!parse_csp("default-src 'self'").allows_inline(k, None, "x()"));
        assert!(parse_csp("script-src 'unsafe-inline'").allows_inline(k, None, "x()"));

        let csp = parse_csp("script-src 'nonce-r4nd0m' 'unsafe-inline'");
        assert!(csp.allows_inline(k, Some("r4nd0m"), "x()"));
        // A nonce in the list disables 'unsafe-inline'.
        assert!(!csp.allows_inline(k, None, "x()"));
        assert!(!csp.allows_inline(k, Some("wrong"), "x()"));
        assert!(!csp.allows_inline(k, Some("R4ND0M"), "x()"));
        // Nonces never apply to attributes.
        assert!(!csp.allows_inline(InlineKind::ScriptAttribute, Some("r4nd0m"), "x()"));
    }

    #[test]
    fn hash_sources_match_sha256() {
        // echo -n "alert('Hello, world.');" | openssl dgst -sha256 -binary | base64
        let csp = parse_csp("script-src 'sha256-qznLcsROx4GACP2dm0UCKCzCG+HiZ1guq6ZZDob/Tng='");
        let k = InlineKind::ScriptElement;
        assert!(csp.allows_inline(k, None, "alert('Hello, world.');"));
        assert!(!csp.allows_inline(k, None, "alert('Hello, world!');"));
        // base64url spelling of the same digest.
        let csp = parse_csp("script-src 'sha256-qznLcsROx4GACP2dm0UCKCzCG-HiZ1guq6ZZDob_Tng'");
        assert!(csp.allows_inline(k, None, "alert('Hello, world.');"));
        // Attributes only match hashes with 'unsafe-hashes'.
        let h = "'sha256-qznLcsROx4GACP2dm0UCKCzCG+HiZ1guq6ZZDob/Tng='";
        let a = InlineKind::ScriptAttribute;
        let content = "alert('Hello, world.');";
        assert!(!parse_csp(&format!("script-src {h}")).allows_inline(a, None, content));
        let csp = parse_csp(&format!("script-src 'unsafe-hashes' {h}"));
        assert!(csp.allows_inline(a, None, content));
    }

    #[test]
    fn strict_dynamic_disables_unsafe_inline_for_scripts_only() {
        let csp =
            parse_csp("script-src 'strict-dynamic' 'unsafe-inline'; style-src 'unsafe-inline'");
        assert!(!csp.allows_inline(InlineKind::ScriptElement, None, "x"));
        assert!(csp.allows_inline(InlineKind::StyleElement, None, "a{}"));
    }

    #[test]
    fn style_attr_and_elem_directives() {
        let csp = parse_csp("style-src 'self'; style-src-attr 'unsafe-inline'");
        assert!(csp.allows_inline(InlineKind::StyleAttribute, None, "color:red"));
        assert!(!csp.allows_inline(InlineKind::StyleElement, None, "a{}"));
    }

    #[test]
    fn sha256_known_vectors() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
