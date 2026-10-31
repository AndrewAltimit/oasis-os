/// `process_ftp_request` as text.
fn req(line: &str, vfs: &mut dyn Vfs) -> String {
    String::from_utf8(process_ftp_request(line, vfs)).unwrap()
}

use super::*;
use oasis_types::error::OasisError;
use oasis_vfs::MemoryVfs;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// -- FTP protocol tests --

#[test]
fn ftp_list_root() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/tmp").unwrap();
    let resp = req("LIST /", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(resp.contains("home"));
    assert!(resp.contains("tmp"));
}

#[test]
fn ftp_list_empty() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/empty").unwrap();
    let resp = req("LIST /empty", &mut vfs);
    assert!(resp.contains("200"));
    assert!(resp.contains("empty"));
}

#[test]
fn ftp_get_file() {
    let mut vfs = MemoryVfs::new();
    vfs.write("/test.txt", b"hello").unwrap();
    let resp = req("GET /test.txt", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(resp.contains("5 bytes"));
    assert!(resp.contains("hello"));
}

#[test]
fn ftp_get_missing() {
    let mut vfs = MemoryVfs::new();
    let resp = req("GET /nope.txt", &mut vfs);
    assert!(resp.starts_with("500"));
}

#[test]
fn ftp_put_file() {
    let mut vfs = MemoryVfs::new();
    let resp = req("PUT /new.txt hello world", &mut vfs);
    assert!(resp.starts_with("200"));
    let data = vfs.read("/new.txt").unwrap();
    assert_eq!(data, b"hello world");
}

#[test]
fn ftp_mkdir() {
    let mut vfs = MemoryVfs::new();
    let resp = req("MKDIR /newdir", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(vfs.exists("/newdir"));
}

#[test]
fn ftp_delete() {
    let mut vfs = MemoryVfs::new();
    vfs.write("/deleteme.txt", b"gone").unwrap();
    let resp = req("DELETE /deleteme.txt", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(!vfs.exists("/deleteme.txt"));
}

#[test]
fn ftp_stat_file() {
    let mut vfs = MemoryVfs::new();
    vfs.write("/info.txt", b"data").unwrap();
    let resp = req("STAT /info.txt", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(resp.contains("file"));
    assert!(resp.contains("4 bytes"));
}

#[test]
fn ftp_stat_dir() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/mydir").unwrap();
    let resp = req("STAT /mydir", &mut vfs);
    assert!(resp.contains("directory"));
}

#[test]
fn ftp_rename() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/src").unwrap();
    vfs.mkdir("/dst").unwrap();
    vfs.write("/src/file.txt", b"hello").unwrap();
    let resp = req("RENAME /src/file.txt /dst/moved.txt", &mut vfs);
    assert!(resp.starts_with("200"));
    assert!(!vfs.exists("/src/file.txt"));
    assert_eq!(vfs.read("/dst/moved.txt").unwrap(), b"hello");
}

#[test]
fn ftp_rename_missing_source() {
    let mut vfs = MemoryVfs::new();
    let resp = req("RENAME /nope.txt /dest.txt", &mut vfs);
    assert!(resp.starts_with("500"));
}

#[test]
fn ftp_rename_missing_args() {
    let mut vfs = MemoryVfs::new();
    let resp = req("RENAME", &mut vfs);
    assert!(resp.starts_with("400"));
    let resp = req("RENAME /only_one", &mut vfs);
    assert!(resp.starts_with("400"));
}

#[test]
fn ftp_quit() {
    let mut vfs = MemoryVfs::new();
    let resp = req("QUIT", &mut vfs);
    assert!(resp.contains("goodbye"));
}

#[test]
fn ftp_unknown_command() {
    let mut vfs = MemoryVfs::new();
    let resp = req("BADCMD", &mut vfs);
    assert!(resp.starts_with("400"));
}

#[test]
fn ftp_empty_command() {
    let mut vfs = MemoryVfs::new();
    let resp = req("", &mut vfs);
    assert!(resp.starts_with("400"));
}

#[test]
fn ftp_get_missing_path() {
    let mut vfs = MemoryVfs::new();
    let resp = req("GET", &mut vfs);
    assert!(resp.starts_with("400"));
}

// -- FTP authentication tests --

/// Shared buffer that records all writes from a mock stream.
type WriteBuf = Arc<Mutex<Vec<u8>>>;

/// Mock network stream backed by an input queue and an output buffer.
struct MockStream {
    input: VecDeque<u8>,
    output: WriteBuf,
    closed: bool,
}

impl MockStream {
    fn new(input: &[u8], output: WriteBuf) -> Self {
        Self {
            input: VecDeque::from(input.to_vec()),
            output,
            closed: false,
        }
    }
}

impl oasis_types::backend::NetworkStream for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> oasis_types::error::Result<usize> {
        if self.input.is_empty() {
            return Err(OasisError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no data",
            )));
        }
        let n = buf.len().min(self.input.len());
        for b in buf.iter_mut().take(n) {
            *b = self.input.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> oasis_types::error::Result<usize> {
        self.output.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn close(&mut self) -> oasis_types::error::Result<()> {
        self.closed = true;
        Ok(())
    }
}

/// Mock network backend that yields pre-built streams.
struct MockBackend {
    pending: VecDeque<Box<dyn oasis_types::backend::NetworkStream>>,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }

    fn add_stream(&mut self, stream: Box<dyn oasis_types::backend::NetworkStream>) {
        self.pending.push_back(stream);
    }
}

impl oasis_types::backend::NetworkBackend for MockBackend {
    fn listen(&mut self, _port: u16) -> oasis_types::error::Result<()> {
        Ok(())
    }

    fn accept(
        &mut self,
    ) -> oasis_types::error::Result<Option<Box<dyn oasis_types::backend::NetworkStream>>> {
        Ok(self.pending.pop_front())
    }

    fn connect(
        &mut self,
        _address: &str,
        _port: u16,
    ) -> oasis_types::error::Result<Box<dyn oasis_types::backend::NetworkStream>> {
        Err(OasisError::Backend("mock: no outbound".into()))
    }
}

/// Helper: collect all bytes written to the shared output buffer.
fn read_output(output: &WriteBuf) -> String {
    String::from_utf8_lossy(&output.lock().unwrap()).into_owned()
}

/// Yields its input, then fails every read with `kind`.
struct FailingStream {
    input: VecDeque<u8>,
    kind: std::io::ErrorKind,
}

impl oasis_types::backend::NetworkStream for FailingStream {
    fn read(&mut self, buf: &mut [u8]) -> oasis_types::error::Result<usize> {
        if self.input.is_empty() {
            return Err(OasisError::Io(std::io::Error::from(self.kind)));
        }
        let n = buf.len().min(self.input.len());
        for b in buf.iter_mut().take(n) {
            *b = self.input.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> oasis_types::error::Result<usize> {
        Ok(data.len())
    }

    fn close(&mut self) -> oasis_types::error::Result<()> {
        Ok(())
    }
}

fn poll_with_read_error(kind: std::io::ErrorKind) -> usize {
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(FailingStream {
        input: VecDeque::from(b"LIST /\n".to_vec()),
        kind,
    }));
    let mut server = FtpServer::new(2121);
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();
    server.poll(&mut backend, &mut vfs).unwrap();
    server.connection_count()
}

#[test]
fn ftp_timed_out_read_drops_connection() {
    // ETIMEDOUT on the non-blocking socket = the kernel gave up on the
    // peer. Retrying it like WouldBlock would keep a dead connection
    // (and its slot) until the 5 minute idle timeout.
    assert_eq!(poll_with_read_error(std::io::ErrorKind::TimedOut), 0);
}

#[test]
fn ftp_would_block_read_keeps_connection() {
    assert_eq!(poll_with_read_error(std::io::ErrorKind::WouldBlock), 1);
}

#[test]
fn ftp_auth_correct_password() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"PASS secret123\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("secret123".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("220 OASIS FTP server ready (auth required)"),
        "should get auth-required greeting"
    );
    assert!(written.contains("230 Authenticated"), "should authenticate");
    assert_eq!(server.connection_count(), 1);
}

