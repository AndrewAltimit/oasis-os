//! Hostile / degenerate input: the browser must not panic, overflow the
//! stack, or hang, and must stay usable afterwards.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oasis_types::input::{Button, InputEvent};

use super::harness::{DEADLINE, Reply, Session, TestServer};

/// Serve `body` at `/page` and `<p>AfterOk</p>` at `/ok`; load the page
/// in a fresh session on a thread with `stack` bytes of stack, drive a
/// few inputs, then check the browser still navigates. Returns the
/// load time.
fn survive(body: String, stack: usize) -> Duration {
    let body = Arc::new(body);
    let server = TestServer::start(move |req| match req.path() {
        "/page" => Reply::html(body.as_bytes().to_vec()),
        _ => Reply::html("<html><body><p>AfterOk</p></body></html>"),
    });
    let url = server.url("/page");
    let ok = server.url("/ok");
    let handle = std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let mut s = Session::new();
            let start = Instant::now();
            s.open(&url);
            let took = start.elapsed();
            s.press(Button::Down);
            s.input(InputEvent::MouseWheel { delta: 3 });
            s.click(100, 100);
            s.input(InputEvent::CursorMove { x: 50, y: 60 });
            s.press(Button::Right);
            s.open(&ok);
            s.assert_shows("AfterOk");
            took
        })
        .unwrap();
    let took = handle.join().expect("browser thread panicked");
    drop(server);
    took
}

/// Stack available to the shell's UI thread on the tightest desktop
/// target (Windows main thread: 1 MiB) for optimized builds. Unoptimized
/// builds use several times more stack per frame, so debug test runs
/// get 4 MiB (still half of a Linux main thread).
const UI_STACK: usize = if cfg!(debug_assertions) {
    4 * 1024 * 1024
} else {
    1024 * 1024
};

#[test]
fn ten_thousand_nested_divs() {
    let mut html = String::from("<html><body>");
    html.push_str(&"<div style=\"padding-left:1px\">".repeat(10_000));
    html.push_str("DeepText");
    html.push_str(&"</div>".repeat(10_000));
    html.push_str("</body></html>");
    let took = survive(html, UI_STACK);
    assert!(took < DEADLINE, "took {took:?}");
}

#[test]
fn deeply_nested_inline_and_unclosed_tags() {
    let mut html = String::from("<html><body><p>");
    html.push_str(&"<b><i><span><a href=#x>".repeat(3_000));
    html.push_str("InlineDeep");
    // Never closed; plus a pile of stray end tags and junk.
    html.push_str(&"</td></tr></table></select>".repeat(2_000));
    html.push_str("<table><tr><td><table><tr><td>".repeat(500).as_str());
    let took = survive(html, UI_STACK);
    assert!(took < DEADLINE, "took {took:?}");
}

#[test]
fn giant_attribute_and_giant_text_run() {
    let mut html = String::from("<html><body><div title=\"");
    html.push_str(&"a".repeat(4 * 1024 * 1024));
    html.push_str("\" data-x='");
    html.push_str(&"<\"&amp;".repeat(100_000));
    html.push_str("'>GiantAttr</div><p>");
    html.push_str(&"wordwithoutanybreakopportunity".repeat(20_000));
    html.push_str("</p></body></html>");
    let took = survive(html, UI_STACK);
    assert!(took < DEADLINE, "took {took:?}");
}

#[test]
fn pathological_css() {
    let mut css = String::new();
    // Deeply nested blocks / nesting rules.
    css.push_str(&"div { ".repeat(3_000));
    css.push_str("color: red;");
    css.push_str(&"} ".repeat(3_000));
    // Unbalanced garbage.
    css.push_str(&"{{{{ ;;; }} @media (((( ".repeat(2_000));
    // Deeply nested selector functions and calc().
    css.push_str(&":is(".repeat(500));
    css.push('p');
    css.push_str(&")".repeat(500));
    css.push_str(" { width: ");
    css.push_str(&"calc(1px + ".repeat(500));
    css.push_str("1px");
    css.push_str(&")".repeat(500));
    css.push_str("; }\n");
    // var() cycles and long chains.
    css.push_str(":root { --a: var(--b); --b: var(--a); }\n");
    for i in 0..500 {
        css.push_str(&format!(
            ":root {{ --v{i}: var(--v{}) var(--v{}); }}\n",
            i + 1,
            i + 1
        ));
    }
    css.push_str("p { color: var(--a); margin: var(--v0); }\n");
    // A huge selector list.
    let list: Vec<String> = (0..20_000).map(|i| format!(".c{i}")).collect();
    css.push_str(&list.join(","));
    css.push_str(" { color: blue; }\n");
    let html = format!(
        "<html><head><style>{css}</style></head><body>\
         <p style=\"{inline}\">CssTorture</p></body></html>",
        inline = "color:red;".repeat(10_000),
    );
    let took = survive(html, UI_STACK);
    assert!(took < DEADLINE, "took {took:?}");
}

#[test]
fn binary_garbage_and_truncated_markup() {
    let mut bytes: Vec<u8> = (0..200_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    bytes.extend_from_slice(b"<html><body><p>tail<script>var x = '");
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let took = survive(body, UI_STACK);
    assert!(took < DEADLINE, "took {took:?}");
}

#[test]
fn many_elements_page() {
    // 20k table cells + 20k links: exercises layout, the link map and
    // hit testing at scale.
    let mut html = String::from("<html><body><table>");
    for r in 0..1_000 {
        html.push_str("<tr>");
        for c in 0..20 {
            html.push_str(&format!("<td><a href=\"/l{r}_{c}\">{r}.{c}</a></td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table></body></html>");
    let took = survive(html, UI_STACK);
    assert!(took < Duration::from_secs(20), "took {took:?}");
}

#[test]
fn malformed_urls_do_not_panic() {
    let mut s = Session::new();
    for url in [
        "",
        "http://",
        "http://:0/",
        "http://[::1/",
        "http://127.0.0.1:99999/",
        "http://127.0.0.1:0/",
        "://nohost",
        "gemini://",
        "data:text/html,<p>DataUrlPage</p>",
        "data:;base64,@@@@",
        "javascript:alert(1)",
        "vfs://../../etc/passwd",
        "http://ex%41mple.test/\u{0}\u{7f}",
        "http://a.b/\u{1F600}?q=\u{1F600}#\u{1F600}",
    ] {
        s.open(url);
        assert_ne!(
            s.browser.loading_state(),
            crate::LoadingState::Loading,
            "{url:?} left the browser loading"
        );
    }
}
