#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! The browser inside the real desktop shell.
//!
//! `oasis-browser`'s own e2e suite drives a bare `BrowserWidget`. These
//! scenarios boot the whole shell (`oasis_app::harness::Harness`) and use
//! the browser only the way a user does: the dashboard icon, clicks on the
//! chrome and on painted page text, the wheel, keys and typing — routed
//! through the shell's input pipeline and window manager. Pages come from
//! an in-process HTTP server on `127.0.0.1`, so network loads really go
//! through the browser's background I/O thread; every wait steps the
//! harness with a short real sleep, bounded by a wall-clock deadline.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oasis_app::Mode;
use oasis_app::harness::Harness;
use oasis_app::headless::DrawnText;
use oasis_core::browser::{BrowserWidget, LoadingState};
use oasis_core::input::{Key, Modifiers};
use oasis_core::vfs::Vfs;

const DEADLINE: Duration = Duration::from_secs(15);
/// How long `/slow` stalls before replying. Long enough that a close
/// which joined the browser's I/O thread would be unmistakable even when
/// a loaded CI pool stretches a non-blocking close to a second.
const SLOW_REPLY: Duration = Duration::from_secs(6);

// ---------------------------------------------------------------------------
// Local HTTP server
// ---------------------------------------------------------------------------

/// A request as received by [`Server`].
#[derive(Debug, Clone)]
struct Req {
    method: String,
    /// Path plus optional `?query`.
    target: String,
    body: Vec<u8>,
}

impl Req {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

/// What the handler answers.
struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    /// Stall this long before answering.
    delay: Duration,
}

impl Reply {
    fn html(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.into().into_bytes(),
            delay: Duration::ZERO,
        }
    }

    fn not_found() -> Self {
        Self {
            status: 404,
            ..Self::html("<html><body><p>nothing here</p></body></html>")
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

/// In-process HTTP/1.1 server on `127.0.0.1:<ephemeral>` (keep-alive,
/// one thread per connection). Records every request and every
/// connection the client closed.
struct Server {
    port: u16,
    log: Arc<Mutex<Vec<Req>>>,
    closed: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    fn start(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Handler = Arc::new(handler);
        let (l, c, s) = (Arc::clone(&log), Arc::clone(&closed), Arc::clone(&stop));
        let accept = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (h, l, c) = (Arc::clone(&handler), Arc::clone(&l), Arc::clone(&c));
                std::thread::spawn(move || serve(conn, &h, &l, &c));
            }
        });
        Self {
            port,
            log,
            closed,
            stop,
            accept: Some(accept),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn requests_to(&self, path: &str) -> Vec<Req> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path() == path)
            .cloned()
            .collect()
    }

    fn closed_connections(&self) -> usize {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
    }
}

fn serve(mut conn: TcpStream, handler: &Handler, log: &Mutex<Vec<Req>>, closed: &AtomicUsize) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(30)));
    let mut buf = Vec::new();
    while let Some(req) = read_request(&mut conn, &mut buf) {
        log.lock().unwrap().push(req.clone());
        let reply = handler(&req);
        std::thread::sleep(reply.delay);
        let head = format!(
            "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
            reply.status,
            reply.content_type,
            reply.body.len()
        );
        if conn.write_all(head.as_bytes()).is_err() || conn.write_all(&reply.body).is_err() {
            break;
        }
    }
    closed.fetch_add(1, Ordering::SeqCst);
    let _ = conn.shutdown(Shutdown::Both);
}

fn read_request(conn: &mut TcpStream, buf: &mut Vec<u8>) -> Option<Req> {
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        let n = conn.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = head_end + 4;
    while buf.len() < body_start + len {
        let n = conn.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = buf[body_start..body_start + len].to_vec();
    buf.drain(..body_start + len);
    Some(Req {
        method,
        target,
        body,
    })
}

/// Decode `application/x-www-form-urlencoded`.
fn form_pairs(s: &str) -> Vec<(String, String)> {
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < b.len() => {
                    let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    i += 2;
                },
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8(out).unwrap()
    }
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (dec(k), dec(v))
        })
        .collect()
}

