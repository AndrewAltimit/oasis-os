//! End-to-end tests of the desktop TV streaming pipeline.
//!
//! An in-process HTTP/1.1 server on `127.0.0.1:0` serves real MP4 fixtures
//! (with Range support and scriptable misbehaviour: throttling, dropped or
//! stalled connections, ignored Range headers, redirects, 404s).  The real
//! download code (`download::stream_download`) fills a real
//! `StreamingInner`, and the real player (`VideoPlayer` via
//! `tune::start_stream_session`, exactly what a tune does) decodes from it.
//!
//! Every test runs under a hard timeout: nothing here may hang.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::streaming_buffer::StreamingInner;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

/// Top-level atom `(offset, size)` in a complete MP4.
fn find_atom(data: &[u8], fourcc: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        if size < 8 {
            return None;
        }
        if &data[pos + 4..pos + 8] == fourcc {
            return Some((pos, size));
        }
        pos += size;
    }
    None
}

/// Add `delta` to every chunk offset (`stco`/`co64`) inside `moov`.
fn shift_chunk_offsets(moov: &mut [u8], delta: u64) {
    fn walk(data: &mut [u8], delta: u64) {
        let mut pos = 0usize;
        while pos + 8 <= data.len() {
            let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            if size < 8 || pos + size > data.len() {
                return;
            }
            let fourcc: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let body = &mut data[pos + 8..pos + size];
            match &fourcc {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => walk(body, delta),
                b"stco" => {
                    let n = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                    for i in 0..n {
                        let at = 8 + i * 4;
                        let v = u32::from_be_bytes(body[at..at + 4].try_into().unwrap());
                        let v = u32::try_from(u64::from(v) + delta).expect("stco overflow");
                        body[at..at + 4].copy_from_slice(&v.to_be_bytes());
                    }
                },
                b"co64" => {
                    let n = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                    for i in 0..n {
                        let at = 8 + i * 8;
                        let v = u64::from_be_bytes(body[at..at + 8].try_into().unwrap());
                        body[at..at + 8].copy_from_slice(&(v + delta).to_be_bytes());
                    }
                },
                _ => {},
            }
            pos += size;
        }
    }
    walk(moov, delta);
}

/// Grow `mdat` by `pad` unreferenced bytes at the start of its payload
/// (fixing up the chunk offsets), turning a small fixture into a large
/// file that takes the >10 MB code paths (tail probe, seek restart)
/// without committing a large fixture.  The layout stays realistic:
/// `mdat` still directly follows `moov` (or `ftyp`) as in a real file.
fn pad_mdat(data: &[u8], pad: usize) -> Vec<u8> {
    let (mdat_off, mdat_size) = find_atom(data, b"mdat").unwrap();
    let (moov_off, moov_size) = find_atom(data, b"moov").unwrap();
    let mut out = Vec::with_capacity(data.len() + pad);
    out.extend_from_slice(&data[..mdat_off]);
    out.extend_from_slice(&u32::try_from(mdat_size + pad).unwrap().to_be_bytes());
    out.extend_from_slice(b"mdat");
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(&data[mdat_off + 8..]);
    // moov either precedes mdat (unmoved) or follows it (moved by `pad`).
    let new_moov = if moov_off < mdat_off {
        moov_off
    } else {
        moov_off + pad
    };
    shift_chunk_offsets(&mut out[new_moov..new_moov + moov_size], pad as u64);
    out
}

/// Assert the buffer holds exactly `expected` starting at file offset
/// `base` (compact failure message instead of a byte dump).
fn assert_buffered(buffer: &StreamingInner, base: u64, expected: &[u8]) {
    let (got_base, got) = buffered(buffer);
    assert_eq!(got_base, base, "window start");
    assert_eq!(got.len(), expected.len(), "bytes buffered");
    let first_diff = got.iter().zip(expected).position(|(a, b)| a != b);
    assert_eq!(
        first_diff, None,
        "buffer differs from the file at this index"
    );
}

// ---------------------------------------------------------------------------
// Test HTTP server
// ---------------------------------------------------------------------------

/// Scriptable server misbehaviour.
#[derive(Clone, Default)]
struct Behavior {
    /// Answer Range requests with `200` + the full body.
    ignore_range: bool,
    /// Throttle body bytes to about this rate.
    bytes_per_sec: Option<u64>,
    /// Which body connection (0 = first) the drop/stall applies to.
    faulty_conn: usize,
    /// Close the faulty body connection after this many body bytes.
    drop_after: Option<usize>,
    /// Stop sending (but keep the socket open) on the faulty body
    /// connection after this many body bytes.
    stall_after: Option<usize>,
    /// Pause the faulty body connection for this long after exactly this
    /// many body bytes (a slow link delivering a packet boundary there).
    pause_after: Option<(usize, Duration)>,
}

