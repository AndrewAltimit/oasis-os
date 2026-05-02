//! Internationalization framework for OASIS_OS.
//!
//! Provides compile-time embedded translations with a `tr!()` macro for
//! localized string lookup. Supports interpolation and falls back to English
//! when a translation key is missing.
//!
//! # Usage
//!
//! ```no_run
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
const ES_TOML: &str = include_str!("../translations/es.toml");
const DE_TOML: &str = include_str!("../translations/de.toml");
const FR_TOML: &str = include_str!("../translations/fr.toml");
const ZH_TOML: &str = include_str!("../translations/zh.toml");
const KO_TOML: &str = include_str!("../translations/ko.toml");

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
    /// Spanish.
    Spanish = 2,
    /// German.
    German = 3,
    /// French.
    French = 4,
    /// Chinese Simplified.
    Chinese = 5,
    /// Korean.
    Korean = 6,
}

impl Locale {
    /// Return the locale code string (e.g. `"en"`, `"ja"`).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
            Self::Spanish => "es",
            Self::German => "de",
            Self::French => "fr",
            Self::Chinese => "zh",
            Self::Korean => "ko",
        }
    }

    /// Return the human-readable name of the locale.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
            Self::Spanish => "Español",
            Self::German => "Deutsch",
            Self::French => "Français",
            Self::Chinese => "简体中文",
            Self::Korean => "한국어",
        }
    }

    /// List all supported locales.
    #[must_use]
    pub fn all() -> &'static [Locale] {
        &[
            Self::English,
            Self::Japanese,
            Self::Spanish,
            Self::German,
            Self::French,
            Self::Chinese,
            Self::Korean,
        ]
    }

    /// Parse a locale from a code string. Returns `None` if unrecognized.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" | "EN" | "english" | "English" => Some(Self::English),
            "ja" | "JA" | "jp" | "JP" | "japanese" | "Japanese" => Some(Self::Japanese),
            "es" | "ES" | "spanish" | "Spanish" => Some(Self::Spanish),
            "de" | "DE" | "german" | "German" => Some(Self::German),
            "fr" | "FR" | "french" | "French" => Some(Self::French),
            "zh" | "ZH" | "chinese" | "Chinese" => Some(Self::Chinese),
            "ko" | "KO" | "korean" | "Korean" => Some(Self::Korean),
            _ => None,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Japanese,
            2 => Self::Spanish,
            3 => Self::German,
            4 => Self::French,
            5 => Self::Chinese,
            6 => Self::Korean,
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