/// A small site: `/` links to `/b`, `/long` is a tall page, `/form`
/// posts to `/submit`, `/blank` draws nothing, `/slow` stalls for
/// [`SLOW_REPLY`].
fn site() -> Server {
    Server::start(|req| match req.path() {
        "/" => Reply::html(
            "<html><head><title>Alpha Page</title></head><body>\
             <p>alpha-content</p><p><a href=\"/b\">go-to-beta</a></p></body></html>",
        ),
        "/b" => Reply::html(
            "<html><head><title>Beta Page</title></head><body>\
             <p>beta-content</p><p><a href=\"/\">back-to-alpha</a></p></body></html>",
        ),
        "/untitled" => Reply::html("<html><body><p>no-title-here</p></body></html>"),
        "/long" => {
            let lines: String = (0..120).map(|i| format!("<p>row-{i}</p>")).collect();
            Reply::html(format!(
                "<html><head><title>Long</title></head><body>{lines}</body></html>"
            ))
        },
        "/wrap" => {
            let words: String = (0..80).map(|i| format!("w{i} ")).collect();
            Reply::html(format!(
                "<html><head><title>Wrap</title></head><body><p>{words}</p></body></html>"
            ))
        },
        "/form" => Reply::html(
            "<html><head><title>Form</title></head><body>\
             <form action=\"/submit\" method=\"post\">\
             <label for=\"q\">QueryLabel</label> <input type=\"text\" name=\"q\" id=\"q\">\
             <input type=\"submit\" value=\"Send\"></form></body></html>",
        ),
        "/submit" => {
            let pairs = form_pairs(&String::from_utf8_lossy(&req.body));
            let q = pairs
                .iter()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            Reply::html(format!(
                "<html><head><title>Result</title></head><body>\
                 <p>method-{}</p><p>got-{}</p></body></html>",
                req.method,
                q.replace(' ', "_")
            ))
        },
        "/js" => Reply::html(
            "<html><head><title>Script Page</title></head><body>\
             <p><button id=\"b\">PressMe</button></p><p id=\"out\">waiting</p>\
             <script>\
             document.getElementById('b').addEventListener('click', function () {\
               document.getElementById('out').textContent = 'clicked-ok';\
               document.title = 'Clicked Title';\
             });\
             </script></body></html>",
        ),
        "/blank" => Reply::html("<html><head><title>Blank</title></head><body></body></html>"),
        "/slow" => Reply::html("<html><body><p>slow-page</p></body></html>").delayed(SLOW_REPLY),
        "/hover" => Reply::html(
            "<html><body><p><a href=\"/b\" onmouseover=\"document.getElementById('h')\
             .textContent = 'hovered-yes'\">hover-link</a></p><p id=\"h\">hovered-no</p>\
             </body></html>",
        ),
        "/nested" => {
            let rows: String = (0..30).map(|i| format!("<p>inner-{i}</p>")).collect();
            Reply::html(format!(
                "<html><body><div style=\"height:60px;overflow:auto\">{rows}</div>\
                 <p>outer-tail</p></body></html>"
            ))
        },
        _ => Reply::not_found(),
    })
}

// ---------------------------------------------------------------------------
// Shell helpers
// ---------------------------------------------------------------------------

fn dump(h: &Harness, name: &str) -> String {
    let dir = std::env::temp_dir().join("oasis-e2e");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("browser_{name}.png"));
    match h.save_png(&path) {
        Ok(()) => format!("(frame saved to {})", path.display()),
        Err(e) => format!("(frame dump failed: {e})"),
    }
}

fn boot() -> Harness {
    let mut h = Harness::new("classic");
    h.settle();
    h
}

