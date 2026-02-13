//! Browser plugin API: custom URL scheme handlers and content filters.
//!
//! Plugins extend the browser through two extension points:
//!
//! - **[`UrlSchemeHandler`]** -- resolves custom URL schemes
//!   (e.g. `oasis://status`, `agent://list`, `trade://cards?set=alpha`).
//!
//! - **[`ContentFilter`]** -- transforms HTML before the parser sees it
//!   (e.g. ad removal, content injection, classification/logging).
//!
//! Both are registered in a [`BrowserPluginRegistry`] which the browser
//! widget queries during navigation and page loading.

use crate::loader::ContentType;
use oasis_types::error::Result;

// -----------------------------------------------------------------------
// UrlSchemeHandler trait
// -----------------------------------------------------------------------

/// Handler for a custom URL scheme (e.g. `oasis://`, `agent://`).
///
/// Implementations must be `Send` so the registry can be moved between
/// threads (e.g. into a worker or across an FFI boundary).
///
/// # Examples
///
/// ```ignore
/// struct OasisScheme;
/// impl UrlSchemeHandler for OasisScheme {
///     fn scheme(&self) -> &str { "oasis" }
///     fn fetch(&self, url: &str) -> Result<(Vec<u8>, ContentType)> {
///         Ok((b"<html><body>Status OK</body></html>".to_vec(),
///             ContentType::Html))
///     }
/// }
/// ```
pub trait UrlSchemeHandler: Send {
    /// The scheme this handler responds to (without the trailing `://`).
    ///
    /// For example, return `"oasis"` to handle `oasis://...` URLs.
    fn scheme(&self) -> &str;

    /// Fetch content for a URL with this scheme.
    ///
    /// The full URL string is passed (including the scheme prefix).
    /// Returns `(response_body, content_type)` on success.
    fn fetch(&self, url: &str) -> Result<(Vec<u8>, ContentType)>;
}

// -----------------------------------------------------------------------
// ContentFilter trait
// -----------------------------------------------------------------------

/// Filter that transforms HTML content before parsing.
///
/// Filters are applied in the order they were registered. Each filter
/// receives the (possibly already-modified) HTML from the previous
/// filter and can choose to transform it or pass it through unchanged.
pub trait ContentFilter: Send {
    /// A unique human-readable name for this filter (used for logging
    /// and debugging).
    fn name(&self) -> &str;

    /// Transform HTML content for the given URL.
    ///
    /// Return `Some(new_html)` to replace the content, or `None` to
    /// pass through unchanged.
    fn filter(&self, url: &str, html: &str) -> Option<String>;
}

// -----------------------------------------------------------------------
// BrowserPluginRegistry
// -----------------------------------------------------------------------

/// Registry of browser plugins (scheme handlers and content filters).
///
/// The browser widget holds one of these and consults it during
/// navigation (scheme resolution) and page loading (content filtering).
pub struct BrowserPluginRegistry {
    scheme_handlers: Vec<Box<dyn UrlSchemeHandler>>,
    content_filters: Vec<Box<dyn ContentFilter>>,
}

impl BrowserPluginRegistry {
    /// Create an empty registry with no handlers or filters.
    pub fn new() -> Self {
        Self {
            scheme_handlers: Vec::new(),
            content_filters: Vec::new(),
        }
    }

    /// Register a custom URL scheme handler.
    ///
    /// If a handler for the same scheme already exists, the new handler
    /// shadows it (the first match wins during resolution, so the
    /// *last* registered handler for a given scheme takes precedence
    /// only if you iterate in reverse -- here we keep first-wins
    /// semantics).
    pub fn register_scheme_handler(&mut self, handler: Box<dyn UrlSchemeHandler>) {
        self.scheme_handlers.push(handler);
    }

    /// Register a content filter.
    ///
    /// Filters are applied in registration order.
    pub fn register_content_filter(&mut self, filter: Box<dyn ContentFilter>) {
        self.content_filters.push(filter);
    }

