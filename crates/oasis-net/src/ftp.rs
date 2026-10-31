//! FTP-like file transfer server over a [`NetworkBackend`], serving a
//! [`Vfs`] (feature `vfs`).
//!
//! The protocol is line-based (`\n`-terminated requests, at most 1024
//! bytes; paths cannot contain spaces):
//!
//! - `PASS <password>` -- authenticate (when the server has a password)
//! - `LIST [path]` -- list a directory: `200 ` then one `<d|f> <size>
//!   <name>` line per entry (`200 (empty)` for none)
//! - `GET <path>` -- `200 <n> bytes\n` followed by exactly `n` raw bytes
//! - `PUT <path> <text>` -- write the rest of the line (trimmed, UTF-8) as
//!   the file content; one-line text payloads only
//! - `MKDIR <path>`, `DELETE <path>`, `STAT <path>`
//! - `RENAME <from> <to>` -- rename/move a file or directory
//! - `QUIT` -- close the connection
//!
//! Errors are `400` (bad request), `500` (VFS error) or `530` (auth).
//! Paths are resolved inside the served directory ([`FtpServer::with_root`];
//! the whole VFS by default) and then by the [`Vfs`], which confines them
//! (see `oasis_vfs::normalize_path`). The password travels in clear text:
//! with one the server listens on all interfaces, so use it on trusted
//! networks only. Failed logins are throttled server-wide.
//!
//! The `ftp`, `push` and `pull` terminal commands live in
//! `oasis-core::transfer`; the desktop shell drives [`FtpServer::poll`].

use std::time::Instant;

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::Result;
use oasis_types::util::ct_eq;
use oasis_vfs::{SubtreeVfs, Vfs};

use crate::auth::{AUTH_TIMEOUT, AuthThrottle};

/// Default FTP server port.
pub const DEFAULT_FTP_PORT: u16 = 2121;

/// Process an FTP protocol request line against the VFS.
///
/// Returns the response bytes to send back to the client (raw file bytes
/// for `GET`, so binary content arrives intact).
pub(crate) fn process_ftp_request(line: &str, vfs: &mut dyn Vfs) -> Vec<u8> {
    match line.trim().split_once(' ') {
        Some((cmd, rest)) if cmd.eq_ignore_ascii_case("GET") && !rest.trim().is_empty() => {
            match vfs.read(rest.trim()) {
                Ok(data) => {
                    let mut resp = format!("200 {} bytes\n", data.len()).into_bytes();
                    resp.extend_from_slice(&data);
                    resp
                },
                Err(e) => format!("500 {e}\n").into_bytes(),
            }
        },
        _ => process_text_request(line, vfs).into_bytes(),
    }
}

