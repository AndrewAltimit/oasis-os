//! Internationalization framework for OASIS_OS.
//!
//! Provides compile-time embedded translations with a `tr!()` macro for
//! localized string lookup. Supports interpolation and falls back to English
//! when a translation key is missing.
//!
//! # Usage
//!
//! ```
//! use oasis_i18n::{tr, set_locale, Locale};
//!
//! // Default locale is English
//! assert_eq!(tr!("ui.ok"), "OK");
//!
//! // Switch to Japanese
//! set_locale(Locale::Japanese);
//! assert_eq!(tr!("ui.cancel"), "キャンセル");
//!
//! // Interpolation
//! set_locale(Locale::English);
//! assert_eq!(tr!("greeting.hello", name = "World"), "Hello, World!");
//!
//! // Reset for other tests
//! set_locale(Locale::English);
//! ```

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Embedded translation data (compiled into the binary)
// ---------------------------------------------------------------------------

const EN_TOML: &str = include_str!("../translations/en.toml");
const JA_TOML: &str = include_str!("../translations/ja.toml");

// ---------------------------------------------------------------------------
// Locale enum
// ---------------------------------------------------------------------------

/// Supported locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Locale {
    /// English (default fallback language).
    English = 0,
    /// Japanese.
    Japanese = 1,
}

impl Locale {
    /// Return the locale code string (e.g. `"en"`, `"ja"`).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
        }
    }

    /// Return the human-readable name of the locale.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }

    /// List all supported locales.
    #[must_use]
    pub fn all() -> &'static [Locale] {
        &[Self::English, Self::Japanese]
    }

    /// Parse a locale from a code string. Returns `None` if unrecognized.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" | "EN" | "english" | "English" => Some(Self::English),
            "ja" | "JA" | "jp" | "JP" | "japanese" | "Japanese" => Some(Self::Japanese),
            _ => None,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Japanese,
            _ => Self::English,
        }
    }
}

// ---------------------------------------------------------------------------
// Global locale state
// ---------------------------------------------------------------------------

static CURRENT_LOCALE: AtomicU8 = AtomicU8::new(Locale::English as u8);

/// Set the active locale. Thread-safe.
pub fn set_locale(locale: Locale) {
    CURRENT_LOCALE.store(locale as u8, Ordering::Relaxed);
}

