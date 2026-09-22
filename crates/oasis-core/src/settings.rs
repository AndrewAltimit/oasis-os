//! Persistent key-value settings store backed by VFS.
//!
//! Settings are stored as a TOML file at a configurable path (default
//! `/system/settings.toml`). The store provides typed get/set access
//! for strings, integers, floats, and booleans.
//!
//! [`UserPrefs`] is the typed view over the user preferences the Settings
//! app manages (skin, resolution, volume, locale, font scale, reduced
//! motion). Hosts restore it at boot and write it back whenever a Settings
//! IPC request changes one of them.

use oasis_skin::SkinFeatures;
use oasis_skin::active_theme::ActiveTheme;
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
            self.load_from_str(text);
        }
    }

    /// Replace the in-memory entries with the contents of a settings
    /// document (the format written by [`Self::to_toml_string`]). Used by
    /// hosts that mirror the settings file to real storage.
    pub fn load_from_str(&mut self, text: &str) {
        self.parse_toml(text);
        self.dirty = false;
    }

    /// Serialize the settings to the on-disk document format.
    pub fn to_toml_string(&self) -> String {
        self.to_toml()
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

/// Settings-store keys of the user preferences managed by the Settings app.
pub mod pref_keys {
    /// Active skin name (string).
    pub const SKIN: &str = "prefs.skin";
    /// Virtual resolution as `"WIDTHxHEIGHT"` (string).
    pub const RESOLUTION: &str = "prefs.resolution";
    /// Master volume 0-100 (int).
    pub const VOLUME: &str = "prefs.volume";
    /// Selected UI locale code, e.g. `"de"` (string).
    pub const LOCALE: &str = "prefs.locale";
    /// Font scale multiplier (float).
    pub const FONT_SCALE: &str = "prefs.font_scale";
    /// Reduced-motion accessibility switch (bool).
    pub const REDUCED_MOTION: &str = "prefs.reduced_motion";
    /// Skin to return to when the high-contrast shortcut is switched off.
    pub const SKIN_BEFORE_HIGH_CONTRAST: &str = "prefs.skin_before_high_contrast";
}

/// Default master volume (0-100) when none is persisted.
pub const DEFAULT_VOLUME: u8 = 80;

/// Smallest user-selectable font scale.
pub const FONT_SCALE_MIN: f32 = 0.75;

/// Largest user-selectable font scale. Kept modest so fixed-height chrome
/// (bars, title bars) still fits the scaled text.
pub const FONT_SCALE_MAX: f32 = 1.5;

/// Typed view over the user preferences persisted in a [`SettingsStore`].
///
/// Missing or malformed entries fall back to the defaults, so a fresh (or
/// hand-edited) settings file never prevents boot.
#[derive(Debug, Clone, PartialEq)]
pub struct UserPrefs {
    /// Skin chosen by the user (`None` = host default / CLI).
    pub skin: Option<String>,
    /// Resolution chosen by the user (`None` = skin default).
    pub resolution: Option<(u32, u32)>,
    /// Master volume, 0-100.
    pub volume: u8,
    /// Selected locale code (`"en"`, `"de"`, ...).
    pub locale: String,
    /// Font scale multiplier, clamped to
    /// [`FONT_SCALE_MIN`]..=[`FONT_SCALE_MAX`].
    pub font_scale: f32,
    /// Force reduced motion regardless of the skin's own setting.
    pub reduced_motion: bool,
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            skin: None,
            resolution: None,
            volume: DEFAULT_VOLUME,
            locale: "en".to_string(),
            font_scale: 1.0,
            reduced_motion: false,
        }
    }
}

impl UserPrefs {
    /// Read the preferences from a settings store.
    pub fn from_store(store: &SettingsStore) -> Self {
        let d = Self::default();
        Self {
            skin: store
                .get_string(pref_keys::SKIN)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string),
            resolution: store
                .get_string(pref_keys::RESOLUTION)
                .and_then(oasis_app_settings::parse_resolution)
                .filter(|(w, h)| *w > 0 && *h > 0),
            volume: store
                .get_int(pref_keys::VOLUME)
                .map_or(d.volume, |v| v.clamp(0, 100) as u8),
            locale: store
                .get_string(pref_keys::LOCALE)
                .and_then(oasis_i18n::Locale::from_code)
                .map_or(d.locale, |l| l.code().to_string()),
            font_scale: store
                .get_float(pref_keys::FONT_SCALE)
                .map_or(d.font_scale, |f| clamp_font_scale(f as f32)),
            reduced_motion: store
                .get_bool(pref_keys::REDUCED_MOTION)
                .unwrap_or(d.reduced_motion),
        }
    }

    /// Write every preference into a settings store (leaving unrelated
    /// keys such as icon positions untouched).
    pub fn write_to(&self, store: &mut SettingsStore) {
        match &self.skin {
            Some(skin) => store.set_string(pref_keys::SKIN, skin.clone()),
            None => {
                store.remove(pref_keys::SKIN);
            },
        }
        match self.resolution {
            Some((w, h)) => store.set_string(pref_keys::RESOLUTION, format!("{w}x{h}")),
            None => {
                store.remove(pref_keys::RESOLUTION);
            },
        }
        store.set_int(pref_keys::VOLUME, i64::from(self.volume.min(100)));
        store.set_string(pref_keys::LOCALE, self.locale.clone());
        store.set_float(
            pref_keys::FONT_SCALE,
            f64::from(clamp_font_scale(self.font_scale)),
        );
        store.set_bool(pref_keys::REDUCED_MOTION, self.reduced_motion);
    }

    /// The selected locale (English when the stored code is unknown).
    pub fn locale(&self) -> oasis_i18n::Locale {
        oasis_i18n::Locale::from_code(&self.locale).unwrap_or(oasis_i18n::Locale::English)
    }

    /// OR the user's reduced-motion preference into a skin's features so
    /// [`ActiveTheme::with_features`] and the window manager honour it.
    pub fn patch_features(&self, features: &mut SkinFeatures) {
        if self.reduced_motion {
            features.reduced_motion = true;
        }
    }

    /// Apply the font scale to a freshly derived theme.
    ///
    /// Scales the content font sizes (`font_body`, `font_hint`,
    /// `font_heading`) and the content line height, and records the factor
    /// in `font_scale` / `ui_theme.font_scale` for widgets that scale
    /// themselves. Chrome fonts (`font_small`) stay fixed so bars keep
    /// fitting. Must be called on an unscaled theme (every host rebuild
    /// starts from the skin), otherwise the factor compounds.
    pub fn apply_font_scale(&self, at: &mut ActiveTheme) {
        let s = clamp_font_scale(self.font_scale);
        at.font_scale = s;
        at.ui_theme.font_scale = s;
        if (s - 1.0).abs() < f32::EPSILON {
            return;
        }
        let scale = |v: u16| ((f32::from(v) * s).round() as u16).max(1);
        at.font_body = scale(at.font_body);
        at.font_hint = scale(at.font_hint);
        at.font_heading = scale(at.font_heading);
        at.terminal_line_height = ((at.terminal_line_height as f32 * s).round() as u32).max(1);
    }
}

