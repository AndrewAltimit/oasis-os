//! Minimal Content Security Policy (CSP) parser and enforcement.
//!
//! Supports `default-src` and `script-src` directives with source
//! values: `'self'`, `'none'`, `*`, and explicit URL/domain patterns.

use super::Url;

/// A parsed Content Security Policy.
#[derive(Debug, Clone, Default)]
pub struct CspPolicy {
    /// Allowed sources for `default-src` (fallback for all resource types).
    pub default_src: Option<Vec<CspSource>>,
    /// Allowed sources for `script-src`.
    pub script_src: Option<Vec<CspSource>>,
}

/// A single CSP source expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CspSource {
    /// `'self'` -- same origin only.
    SelF,
    /// `'none'` -- block everything.
    None,
    /// `*` -- allow any source.
    Wildcard,
    /// An explicit host or URL pattern (e.g. `https://cdn.example.com`).
    Host(String),
}

/// Parse a CSP header value into a [`CspPolicy`].
///
/// The header is a semicolon-separated list of directives, each of
/// the form `directive-name source1 source2 ...`.
pub fn parse_csp(header: &str) -> CspPolicy {
    let mut policy = CspPolicy::default();

    for directive in header.split(';') {
        let directive = directive.trim();
        if directive.is_empty() {
            continue;
        }

        let mut parts = directive.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };

        let sources: Vec<CspSource> = parts.map(parse_source).collect();

        match name.to_lowercase().as_str() {
            "default-src" => policy.default_src = Some(sources),
            "script-src" => policy.script_src = Some(sources),
            _ => {
                // Unsupported directive -- ignore.
            },
        }
    }

    policy
}

/// Parse a single CSP source token.
fn parse_source(token: &str) -> CspSource {
    match token.to_lowercase().as_str() {
        "'self'" => CspSource::SelF,
        "'none'" => CspSource::None,
        "*" => CspSource::Wildcard,
        _ => CspSource::Host(token.to_string()),
    }
}

impl CspPolicy {
    /// Check whether a resource URL is allowed for a given resource type.
    ///
    /// `page_url` is the origin of the page that declared the policy.
    /// Returns `true` if the load is permitted, `false` if blocked.
    pub fn allows(
        &self,
        resource_url: &str,
        page_url: &str,
        resource_type: CspResourceType,
    ) -> bool {
        let sources = match resource_type {
            CspResourceType::Script => self.script_src.as_ref().or(self.default_src.as_ref()),
            _ => self.default_src.as_ref(),
        };

        let Some(sources) = sources else {
            // No matching directive -- allow by default.
            return true;
        };

        // If directive is present but empty, block everything.
        if sources.is_empty() {
            return false;
        }

        for source in sources {
            match source {
                CspSource::Wildcard => return true,
                CspSource::None => return false,
                CspSource::SelF => {
                    if is_same_origin(page_url, resource_url) {
                        return true;
                    }
                },
                CspSource::Host(pattern) => {
                    if host_matches(pattern, resource_url) {
                        return true;
                    }
                },
            }
        }

        false
    }

    /// Returns `true` if this policy has any directives set.
    pub fn is_active(&self) -> bool {
        self.default_src.is_some() || self.script_src.is_some()
    }
}

/// Resource types for CSP enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspResourceType {
    /// Image sub-resources (`<img>`).
    Image,
    /// Stylesheets (`<link rel=stylesheet>`).
    Style,
    /// Scripts (`<script>`).
    Script,
    /// Other or unknown resource type.
    Default,
}

/// Check if two URLs share the same origin (scheme + host + port).
fn is_same_origin(page_url: &str, resource_url: &str) -> bool {
    let page = match Url::parse(page_url) {
        Some(u) => u,
        None => return false,
    };
    let resource = match Url::parse(resource_url) {
        Some(u) => u,
        None => return false,
    };
    page.origin() == resource.origin()
}

/// Check if a host pattern matches the resource URL.
///
/// Supports full URLs (`https://cdn.example.com`) and bare domains
/// (`cdn.example.com`). Wildcard subdomain (`*.example.com`) is also
/// handled.
fn host_matches(pattern: &str, resource_url: &str) -> bool {
    let resource = match Url::parse(resource_url) {
        Some(u) => u,
        None => return false,
    };

    // If the pattern has a scheme, parse as a URL.
    if pattern.contains("://") {
        if let Some(pattern_url) = Url::parse(pattern) {
            // Scheme must match.
            if pattern_url.scheme != resource.scheme {
                return false;
            }
            return domain_matches(&pattern_url.host, &resource.host);
        }
        return false;
    }

    // Bare domain pattern (no scheme).
    domain_matches(pattern, &resource.host)
}

