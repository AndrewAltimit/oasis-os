//! Core types for the command interpreter.
//!
//! Contains [`CommandOutput`], [`Environment`], the [`Command`] trait,
//! [`ShellFunction`], and shell constants.

use oasis_platform::{NetworkService, PowerService, TimeService, UsbService};
use oasis_types::error::Result;
use oasis_vfs::Vfs;

/// Output produced by a command.
#[derive(Debug, Clone)]
pub enum CommandOutput {
    /// Plain text lines.
    Text(String),
    /// Tabular data (header row + data rows).
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Command produced no visible output.
    None,
    /// Signal to clear the terminal output buffer.
    Clear,
    /// Signal to the app to start/stop the remote terminal listener.
    ListenToggle {
        /// Port to listen on (0 = stop).
        port: u16,
    },
    /// Signal to the app to connect to a remote host.
    RemoteConnect {
        address: String,
        port: u16,
        psk: Option<String>,
    },
    /// Signal to the app to toggle browser sandbox mode.
    BrowserSandbox {
        /// `true` = sandbox on (VFS only), `false` = networking enabled.
        enable: bool,
    },
    /// Signal to the app to swap the active skin.
    SkinSwap {
        /// Skin name or path to load.
        name: String,
    },
    /// Signal to the app to start/stop the FTP file server.
    FtpToggle {
        /// Port to listen on (0 = stop).
        port: u16,
        /// Optional password for FTP authentication.
        password: Option<String>,
    },
    /// Multiple outputs from a chained command (e.g. `skin xp ; echo Done`).
    /// Each inner output is processed in order by the app layer.
    Multi(Vec<CommandOutput>),
}

/// Shared mutable environment passed to every command.
pub struct Environment<'a> {
    /// Current working directory (VFS path).
    pub cwd: String,
    /// The virtual file system.
    pub vfs: &'a mut dyn Vfs,
    /// Power service for battery/CPU queries.
    pub power: Option<&'a dyn PowerService>,
    /// Time service for clock/uptime queries.
    pub time: Option<&'a dyn TimeService>,
    /// USB service for status queries.
    pub usb: Option<&'a dyn UsbService>,
    /// Network service for WiFi status queries.
    pub network: Option<&'a dyn NetworkService>,
    /// TLS provider for HTTPS connections.
    pub tls: Option<&'a dyn oasis_net::tls::TlsProvider>,
    /// Piped input from a previous command in a pipeline.
    pub stdin: Option<String>,
    /// Accumulated stderr output from the most recent command.
    /// Commands append error messages here. Cleared before each command.
    pub stderr: String,
}

/// A single executable command.
pub trait Command {
    /// The command name (what the user types).
    fn name(&self) -> &str;

    /// One-line description for `help`.
    fn description(&self) -> &str;

    /// Usage string (e.g. "ls \[path\]").
    fn usage(&self) -> &str;

    /// Command category for grouping in `help` output.
    fn category(&self) -> &str {
        "general"
    }

    /// Execute the command with the given arguments and environment.
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput>;
}

/// Maximum number of history entries to retain.
pub(crate) const MAX_HISTORY: usize = 100;

/// Maximum shell function call depth (prevents infinite recursion).
pub(crate) const MAX_CALL_DEPTH: usize = 64;

/// A user-defined shell function.
#[derive(Clone, Debug)]
pub(crate) struct ShellFunction {
    /// Function body lines (semicolon-separated or newline-separated).
    pub(crate) body: String,
}
