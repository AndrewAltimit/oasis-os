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

/// Idle connection timeout in seconds.
const FTP_IDLE_TIMEOUT_SECS: u64 = 300;

/// Maximum failed authentication attempts before disconnecting.
const MAX_AUTH_FAILURES: u8 = 3;

/// A single FTP client connection.
struct FtpConnection {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    last_activity: Instant,
    /// Whether this connection has been authenticated.
    authenticated: bool,
    /// Number of failed authentication attempts.
    failed_attempts: u8,
}

impl FtpConnection {
    fn new(stream: Box<dyn NetworkStream>, authenticated: bool) -> Self {
        Self {
            stream,
            read_buf: Vec::with_capacity(256),
            last_activity: Instant::now(),
            authenticated,
            failed_attempts: 0,
        }
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
    pub fn start(&mut self, backend: &mut dyn NetworkBackend) -> Result<()> {
        backend.listen(self.port)?;
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
    /// immediately against the provided VFS.
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
                    let _ = conn.stream.write(greeting);
                    self.connections.push(conn);
                },
                Ok(None) => {},
                Err(e) => log::warn!("FTP accept error: {e}"),
            }
        }

        // Read from all connections.
        let mut to_remove = Vec::new();

        for (idx, conn) in self.connections.iter_mut().enumerate() {
            // Check idle timeout.
            if conn.last_activity.elapsed() > idle_timeout {
                let _ = conn.stream.write(b"421 Idle timeout\r\n");
                to_remove.push(idx);
                continue;
            }

            let mut buf = [0u8; 512];
            match conn.stream.read(&mut buf) {
                Ok(0) => {},
                Err(OasisError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data yet.
                },
                Ok(n) => {
                    conn.last_activity = Instant::now();
                    conn.read_buf.extend_from_slice(&buf[..n]);

                    // Process complete lines (capped per poll cycle).
                    let mut cmds_processed = 0usize;
                    let mut should_close = false;
                    while cmds_processed < MAX_CMDS_PER_POLL {
                        let Some(pos) = conn.read_buf.iter().position(|&b| b == b'\n') else {
                            break;
                        };
                        cmds_processed += 1;
                        let line_bytes: Vec<u8> = conn.read_buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // Check for QUIT (always allowed).
                        if line.eq_ignore_ascii_case("QUIT") {
                            let _ = conn.stream.write(b"200 goodbye\r\n");
                            to_remove.push(idx);
                            should_close = true;
                            break;
                        }

                        // Authentication gate.
                        if !conn.authenticated
                            && let Some(ref expected) = self.password
                        {
                            let upper = line.to_uppercase();
                            if upper.starts_with("PASS ") {
                                let supplied = line[5..].trim();
                                if supplied == expected.as_str() {
                                    conn.authenticated = true;
                                    let _ = conn.stream.write(b"230 Authenticated\r\n");
                                } else {
                                    conn.failed_attempts += 1;
                                    if conn.failed_attempts >= MAX_AUTH_FAILURES {
                                        let _ = conn.stream.write(b"530 Too many failures\r\n");
                                        to_remove.push(idx);
                                        should_close = true;
                                        break;
                                    }
                                    let _ = conn.stream.write(b"530 Authentication failed\r\n");
                                }
                            } else {
                                let _ = conn.stream.write(b"530 Not authenticated\r\n");
                            }
                            continue;
                        }

                        // Process command against VFS.
                        let response = process_ftp_request(&line, vfs);
                        let _ = conn.stream.write(response.as_bytes());
                    }

                    if should_close {
                        continue;
                    }

                    // Guard against overlong lines.
                    if conn.read_buf.len() > MAX_FTP_LINE_LEN {
                        conn.read_buf.clear();
                        let _ = conn.stream.write(b"500 line too long\r\n");
                    }
                },
                Err(_) => {
                    to_remove.push(idx);
                },
            }
        }

        // Remove closed connections (in reverse to preserve indices).
        to_remove.sort_unstable();
        to_remove.dedup();
        for idx in to_remove.into_iter().rev() {
            let mut conn = self.connections.remove(idx);
            let _ = conn.stream.close();
        }

        Ok(())
    }

    /// Shut down all connections and stop listening.
    pub fn stop(&mut self) {
        for conn in &mut self.connections {
            let _ = conn.stream.write(b"421 Server shutting down\r\n");
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
}
