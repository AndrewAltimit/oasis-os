//! Link clicks, history, reload, fragments and error pages.

use oasis_types::input::{Button, InputEvent, Trigger};

use super::harness::{Reply, Session, TestServer};
use crate::LoadingState;

fn site() -> TestServer {
    TestServer::start(|req| match req.path() {
        "/" => Reply::html(
            "<html><head><title>Home</title></head><body>\
             <h1>HomePage</h1>\
             <p><a href=\"/second\">SecondLink</a></p>\
             <p><a href=\"third?next=http://example.test/x\">ThirdLink</a></p>\
             </body></html>",
        ),
        "/second" => Reply::html(
            "<html><head><title>Second</title></head><body>\
             <p>SecondPage</p><a href=\"/\">HomeLink</a></body></html>",
        ),
        "/third" => Reply::html("<html><body><p>ThirdPage</p></body></html>"),
        "/long" => {
            let mut html = String::from(
                "<html><body><p><a href=\"#target\">JumpLink</a></p>\
                 <p><a href=\"/second\">SecondLink</a></p>",
            );
            for i in 0..120 {
                html.push_str(&format!("<p>filler line {i}</p>"));
            }
            html.push_str("<h2 id=\"target\">TargetHeading</h2><p>after target</p>");
            for i in 0..60 {
                html.push_str(&format!("<p>tail line {i}</p>"));
            }
            html.push_str("<p><a href=\"/second\">BottomLink</a></p></body></html>");
            Reply::html(html)
        },
        "/boom" => Reply::with(
            500,
            &[("Content-Type", "text/html")],
            "<html><body><p>ServerExploded</p></body></html>",
        ),
        _ => Reply::not_found(),
    })
}

#[test]
fn click_link_back_forward_reload() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.assert_shows("HomePage");
    assert_eq!(s.browser.title(), Some("Home"));
    assert_eq!(s.browser.loading_state(), LoadingState::Idle);

    s.click_text("SecondLink");
    assert_eq!(s.url(), server.url("/second"));
    s.assert_shows("SecondPage");
    assert!(!s.shows("HomePage"));
    // The server saw a Referer for the in-page navigation.
    let second = server.requests_to("/second");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].header("referer"), Some(server.url("/").as_str()));

    // Back (Cancel button) restores the first page.
    s.press(Button::Cancel);
    assert_eq!(s.url(), server.url("/"));
    s.assert_shows("HomePage");
    assert!(s.browser.navigation().can_go_forward());

    // Forward via the chrome's forward button (second 28px button).
    s.click(28 + 10, 10);
    assert_eq!(s.url(), server.url("/second"));
    s.assert_shows("SecondPage");

    // Back/forward are served from cache: still only one fetch.
    assert_eq!(server.requests_to("/second").len(), 1);

    // Reload through the URL bar: click it, confirm.
    s.click(200, 10);
    assert!(s.browser.accepts_text(), "URL bar should take focus");
    s.press(Button::Confirm);
    assert_eq!(s.url(), server.url("/second"));
    s.assert_shows("SecondPage");
    assert_eq!(
        server.requests_to("/second").len(),
        2,
        "reload must hit the network"
    );
}

#[test]
fn typed_url_navigates() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click(200, 10);
    // Typing replaces the selected URL.
    s.type_str(&server.url("/third"));
    s.press(Button::Confirm);
    assert_eq!(s.url(), server.url("/third"));
    s.assert_shows("ThirdPage");
}

#[test]
fn relative_link_with_url_in_query_stays_on_origin() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click_text("ThirdLink");
    s.assert_shows("ThirdPage");
    let reqs = server.requests_to("/third");
    assert_eq!(reqs.len(), 1, "link should resolve against the page URL");
    assert_eq!(reqs[0].query(), Some("next=http://example.test/x"));
}

#[test]
fn fragment_link_scrolls_without_refetch() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/long"));
    assert!(!s.shows("TargetHeading"), "target starts below the fold");
    assert_eq!(s.browser.scroll().scroll_y, 0);

    s.click_text("JumpLink");
    s.assert_shows("TargetHeading");
    assert!(s.browser.scroll().scroll_y > 0);
    assert_eq!(s.url(), format!("{}#target", server.url("/long")));
    assert_eq!(
        server.requests_to("/long").len(),
        1,
        "same-document fragment navigation must not refetch"
    );

    // Back returns to the top of the same document.
    s.press(Button::Cancel);
    assert_eq!(s.url(), server.url("/long"));
    assert_eq!(s.browser.scroll().scroll_y, 0);
    s.assert_shows("JumpLink");

    // Forward lands on the fragment again, still without a refetch.
    s.click(28 + 10, 10);
    s.assert_shows("TargetHeading");
    assert_eq!(server.requests_to("/long").len(), 1);
}

