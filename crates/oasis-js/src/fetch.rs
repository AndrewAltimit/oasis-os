//! Basic `fetch()` API for the JavaScript engine.
//!
//! Provides a synchronous `fetch()` global that returns a resolved Promise
//! with a Response-like object (`.status`, `.ok`, `.text()`, `.json()`).
//!
//! The actual HTTP implementation is injected via the [`FetchHandler`] trait
//! — `oasis-js` never performs I/O directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Ctx, Function, Result as JsResult};

/// An outgoing fetch request.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// The URL to fetch.
    pub url: String,
    /// HTTP method (e.g. `"GET"`, `"POST"`). Defaults to `"GET"`.
    pub method: String,
    /// Request headers.
    pub headers: HashMap<String, String>,
    /// Optional request body.
    pub body: Option<String>,
}

/// A fetch response returned by a [`FetchHandler`].
#[derive(Debug, Clone)]
pub struct FetchResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body as text.
    pub body: String,
}

/// Trait for performing HTTP requests on behalf of `fetch()`.
///
/// Implementations live outside `oasis-js` (e.g. in `oasis-browser` or
/// an app crate) and may use `oasis-net` or any other HTTP client.
pub trait FetchHandler {
    /// Execute a synchronous HTTP request and return the response.
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, String>;
}

/// A mock fetch handler that returns canned responses for testing.
///
/// Map URLs to `FetchResponse` values; any URL not in the map returns
/// a 404 response.
#[derive(Debug, Clone, Default)]
pub struct MockFetchHandler {
    /// URL -> canned response.
    pub responses: HashMap<String, FetchResponse>,
}

impl MockFetchHandler {
    /// Create an empty mock handler (all requests return 404).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canned response for the given URL.
    pub fn add(&mut self, url: impl Into<String>, response: FetchResponse) {
        self.responses.insert(url.into(), response);
    }
}

impl FetchHandler for MockFetchHandler {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, String> {
        match self.responses.get(&request.url) {
            Some(r) => Ok(r.clone()),
            None => Ok(FetchResponse {
                status: 404,
                headers: HashMap::new(),
                body: "Not Found".into(),
            }),
        }
    }
}

/// Shared, interior-mutable fetch handler for use from JS closures.
pub(crate) type SharedFetchHandler = Rc<RefCell<Option<Box<dyn FetchHandler>>>>;