/// Every request except a well-formed `GET`.
fn process_text_request(line: &str, vfs: &mut dyn Vfs) -> String {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("").to_uppercase();

    match cmd.as_str() {
        "LIST" => {
            let path = parts.get(1).copied().unwrap_or("/");
            match vfs.readdir(path) {
                Ok(entries) => {
                    if entries.is_empty() {
                        return "200 (empty)\n".to_string();
                    }
                    let mut resp = String::from("200 ");
                    for entry in &entries {
                        let kind = match entry.kind {
                            oasis_vfs::EntryKind::Directory => "d",
                            oasis_vfs::EntryKind::File => "f",
                        };
                        // A newline in a (host-side) file name would forge
                        // listing lines: show control characters as `?`.
                        let name: String = entry
                            .name
                            .chars()
                            .map(|c| if c.is_control() { '?' } else { c })
                            .collect();
                        resp.push_str(&format!("{kind} {} {name}\n", entry.size));
                    }
                    resp
                },
                Err(e) => format!("500 {e}\n"),
            }
        },
        "GET" => {
            let path = parts.get(1).copied().unwrap_or("");
            if path.is_empty() {
                return "400 missing path\n".to_string();
            }
            // A path is present: `process_ftp_request` serves it.
            format!("400 bad request: {path}\n")
        },
        "PUT" => {
            let path = parts.get(1).copied().unwrap_or("");
            let content = parts.get(2).copied().unwrap_or("");
            if path.is_empty() {
                return "400 missing path\n".to_string();
            }
            match vfs.write(path, content.as_bytes()) {
                Ok(()) => format!("200 written {} bytes to {path}\n", content.len()),
                Err(e) => format!("500 {e}\n"),
            }
        },
        "MKDIR" => {
            let path = parts.get(1).copied().unwrap_or("");
            if path.is_empty() {
                return "400 missing path\n".to_string();
            }
            match vfs.mkdir(path) {
                Ok(()) => format!("200 created {path}\n"),
                Err(e) => format!("500 {e}\n"),
            }
        },
        "DELETE" => {
            let path = parts.get(1).copied().unwrap_or("");
            if path.is_empty() {
                return "400 missing path\n".to_string();
            }
            match vfs.remove(path) {
                Ok(()) => format!("200 deleted {path}\n"),
                Err(e) => format!("500 {e}\n"),
            }
        },
        "RENAME" => {
            let from = parts.get(1).copied().unwrap_or("");
            let to = parts.get(2).copied().unwrap_or("");
            if from.is_empty() || to.is_empty() {
                return "400 missing paths (usage: RENAME <from> <to>)\n".to_string();
            }
            match vfs.rename(from, to) {
                Ok(()) => format!("200 renamed {from} -> {to}\n"),
                Err(e) => format!("500 {e}\n"),
            }
        },
        "STAT" => {
            let path = parts.get(1).copied().unwrap_or("");
            if path.is_empty() {
                return "400 missing path\n".to_string();
            }
            match vfs.stat(path) {
                Ok(meta) => {
                    let kind = match meta.kind {
                        oasis_vfs::EntryKind::Directory => "directory",
                        oasis_vfs::EntryKind::File => "file",
                    };
                    format!("200 {kind} {} bytes\n", meta.size)
                },
                Err(e) => format!("500 {e}\n"),
            }
        },
        "QUIT" => "200 goodbye\n".to_string(),
        "" => "400 empty command\n".to_string(),
        _ => format!("400 unknown command: {cmd}\n"),
    }
}

// ---------------------------------------------------------------------------
// FTP Server
// ---------------------------------------------------------------------------

/// Maximum simultaneous FTP connections.
const MAX_FTP_CONNECTIONS: usize = 4;

/// Maximum bytes in a single FTP input line.
const MAX_FTP_LINE_LEN: usize = 1024;

/// Maximum commands to process per connection per poll cycle.
const MAX_CMDS_PER_POLL: usize = 16;

/// Maximum bytes read from one connection in a single poll.
const MAX_READ_PER_POLL: usize = 64 * 1024;

/// While more than this much output is queued for a connection (the peer
/// is not reading), no further commands are processed on it, so memory per
/// connection stays bounded at roughly this plus one response.
const MAX_PENDING_OUTPUT: usize = 1024 * 1024;

/// How long a closing connection may take to drain its queued output.
const CLOSE_DRAIN_TIMEOUT_SECS: u64 = 2;

/// Idle connection timeout in seconds.
const FTP_IDLE_TIMEOUT_SECS: u64 = 300;

/// Maximum failed authentication attempts on one connection before it is
/// closed (the server-wide [`AuthThrottle`] bounds reconnect-and-retry).
const MAX_CONN_AUTH_FAILURES: u8 = 3;

