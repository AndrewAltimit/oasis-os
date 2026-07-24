//! Core types for the command interpreter.
//!
//! Contains [`CommandOutput`], [`Environment`], the [`Command`] trait,
//! [`ShellFunction`], and shell constants.

use oasis_platform::{NetworkService, PowerService, TimeService, UsbService};
use oasis_types::error::Result;
use oasis_vfs::Vfs;

/// Signals sent from commands to the app layer.
///
/// These represent side-effects (connect to a host, swap a skin, etc.)
/// rather than display output. Separated from [`CommandOutput`] so data
/// and control flow are not mixed in the same enum.
#[derive(Debug, Clone)]
pub enum CommandSignal {
    /// Start/stop the remote terminal listener.
    ListenToggle {
        /// Port to listen on (0 = stop).
        port: u16,
    },
    /// Connect to a remote host.
    RemoteConnect {
        address: String,
        port: u16,
        psk: Option<String>,
    },
    /// Toggle browser sandbox mode.
    BrowserSandbox {
        /// `true` = sandbox on (VFS only), `false` = networking enabled.
        enable: bool,
    },
    /// Swap the active skin.
    SkinSwap {
        /// Skin name or path to load.
        name: String,
    },
    /// Start/stop the FTP file server.
    FtpToggle {
        /// Port to listen on (0 = stop).
        port: u16,
        /// Optional password for FTP authentication.
        password: Option<String>,
    },
    /// Start/stop the optional MCP control server (handled by the app layer
    /// only when built with the `mcp` feature).
    McpToggle {
        /// `true` to start, `false` to stop.
        start: bool,
        /// Loopback port to listen on when starting.
        port: u16,
        /// Optional bearer token required on every request.
        token: Option<String>,
    },
}

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
    /// A signal to the app layer (network, skin, etc.).
    Signal(CommandSignal),
    /// Multiple outputs from a chained command (e.g. `skin xp ; echo Done`).
    /// Each inner output is processed in order by the app layer.
    Multi(Vec<CommandOutput>),
}

// Convenience constructors matching the old flat variant names.
// These keep the diff smaller for callers that construct signals.
impl CommandOutput {
    /// Shorthand for `Signal(CommandSignal::ListenToggle { port })`.
    pub fn listen_toggle(port: u16) -> Self {
        Self::Signal(CommandSignal::ListenToggle { port })
    }

    /// Shorthand for `Signal(CommandSignal::RemoteConnect { .. })`.
    pub fn remote_connect(address: String, port: u16, psk: Option<String>) -> Self {
        Self::Signal(CommandSignal::RemoteConnect { address, port, psk })
    }

    /// Shorthand for `Signal(CommandSignal::BrowserSandbox { enable })`.
    pub fn browser_sandbox(enable: bool) -> Self {
        Self::Signal(CommandSignal::BrowserSandbox { enable })
    }

    /// Shorthand for `Signal(CommandSignal::SkinSwap { name })`.
    pub fn skin_swap(name: String) -> Self {
        Self::Signal(CommandSignal::SkinSwap { name })
    }

    /// Shorthand for `Signal(CommandSignal::FtpToggle { port, password })`.
    pub fn ftp_toggle(port: u16, password: Option<String>) -> Self {
        Self::Signal(CommandSignal::FtpToggle { port, password })
    }

    /// Shorthand for `Signal(CommandSignal::McpToggle { .. })`.
    pub fn mcp_toggle(start: bool, port: u16, token: Option<String>) -> Self {
        Self::Signal(CommandSignal::McpToggle { start, port, token })
    }
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