#[test]
fn back_restores_scroll_position() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/long"));
    // Page down until the bottom link is visible.
    for _ in 0..40 {
        if s.shows("BottomLink") {
            break;
        }
        s.input(InputEvent::TriggerPress(Trigger::Right));
    }
    s.assert_shows("BottomLink");
    let saved = s.browser.scroll().scroll_y;
    assert!(saved > 0);

    s.click_text("BottomLink");
    s.assert_shows("SecondPage");
    assert_eq!(s.browser.scroll().scroll_y, 0);

    s.press(Button::Cancel);
    assert_eq!(s.url(), server.url("/long"));
    assert_eq!(
        s.browser.scroll().scroll_y,
        saved,
        "back must restore the scroll offset"
    );
    s.assert_shows("BottomLink");
}

#[test]
fn loading_url_with_fragment_scrolls_to_target() {
    let server = site();
    let mut s = Session::new();
    s.open(&format!("{}#target", server.url("/long")));
    s.assert_shows("TargetHeading");
    // The fragment is never sent on the wire.
    let reqs = server.requests_to("/long");
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].target, "/long");
}

#[test]
fn http_error_statuses_render_server_body() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/does-not-exist"));
    s.assert_shows("nothing-here-404");
    s.open(&server.url("/boom"));
    s.assert_shows("ServerExploded");
    // The browser stays usable afterwards.
    s.open(&server.url("/"));
    s.assert_shows("HomePage");
}

#[test]
fn connection_refused_shows_error_page_and_keeps_url() {
    // Grab a free port, then close it so nothing is listening.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dead = format!("http://127.0.0.1:{port}/gone");
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.open(&dead);
    assert_eq!(s.browser.loading_state(), LoadingState::Error);
    assert!(s.browser.error_message().is_some());
    assert!(!s.content_text().is_empty(), "an error page is rendered");
    assert_eq!(s.url(), dead, "the failed URL stays in the URL bar");
    // Back still returns to the page before the failure.
    s.press(Button::Cancel);
    assert_eq!(s.url(), server.url("/"));
    s.assert_shows("HomePage");
}

/// TLS provider that hands the TCP stream through untouched, so a
/// plain-TCP test server can speak Gemini.
struct PlainTls;

impl oasis_net::tls::TlsProvider for PlainTls {
    fn connect_tls(
        &self,
        stream: Box<dyn oasis_types::backend::NetworkStream>,
        _server_name: &str,
    ) -> oasis_types::error::Result<Box<dyn oasis_types::backend::NetworkStream>> {
        Ok(stream)
    }
}

/// Minimal Gemini server: one request line per connection.
fn gemini_server(respond: fn(&str) -> String) -> u16 {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut reader = BufReader::new(conn);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut conn = reader.into_inner();
            let _ = conn.write_all(respond(line.trim_end()).as_bytes());
        }
    });
    port
}

#[test]
fn gemini_capsule_navigation() {
    let port = gemini_server(|url| {
        if url.ends_with("/next") {
            "20 text/gemini\r\n# SecondCapsule\nReachedNext\n".to_string()
        } else if url.ends_with("/moved") {
            "31 /next\r\n".to_string()
        } else if url.ends_with("/missing") {
            "51 NotHere\r\n".to_string()
        } else {
            "20 text/gemini\r\n# CapsuleHome\nplain line\n=> /next NextCapsule\n\
             * bullet\n```\npreformatted <b>raw</b>\n```\n"
                .to_string()
        }
    });
    let base = format!("gemini://127.0.0.1:{port}");
    let mut s = Session::new();
    s.browser.set_tls_provider(Box::new(PlainTls));
    s.open(&format!("{base}/"));
    s.assert_shows("CapsuleHome");
    s.assert_shows("<b>raw</b>");
    assert_eq!(s.browser.title(), Some("CapsuleHome"));

    s.click_text("NextCapsule");
    assert_eq!(s.url(), format!("{base}/next"));
    s.assert_shows("ReachedNext");

    s.press(Button::Cancel);
    s.assert_shows("CapsuleHome");

    // Redirect status and an error status.
    s.open(&format!("{base}/moved"));
    s.assert_shows("ReachedNext");
    s.open(&format!("{base}/missing"));
    s.assert_shows("NotHere");
}

#[test]
fn keyboard_link_navigation() {
    let server = site();
    let mut s = Session::new();
    s.open(&server.url("/"));
    // Right selects the first link; Confirm activates it.
    s.press(Button::Right);
    s.press(Button::Confirm);
    assert_eq!(s.url(), server.url("/second"));
    s.assert_shows("SecondPage");
}