#[derive(Debug, Clone)]
struct RequestLog {
    path: String,
    range_start: Option<u64>,
    status: u16,
}

struct TestServer {
    base: String,
    log: Arc<Mutex<Vec<RequestLog>>>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    /// Serve `files` (path -> body).  `/redirect/<name>` answers `302` to
    /// `/<name>`; anything unknown is `404`.
    fn start(files: HashMap<&'static str, Vec<u8>>, behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let files: Arc<HashMap<String, Arc<Vec<u8>>>> = Arc::new(
            files
                .into_iter()
                .map(|(k, v)| (format!("/{k}"), Arc::new(v)))
                .collect(),
        );
        let body_conns = Arc::new(AtomicUsize::new(0));
        {
            let (log, stop, base) = (Arc::clone(&log), Arc::clone(&stop), base.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((sock, _)) => {
                            let ctx = ConnCtx {
                                files: Arc::clone(&files),
                                behavior: behavior.clone(),
                                log: Arc::clone(&log),
                                stop: Arc::clone(&stop),
                                base: base.clone(),
                                body_conns: Arc::clone(&body_conns),
                            };
                            std::thread::spawn(move || ctx.serve(sock));
                        },
                        Err(_) => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
            });
        }
        Self { base, log, stop }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base)
    }

    fn requests(&self) -> Vec<RequestLog> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

struct ConnCtx {
    files: Arc<HashMap<String, Arc<Vec<u8>>>>,
    behavior: Behavior,
    log: Arc<Mutex<Vec<RequestLog>>>,
    stop: Arc<AtomicBool>,
    base: String,
    body_conns: Arc<AtomicUsize>,
}

impl ConnCtx {
    fn serve(self, mut sock: TcpStream) {
        let _ = sock.set_nonblocking(false);
        let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            match sock.read(&mut buf) {
                Ok(0) => return,
                Err(e) => {
                    eprintln!("server read err: {e:?}");
                    return;
                },
                Ok(n) => req.extend_from_slice(&buf[..n]),
            }
        }
        let text = String::from_utf8_lossy(&req).to_string();
        let path = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        let range_start = text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if !k.trim().eq_ignore_ascii_case("range") {
                return None;
            }
            v.trim()
                .strip_prefix("bytes=")?
                .split('-')
                .next()?
                .parse::<u64>()
                .ok()
        });
        let record = |status| {
            self.log.lock().unwrap().push(RequestLog {
                path: path.clone(),
                range_start,
                status,
            });
        };

        if let Some(target) = path.strip_prefix("/redirect") {
            record(302);
            let resp = format!(
                "HTTP/1.1 302 Found\r\nLocation: {}{target}\r\nContent-Length: 0\r\n\
                 Connection: close\r\n\r\n",
                self.base
            );
            let _ = sock.write_all(resp.as_bytes());
            return;
        }
        let Some(data) = self.files.get(&path).cloned() else {
            record(404);
            let _ = sock.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found",
            );
            return;
        };

        let len = data.len() as u64;
        let (status, start) = match range_start {
            Some(s) if !self.behavior.ignore_range => {
                if s >= len {
                    record(416);
                    let _ = sock.write_all(
                        format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\n\
                             Content-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                    return;
                }
                (206, s)
            },
            _ => (200, 0),
        };
        record(status);
        let body = &data[start as usize..];
        let mut head = format!(
            "HTTP/1.1 {status} {}\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n\
             Accept-Ranges: bytes\r\nConnection: close\r\n",
            if status == 206 {
                "Partial Content"
            } else {
                "OK"
            },
            body.len()
        );
        if status == 206 {
            head.push_str(&format!(
                "Content-Range: bytes {start}-{}/{len}\r\n",
                len - 1
            ));
        }
        head.push_str("\r\n");
        if sock.write_all(head.as_bytes()).is_err() {
            return;
        }

        let conn = self.body_conns.fetch_add(1, Ordering::AcqRel);
        let first = conn == self.behavior.faulty_conn;
        let chunk = 8 * 1024;
        let mut sent = 0usize;
        while sent < body.len() {
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            if first && self.behavior.drop_after.is_some_and(|n| sent >= n) {
                let _ = sock.shutdown(std::net::Shutdown::Both);
                return;
            }
            if first && self.behavior.stall_after.is_some_and(|n| sent >= n) {
                // Hold the connection open without sending anything.
                let until = Instant::now() + Duration::from_secs(30);
                while Instant::now() < until && !self.stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                return;
            }
            if first
                && let Some((at, pause)) = self.behavior.pause_after
                && sent == at
            {
                std::thread::sleep(pause);
            }
            let mut end = (sent + chunk).min(body.len());
            if first {
                let pause_at = self.behavior.pause_after.map(|(at, _)| at);
                for limit in [
                    self.behavior.drop_after,
                    self.behavior.stall_after,
                    pause_at,
                ]
                .into_iter()
                .flatten()
                {
                    if sent < limit {
                        end = end.min(limit);
                    }
                }
            }
            if sock.write_all(&body[sent..end]).is_err() {
                return;
            }
            if let Some(rate) = self.behavior.bytes_per_sec {
                let secs = (end - sent) as f64 / rate as f64;
                std::thread::sleep(Duration::from_secs_f64(secs));
            }
            sent = end;
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Run `f` on a worker thread; fail the test if it takes longer than `secs`.
fn with_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    // `RUST_LOG=info` shows the pipeline's own logging for a failing test.
    let _ = env_logger::builder().is_test(true).try_init();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => {
            let _ = handle.join();
            v
        },
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("test exceeded hard timeout of {secs}s (hang)")
        },
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("worker exited without a result"),
        },
    }
}