/// Install the `fetch()` global into the given JS context.
///
/// The `handler` is stored in an `Rc<RefCell<..>>` and called
/// synchronously when JS invokes `fetch(url, options?)`. The returned
/// Promise resolves immediately (no async I/O).
pub(crate) fn install(ctx: &Ctx<'_>, handler: SharedFetchHandler) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_fetch(url, method, headers_json, body_or_empty)
    //    -> JSON string: { ok, status, body, headers_json } | { error }
    let h = Rc::clone(&handler);
    globals.set(
        "__oasis_fetch",
        Function::new(
            ctx.clone(),
            move |url: String, method: String, headers_json: String, body: String| -> String {
                let guard = h.borrow();
                let Some(handler) = guard.as_ref() else {
                    return r#"{"error":"no fetch handler installed"}"#.to_string();
                };

                // Parse headers from JSON.
                let headers: HashMap<String, String> = parse_simple_json_object(&headers_json);

                let request = FetchRequest {
                    url,
                    method,
                    headers,
                    body: if body.is_empty() { None } else { Some(body) },
                };

                match handler.fetch(request) {
                    Ok(resp) => {
                        let resp_headers = serialize_simple_json_object(&resp.headers);
                        // Escape body for JSON embedding.
                        let escaped_body = json_escape(&resp.body);
                        format!(
                            r#"{{"ok":true,"status":{},"body":"{}","headers":{}}}"#,
                            resp.status, escaped_body, resp_headers,
                        )
                    },
                    Err(e) => {
                        let escaped = json_escape(&e);
                        format!(r#"{{"error":"{}"}}"#, escaped)
                    },
                }
            },
        )?,
    )?;

    // -- JS wrapper: fetch(url, options?) -> Promise<Response>
    ctx.eval::<(), _>(
        br#"
globalThis.fetch = function(url, options) {
    if (typeof url !== 'string') {
        return Promise.reject(new TypeError('fetch: url must be a string'));
    }
    var method = 'GET';
    var headers = {};
    var body = '';
    if (options && typeof options === 'object') {
        if (options.method) method = String(options.method).toUpperCase();
        if (options.headers && typeof options.headers === 'object') {
            var h = options.headers;
            var keys = Object.keys(h);
            for (var i = 0; i < keys.length; i++) {
                headers[keys[i]] = String(h[keys[i]]);
            }
        }
        if (options.body !== undefined && options.body !== null) {
            body = String(options.body);
        }
    }
    var headersJson = JSON.stringify(headers);
    var raw = __oasis_fetch(url, method, headersJson, body);
    var result = JSON.parse(raw);
    if (result.error) {
        return Promise.reject(new Error(result.error));
    }
    var respHeaders = result.headers || {};
    var response = {
        status: result.status,
        ok: result.status >= 200 && result.status < 300,
        headers: {
            get: function(name) {
                var lower = String(name).toLowerCase();
                var keys = Object.keys(respHeaders);
                for (var i = 0; i < keys.length; i++) {
                    if (keys[i].toLowerCase() === lower) {
                        return respHeaders[keys[i]];
                    }
                }
                return null;
            }
        },
        text: function() {
            return Promise.resolve(result.body);
        },
        json: function() {
            try {
                return Promise.resolve(JSON.parse(result.body));
            } catch (e) {
                return Promise.reject(e);
            }
        }
    };
    return Promise.resolve(response);
};
"#,
    )?;

    Ok(())
}

/// Minimal JSON object parser for `{ "key": "value", ... }`.
/// Only handles flat string-valued objects (sufficient for headers).
fn parse_simple_json_object(json: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let trimmed = json.trim();
    if trimmed.len() < 2 || !trimmed.starts_with('{') {
        return map;
    }
    // Strip outer braces.
    let inner = &trimmed[1..trimmed.len() - 1];
    // Naively split on `","` boundaries within the JSON — this is good
    // enough for header maps that don't contain embedded quotes.
    let mut chars = inner.chars().peekable();
    while chars.peek().is_some() {
        if let Some((key, value)) = parse_json_kv(&mut chars) {
            map.insert(key, value);
        }
    }
    map
}

/// Parse one `"key":"value"` pair, advancing the iterator.
fn parse_json_kv(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<(String, String)> {
    // Skip whitespace and commas.
    skip_ws_comma(chars);
    let key = parse_json_string(chars)?;
    skip_ws_comma(chars);
    // Expect colon.
    if chars.peek() == Some(&':') {
        chars.next();
    }
    skip_ws_comma(chars);
    let value = parse_json_string(chars)?;
    Some((key, value))
}

fn skip_ws_comma(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(&c) = chars.peek() {
        if c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == ',' {
            chars.next();
        } else {
            break;
        }
    }
}

fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    if chars.peek() != Some(&'"') {
        return None;
    }
    chars.next(); // consume opening quote
    let mut s = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                match escaped {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    other => {
                        s.push('\\');
                        s.push(other);
                    },
                }
            }
        } else if c == '"' {
            return Some(s);
        } else {
            s.push(c);
        }
    }
    Some(s)
}

/// Serialize a flat `HashMap<String, String>` as a JSON object.
fn serialize_simple_json_object(map: &HashMap<String, String>) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in map.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(k));
        out.push_str("\":\"");
        out.push_str(&json_escape(v));
        out.push('"');
    }
    out.push('}');
    out
}

