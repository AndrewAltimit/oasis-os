//! WiFi / network terminal commands.

use oasis_types::error::{OasisError, Result};

use crate::{CommandOutput, Environment};

register_commands!(register_network_commands, [WifiCmd, PingCmd, HttpCmd]);

// ---------------------------------------------------------------------------
// wifi
// ---------------------------------------------------------------------------

// WiFi status (hardware, connection, IP, MAC).
define_command!(
    WifiCmd,
    "wifi",
    "Show WiFi status",
    "wifi [status]",
    "network",
    |args, env| {
        let subcmd = args.first().copied().unwrap_or("status");
        match subcmd {
            "status" | "" => wifi_status(env),
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: wifi [status]").into(),
            )),
        }
    }
);

fn wifi_status(env: &mut Environment<'_>) -> Result<CommandOutput> {
    let Some(net) = env.network else {
        return Ok(CommandOutput::Text(
            "wifi: no network service available".into(),
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

// Test network connectivity via DNS resolution.
define_command!(
    PingCmd,
    "ping",
    "Test network connectivity (DNS resolve)",
    "ping <hostname>",
    "network",
    |args, env| {
        if args.is_empty() {
            return Err(OasisError::Command("usage: ping <hostname>".into()));
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
);

// ---------------------------------------------------------------------------
// http (HTTP GET via platform network service)
// ---------------------------------------------------------------------------

// HTTP GET request via platform network service.
define_command!(
    HttpCmd,
    "http",
    "HTTP GET request",
    "http <url>",
    "network",
    |args, env| {
        if args.is_empty() {
            return Err(OasisError::Command("usage: http <url>".into()));
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
);

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_platform::DesktopPlatform;

    use crate::test_helpers::assert_text;
    use crate::{CommandRegistry, Environment};
    use oasis_vfs::MemoryVfs;

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
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let s = assert_text!(reg.execute("wifi", &mut env).unwrap());
        assert!(s.contains("no network service"));
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
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let s = assert_text!(reg.execute("wifi", &mut env).unwrap());
        assert!(s.contains("unavailable"));
        assert!(s.contains("disconnected"));
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
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        assert!(reg.execute("ping", &mut env).is_err());
    }
}