fn tls() -> oasis_core::net::RustlsTlsProvider {
    oasis_core::net::RustlsTlsProvider::new()
}

/// Run the real download into a fresh buffer; return it and the result.
fn download(url: &str, seek_secs: u64) -> (Arc<StreamingInner>, Result<(), String>) {
    let buffer = Arc::new(StreamingInner::new());
    let result = super::download::stream_download(url, &tls(), &buffer, seek_secs);
    (buffer, result)
}

/// The contiguous bytes the buffer holds, as `(file_offset, bytes)`.
fn buffered(buffer: &StreamingInner) -> (u64, Vec<u8>) {
    let s = buffer.state.lock().unwrap();
    (s.base_offset, s.buf.clone())
}

// ---------------------------------------------------------------------------
// Download layer: HTTP behaviour -> bytes in the StreamingInner
// ---------------------------------------------------------------------------

#[test]
fn download_plain_http_delivers_exact_bytes() {
    with_timeout(30, || {
        for name in ["test_320x240_2s.mp4", "streaming/moov_end_4s.mp4"] {
            let data = fixture(name);
            let server = TestServer::start(
                HashMap::from([("v.mp4", data.clone())]),
                Behavior::default(),
            );
            let (buffer, result) = download(&server.url("v.mp4"), 0);
            result.unwrap();
            assert!(buffer.is_done());
            assert_buffered(&buffer, 0, &data);
            assert_eq!(buffer.total_size.load(Ordering::Acquire), data.len() as u64);
            assert!(
                buffer.state.lock().unwrap().moov.is_some(),
                "{name}: moov retained"
            );
        }
    });
}

#[test]
fn download_follows_redirect() {
    with_timeout(30, || {
        let data = fixture("streaming/bframes_4s.mp4");
        let server = TestServer::start(
            HashMap::from([("v.mp4", data.clone())]),
            Behavior::default(),
        );
        let (buffer, result) = download(&server.url("redirect/v.mp4"), 0);
        result.unwrap();
        assert_buffered(&buffer, 0, &data);
        let reqs: Vec<(String, u16)> = server
            .requests()
            .into_iter()
            .map(|r| (r.path, r.status))
            .collect();
        assert_eq!(
            reqs,
            [
                ("/redirect/v.mp4".to_string(), 302),
                ("/v.mp4".to_string(), 200)
            ]
        );
    });
}

#[test]
fn download_404_is_an_error() {
    with_timeout(30, || {
        let server = TestServer::start(HashMap::new(), Behavior::default());
        let (_buffer, result) = download(&server.url("missing.mp4"), 0);
        let err = result.unwrap_err();
        assert!(err.contains("404"), "{err}");
    });
}