#[test]
fn ftp_auth_wrong_password() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"PASS wrong\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("secret123".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("530 Authentication failed"),
        "should reject wrong password"
    );
    assert_eq!(server.connection_count(), 1, "should stay connected");
}

#[test]
fn ftp_auth_too_many_failures() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"PASS bad1\nPASS bad2\nPASS bad3\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("correct".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("530 Too many failures"),
        "should disconnect after 3 failures"
    );
    assert_eq!(server.connection_count(), 0, "connection should be removed");
}

#[test]
fn ftp_failed_logins_are_throttled_across_connections() {
    // Reconnecting used to reset the 3-strikes limit: unlimited guesses.
    let mut backend = MockBackend::new();
    let mut server = FtpServer::new(2121).with_password("correct".to_string());
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    for _ in 0..3 {
        let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
        backend.add_stream(Box::new(MockStream::new(
            b"PASS a\nPASS b\n",
            Arc::clone(&output),
        )));
        server.poll(&mut backend, &mut vfs).unwrap();
    }
    assert!(server.auth_failures.is_locked());
    // Even the right password is refused while locked out.
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    backend.add_stream(Box::new(MockStream::new(
        b"PASS correct\n",
        Arc::clone(&output),
    )));
    server.poll(&mut backend, &mut vfs).unwrap();
    let written = read_output(&output);
    assert!(written.starts_with("421 "), "{written:?}");
    assert!(!written.contains("230"), "{written:?}");
}

