//! Error types for OASIS_OS.

use std::io;

/// Errors produced by the OASIS_OS framework.
#[derive(Debug, thiserror::Error)]
pub enum OasisError {
    #[error("SDI error: {0}")]
    Sdi(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("VFS error: {0}")]
    Vfs(String),

    #[error("command error: {0}")]
    Command(String),

    #[error("platform error: {0}")]
    Platform(String),

    #[error("window manager error: {0}")]
    Wm(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, OasisError>;

#[cfg(test)]
mod tests {
    use super::*;

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
}