#[test]
fn download_throttled_bandwidth_completes() {
    with_timeout(60, || {
        let data = fixture("streaming/noaudio_2s.mp4");
        let behavior = Behavior {
            bytes_per_sec: Some(64 * 1024),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let t0 = Instant::now();
        let (buffer, result) = download(&server.url("v.mp4"), 0);
        result.unwrap();
        assert_buffered(&buffer, 0, &data);
        assert!(
            t0.elapsed() >= Duration::from_millis(200),
            "throttle not applied"
        );
    });
}

#[test]
fn download_resumes_after_connection_drop() {
    // The server closes the linear download mid-body. The download must
    // resume with a Range request from the last byte received instead of
    // treating the short body as the end of the file.
    with_timeout(60, || {
        let data = fixture("streaming/moov_end_4s.mp4");
        let behavior = Behavior {
            drop_after: Some(30_000),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let (buffer, result) = download(&server.url("v.mp4"), 0);
        result.unwrap();
        let (base, bytes) = buffered(&buffer);
        assert_eq!(base, 0);
        assert_eq!(
            bytes.len(),
            data.len(),
            "download ended early at {}",
            bytes.len()
        );
        assert!(bytes == data, "resumed bytes differ");
        assert!(
            server
                .requests()
                .iter()
                .any(|r| r.range_start == Some(30_000)),
            "no Range resume: {:?}",
            server.requests()
        );
    });
}

#[test]
fn download_resumes_after_stall() {
    // The server stops sending but keeps the socket open. Stall detection
    // must reconnect with a Range request rather than wait out the 120 s
    // download deadline.
    with_timeout(60, || {
        let data = fixture("streaming/bframes_4s.mp4");
        let behavior = Behavior {
            stall_after: Some(40_000),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let t0 = Instant::now();
        let (buffer, result) = download(&server.url("v.mp4"), 0);
        result.unwrap();
        assert_buffered(&buffer, 0, &data);
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "took {:?}",
            t0.elapsed()
        );
        assert!(
            server
                .requests()
                .iter()
                .any(|r| r.range_start == Some(40_000))
        );
    });
}

#[test]
fn download_resume_against_server_ignoring_range() {
    // Resume after a drop when the server answers Range with 200 + the
    // full body: the prefix already received must be skipped, not
    // appended again.
    with_timeout(60, || {
        let data = fixture("streaming/moov_end_4s.mp4");
        let behavior = Behavior {
            drop_after: Some(25_000),
            ignore_range: true,
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let (buffer, result) = download(&server.url("v.mp4"), 0);
        result.unwrap();
        assert_buffered(&buffer, 0, &data);
    });
}

#[test]
fn range_download_resumes_after_early_close() {
    // Tuned mid-episode (seek restart via Range), then the Range
    // connection closes early: resume from the frontier instead of
    // treating the short body as end-of-file.
    with_timeout(60, || {
        let data = pad_mdat(&fixture("streaming/bframes_4s.mp4"), 20 << 20);
        let behavior = Behavior {
            faulty_conn: 1,
            drop_after: Some(100_000),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let (buffer, result) = download(&server.url("v.mp4"), 2);
        result.unwrap();
        let (base, _) = buffered(&buffer);
        assert!(base > 19 << 20, "no restart");
        assert_buffered(&buffer, base, &data[base as usize..]);
        let ranges: Vec<u64> = server
            .requests()
            .iter()
            .filter_map(|r| r.range_start)
            .collect();
        assert_eq!(ranges, [base, base + 100_000]);
    });
}

#[test]
fn range_download_survives_a_long_throttle() {
    // The decoder falls behind (throttle engaged) for longer than the old
    // 9 s "stalled while throttling" limit. That is normal playback of
    // low-bitrate video, not a network stall: no reconnects, no error.
    with_timeout(60, || {
        let data = pad_mdat(&fixture("streaming/bframes_4s.mp4"), 20 << 20);
        let total = data.len() as u64;
        let server = TestServer::start(
            HashMap::from([("v.mp4", data.clone())]),
            Behavior::default(),
        );
        let buffer = Arc::new(StreamingInner::new());
        buffer.total_size.store(total, Ordering::Release);
        // Decoder parked at byte 1: the download pauses at 16 MB ahead.
        buffer.decoder_pos.store(1, Ordering::Release);
        let thread_buf = Arc::clone(&buffer);
        let url = server.url("v.mp4");
        let handle = std::thread::spawn(move || {
            super::download::stream_download_range(&url, &tls(), &thread_buf, 0, total)
        });
        std::thread::sleep(Duration::from_secs(11));
        assert!(!buffer.is_done(), "download gave up while throttled");
        assert!(buffer.error.lock().unwrap().is_none());
        // The decoder catches up; the download finishes on the same
        // connection.
        buffer.decoder_pos.store(total, Ordering::Release);
        buffer.condvar.notify_all();
        handle.join().unwrap().unwrap();
        assert!(buffer.error.lock().unwrap().is_none());
        assert_buffered(&buffer, 0, &data);
        let ranges = server
            .requests()
            .iter()
            .filter(|r| r.range_start.is_some())
            .count();
        assert_eq!(
            ranges,
            1,
            "reconnected while throttled: {:?}",
            server.requests()
        );
    });
}

#[test]
fn download_truncated_file_ends_at_content_length() {
    with_timeout(30, || {
        let data = fixture("streaming/bframes_4s.mp4");
        let cut = data[..data.len() / 2].to_vec();
        let server =
            TestServer::start(HashMap::from([("v.mp4", cut.clone())]), Behavior::default());
        let (buffer, result) = download(&server.url("v.mp4"), 0);
        result.unwrap();
        assert_buffered(&buffer, 0, &cut);
    });
}

#[test]
fn download_cancel_stops_promptly() {
    with_timeout(30, || {
        let data = pad_mdat(&fixture("streaming/bframes_4s.mp4"), 12 << 20);
        let behavior = Behavior {
            bytes_per_sec: Some(256 * 1024),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data)]), behavior);
        let buffer = Arc::new(StreamingInner::new());
        let thread_buf = Arc::clone(&buffer);
        let url = server.url("v.mp4");
        let handle = std::thread::spawn(move || {
            super::download::stream_download(&url, &tls(), &thread_buf, 0)
        });
        std::thread::sleep(Duration::from_millis(300));
        let t0 = Instant::now();
        buffer.cancel();
        handle.join().unwrap().unwrap();
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "cancel took {:?}",
            t0.elapsed()
        );
    });
}

