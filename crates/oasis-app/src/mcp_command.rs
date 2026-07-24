//! `mcp-server` terminal command: start/stop the optional MCP control server.
//!
//! Emits [`CommandSignal::McpToggle`](oasis_core::terminal::CommandSignal),
//! which the app layer (`process_command_output`) turns into an actual
//! loopback listener. Registered only when built with the `mcp` feature.

use oasis_core::error::{OasisError, Result};
use oasis_core::terminal::{Command, CommandOutput, CommandRegistry, Environment};

/// The `mcp-server` command.
pub struct McpServerCmd;

impl Command for McpServerCmd {
    fn name(&self) -> &str {
        "mcp-server"
    }

    fn description(&self) -> &str {
        "Start/stop the MCP control server for local agents"
    }

    fn usage(&self) -> &str {
        "mcp-server start [port] [--token TOKEN] | mcp-server stop"
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        match args.first().copied() {
            Some("start") => {
                let mut port: u16 = 7345;
                let mut token: Option<String> = None;
                let mut i = 1;
                while i < args.len() {
                    match args.get(i).copied() {
                        Some("--token") => {
                            token = args.get(i + 1).map(|s| (*s).to_string());
                            i += 2;
                        },
                        Some(p) => {
                            if let Ok(n) = p.parse::<u16>() {
                                port = n;
                            }
                            i += 1;
                        },
                        None => break,
                    }
                }
                Ok(CommandOutput::mcp_toggle(true, port, token))
            },
            Some("stop") => Ok(CommandOutput::mcp_toggle(false, 0, None)),
            Some("status") | None => Ok(CommandOutput::Text(
                "usage: mcp-server start [port] [--token TOKEN] | mcp-server stop".to_string(),
            )),
            Some(other) => Err(OasisError::Command(
                format!("unknown subcommand: {other}").into(),
            )),
        }
    }
}

/// Register the `mcp-server` command.
pub fn register(reg: &mut CommandRegistry) {
    reg.register(Box::new(McpServerCmd));
}