/// Escape a string for embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            },
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unit tests for types and helpers ---------------------------------

    #[test]
    fn mock_handler_returns_canned_response() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/api",
            FetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: r#"{"key":"value"}"#.into(),
            },
        );

        let req = FetchRequest {
            url: "https://example.com/api".into(),
            method: "GET".into(),
            headers: HashMap::new(),
            body: None,
        };
        let resp = mock.fetch(req).expect("should succeed");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, r#"{"key":"value"}"#);
    }

    #[test]
    fn mock_handler_returns_404_for_unknown() {
        let mock = MockFetchHandler::new();
        let req = FetchRequest {
            url: "https://unknown.example.com".into(),
            method: "GET".into(),
            headers: HashMap::new(),
            body: None,
        };
        let resp = mock.fetch(req).expect("should succeed with 404");
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn json_escape_special_chars() {
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
    }

    #[test]
    fn parse_and_serialize_roundtrip() {
        let mut map = HashMap::new();
        map.insert("Content-Type".into(), "text/plain".into());
        let json = serialize_simple_json_object(&map);
        let parsed = parse_simple_json_object(&json);
        assert_eq!(
            parsed.get("Content-Type").map(|s| s.as_str()),
            Some("text/plain")
        );
    }

    // -- Integration tests via JsEngine -----------------------------------

    fn engine_with_mock(mock: MockFetchHandler) -> crate::JsEngine {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).expect("engine");
        engine.install_fetch_handler(Box::new(mock));
        engine
    }

    #[test]
    fn fetch_basic_get() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/hello",
            FetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: "Hello, world!".into(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/hello')\
                 .then(function(r) { return r.text(); })\
                 .then(function(t) { console.log(t); })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "Hello, world!");
    }

    #[test]
    fn fetch_json_response() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/data",
            FetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: r#"{"name":"oasis"}"#.into(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/data')\
                 .then(function(r) { return r.json(); })\
                 .then(function(obj) { console.log(obj.name); })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "oasis");
    }

    #[test]
    fn fetch_status_and_ok() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/ok",
            FetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: String::new(),
            },
        );
        mock.add(
            "https://example.com/fail",
            FetchResponse {
                status: 500,
                headers: HashMap::new(),
                body: String::new(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/ok')\
                 .then(function(r) {\
                     console.log('ok=' + r.ok + ' status=' + r.status);\
                 })",
            )
            .expect("eval");
        engine
            .eval(
                "fetch('https://example.com/fail')\
                 .then(function(r) {\
                     console.log('ok=' + r.ok + ' status=' + r.status);\
                 })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "ok=true status=200"));
        assert!(out.iter().any(|e| e.message == "ok=false status=500"));
    }

    #[test]
    fn fetch_with_options() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/post",
            FetchResponse {
                status: 201,
                headers: HashMap::new(),
                body: "created".into(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/post', {\
                     method: 'POST',\
                     headers: {'Content-Type': 'application/json'},\
                     body: '{\"x\":1}'\
                 }).then(function(r) {\
                     console.log('status=' + r.status);\
                     return r.text();\
                 }).then(function(t) {\
                     console.log(t);\
                 })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "status=201"));
        assert!(out.iter().any(|e| e.message == "created"));
    }

    #[test]
    fn fetch_no_handler_rejects() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).expect("engine");
        engine
            .eval(
                "fetch('https://example.com')\
                 .catch(function(e) { console.log('err:' + e.message); })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message.contains("no fetch handler")));
    }

    #[test]
    fn fetch_404_for_unknown_url() {
        let mock = MockFetchHandler::new();
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://unknown.example.com')\
                 .then(function(r) {\
                     console.log('status=' + r.status + ' ok=' + r.ok);\
                 })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "status=404 ok=false"));
    }

    #[test]
    fn fetch_response_headers() {
        let mut mock = MockFetchHandler::new();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".into(), "text/html".into());
        mock.add(
            "https://example.com/page",
            FetchResponse {
                status: 200,
                headers,
                body: "<h1>hi</h1>".into(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/page')\
                 .then(function(r) {\
                     console.log(r.headers.get('content-type'));\
                 })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "text/html"));
    }

    #[test]
    fn fetch_invalid_json_rejects() {
        let mut mock = MockFetchHandler::new();
        mock.add(
            "https://example.com/bad",
            FetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: "not json".into(),
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/bad')\
                 .then(function(r) { return r.json(); })\
                 .catch(function(e) { console.log('parse_err'); })",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "parse_err"));
    }
}