#[test]
fn moov_at_end_large_file_found_by_tail_probe() {
    // > 10 MB with moov at the end: the deferred tail probe must fetch
    // moov via Range long before the linear download gets there.
    with_timeout(60, || {
        let data = pad_mdat(&fixture("streaming/moov_end_4s.mp4"), 16 << 20);
        let (moov_off, _) = find_atom(&data, b"moov").unwrap();
        let behavior = Behavior {
            bytes_per_sec: Some(8 << 20),
            ..Behavior::default()
        };
        let server = TestServer::start(HashMap::from([("v.mp4", data.clone())]), behavior);
        let buffer = Arc::new(StreamingInner::new());
        let thread_buf = Arc::clone(&buffer);
        let url = server.url("v.mp4");
        std::thread::spawn(move || super::download::stream_download(&url, &tls(), &thread_buf, 0));
        let moov = buffer.wait_for_moov(Duration::from_secs(20)).expect("moov");
        assert_eq!(moov.len(), data.len() - moov_off);
        assert!(
            buffer.bytes_received() < moov_off as u64,
            "moov only found by the linear download"
        );
        assert!(server.requests().iter().any(|r| r.range_start.is_some()));
        buffer.cancel();
    });
}

#[test]
fn seek_restart_skips_ahead_with_range_request() {
    // > 10 MB, moov at start, tuned mid-episode: after moov arrives the
    // download restarts near the seek point instead of pulling the whole
    // prefix.
    with_timeout(60, || {
        let data = pad_mdat(&fixture("streaming/bframes_4s.mp4"), 20 << 20);
        let server = TestServer::start(
            HashMap::from([("v.mp4", data.clone())]),
            Behavior::default(),
        );
        let (buffer, result) = download(&server.url("v.mp4"), 2);
        result.unwrap();
        let (base, bytes) = buffered(&buffer);
        assert!(base > 19 << 20, "no restart: window starts at {base}");
        assert_eq!(
            &data[base as usize..],
            &bytes[..],
            "restart bytes misaligned"
        );
        let ranged = server
            .requests()
            .iter()
            .filter(|r| r.range_start.is_some())
            .count();
        assert_eq!(ranged, 1, "{:?}", server.requests());
    });
}

// ---------------------------------------------------------------------------
// Full player: tune -> buffering -> playing -> channel change -> stop
// ---------------------------------------------------------------------------

#[cfg(feature = "video-decode")]
mod player {
    use super::*;
    use crate::video_player::{AudioOutput, PlayerState, VideoPlayer};
    use oasis_core::backend::{
        Color, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiShapes, SdiText,
        SdiTextures, SdiVector, TextureId,
    };
    use oasis_core::error::Result as OResult;

    /// Backend recording uploaded frame textures.
    #[derive(Default)]
    struct FrameSink {
        next_id: u32,
        live: usize,
        uploads: Vec<(u32, u32, u64)>,
    }

