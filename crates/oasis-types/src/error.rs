//! Error types for OASIS_OS.
//!
//! Each domain has its own structured error enum (e.g. [`SdiError`],
//! [`VfsError`]) so callers can programmatically match on specific
//! failure modes.  Every sub-error includes an `Other(String)` fallback
//! for one-off messages that don't warrant a dedicated variant.

use std::io;

// ---------------------------------------------------------------------------
// Domain-specific error enums
// ---------------------------------------------------------------------------

/// Errors from the Scene Display Interface (SDI) object registry.
#[derive(Debug, thiserror::Error)]
pub enum SdiError {
    /// A named SDI object was not found.
    #[error("object not found: {name}")]
    NotFound { name: String },

    /// Catch-all for SDI errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from backend implementations (rendering, networking, audio).
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The backend has not been initialised yet.
    #[error("not initialized")]
    NotInitialized,

    /// Attempted an operation that requires a connection, but none exists.
    #[error("not connected")]
    NotConnected,

    /// Attempted to accept on a listener that isn't active.
    #[error("not listening")]
    NotListening,

    /// A referenced texture ID does not exist.
    #[error("texture not found: {id}")]
    TextureNotFound { id: u64 },

    /// Catch-all for backend errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from configuration file parsing (TOML skins, agents, MCP).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A TOML configuration file failed to parse.
    #[error("{filename}: {message}")]
    ParseError { filename: String, message: String },

    /// A version string could not be parsed.
    #[error("invalid version in {path}: {text}")]
    InvalidVersion { path: String, text: String },

    /// A required key was missing from the configuration.
    #[error("missing key: {key}")]
    MissingKey { key: String },

    /// Catch-all for config errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from the virtual file system.
#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    /// The requested path does not exist.
    #[error("no such path: {path}")]
    NotFound { path: String },

    /// Expected a file but found a directory.
    #[error("is a directory: {path}")]
    IsDirectory { path: String },

    /// Expected a directory but found a file (or nothing).
    #[error("not a directory: {path}")]
    NotADirectory { path: String },

    /// Attempted to remove a non-empty directory.
    #[error("directory not empty: {path}")]
    DirectoryNotEmpty { path: String },

    /// Attempted to remove the VFS root.
    #[error("cannot remove root")]
    CannotRemoveRoot,

    /// Path traversal outside VFS root detected.
    #[error("path traversal not allowed: {path}")]
    PathTraversal { path: String },

    /// Catch-all for VFS errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from the command interpreter / terminal.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// Incorrect command usage (printed as a hint to the user).
    #[error("usage: {0}")]
    Usage(String),

    /// The command name was not recognised.
    #[error("unknown command: {name}")]
    UnknownCommand { name: String },

    /// A shell syntax error (unterminated quote, missing keyword).
    #[error("{0}")]
    Syntax(String),

    /// Catch-all for command errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from the window manager.
#[derive(Debug, thiserror::Error)]
pub enum WmError {
    /// A window with the given ID was not found.
    #[error("window not found: {id}")]
    WindowNotFound { id: String },

    /// Attempted to create a window whose ID already exists.
    #[error("window already exists: {id}")]
    WindowAlreadyExists { id: String },

    /// Catch-all for window manager errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from the plugin system.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The named plugin was not found in the registry.
    #[error("plugin not found: {name}")]
    NotFound { name: String },

    /// A plugin panicked during a lifecycle phase.
    #[error("{phase} explosion")]
    LifecycleExplosion { phase: String },

    /// Catch-all for plugin errors without a dedicated variant.
    #[error("{0}")]
    Other(String),
}

/// Errors from platform services (power, time, USB, etc.).
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// Catch-all for platform errors.
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// From<String> / From<&str> impls for backward compatibility
// ---------------------------------------------------------------------------

macro_rules! impl_from_string {
    ($ty:ident) => {
        impl From<String> for $ty {
            fn from(s: String) -> Self {
                Self::Other(s)
            }
        }

        impl From<&str> for $ty {
            fn from(s: &str) -> Self {
                Self::Other(s.to_owned())
            }
        }
    };
}

impl_from_string!(SdiError);
impl_from_string!(BackendError);
impl_from_string!(ConfigError);
impl_from_string!(VfsError);
impl_from_string!(CommandError);
impl_from_string!(WmError);
impl_from_string!(PluginError);
impl_from_string!(PlatformError);

// ---------------------------------------------------------------------------
// Top-level error enum
// ---------------------------------------------------------------------------

/// Errors produced by the OASIS_OS framework.
///
/// Each variant wraps a domain-specific error enum that provides structured
/// sub-variants for common failure modes, plus an `Other(String)` fallback.
#[derive(Debug, thiserror::Error)]
pub enum OasisError {
    /// Scene Display Interface error.
    #[error("SDI error: {0}")]
    Sdi(SdiError),