#[test]
fn ftp_unauthenticated_connection_times_out() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(MockStream::new(b"", Arc::clone(&output))));
    let mut server = FtpServer::new(2121).with_password("pw".to_string());
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();
    assert_eq!(server.connection_count(), 1);
    server.connections[0].accepted_at = Instant::now() - AUTH_TIMEOUT * 2;
    server.poll(&mut backend, &mut vfs).unwrap();
    assert_eq!(server.connection_count(), 0);
    assert!(read_output(&output).contains("530 Authentication timeout"));
}

#[test]
fn ftp_empty_line_flood_keeps_read_buffer_bounded() {
    // Empty lines count toward the per-poll cap but queue no output, so
    // the buffer used to grow by 64 KiB per poll without bound.
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let flood = vec![b'\n'; 1024 * 1024];
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(MockStream::new(&flood, Arc::clone(&output))));
    let mut server = FtpServer::new(2121).with_password("pw".to_string());
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    for _ in 0..50 {
        server.poll(&mut backend, &mut vfs).unwrap();
        let buffered = server.connections[0].read_buf.len();
        assert!(
            buffered <= MAX_FTP_LINE_LEN + MAX_READ_PER_POLL,
            "read_buf grew to {buffered}"
        );
    }
}

#[test]
fn ftp_get_returns_binary_content_verbatim() {
    let mut vfs = MemoryVfs::new();
    let data = [0u8, 0xff, 0xfe, b'\n', 0x80, b'x'];
    vfs.write("/bin", &data).unwrap();
    let resp = process_ftp_request("GET /bin", &mut vfs);
    let head = b"200 6 bytes\n";
    assert_eq!(&resp[..head.len()], head);
    assert_eq!(&resp[head.len()..], &data);
}

#[test]
fn ftp_command_before_auth_after_lockout_is_not_served() {
    let mut server = FtpServer::new(2121).with_password("pw".to_string());
    for _ in 0..crate::auth::MAX_AUTH_FAILURES {
        server.auth_failures.record_failure("test");
    }
    let mut backend = MockBackend::new();
    server.start(&mut backend).unwrap();
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    backend.add_stream(Box::new(MockStream::new(b"LIST /\n", Arc::clone(&output))));
    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();
    assert_eq!(server.connection_count(), 0);
    assert!(!read_output(&output).contains("200"));
}

#[test]
fn ftp_auth_command_before_auth() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"LIST /\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("secret".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("530 Not authenticated"),
        "should reject commands before auth"
    );
}

#[test]
fn ftp_auth_then_command() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"PASS mypass\nLIST /\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("mypass".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("230 Authenticated"),
        "should authenticate first"
    );
    assert!(written.contains("200"), "LIST should succeed after auth");
}

#[test]
fn ftp_no_password_immediately_authenticated() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"LIST /\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121);
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("220 OASIS FTP server ready\r\n"),
        "should get standard greeting"
    );
    assert!(
        !written.contains("auth required"),
        "should not mention auth"
    );
    assert!(written.contains("200"), "LIST should work without auth");
}

#[test]
fn ftp_root_confines_clients_to_the_served_directory() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(
        b"LIST /\nGET /../system/secret\nGET /system/secret\nPUT /up.txt hi\n\
          DELETE /\nGET /notes.txt\n",
        Arc::clone(&output),
    );
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_root("/home/").unwrap();
    assert_eq!(server.root(), "/home");
    assert!(FtpServer::new(1).with_root("/../etc").is_err());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.write("/home/notes.txt", b"mine").unwrap();
    vfs.mkdir("/system").unwrap();
    vfs.write("/system/secret", b"s3cret").unwrap();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(written.contains("f 4 notes.txt"), "{written}");
    assert!(written.contains("path traversal not allowed"), "{written}");
    assert!(!written.contains("s3cret"), "{written}");
    assert!(written.contains("200 4 bytes\nmine"), "{written}");
    assert!(written.contains("cannot remove root"), "{written}");
    assert_eq!(vfs.read("/home/up.txt").unwrap(), b"hi");
    assert!(!vfs.exists("/up.txt"));
}

