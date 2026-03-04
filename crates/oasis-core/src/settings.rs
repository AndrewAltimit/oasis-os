//! Persistent key-value settings store backed by VFS.
//!
//! Settings are stored as a TOML file at a configurable path (default
//! `/system/settings.toml`). The store provides typed get/set access
//! for strings, integers, floats, and booleans.

use oasis_vfs::Vfs;
use std::collections::BTreeMap;

/// Default VFS path for the settings file.
pub const DEFAULT_PATH: &str = "/system/settings.toml";

/// A persistent key-value settings store.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    /// The VFS path to save/load from.
    pub path: String,
    /// In-memory key-value pairs.
    entries: BTreeMap<String, SettingsValue>,
    /// Whether in-memory state differs from the persisted file.
    dirty: bool,
}

/// A typed settings value.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValue {
    /// String value.
    String(String),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Bool(bool),
}

impl SettingsStore {
    /// Create a new empty settings store.
    pub fn new() -> Self {
        Self {
            path: DEFAULT_PATH.to_string(),
            entries: BTreeMap::new(),
            dirty: false,
        }
    }

    /// Load settings from the VFS. Silently ignores missing files.
    pub fn load(&mut self, vfs: &dyn Vfs) {
        if let Ok(data) = vfs.read(&self.path)
            && let Ok(text) = std::str::from_utf8(&data)
        {
            self.parse_toml(text);
            self.dirty = false;
        }
    }

    /// Save settings to the VFS.
    pub fn save(&mut self, vfs: &mut dyn Vfs) {
        let toml = self.to_toml();
        // Ensure parent directory exists.
        if let Some(parent) = self.path.rsplit_once('/')
            && let Err(e) = vfs.mkdir(parent.0)
        {
            log::warn!("settings mkdir({}) failed: {e}", parent.0);
        }
        if let Err(e) = vfs.write(&self.path, toml.as_bytes()) {
            log::warn!("settings save({}) failed: {e}", self.path);
        }
        self.dirty = false;
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Get a string value.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.entries.get(key) {
            Some(SettingsValue::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Get an integer value.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.entries.get(key) {
            Some(SettingsValue::Int(n)) => Some(*n),
            _ => None,
        }
    }

    /// Get a float value.
    pub fn get_float(&self, key: &str) -> Option<f64> {
        match self.entries.get(key) {
            Some(SettingsValue::Float(f)) => Some(*f),
            _ => None,
        }
    }

    /// Get a boolean value.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.entries.get(key) {
            Some(SettingsValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    /// Set a string value.
    pub fn set_string(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries
            .insert(key.into(), SettingsValue::String(value.into()));
        self.dirty = true;
    }

    /// Set an integer value.
    pub fn set_int(&mut self, key: impl Into<String>, value: i64) {
        self.entries.insert(key.into(), SettingsValue::Int(value));
        self.dirty = true;
    }

    /// Set a float value.
    pub fn set_float(&mut self, key: impl Into<String>, value: f64) {
        self.entries.insert(key.into(), SettingsValue::Float(value));
        self.dirty = true;
    }

    /// Set a boolean value.
    pub fn set_bool(&mut self, key: impl Into<String>, value: bool) {
        self.entries.insert(key.into(), SettingsValue::Bool(value));
        self.dirty = true;
    }

    /// Remove a key.
    pub fn remove(&mut self, key: &str) -> bool {
        let removed = self.entries.remove(key).is_some();
        if removed {
            self.dirty = true;
        }
        removed
    }

    /// List all keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Parse a TOML string into settings.
    fn parse_toml(&mut self, text: &str) {
        self.entries.clear();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim();
                let parsed = if value == "true" {
                    SettingsValue::Bool(true)
                } else if value == "false" {
                    SettingsValue::Bool(false)
                } else if let Some(s) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    // Unescape TOML basic string.
                    SettingsValue::String(
                        s.replace("\\\\", "\x00")
                            .replace("\\\"", "\"")
                            .replace("\\n", "\n")
                            .replace("\\r", "\r")
                            .replace('\x00', "\\"),
                    )
                } else if let Ok(n) = value.parse::<i64>() {
                    SettingsValue::Int(n)
                } else if let Ok(f) = value.parse::<f64>() {
                    SettingsValue::Float(f)
                } else {
                    SettingsValue::String(value.to_string())
                };
                self.entries.insert(key, parsed);
            }
        }
    }

