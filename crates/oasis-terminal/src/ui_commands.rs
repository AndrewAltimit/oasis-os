//! Window & UI control commands: wm, sdi, theme, notify, screenshot.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment};

// ---------------------------------------------------------------------------
// wm
// ---------------------------------------------------------------------------

struct WmCmd;
impl Command for WmCmd {
    fn name(&self) -> &str {
        "wm"
    }
    fn description(&self) -> &str {
        "Window manager control"
    }
    fn usage(&self) -> &str {
        "wm [list|close <id>|focus <id>|minimize <id>]"
    }
    fn category(&self) -> &str {
        "ui"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");
        match subcmd {
            "list" => {
                // Read window state from VFS if available.
                let status_path = "/var/wm/status";
                if env.vfs.exists(status_path) {
                    let data = env.vfs.read(status_path)?;
                    Ok(CommandOutput::Text(
                        String::from_utf8_lossy(&data).into_owned(),
                    ))
                } else {
                    Ok(CommandOutput::Text(
                        "(no window manager status available)".to_string(),
                    ))
                }
            },
            "close" | "focus" | "minimize" | "maximize" => {
                let id = args.get(1).copied().unwrap_or("");
                if id.is_empty() {
                    return Err(OasisError::Command(format!(
                        "usage: wm {subcmd} <window-id>"
                    )));
                }
                let request = format!("{subcmd} {id}");
                let req_path = "/var/wm/request";
                env.vfs.write(req_path, request.as_bytes())?;
                Ok(CommandOutput::Text(format!("WM request: {subcmd} {id}")))
            },
            _ => Err(OasisError::Command(format!(
                "unknown subcommand: {subcmd}\nusage: {}",
                self.usage()
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// sdi
// ---------------------------------------------------------------------------

struct SdiCmd;
impl Command for SdiCmd {
    fn name(&self) -> &str {
        "sdi"
    }
    fn description(&self) -> &str {
        "Inspect SDI scene objects"
    }
    fn usage(&self) -> &str {
        "sdi [list|get <name>]"
    }
    fn category(&self) -> &str {
        "ui"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");
        match subcmd {
            "list" => {
                let status_path = "/var/sdi/status";
                if env.vfs.exists(status_path) {
                    let data = env.vfs.read(status_path)?;
                    Ok(CommandOutput::Text(
                        String::from_utf8_lossy(&data).into_owned(),
                    ))
                } else {
                    Ok(CommandOutput::Text(
                        "(SDI status not available -- enable debug output)".to_string(),
                    ))
                }
            },
            "get" => {
                let name = args.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return Err(OasisError::Command("usage: sdi get <name>".to_string()));
                }
                Ok(CommandOutput::Text(format!(
                    "SDI object '{name}': (query via VFS not yet implemented)"
                )))
            },
            _ => Err(OasisError::Command(format!("unknown subcommand: {subcmd}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// theme
// ---------------------------------------------------------------------------

struct ThemeCmd;
impl Command for ThemeCmd {
    fn name(&self) -> &str {
        "theme"
    }
    fn description(&self) -> &str {
        "Show or modify current theme"
    }
    fn usage(&self) -> &str {
        "theme [show|colors]"
    }
    fn category(&self) -> &str {
        "ui"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("show");
        match subcmd {
            "show" | "colors" => {
                let theme_path = "/var/theme/current";
                if env.vfs.exists(theme_path) {
                    let data = env.vfs.read(theme_path)?;
                    Ok(CommandOutput::Text(
                        String::from_utf8_lossy(&data).into_owned(),
                    ))
                } else {
                    Ok(CommandOutput::Text(
                        "Current theme: (use 'skin' to switch themes)".to_string(),
                    ))
                }
            },
            _ => Err(OasisError::Command(format!("unknown subcommand: {subcmd}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// notify
// ---------------------------------------------------------------------------

struct NotifyCmd;
impl Command for NotifyCmd {
    fn name(&self) -> &str {
        "notify"
    }
    fn description(&self) -> &str {
        "Show a notification message"
    }
    fn usage(&self) -> &str {
        "notify <message>"
    }
    fn category(&self) -> &str {
        "ui"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: notify <message>".to_string()));
        }
        let message = args.join(" ");
        // Write notification to VFS for the UI layer to pick up.
        let notify_path = "/var/notify/message";
        env.vfs.write(notify_path, message.as_bytes())?;
        Ok(CommandOutput::Text(format!(
            "Notification queued: {message}"
        )))
    }
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

struct ScreenshotCmd;
impl Command for ScreenshotCmd {
    fn name(&self) -> &str {
        "screenshot"
    }
    fn description(&self) -> &str {
        "Take a screenshot"
    }
    fn usage(&self) -> &str {
        "screenshot [path]"
    }
    fn category(&self) -> &str {
        "ui"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let path = args.first().copied().unwrap_or("/tmp/screenshot.bmp");
        let req_path = "/var/screenshot/request";
        env.vfs.write(req_path, path.as_bytes())?;
        Ok(CommandOutput::Text(format!(
            "Screenshot request queued: {path}"
        )))
    }
}

/// Register UI control commands.
pub fn register_ui_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(WmCmd));
    reg.register(Box::new(SdiCmd));
    reg.register(Box::new(ThemeCmd));
    reg.register(Box::new(NotifyCmd));
    reg.register(Box::new(ScreenshotCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

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

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/wm").unwrap();
        vfs.mkdir("/var/notify").unwrap();
        vfs.mkdir("/var/screenshot").unwrap();
        (reg, vfs)
    }

    #[test]
    fn wm_list_no_status() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "wm list").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("no window manager")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn wm_close_no_id() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "wm close").is_err());
    }

    #[test]
    fn wm_close_queues_request() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "wm close browser").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("close browser")),
            _ => panic!("expected text"),
        }
        let data = vfs.read("/var/wm/request").unwrap();
        assert_eq!(data, b"close browser");
    }

    #[test]
    fn sdi_list_no_status() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "sdi").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("SDI status")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn theme_show() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "theme").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("theme")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn notify_queues_message() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "notify Hello World").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("Hello World")),
            _ => panic!("expected text"),
        }
        let data = vfs.read("/var/notify/message").unwrap();
        assert_eq!(data, b"Hello World");
    }

    #[test]
    fn screenshot_queues_request() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "screenshot /tmp/shot.bmp").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("shot.bmp")),
            _ => panic!("expected text"),
        }
    }
}
