//! Loopback end-to-end tests for the FTP-like transfer server.
//!
//! `FtpServer` runs over a real `StdNetworkBackend` bound to an ephemeral
//! port and is polled on the test thread against a `MemoryVfs` or a
//! `RealVfs` rooted in a temp directory; clients are raw `TcpStream`s on
//! helper threads. Every wait has a deadline.
//!
//! The wire protocol is line-based text (`PUT <path> <content>` carries the
//! content inline on one line, `GET` returns `200 <n> bytes\n` followed by
//! the text), so only single-line text payloads can be uploaded; binary
//! round-trips are not representable and are not tested.

#![allow(clippy::unwrap_used)]

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use oasis_core::net::StdNetworkBackend;
use oasis_core::transfer::FtpServer;
use oasis_core::vfs::{MemoryVfs, RealVfs, Vfs};

const DEADLINE: Duration = Duration::from_secs(10);

fn start(server: FtpServer) -> (StdNetworkBackend, FtpServer, u16) {
    let mut backend = StdNetworkBackend::new();
    let mut server = server;
    server.start(&mut backend).unwrap();
    let port = backend.local_addr().unwrap().port();
    (backend, server, port)
}

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    s
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn read_until(s: &mut TcpStream, acc: &mut Vec<u8>, marker: &[u8]) -> bool {
    let start = Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    while !contains(acc, marker) {
        if start.elapsed() > DEADLINE {
            return false;
        }
        match s.read(&mut buf) {
            Ok(0) => return contains(acc, marker),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {},
            Err(_) => return contains(acc, marker),
        }
    }
    true
}

fn read_to_eof(s: &mut TcpStream) -> (Vec<u8>, bool) {
    let start = Instant::now();
    let mut acc = Vec::new();
    let mut buf = [0u8; 4096];
    while start.elapsed() < DEADLINE {
        match s.read(&mut buf) {
            Ok(0) => return (acc, true),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {},
            Err(_) => return (acc, true),
        }
    }
    (acc, false)
}

/// Line-oriented client over a raw socket.
struct Client {
    s: TcpStream,
    buf: Vec<u8>,
}

impl Client {
    fn open(port: u16) -> Self {
        let mut c = Self {
            s: connect(port),
            buf: Vec::new(),
        };
        let greeting = c.line();
        assert!(greeting.starts_with("220 "), "{greeting}");
        c
    }

    /// Next `\n`-terminated line (without the terminator).
    fn line(&mut self) -> String {
        assert!(
            read_until(&mut self.s, &mut self.buf, b"\n"),
            "no line; have {:?}",
            String::from_utf8_lossy(&self.buf)
        );
        let pos = self.buf.iter().position(|&b| b == b'\n').unwrap();
        let line: Vec<u8> = self.buf.drain(..=pos).collect();
        String::from_utf8_lossy(&line).trim_end().to_string()
    }

    /// Exactly `n` raw bytes.
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let start = Instant::now();
        let mut chunk = vec![0u8; 64 * 1024];
        while self.buf.len() < n {
            assert!(start.elapsed() < DEADLINE, "short body");
            match self.s.read(&mut chunk) {
                Ok(0) => panic!("EOF after {} of {n} bytes", self.buf.len()),
                Ok(k) => self.buf.extend_from_slice(&chunk[..k]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {},
                Err(e) => panic!("read: {e}"),
            }
        }
        self.buf.drain(..n).collect()
    }

