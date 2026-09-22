//! `fetch()` API for the JavaScript engine.
//!
//! Provides a `fetch(input, init?)` global returning a real `Promise` that
//! resolves to a standard-shaped `Response` (`status`, `ok`, `statusText`,
//! `url`, `headers.get()`, `text()`, `json()`, `clone()`), plus `Headers`
//! and `Response` constructors.
//!
//! The actual HTTP implementation is injected via the [`FetchHandler`] trait
//! — `oasis-js` never performs I/O directly. The handler is invoked
//! synchronously from inside the `Promise` executor; the promise then
//! settles and `.then()` callbacks run on the next microtask drain, so
//! chains like `fetch(u).then(r => r.json()).then(d => ...)` behave as on
//! the web.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Ctx, Function, Result as JsResult};

/// An outgoing fetch request.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// The URL to fetch, exactly as passed to `fetch()` (may be relative;
    /// resolving it is the handler's job).
    pub url: String,
    /// HTTP method (e.g. `"GET"`, `"POST"`). Defaults to `"GET"`.
    /// Standard methods are upper-cased.
    pub method: String,
    /// Request headers (names lower-cased).
    pub headers: HashMap<String, String>,
    /// Optional request body.
    pub body: Option<String>,
}

/// A fetch response returned by a [`FetchHandler`].
#[derive(Debug, Clone, Default)]
pub struct FetchResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body as text.
    pub body: String,
    /// HTTP reason phrase. When empty, JS sees the standard phrase for
    /// `status` (e.g. `"Not Found"` for 404).
    pub status_text: String,
    /// Final URL after redirects. When empty, JS sees the request URL.
    pub url: String,
}

/// Trait for performing HTTP requests on behalf of `fetch()`.
///
/// Implementations live outside `oasis-js` (e.g. in `oasis-browser` or
/// an app crate) and may use `oasis-net` or any other HTTP client.
/// Returning `Err` rejects the JS promise with a `TypeError`
/// (a network error in `fetch()` terms).
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
                body: "Not Found".into(),
                ..FetchResponse::default()
            }),
        }
    }
}

/// Shared, interior-mutable fetch handler for use from JS closures.
pub(crate) type SharedFetchHandler = Rc<RefCell<Option<Box<dyn FetchHandler>>>>;

/// Install the `fetch()` global into the given JS context.
///
/// The `handler` is stored in an `Rc<RefCell<..>>` and called
/// synchronously when JS invokes `fetch(url, options?)`.
pub(crate) fn install(ctx: &Ctx<'_>, handler: SharedFetchHandler) -> JsResult<()> {
    set_native_fetch(ctx, handler)?;
    ctx.eval::<(), _>(FETCH_JS)?;
    Ok(())
}

/// Bind `handler` as the transport behind `fetch()` in this context only.
///
/// This replaces the native `__oasis_fetch` hook that the JS `fetch()`
/// wrapper (installed by [`JsEngine::new`](crate::JsEngine::new)) calls,
/// so the context gets the standard Promise/`Response` surface backed by
/// `handler`. Use it when you hold a `Ctx` rather than the engine (e.g.
/// `oasis-browser` binding a per-page, origin-aware handler); otherwise
/// prefer [`JsEngine::install_fetch_handler`](crate::JsEngine::install_fetch_handler).
pub fn bind_fetch_handler(ctx: &Ctx<'_>, handler: Box<dyn FetchHandler>) -> JsResult<()> {
    set_native_fetch(ctx, Rc::new(RefCell::new(Some(handler))))
}

/// Register `__oasis_fetch(url, method, headers_json, body_or_empty)
/// -> JSON { status, statusText, url, body, headers } | { error }`.
fn set_native_fetch(ctx: &Ctx<'_>, handler: SharedFetchHandler) -> JsResult<()> {
    ctx.globals().set(
        "__oasis_fetch",
        Function::new(
            ctx.clone(),
            move |url: String, method: String, headers_json: String, body: String| -> String {
                let guard = handler.borrow();
                let Some(handler) = guard.as_ref() else {
                    return r#"{"error":"no fetch handler installed"}"#.to_string();
                };

                let request = FetchRequest {
                    url,
                    method,
                    headers: parse_simple_json_object(&headers_json),
                    body: if body.is_empty() { None } else { Some(body) },
                };

                match handler.fetch(request) {
                    Ok(resp) => format!(
                        r#"{{"status":{},"statusText":"{}","url":"{}","body":"{}","headers":{}}}"#,
                        resp.status,
                        json_escape(&resp.status_text),
                        json_escape(&resp.url),
                        json_escape(&resp.body),
                        serialize_simple_json_object(&resp.headers),
                    ),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
                }
            },
        )?,
    )
}

