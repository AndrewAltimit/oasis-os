//! File transfer services -- FTP-like server and push/pull commands.
//!
//! Provides a minimal file transfer protocol over TCP using the
//! `NetworkBackend` trait. The protocol is line-based:
//!
//! - `LIST <path>` -- list directory contents
//! - `GET <path>`  -- retrieve file (response: size + data)
//! - `PUT <path> <size>` -- upload file
//! - `RENAME <from> <to>` -- rename/move file or directory
//! - `QUIT` -- close connection
//!
//! Also provides terminal commands: `ftp start/stop`, `push`, `pull`.

use std::time::Instant;

use oasis_types::backend::{NetworkBackend, NetworkStream};

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, Environment};
use crate::vfs::Vfs;

/// Default FTP server port.
pub const DEFAULT_FTP_PORT: u16 = 2121;

/// VFS path for FTP configuration.
pub const FTP_STATUS_PATH: &str = "/var/ftp/status";
pub const FTP_REQUEST_PATH: &str = "/var/ftp/request";

/// Process an FTP protocol request line against the VFS.
///
/// Returns a response string to send back to the client.
pub fn process_ftp_request(line: &str, vfs: &mut dyn Vfs) -> String {
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
                            crate::vfs::EntryKind::Directory => "d",
                            crate::vfs::EntryKind::File => "f",
                        };
                        resp.push_str(&format!("{kind} {} {}\n", entry.size, entry.name));
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
            match vfs.read(path) {
                Ok(data) => {
                    // For text mode: return content as text.
                    let text = String::from_utf8_lossy(&data);
                    format!("200 {} bytes\n{text}", data.len())
                },
                Err(e) => format!("500 {e}\n"),
            }
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
                        crate::vfs::EntryKind::Directory => "directory",
                        crate::vfs::EntryKind::File => "file",
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

/// Maximum failed authentication attempts before disconnecting.
const MAX_AUTH_FAILURES: u8 = 3;