/// Check if a domain pattern matches a host.
///
/// Supports wildcard prefix: `*.example.com` matches
/// `sub.example.com` and `a.b.example.com`.
fn domain_matches(pattern: &str, host: &str) -> bool {
    if pattern == host {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // Wildcard: host must end with .suffix or equal suffix.
        if host == suffix {
            return true;
        }
        if host.ends_with(&format!(".{suffix}")) {
            return true;
        }
    }
    false
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_header() {
        let policy = parse_csp("");
        assert!(!policy.is_active());
    }

    #[test]
    fn parse_default_src_none() {
        let policy = parse_csp("default-src 'none'");
        assert!(policy.is_active());
        assert_eq!(policy.default_src, Some(vec![CspSource::None]),);
    }

    #[test]
    fn parse_default_src_self() {
        let policy = parse_csp("default-src 'self'");
        assert_eq!(policy.default_src, Some(vec![CspSource::SelF]),);
    }

    #[test]
    fn parse_multiple_sources() {
        let policy = parse_csp("default-src 'self' https://cdn.example.com *");
        let sources = policy.default_src.unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0], CspSource::SelF);
        assert_eq!(
            sources[1],
            CspSource::Host("https://cdn.example.com".to_string()),
        );
        assert_eq!(sources[2], CspSource::Wildcard);
    }

    #[test]
    fn parse_multiple_directives() {
        let policy = parse_csp("default-src 'self'; script-src 'none'");
        assert_eq!(policy.default_src, Some(vec![CspSource::SelF]),);
        assert_eq!(policy.script_src, Some(vec![CspSource::None]),);
    }

    #[test]
    fn allows_wildcard() {
        let policy = parse_csp("default-src *");
        assert!(policy.allows(
            "http://evil.com/script.js",
            "http://example.com/page",
            CspResourceType::Default,
        ));
    }

    #[test]
    fn blocks_none() {
        let policy = parse_csp("default-src 'none'");
        assert!(!policy.allows(
            "http://example.com/image.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
    }

    #[test]
    fn allows_self_same_origin() {
        let policy = parse_csp("default-src 'self'");
        assert!(policy.allows(
            "http://example.com/style.css",
            "http://example.com/page",
            CspResourceType::Style,
        ));
    }

    #[test]
    fn blocks_self_different_origin() {
        let policy = parse_csp("default-src 'self'");
        assert!(!policy.allows(
            "http://other.com/style.css",
            "http://example.com/page",
            CspResourceType::Style,
        ));
    }

    #[test]
    fn allows_explicit_host() {
        let policy = parse_csp("default-src https://cdn.example.com");
        assert!(policy.allows(
            "https://cdn.example.com/img.png",
            "https://example.com/page",
            CspResourceType::Image,
        ));
    }

    #[test]
    fn blocks_non_matching_host() {
        let policy = parse_csp("default-src https://cdn.example.com");
        assert!(!policy.allows(
            "https://evil.com/img.png",
            "https://example.com/page",
            CspResourceType::Image,
        ));
    }

    #[test]
    fn script_src_overrides_default() {
        let policy = parse_csp("default-src 'self'; script-src 'none'");
        // Images fall back to default-src (self) -- allowed.
        assert!(policy.allows(
            "http://example.com/img.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
        // Scripts use script-src (none) -- blocked.
        assert!(!policy.allows(
            "http://example.com/app.js",
            "http://example.com/page",
            CspResourceType::Script,
        ));
    }

    #[test]
    fn no_directive_allows_all() {
        let policy = parse_csp("");
        assert!(policy.allows(
            "http://any.com/anything",
            "http://example.com/page",
            CspResourceType::Default,
        ));
    }

    #[test]
    fn wildcard_subdomain() {
        let policy = parse_csp("default-src *.example.com");
        assert!(policy.allows(
            "http://cdn.example.com/img.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
        assert!(policy.allows(
            "http://a.b.example.com/img.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
        assert!(!policy.allows(
            "http://other.com/img.png",
            "http://example.com/page",
            CspResourceType::Image,
        ));
    }

    #[test]
    fn is_same_origin_basic() {
        assert!(is_same_origin(
            "http://example.com/page",
            "http://example.com/other",
        ));
        assert!(!is_same_origin(
            "http://example.com/page",
            "https://example.com/other",
        ));
        assert!(!is_same_origin(
            "http://example.com/page",
            "http://other.com/page",
        ));
    }
}