    /// Look up a scheme handler for the given URL.
    ///
    /// The URL is expected to start with `"scheme://"`. Returns `None`
    /// if no handler matches.
    pub fn resolve_scheme(&self, url: &str) -> Option<&dyn UrlSchemeHandler> {
        let scheme = url_scheme(url)?;
        self.scheme_handlers
            .iter()
            .find(|h| h.scheme().eq_ignore_ascii_case(scheme))
            .map(|h| &**h)
    }

    /// Apply all content filters to HTML in registration order.
    ///
    /// Each filter receives the (possibly already-modified) HTML from
    /// the previous filter. Filters that return `None` leave the HTML
    /// unchanged.
    pub fn apply_filters(&self, url: &str, html: &str) -> String {
        let mut current = html.to_string();
        for f in &self.content_filters {
            if let Some(transformed) = f.filter(url, &current) {
                current = transformed;
            }
        }
        current
    }

    /// Check if a custom scheme is registered (case-insensitive).
    pub fn has_scheme(&self, scheme: &str) -> bool {
        self.scheme_handlers
            .iter()
            .any(|h| h.scheme().eq_ignore_ascii_case(scheme))
    }

    /// List all registered scheme names (in registration order).
    pub fn registered_schemes(&self) -> Vec<&str> {
        self.scheme_handlers.iter().map(|h| h.scheme()).collect()
    }
}