fn browser(h: &Harness) -> &BrowserWidget {
    h.state()
        .content
        .browser
        .as_ref()
        .expect("browser window open")
}

/// Step frames (with a short real sleep so the I/O thread runs) until
/// `cond` holds; panics after [`DEADLINE`].
#[track_caller]
fn wait_for(h: &mut Harness, what: &str, cond: impl Fn(&Harness) -> bool) {
    let start = Instant::now();
    while !cond(h) {
        assert!(
            start.elapsed() < DEADLINE,
            "timed out waiting for {what}; url={:?} state={:?} {}",
            h.state()
                .content
                .browser
                .as_ref()
                .and_then(|b| b.current_url().map(str::to_string)),
            h.state()
                .content
                .browser
                .as_ref()
                .map(|b| b.loading_state()),
            dump(h, "timeout")
        );
        h.step(&[]);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Wait until the page (and its sub-resources) finished loading, then
/// let the UI settle.
#[track_caller]
fn wait_loaded(h: &mut Harness) {
    wait_for(h, "page load", |h| {
        let bw = browser(h);
        bw.loading_state() != LoadingState::Loading && bw.io_thread_in_flight().unwrap_or(0) == 0
    });
    h.settle();
}

/// The browser's window rect as set by the WM at the last paint.
fn viewport(h: &Harness) -> (i32, i32, i32, i32) {
    let bw = browser(h);
    (
        bw.window_x(),
        bw.window_y(),
        bw.window_w() as i32,
        bw.window_h() as i32,
    )
}

/// Text runs painted in the page area (below the chrome, above the
/// status bar) of a freshly presented frame, in reading order.
fn page_runs(h: &mut Harness) -> Vec<DrawnText> {
    h.render_now();
    let (x, y, w, hh) = viewport(h);
    let cfg = &browser(h).config;
    let top = y + cfg.url_bar_height as i32;
    let bottom = y + hh - cfg.status_bar_height as i32;
    let mut runs: Vec<DrawnText> = h
        .frame_text_calls()
        .iter()
        .filter(|t| t.x >= x && t.x < x + w && t.y >= top && t.y < bottom)
        .cloned()
        .collect();
    runs.sort_by_key(|t| (t.y, t.x));
    runs
}

fn page_text(h: &mut Harness) -> String {
    page_runs(h)
        .into_iter()
        .map(|t| t.text)
        .collect::<Vec<_>>()
        .join(" ")
}

#[track_caller]
fn assert_shows(h: &mut Harness, needle: &str) {
    let text = page_text(h);
    assert!(
        text.contains(needle),
        "page should show {needle:?}; url={:?}; visible: {text:?} {}",
        browser(h).current_url(),
        dump(h, "assert_shows")
    );
}

/// Click on the painted page text containing `needle`.
#[track_caller]
fn click_page_text(h: &mut Harness, needle: &str) {
    let run = page_runs(h)
        .into_iter()
        .find(|t| t.text.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no painted {needle:?} to click; visible: {:?} {}",
                page_text(h),
                dump(h, "click_page_text")
            )
        });
    h.click(run.x + 2, run.y + 3);
}

/// Chrome button centers: back, forward, URL bar, bookmarks, home.
enum Chrome {
    Back,
    Forward,
    UrlBar,
}

fn click_chrome(h: &mut Harness, what: Chrome) {
    let (x, y, _, _) = viewport(h);
    let cfg = &browser(h).config;
    let (bw, bh) = (cfg.button_width as i32, cfg.url_bar_height as i32);
    let cx = match what {
        Chrome::Back => x + bw / 2,
        Chrome::Forward => x + bw + bw / 2,
        Chrome::UrlBar => x + bw * 2 + 24,
    };
    h.click(cx, y + bh / 2);
}

/// Open the browser from its dashboard icon.
fn open_browser(h: &mut Harness) {
    assert!(
        h.click_app_icon("Browser"),
        "Browser icon on the dashboard: {:?}",
        h.dashboard_apps()
    );
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    wait_loaded(h);
}