/// A single FTP client connection.
struct FtpConnection {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    /// Output the non-blocking socket has not accepted yet; bytes before
    /// `write_pos` are already sent.
    write_buf: Vec<u8>,
    write_pos: usize,
    last_activity: Instant,
    /// When the connection was accepted (for the auth timeout).
    accepted_at: Instant,
    /// Whether this connection has been authenticated.
    authenticated: bool,
    /// Number of failed authentication attempts.
    failed_attempts: u8,
    /// Peer closed its side (read returned 0).
    eof: bool,
    /// Set when the connection should close once its output has drained.
    closing_since: Option<Instant>,
    /// Hard I/O failure: drop now.
    dead: bool,
}

impl FtpConnection {
    fn new(stream: Box<dyn NetworkStream>, authenticated: bool) -> Self {
        Self {
            stream,
            read_buf: Vec::with_capacity(256),
            write_buf: Vec::new(),
            write_pos: 0,
            last_activity: Instant::now(),
            accepted_at: Instant::now(),
            authenticated,
            failed_attempts: 0,
            eof: false,
            closing_since: None,
            dead: false,
        }
    }

    fn pending(&self) -> usize {
        self.write_buf.len() - self.write_pos
    }

    fn queue(&mut self, data: &[u8]) {
        if !self.dead {
            self.write_buf.extend_from_slice(data);
        }
    }

    /// Write as much queued output as the non-blocking socket accepts.
    fn flush(&mut self) {
        while !self.dead && self.write_pos < self.write_buf.len() {
            match self.stream.write(&self.write_buf[self.write_pos..]) {
                Ok(0) => break,
                Ok(n) => self.write_pos += n,
                Err(ref e) if e.is_would_block() => break,
                Err(_) => self.dead = true,
            }
        }
        if self.write_pos == self.write_buf.len() {
            self.write_buf.clear();
            self.write_pos = 0;
        } else if self.write_pos >= MAX_PENDING_OUTPUT {
            self.write_buf.drain(..self.write_pos);
            self.write_pos = 0;
        }
        let _ = self.stream.flush();
    }

    fn close_after_flush(&mut self) {
        if self.closing_since.is_none() {
            self.closing_since = Some(Instant::now());
        }
    }

    fn finished(&self) -> bool {
        self.dead
            || self.closing_since.is_some_and(|since| {
                self.pending() == 0 || since.elapsed().as_secs() >= CLOSE_DRAIN_TIMEOUT_SECS
            })
    }

    /// Read until the socket would block, EOF, or the per-poll cap.
    fn read_available(&mut self) {
        let mut buf = [0u8; 4096];
        let mut total = 0usize;
        while total < MAX_READ_PER_POLL && !self.eof {
            match self.stream.read(&mut buf) {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    total += n;
                    self.last_activity = Instant::now();
                    self.read_buf.extend_from_slice(&buf[..n]);
                },
                Err(ref e) if e.is_would_block() => break,
                Err(_) => {
                    self.dead = true;
                    break;
                },
            }
        }
    }

    /// Process up to [`MAX_CMDS_PER_POLL`] buffered lines.
    fn process_lines(
        &mut self,
        password: Option<&str>,
        throttle: &mut AuthThrottle,
        vfs: &mut dyn Vfs,
    ) {
        let mut cmds_processed = 0usize;
        while cmds_processed < MAX_CMDS_PER_POLL && self.pending() < MAX_PENDING_OUTPUT {
            let Some(pos) = self.read_buf.iter().position(|&b| b == b'\n') else {
                break;
            };
            if pos > MAX_FTP_LINE_LEN {
                self.overlong();
                return;
            }
            cmds_processed += 1;
            let line_bytes: Vec<u8> = self.read_buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

            if line.is_empty() {
                continue;
            }

            // Check for QUIT (always allowed). Nothing after it runs.
            if line.eq_ignore_ascii_case("QUIT") {
                self.queue(b"200 goodbye\r\n");
                self.read_buf.clear();
                self.close_after_flush();
                return;
            }

            // Authentication gate.
            if !self.authenticated
                && let Some(expected) = password
            {
                let supplied = line
                    .get(..5)
                    .filter(|p| p.eq_ignore_ascii_case("PASS "))
                    .map(|_| line[5..].trim());
                match supplied {
                    Some(pass) if ct_eq(pass.as_bytes(), expected.as_bytes()) => {
                        self.authenticated = true;
                        self.queue(b"230 Authenticated\r\n");
                    },
                    Some(_) => {
                        self.failed_attempts += 1;
                        throttle.record_failure("FTP server");
                        if self.failed_attempts >= MAX_CONN_AUTH_FAILURES || throttle.is_locked() {
                            self.queue(b"530 Too many failures\r\n");
                            // No further guesses on this connection.
                            self.read_buf.clear();
                            self.close_after_flush();
                            return;
                        }
                        self.queue(b"530 Authentication failed\r\n");
                    },
                    None => self.queue(b"530 Not authenticated\r\n"),
                }
                continue;
            }

            // Process command against VFS.
            let response = process_ftp_request(&line, vfs);
            self.queue(&response);
        }

        // Guard against overlong (unterminated) lines.
        if self.read_buf.len() > MAX_FTP_LINE_LEN
            && !self.read_buf[..=MAX_FTP_LINE_LEN].contains(&b'\n')
        {
            self.overlong();
        }
    }

    /// The peer sent a line over [`MAX_FTP_LINE_LEN`]: report and hang up
    /// (resyncing mid-line would run the tail as a command).
    fn overlong(&mut self) {
        self.read_buf.clear();
        self.queue(b"500 line too long\r\n");
        self.close_after_flush();
    }
}