/// Get the active locale. Thread-safe.
#[must_use]
pub fn get_locale() -> Locale {
    Locale::from_u8(CURRENT_LOCALE.load(Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// Translation catalog
// ---------------------------------------------------------------------------

/// Flat key-value map for a single locale (e.g. `"ui.ok" -> "OK"`).
type TranslationMap = HashMap<String, String>;

/// TOML structure: each section is a table of key = "value".
#[derive(Deserialize)]
#[serde(transparent)]
struct TomlRoot {
    sections: HashMap<String, HashMap<String, String>>,
}

/// Flatten a TOML file into dot-separated keys.
fn parse_translations(toml_str: &str) -> TranslationMap {
    let root: TomlRoot = toml::from_str(toml_str).unwrap_or_else(|e| {
        panic!("oasis-i18n: failed to parse translation TOML: {e}");
    });
    let mut map = TranslationMap::new();
    for (section, entries) in &root.sections {
        for (key, value) in entries {
            map.insert(format!("{section}.{key}"), value.clone());
        }
    }
    map
}

struct Catalog {
    locales: HashMap<Locale, TranslationMap>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let mut locales = HashMap::new();
    locales.insert(Locale::English, parse_translations(EN_TOML));
    locales.insert(Locale::Japanese, parse_translations(JA_TOML));
    Catalog { locales }
});

// ---------------------------------------------------------------------------
// Public lookup API
// ---------------------------------------------------------------------------

/// Look up a translation key for a specific locale.
///
/// Falls back to English if the key is not found in the given locale.
/// Returns the key itself if not found in any locale.
#[must_use]
pub fn translate_for(key: &str, locale: Locale) -> &str {
    let catalog = &*CATALOG;

    // Try requested locale first
    if let Some(map) = catalog.locales.get(&locale)
        && let Some(value) = map.get(key)
    {
        return value;
    }

    // Fallback to English
    if locale != Locale::English
        && let Some(map) = catalog.locales.get(&Locale::English)
        && let Some(value) = map.get(key)
    {
        return value;
    }

    // Key not found anywhere -- return key as-is
    key
}

/// Look up a translation key for the current locale.
///
/// Falls back to English if the key is not found in the active locale.
/// Returns the key itself if not found in any locale.
#[must_use]
pub fn translate(key: &str) -> &str {
    translate_for(key, get_locale())
}

/// Look up a translation key for a specific locale and perform `{name}`
/// interpolation.
///
/// Falls back to English if the key is not found in the given locale.
/// Returns the key itself if not found in any locale.
#[must_use]
pub fn translate_with_for(key: &str, args: &[(&str, &str)], locale: Locale) -> String {
    let template = translate_for(key, locale);
    let mut result = template.to_owned();
    for &(name, value) in args {
        let placeholder = format!("{{{name}}}");
        result = result.replace(&placeholder, value);
    }
    result
}

/// Look up a translation key and perform `{name}` interpolation.
///
/// Falls back to English if the key is not found in the active locale.
/// Returns the key itself if not found in any locale.
#[must_use]
pub fn translate_with(key: &str, args: &[(&str, &str)]) -> String {
    translate_with_for(key, args, get_locale())
}

// ---------------------------------------------------------------------------
// tr!() macro
// ---------------------------------------------------------------------------

/// Translate a string key using the current locale.
///
/// # Simple lookup
/// ```
/// use oasis_i18n::tr;
/// let s = tr!("ui.ok"); // "OK"
/// ```
///
/// # With interpolation
/// ```
/// use oasis_i18n::tr;
/// let s = tr!("greeting.hello", name = "World"); // "Hello, World!"
/// ```
#[macro_export]
macro_rules! tr {
    // Simple key lookup (returns &str, zero allocation)
    ($key:expr) => {
        $crate::translate($key)
    };
    // Key with named interpolation arguments (returns String)
    ($key:expr, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::translate_with($key, &[
            $( (stringify!($name), &$val.to_string()) ),+
        ])
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Use locale-explicit functions (`translate_for`, `translate_with_for`)
    // to avoid test races from shared global locale state.

    #[test]
    fn test_simple_lookup_en() {
        assert_eq!(translate_for("ui.ok", Locale::English), "OK");
        assert_eq!(translate_for("ui.cancel", Locale::English), "Cancel");
        assert_eq!(translate_for("menu.file", Locale::English), "File");
    }

    #[test]
    fn test_simple_lookup_ja() {
        assert_eq!(translate_for("ui.cancel", Locale::Japanese), "キャンセル");
        assert_eq!(translate_for("ui.close", Locale::Japanese), "閉じる");
        assert_eq!(translate_for("menu.file", Locale::Japanese), "ファイル");
    }

    #[test]
    fn test_interpolation_en() {
        let s = translate_with_for("greeting.hello", &[("name", "World")], Locale::English);
        assert_eq!(s, "Hello, World!");
    }

    #[test]
    fn test_interpolation_ja() {
        let s = translate_with_for("greeting.hello", &[("name", "World")], Locale::Japanese);
        assert_eq!(s, "こんにちは、World！");
    }

    #[test]
    fn test_fallback_to_english() {
        // Unknown key returns key itself regardless of locale
        assert_eq!(
            translate_for("nonexistent.key", Locale::Japanese),
            "nonexistent.key"
        );
    }

    #[test]
    fn test_missing_key_returns_key() {
        assert_eq!(
            translate_for("does.not.exist", Locale::English),
            "does.not.exist"
        );
    }

    #[test]
    fn test_locale_code() {
        assert_eq!(Locale::English.code(), "en");
        assert_eq!(Locale::Japanese.code(), "ja");
    }

    #[test]
    fn test_locale_name() {
        assert_eq!(Locale::English.name(), "English");
        assert_eq!(Locale::Japanese.name(), "日本語");
    }

    #[test]
    fn test_locale_from_code() {
        assert_eq!(Locale::from_code("en"), Some(Locale::English));
        assert_eq!(Locale::from_code("ja"), Some(Locale::Japanese));
        assert_eq!(Locale::from_code("jp"), Some(Locale::Japanese));
        assert_eq!(Locale::from_code("fr"), None);
    }

    #[test]
    fn test_locale_all() {
        let all = Locale::all();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&Locale::English));
        assert!(all.contains(&Locale::Japanese));
    }

    #[test]
    fn test_terminal_interpolation() {
        let s = translate_with_for(
            "terminal.command_not_found",
            &[("command", "foobar")],
            Locale::English,
        );
        assert_eq!(s, "command not found: foobar");
    }

    #[test]
    fn test_terminal_interpolation_ja() {
        let s = translate_with_for(
            "terminal.file_not_found",
            &[("path", "/tmp/test")],
            Locale::Japanese,
        );
        assert_eq!(s, "ファイルが見つかりません: /tmp/test");
    }

    #[test]
    fn test_app_names() {
        assert_eq!(translate_for("app.dashboard", Locale::English), "Dashboard");
        assert_eq!(translate_for("app.browser", Locale::English), "Browser");
        assert_eq!(
            translate_for("app.dashboard", Locale::Japanese),
            "ダッシュボード"
        );
        assert_eq!(translate_for("app.browser", Locale::Japanese), "ブラウザ");
    }

    #[test]
    fn test_get_set_locale() {
        // This test mutates global state but only checks its own writes
        set_locale(Locale::Japanese);
        assert_eq!(get_locale(), Locale::Japanese);
        set_locale(Locale::English);
        assert_eq!(get_locale(), Locale::English);
    }

    #[test]
    fn test_tr_macro_simple() {
        // tr! macro uses global locale; test with explicit set
        set_locale(Locale::English);
        assert_eq!(tr!("ui.ok"), "OK");
    }

    #[test]
    fn test_tr_macro_interpolation() {
        set_locale(Locale::English);
        let s = tr!("greeting.hello", name = 42);
        assert_eq!(s, "Hello, 42!");
    }

    #[test]
    fn test_system_strings() {
        assert_eq!(
            translate_for("system.language", Locale::English),
            "Language"
        );
        assert_eq!(translate_for("system.language", Locale::Japanese), "言語");
        assert_eq!(
            translate_for("system.connected", Locale::Japanese),
            "接続済み"
        );
    }
}