/// Click the URL bar, type `url` over the selected URL, press Enter.
fn type_url(h: &mut Harness, url: &str) {
    click_chrome(h, Chrome::UrlBar);
    assert!(browser(h).url_bar_focused(), "URL bar focused by the click");
    h.type_text(url);
    h.key(Key::Enter);
}

fn go(h: &mut Harness, url: &str) {
    type_url(h, url);
    wait_loaded(h);
    assert_eq!(browser(h).current_url(), Some(url));
}

fn window_title(h: &Harness) -> String {
    h.find_window("browser").expect("browser window").title
}

/// Run `cmd` in the shell terminal: the fullscreen terminal from the
/// dashboard, or a terminal window while other windows are open.
fn run_command(h: &mut Harness, cmd: &str) {
    if h.mode() == Mode::Desktop {
        assert!(h.open_app("Terminal"));
        h.settle();
        h.type_text(cmd);
        h.key(Key::Enter);
    } else {
        h.terminal(cmd);
    }
}

fn output_contains(h: &Harness, needle: &str) -> bool {
    h.state()
        .terminal
        .output_lines
        .iter()
        .any(|l| l.contains(needle))
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn opens_from_dashboard_and_loads_a_typed_url() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    // The home page from the VFS.
    assert_shows(&mut h, "Welcome");
    assert_eq!(window_title(&h), "OASIS Home - Browser");

    let url = srv.url("/");
    go(&mut h, &url);
    assert_shows(&mut h, "alpha-content");
    assert_eq!(srv.requests_to("/").len(), 1);
    // The URL bar shows the page's URL again once it lost focus.
    assert!(!browser(&h).url_bar_focused());
    assert!(
        h.text_drawn_contains(&format!("127.0.0.1:{}", srv.port)),
        "URL bar text: {:?}",
        h.frame_text()
    );
}

#[test]
fn links_and_history_via_chrome_buttons_and_keys() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    let (a, b) = (srv.url("/"), srv.url("/b"));
    go(&mut h, &a);

    click_page_text(&mut h, "go-to-beta");
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(b.as_str()));
    assert_shows(&mut h, "beta-content");

    // Chrome buttons.
    click_chrome(&mut h, Chrome::Back);
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(a.as_str()));
    assert_shows(&mut h, "alpha-content");
    click_chrome(&mut h, Chrome::Forward);
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(b.as_str()));
    assert_shows(&mut h, "beta-content");

    // Keyboard: Alt+Left / Alt+Right.
    h.key_with(Key::Left, Modifiers::ALT);
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(a.as_str()));
    h.key_with(Key::Right, Modifiers::ALT);
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(b.as_str()));

    // Reload (F5, Ctrl+R) refetches without touching history.
    let before = srv.requests_to("/b").len();
    h.key(Key::F(5));
    wait_loaded(&mut h);
    h.key_with(Key::Char('r'), Modifiers::CTRL);
    wait_loaded(&mut h);
    assert_eq!(srv.requests_to("/b").len(), before + 2, "reload refetches");
    assert_shows(&mut h, "beta-content");
    assert!(!browser(&h).navigation().can_go_forward());
    click_chrome(&mut h, Chrome::Back);
    wait_loaded(&mut h);
    assert_eq!(
        browser(&h).current_url(),
        Some(a.as_str()),
        "reload must not add history entries"
    );
    // Home is the entry before `a`; nothing else was pushed.
    click_chrome(&mut h, Chrome::Back);
    wait_loaded(&mut h);
    assert!(!browser(&h).navigation().can_go_back());
}