    /// Serialize settings to TOML string.
    fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# OASIS OS Settings\n\n");
        for (key, value) in &self.entries {
            match value {
                SettingsValue::String(s) => {
                    // Escape backslashes first, then quotes, then newlines.
                    let escaped = s
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r");
                    out.push_str(&format!("{key} = \"{escaped}\"\n"));
                },
                SettingsValue::Int(n) => {
                    out.push_str(&format!("{key} = {n}\n"));
                },
                SettingsValue::Float(f) => {
                    let s = format!("{f}");
                    if s.contains('.') {
                        out.push_str(&format!("{key} = {s}\n"));
                    } else {
                        out.push_str(&format!("{key} = {s}.0\n"));
                    }
                },
                SettingsValue::Bool(b) => {
                    out.push_str(&format!("{key} = {b}\n"));
                },
            }
        }
        out
    }
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn new_is_empty() {
        let s = SettingsStore::new();
        assert!(!s.is_dirty());
        assert_eq!(s.keys().count(), 0);
    }

    #[test]
    fn set_and_get_string() {
        let mut s = SettingsStore::new();
        s.set_string("skin", "cyberpunk");
        assert_eq!(s.get_string("skin"), Some("cyberpunk"));
        assert!(s.is_dirty());
    }

    #[test]
    fn set_and_get_int() {
        let mut s = SettingsStore::new();
        s.set_int("volume", 80);
        assert_eq!(s.get_int("volume"), Some(80));
    }

    #[test]
    fn set_and_get_float() {
        let mut s = SettingsStore::new();
        s.set_float("font_scale", 1.5);
        assert_eq!(s.get_float("font_scale"), Some(1.5));
    }

    #[test]
    fn set_and_get_bool() {
        let mut s = SettingsStore::new();
        s.set_bool("dark_mode", true);
        assert_eq!(s.get_bool("dark_mode"), Some(true));
    }

    #[test]
    fn get_wrong_type_returns_none() {
        let mut s = SettingsStore::new();
        s.set_string("key", "value");
        assert_eq!(s.get_int("key"), None);
        assert_eq!(s.get_bool("key"), None);
    }

    #[test]
    fn remove_key() {
        let mut s = SettingsStore::new();
        s.set_string("key", "value");
        assert!(s.remove("key"));
        assert_eq!(s.get_string("key"), None);
        assert!(!s.remove("nonexistent"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let mut vfs = MemoryVfs::new();
        let mut s = SettingsStore::new();
        s.set_string("skin", "retro");
        s.set_int("volume", 75);
        s.set_bool("fullscreen", true);
        s.set_float("scale", 2.0);
        s.save(&mut vfs);
        assert!(!s.is_dirty());

        let mut s2 = SettingsStore::new();
        s2.load(&vfs);
        assert_eq!(s2.get_string("skin"), Some("retro"));
        assert_eq!(s2.get_int("volume"), Some(75));
        assert_eq!(s2.get_bool("fullscreen"), Some(true));
        assert_eq!(s2.get_float("scale"), Some(2.0));
    }

    #[test]
    fn load_missing_file_no_panic() {
        let vfs = MemoryVfs::new();
        let mut s = SettingsStore::new();
        s.load(&vfs);
        assert_eq!(s.keys().count(), 0);
    }

    #[test]
    fn parse_toml_comments_and_blanks() {
        let mut s = SettingsStore::new();
        s.parse_toml("# comment\n\nkey = \"value\"\n");
        assert_eq!(s.get_string("key"), Some("value"));
    }

    #[test]
    fn keys_sorted() {
        let mut s = SettingsStore::new();
        s.set_string("z", "last");
        s.set_string("a", "first");
        let keys: Vec<&str> = s.keys().collect();
        assert_eq!(keys, vec!["a", "z"]);
    }

    #[test]
    fn roundtrip_special_chars() {
        let mut vfs = MemoryVfs::new();
        let mut s = SettingsStore::new();
        s.set_string("path", r#"C:\Users\test"#);
        s.set_string("quoted", r#"He said "hello""#);
        s.set_string("both", r#"a\"b"#);
        s.save(&mut vfs);

        let mut s2 = SettingsStore::new();
        s2.load(&vfs);
        assert_eq!(s2.get_string("path"), Some(r#"C:\Users\test"#));
        assert_eq!(s2.get_string("quoted"), Some(r#"He said "hello""#));
        assert_eq!(s2.get_string("both"), Some(r#"a\"b"#));
    }

    #[test]
    fn roundtrip_newlines() {
        let mut vfs = MemoryVfs::new();
        let mut s = SettingsStore::new();
        s.set_string("multi", "line1\nline2\nline3");
        s.set_string("cr", "a\rb");
        s.set_string("crlf", "hello\r\nworld");
        s.set_string("mixed", "path\\with\nnewline");
        s.save(&mut vfs);

        let mut s2 = SettingsStore::new();
        s2.load(&vfs);
        assert_eq!(s2.get_string("multi"), Some("line1\nline2\nline3"));
        assert_eq!(s2.get_string("cr"), Some("a\rb"));
        assert_eq!(s2.get_string("crlf"), Some("hello\r\nworld"));
        assert_eq!(s2.get_string("mixed"), Some("path\\with\nnewline"));
    }

    #[test]
    fn overwrite_value() {
        let mut s = SettingsStore::new();
        s.set_int("volume", 50);
        s.set_int("volume", 100);
        assert_eq!(s.get_int("volume"), Some(100));
    }
}
