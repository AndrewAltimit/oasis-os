//! Test harness: a scriptable local HTTP server and a browser session
//! driver that mimics the shell's frame loop.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oasis_types::input::{Button, InputEvent};
use oasis_vfs::MemoryVfs;

use crate::test_utils::{DrawCall, MockBackend};
use crate::{BrowserConfig, BrowserWidget};

/// Upper bound on any single wait. Generous so slow CI machines don't
/// flake, but finite so a regression fails instead of hanging.
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

// -------------------------------------------------------------------
// HTTP server
// -------------------------------------------------------------------

/// A request as received by the test server.
#[derive(Debug, Clone)]
pub(crate) struct Req {
    pub method: String,
    /// Request target: path plus optional `?query`.
    pub target: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self) -> Option<&str> {
        self.target.split_once('?').map(|(_, q)| q)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// One step of a scripted response.
pub(crate) enum Step {
    Write(Vec<u8>),
    Sleep(u64),
}

/// A scripted response.
pub(crate) struct Reply {
    pub steps: Vec<Step>,
    /// Close the connection after the response.
    pub close: bool,
}

impl Reply {
    /// Build a complete response with a `Content-Length` header.
    pub fn with(status: u16, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&body);
        Self {
            steps: vec![Step::Write(bytes)],
            close: false,
        }
    }

    /// `200 OK` with `Content-Type: text/html`.
    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self::with(200, &[("Content-Type", "text/html; charset=utf-8")], body)
    }

    /// A redirect to `location`.
    pub fn redirect(status: u16, location: &str) -> Self {
        Self::with(status, &[("Location", location)], Vec::new())
    }

    /// `404 Not Found` with an HTML body.
    pub fn not_found() -> Self {
        Self::with(
            404,
            &[("Content-Type", "text/html")],
            "<html><body><h1>Missing</h1><p>nothing-here-404</p></body></html>",
        )
    }

    /// A fully scripted response (raw bytes and pauses).
    pub fn script(steps: Vec<Step>, close: bool) -> Self {
        Self { steps, close }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

/// In-process HTTP/1.1 server on `127.0.0.1:<ephemeral>`.
///
/// Every connection gets its own thread and supports keep-alive, so
/// the browser's connection pool is exercised realistically.
pub(crate) struct TestServer {
    pub port: u16,
    log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn start(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Handler = Arc::new(handler);
        let (log2, stop2) = (Arc::clone(&log), Arc::clone(&stop));
        let accept = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (h, l, s) = (Arc::clone(&handler), Arc::clone(&log2), Arc::clone(&stop2));
                std::thread::spawn(move || serve_conn(conn, &h, &l, &s));
            }
        });
        Self {
            port,
            log,
            stop,
            accept: Some(accept),
        }
    }

    /// Absolute URL for `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Origin (`http://127.0.0.1:<port>`).
    #[cfg_attr(not(feature = "javascript"), allow(dead_code))]
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Snapshot of every request received so far.
    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }

    /// Requests whose path equals `path`.
    pub fn requests_to(&self, path: &str) -> Vec<Req> {
        self.requests()
            .into_iter()
            .filter(|r| r.path() == path)
            .collect()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblock `accept()`.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
    }
}

fn serve_conn(mut conn: TcpStream, handler: &Handler, log: &Mutex<Vec<Req>>, stop: &AtomicBool) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(20)));
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(req) = read_request(&mut conn, &mut buf) else {
            return;
        };
        log.lock().unwrap().push(req.clone());
        let reply = handler(&req);
        for step in reply.steps {
            match step {
                Step::Write(bytes) => {
                    if conn.write_all(&bytes).is_err() {
                        return;
                    }
                    let _ = conn.flush();
                },
                Step::Sleep(ms) => std::thread::sleep(Duration::from_millis(ms)),
            }
        }
        if reply.close {
            let _ = conn.shutdown(Shutdown::Both);
            return;
        }
    }
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
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
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
        headers,
        body,
    })
}