#[test]
fn ftp_auth_quit_before_auth() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let stream = MockStream::new(b"QUIT\n", Arc::clone(&output));

    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));

    let mut server = FtpServer::new(2121).with_password("secret".to_string());
    server.start(&mut backend).unwrap();

    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();

    let written = read_output(&output);
    assert!(
        written.contains("200 goodbye"),
        "QUIT should always be allowed"
    );
    assert_eq!(server.connection_count(), 0);
}

/// Accepts at most `budget` more bytes; further writes would block.
struct TrickleStream {
    input: VecDeque<u8>,
    output: WriteBuf,
    budget: Arc<Mutex<usize>>,
}

impl oasis_types::backend::NetworkStream for TrickleStream {
    fn read(&mut self, buf: &mut [u8]) -> oasis_types::error::Result<usize> {
        if self.input.is_empty() {
            return Err(OasisError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no data",
            )));
        }
        let n = buf.len().min(self.input.len());
        for b in buf.iter_mut().take(n) {
            *b = self.input.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> oasis_types::error::Result<usize> {
        let mut budget = self.budget.lock().unwrap();
        if *budget == 0 {
            return Err(OasisError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "full",
            )));
        }
        let n = data.len().min(*budget);
        *budget -= n;
        self.output.lock().unwrap().extend_from_slice(&data[..n]);
        Ok(n)
    }

    fn close(&mut self) -> oasis_types::error::Result<()> {
        Ok(())
    }
}

#[test]
fn ftp_large_get_survives_partial_writes() {
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let budget = Arc::new(Mutex::new(16usize));
    let stream = TrickleStream {
        input: VecDeque::from(b"GET /big.txt\n".to_vec()),
        output: Arc::clone(&output),
        budget: Arc::clone(&budget),
    };
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));
    let mut server = FtpServer::new(2121);
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    let big = "q".repeat(50_000);
    vfs.write("/big.txt", big.as_bytes()).unwrap();

    for _ in 0..100 {
        *budget.lock().unwrap() = 1000;
        server.poll(&mut backend, &mut vfs).unwrap();
    }
    let out = read_output(&output);
    assert_eq!(
        out,
        format!("220 OASIS FTP server ready\r\n200 50000 bytes\n{big}")
    );
}

#[test]
fn ftp_buffered_commands_run_without_new_data() {
    // 40 commands arrive at once; only 16 run per poll, and the rest
    // must still run on later polls although no new bytes arrive.
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let input: String = (0..40).map(|i| format!("PUT /f{i} x\n")).collect();
    let stream = MockStream::new(input.as_bytes(), Arc::clone(&output));
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(stream));
    let mut server = FtpServer::new(2121);
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    for _ in 0..4 {
        server.poll(&mut backend, &mut vfs).unwrap();
    }
    assert_eq!(read_output(&output).matches("200 written").count(), 40);
}

#[test]
fn ftp_eof_releases_connection() {
    struct EofStream;
    impl oasis_types::backend::NetworkStream for EofStream {
        fn read(&mut self, _buf: &mut [u8]) -> oasis_types::error::Result<usize> {
            Ok(0)
        }
        fn write(&mut self, data: &[u8]) -> oasis_types::error::Result<usize> {
            Ok(data.len())
        }
        fn close(&mut self) -> oasis_types::error::Result<()> {
            Ok(())
        }
    }
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(EofStream));
    let mut server = FtpServer::new(2121);
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    server.poll(&mut backend, &mut vfs).unwrap();
    assert_eq!(server.connection_count(), 0);
}

#[test]
fn ftp_empty_password_means_no_auth_on_loopback() {
    // Regression: `with_password("")` bound loopback (no password) yet
    // still demanded a `PASS` that could never match.
    let output: WriteBuf = Arc::new(Mutex::new(Vec::new()));
    let mut backend = MockBackend::new();
    backend.add_stream(Box::new(MockStream::new(b"LIST /\n", Arc::clone(&output))));
    let mut server = FtpServer::new(2121).with_password(String::new());
    server.start(&mut backend).unwrap();
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    server.poll(&mut backend, &mut vfs).unwrap();
    let written = read_output(&output);
    assert!(written.contains("d 0 home"), "{written}");
    assert!(!written.contains("530"), "{written}");
}

#[test]
fn ftp_list_escapes_control_characters_in_names() {
    let mut vfs = MemoryVfs::new();
    vfs.write("/a\nf 0 forged", b"x").unwrap();
    assert_eq!(req("LIST /", &mut vfs), "200 f 1 a?f 0 forged\n");
}

#[test]
fn ftp_password_compare_is_exact() {
    assert!(ct_eq(b"secret", b"secret"));
    assert!(!ct_eq(b"secret", b"secreT"));
    assert!(!ct_eq(b"secret", b"secret1"));
    assert!(!ct_eq(b"", b"x"));
}