    /// Backend (rendering / networking / audio) error.
    #[error("backend error: {0}")]
    Backend(BackendError),

    /// Configuration file error.
    #[error("config error: {0}")]
    Config(ConfigError),

    /// Virtual file system error.
    #[error("VFS error: {0}")]
    Vfs(VfsError),

    /// Command interpreter error.
    #[error("command error: {0}")]
    Command(CommandError),

    /// Platform service error.
    #[error("platform error: {0}")]
    Platform(PlatformError),

    /// Window manager error.
    #[error("window manager error: {0}")]
    Wm(WmError),

    /// Plugin system error.
    #[error("plugin error: {0}")]
    Plugin(PluginError),

    /// Video pipeline error (demux, decode, no track).
    #[error("video error: {0}")]
    Video(String),

    /// JavaScript engine error.
    #[error("JavaScript error: {0}")]
    JavaScript(String),

    /// Calculator expression error.
    #[error("calc error: {0}")]
    Calc(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// TOML deserialization error.
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// JSON serialization / deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, OasisError>;

// Manual From impls so callers can `?` a domain error into OasisError.

impl From<SdiError> for OasisError {
    fn from(e: SdiError) -> Self {
        Self::Sdi(e)
    }
}

impl From<BackendError> for OasisError {
    fn from(e: BackendError) -> Self {
        Self::Backend(e)
    }
}

impl From<ConfigError> for OasisError {
    fn from(e: ConfigError) -> Self {
        Self::Config(e)
    }
}

impl From<VfsError> for OasisError {
    fn from(e: VfsError) -> Self {
        Self::Vfs(e)
    }
}

impl From<CommandError> for OasisError {
    fn from(e: CommandError) -> Self {
        Self::Command(e)
    }
}

impl From<PlatformError> for OasisError {
    fn from(e: PlatformError) -> Self {
        Self::Platform(e)
    }
}

impl From<WmError> for OasisError {
    fn from(e: WmError) -> Self {
        Self::Wm(e)
    }
}

impl From<PluginError> for OasisError {
    fn from(e: PluginError) -> Self {
        Self::Plugin(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Sub-error From impls -------------------------------------------------

    #[test]
    fn sdi_error_from_string() {
        let e: SdiError = "oops".into();
        assert_eq!(format!("{e}"), "oops");
    }

    #[test]
    fn sdi_error_not_found() {
        let e = SdiError::NotFound { name: "foo".into() };
        assert_eq!(format!("{e}"), "object not found: foo");
    }

    #[test]
    fn backend_error_variants() {
        assert_eq!(
            format!("{}", BackendError::NotInitialized),
            "not initialized"
        );
        assert_eq!(format!("{}", BackendError::NotConnected), "not connected");
        assert_eq!(format!("{}", BackendError::NotListening), "not listening");
        let t = BackendError::TextureNotFound { id: 42 };
        assert_eq!(format!("{t}"), "texture not found: 42");
    }

    #[test]
    fn config_error_parse() {
        let e = ConfigError::ParseError {
            filename: "skin.toml".into(),
            message: "bad key".into(),
        };
        assert_eq!(format!("{e}"), "skin.toml: bad key");
    }

    #[test]
    fn vfs_error_variants() {
        let e = VfsError::NotFound { path: "/a".into() };
        assert_eq!(format!("{e}"), "no such path: /a");
        assert_eq!(
            format!("{}", VfsError::IsDirectory { path: "/b".into() }),
            "is a directory: /b"
        );
        assert_eq!(
            format!("{}", VfsError::CannotRemoveRoot),
            "cannot remove root"
        );
        assert_eq!(
            format!("{}", VfsError::DirectoryNotEmpty { path: "/c".into() }),
            "directory not empty: /c"
        );
    }

    #[test]
    fn command_error_variants() {
        let e = CommandError::Usage("ls [path]".into());
        assert_eq!(format!("{e}"), "usage: ls [path]");
        let e = CommandError::UnknownCommand { name: "foo".into() };
        assert_eq!(format!("{e}"), "unknown command: foo");
    }

    #[test]
    fn wm_error_variants() {
        let e = WmError::WindowNotFound { id: "w1".into() };
        assert_eq!(format!("{e}"), "window not found: w1");
        let e = WmError::WindowAlreadyExists { id: "w2".into() };
        assert_eq!(format!("{e}"), "window already exists: w2");
    }

    #[test]
    fn plugin_error_variants() {
        let e = PluginError::NotFound {
            name: "cool".into(),
        };
        assert_eq!(format!("{e}"), "plugin not found: cool");
        let e = PluginError::LifecycleExplosion {
            phase: "init".into(),
        };
        assert_eq!(format!("{e}"), "init explosion");
    }

    // -- OasisError display ---------------------------------------------------

    #[test]
    fn sdi_error_display() {
        let e = OasisError::Sdi("object not found".into());
        assert_eq!(format!("{e}"), "SDI error: object not found");
    }

    #[test]
    fn backend_error_display() {
        let e = OasisError::Backend("init failed".into());
        assert_eq!(format!("{e}"), "backend error: init failed");
    }

    #[test]
    fn config_error_display() {
        let e = OasisError::Config("missing key".into());
        assert_eq!(format!("{e}"), "config error: missing key");
    }

    #[test]
    fn vfs_error_display() {
        let e = OasisError::Vfs("file not found".into());
        assert_eq!(format!("{e}"), "VFS error: file not found");
    }

    #[test]
    fn command_error_display() {
        let e = OasisError::Command("unknown cmd".into());
        assert_eq!(format!("{e}"), "command error: unknown cmd");
    }

    #[test]
    fn platform_error_display() {
        let e = OasisError::Platform("no battery".into());
        assert_eq!(format!("{e}"), "platform error: no battery");
    }

    #[test]
    fn wm_error_display() {
        let e = OasisError::Wm("window not found".into());
        assert_eq!(format!("{e}"), "window manager error: window not found");
    }

    #[test]
    fn plugin_error_display() {
        let e = OasisError::Plugin("load failed".into());
        assert_eq!(format!("{e}"), "plugin error: load failed");
    }

    #[test]
    fn io_error_from_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "gone");
        let e: OasisError = io_err.into();
        let msg = format!("{e}");
        assert!(msg.contains("I/O error"));
        assert!(msg.contains("gone"));
    }

    #[test]
    fn toml_error_from_conversion() {
        let bad_toml = "this is [[[not valid toml";
        let toml_err = toml::from_str::<toml::Value>(bad_toml).unwrap_err();
        let e: OasisError = toml_err.into();
        let msg = format!("{e}");
        assert!(msg.contains("TOML parse error"));
    }

    #[test]
    fn json_error_from_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let e: OasisError = json_err.into();
        let msg = format!("{e}");
        assert!(msg.contains("JSON error"));
    }

    #[test]
    fn error_is_debug() {
        let e = OasisError::Sdi("test".into());
        let dbg = format!("{e:?}");
        assert!(dbg.contains("Sdi"));
    }

    #[test]
    fn result_alias_ok() {
        let r: Result<i32> = Ok(42);
        assert_eq!(r.unwrap(), 42);
    }

    #[test]
    fn result_alias_err() {
        let r: Result<i32> = Err(OasisError::Vfs("oops".into()));
        assert!(r.is_err());
    }

    // -- Structured error -> OasisError conversions ---------------------------

    #[test]
    fn sdi_not_found_into_oasis() {
        let e: OasisError = SdiError::NotFound { name: "bar".into() }.into();
        assert!(format!("{e}").contains("object not found: bar"));
    }

    #[test]
    fn vfs_not_found_into_oasis() {
        let e: OasisError = VfsError::NotFound { path: "/x".into() }.into();
        assert!(format!("{e}").contains("no such path: /x"));
    }

    #[test]
    fn wm_not_found_into_oasis() {
        let e: OasisError = WmError::WindowNotFound { id: "w1".into() }.into();
        assert!(format!("{e}").contains("window not found: w1"));
    }

    #[test]
    fn backend_not_connected_into_oasis() {
        let e: OasisError = BackendError::NotConnected.into();
        assert!(format!("{e}").contains("not connected"));
    }

    #[test]
    fn command_usage_into_oasis() {
        let e: OasisError = CommandError::Usage("ls".into()).into();
        assert!(format!("{e}").contains("usage: ls"));
    }

    // -- Pattern matching on structured errors --------------------------------

    #[test]
    fn match_vfs_not_found() {
        let e = OasisError::Vfs(VfsError::NotFound {
            path: "/tmp".into(),
        });
        match e {
            OasisError::Vfs(VfsError::NotFound { ref path }) => {
                assert_eq!(path, "/tmp");
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn match_backend_not_connected() {
        let e = OasisError::Backend(BackendError::NotConnected);
        match e {
            OasisError::Backend(BackendError::NotConnected) => {},
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn match_wm_window_not_found() {
        let e = OasisError::Wm(WmError::WindowNotFound { id: "main".into() });
        match e {
            OasisError::Wm(WmError::WindowNotFound { ref id }) => {
                assert_eq!(id, "main");
            },
            _ => panic!("wrong variant"),
        }
    }
}