    fn send(&mut self, line: &str) {
        self.s.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    fn cmd(&mut self, line: &str) -> String {
        self.send(line);
        self.line()
    }

    /// `GET` and return the body.
    fn get(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let head = self.cmd(&format!("GET {path}"));
        let Some(rest) = head.strip_prefix("200 ") else {
            return Err(head);
        };
        let n: usize = rest.strip_suffix(" bytes").unwrap().parse().unwrap();
        Ok(self.bytes(n))
    }

    /// Multi-line `LIST` response (`200 ...` then entry lines); returns
    /// the entry names. Relies on the entry count being known.
    fn list(&mut self, path: &str, entries: usize) -> Vec<String> {
        let first = self.cmd(&format!("LIST {path}"));
        assert!(first.starts_with("200 "), "{first}");
        if first == "200 (empty)" {
            return Vec::new();
        }
        let mut lines = vec![first["200 ".len()..].to_string()];
        for _ in 1..entries {
            lines.push(self.line());
        }
        lines
            .iter()
            .map(|l| l.rsplit(' ').next().unwrap().to_string())
            .collect()
    }
}

/// Poll the server until the client thread finishes.
fn serve<T>(
    backend: &mut StdNetworkBackend,
    server: &mut FtpServer,
    vfs: &mut dyn Vfs,
    client: JoinHandle<T>,
) -> T {
    let start = Instant::now();
    while !client.is_finished() {
        server.poll(backend, vfs).unwrap();
        assert!(start.elapsed() < DEADLINE, "client thread did not finish");
        thread::sleep(Duration::from_millis(1));
    }
    client.join().unwrap()
}

fn poll_until(
    backend: &mut StdNetworkBackend,
    server: &mut FtpServer,
    vfs: &mut dyn Vfs,
    mut cond: impl FnMut(&FtpServer) -> bool,
) {
    let start = Instant::now();
    while !cond(server) {
        server.poll(backend, vfs).unwrap();
        assert!(start.elapsed() < DEADLINE, "condition not reached");
        thread::sleep(Duration::from_millis(1));
    }
}

/// FNV-1a, enough to compare payloads end to end.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Printable single-line payload with spaces inside.
fn payload(len: usize, seed: u64) -> String {
    let mut x = seed;
    (0..len)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            if i == 0 || i == len - 1 {
                'x'
            } else {
                char::from(b' ' + (x % 95) as u8)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------

#[test]
fn unauthenticated_server_binds_loopback_only() {
    let (backend, _server, _port) = start(FtpServer::new(0));
    let addr = backend.local_addr().unwrap();
    assert!(
        addr.ip().is_loopback(),
        "password-less FTP server bound {addr}: every host on the network could read/write files"
    );
}

#[test]
fn memory_vfs_put_get_round_trip_with_checksums() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/up").unwrap();
    let payloads: Vec<String> = (0..8)
        .map(|i| payload(100 + i * 110, i as u64 + 1))
        .collect();
    let sent = payloads.clone();
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        let mut sums = Vec::new();
        for (i, p) in sent.iter().enumerate() {
            let resp = c.cmd(&format!("PUT /up/f{i}.txt {p}"));
            assert_eq!(
                resp,
                format!("200 written {} bytes to /up/f{i}.txt", p.len())
            );
        }
        for i in 0..sent.len() {
            let body = c.get(&format!("/up/f{i}.txt")).unwrap();
            sums.push(fnv1a(&body));
        }
        assert_eq!(c.cmd("QUIT"), "200 goodbye");
        sums
    });
    let sums = serve(&mut backend, &mut server, &mut vfs, client);
    for (i, p) in payloads.iter().enumerate() {
        assert_eq!(sums[i], fnv1a(p.as_bytes()), "payload {i} corrupted");
        assert_eq!(vfs.read(&format!("/up/f{i}.txt")).unwrap(), p.as_bytes());
    }
    poll_until(&mut backend, &mut server, &mut vfs, |s| {
        s.connection_count() == 0
    });
}

#[test]
fn real_vfs_file_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let mut vfs = RealVfs::new(tmp.path()).unwrap();
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        assert!(c.cmd("MKDIR /docs").starts_with("200 created"));
        assert!(
            c.cmd("PUT /docs/a.txt hello world")
                .starts_with("200 written 11")
        );
        assert_eq!(c.cmd("STAT /docs/a.txt"), "200 file 11 bytes");
        assert!(c.cmd("STAT /docs").starts_with("200 directory"));
        assert_eq!(c.list("/docs", 1), vec!["a.txt".to_string()]);
        assert!(
            c.cmd("RENAME /docs/a.txt /docs/b.txt")
                .starts_with("200 renamed")
        );
        assert!(c.cmd("STAT /docs/a.txt").starts_with("500 "));
        assert_eq!(c.get("/docs/b.txt").unwrap(), b"hello world");
        assert!(c.cmd("GET /docs/missing.txt").starts_with("500 "));
        assert!(c.cmd("PUT /docs/c.txt second").starts_with("200 "));
        assert!(c.cmd("DELETE /docs/b.txt").starts_with("200 deleted"));
        assert_eq!(c.list("/docs", 1), vec!["c.txt".to_string()]);
        assert!(c.cmd("FROB x").starts_with("400 unknown command"));
        assert!(c.cmd("GET").starts_with("400 "));
    });
    serve(&mut backend, &mut server, &mut vfs, client);
    let on_disk = std::fs::read(tmp.path().join("docs").join("c.txt")).unwrap();
    assert_eq!(on_disk, b"second");
    assert!(!tmp.path().join("docs").join("b.txt").exists());
}