impl Default for BrowserPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Extract the scheme portion from a URL string.
///
/// Returns `Some("oasis")` for `"oasis://status"`, or `None` if the
/// URL does not contain `"://"`.
fn url_scheme(url: &str) -> Option<&str> {
    url.find("://").map(|i| &url[..i])
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test helpers ---------------------------------------------------

    /// A simple scheme handler that returns a fixed HTML page.
    struct FixedSchemeHandler {
        name: String,
        body: Vec<u8>,
    }

    impl FixedSchemeHandler {
        fn new(scheme: &str, body: &str) -> Self {
            Self {
                name: scheme.to_string(),
                body: body.as_bytes().to_vec(),
            }
        }
    }

    impl UrlSchemeHandler for FixedSchemeHandler {
        fn scheme(&self) -> &str {
            &self.name
        }

        fn fetch(&self, _url: &str) -> Result<(Vec<u8>, ContentType)> {
            Ok((self.body.clone(), ContentType::Html))
        }
    }

    /// A content filter that uppercases all HTML.
    struct UppercaseFilter;

    impl ContentFilter for UppercaseFilter {
        fn name(&self) -> &str {
            "uppercase"
        }

        fn filter(&self, _url: &str, html: &str) -> Option<String> {
            Some(html.to_uppercase())
        }
    }

    /// A content filter that passes through unchanged (returns None).
    struct PassthroughFilter;

    impl ContentFilter for PassthroughFilter {
        fn name(&self) -> &str {
            "passthrough"
        }

        fn filter(&self, _url: &str, _html: &str) -> Option<String> {
            None
        }
    }

    /// A content filter that appends a suffix to the HTML.
    struct SuffixFilter {
        suffix: String,
    }

    impl SuffixFilter {
        fn new(suffix: &str) -> Self {
            Self {
                suffix: suffix.to_string(),
            }
        }
    }

    impl ContentFilter for SuffixFilter {
        fn name(&self) -> &str {
            "suffix"
        }

        fn filter(&self, _url: &str, html: &str) -> Option<String> {
            Some(format!("{}{}", html, self.suffix))
        }
    }

    // -- Tests ----------------------------------------------------------

    #[test]
    fn empty_registry_has_no_schemes() {
        let reg = BrowserPluginRegistry::new();
        assert!(reg.registered_schemes().is_empty());
        assert!(!reg.has_scheme("oasis"));
        assert!(reg.resolve_scheme("oasis://test").is_none());
    }

    #[test]
    fn register_and_resolve_scheme_handler() {
        let mut reg = BrowserPluginRegistry::new();
        let handler = FixedSchemeHandler::new("oasis", "<html>OK</html>");
        reg.register_scheme_handler(Box::new(handler));

        let resolved = reg.resolve_scheme("oasis://status");
        assert!(resolved.is_some());
        let h = resolved.unwrap();
        assert_eq!(h.scheme(), "oasis");

        let (body, ct) = h.fetch("oasis://status").unwrap();
        assert_eq!(ct, ContentType::Html);
        assert_eq!(body, b"<html>OK</html>");
    }

    #[test]
    fn unknown_scheme_returns_none() {
        let mut reg = BrowserPluginRegistry::new();
        let handler = FixedSchemeHandler::new("oasis", "data");
        reg.register_scheme_handler(Box::new(handler));

        assert!(reg.resolve_scheme("agent://list").is_none());
        assert!(reg.resolve_scheme("http://example.com").is_none());
    }

    #[test]
    fn register_and_apply_content_filter() {
        let mut reg = BrowserPluginRegistry::new();
        reg.register_content_filter(Box::new(UppercaseFilter));

        let result = reg.apply_filters("http://example.com", "<html>hello</html>");
        assert_eq!(result, "<HTML>HELLO</HTML>");
    }

    #[test]
    fn multiple_filters_applied_in_order() {
        let mut reg = BrowserPluginRegistry::new();
        // First: append "!!"
        reg.register_content_filter(Box::new(SuffixFilter::new("!!")));
        // Second: uppercase everything
        reg.register_content_filter(Box::new(UppercaseFilter));

        let result = reg.apply_filters("http://example.com", "hello");
        // Step 1: "hello" -> "hello!!"
        // Step 2: "hello!!" -> "HELLO!!"
        assert_eq!(result, "HELLO!!");
    }

    #[test]
    fn filter_returning_none_passes_through() {
        let mut reg = BrowserPluginRegistry::new();
        reg.register_content_filter(Box::new(PassthroughFilter));

        let input = "<html>unchanged</html>";
        let result = reg.apply_filters("http://example.com", input);
        assert_eq!(result, input);
    }

    #[test]
    fn has_scheme_returns_correct_values() {
        let mut reg = BrowserPluginRegistry::new();
        assert!(!reg.has_scheme("oasis"));

        let h1 = FixedSchemeHandler::new("oasis", "data");
        let h2 = FixedSchemeHandler::new("agent", "data");
        reg.register_scheme_handler(Box::new(h1));
        reg.register_scheme_handler(Box::new(h2));

        assert!(reg.has_scheme("oasis"));
        assert!(reg.has_scheme("agent"));
        assert!(reg.has_scheme("OASIS")); // case-insensitive
        assert!(!reg.has_scheme("trade"));
    }

    #[test]
    fn registered_schemes_lists_all_schemes() {
        let mut reg = BrowserPluginRegistry::new();

        let h1 = FixedSchemeHandler::new("oasis", "a");
        let h2 = FixedSchemeHandler::new("agent", "b");
        let h3 = FixedSchemeHandler::new("trade", "c");
        reg.register_scheme_handler(Box::new(h1));
        reg.register_scheme_handler(Box::new(h2));
        reg.register_scheme_handler(Box::new(h3));

        let schemes = reg.registered_schemes();
        assert_eq!(schemes.len(), 3);
        assert_eq!(schemes[0], "oasis");
        assert_eq!(schemes[1], "agent");
        assert_eq!(schemes[2], "trade");
    }

    #[test]
    fn default_delegates_to_new() {
        let reg = BrowserPluginRegistry::default();
        assert!(reg.registered_schemes().is_empty());
    }

    #[test]
    fn url_scheme_extraction() {
        assert_eq!(url_scheme("oasis://status"), Some("oasis"));
        assert_eq!(url_scheme("agent://list"), Some("agent"));
        assert_eq!(url_scheme("no-scheme-here"), None);
        assert_eq!(url_scheme(""), None);
    }
}