#[test]
fn form_typing_and_submit_reach_the_server() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/form"));
    let desktop = h.state().ui.desktops.active_desktop();

    // Clicking the label focuses the field.
    click_page_text(&mut h, "QueryLabel");
    assert!(browser(&h).accepts_text(), "text field focused");
    // 'q' and 'e' are also the L/R trigger keys (virtual desktop
    // switch): typing must not trigger them.
    h.type_text("queue tex");
    h.key(Key::Backspace);
    h.type_text("st");
    assert_eq!(
        h.state().ui.desktops.active_desktop(),
        desktop,
        "typing q/e switched the virtual desktop"
    );
    h.key(Key::Enter);
    wait_loaded(&mut h);

    let posts = srv.requests_to("/submit");
    assert_eq!(posts.len(), 1, "one submission");
    assert_eq!(posts[0].method, "POST");
    assert_eq!(
        form_pairs(&String::from_utf8_lossy(&posts[0].body)),
        vec![("q".to_string(), "queue test".to_string())]
    );
    assert_shows(&mut h, "got-queue_test");
}

#[test]
fn escape_cancels_typing_before_it_closes_the_window() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    let a = srv.url("/");
    go(&mut h, &a);

    // Escape in the URL bar discards the edit; the window stays.
    type_url_without_enter(&mut h, "http://discarded.invalid/");
    h.key(Key::Escape);
    assert!(
        h.find_window("browser").is_some(),
        "Escape closed the browser"
    );
    assert!(!browser(&h).url_bar_focused());
    assert_eq!(browser(&h).current_url(), Some(a.as_str()));

    // Escape in a page text field leaves the field.
    go(&mut h, &srv.url("/form"));
    click_page_text(&mut h, "QueryLabel");
    assert!(browser(&h).accepts_text());
    h.key(Key::Escape);
    assert!(
        h.find_window("browser").is_some(),
        "Escape closed the browser"
    );
    assert!(!browser(&h).accepts_text(), "field still focused");

    // With nothing being typed, Escape keeps closing the browser.
    h.key(Key::Escape);
    h.settle();
    assert!(h.find_window("browser").is_none());
    assert!(h.state().content.browser.is_none());
}

fn type_url_without_enter(h: &mut Harness, url: &str) {
    click_chrome(h, Chrome::UrlBar);
    assert!(browser(h).url_bar_focused());
    h.type_text(url);
}

#[cfg(feature = "javascript")]
#[test]
fn js_click_handler_updates_page_and_title() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/js"));
    assert_shows(&mut h, "waiting");
    assert_eq!(window_title(&h), "Script Page - Browser");

    click_page_text(&mut h, "PressMe");
    h.settle();
    assert_shows(&mut h, "clicked-ok");
    assert_eq!(window_title(&h), "Clicked Title - Browser");
    h.render_now();
    assert!(
        h.text_drawn_contains("Clicked Title"),
        "titlebar repainted: {:?}",
        h.frame_text()
    );
}

#[test]
fn page_title_follows_navigation() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/"));
    assert_eq!(window_title(&h), "Alpha Page - Browser");
    h.render_now();
    assert!(h.text_drawn_contains("Alpha Page"), "{:?}", h.frame_text());
    go(&mut h, &srv.url("/untitled"));
    assert_eq!(window_title(&h), "Browser");
    go(&mut h, &srv.url("/b"));
    assert_eq!(window_title(&h), "Beta Page - Browser");
}

#[test]
fn wheel_and_keys_scroll_the_page() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/long"));
    assert_shows(&mut h, "row-0");
    let (x, y, w, hh) = viewport(&h);
    h.move_to(x + w / 2, y + hh / 2);

    h.scroll(2);
    h.settle();
    let scrolled = browser(&h).scroll().scroll_y;
    assert!(scrolled > 0, "wheel scrolled the page");
    assert!(
        !shows_run(&mut h, "row-0"),
        "row-0 scrolled out: {:?}",
        page_text(&mut h)
    );
    h.scroll(-2);
    h.settle();
    assert_eq!(browser(&h).scroll().scroll_y, 0);
    assert_shows(&mut h, "row-0");

    h.key(Key::Down);
    h.settle();
    assert!(browser(&h).scroll().scroll_y > 0, "Down arrow scrolls");
    h.key(Key::End);
    h.settle();
    assert!(
        browser(&h).scroll().at_bottom(),
        "End scrolls to the bottom"
    );
    assert_shows(&mut h, "row-119");
    h.key(Key::Home);
    h.settle();
    assert_eq!(browser(&h).scroll().scroll_y, 0, "Home scrolls to the top");
    h.key(Key::PageDown);
    h.settle();
    let paged = browser(&h).scroll().scroll_y;
    assert!(paged > scrolled / 2, "PageDown scrolls a page ({paged})");
    h.key(Key::PageUp);
    h.settle();
    assert_eq!(browser(&h).scroll().scroll_y, 0);
}