#[test]
fn real_vfs_refuses_path_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, b"TOPSECRET").unwrap();
    let secret_abs = secret.to_string_lossy().replace('\\', "/");
    let mut vfs = RealVfs::new(&root).unwrap();

    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        let mut transcript = String::new();
        for req in [
            "GET ../secret.txt".to_string(),
            "GET /../secret.txt".to_string(),
            "GET ../../secret.txt".to_string(),
            "GET /sub/../../secret.txt".to_string(),
            "GET ..\\secret.txt".to_string(),
            format!("GET {secret_abs}"),
            "STAT ../secret.txt".to_string(),
            "LIST ..".to_string(),
            "LIST /sub/../..".to_string(),
            "PUT ../escape.txt pwned".to_string(),
            "PUT /nope/../../escape.txt pwned".to_string(),
            "MKDIR ../evil".to_string(),
            "RENAME /sub ../moved".to_string(),
            "DELETE ../secret.txt".to_string(),
        ] {
            let resp = c.cmd(&req);
            assert!(resp.starts_with("500 "), "{req:?} was not refused: {resp}");
            transcript.push_str(&resp);
        }
        transcript
    });
    let transcript = serve(&mut backend, &mut server, &mut vfs, client);
    assert!(!transcript.contains("TOPSECRET"));
    assert_eq!(std::fs::read(&secret).unwrap(), b"TOPSECRET");
    assert!(!tmp.path().join("escape.txt").exists());
    assert!(!tmp.path().join("evil").exists());
    assert!(!tmp.path().join("moved").exists());
    assert!(root.join("sub").is_dir());
}

#[test]
fn password_gate() {
    let (mut backend, mut server, port) = start(FtpServer::new(0).with_password("pw".into()));
    let mut vfs = MemoryVfs::new();
    vfs.write("/f.txt", b"guarded").unwrap();
    let client = thread::spawn(move || {
        // Wrong passwords: 530 twice, then disconnected on the third.
        let mut bad = Client::open(port);
        assert_eq!(bad.cmd("GET /f.txt"), "530 Not authenticated");
        assert_eq!(bad.cmd("PASS nope"), "530 Authentication failed");
        assert_eq!(bad.cmd("PASS pw "), "230 Authenticated");
        drop(bad);

        let mut brute = Client::open(port);
        brute.send("PASS a");
        brute.send("PASS b");
        brute.send("PASS c");
        brute.send("PASS pw");
        let mut rest = Vec::new();
        let (tail, eof) = read_to_eof(&mut brute.s);
        rest.extend_from_slice(&brute.buf);
        rest.extend_from_slice(&tail);
        assert!(eof, "not disconnected after 3 failures");
        let rest = String::from_utf8_lossy(&rest).into_owned();
        assert!(rest.contains("530 Too many failures"), "{rest}");
        assert!(!rest.contains("230"), "{rest}");

        let mut good = Client::open(port);
        assert_eq!(good.cmd("PASS pw"), "230 Authenticated");
        assert_eq!(good.get("/f.txt").unwrap(), b"guarded");
    });
    serve(&mut backend, &mut server, &mut vfs, client);
}

