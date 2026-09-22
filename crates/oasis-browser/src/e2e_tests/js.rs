//! JavaScript sessions: event-driven DOM mutation, timers, fetch policy,
//! Web Storage partitioning, script navigation and the watchdog.

use std::time::{Duration, Instant};

use super::harness::{Reply, Session, TestServer, form_pairs};
use crate::layout::box_model::ReplacedContent;

fn html(body: &str) -> Reply {
    Reply::html(format!("<html><body>{body}</body></html>"))
}

#[test]
fn click_handler_mutation_rerenders() {
    let server = TestServer::start(|_| {
        html(
            "<p><button id=\"b\" onclick=\"document.getElementById('out').textContent = \
             'AfterClick'; this.textContent = 'Pressed'\">PressMe</button></p>\
             <p id=\"out\">BeforeClick</p>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.assert_shows("BeforeClick");
    s.click_element("b");
    s.assert_shows("AfterClick");
    assert!(!s.shows("BeforeClick"));
}

#[test]
fn js_mutation_keeps_external_stylesheet_rules() {
    let server = TestServer::start(|req| match req.path() {
        "/s.css" => Reply::with(
            200,
            &[("Content-Type", "text/css")],
            ".gone { display: none; }",
        ),
        _ => Reply::html(
            "<html><head><link rel=\"stylesheet\" href=\"/s.css\"></head><body>\
             <p class=\"gone\">HiddenByLinkedCss</p>\
             <p><button id=\"b\" onclick=\"document.getElementById('o').textContent = \
             'Mutated'\">B</button></p><p id=\"o\">Original</p></body></html>",
        ),
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    assert!(!s.shows("HiddenByLinkedCss"), "linked sheet applied");
    s.click_element("b");
    s.assert_shows("Mutated");
    assert!(
        !s.shows("HiddenByLinkedCss"),
        "the re-cascade after a JS mutation must keep linked-sheet rules"
    );
}

#[test]
fn set_timeout_fires_after_ticks_and_rerenders() {
    let server = TestServer::start(|_| {
        html(
            "<p id=\"t\">TimerPending</p>\
             <script>setTimeout(function () {\
               document.getElementById('t').textContent = 'TimerFired';\
             }, 100);</script>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.assert_shows("TimerPending");
    s.run_for(350);
    s.assert_shows("TimerFired");
}

#[test]
fn set_interval_updates_accumulate() {
    let server = TestServer::start(|_| {
        html(
            "<p id=\"n\">count0</p>\
             <script>var n = 0; var id = setInterval(function () {\
               n++; document.getElementById('n').textContent = 'count' + n;\
               if (n == 3) clearInterval(id);\
             }, 30);</script>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.run_for(400);
    s.assert_shows("count3");
}

#[test]
fn fetch_same_origin_allowed_cross_origin_blocked() {
    let other = TestServer::start(|req| match req.path() {
        "/open" => Reply::with(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            "OpenData",
        ),
        _ => Reply::with(200, &[("Content-Type", "text/plain")], "SecretData"),
    });
    let other_origin = other.origin();
    let server = TestServer::start(move |req| match req.path() {
        "/data" => Reply::with(200, &[("Content-Type", "text/plain")], "SameOriginData"),
        _ => html(&format!(
            "<p id=\"a\">a-pending</p><p id=\"b\">b-pending</p><p id=\"c\">c-pending</p>\
             <script>\
             function show(id, v) {{ document.getElementById(id).textContent = v; }}\
             fetch('/data').then(function (r) {{ return r.text(); }})\
               .then(function (t) {{ show('a', 'got-' + t); }},\
                     function (e) {{ show('a', 'a-failed'); }});\
             fetch('{other_origin}/secret').then(function (r) {{ return r.text(); }})\
               .then(function (t) {{ show('b', 'LEAKED-' + t); }},\
                     function (e) {{ show('b', 'b-blocked'); }});\
             fetch('{other_origin}/open').then(function (r) {{ return r.text(); }})\
               .then(function (t) {{ show('c', 'cors-' + t); }},\
                     function (e) {{ show('c', 'c-failed'); }});\
             </script>"
        )),
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.run_for(100);
    s.assert_shows("got-SameOriginData");
    s.assert_shows("b-blocked");
    assert!(!s.shows("LEAKED"));
    s.assert_shows("cors-OpenData");
}

#[test]
fn local_storage_persists_per_origin() {
    let page = |req: &super::harness::Req| match req.path() {
        "/set" => html(
            "<script>localStorage.setItem('k', 'StoredValue');</script>\
             <p>SetDone</p><a href=\"/get\">GetLink</a>",
        ),
        _ => html(
            "<p id=\"v\">x</p><script>document.getElementById('v').textContent = \
             'value-' + (localStorage.getItem('k') || 'none');</script>",
        ),
    };
    let a = TestServer::start(page);
    let b = TestServer::start(page);
    let mut s = Session::new();
    s.open(&a.url("/set"));
    s.assert_shows("SetDone");
    // Same origin, new document.
    s.click_text("GetLink");
    s.assert_shows("value-StoredValue");
    // A different origin (different port) cannot see it.
    s.open(&b.url("/get"));
    s.assert_shows("value-none");
    // Coming back to the first origin, it is still there.
    s.open(&a.url("/get"));
    s.assert_shows("value-StoredValue");
}

#[test]
fn document_title_set_by_script_reaches_the_chrome() {
    let server = TestServer::start(|_| {
        html(
            "<script>document.title = 'LoadTitle';</script>\
             <p><button id=\"b\" onclick=\"document.title = 'ClickTitle'\">T</button></p>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    assert_eq!(s.browser.title(), Some("LoadTitle"));
    s.click_element("b");
    assert_eq!(s.browser.title(), Some("ClickTitle"));
}

#[test]
fn script_navigation_via_location() {
    let server = TestServer::start(|req| match req.path() {
        "/start" => {
            html("<p><button id=\"go\" onclick=\"location.href = '/dest'\">GoButton</button></p>")
        },
        "/dest" => html("<p>Destination</p>"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/start"));
    s.click_element("go");
    s.assert_shows("Destination");
    assert_eq!(s.url(), server.url("/dest"));
}

#[test]
fn runaway_scripts_are_interrupted_and_browser_stays_usable() {
    let server = TestServer::start(|req| match req.path() {
        "/spin" => html(
            "<p id=\"s\">SpinPage</p>\
             <p id=\"m\">x</p>\
             <script>document.getElementById('s').textContent = 'SpinStarted';\
             while (true) {}</script>\
             <script>document.getElementById('m').textContent = 'SecondScriptRan';</script>",
        ),
        "/after" => html(
            "<p id=\"o\">x</p><script>document.getElementById('o').textContent = \
             'JsStillWorks';</script>",
        ),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    let start = Instant::now();
    s.open(&server.url("/spin"));
    assert!(
        start.elapsed() < Duration::from_secs(9),
        "watchdog should stop the loop, took {:?}",
        start.elapsed()
    );
    // Work done before the loop is kept; later scripts still run.
    s.assert_shows("SpinStarted");
    s.assert_shows("SecondScriptRan");
    s.open(&server.url("/after"));
    s.assert_shows("JsStillWorks");
}

// -- Form state shared between the user and scripts --------------------

/// A page with a text field `q` in a POST form, plus buttons whose
/// handlers read, overwrite, or merely touch unrelated DOM.
fn form_script_server() -> TestServer {
    TestServer::start(|req| match req.path() {
        "/" => html(
            "<form action=\"/post\" method=\"post\">\
               <p><label for=\"q\">FieldLabel</label> \
                  <input type=\"text\" id=\"q\" name=\"q\" style=\"width:200px\" \
                   oninput=\"document.getElementById('echo').textContent = 'echo-' + this.value\">\
                  <input type=\"hidden\" id=\"tok\" name=\"tok\" value=\"orig\"></p>\
             </form>\
             <p><button id=\"touch\" onclick=\"document.getElementById('log').textContent = \
               'touched'\">Touch</button>\
                <button id=\"read\" onclick=\"document.getElementById('log').textContent = \
               'read-' + document.getElementById('q').value\">Read</button>\
                <button id=\"set\" onclick=\"document.getElementById('q').value = 'preset'; \
               document.getElementById('tok').value = 'jstoken'\">Set</button></p>\
             <p id=\"log\">log-empty</p><p id=\"echo\">echo-none</p>",
        ),
        "/post" => {
            let pairs = form_pairs(&String::from_utf8_lossy(&req.body));
            let q = pairs
                .iter()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.as_str());
            let tok = pairs
                .iter()
                .find(|(k, _)| k == "tok")
                .map(|(_, v)| v.as_str());
            html(&format!(
                "<p>posted-q-{}</p><p>posted-tok-{}</p>",
                q.unwrap_or("none"),
                tok.unwrap_or("none")
            ))
        },
        _ => Reply::not_found(),
    })
}

fn text_value(s: &Session, id: &str) -> String {
    match s.replaced(id) {
        ReplacedContent::TextInput { value, .. } => value,
        other => panic!("#{id} is not a text input: {other:?}"),
    }
}

/// Typed text used to live only in the form manager and the rendered
/// DOM, not in the script-side DOM. Any script mutation (here: a click
/// handler touching an unrelated node) replaced the rendered DOM with
/// the script-side copy and visually wiped the typed value.
#[test]
fn script_mutation_after_typing_keeps_the_typed_value() {
    let server = form_script_server();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click_text("FieldLabel");
    s.type_str("hello");
    assert_eq!(text_value(&s, "q"), "hello");
    s.click_element("touch");
    s.assert_shows("touched");
    assert_eq!(
        text_value(&s, "q"),
        "hello",
        "typed value survives the mutation"
    );
}

/// Scripts must read what the user typed (`input.value`), and `input`
/// handlers run after the edit.
#[test]
fn scripts_read_the_typed_value() {
    let server = form_script_server();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click_text("FieldLabel");
    s.type_str("abc");
    s.assert_shows("echo-abc");
    s.click_element("read");
    s.assert_shows("read-abc");
}

/// A value set by script (`input.value = ...`) is what the user then
/// edits and what gets submitted; it used to be overwritten by the form
/// manager's stale copy on the next keystroke and ignored on submit.
#[test]
fn script_set_values_are_edited_and_submitted() {
    let server = form_script_server();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click_element("set");
    assert_eq!(text_value(&s, "q"), "preset");
    s.click_text("FieldLabel");
    s.type_str("X");
    assert_eq!(text_value(&s, "q"), "presetX");
    s.press(oasis_types::input::Button::Confirm);
    s.assert_shows("posted-q-presetX");
    s.assert_shows("posted-tok-jstoken");
}