/// Whether a painted page run is exactly `word`.
fn shows_run(h: &mut Harness, word: &str) -> bool {
    page_runs(h).iter().any(|t| t.text.trim() == word)
}

/// The wheel over an `overflow: auto` box scrolls that box, not the
/// page. Needs the pointer position, which the shell never forwarded.
#[test]
fn wheel_over_a_nested_scroller_scrolls_it() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/nested"));
    let run = page_runs(&mut h)
        .into_iter()
        .find(|t| t.text.trim() == "inner-0")
        .expect("inner-0 painted");
    h.move_to(run.x + 2, run.y + 3);
    h.scroll(1);
    h.settle();
    assert_eq!(
        browser(&h).scroll().scroll_y,
        0,
        "the page itself stayed put"
    );
    assert!(
        !shows_run(&mut h, "inner-0"),
        "the inner box scrolled: {:?} {}",
        page_text(&mut h),
        dump(&h, "nested_scroll")
    );
    assert!(shows_run(&mut h, "outer-tail"));
}

#[cfg(feature = "javascript")]
#[test]
fn pointer_hover_reaches_page_scripts() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/hover"));
    let run = page_runs(&mut h)
        .into_iter()
        .find(|t| t.text.contains("hover-link"))
        .expect("link painted");
    h.move_to(run.x + 2, run.y + 3);
    h.settle();
    assert_shows(&mut h, "hovered-yes");
}

#[test]
fn resizing_the_window_relays_out_the_page() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/wrap"));
    let lines = |h: &mut Harness| {
        let mut ys: Vec<i32> = page_runs(h)
            .iter()
            .filter(|t| t.text.starts_with('w'))
            .map(|t| t.y)
            .collect();
        ys.dedup();
        ys.len()
    };
    let (w0, narrow_lines) = (viewport(&h).2, lines(&mut h));
    assert!(narrow_lines >= 3, "paragraph wraps in the small window");

    // Maximize (Super+Up): wider content, fewer lines.
    h.key_with(Key::Up, Modifiers::SUPER);
    h.settle();
    let w1 = viewport(&h).2;
    assert!(w1 > w0, "window grew ({w0} -> {w1})");
    let wide_lines = lines(&mut h);
    assert!(
        wide_lines < narrow_lines,
        "relayout at the new width: {narrow_lines} -> {wide_lines} lines {}",
        dump(&h, "resize")
    );
    assert_shows(&mut h, "w0");
}

#[test]
fn skin_switch_restyles_the_open_browser() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    go(&mut h, &srv.url("/"));
    let before = browser(&h).config.chrome_bg;

    run_command(&mut h, "skin retro-cga");
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "retro-cga");
    let themed = h.state().browser_config.chrome_bg;
    assert_ne!(before, themed, "test needs skins with different chrome");
    assert_eq!(
        browser(&h).config.chrome_bg,
        themed,
        "open browser adopted the skin"
    );

    // Still the same page, painted with the new chrome.
    assert!(h.find_window("browser").is_some());
    assert_eq!(browser(&h).current_url(), Some(srv.url("/").as_str()));
    assert_shows(&mut h, "alpha-content");
    let (x, y, _, _) = viewport(&h);
    let px = h.pixel((x + 70) as u32, (y + 1) as u32);
    assert_eq!(
        (px[0], px[1], px[2]),
        (themed.r, themed.g, themed.b),
        "chrome background repainted {}",
        dump(&h, "skin_switch")
    );
}

