//! File transfer terminal commands: `ftp start/stop/status`, `push`,
//! `pull`.
//!
//! The FTP-like server itself ([`FtpServer`], [`process_ftp_request`])
//! lives in `oasis-net` (feature `vfs`) and is re-exported here; the
//! `ftp` command only signals the host (`CommandSignal::FtpToggle`), which
//! owns and polls the server.

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, Environment};

pub use oasis_net::ftp::{
    DEFAULT_FTP_PORT, FTP_REQUEST_PATH, FTP_STATUS_PATH, FtpServer, process_ftp_request,
};

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
    use crate::vfs::{MemoryVfs, Vfs};

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