/// Poll-based FTP file server.
///
/// Accepts TCP connections and processes FTP protocol commands against
/// the VFS. Designed for non-blocking polling from the main loop,
/// following the same pattern as `RemoteListener`.
pub struct FtpServer {
    port: u16,
    connections: Vec<FtpConnection>,
    listening: bool,
    /// Optional password for authentication. When `None`, all
    /// connections are immediately authenticated.
    password: Option<String>,
    /// Server-wide throttle for failed `PASS` attempts.
    auth_failures: AuthThrottle,
    /// Directory of the VFS that clients see as `/` (normalized).
    root: String,
}

impl FtpServer {
    /// Create a new FTP server on the given port, serving the whole VFS
    /// (narrow it with [`with_root`](Self::with_root)).
    pub fn new(port: u16) -> Self {
        Self {
            port,
            connections: Vec::new(),
            listening: false,
            password: None,
            auth_failures: AuthThrottle::new(),
            root: "/".to_string(),
        }
    }

    /// Set a password for FTP authentication (builder pattern).
    ///
    /// An empty password means no authentication, exactly like not calling
    /// this: [`start`](Self::start) then binds loopback only. (It used to
    /// demand a `PASS` that could never match, locking every client out.)
    pub fn with_password(mut self, password: String) -> Self {
        self.password = (!password.is_empty()).then_some(password);
        self
    }

    /// Serve only the directory `root` of the VFS: clients see it as `/`
    /// and cannot name anything outside it (a [`SubtreeVfs`] view). Hosts
    /// whose VFS holds system state (`/system`, `/var` IPC files) should
    /// serve a user directory such as `/home`.
    ///
    /// # Errors
    ///
    /// When `root` is not a valid VFS path (`..` above `/`, NUL byte).
    pub fn with_root(mut self, root: &str) -> Result<Self> {
        self.root = oasis_vfs::normalize_path(root)?.into_owned();
        Ok(self)
    }

    /// The served directory (`/` = the whole VFS).
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Start listening on the configured port.
    ///
    /// Without a (non-empty) password the server grants read/write access to
    /// anyone who can connect, so it binds loopback only
    /// ([`NetworkBackend::listen_loopback`]); with a password it listens on
    /// all interfaces.
    pub fn start(&mut self, backend: &mut dyn NetworkBackend) -> Result<()> {
        if self.password.is_some() {
            backend.listen(self.port)?;
        } else {
            backend.listen_loopback(self.port)?;
        }
        self.listening = true;
        Ok(())
    }