#[test]
fn unreachable_host_shows_an_error_page_without_hanging() {
    // A port nothing listens on: connection refused.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut h = boot();
    open_browser(&mut h);
    let url = format!("http://127.0.0.1:{port}/");
    type_url(&mut h, &url);
    let start = Instant::now();
    wait_for(&mut h, "error page", |h| {
        browser(h).loading_state() == LoadingState::Error
    });
    assert!(start.elapsed() < Duration::from_secs(10));
    h.settle();
    assert_eq!(browser(&h).current_url(), Some(url.as_str()));
    assert!(browser(&h).error_message().is_some());
    assert_shows(&mut h, "Connection");
    // The shell keeps running normally.
    click_chrome(&mut h, Chrome::Back);
    wait_loaded(&mut h);
    assert_shows(&mut h, "Welcome");
}

#[test]
fn closing_the_browser_during_a_slow_load_is_prompt_and_clean() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    type_url(&mut h, &srv.url("/slow"));
    wait_for(&mut h, "slow request sent", |_| {
        !srv.requests_to("/slow").is_empty()
    });
    assert_eq!(browser(&h).loading_state(), LoadingState::Loading);

    let start = Instant::now();
    assert!(h.close_window("browser"));
    let took = start.elapsed();
    // A close that waited on the I/O thread would take most of
    // SLOW_REPLY; half of it leaves room for a loaded runner (a
    // non-blocking close has read ~1s on a busy nextest pool).
    assert!(took < SLOW_REPLY / 2, "closing blocked the UI for {took:?}");
    h.settle();
    assert!(h.state().content.browser.is_none());
    assert!(h.find_window("browser").is_none());
    assert_eq!(h.mode(), Mode::Dashboard);

    // The abandoned request finishes on the server; the browser's I/O
    // thread then exits and its pooled connection closes.
    let start = Instant::now();
    while srv.closed_connections() == 0 {
        assert!(
            start.elapsed() < DEADLINE,
            "browser I/O thread never released its connection"
        );
        h.step(&[]);
        std::thread::sleep(Duration::from_millis(5));
    }
    // A fresh browser works normally afterwards.
    open_browser(&mut h);
    go(&mut h, &srv.url("/b"));
    assert_shows(&mut h, "beta-content");
}

/// A loaded static page must let the shell elide frames. Frames the
/// SDI scene asks for (status-bar updates and the like, which happen
/// without any window open too) don't count; beyond those only the
/// once-a-second heartbeat may present. Before the fix, a page whose
/// display list records nothing (`/blank`) presented every frame.
#[test]
fn static_pages_stop_presenting_frames() {
    let srv = site();
    let mut h = boot();
    open_browser(&mut h);
    for path in ["/", "/blank"] {
        go(&mut h, &srv.url(path));
        // Past the input redraw grace period.
        h.run_frames(30);
        let mut unexplained = 0;
        for _ in 0..180 {
            let out = h.step(&[]);
            if out.redraw && !out.scene_changed {
                unexplained += 1;
            }
        }
        assert!(
            unexplained <= 4,
            "{path}: {unexplained} of 180 idle frames presented (browser wants_frame={})",
            browser(&h).wants_frame()
        );
    }
}

// ---------------------------------------------------------------------------
// Terminal commands wired to the shell
// ---------------------------------------------------------------------------

