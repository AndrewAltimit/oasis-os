//! WiFi / network terminal commands.

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, CommandRegistry, Environment};

/// Register network-related terminal commands.
pub fn register_network_commands(reg: &mut CommandRegistry) {
    reg.register(Box::new(WifiCmd));
    reg.register(Box::new(PingCmd));
    reg.register(Box::new(HttpCmd));
}

// ---------------------------------------------------------------------------
// wifi
// ---------------------------------------------------------------------------

struct WifiCmd;
impl Command for WifiCmd {
    fn name(&self) -> &str {
        "wifi"
    }
    fn description(&self) -> &str {
        "Show WiFi status"
    }
    fn usage(&self) -> &str {
        "wifi [status]"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("status");
        match subcmd {
            "status" | "" => wifi_status(env),
            _ => Err(OasisError::Command(format!(
                "unknown subcommand: {subcmd}\nusage: {}",
                self.usage()
            ))),
        }
    }
}

fn wifi_status(env: &mut Environment<'_>) -> Result<CommandOutput> {
    let Some(net) = env.network else {
        return Ok(CommandOutput::Text(
            "wifi: no network service available".to_string(),
        ));
    };
    let info = net.wifi_info()?;

    let mut lines = Vec::new();
    lines.push(format!(
        "WLAN hardware: {}",
        if info.available {
            "available"
        } else {
            "unavailable"
        }
    ));
    lines.push(format!(
        "Connection:    {}",
        if info.connected {
            "connected"
        } else {
            "disconnected"
        }
    ));
    if let Some(ip) = &info.ip_address {
        lines.push(format!("IP address:    {ip}"));
    }
    let mac = &info.mac_address;
    lines.push(format!(
        "MAC address:   {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    ));

    Ok(CommandOutput::Text(lines.join("\n")))
}

// ---------------------------------------------------------------------------
// ping (connectivity test via DNS resolve)
// ---------------------------------------------------------------------------

struct PingCmd;
impl Command for PingCmd {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "Test network connectivity (DNS resolve)"
    }
    fn usage(&self) -> &str {
        "ping <hostname>"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: ping <hostname>".to_string()));
        }
        let Some(net) = env.network else {
            return Ok(CommandOutput::Text(
                "ping: no network service available".to_string(),
            ));
        };
        let info = net.wifi_info()?;
        if !info.connected {
            return Ok(CommandOutput::Text("ping: WiFi not connected".to_string()));
        }
        // The actual DNS resolve / ICMP ping would be handled by the
        // platform-specific implementation. For now, just report status.
        Ok(CommandOutput::Text(format!(
            "Network is up (IP: {})",
            info.ip_address.as_deref().unwrap_or("unknown"),
        )))
    }
}

// ---------------------------------------------------------------------------
// http (HTTP GET via platform network service)
// ---------------------------------------------------------------------------

struct HttpCmd;
impl Command for HttpCmd {
    fn name(&self) -> &str {
        "http"
    }
    fn description(&self) -> &str {
        "HTTP GET request"
    }
    fn usage(&self) -> &str {
        "http <url>"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: http <url>".to_string()));
        }
        let Some(net) = env.network else {
            return Ok(CommandOutput::Text(
                "http: no network service available".to_string(),
            ));
        };
        let url = args[0];
        match net.http_get(url) {
            Ok(resp) => {
                let body_text = String::from_utf8_lossy(&resp.body);
                // Truncate long responses for terminal display.
                let truncated = if body_text.len() > 2048 {
                    let end = body_text.floor_char_boundary(2048);
                    format!(
                        "{}...\n(truncated, {} bytes total)",
                        &body_text[..end],
                        resp.body.len()
                    )
                } else {
                    body_text.into_owned()
                };
                Ok(CommandOutput::Text(format!(
                    "HTTP {} ({})\n{}",
                    resp.status_code,
                    resp.body.len(),
                    truncated,
                )))
            },
            Err(e) => Ok(CommandOutput::Text(format!("http: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::DesktopPlatform;
    use crate::terminal::Environment;
    use crate::vfs::MemoryVfs;

    #[test]
    fn wifi_no_service() {
        let mut reg = CommandRegistry::new();
        register_network_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
        };
        match reg.execute("wifi", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("no network service")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn wifi_with_desktop_platform() {
        let mut reg = CommandRegistry::new();
        register_network_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let platform = DesktopPlatform::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: Some(&platform),
        };
        match reg.execute("wifi", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("unavailable"));
                assert!(s.contains("disconnected"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn ping_no_args() {
        let mut reg = CommandRegistry::new();
        register_network_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
        };
        assert!(reg.execute("ping", &mut env).is_err());
    }
}
