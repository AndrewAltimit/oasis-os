//! Request policy: which requests the server answers, and how.
//!
//! Every request passes [`gate`] (Origin / Host / bearer token / route /
//! media type) twice: once as soon as its header section is in, before any
//! body is buffered, and again in [`handle_http_request`] on the complete
//! request, which then routes it to the JSON-RPC dispatcher.

use crate::dispatch::{Handled, handle_message};
use crate::http::{HttpRequest, RequestHead, build_response};
use crate::tools::ToolDispatcher;

fn path_matches(path: &str) -> bool {
    path == "/mcp" || path == "/" || path.starts_with("/mcp?")
}

/// Whether `authority` (a `Host` value or the host part of an `Origin`) names
/// the loopback interface: `127.0.0.1`, `localhost` or `[::1]`, optionally
/// followed by `:port`.
pub(crate) fn is_loopback_authority(authority: &str) -> bool {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: `[::1]` or `[::1]:port`.
        let Some((h, after)) = rest.split_once(']') else {
            return false;
        };
        if !after.is_empty() && !after.starts_with(':') {
            return false;
        }
        (format!("[{h}]"), after.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    if let Some(p) = port
        && (p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "[::1]"
}

/// `Origin` check: absent (non-browser clients) is allowed; otherwise it must
/// be an `http`/`https` origin on a loopback authority. `Origin: null`
/// (sandboxed iframes, `file://`) is rejected.
fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let lower = origin.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    rest.is_some_and(is_loopback_authority)
}

/// `Host` check (DNS-rebinding defence): a rebound hostname such as
/// `attacker.example:7345` resolves to 127.0.0.1 but still carries the
/// attacker's name in `Host`, so only loopback names are accepted. Absent
/// `Host` (HTTP/1.0 tooling) is allowed; browsers always send it.
fn host_allowed(host: Option<&str>) -> bool {
    host.is_none_or(is_loopback_authority)
}

/// Whether a `Content-Type` value is `application/json` (parameters such as
/// `; charset=utf-8` are allowed).
fn is_json_content_type(ct: Option<&str>) -> bool {
    ct.is_some_and(|v| {
        let media = v.split(';').next().unwrap_or("").trim();
        media.eq_ignore_ascii_case("application/json")
    })
}

/// The checks that depend only on the header section, in order: Origin /
/// Host (browser attacks: a cross-site page can POST to loopback -- simple
/// `text/plain` requests skip CORS preflight -- and DNS rebinding can make a
/// foreign hostname resolve to 127.0.0.1), the bearer token, then the
/// route and media type of a `POST`. Returns the refusal status, or `None`
/// when the request may proceed.
///
/// The server runs this as soon as the headers are in, before buffering a
/// body or sending `100 Continue`, and again on the complete request.
pub(crate) fn gate(head: &RequestHead, token: Option<&str>) -> Option<u16> {
    if !origin_allowed(head.origin.as_deref()) || !host_allowed(head.host.as_deref()) {
        return Some(403);
    }
    if let Some(expected) = token {
        let ok = head
            .auth_bearer
            .as_deref()
            .is_some_and(|got| oasis_types::util::ct_eq(got.as_bytes(), expected.as_bytes()));
        if !ok {
            return Some(401);
        }
    }
    if head.method == "POST" {
        if !path_matches(&head.path) {
            return Some(404);
        }
        // Requiring `application/json` forces a CORS preflight for any
        // cross-origin browser request (which we never approve).
        if !is_json_content_type(head.content_type.as_deref()) {
            return Some(415);
        }
    }
    None
}

/// The response for a [`gate`] refusal.
pub(crate) fn refusal(status: u16, keep_alive: bool) -> Vec<u8> {
    let extra: &[(&str, &str)] = if status == 401 {
        &[("WWW-Authenticate", "Bearer")]
    } else {
        &[]
    };
    build_response(status, keep_alive, extra, None, b"")
}

/// Methods advertised in `Allow` (no server-initiated SSE stream, no
/// sessions to delete).
const ALLOW: (&str, &str) = ("Allow", "POST, OPTIONS");

/// Answer one complete request. Returns the response bytes and whether the
/// connection stays open.
///
/// A `403` always closes it, and so does a `401`: otherwise one keep-alive
/// connection could pipeline hundreds of token guesses into every poll.
/// Closing bounds a guesser to one attempt per accepted connection (and
/// the server accepts one connection per poll).
pub(crate) fn handle_http_request(
    req: &HttpRequest,
    disp: &mut dyn ToolDispatcher,
    token: Option<&str>,
) -> (Vec<u8>, bool) {
    let keep_alive = req.head.keep_alive;
    if let Some(status) = gate(&req.head, token) {
        let keep_alive = keep_alive && !matches!(status, 401 | 403);
        return (refusal(status, keep_alive), keep_alive);
    }
    match req.head.method.as_str() {
        "POST" => match handle_message(&req.body, disp) {
            Handled::Notification => (build_response(202, keep_alive, &[], None, b""), keep_alive),
            Handled::Response(body) => (
                build_response(200, keep_alive, &[], Some("application/json"), &body),
                keep_alive,
            ),
        },
        "OPTIONS" => (
            build_response(204, keep_alive, &[ALLOW], None, b""),
            keep_alive,
        ),
        // GET (no SSE stream), DELETE (no sessions are issued) and anything
        // else: 405 is spec-permitted for both GET and DELETE.
        _ => (
            build_response(405, keep_alive, &[ALLOW], None, b""),
            keep_alive,
        ),
    }
}