#[test]
fn browse_command_drives_the_browser() {
    let srv = site();
    let mut h = boot();
    let (a, b) = (srv.url("/"), srv.url("/b"));

    run_command(&mut h, &format!("browse {a}"));
    assert!(output_contains(&h, "[browser] Opening"));
    assert!(
        h.find_window("browser").is_some(),
        "browse opened the browser"
    );
    wait_loaded(&mut h);
    assert_eq!(browser(&h).current_url(), Some(a.as_str()));
    assert_shows(&mut h, "alpha-content");

    for (cmd, want) in [
        (format!("browse {b}"), b.as_str()),
        ("browse back".to_string(), a.as_str()),
        ("browse forward".to_string(), b.as_str()),
    ] {
        run_command(&mut h, &cmd);
        wait_loaded(&mut h);
        assert_eq!(browser(&h).current_url(), Some(want), "after `{cmd}`");
        // The command focuses the browser over the terminal window.
        assert_eq!(h.state().wm.active_window(), Some("browser"));
    }
    assert_shows(&mut h, "beta-content");

    let before = srv.requests_to("/b").len();
    run_command(&mut h, "browse reload");
    wait_loaded(&mut h);
    assert_eq!(srv.requests_to("/b").len(), before + 1, "reload refetched");
    assert_eq!(browser(&h).current_url(), Some(b.as_str()));

    run_command(&mut h, "browse history");
    wait_loaded(&mut h);
    // Titles are laid out word by word.
    assert_shows(&mut h, "Alpha");
    assert_shows(&mut h, "Beta");

    run_command(&mut h, "browse home");
    wait_loaded(&mut h);
    assert_shows(&mut h, "Welcome");
}

#[test]
fn notify_command_shows_a_toast() {
    let mut h = boot();
    let (shown, _) = h.state().toasts.shown_counts();
    run_command(&mut h, "notify --level warning disk-nearly-full");
    assert!(output_contains(&h, "Notification queued"));
    h.run_frames(2);
    assert_eq!(h.state().toasts.shown_counts().0, shown + 1);
    assert!(
        h.sdi_text_contains("disk-nearly-full"),
        "toast visible: {:?}",
        h.sdi_texts()
    );
    // Consumed: it does not repeat.
    h.run_frames(5);
    assert_eq!(h.state().toasts.shown_counts().0, shown + 1);
}

#[test]
fn screenshot_command_saves_the_presented_frame() {
    let mut h = boot();
    run_command(&mut h, "screenshot /tmp/shot.bmp");
    h.run_frames(2);
    assert!(
        output_contains(&h, "Screenshot saved: /tmp/shot.bmp"),
        "{:?}",
        h.state().terminal.output_lines
    );
    let (w, hh) = h.size();
    let bmp = h.vfs().read("/tmp/shot.bmp").unwrap();
    assert_eq!(&bmp[..2], b"BM");
    let bw = i32::from_le_bytes(bmp[18..22].try_into().unwrap());
    let bh = i32::from_le_bytes(bmp[22..26].try_into().unwrap());
    assert_eq!((bw, bh), (w as i32, hh as i32));
    // The saved image has real content (the terminal), not a blank.
    let mut colors: Vec<&[u8]> = bmp[54..].chunks(3).collect();
    colors.sort();
    colors.dedup();
    assert!(colors.len() > 3, "screenshot is blank");

    run_command(&mut h, "screenshot /tmp/shot.png");
    h.run_frames(2);
    let png = h.vfs().read("/tmp/shot.png").unwrap();
    assert_eq!(&png[1..4], b"PNG");

    // A path that can't be written reports the error.
    run_command(&mut h, "screenshot /no/such/dir/x.bmp");
    h.run_frames(2);
    assert!(output_contains(&h, "screenshot: /no/such/dir/x.bmp"));
}

#[test]
fn theme_command_reports_the_active_skin() {
    let mut h = boot();
    run_command(&mut h, "theme");
    assert!(
        output_contains(&h, "skin: classic"),
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(output_contains(&h, "background: #"));
    run_command(&mut h, "skin xp");
    h.settle();
    run_command(&mut h, "theme");
    assert!(output_contains(&h, "skin: xp"));
}