/// JS half of the fetch API: `Headers`, `Response` and `fetch()`.
const FETCH_JS: &str = r#"
(function() {
  var STATUS_TEXT = {
    200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content',
    206: 'Partial Content', 301: 'Moved Permanently', 302: 'Found',
    303: 'See Other', 304: 'Not Modified', 307: 'Temporary Redirect',
    308: 'Permanent Redirect', 400: 'Bad Request', 401: 'Unauthorized',
    403: 'Forbidden', 404: 'Not Found', 405: 'Method Not Allowed',
    408: 'Request Timeout', 409: 'Conflict', 410: 'Gone',
    413: 'Payload Too Large', 415: 'Unsupported Media Type',
    429: 'Too Many Requests', 500: 'Internal Server Error',
    501: 'Not Implemented', 502: 'Bad Gateway', 503: 'Service Unavailable',
    504: 'Gateway Timeout'
  };
  var hasOwn = Object.prototype.hasOwnProperty;

  function Headers(init) {
    this.__map = {};
    if (init === undefined || init === null) return;
    var self = this;
    if (init instanceof Headers) {
      init.forEach(function(v, k) { self.append(k, v); });
    } else if (Array.isArray(init)) {
      for (var i = 0; i < init.length; i++) this.append(init[i][0], init[i][1]);
    } else if (typeof init === 'object') {
      Object.keys(init).forEach(function(k) { self.append(k, init[k]); });
    }
  }
  Headers.prototype.append = function(name, value) {
    var k = String(name).toLowerCase(), v = String(value);
    this.__map[k] = hasOwn.call(this.__map, k) ? this.__map[k] + ', ' + v : v;
  };
  Headers.prototype.set = function(name, value) {
    this.__map[String(name).toLowerCase()] = String(value);
  };
  Headers.prototype.get = function(name) {
    var k = String(name).toLowerCase();
    return hasOwn.call(this.__map, k) ? this.__map[k] : null;
  };
  Headers.prototype.has = function(name) {
    return hasOwn.call(this.__map, String(name).toLowerCase());
  };
  Headers.prototype['delete'] = function(name) {
    delete this.__map[String(name).toLowerCase()];
  };
  Headers.prototype.__entries = function() {
    var map = this.__map;
    return Object.keys(map).sort().map(function(k) { return [k, map[k]]; });
  };
  Headers.prototype.forEach = function(cb, thisArg) {
    var e = this.__entries();
    for (var i = 0; i < e.length; i++) cb.call(thisArg, e[i][1], e[i][0], this);
  };
  Headers.prototype.entries = function() { return this.__entries()[Symbol.iterator](); };
  Headers.prototype.keys = function() {
    return this.__entries().map(function(e) { return e[0]; })[Symbol.iterator]();
  };
  Headers.prototype.values = function() {
    return this.__entries().map(function(e) { return e[1]; })[Symbol.iterator]();
  };
  Headers.prototype[Symbol.iterator] = Headers.prototype.entries;

  function Response(body, init) {
    init = init || {};
    this.__body = (body === undefined || body === null) ? '' : String(body);
    this.status = init.status === undefined ? 200 : (init.status | 0);
    this.statusText = init.statusText ? String(init.statusText)
      : (STATUS_TEXT[this.status] || '');
    this.ok = this.status >= 200 && this.status < 300;
    this.headers = init.headers instanceof Headers ? init.headers : new Headers(init.headers);
    this.url = init.url ? String(init.url) : '';
    this.redirected = !!init.redirected;
    this.type = init.type || 'default';
    this.bodyUsed = false;
  }
  Response.prototype.__consume = function() {
    if (this.bodyUsed) {
      return Promise.reject(new TypeError('Body has already been consumed.'));
    }
    this.bodyUsed = true;
    return Promise.resolve(this.__body);
  };
  Response.prototype.text = function() { return this.__consume(); };
  Response.prototype.json = function() {
    return this.__consume().then(function(t) { return JSON.parse(t); });
  };
  Response.prototype.arrayBuffer = function() {
    return this.__consume().then(function(t) {
      var bin = unescape(encodeURIComponent(t));
      var out = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return out.buffer;
    });
  };
  Response.prototype.clone = function() {
    if (this.bodyUsed) throw new TypeError('Response body is already used');
    return new Response(this.__body, {
      status: this.status, statusText: this.statusText,
      headers: new Headers(this.headers), url: this.url,
      redirected: this.redirected, type: this.type
    });
  };

  var STANDARD_METHODS = ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'PATCH', 'POST', 'PUT'];

  globalThis.Headers = Headers;
  globalThis.Response = Response;
  globalThis.fetch = function(input, init) {
    return new Promise(function(resolve, reject) {
      var url, method = 'GET', headers = new Headers(), body = '';
      if (input !== null && typeof input === 'object' && typeof input.url === 'string') {
        url = input.url;
        if (input.method) method = String(input.method);
        if (input.headers) headers = new Headers(input.headers);
        if (input.body !== undefined && input.body !== null) body = String(input.body);
      } else {
        url = String(input);
      }
      if (init !== null && typeof init === 'object') {
        if (init.method) method = String(init.method);
        if (init.headers) headers = new Headers(init.headers);
        if (init.body !== undefined && init.body !== null) body = String(init.body);
      }
      var upper = method.toUpperCase();
      if (STANDARD_METHODS.indexOf(upper) >= 0) method = upper;
      if ((method === 'GET' || method === 'HEAD') && body !== '') {
        throw new TypeError('fetch: ' + method + ' request cannot have a body');
      }
      var plain = {};
      headers.forEach(function(v, k) { plain[k] = v; });
      var result = JSON.parse(__oasis_fetch(url, method, JSON.stringify(plain), body));
      if (result.error) {
        reject(new TypeError('Failed to fetch: ' + result.error));
        return;
      }
      resolve(new Response(method === 'HEAD' ? '' : result.body, {
        status: result.status,
        statusText: result.statusText,
        headers: result.headers || {},
        url: result.url || url,
        type: 'basic'
      }));
    });
  };
})();
"#;

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
        } else {
            break;
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
                    '/' => s.push('/'),
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
    None // Unterminated string.
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
                ..FetchResponse::default()
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
                ..FetchResponse::default()
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
                ..FetchResponse::default()
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
                ..FetchResponse::default()
            },
        );
        mock.add(
            "https://example.com/fail",
            FetchResponse {
                status: 500,
                headers: HashMap::new(),
                body: String::new(),
                ..FetchResponse::default()
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
                ..FetchResponse::default()
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
                ..FetchResponse::default()
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
                ..FetchResponse::default()
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

    #[test]
    fn fetch_response_surface() {
        let mut mock = MockFetchHandler::new();
        let mut headers = HashMap::new();
        headers.insert("X-Thing".into(), "1".into());
        mock.add(
            "https://example.com/r",
            FetchResponse {
                status: 404,
                headers,
                body: "gone".into(),
                ..FetchResponse::default()
            },
        );
        let engine = engine_with_mock(mock);
        engine
            .eval(
                "fetch('https://example.com/r').then(function(r) {\
                     var c = r.clone();\
                     console.log(r.statusText + '|' + r.url + '|' + r.headers.has('x-thing')\
                         + '|' + (r instanceof Response));\
                     return r.text().then(function(t) {\
                         console.log(t + '|' + r.bodyUsed);\
                         return r.text();\
                     }).catch(function(e) { console.log('reuse:' + e.name); })\
                       .then(function() { return c.text(); })\
                       .then(function(t2) { console.log('clone:' + t2); });\
                 })",
            )
            .expect("eval");
        let msgs: Vec<String> = engine
            .console_output()
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(
            msgs,
            vec![
                "Not Found|https://example.com/r|true|true",
                "gone|true",
                "reuse:TypeError",
                "clone:gone",
            ]
        );
    }

    #[test]
    fn fetch_get_with_body_rejects_and_headers_object_accepted() {
        let engine = engine_with_mock(MockFetchHandler::new());
        engine
            .eval(
                "fetch('https://example.com', { body: 'x' })\
                 .catch(function(e) { console.log('body:' + e.name); });\
                 fetch('https://example.com', { headers: new Headers([['A', 'b']]) })\
                 .then(function(r) { console.log('status:' + r.status); });",
            )
            .expect("eval");
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.message == "body:TypeError"));
        assert!(out.iter().any(|e| e.message == "status:404"));
    }

    #[test]
    fn bind_fetch_handler_replaces_transport_in_context() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).expect("engine");
        let mut mock = MockFetchHandler::new();
        mock.add(
            "/rel",
            FetchResponse {
                status: 200,
                body: "bound".into(),
                ..FetchResponse::default()
            },
        );
        engine
            .with_context(|ctx| bind_fetch_handler(&ctx, Box::new(mock)))
            .expect("bind");
        engine
            .eval(
                "fetch('/rel').then(function(r) { return r.text(); })\
                 .then(function(t) { console.log(t); })",
            )
            .expect("eval");
        assert!(engine.console_output().iter().any(|e| e.message == "bound"));
    }
}