/// A client that sends half a `PUT` line and disconnects leaves no file
/// behind and releases its slot.
#[test]
fn partial_transfer_then_disconnect() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        c.s.write_all(b"PUT /partial.txt abcdef").unwrap();
        c.s.shutdown(std::net::Shutdown::Write).unwrap();
        read_to_eof(&mut c.s)
    });
    let (_, eof) = serve(&mut backend, &mut server, &mut vfs, client);
    assert!(eof, "server kept a half-closed connection open");
    poll_until(&mut backend, &mut server, &mut vfs, |s| {
        s.connection_count() == 0
    });
    assert!(!vfs.exists("/partial.txt"));
}

/// Clients that vanish without QUIT must not hold slots until the idle
/// timeout (there are only 4).
#[test]
fn abrupt_disconnects_release_slots() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    for round in 0..3 {
        let clients: Vec<Client> = (0..4).map(|_| Client::open_nonblocking(port)).collect();
        poll_until(&mut backend, &mut server, &mut vfs, |s| {
            s.connection_count() == 4
        });
        let clients: Vec<Client> = clients
            .into_iter()
            .map(|mut c| {
                let g = c.line();
                assert!(g.starts_with("220"), "round {round}: {g}");
                c
            })
            .collect();
        drop(clients);
        poll_until(&mut backend, &mut server, &mut vfs, |s| {
            s.connection_count() == 0
        });
    }
}

impl Client {
    /// Connect without waiting for the greeting (the server must be
    /// polled to accept first).
    fn open_nonblocking(port: u16) -> Self {
        Self {
            s: connect(port),
            buf: Vec::new(),
        }
    }
}

/// More than 16 commands in one write: all get answered without the
/// client having to send anything else.
#[test]
fn burst_of_commands_is_fully_answered() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        let burst: String = (0..40).map(|i| format!("PUT /b{i} v{i}\n")).collect();
        c.s.write_all(burst.as_bytes()).unwrap();
        for i in 0..40 {
            let line = c.line();
            assert!(line.starts_with("200 written"), "#{i}: {line}");
        }
    });
    serve(&mut backend, &mut server, &mut vfs, client);
    for i in 0..40 {
        assert_eq!(
            vfs.read(&format!("/b{i}")).unwrap(),
            format!("v{i}").as_bytes()
        );
    }
}

/// A multi-megabyte file comes back complete (the socket buffer is much
/// smaller, so the server must queue and resume writes).
#[test]
fn large_get_is_not_truncated() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    let big: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|i| b'a' + (i % 26) as u8)
        .collect();
    vfs.write("/big.txt", &big).unwrap();
    let want = fnv1a(&big);
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        c.s.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let body = c.get("/big.txt").unwrap();
        (body.len(), fnv1a(&body))
    });
    let (len, sum) = serve(&mut backend, &mut server, &mut vfs, client);
    assert_eq!(len, 4 * 1024 * 1024);
    assert_eq!(sum, want);
}

/// Garbage, invalid UTF-8 and overlong lines never panic the server.
#[test]
fn garbage_is_survivable() {
    let (mut backend, mut server, port) = start(FtpServer::new(0));
    let mut vfs = MemoryVfs::new();
    let client = thread::spawn(move || {
        let mut c = Client::open(port);
        let mut junk: Vec<u8> = (0..=255u8).filter(|b| *b != b'\n').collect();
        junk.extend_from_slice(b"\n\xff\xfe\n");
        c.s.write_all(&junk).unwrap();
        c.s.write_all(b"STAT /\n").unwrap();
        assert!(read_until(&mut c.s, &mut c.buf, b"200 directory"));

        // An overlong line is refused and the connection closed, rather
        // than resyncing mid-line and running its tail as a command.
        let mut c = Client::open(port);
        c.s.write_all(&vec![b'Z'; 5000]).unwrap();
        let _ = c.s.write_all(b"\nSTAT /\n");
        let (rest, eof) = read_to_eof(&mut c.s);
        c.buf.extend_from_slice(&rest);
        assert!(eof);
        let out = String::from_utf8_lossy(&c.buf).into_owned();
        assert!(!out.contains("400 unknown command"), "{out}");
        assert!(!out.contains("200 directory"), "{out}");

        // The server is still fine for the next client.
        let mut c = Client::open(port);
        assert!(c.cmd("STAT /").starts_with("200 directory"));
    });
    serve(&mut backend, &mut server, &mut vfs, client);
}