/// Flatten a TOML file into dot-separated keys. `locale_name` is only used to
/// name the offending locale in the panic message — the embedded TOML strings
/// are static, so a parse failure means a malformed in-tree translation file
/// and is treated as an unrecoverable init error.
fn parse_translations(locale_name: &str, toml_str: &str) -> TranslationMap {
    let root: TomlRoot = toml::from_str(toml_str).unwrap_or_else(|e| {
        panic!("oasis-i18n: failed to parse '{locale_name}' translation TOML: {e}");
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
    locales.insert(Locale::English, parse_translations("en", EN_TOML));
    locales.insert(Locale::Japanese, parse_translations("ja", JA_TOML));
    locales.insert(Locale::Spanish, parse_translations("es", ES_TOML));
    locales.insert(Locale::German, parse_translations("de", DE_TOML));
    locales.insert(Locale::French, parse_translations("fr", FR_TOML));
    locales.insert(Locale::Chinese, parse_translations("zh", ZH_TOML));
    locales.insert(Locale::Korean, parse_translations("ko", KO_TOML));
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
/// ```no_run
/// use oasis_i18n::tr;
/// let s = tr!("ui.ok"); // "OK"
/// ```
///
/// # With interpolation
/// ```no_run
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
        assert_eq!(Locale::from_code("es"), Some(Locale::Spanish));
        assert_eq!(Locale::from_code("de"), Some(Locale::German));
        assert_eq!(Locale::from_code("fr"), Some(Locale::French));
        assert_eq!(Locale::from_code("zh"), Some(Locale::Chinese));
        assert_eq!(Locale::from_code("ko"), Some(Locale::Korean));
        assert_eq!(Locale::from_code("xx"), None);
    }

    #[test]
    fn test_locale_all() {
        let all = Locale::all();
        assert_eq!(all.len(), 7);
        assert!(all.contains(&Locale::English));
        assert!(all.contains(&Locale::Japanese));
        assert!(all.contains(&Locale::Spanish));
        assert!(all.contains(&Locale::German));
        assert!(all.contains(&Locale::French));
        assert!(all.contains(&Locale::Chinese));
        assert!(all.contains(&Locale::Korean));
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

    // --- New locale tests ---

    #[test]
    fn test_locale_codes_and_names() {
        assert_eq!(Locale::Spanish.code(), "es");
        assert_eq!(Locale::German.code(), "de");
        assert_eq!(Locale::French.code(), "fr");
        assert_eq!(Locale::Chinese.code(), "zh");
        assert_eq!(Locale::Korean.code(), "ko");

        assert_eq!(Locale::Spanish.name(), "Español");
        assert_eq!(Locale::German.name(), "Deutsch");
        assert_eq!(Locale::French.name(), "Français");
        assert_eq!(Locale::Chinese.name(), "简体中文");
        assert_eq!(Locale::Korean.name(), "한국어");
    }

    #[test]
    fn test_spanish_translations() {
        assert_eq!(translate_for("ui.cancel", Locale::Spanish), "Cancelar");
        assert_eq!(translate_for("menu.file", Locale::Spanish), "Archivo");
        assert_eq!(translate_for("app.browser", Locale::Spanish), "Navegador");
        assert_eq!(translate_for("system.language", Locale::Spanish), "Idioma");
    }

    #[test]
    fn test_german_translations() {
        assert_eq!(translate_for("ui.cancel", Locale::German), "Abbrechen");
        assert_eq!(translate_for("menu.file", Locale::German), "Datei");
        assert_eq!(translate_for("app.browser", Locale::German), "Browser");
        assert_eq!(translate_for("system.language", Locale::German), "Sprache");
    }

    #[test]
    fn test_french_translations() {
        assert_eq!(translate_for("ui.cancel", Locale::French), "Annuler");
        assert_eq!(translate_for("menu.file", Locale::French), "Fichier");
        assert_eq!(translate_for("app.browser", Locale::French), "Navigateur");
        assert_eq!(translate_for("system.language", Locale::French), "Langue");
    }

    #[test]
    fn test_chinese_translations() {
        assert_eq!(translate_for("ui.cancel", Locale::Chinese), "取消");
        assert_eq!(translate_for("menu.file", Locale::Chinese), "文件");
        assert_eq!(translate_for("app.browser", Locale::Chinese), "浏览器");
        assert_eq!(translate_for("system.language", Locale::Chinese), "语言");
    }

    #[test]
    fn test_korean_translations() {
        assert_eq!(translate_for("ui.cancel", Locale::Korean), "취소");
        assert_eq!(translate_for("menu.file", Locale::Korean), "파일");
        assert_eq!(translate_for("app.browser", Locale::Korean), "브라우저");
        assert_eq!(translate_for("system.language", Locale::Korean), "언어");
    }

    #[test]
    fn test_interpolation_new_locales() {
        let es = translate_with_for("greeting.hello", &[("name", "Mundo")], Locale::Spanish);
        assert_eq!(es, "¡Hola, Mundo!");

        let de = translate_with_for("greeting.hello", &[("name", "Welt")], Locale::German);
        assert_eq!(de, "Hallo, Welt!");

        let fr = translate_with_for("greeting.hello", &[("name", "Monde")], Locale::French);
        assert_eq!(fr, "Bonjour, Monde !");

        let zh = translate_with_for("greeting.hello", &[("name", "世界")], Locale::Chinese);
        assert_eq!(zh, "你好，世界！");

        let ko = translate_with_for("greeting.hello", &[("name", "세계")], Locale::Korean);
        assert_eq!(ko, "안녕하세요, 세계!");
    }

    #[test]
    fn test_all_en_keys_present_in_all_locales() {
        let catalog = &*CATALOG;
        let en_map = catalog.locales.get(&Locale::English).expect("en catalog");
        let en_keys: Vec<&String> = en_map.keys().collect();

        for locale in Locale::all() {
            if *locale == Locale::English {
                continue;
            }
            let map = catalog
                .locales
                .get(locale)
                .unwrap_or_else(|| panic!("missing catalog for {:?}", locale));
            for key in &en_keys {
                assert!(
                    map.contains_key(key.as_str()),
                    "locale {:?} ({}) is missing key: {}",
                    locale,
                    locale.code(),
                    key,
                );
            }
        }
    }
}
