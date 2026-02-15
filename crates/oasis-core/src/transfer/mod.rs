//! File transfer services -- FTP-like server and push/pull commands.
//!
//! Provides a minimal file transfer protocol over TCP using the
//! `NetworkBackend` trait. The protocol is line-based:
//!
//! - `LIST <path>` -- list directory contents
//! - `GET <path>`  -- retrieve file (response: size + data)
//! - `PUT <path> <size>` -- upload file
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

/// Idle connection timeout in seconds.
const FTP_IDLE_TIMEOUT_SECS: u64 = 300;

/// A single FTP client connection.
struct FtpConnection {
    stream: Box<dyn NetworkStream>,
    read_buf: Vec<u8>,
    last_activity: Instant,
}

impl FtpConnection {
    fn new(stream: Box<dyn NetworkStream>) -> Self {
        Self {
            stream,
            read_buf: Vec::with_capacity(256),
            last_activity: Instant::now(),
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
}

impl FtpServer {
    /// Create a new FTP server on the given port.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            connections: Vec::new(),
            listening: false,
        }
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
        if self.connections.len() < MAX_FTP_CONNECTIONS {
            match backend.accept() {
                Ok(Some(stream)) => {
                    let mut conn = FtpConnection::new(stream);
                    let _ = conn.stream.write(b"220 OASIS FTP server ready\r\n");
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

                    // Process complete lines.
                    while let Some(pos) = conn.read_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = conn.read_buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // Check for QUIT.
                        if line.eq_ignore_ascii_case("QUIT") {
                            let _ = conn.stream.write(b"200 goodbye\r\n");
                            to_remove.push(idx);
                            break;
                        }

                        // Process command against VFS.
                        let response = process_ftp_request(&line, vfs);
                        let _ = conn.stream.write(response.as_bytes());
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
        "ftp [start [port]|stop|status]"
    }
    fn category(&self) -> &str {
        "transfer"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("status");

        match subcmd {
            "start" => {
                let port = args
                    .get(1)
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(DEFAULT_FTP_PORT);
                Ok(CommandOutput::FtpToggle { port })
            },
            "stop" => Ok(CommandOutput::FtpToggle { port: 0 }),
            "status" => {
                if env.vfs.exists(FTP_STATUS_PATH) {
                    let data = env.vfs.read(FTP_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    Ok(CommandOutput::Text(format!("FTP: {text}")))
                } else {
                    Ok(CommandOutput::Text("FTP: inactive".to_string()))
                }
            },
            _ => Err(OasisError::Command(format!(
                "unknown subcommand: {subcmd}\nusage: {}",
                self.usage()
            ))),
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
            .ok_or_else(|| OasisError::Command("usage: push <source> <dest>".to_string()))?;
        let dest = args
            .get(1)
            .copied()
            .ok_or_else(|| OasisError::Command("usage: push <source> <dest>".to_string()))?;

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
            .ok_or_else(|| OasisError::Command("usage: pull <source> <dest>".to_string()))?;
        let dest = args
            .get(1)
            .copied()
            .ok_or_else(|| OasisError::Command("usage: pull <source> <dest>".to_string()))?;

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
    use crate::terminal::CommandRegistry;
    use crate::vfs::MemoryVfs;

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
        match exec(&reg, &mut vfs, "ftp status").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("inactive")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn ftp_cmd_start() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "ftp start 8021").unwrap() {
            CommandOutput::FtpToggle { port } => assert_eq!(port, 8021),
            other => panic!("expected FtpToggle, got {other:?}"),
        }
    }

    #[test]
    fn ftp_cmd_start_default_port() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "ftp start").unwrap() {
            CommandOutput::FtpToggle { port } => {
                assert_eq!(port, DEFAULT_FTP_PORT);
            },
            other => panic!("expected FtpToggle, got {other:?}"),
        }
    }

    #[test]
    fn ftp_cmd_stop() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "ftp stop").unwrap() {
            CommandOutput::FtpToggle { port } => assert_eq!(port, 0),
            other => panic!("expected FtpToggle stop, got {other:?}"),
        }
    }

    #[test]
    fn ftp_cmd_unknown() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "ftp badcmd").is_err());
    }

    #[test]
    fn push_copies_file() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "push /home/test.txt /tmp/copy.txt").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("9 bytes")),
            _ => panic!("expected text"),
        }
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
        match exec(&reg, &mut vfs, "pull /home/test.txt /tmp/pulled.txt").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("9 bytes")),
            _ => panic!("expected text"),
        }
    }
}