    fn hash(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in data.iter().step_by(7) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    impl SdiCore for FrameSink {
        fn init(&mut self, _w: u32, _h: u32) -> OResult<()> {
            Ok(())
        }
        fn clear(&mut self, _c: Color) -> OResult<()> {
            Ok(())
        }
        fn blit(&mut self, _t: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> OResult<()> {
            Ok(())
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _c: Color) -> OResult<()> {
            Ok(())
        }
        fn draw_text(&mut self, _t: &str, _x: i32, _y: i32, _s: u16, _c: Color) -> OResult<()> {
            Ok(())
        }
        fn swap_buffers(&mut self) -> OResult<()> {
            Ok(())
        }
        fn load_texture(&mut self, w: u32, h: u32, d: &[u8]) -> OResult<TextureId> {
            assert_eq!(d.len(), (w * h * 4) as usize, "texture size mismatch");
            self.next_id += 1;
            self.live += 1;
            self.uploads.push((w, h, hash(d)));
            Ok(TextureId(u64::from(self.next_id)))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> OResult<()> {
            self.live -= 1;
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> OResult<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> OResult<()> {
            Ok(())
        }
        fn measure_text(&self, _t: &str, _s: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> OResult<Vec<u8>> {
            Ok(Vec::new())
        }
        fn shutdown(&mut self) -> OResult<()> {
            Ok(())
        }
    }
    impl SdiShapes for FrameSink {}
    impl SdiGradients for FrameSink {}
    impl SdiAlpha for FrameSink {}
    impl SdiText for FrameSink {}
    impl SdiTextures for FrameSink {}
    impl SdiClipTransform for FrameSink {}
    impl SdiVector for FrameSink {}
    impl SdiBatch for FrameSink {}
    impl oasis_core::backend::SdiRenderTarget for FrameSink {}

    /// What happened during a playback session.
    #[derive(Default, Debug)]
    struct Session {
        /// `(virtual secs since tune, pts)` for every displayed frame.
        shown: Vec<(f64, f64)>,
        /// Player state after each tick (deduplicated transitions).
        states: Vec<PlayerState>,
        /// Interleaved audio samples per channel, and the last format.
        audio_frames: usize,
        audio_format: Option<(u32, u16)>,
        finished: bool,
        /// Virtual seconds from tune to the first frame.
        first_frame_at: Option<f64>,
    }

    /// Drive `player` with a virtual 60 Hz clock until it finishes or
    /// `max_virtual` seconds pass.  Each tick gives the (real-time)
    /// decode thread `real_step` to make progress.
    fn run(
        player: &mut VideoPlayer,
        sink: &mut FrameSink,
        max_virtual: f64,
        real_step: Duration,
    ) -> Session {
        let t0 = Instant::now();
        let tick = Duration::from_micros(16_667);
        let mut now = t0;
        let mut out = Session::default();
        let mut last_pts_log = 0usize;
        loop {
            let (_tex, audio) = player.tick_at(sink, now);
            let st = player.state();
            if out.states.last() != Some(&st) {
                out.states.push(st);
            }
            let log = player.display_log();
            for &(at, pts) in &log[last_pts_log..] {
                let v = at.duration_since(t0).as_secs_f64();
                out.first_frame_at.get_or_insert(v);
                out.shown.push((v, pts));
            }
            last_pts_log = log.len();
            if let AudioOutput::PcmF32(chunks) = audio {
                for c in chunks {
                    out.audio_frames += c.pcm_f32.len() / usize::from(c.channels.max(1));
                    out.audio_format = Some((c.sample_rate, c.channels));
                }
            }
            if player.is_finished() {
                out.finished = true;
                break;
            }
            if now.duration_since(t0).as_secs_f64() > max_virtual {
                break;
            }
            now += tick;
            std::thread::sleep(real_step);
        }
        out
    }

    fn tune(player: &mut VideoPlayer, url: &str, seek: u64) -> Arc<StreamingInner> {
        super::super::tune::start_stream_session(player, url, tls(), seek, 480, 272)
    }

    /// Frame pacing: every frame shown once, in order, on time.
    fn assert_steady(s: &Session, fps: f64, expected_frames: usize) {
        let dur = 1.0 / fps;
        assert_eq!(
            s.shown.len(),
            expected_frames,
            "frames shown: {:?}",
            s.shown
        );
        for w in s.shown.windows(2) {
            let (t0, p0) = w[0];
            let (t1, p1) = w[1];
            assert!(
                (p1 - p0 - dur).abs() < dur * 0.25,
                "pts jump {p0:.3} -> {p1:.3} (skipped/duplicated frame)"
            );
            // Displayed within one 60 Hz tick of its slot, relative to the
            // first frame.
            let (ts, ps) = s.shown[0];
            let due = ts + (p1 - ps);
            assert!(
                t1 >= due - 1e-6 && t1 - due < 0.034,
                "frame pts {p1:.3} shown at {t1:.3}, due {due:.3} (prev at {t0:.3})"
            );
        }
    }

    fn assert_transitions(s: &Session) {
        // Idle-free lifecycle: Starting -> Playing, never back to Starting
        // (no buffering/playing oscillation).
        assert_eq!(
            s.states.first(),
            Some(&PlayerState::Starting),
            "{:?}",
            s.states
        );
        let playing = s
            .states
            .iter()
            .filter(|st| **st == PlayerState::Playing)
            .count();
        assert!(playing <= 1, "state thrash: {:?}", s.states);
        assert!(!s.states.contains(&PlayerState::Error), "{:?}", s.states);
    }

    #[test]
    fn tune_plays_whole_episode_with_steady_pacing_and_audio() {
        with_timeout(60, || {
            let data = fixture("streaming/bframes_4s.mp4");
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 10.0, Duration::from_millis(4));
            assert!(s.finished, "never finished: {:?}", s.states);
            assert_transitions(&s);
            assert_steady(&s, 24.0, 96);
            assert_eq!(s.shown[0].1, 0.0);
            assert!(sink.uploads.iter().all(|&(w, h, _)| (w, h) == (320, 240)));
            // All audio delivered: 4 s at 48 kHz stereo (+/- 3 AAC frames).
            assert_eq!(s.audio_format, Some((48000, 2)));
            let expected = 4.0 * 48000.0;
            assert!(
                (s.audio_frames as f64 - expected).abs() <= 3.0 * 1024.0,
                "audio frames {} vs {expected}",
                s.audio_frames
            );
            session.cancel();
        });
    }

    #[test]
    fn mono_audio_reaches_the_player_as_mono() {
        with_timeout(60, || {
            let data = fixture("streaming/odd_250x138_2s.mp4");
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 8.0, Duration::from_millis(4));
            assert!(s.finished);
            assert_steady(&s, 20.0, 40);
            assert!(sink.uploads.iter().all(|&(w, h, _)| (w, h) == (250, 138)));
            assert_eq!(s.audio_format, Some((22050, 1)));
            let expected = 2.0 * 22050.0;
            assert!((s.audio_frames as f64 - expected).abs() <= 3.0 * 1024.0);
        });
    }

    #[test]
    fn video_only_episode_plays_and_finishes() {
        with_timeout(60, || {
            let data = fixture("streaming/noaudio_2s.mp4");
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 8.0, Duration::from_millis(4));
            assert!(s.finished);
            assert_transitions(&s);
            assert_steady(&s, 15.0, 30);
            assert_eq!(s.audio_frames, 0);
        });
    }

    #[test]
    fn audio_only_episode_delivers_all_audio() {
        // No video track: the decode thread runs in audio-only mode. The
        // player must hand over every sample (the audio backend paces
        // playback), not drop what doesn't fit the channel.
        with_timeout(60, || {
            let data = fixture("streaming/audio_only_30s.mp4");
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 60.0, Duration::from_millis(4));
            assert!(s.finished, "{:?}", s.states);
            assert!(s.shown.is_empty());
            assert_eq!(s.audio_format, Some((22050, 1)));
            let expected = 30.0 * 22050.0;
            assert!(
                (s.audio_frames as f64 - expected).abs() <= 3.0 * 1024.0,
                "audio frames {} vs {expected}",
                s.audio_frames
            );
        });
    }

    #[test]
    fn tune_mid_episode_starts_at_keyframe() {
        with_timeout(60, || {
            let data = pad_mdat(&fixture("streaming/bframes_4s.mp4"), 20 << 20);
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 2);
            let s = run(&mut player, &mut sink, 10.0, Duration::from_millis(4));
            assert!(s.finished, "{:?}", s.states);
            assert_transitions(&s);
            assert_eq!(s.shown.first().map(|f| f.1), Some(2.0));
            assert_steady(&s, 24.0, 48);
            let expected = 2.0 * 48000.0;
            assert!(
                (s.audio_frames as f64 - expected).abs() <= 3.0 * 1024.0,
                "audio frames {} vs {expected}",
                s.audio_frames
            );
        });
    }