/// Clamp a font scale to the user-selectable range (NaN maps to 1.0).
pub fn clamp_font_scale(f: f32) -> f32 {
    if f.is_nan() {
        1.0
    } else {
        f.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX)
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
        s.set_string("skin", "balatro");
        assert_eq!(s.get_string("skin"), Some("balatro"));
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
    fn user_prefs_defaults_from_empty_store() {
        let prefs = UserPrefs::from_store(&SettingsStore::new());
        assert_eq!(prefs, UserPrefs::default());
        assert_eq!(prefs.locale(), oasis_i18n::Locale::English);
    }

    #[test]
    fn user_prefs_persist_and_reload_through_vfs() {
        let mut vfs = MemoryVfs::new();
        let mut store = SettingsStore::new();
        // Unrelated keys (icon positions) must survive a prefs write.
        store.set_string("icon_positions.classic./apps/a", "1,2");
        let prefs = UserPrefs {
            skin: Some("paper".to_string()),
            resolution: Some((1280, 720)),
            volume: 35,
            locale: "de".to_string(),
            font_scale: 1.25,
            reduced_motion: true,
        };
        prefs.write_to(&mut store);
        store.save(&mut vfs);

        let mut reloaded = SettingsStore::new();
        reloaded.load(&vfs);
        assert_eq!(UserPrefs::from_store(&reloaded), prefs);
        assert_eq!(
            reloaded.get_string("icon_positions.classic./apps/a"),
            Some("1,2")
        );
    }

    #[test]
    fn user_prefs_sanitize_malformed_values() {
        let mut store = SettingsStore::new();
        store.load_from_str(
            "prefs.volume = 400\nprefs.locale = \"xx\"\nprefs.font_scale = 9.0\n\
             prefs.resolution = \"junk\"\nprefs.skin = \"\"\n",
        );
        let prefs = UserPrefs::from_store(&store);
        assert_eq!(prefs.volume, 100);
        assert_eq!(prefs.locale, "en");
        assert_eq!(prefs.font_scale, FONT_SCALE_MAX);
        assert_eq!(prefs.resolution, None);
        assert_eq!(prefs.skin, None);
    }

    #[test]
    fn user_prefs_font_scale_scales_content_fonts_only() {
        let base = ActiveTheme::default();
        let mut at = base.clone();
        let prefs = UserPrefs {
            font_scale: 1.5,
            ..UserPrefs::default()
        };
        prefs.apply_font_scale(&mut at);
        assert_eq!(at.font_scale, 1.5);
        assert_eq!(at.ui_theme.font_scale, 1.5);
        assert!(at.font_body > base.font_body);
        assert!(at.terminal_line_height > base.terminal_line_height);
        assert_eq!(at.font_small, base.font_small);
    }

    #[test]
    fn user_prefs_reduced_motion_patches_features() {
        let mut features = SkinFeatures::default();
        UserPrefs::default().patch_features(&mut features);
        assert!(!features.reduced_motion);
        let prefs = UserPrefs {
            reduced_motion: true,
            ..UserPrefs::default()
        };
        prefs.patch_features(&mut features);
        assert!(features.reduced_motion);
    }

    #[test]
    fn toml_string_roundtrip() {
        let mut s = SettingsStore::new();
        s.set_int("prefs.volume", 55);
        let text = s.to_toml_string();
        let mut s2 = SettingsStore::new();
        s2.load_from_str(&text);
        assert_eq!(s2.get_int("prefs.volume"), Some(55));
        assert!(!s2.is_dirty());
    }

    #[test]
    fn overwrite_value() {
        let mut s = SettingsStore::new();
        s.set_int("volume", 50);
        s.set_int("volume", 100);
        assert_eq!(s.get_int("volume"), Some(100));
    }
}
