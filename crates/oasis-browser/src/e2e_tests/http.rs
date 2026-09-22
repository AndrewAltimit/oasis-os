//! HTTP transport behaviour seen through the browser: redirects, error
//! statuses, slow / truncated / oversized / compressed / chunked
//! bodies, content types, caching and cookies.

use std::io::Write;
use std::time::{Duration, Instant};

use super::harness::{Reply, Session, Step, TestServer};
use crate::LoadingState;

fn page(text: &str) -> Reply {
    Reply::html(format!("<html><body><p>{text}</p></body></html>"))
}

#[test]
fn redirect_chain_lands_on_final_url() {
    let server = TestServer::start(|req| match req.path() {
        "/a" => Reply::redirect(301, "/b"),
        "/b" => Reply::redirect(302, "c"),
        "/c" => page("FinalStop"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/a"));
    s.assert_shows("FinalStop");
    assert_eq!(s.url(), server.url("/c"));
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn five_redirects_are_followed_and_loops_are_capped() {
    // /r5 -> /r4 -> ... -> /r1 -> /r0 (five redirects, the documented
    // maximum), and /loop -> /loop forever.
    let server = TestServer::start(|req| {
        let p = req.path();
        if p == "/loop" {
            return Reply::redirect(302, "/loop");
        }
        match p.strip_prefix("/r").and_then(|n| n.parse::<u32>().ok()) {
            Some(0) => page("ChainDone"),
            Some(n) => Reply::redirect(302, &format!("/r{}", n - 1)),
            None => Reply::not_found(),
        }
    });
    let mut s = Session::new();
    s.open(&server.url("/r5"));
    s.assert_shows("ChainDone");

    let start = Instant::now();
    s.open(&server.url("/loop"));
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(s.browser.loading_state(), LoadingState::Error);
    assert!(
        s.browser
            .error_message()
            .is_some_and(|m| m.contains("redirect")),
        "{:?}",
        s.browser.error_message()
    );
    let loops = server.requests_to("/loop").len();
    assert!(
        (2..=21).contains(&loops),
        "redirect loop made {loops} requests"
    );
    assert_eq!(s.url(), server.url("/loop"));
}

#[test]
fn post_then_303_redirect_becomes_get() {
    let server = TestServer::start(|req| match (req.method.as_str(), req.path()) {
        ("GET", "/form") => Reply::html(
            "<html><body><form method=post action=/submit>\
             <input type=hidden name=k value=v>\
             <input type=submit id=b value=Save></form></body></html>",
        ),
        ("POST", "/submit") => Reply::redirect(303, "/done"),
        ("GET", "/done") => page("SavedOk"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/form"));
    s.click_element("b");
    s.assert_shows("SavedOk");
    let done = server.requests_to("/done");
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].method, "GET");
    assert!(done[0].body.is_empty());
}

#[test]
fn gzip_and_deflate_bodies_are_decoded() {
    fn gz(s: &str) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    }
    fn deflate(s: &str) -> Vec<u8> {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    }
    let server = TestServer::start(|req| match req.path() {
        "/gz" => Reply::with(
            200,
            &[("Content-Type", "text/html"), ("Content-Encoding", "gzip")],
            gz("<html><body><p>GzipWorks</p></body></html>"),
        ),
        "/df" => Reply::with(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Encoding", "deflate"),
            ],
            deflate("<html><body><p>DeflateWorks</p></body></html>"),
        ),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/gz"));
    s.assert_shows("GzipWorks");
    s.open(&server.url("/df"));
    s.assert_shows("DeflateWorks");
    // The browser advertised the encodings it can decode.
    let ae = server.requests()[0]
        .header("accept-encoding")
        .unwrap()
        .to_string();
    assert!(ae.contains("gzip"), "{ae}");
}

/// Encode one chunk of a chunked body.
fn chunk(data: &str) -> Vec<u8> {
    format!("{:x}\r\n{data}\r\n", data.len()).into_bytes()
}

#[test]
fn chunked_body_split_across_slow_writes() {
    let server = TestServer::start(|req| match req.path() {
        "/chunked" => Reply::script(
            vec![
                Step::Write(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                      Transfer-Encoding: chunked\r\n\r\n"
                        .to_vec(),
                ),
                // A chunk whose data happens to end in "0\r\n" — the
                // tail then looks like a terminating zero chunk.
                Step::Write(chunk("<html><body><p>ChunkOne total 10\r\n")),
                Step::Sleep(150),
                Step::Write(chunk("</p><p>ChunkTwo</p>")),
                Step::Sleep(50),
                Step::Write(chunk("<p>ChunkThree</p></body></html>")),
                Step::Write(b"0\r\n\r\n".to_vec()),
            ],
            false,
        ),
        "/after" => page("AfterChunked"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/chunked"));
    s.assert_shows("ChunkOne");
    s.assert_shows("ChunkTwo");
    s.assert_shows("ChunkThree");
    // The kept-alive connection is still in a clean state.
    s.open(&server.url("/after"));
    s.assert_shows("AfterChunked");
}

#[test]
fn slow_response_keeps_the_frame_loop_responsive() {
    let server = TestServer::start(|req| match req.path() {
        "/slow" => Reply::script(
            vec![
                Step::Write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n".to_vec()),
                Step::Sleep(400),
                Step::Write({
                    let body = "<html><body><p>SlowPage</p></body></html>";
                    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
                }),
            ],
            true,
        ),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.browser.navigate_vfs(&server.url("/slow"), &s.vfs);
    // While the load is in flight every frame returns promptly.
    let t = Instant::now();
    s.browser.tick(&s.vfs);
    let mut backend = crate::test_utils::MockBackend::new();
    s.browser.paint(&mut backend).unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "frame blocked on I/O"
    );
    assert_eq!(s.browser.loading_state(), LoadingState::Loading);
    assert!(s.browser.wants_frame());
    s.settle();
    s.assert_shows("SlowPage");
}

#[test]
fn truncated_body_renders_what_arrived_and_recovers() {
    let server = TestServer::start(|req| match req.path() {
        "/trunc" => Reply::script(
            vec![Step::Write(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5000\r\n\r\n\
                  <html><body><p>TruncStart</p>"
                    .to_vec(),
            )],
            true,
        ),
        "/ok" => page("StillWorks"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/trunc"));
    s.assert_shows("TruncStart");
    s.open(&server.url("/ok"));
    s.assert_shows("StillWorks");
}

#[test]
fn oversized_and_decompression_bomb_bodies_are_refused() {
    const NINE_MB: usize = 9 * 1024 * 1024;
    let bomb = {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(&vec![b'a'; 20 * 1024 * 1024]).unwrap();
        e.finish().unwrap()
    };
    let server = TestServer::start(move |req| match req.path() {
        "/huge" => Reply::with(200, &[("Content-Type", "text/html")], vec![b'x'; NINE_MB]),
        "/huge-chunked" => {
            let mut steps = vec![Step::Write(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                  Transfer-Encoding: chunked\r\n\r\n"
                    .to_vec(),
            )];
            let piece = "y".repeat(512 * 1024);
            for _ in 0..18 {
                steps.push(Step::Write(chunk(&piece)));
            }
            steps.push(Step::Write(b"0\r\n\r\n".to_vec()));
            Reply::script(steps, true)
        },
        "/bomb" => Reply::with(
            200,
            &[("Content-Type", "text/html"), ("Content-Encoding", "gzip")],
            bomb.clone(),
        ),
        "/ok" => page("SmallPage"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    for path in ["/huge", "/huge-chunked", "/bomb"] {
        let start = Instant::now();
        s.open(&server.url(path));
        assert_eq!(
            s.browser.loading_state(),
            LoadingState::Error,
            "{path} should be refused"
        );
        assert!(
            s.browser
                .error_message()
                .is_some_and(|m| m.contains("large") || m.contains("8 MB")),
            "{path}: {:?}",
            s.browser.error_message()
        );
        assert!(start.elapsed() < Duration::from_secs(8), "{path} too slow");
        s.open(&server.url("/ok"));
        s.assert_shows("SmallPage");
    }
}

#[test]
fn plain_text_is_shown_verbatim_not_parsed_as_html() {
    let server = TestServer::start(|req| match req.path() {
        "/notes.txt" => Reply::with(
            200,
            &[("Content-Type", "text/plain; charset=utf-8")],
            "line <b>one</b>\n<script>document.title='pwned'</script>",
        ),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/notes.txt"));
    s.assert_shows("<b>one</b>");
    s.assert_shows("<script>");
    assert_ne!(s.browser.title(), Some("pwned"));
}

#[test]
fn unexpected_content_types_do_not_break_the_browser() {
    let server = TestServer::start(|req| match req.path() {
        "/fake.png" => Reply::with(200, &[("Content-Type", "image/png")], "not a png"),
        "/bin" => Reply::with(
            200,
            &[("Content-Type", "application/octet-stream")],
            vec![0u8, 159, 146, 150, 255, 0, 1, 2],
        ),
        "/weird" => Reply::with(
            200,
            &[("Content-Type", ";;;===")],
            "<html><body><p>WeirdType</p></body></html>",
        ),
        "/ok" => page("StillFine"),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    for path in ["/fake.png", "/bin", "/weird"] {
        s.open(&server.url(path));
        assert_ne!(s.browser.loading_state(), LoadingState::Loading);
    }
    s.open(&server.url("/ok"));
    s.assert_shows("StillFine");
}

#[test]
fn reload_revalidates_with_etag_and_uses_cached_body_on_304() {
    let server = TestServer::start(|req| match req.path() {
        "/cached" => {
            if req.header("if-none-match") == Some("\"v1\"") {
                Reply::with(304, &[("ETag", "\"v1\"")], Vec::new())
            } else {
                Reply::with(
                    200,
                    &[("Content-Type", "text/html"), ("ETag", "\"v1\"")],
                    "<html><body><p>CachedBody</p></body></html>",
                )
            }
        },
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/cached"));
    s.assert_shows("CachedBody");
    s.reload();
    let reqs = server.requests_to("/cached");
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].header("if-none-match"), Some("\"v1\""));
    s.assert_shows("CachedBody");
}

#[test]
fn cookies_round_trip() {
    let server = TestServer::start(|req| match req.path() {
        "/login" => Reply::with(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "sid=abc123; Path=/"),
            ],
            "<html><body><a href=\"/me\">MeLink</a></body></html>",
        ),
        "/me" => page(&format!(
            "cookie={}",
            req.header("cookie").unwrap_or("none")
        )),
        _ => Reply::not_found(),
    });
    let mut s = Session::new();
    s.open(&server.url("/login"));
    s.click_text("MeLink");
    s.assert_shows("cookie=sid=abc123");
    // The first request carried no cookie.
    assert_eq!(server.requests_to("/login")[0].header("cookie"), None);
}
