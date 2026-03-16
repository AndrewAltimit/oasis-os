//! Session-scoped cookie jar.

use super::Url;

/// A single HTTP cookie.
#[derive(Debug, Clone)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie belongs to.
    pub domain: String,
    /// Path scope for the cookie.
    pub path: String,
    /// Whether the cookie requires HTTPS.
    pub secure: bool,
    /// Whether the cookie is inaccessible to JavaScript.
    pub http_only: bool,
    /// Optional expiry as a Unix timestamp.
    pub expires: Option<u64>,
}

/// Session-scoped cookie store.
///
/// Cookies are held in memory for the lifetime of the browser session
/// and are never persisted to disk.
#[derive(Debug, Clone)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
        }
    }

    /// Parse `Set-Cookie` headers from a response and store the cookies.
    ///
    /// Only headers whose name (case-insensitive) is `set-cookie` are
    /// processed.  The `url` is used to fill in default domain/path when
    /// the header does not specify them.
    pub fn set_cookies(&mut self, url: &Url, headers: &[(String, String)]) {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("set-cookie")
                && let Some(cookie) = parse_set_cookie(value, url)
            {
                // Replace any existing cookie with the same
                // (name, domain, path) tuple.
                self.cookies.retain(|c| {
                    !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
                });
                self.cookies.push(cookie);
            }
        }
    }

    /// Build a `Cookie` header value for the given URL.
    ///
    /// Returns `None` when no cookies match.
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        let pairs: Vec<String> = self
            .cookies
            .iter()
            .filter(|c| cookie_matches(c, url))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();

        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------
// Internals
// -------------------------------------------------------------------

/// Parse a single `Set-Cookie` header value into a [`Cookie`].
fn parse_set_cookie(header: &str, url: &Url) -> Option<Cookie> {
    let mut parts = header.split(';');

    // First segment: name=value
    let name_value = parts.next()?.trim();
    let (name, value) = name_value.split_once('=')?;
    let name = name.trim().to_string();
    let value = value.trim().to_string();

    if name.is_empty() {
        return None;
    }

    let mut domain = url.host.to_lowercase();
    let mut path = url.directory().to_string();
    let mut secure = false;
    let mut http_only = false;
    let mut expires: Option<u64> = None;

    for part in parts {
        let part = part.trim();
        if let Some((attr, attr_val)) = part.split_once('=') {
            let attr = attr.trim().to_lowercase();
            let attr_val = attr_val.trim();
            match attr.as_str() {
                "domain" => {
                    domain = attr_val.trim_start_matches('.').to_lowercase();
                },
                "path" => {
                    path = attr_val.to_string();
                },
                "max-age" => {
                    if let Ok(secs) = attr_val.parse::<u64>() {
                        // Approximate: treat max-age as an absolute
                        // future timestamp offset from a fixed epoch.
                        // For a session-scoped jar this is sufficient.
                        expires = Some(secs);
                    }
                },
                _ => {},
            }
        } else {
            match part.to_lowercase().as_str() {
                "secure" => secure = true,
                "httponly" => http_only = true,
                _ => {},
            }
        }
    }

    Some(Cookie {
        name,
        value,
        domain,
        path,
        secure,
        http_only,
        expires,
    })
}

/// Check whether a cookie should be sent for the given URL.
fn cookie_matches(cookie: &Cookie, url: &Url) -> bool {
    // Secure cookies only over HTTPS.
    if cookie.secure && url.scheme != "https" {
        return false;
    }

    // Domain suffix match.
    let host = url.host.to_lowercase();
    if host != cookie.domain && !host.ends_with(&format!(".{}", cookie.domain)) {
        return false;
    }

    // Path prefix match.
    if !url.path.starts_with(&cookie.path) {
        return false;
    }

    true
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid test URL")
    }

    #[test]
    fn parse_simple_set_cookie() {
        let u = url("http://example.com/path/page");
        let cookie = parse_set_cookie("session=abc123", &u).expect("should parse");
        assert_eq!(cookie.name, "session");
        assert_eq!(cookie.value, "abc123");
        assert_eq!(cookie.domain, "example.com");
        assert_eq!(cookie.path, "/path/");
        assert!(!cookie.secure);
        assert!(!cookie.http_only);
    }

    #[test]
    fn parse_full_set_cookie() {
        let u = url("http://example.com/");
        let cookie = parse_set_cookie(
            "id=42; Path=/; Domain=.example.com; Secure; HttpOnly; Max-Age=3600",
            &u,
        )
        .expect("should parse");
        assert_eq!(cookie.name, "id");
        assert_eq!(cookie.value, "42");
        assert_eq!(cookie.domain, "example.com");
        assert_eq!(cookie.path, "/");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.expires, Some(3600));
    }

    #[test]
    fn cookie_jar_stores_and_retrieves() {
        let u = url("http://example.com/app/index");
        let mut jar = CookieJar::new();
        jar.set_cookies(&u, &[("set-cookie".to_string(), "token=xyz".to_string())]);

        let header = jar.cookie_header(&u);
        assert_eq!(header, Some("token=xyz".to_string()));
    }

    #[test]
    fn cookie_jar_domain_mismatch() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies(&u, &[("set-cookie".to_string(), "a=1".to_string())]);

        let other = url("http://other.com/");
        assert!(jar.cookie_header(&other).is_none());
    }

    #[test]
    fn cookie_jar_subdomain_match() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies(
            &u,
            &[(
                "set-cookie".to_string(),
                "a=1; Domain=.example.com".to_string(),
            )],
        );

        let sub = url("http://sub.example.com/");
        assert_eq!(jar.cookie_header(&sub), Some("a=1".to_string()));
    }

    #[test]
    fn cookie_jar_path_mismatch() {
        let u = url("http://example.com/app/page");
        let mut jar = CookieJar::new();
        jar.set_cookies(
            &u,
            &[("set-cookie".to_string(), "a=1; Path=/app/".to_string())],
        );

        let other = url("http://example.com/other/page");
        assert!(jar.cookie_header(&other).is_none());
    }

    #[test]
    fn secure_cookie_not_sent_over_http() {
        let u = url("https://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies(&u, &[("set-cookie".to_string(), "s=1; Secure".to_string())]);

        let http = url("http://example.com/");
        assert!(jar.cookie_header(&http).is_none());
    }

    #[test]
    fn cookie_jar_replaces_duplicate() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies(&u, &[("set-cookie".to_string(), "a=1".to_string())]);
        jar.set_cookies(&u, &[("set-cookie".to_string(), "a=2".to_string())]);

        assert_eq!(jar.cookie_header(&u), Some("a=2".to_string()),);
    }

    #[test]
    fn multiple_cookies_in_header() {
        let u = url("http://example.com/");
        let mut jar = CookieJar::new();
        jar.set_cookies(
            &u,
            &[
                ("set-cookie".to_string(), "a=1".to_string()),
                ("set-cookie".to_string(), "b=2".to_string()),
            ],
        );

        let header = jar.cookie_header(&u).expect("should have cookies");
        assert!(header.contains("a=1"));
        assert!(header.contains("b=2"));
        assert!(header.contains("; "));
    }
}