/// Constant-time byte comparison (does not leak the password via timing).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A single FTP client connection.
struct FtpConnection {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    /// Output the non-blocking socket has not accepted yet; bytes before
    /// `write_pos` are already sent.
    write_buf: Vec<u8>,
    write_pos: usize,
    last_activity: Instant,
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
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
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
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.dead = true;
                    break;
                },
            }
        }
    }

    /// Process up to [`MAX_CMDS_PER_POLL`] buffered lines.
    fn process_lines(&mut self, password: Option<&str>, vfs: &mut dyn Vfs) {
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
                    Some(pass) if constant_time_eq(pass.as_bytes(), expected.as_bytes()) => {
                        self.authenticated = true;
                        self.queue(b"230 Authenticated\r\n");
                    },
                    Some(_) => {
                        self.failed_attempts += 1;
                        if self.failed_attempts >= MAX_AUTH_FAILURES {
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
            self.queue(response.as_bytes());
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
}

impl FtpServer {
    /// Create a new FTP server on the given port.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            connections: Vec::new(),
            listening: false,
            password: None,
        }
    }

    /// Set an optional password for FTP authentication (builder pattern).
    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    /// Start listening on the configured port.
    ///
    /// Without a (non-empty) password the server grants read/write access to
    /// anyone who can connect, so it binds loopback only
    /// ([`NetworkBackend::listen_loopback`]); with a password it listens on
    /// all interfaces.
    pub fn start(&mut self, backend: &mut dyn NetworkBackend) -> Result<()> {
        if self.password.as_deref().is_some_and(|p| !p.is_empty()) {
            backend.listen(self.port)?;
        } else {
            backend.listen_loopback(self.port)?;
        }
        self.listening = true;
        Ok(())
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
        for conn in &mut self.connections {
            conn.flush();
            if conn.dead || conn.closing_since.is_some() {
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
            // draining the responses already queued for it.
            if conn.pending() < MAX_PENDING_OUTPUT {
                conn.read_available();
            }
            if conn.dead {
                continue;
            }
            // Buffered lines are processed every poll, not only when new
            // bytes arrive (a burst beyond MAX_CMDS_PER_POLL would stall).
            conn.process_lines(password, vfs);

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

// ---------------------------------------------------------------------------
// Terminal commands
// ---------------------------------------------------------------------------

/// `ftp` -- manage the FTP server.
pub struct FtpCmd;

impl Command for FtpCmd {
    fn name(&self) -> &str {
        "ftp"
    }
    fn description(&self) -> &str {
        "Manage the file transfer server"
    }
    fn usage(&self) -> &str {
        "ftp [start [port] [--password <pass>]|stop|status]"
    }
    fn category(&self) -> &str {
        "transfer"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("status");

        match subcmd {
            "start" => {
                let mut port = DEFAULT_FTP_PORT;
                let mut password: Option<String> = None;
                let mut i = 1;
                while i < args.len() {
                    if args[i] == "--password" {
                        if let Some(&pass) = args.get(i + 1) {
                            password = Some(pass.to_string());
                            i += 2;
                        } else {
                            return Err(OasisError::Command("--password requires a value".into()));
                        }
                    } else if let Ok(p) = args[i].parse::<u16>() {
                        port = p;
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                Ok(CommandOutput::ftp_toggle(port, password))
            },
            "stop" => Ok(CommandOutput::ftp_toggle(0, None)),
            "status" => {
                if env.vfs.exists(FTP_STATUS_PATH) {
                    let data = env.vfs.read(FTP_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    Ok(CommandOutput::Text(format!("FTP: {text}")))
                } else {
                    Ok(CommandOutput::Text("FTP: inactive".to_string()))
                }
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: {}", self.usage()).into(),
            )),
        }
    }
}

/// `push` -- upload a local VFS file (placeholder for remote transfer).
pub struct PushCmd;

impl Command for PushCmd {
    fn name(&self) -> &str {
        "push"
    }
    fn description(&self) -> &str {
        "Copy a file to a transfer staging area"
    }
    fn usage(&self) -> &str {
        "push <source> <dest>"
    }
    fn category(&self) -> &str {
        "transfer"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let src = args
            .first()
            .copied()
            .ok_or_else(|| OasisError::Command("usage: push <source> <dest>".into()))?;
        let dest = args
            .get(1)
            .copied()
            .ok_or_else(|| OasisError::Command("usage: push <source> <dest>".into()))?;

        let data = env.vfs.read(src)?;
        env.vfs.write(dest, &data)?;
        Ok(CommandOutput::Text(format!(
            "Copied {} bytes: {src} -> {dest}",
            data.len()
        )))
    }
}

/// `pull` -- download a file (VFS copy for now).
pub struct PullCmd;

impl Command for PullCmd {
    fn name(&self) -> &str {
        "pull"
    }
    fn description(&self) -> &str {
        "Copy a file from a transfer staging area"
    }
    fn usage(&self) -> &str {
        "pull <source> <dest>"
    }
    fn category(&self) -> &str {
        "transfer"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let src = args
            .first()
            .copied()
            .ok_or_else(|| OasisError::Command("usage: pull <source> <dest>".into()))?;
        let dest = args
            .get(1)
            .copied()
            .ok_or_else(|| OasisError::Command("usage: pull <source> <dest>".into()))?;

        let data = env.vfs.read(src)?;
        env.vfs.write(dest, &data)?;
        Ok(CommandOutput::Text(format!(
            "Copied {} bytes: {src} -> {dest}",
            data.len()
        )))
    }
}

/// Register transfer commands.
pub fn register_transfer_commands(reg: &mut crate::terminal::CommandRegistry) {
    reg.register(Box::new(FtpCmd));
    reg.register(Box::new(PushCmd));
    reg.register(Box::new(PullCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{CommandRegistry, CommandSignal};
    use crate::vfs::MemoryVfs;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_transfer_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/tmp").unwrap();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/ftp").unwrap();
        vfs.write("/home/test.txt", b"Hello FTP").unwrap();
        (reg, vfs)
    }

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    // -- FTP protocol tests --

    #[test]
    fn ftp_list_root() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/tmp").unwrap();
        let resp = process_ftp_request("LIST /", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(resp.contains("home"));
        assert!(resp.contains("tmp"));
    }

    #[test]
    fn ftp_list_empty() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/empty").unwrap();
        let resp = process_ftp_request("LIST /empty", &mut vfs);
        assert!(resp.contains("200"));
        assert!(resp.contains("empty"));
    }

    #[test]
    fn ftp_get_file() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"hello").unwrap();
        let resp = process_ftp_request("GET /test.txt", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(resp.contains("5 bytes"));
        assert!(resp.contains("hello"));
    }

    #[test]
    fn ftp_get_missing() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("GET /nope.txt", &mut vfs);
        assert!(resp.starts_with("500"));
    }

    #[test]
    fn ftp_put_file() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("PUT /new.txt hello world", &mut vfs);
        assert!(resp.starts_with("200"));
        let data = vfs.read("/new.txt").unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn ftp_mkdir() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("MKDIR /newdir", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(vfs.exists("/newdir"));
    }

    #[test]
    fn ftp_delete() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/deleteme.txt", b"gone").unwrap();
        let resp = process_ftp_request("DELETE /deleteme.txt", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(!vfs.exists("/deleteme.txt"));
    }

    #[test]
    fn ftp_stat_file() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/info.txt", b"data").unwrap();
        let resp = process_ftp_request("STAT /info.txt", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(resp.contains("file"));
        assert!(resp.contains("4 bytes"));
    }

    #[test]
    fn ftp_stat_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/mydir").unwrap();
        let resp = process_ftp_request("STAT /mydir", &mut vfs);
        assert!(resp.contains("directory"));
    }

    #[test]
    fn ftp_rename() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/src").unwrap();
        vfs.mkdir("/dst").unwrap();
        vfs.write("/src/file.txt", b"hello").unwrap();
        let resp = process_ftp_request("RENAME /src/file.txt /dst/moved.txt", &mut vfs);
        assert!(resp.starts_with("200"));
        assert!(!vfs.exists("/src/file.txt"));
        assert_eq!(vfs.read("/dst/moved.txt").unwrap(), b"hello");
    }

    #[test]
    fn ftp_rename_missing_source() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("RENAME /nope.txt /dest.txt", &mut vfs);
        assert!(resp.starts_with("500"));
    }

    #[test]
    fn ftp_rename_missing_args() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("RENAME", &mut vfs);
        assert!(resp.starts_with("400"));
        let resp = process_ftp_request("RENAME /only_one", &mut vfs);
        assert!(resp.starts_with("400"));
    }

    #[test]
    fn ftp_quit() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("QUIT", &mut vfs);
        assert!(resp.contains("goodbye"));
    }

    #[test]
    fn ftp_unknown_command() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("BADCMD", &mut vfs);
        assert!(resp.starts_with("400"));
    }

    #[test]
    fn ftp_empty_command() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("", &mut vfs);
        assert!(resp.starts_with("400"));
    }

    #[test]
    fn ftp_get_missing_path() {
        let mut vfs = MemoryVfs::new();
        let resp = process_ftp_request("GET", &mut vfs);
        assert!(resp.starts_with("400"));
    }

    // -- Terminal command tests --

    #[test]
    fn ftp_cmd_status_inactive() {
        let (reg, mut vfs) = setup();
        // Remove the status file to test default.
        vfs.remove(FTP_STATUS_PATH).ok();
        let __out = exec(&reg, &mut vfs, "ftp status").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("inactive"));
    }

    #[test]
    fn ftp_cmd_start() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "ftp start 8021").unwrap();
        let CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) = __out else {
            panic!("expected CommandOutput::Signal(CommandSignal::FtpToggle), got {__out:?}");
        };
        assert_eq!(port, 8021);
        assert!(password.is_none());
    }

    #[test]
    fn ftp_cmd_start_default_port() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "ftp start").unwrap();
        let CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) = __out else {
            panic!("expected CommandOutput::Signal(CommandSignal::FtpToggle), got {__out:?}");
        };
        assert_eq!(port, DEFAULT_FTP_PORT);
        assert!(password.is_none());
    }

    #[test]
    fn ftp_cmd_stop() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "ftp stop").unwrap();
        let CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) = __out else {
            panic!("expected CommandOutput::Signal(CommandSignal::FtpToggle), got {__out:?}");
        };
        assert_eq!(port, 0);
        assert!(password.is_none());
    }

    #[test]
    fn ftp_cmd_unknown() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "ftp badcmd").is_err());
    }

    #[test]
    fn push_copies_file() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "push /home/test.txt /tmp/copy.txt").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("9 bytes"));
        let data = vfs.read("/tmp/copy.txt").unwrap();
        assert_eq!(data, b"Hello FTP");
    }

    #[test]
    fn push_missing_source() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "push /nope.txt /tmp/out.txt").is_err());
    }

    #[test]
    fn push_missing_args() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "push").is_err());
        assert!(exec(&reg, &mut vfs, "push /home/test.txt").is_err());
    }

    #[test]
    fn pull_copies_file() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "pull /home/test.txt /tmp/pulled.txt").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("9 bytes"));
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
        fn read(&mut self, buf: &mut [u8]) -> crate::error::Result<usize> {
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

        fn write(&mut self, data: &[u8]) -> crate::error::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }

        fn close(&mut self) -> crate::error::Result<()> {
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
        fn listen(&mut self, _port: u16) -> crate::error::Result<()> {
            Ok(())
        }

        fn accept(
            &mut self,
        ) -> crate::error::Result<Option<Box<dyn oasis_types::backend::NetworkStream>>> {
            Ok(self.pending.pop_front())
        }

        fn connect(
            &mut self,
            _address: &str,
            _port: u16,
        ) -> crate::error::Result<Box<dyn oasis_types::backend::NetworkStream>> {
            Err(OasisError::Backend("mock: no outbound".into()))
        }
    }

    /// Helper: collect all bytes written to the shared output buffer.
    fn read_output(output: &WriteBuf) -> String {
        String::from_utf8_lossy(&output.lock().unwrap()).into_owned()
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

    #[test]
    fn ftp_cmd_start_with_password() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "ftp start 8021 --password secret").unwrap();
        let CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) = __out else {
            panic!("expected CommandOutput::Signal(CommandSignal::FtpToggle), got {__out:?}");
        };
        assert_eq!(port, 8021);
        assert_eq!(password.as_deref(), Some("secret"));
    }

    #[test]
    fn ftp_cmd_start_password_default_port() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "ftp start --password mypass").unwrap();
        let CommandOutput::Signal(CommandSignal::FtpToggle { port, password }) = __out else {
            panic!("expected CommandOutput::Signal(CommandSignal::FtpToggle), got {__out:?}");
        };
        assert_eq!(port, DEFAULT_FTP_PORT);
        assert_eq!(password.as_deref(), Some("mypass"));
    }

    #[test]
    fn ftp_cmd_start_password_missing_value() {
        let (reg, mut vfs) = setup();
        assert!(
            exec(&reg, &mut vfs, "ftp start --password").is_err(),
            "--password without value should error"
        );
    }

    /// Accepts at most `budget` more bytes; further writes would block.
    struct TrickleStream {
        input: VecDeque<u8>,
        output: WriteBuf,
        budget: Arc<Mutex<usize>>,
    }

    impl oasis_types::backend::NetworkStream for TrickleStream {
        fn read(&mut self, buf: &mut [u8]) -> crate::error::Result<usize> {
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

        fn write(&mut self, data: &[u8]) -> crate::error::Result<usize> {
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

        fn close(&mut self) -> crate::error::Result<()> {
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
            fn read(&mut self, _buf: &mut [u8]) -> crate::error::Result<usize> {
                Ok(0)
            }
            fn write(&mut self, data: &[u8]) -> crate::error::Result<usize> {
                Ok(data.len())
            }
            fn close(&mut self) -> crate::error::Result<()> {
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
    fn ftp_password_compare_is_exact() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret1"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