    /// The configured port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the server is active.
    pub fn is_listening(&self) -> bool {
        self.listening
    }

    /// Number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Poll for new connections and process FTP commands.
    ///
    /// Call from the main loop each frame. Commands are executed
    /// immediately against the provided VFS. Responses are queued and
    /// written as the socket accepts them, so large `GET`s arrive intact.
    pub fn poll(&mut self, backend: &mut dyn NetworkBackend, vfs: &mut dyn Vfs) -> Result<()> {
        if !self.listening {
            return Ok(());
        }

        let idle_timeout = std::time::Duration::from_secs(FTP_IDLE_TIMEOUT_SECS);

        // Accept new connections.
        let requires_auth = self.password.is_some();
        if self.connections.len() < MAX_FTP_CONNECTIONS {
            match backend.accept() {
                Ok(Some(mut stream)) if requires_auth && self.auth_failures.is_locked() => {
                    let _ = stream.write(b"421 Too many failed logins, try later\r\n");
                    let _ = stream.close();
                },
                Ok(Some(stream)) => {
                    let mut conn = FtpConnection::new(stream, !requires_auth);
                    let greeting = if requires_auth {
                        &b"220 OASIS FTP server ready (auth required)\r\n"[..]
                    } else {
                        &b"220 OASIS FTP server ready\r\n"[..]
                    };
                    conn.queue(greeting);
                    conn.flush();
                    self.connections.push(conn);
                },
                Ok(None) => {},
                Err(e) => log::warn!("FTP accept error: {e}"),
            }
        }

        let password = self.password.as_deref();
        // Clients see only the served subtree.
        let mut view = SubtreeVfs::new(vfs, &self.root)?;
        for conn in &mut self.connections {
            conn.flush();
            if conn.dead || conn.closing_since.is_some() {
                continue;
            }

            // Unauthenticated peers get a short deadline, so idle sockets
            // cannot hold every slot for the whole idle timeout.
            if !conn.authenticated && conn.accepted_at.elapsed() > AUTH_TIMEOUT {
                conn.queue(b"530 Authentication timeout\r\n");
                conn.close_after_flush();
                conn.flush();
                continue;
            }

            // Check idle timeout.
            if conn.last_activity.elapsed() > idle_timeout {
                conn.queue(b"421 Idle timeout\r\n");
                conn.close_after_flush();
                conn.flush();
                continue;
            }

            // Backpressure: leave input unread while the peer is not
            // draining the responses already queued for it, or while lines
            // from an earlier burst are still waiting (bounds `read_buf`).
            if conn.pending() < MAX_PENDING_OUTPUT && conn.read_buf.len() <= MAX_FTP_LINE_LEN {
                conn.read_available();
            }
            if conn.dead {
                continue;
            }
            // Buffered lines are processed every poll, not only when new
            // bytes arrive (a burst beyond MAX_CMDS_PER_POLL would stall).
            conn.process_lines(password, &mut self.auth_failures, &mut view);

            // Peer closed: once every complete line has been answered, drop
            // it (an unterminated trailing line is a partial transfer and is
            // discarded).
            if conn.eof && !conn.read_buf.contains(&b'\n') {
                conn.read_buf.clear();
                conn.close_after_flush();
            }
            conn.flush();
        }

        // Remove finished connections.
        self.connections.retain_mut(|conn| {
            if conn.finished() {
                let _ = conn.stream.close();
                false
            } else {
                true
            }
        });

        Ok(())
    }

    /// Shut down all connections and stop listening.
    pub fn stop(&mut self) {
        for conn in &mut self.connections {
            conn.queue(b"421 Server shutting down\r\n");
            conn.flush();
            let _ = conn.stream.close();
        }
        self.connections.clear();
        self.listening = false;
    }
}

#[cfg(test)]
mod tests;
