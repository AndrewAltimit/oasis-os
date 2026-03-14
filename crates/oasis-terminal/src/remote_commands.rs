//! Remote terminal commands: listen, remote, hosts.

use oasis_types::error::{OasisError, Result};

use crate::cmd_helpers::require_args;
use crate::interpreter::{Command, CommandOutput, Environment};

register_commands!(register_remote_commands, [ListenCmd, RemoteCmd, HostsCmd]);

// ---------------------------------------------------------------------------
// listen
// ---------------------------------------------------------------------------

struct ListenCmd;
impl Command for ListenCmd {
    fn name(&self) -> &str {
        "listen"
    }
    fn description(&self) -> &str {
        "Start/stop remote terminal listener"
    }
    fn usage(&self) -> &str {
        "listen [port|stop]"
    }
    fn category(&self) -> &str {
        "network"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Ok(CommandOutput::listen_toggle(9000));
        }
        if args[0] == "stop" {
            return Ok(CommandOutput::listen_toggle(0));
        }
        match args[0].parse::<u16>() {
            Ok(port) => Ok(CommandOutput::listen_toggle(port)),
            Err(_) => Err(OasisError::Command("usage: listen [port|stop]".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// remote
// ---------------------------------------------------------------------------

struct RemoteCmd;
impl Command for RemoteCmd {
    fn name(&self) -> &str {
        "remote"
    }
    fn description(&self) -> &str {
        "Connect to a remote host"
    }
    fn usage(&self) -> &str {
        "remote <host|addr:port>"
    }
    fn category(&self) -> &str {
        "network"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        require_args(args, 1, "remote <host|addr:port>")?;
        let target = args[0];

        // Try addr:port format.
        if let Some((addr, port_str)) = target.rsplit_once(':')
            && let Ok(port) = port_str.parse::<u16>()
        {
            return Ok(CommandOutput::remote_connect(addr.into(), port, None));
        }

        // Look up saved host from VFS config.
        let hosts_path = "/etc/hosts.toml";
        if env.vfs.exists(hosts_path) {
            let data = env.vfs.read(hosts_path)?;
            let toml_str = String::from_utf8_lossy(&data);
            if let Ok(hosts) = oasis_net::parse_hosts(&toml_str) {
                for host in &hosts {
                    if host.name == target {
                        return Ok(CommandOutput::remote_connect(
                            host.address.clone(),
                            host.port,
                            host.psk.clone(),
                        ));
                    }
                }
            }
        }

        Err(OasisError::Command(
            format!("unknown host: {target}  (use addr:port or configure in /etc/hosts.toml)")
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// hosts
// ---------------------------------------------------------------------------

struct HostsCmd;
impl Command for HostsCmd {
    fn name(&self) -> &str {
        "hosts"
    }
    fn description(&self) -> &str {
        "List saved remote hosts"
    }
    fn usage(&self) -> &str {
        "hosts"
    }
    fn category(&self) -> &str {
        "network"
    }
    fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let hosts_path = "/etc/hosts.toml";
        if !env.vfs.exists(hosts_path) {
            return Ok(CommandOutput::Text(
                "(no hosts configured -- create /etc/hosts.toml)".into(),
            ));
        }
        let data = env.vfs.read(hosts_path)?;
        let toml_str = String::from_utf8_lossy(&data);
        let hosts = oasis_net::parse_hosts(&toml_str)?;
        if hosts.is_empty() {
            return Ok(CommandOutput::Text("(no hosts defined)".to_string()));
        }
        let mut lines = Vec::new();
        for h in &hosts {
            lines.push(format!(
                "  {} -> {}:{} ({})",
                h.name, h.address, h.port, h.protocol
            ));
        }
        Ok(CommandOutput::Text(format!(
            "Saved hosts:\n{}",
            lines.join("\n")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, CommandSignal, Environment};
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
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn listen_default_port() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let __out = exec(&reg, &mut vfs, "listen").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected ListenToggle, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::ListenToggle { port }) = __out else {
            unreachable!()
        };
        assert_eq!(port, 9000);
    }

    #[test]
    fn listen_custom_port() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let __out = exec(&reg, &mut vfs, "listen 8080").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected ListenToggle, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::ListenToggle { port }) = __out else {
            unreachable!()
        };
        assert_eq!(port, 8080);
    }

    #[test]
    fn listen_stop() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let __out = exec(&reg, &mut vfs, "listen stop").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected ListenToggle, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::ListenToggle { port }) = __out else {
            unreachable!()
        };
        assert_eq!(port, 0);
    }

    #[test]
    fn remote_addr_port() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let __out = exec(&reg, &mut vfs, "remote 192.168.0.50:9000").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected RemoteConnect, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::RemoteConnect { address, port, psk }) = __out
        else {
            unreachable!()
        };
        assert_eq!(address, "192.168.0.50");
        assert_eq!(port, 9000);
        assert!(psk.is_none());
    }

    #[test]
    fn remote_saved_host() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.write(
            "/etc/hosts.toml",
            br#"
[[host]]
name = "myserver"
address = "10.0.0.1"
port = 8080
psk = "secret"
"#,
        )
        .unwrap();
        let __out = exec(&reg, &mut vfs, "remote myserver").unwrap();
        assert!(
            matches!(&__out, CommandOutput::Signal(_)),
            "expected RemoteConnect, got {__out:?}"
        );
        let CommandOutput::Signal(CommandSignal::RemoteConnect { address, port, psk }) = __out
        else {
            unreachable!()
        };
        assert_eq!(address, "10.0.0.1");
        assert_eq!(port, 8080);
        assert_eq!(psk, Some("secret".to_string()));
    }

    #[test]
    fn remote_unknown_host() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "remote unknown").is_err());
    }

    #[test]
    fn remote_no_args() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "remote").is_err());
    }

    #[test]
    fn hosts_no_config() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(
            assert_text!(exec(&reg, &mut vfs, "hosts").unwrap()).contains("no hosts configured")
        );
    }

    #[test]
    fn hosts_with_config() {
        let mut reg = CommandRegistry::new();
        register_remote_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.write(
            "/etc/hosts.toml",
            br#"
[[host]]
name = "server1"
address = "1.2.3.4"
port = 9000

[[host]]
name = "server2"
address = "5.6.7.8"
port = 22
protocol = "raw-tcp"
"#,
        )
        .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "hosts").unwrap());
        assert!(s.contains("server1"));
        assert!(s.contains("server2"));
        assert!(s.contains("1.2.3.4"));
    }
}