    #[test]
    fn data_after_moov_arriving_late_does_not_break_the_probe() {
        // The download delivers moov, then pauses (a packet boundary on a
        // slow link) before the atoms that follow it. The decoder opens
        // during the pause; it must not mistake the not-yet-downloaded
        // `free`/`mdat` headers for zeros -- tuned from the start or
        // mid-episode.
        with_timeout(60, || {
            let data = fixture("streaming/bframes_4s.mp4");
            let (moov_off, moov_size) = find_atom(&data, b"moov").unwrap();
            for seek in [0u64, 2] {
                let behavior = Behavior {
                    pause_after: Some((moov_off + moov_size, Duration::from_millis(700))),
                    ..Behavior::default()
                };
                let server = TestServer::start(HashMap::from([("ep.mp4", data.clone())]), behavior);
                let mut player = VideoPlayer::new();
                let mut sink = FrameSink::default();
                let _session = tune(&mut player, &server.url("ep.mp4"), seek);
                let s = run(&mut player, &mut sink, 12.0, Duration::from_millis(4));
                assert!(s.finished, "seek {seek}: {:?}", s.states);
                assert_transitions(&s);
                assert_eq!(
                    s.shown.first().map(|f| f.1),
                    Some(seek as f64),
                    "seek {seek}"
                );
                assert_steady(&s, 24.0, 96 - 24 * seek as usize);
            }
        });
    }