/// Decode an `application/x-www-form-urlencoded` string into pairs.
pub(crate) fn form_pairs(s: &str) -> Vec<(String, String)> {
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
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

// -------------------------------------------------------------------
// Browser session
// -------------------------------------------------------------------

/// A browser widget plus the host-side state the shell would hold
/// (VFS handle, last painted frame).
pub(crate) struct Session {
    pub browser: BrowserWidget,
    pub vfs: MemoryVfs,
    pub frame: MockBackend,
}

impl Session {
    pub fn new() -> Self {
        Self::sized(640, 480)
    }

    pub fn sized(w: u32, h: u32) -> Self {
        let mut config = BrowserConfig::default();
        config.features.home_url = "about:blank".to_string();
        let mut browser = BrowserWidget::new(config);
        browser.set_window(0, 0, w, h);
        Self {
            browser,
            vfs: MemoryVfs::new(),
            frame: MockBackend::new(),
        }
    }

    /// Navigate the way the shell's URL entry does, then settle.
    pub fn open(&mut self, url: &str) {
        self.browser.navigate_vfs(url, &self.vfs);
        self.settle();
    }

    /// Run the host frame loop (`tick` → `paint`) until the widget
    /// stops asking for frames, then capture a full frame.
    pub fn settle(&mut self) {
        let start = Instant::now();
        loop {
            self.browser.tick(&self.vfs);
            if !self.browser.wants_frame() {
                break;
            }
            let mut backend = MockBackend::new();
            self.browser.paint(&mut backend).unwrap();
            assert!(
                start.elapsed() < DEADLINE,
                "browser did not settle within {DEADLINE:?} (url={:?}, state={:?})",
                self.browser.current_url(),
                self.browser.loading_state(),
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.snapshot();
    }

    /// Keep ticking and painting for `ms` of wall-clock time (lets JS
    /// timers and animations run), then capture a full frame.
    #[cfg_attr(not(feature = "javascript"), allow(dead_code))]
    pub fn run_for(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            self.browser.tick(&self.vfs);
            let mut backend = MockBackend::new();
            self.browser.paint(&mut backend).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
    }

    /// Run the frame loop until `needle` is visible, failing after
    /// [`DEADLINE`]. Prefer this over a fixed [`Session::run_for`] when
    /// asserting on async work (timers, fetches): a loaded CI runner can
    /// stretch wall-clock waits arbitrarily.
    #[track_caller]
    pub fn wait_for(&mut self, needle: &str) {
        let start = Instant::now();
        loop {
            self.settle();
            if self.shows(needle) || start.elapsed() >= DEADLINE {
                break;
            }
            self.run_for(10);
        }
        self.assert_shows(needle);
    }

    /// Paint one complete frame (forcing a full display-list replay)
    /// into `self.frame`.
    pub fn snapshot(&mut self) {
        self.browser.full_repaint_needed = true;
        self.frame = MockBackend::new();
        self.browser.paint(&mut self.frame).unwrap();
    }

    fn content_top(&self) -> i32 {
        self.browser.window_y() + self.browser.config.url_bar_height as i32
    }

    fn content_bottom(&self) -> i32 {
        self.browser.window_y() + self.browser.window_h() as i32
            - self.browser.config.status_bar_height as i32
    }

    /// Text runs painted inside the visible content viewport, in
    /// reading order, as `(text, x, y)`.
    pub fn content_runs(&self) -> Vec<(String, i32, i32)> {
        let (top, bottom) = (self.content_top(), self.content_bottom());
        let mut runs: Vec<(String, i32, i32)> = self
            .frame
            .calls
            .iter()
            .filter_map(|c| match c {
                DrawCall::DrawText { text, x, y, .. } if *y >= top && *y < bottom => {
                    Some((text.clone(), *x, *y))
                },
                _ => None,
            })
            .collect();
        runs.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.cmp(&b.1)));
        runs
    }

    /// All text visible in the content viewport, space-joined.
    pub fn content_text(&self) -> String {
        self.content_runs()
            .into_iter()
            .map(|(t, _, _)| t)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Does the visible content show `needle`?
    pub fn shows(&self, needle: &str) -> bool {
        self.content_text().contains(needle)
    }

    /// Assert the visible content shows `needle`.
    #[track_caller]
    pub fn assert_shows(&self, needle: &str) {
        assert!(
            self.shows(needle),
            "expected page to show {needle:?}; url={:?}; visible text: {:?}",
            self.browser.current_url(),
            self.content_text(),
        );
    }

    /// Screen position of the first visible text run containing
    /// `needle` (a point just inside its top-left corner).
    pub fn text_pos(&self, needle: &str) -> Option<(i32, i32)> {
        self.content_runs()
            .into_iter()
            .find(|(t, _, _)| t.contains(needle))
            .map(|(_, x, y)| (x + 2, y + 3))
    }

    /// Click on the visible text `needle`, then settle.
    #[track_caller]
    pub fn click_text(&mut self, needle: &str) {
        let (x, y) = self.text_pos(needle).unwrap_or_else(|| {
            panic!(
                "no visible text {needle:?} to click; visible: {:?}",
                self.content_text()
            )
        });
        self.click(x, y);
    }

    /// Screen-space center of the element with `id` (from the current
    /// layout, accounting for chrome and scroll) — where a user would
    /// click on it.
    #[track_caller]
    pub fn element_center(&self, id: &str) -> (i32, i32) {
        let doc = self.browser.document.as_ref().expect("no document");
        let nid = doc
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"));
        let layout = self.browser.layout_root.as_ref().expect("no layout");
        let r = BrowserWidget::find_node_rect(layout, nid)
            .unwrap_or_else(|| panic!("#{id} has no layout box"));
        let x = self.browser.window_x() as f32 + r.x + r.width / 2.0
            - self.browser.scroll().scroll_x as f32;
        let y = self.content_top() as f32 + r.y + r.height / 2.0
            - self.browser.scroll().scroll_y as f32;
        (x as i32, y as i32)
    }

    /// The replaced content (form control state as laid out and
    /// painted) of the element with `id`.
    #[track_caller]
    pub fn replaced(&self, id: &str) -> crate::layout::box_model::ReplacedContent {
        use crate::layout::box_model::{BoxType, LayoutBox};
        fn find(lb: &LayoutBox, nid: usize) -> Option<&LayoutBox> {
            if lb.node == Some(nid) && matches!(lb.box_type, BoxType::Replaced(_)) {
                return Some(lb);
            }
            lb.children.iter().find_map(|c| find(c, nid))
        }
        let doc = self.browser.document.as_ref().expect("no document");
        let nid = doc.get_element_by_id(id).expect("no such element");
        let layout = self.browser.layout_root.as_ref().expect("no layout");
        match find(layout, nid).map(|b| &b.box_type) {
            Some(BoxType::Replaced(r)) => r.clone(),
            _ => panic!("#{id} is not a replaced element"),
        }
    }

    /// Click the center of the element with `id`, then settle.
    #[track_caller]
    pub fn click_element(&mut self, id: &str) {
        let (x, y) = self.element_center(id);
        self.click(x, y);
    }

    /// Pointer click at screen coordinates, then settle.
    pub fn click(&mut self, x: i32, y: i32) {
        self.browser
            .handle_input(&InputEvent::PointerClick { x, y }, &self.vfs);
        self.settle();
    }

    /// Deliver an input event, then settle.
    pub fn input(&mut self, ev: InputEvent) -> bool {
        let consumed = self.browser.handle_input(&ev, &self.vfs);
        self.settle();
        consumed
    }

    /// Press a gamepad/keyboard button.
    pub fn press(&mut self, b: Button) {
        self.input(InputEvent::ButtonPress(b));
    }

    /// Type a string as individual `TextInput` events.
    pub fn type_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.browser
                .handle_input(&InputEvent::TextInput(ch), &self.vfs);
        }
        self.settle();
    }

    /// Current URL as shown in the chrome.
    pub fn url(&self) -> String {
        self.browser.current_url().unwrap_or("").to_string()
    }

    /// Reload the current page the way the shell does (re-enter the
    /// current URL).
    pub fn reload(&mut self) {
        let url = self.url();
        self.open(&url);
    }
}
