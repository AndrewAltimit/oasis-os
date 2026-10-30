//! URL parsing and reference resolution (a WHATWG-flavoured subset of
//! RFC 3986): [`Url`], origins, request targets and referrer stripping.

use std::fmt;

/// A parsed URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    /// Scheme component (e.g. `"http"`, `"vfs"`, `"gemini"`).
    pub scheme: String,
    /// Host component (e.g. `"example.com"`). For `vfs://` URLs this is
    /// the first path segment. Lower-cased for network schemes
    /// (`http`, `https`, `gemini`, `ws`, `wss`, `ftp`); never contains
    /// userinfo.
    pub host: String,
    /// Optional explicit port number. A network scheme's default port
    /// (`http:80`, `https:443`, `gemini:1965`, ...) is elided to `None`.
    pub port: Option<u16>,
    /// `user[:password]` from the authority (`http://user:pw@host/`),
    /// kept apart from `host` so it can never be mistaken for one. It is
    /// not sent anywhere (no HTTP auth support) and is omitted by
    /// `Display`.
    pub userinfo: Option<String>,
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
                userinfo: None,
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

    /// Internal helper: parse `[userinfo@]host[:port]/path?query#fragment`
    /// after the scheme has been stripped.
    ///
    /// For network schemes ([`is_special_scheme`]) the authority is
    /// normalized the way WHATWG URL parsing does: `\` counts as a path
    /// separator, userinfo is split off at the last `@`, the host is
    /// lower-cased, bracketed IPv6 literals keep their colons, a
    /// non-numeric port makes the URL invalid, and the scheme's default
    /// port is elided. Other schemes (`vfs`, `about`, ...) keep the
    /// lenient legacy parse, since their "host" is a case-sensitive VFS
    /// path segment.
    fn parse_authority_and_path(scheme: &str, rest: &str) -> Option<Url> {
        let scheme = scheme.to_ascii_lowercase();
        let special = is_special_scheme(&scheme);
        let rest_owned;
        let rest = if special && rest.contains('\\') {
            rest_owned = rest.replace('\\', "/");
            rest_owned.as_str()
        } else {
            rest
        };

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
        let path = if path.is_empty() { "/" } else { path };

        let (userinfo, host, port) = if special {
            let (userinfo, hostport) = match authority.rfind('@') {
                Some(i) => (Some(authority[..i].to_string()), &authority[i + 1..]),
                None => (None, authority),
            };
            let (host, port_str) = if hostport.starts_with('[') {
                // Bracketed IPv6 literal: the port follows `]:`.
                let close = hostport.find(']')?;
                let after = &hostport[close + 1..];
                let port_str = match after.strip_prefix(':') {
                    Some(p) => Some(p),
                    None if after.is_empty() => None,
                    None => return None,
                };
                (&hostport[..=close], port_str)
            } else {
                match hostport.rfind(':') {
                    Some(i) => (&hostport[..i], Some(&hostport[i + 1..])),
                    None => (hostport, None),
                }
            };
            let port = match port_str {
                None | Some("") => None,
                Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => Some(p.parse::<u16>().ok()?),
                Some(_) => return None,
            };
            let port = port.filter(|&p| Some(p) != default_port(&scheme));
            (userinfo, host.to_ascii_lowercase(), port)
        } else {
            // Legacy lenient parse: a trailing `:digits` is the port,
            // anything else stays part of the host.
            let (host, port) = match authority.rfind(':') {
                Some(i) => match authority[i + 1..].parse::<u16>() {
                    Ok(p) => (&authority[..i], Some(p)),
                    Err(_) => (authority, None),
                },
                None => (authority, None),
            };
            (None, host.to_string(), port)
        };

        Some(Url {
            scheme,
            host,
            port,
            userinfo,
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

        // Absolute URL (starts with a scheme) -- return as-is. Only a
        // leading `scheme:` counts: a relative reference may carry
        // `://` in its query (`login?next=http://x/`) and must still
        // resolve against the base.
        if has_scheme(relative) {
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
        if let Some(rest) = relative.strip_prefix('?') {
            // `?q=1#frag`: the fragment belongs in `fragment`, not the query.
            let (query, fragment) = match rest.find('#') {
                Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
                None => (rest, None),
            };
            let mut resolved = self.clone();
            resolved.query = Some(query.to_string());
            resolved.fragment = fragment;
            return Some(resolved);
        }

        // Absolute path.
        if relative.starts_with('/') {
            // Split off query and fragment; dot segments are removed
            // here too (`/a/../b` is `/b`).
            let (path, query, fragment) = split_path_query_fragment(relative);
            return Some(Url {
                scheme: self.scheme.clone(),
                host: self.host.clone(),
                port: self.port,
                userinfo: self.userinfo.clone(),
                path: resolve_path("/", &path),
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
            userinfo: self.userinfo.clone(),
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

    /// The serialized origin (`scheme://host[:port]`), or `None` for a
    /// URL without a scheme or host (an opaque origin).
    ///
    /// This is the one origin definition used by CSP `'self'`, `fetch()`
    /// same-origin / CORS checks, Web Storage partitioning and the
    /// redirect header policy. The host is lower-cased and a default
    /// port elided (both already true for parsed network URLs; repeated
    /// here so hand-built `Url`s compare the same way), so
    /// `https://Example.com:443` and `https://example.com` are one origin.
    pub fn origin(&self) -> Option<String> {
        if self.scheme.is_empty() || self.host.is_empty() {
            return None;
        }
        let host = if is_special_scheme(&self.scheme) {
            self.host.to_ascii_lowercase()
        } else {
            self.host.clone()
        };
        Some(match self.port {
            Some(p) if Some(p) != default_port(&self.scheme) => {
                format!("{}://{host}:{p}", self.scheme)
            },
            _ => format!("{}://{host}", self.scheme),
        })
    }

    /// Whether `self` and `other` have the same (non-opaque) origin.
    pub fn same_origin(&self, other: &Url) -> bool {
        match (self.origin(), other.origin()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// The HTTP request target (`path[?query]`, `/` when empty) with
    /// C0 controls, space, DEL and non-ASCII bytes percent-encoded, as
    /// the WHATWG URL parser would have stored them. A raw CR / LF / SP
    /// here would otherwise split the HTTP/1.1 request line.
    pub fn request_target(&self) -> String {
        let mut out = String::with_capacity(self.path.len() + 1);
        if self.path.is_empty() {
            out.push('/');
        }
        push_target_encoded(&mut out, &self.path);
        if let Some(ref q) = self.query {
            out.push('?');
            push_target_encoded(&mut out, q);
        }
        out
    }
}

fn push_target_encoded(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    for &b in s.as_bytes() {
        if b <= b' ' || b >= 0x7f {
            let _ = write!(out, "%{b:02X}");
        } else {
            out.push(char::from(b));
        }
    }
}

/// Schemes whose authority is a network host: their hosts are
/// case-insensitive and they have a default port.
pub fn is_special_scheme(scheme: &str) -> bool {
    default_port(scheme).is_some()
}

/// The default port of a network scheme.
pub fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "gemini" => Some(1965),
        "ftp" => Some(21),
        _ => None,
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

/// Whether `s` begins with a URI scheme (`ALPHA *( ALPHA / DIGIT / "+" /
/// "-" / "." ) ":"`, RFC 3986 §3.1), i.e. is an absolute reference
/// rather than a relative one.
fn has_scheme(s: &str) -> bool {
    let Some(colon) = s.find(':') else {
        return false;
    };
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

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

/// Resolve a relative path against a base directory, removing `.` and
/// `..` segments (RFC 3986 §5.2.4). `..` never climbs above the root,
/// and a reference that names a directory (`sub/`, `.`, `..`) keeps its
/// trailing `/`.
fn resolve_path(base_dir: &str, relative: &str) -> String {
    let mut segments: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();

    let mut trailing_slash = false;
    for seg in relative.split('/') {
        trailing_slash = matches!(seg, "" | "." | "..");
        match seg {
            "" | "." => {},
            ".." => {
                segments.pop();
            },
            s => segments.push(s),
        }
    }

    let mut path = format!("/{}", segments.join("/"));
    if trailing_slash && !segments.is_empty() {
        path.push('/');
    }
    path
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