    #[test]
    fn moov_at_end_episode_plays() {
        with_timeout(60, || {
            let data = fixture("streaming/moov_end_4s.mp4");
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 10.0, Duration::from_millis(4));
            assert!(s.finished);
            assert_transitions(&s);
            assert_steady(&s, 24.0, 96);
        });
    }

    #[test]
    fn slow_network_buffers_then_plays_without_skipping() {
        // Download slower than real time: playback stalls and resumes. Every
        // frame is still shown exactly once and in order (the clock rebases
        // after a stall instead of skipping ahead).
        with_timeout(90, || {
            let data = fixture("streaming/moov_end_4s.mp4");
            let behavior = Behavior {
                bytes_per_sec: Some(16 * 1024),
                ..Behavior::default()
            };
            let server = TestServer::start(HashMap::from([("ep.mp4", data)]), behavior);
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            // Real-time ticking: the network is real time too.
            let s = run(&mut player, &mut sink, 60.0, Duration::from_micros(16_667));
            assert!(s.finished, "{:?}", s.states);
            assert_transitions(&s);
            assert_eq!(s.shown.len(), 96, "every frame shown once");
            for w in s.shown.windows(2) {
                assert!(w[1].1 > w[0].1, "pts out of order");
            }
        });
    }

    #[test]
    fn channel_change_switches_streams_and_releases_the_old_session() {
        with_timeout(60, || {
            let a = fixture("streaming/bframes_4s.mp4");
            let b = fixture("streaming/odd_250x138_2s.mp4");
            let server = TestServer::start(
                HashMap::from([("a.mp4", a), ("b.mp4", b)]),
                Behavior::default(),
            );
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();

            let session_a = tune(&mut player, &server.url("a.mp4"), 0);
            let s = run(&mut player, &mut sink, 1.0, Duration::from_millis(4));
            assert!(!s.shown.is_empty(), "channel A never started");
            assert!(!s.finished);

            // Channel change, as `handle_tune_requests` does it.
            session_a.cancel();
            player.stop(&mut sink);
            assert_eq!(player.state(), PlayerState::Idle);
            let uploads_before = sink.uploads.len();
            let session_b = tune(&mut player, &server.url("b.mp4"), 0);
            let s = run(&mut player, &mut sink, 8.0, Duration::from_millis(4));
            assert!(s.finished);
            assert_transitions(&s);
            assert_steady(&s, 20.0, 40);
            assert!(
                sink.uploads[uploads_before..]
                    .iter()
                    .all(|&(w, h, _)| (w, h) == (250, 138)),
                "frame from the old channel shown after the switch"
            );

            // The old session's download + decode threads let go of it.
            let deadline = Instant::now() + Duration::from_secs(5);
            while Arc::strong_count(&session_a) > 1 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                Arc::strong_count(&session_a),
                1,
                "old session still referenced"
            );

            player.stop(&mut sink);
            session_b.cancel();
            assert_eq!(sink.live, 0, "texture leak");
        });
    }

    #[test]
    fn missing_episode_ends_the_session_quickly() {
        with_timeout(30, || {
            let server = TestServer::start(HashMap::new(), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let t0 = Instant::now();
            let _session = tune(&mut player, &server.url("gone.mp4"), 0);
            let s = run(&mut player, &mut sink, 20.0, Duration::from_millis(10));
            assert!(s.finished, "404 left the player waiting: {:?}", s.states);
            assert!(s.shown.is_empty());
            assert!(
                t0.elapsed() < Duration::from_secs(10),
                "took {:?}",
                t0.elapsed()
            );
        });
    }

    #[test]
    fn truncated_episode_plays_what_it_has_then_finishes() {
        with_timeout(60, || {
            let data = fixture("streaming/bframes_4s.mp4");
            let cut = data[..data.len() * 6 / 10].to_vec();
            let server = TestServer::start(HashMap::from([("ep.mp4", cut)]), Behavior::default());
            let mut player = VideoPlayer::new();
            let mut sink = FrameSink::default();
            let _session = tune(&mut player, &server.url("ep.mp4"), 0);
            let s = run(&mut player, &mut sink, 10.0, Duration::from_millis(4));
            assert!(s.finished);
            assert!(
                s.shown.len() > 10 && s.shown.len() < 96,
                "{}",
                s.shown.len()
            );
            for w in s.shown.windows(2) {
                assert!(w[1].1 > w[0].1);
            }
        });
    }
}
