//! File transfer terminal commands: `ftp start/stop/status`, `push`,
//! `pull`.
//!
//! The FTP-like server itself ([`FtpServer`]) lives in `oasis_net::ftp`
//! (feature `vfs`) and is re-exported here; the `ftp` command only signals
//! the host (`CommandSignal::FtpToggle`), which owns and polls the server.
//! `ftp status` reads the host's network service state
//! ([`NET_STATE_PATH`], the same document the Network app shows).

use crate::error::{OasisError, Result};
use crate::ipc::{NET_STATE_PATH, NetState};
use crate::terminal::{Command, CommandOutput, Environment};

pub use oasis_net::ftp::{DEFAULT_FTP_PORT, FtpServer};

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
                    } else {
                        // Port 0 is the host's "stop" sentinel in
                        // `CommandSignal::FtpToggle`, so it cannot be a
                        // listen port; anything unparseable is a typo that
                        // must not silently fall back to the default port.
                        port = match args[i].parse::<u16>() {
                            Ok(p) if p != 0 => p,
                            _ => {
                                return Err(OasisError::Command(
                                    format!("invalid port: {}\nusage: {}", args[i], self.usage())
                                        .into(),
                                ));
                            },
                        };
                        i += 1;
                    }
                }
                Ok(CommandOutput::ftp_toggle(port, password))
            },
            "stop" => Ok(CommandOutput::ftp_toggle(0, None)),
            "status" => {
                // The host publishes its services' state every frame; a
                // host that does not (no FTP support) says so.
                let Ok(data) = env.vfs.read(NET_STATE_PATH) else {
                    return Ok(CommandOutput::Text(
                        "FTP: status not available on this host".to_string(),
                    ));
                };
                let state = NetState::parse(&String::from_utf8_lossy(&data));
                Ok(CommandOutput::Text(match state.ftp_port {
                    Some(port) => format!("FTP: listening on port {port}"),
                    None => "FTP: inactive".to_string(),
                }))
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
        copy_file(args, env, self.usage())
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
        copy_file(args, env, self.usage())
    }
}

/// Shared body of `push` / `pull`: copy `args[0]` to `args[1]` in the VFS.
fn copy_file(args: &[&str], env: &mut Environment<'_>, usage: &str) -> Result<CommandOutput> {
    let [src, dest, ..] = args else {
        return Err(OasisError::Command(format!("usage: {usage}").into()));
    };
    let data = env.vfs.read(src)?;
    env.vfs.write(dest, &data)?;
    Ok(CommandOutput::Text(format!(
        "Copied {} bytes: {src} -> {dest}",
        data.len()
    )))
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
    use crate::vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_transfer_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/tmp").unwrap();
        vfs.mkdir("/var").unwrap();
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

    // -- Terminal command tests --

    fn status_text(reg: &CommandRegistry, vfs: &mut MemoryVfs) -> String {
        let out = exec(reg, vfs, "ftp status").unwrap();
        let CommandOutput::Text(s) = out else {
            panic!("expected CommandOutput::Text, got {out:?}");
        };
        s
    }

    /// `ftp status` used to read a file no host wrote, so it said
    /// "inactive" while the server ran. It now reads the host's published
    /// network state.
    #[test]
    fn ftp_cmd_status_follows_the_host_net_state() {
        let (reg, mut vfs) = setup();
        assert!(status_text(&reg, &mut vfs).contains("not available"));

        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        let mut st = NetState::default();
        vfs.write(NET_STATE_PATH, st.to_text().as_bytes()).unwrap();
        assert_eq!(status_text(&reg, &mut vfs), "FTP: inactive");

        st.ftp_port = Some(2121);
        vfs.write(NET_STATE_PATH, st.to_text().as_bytes()).unwrap();
        assert_eq!(status_text(&reg, &mut vfs), "FTP: listening on port 2121");
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
    fn ftp_cmd_start_rejects_bad_port() {
        let (reg, mut vfs) = setup();
        // `0` is the stop sentinel: `ftp start 0` used to stop the server.
        assert!(exec(&reg, &mut vfs, "ftp start 0").is_err());
        // Out-of-range / typo'd ports used to silently fall back to 2121.
        assert!(exec(&reg, &mut vfs, "ftp start 70000").is_err());
        assert!(exec(&reg, &mut vfs, "ftp start 80x").is_err());
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
