//! Plugin management commands for the terminal.
//!
//! Provides `plugin list`, `plugin load`, and `plugin unload`. The command
//! interpreter has no access to the plugin manager (it lives in the host),
//! so the commands talk to it through VFS IPC: the host publishes the
//! plugin list at [`PLUGIN_STATUS_PATH`] and applies the
//! [`PluginRequest`] lines queued at [`PLUGIN_REQUEST_PATH`] (the desktop
//! shell does both every frame, see `oasis-app/src/commands/ipc.rs`).

use crate::error::{OasisError, Result};
pub use crate::ipc::{PLUGIN_REQUEST_PATH, PLUGIN_STATUS_PATH, PluginRequest};
use crate::terminal::{Command, CommandOutput, Environment};

/// Printed when no host publishes plugin status (no plugin manager runs).
const NO_HOST: &str = "plugin management is not available on this host";

const USAGE: &str = "plugin [list|load <name>|unload <name>]";

/// Append `request` to [`PLUGIN_REQUEST_PATH`] (a line queue: `plugin
/// unload a; plugin load b` in one frame keeps both).
fn queue(env: &mut Environment<'_>, request: &PluginRequest) -> Result<()> {
    let mut data = env.vfs.read(PLUGIN_REQUEST_PATH).unwrap_or_default();
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    data.extend_from_slice(request.to_line().as_bytes());
    env.vfs.write(PLUGIN_REQUEST_PATH, &data)
}

/// Manage plugins via VFS-based communication with the host.
pub struct PluginCmd;
impl Command for PluginCmd {
    fn name(&self) -> &str {
        "plugin"
    }
    fn description(&self) -> &str {
        "Manage plugins"
    }
    fn usage(&self) -> &str {
        USAGE
    }
    fn category(&self) -> &str {
        "plugin"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");
        let status = env
            .vfs
            .read(PLUGIN_STATUS_PATH)
            .ok()
            .map(|d| String::from_utf8_lossy(&d).into_owned());

        let request = match subcmd {
            "list" => {
                return Ok(CommandOutput::Text(match status {
                    None => NO_HOST.to_string(),
                    Some(text) if text.trim().is_empty() => "(no plugins)".to_string(),
                    Some(text) => format!("Plugins:\n{}", text.trim_end()),
                }));
            },
            "load" | "unload" => {
                let name = args.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return Err(OasisError::Command(
                        format!("usage: plugin {subcmd} <name>").into(),
                    ));
                }
                if subcmd == "load" {
                    PluginRequest::Load(name.to_string())
                } else {
                    PluginRequest::Unload(name.to_string())
                }
            },
            _ => {
                return Err(OasisError::Command(
                    format!("unknown subcommand: {subcmd}\nusage: {USAGE}").into(),
                ));
            },
        };
        if status.is_none() {
            return Ok(CommandOutput::Text(NO_HOST.to_string()));
        }
        queue(env, &request)?;
        // The host reports the outcome in the terminal when it applies
        // the request (this frame).
        Ok(CommandOutput::Text(format!(
            "Plugin {subcmd} requested: {}",
            args[1]
        )))
    }
}

/// Register plugin management commands into a registry.
pub fn register_plugin_commands(reg: &mut crate::terminal::CommandRegistry) {
    reg.register(Box::new(PluginCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{CommandOutput, CommandRegistry, Environment};
    use crate::vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_plugin_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/plugin").unwrap();
        vfs.write(PLUGIN_STATUS_PATH, b"").unwrap();
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

    fn text(out: Result<CommandOutput>) -> String {
        match out.unwrap() {
            CommandOutput::Text(s) => s,
            other => panic!("expected text, got {other:?}"),
        }
    }

    /// Without a host plugin manager every subcommand says so instead of
    /// queuing requests nobody reads.
    #[test]
    fn plugin_without_host_is_honest() {
        let mut reg = CommandRegistry::new();
        register_plugin_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        for line in [
            "plugin list",
            "plugin",
            "plugin load hello",
            "plugin unload x",
        ] {
            let s = text(exec(&reg, &mut vfs, line));
            assert!(s.contains("not available"), "{line}: {s}");
        }
        assert!(!vfs.exists(PLUGIN_REQUEST_PATH));
    }

    #[test]
    fn plugin_list_with_status() {
        let (reg, mut vfs) = setup();
        vfs.write(
            PLUGIN_STATUS_PATH,
            b"hello (1.0.0) [active] Hello\nclock-widget (1.0.0) [active] Clock\n",
        )
        .unwrap();
        let s = text(exec(&reg, &mut vfs, "plugin list"));
        assert!(s.starts_with("Plugins:"), "{s}");
        assert!(s.contains("hello"));
        assert!(s.contains("clock-widget"));
        assert_eq!(
            text(exec(&reg, &mut vfs, "plugin")),
            s,
            "list is the default"
        );
    }

    #[test]
    fn plugin_list_empty() {
        let (reg, mut vfs) = setup();
        assert!(text(exec(&reg, &mut vfs, "plugin list")).contains("no plugins"));
    }

    #[test]
    fn plugin_load_and_unload_queue_requests() {
        let (reg, mut vfs) = setup();
        let s = text(exec(&reg, &mut vfs, "plugin unload hello"));
        assert!(s.contains("hello"), "{s}");
        text(exec(&reg, &mut vfs, "plugin load hello"));
        let data = String::from_utf8(vfs.read(PLUGIN_REQUEST_PATH).unwrap()).unwrap();
        let reqs: Vec<_> = data.lines().filter_map(PluginRequest::parse).collect();
        assert_eq!(
            reqs,
            [
                PluginRequest::Unload("hello".into()),
                PluginRequest::Load("hello".into())
            ]
        );
    }

    #[test]
    fn plugin_load_no_name() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "plugin load").is_err());
        assert!(exec(&reg, &mut vfs, "plugin unload").is_err());
    }

    #[test]
    fn plugin_unknown_subcommand() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "plugin badcmd").is_err());
    }
}
